use std::time::Duration;

use anyhow::{Context, anyhow};
use chrono::{DateTime, Utc};
use domain::{BreakDetail, BreakView, Deal, DealsResponse, ErrorBody, FeedsResponse, LoginRequest, LoginResponse, MeResponse, Position, ProvenanceResponse, ResolveRequest};
use reqwest::{Certificate, RequestBuilder, Response, StatusCode};
use uuid::Uuid;

/// Marks machine-initiated requests so the server does not count them as user activity.
const PASSIVE_HEADER: &str = "x-treasury-passive";

/// A non-2xx reply, with the server's message when it sent one.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct ApiFailure {
    pub status: StatusCode,
    pub message: String,
}

pub fn is_unauthorized(e: &anyhow::Error) -> bool {
    e.downcast_ref::<ApiFailure>().is_some_and(|f| f.status == StatusCode::UNAUTHORIZED)
}

#[derive(Clone)]
pub struct Api {
    http: reqwest::Client,
    base: String,
    token: Option<String>,
    ca: Option<Certificate>,
}

impl Api {
    pub fn new(base: &str, ca: Option<Certificate>) -> Self {
        let http = build_client(&ca, Some(Duration::from_secs(20)));
        Self { http, base: base.trim_end_matches('/').to_string(), token: None, ca }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    /// A client with no overall timeout, for the long-lived event stream.
    pub fn stream_client(&self) -> reqwest::Client {
        build_client(&self.ca, None)
    }

    fn auth(&self, rb: RequestBuilder, passive: bool) -> RequestBuilder {
        let rb = match &self.token {
            Some(t) => rb.bearer_auth(t),
            None => rb,
        };
        if passive { rb.header(PASSIVE_HEADER, "1") } else { rb }
    }

    /// Consumes the unauthenticated client and returns one carrying the session token.
    pub async fn login(mut self, username: &str, password: &str) -> anyhow::Result<(Self, LoginResponse)> {
        let res = self
            .http
            .post(format!("{}/auth/login", self.base))
            .json(&LoginRequest { username: username.to_string(), password: password.to_string() })
            .send()
            .await
            .map_err(connect_error)?;
        let res = check(res).await.map_err(|e| match e.downcast_ref::<ApiFailure>() {
            Some(f) if f.status == StatusCode::UNAUTHORIZED => anyhow!("invalid username or password"),
            _ => e,
        })?;
        let resp: LoginResponse = res.json().await.context("decoding login response")?;
        self.token = Some(resp.token.clone());
        Ok((self, resp))
    }

    pub async fn logout(&self) -> anyhow::Result<()> {
        check(self.auth(self.http.post(format!("{}/auth/logout", self.base)), false).send().await?).await?;
        Ok(())
    }

    /// Proves the user is still here; counts as activity for the idle timeout.
    pub async fn me(&self) -> anyhow::Result<MeResponse> {
        let res = self.auth(self.http.get(format!("{}/me", self.base)), false).send().await.map_err(connect_error)?;
        Ok(check(res).await?.json().await?)
    }

    pub async fn deals(&self, since: Option<DateTime<Utc>>, passive: bool) -> anyhow::Result<DealsResponse> {
        let mut req = self.http.get(format!("{}/deals", self.base));
        if let Some(s) = since {
            req = req.query(&[("since", s.to_rfc3339())]);
        }
        Ok(check(self.auth(req, passive).send().await.map_err(connect_error)?).await?.json().await?)
    }

    pub async fn deal(&self, id: Uuid) -> anyhow::Result<Deal> {
        let res = self.auth(self.http.get(format!("{}/deals/{id}", self.base)), true).send().await?;
        Ok(check(res).await?.json().await?)
    }

    pub async fn provenance(&self, id: Uuid) -> anyhow::Result<ProvenanceResponse> {
        let res = self.auth(self.http.get(format!("{}/deals/{id}/provenance", self.base)), true).send().await?;
        Ok(check(res).await?.json().await?)
    }

    pub async fn breaks(&self, all: bool, passive: bool) -> anyhow::Result<Vec<BreakView>> {
        let req = self.http.get(format!("{}/breaks", self.base)).query(&[("status", if all { "all" } else { "open" })]);
        Ok(check(self.auth(req, passive).send().await.map_err(connect_error)?).await?.json().await?)
    }

    pub async fn break_detail(&self, id: i64) -> anyhow::Result<BreakDetail> {
        let res = self.auth(self.http.get(format!("{}/breaks/{id}", self.base)), true).send().await?;
        Ok(check(res).await?.json().await?)
    }

    pub async fn ack_break(&self, id: i64) -> anyhow::Result<BreakView> {
        let res = self.auth(self.http.post(format!("{}/breaks/{id}/ack", self.base)), false).send().await.map_err(connect_error)?;
        Ok(check(res).await?.json().await?)
    }

    pub async fn resolve_break(&self, id: i64, req: &ResolveRequest) -> anyhow::Result<BreakView> {
        let res = self.auth(self.http.post(format!("{}/breaks/{id}/resolve", self.base)).json(req), false).send().await.map_err(connect_error)?;
        Ok(check(res).await?.json().await?)
    }

    pub async fn feeds(&self) -> anyhow::Result<FeedsResponse> {
        let res = self.auth(self.http.get(format!("{}/feeds", self.base)), true).send().await.map_err(connect_error)?;
        Ok(check(res).await?.json().await?)
    }

    pub async fn positions(&self, passive: bool) -> anyhow::Result<Vec<Position>> {
        let res = self.auth(self.http.get(format!("{}/positions", self.base)), passive).send().await.map_err(connect_error)?;
        Ok(check(res).await?.json().await?)
    }
}

fn build_client(ca: &Option<Certificate>, timeout: Option<Duration>) -> reqwest::Client {
    let mut b = reqwest::Client::builder().connect_timeout(Duration::from_secs(8));
    if let Some(t) = timeout {
        b = b.timeout(t);
    }
    if let Some(c) = ca {
        b = b.add_root_certificate(c.clone());
    }
    b.build().expect("reqwest client")
}

fn connect_error(e: reqwest::Error) -> anyhow::Error {
    if e.is_connect() {
        anyhow!("cannot reach the server")
    } else if e.is_timeout() {
        anyhow!("server timed out")
    } else {
        anyhow!(e)
    }
}

/// Turn non-2xx responses into `ApiFailure`, using the server's `{error}` body when present.
async fn check(res: Response) -> anyhow::Result<Response> {
    let status = res.status();
    if status.is_success() {
        return Ok(res);
    }
    let message = res
        .json::<ErrorBody>()
        .await
        .map(|b| b.error)
        .unwrap_or_else(|_| status.canonical_reason().unwrap_or("request failed").to_string());
    Err(ApiFailure { status, message }.into())
}
