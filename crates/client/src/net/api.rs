use std::time::Duration;

use anyhow::{Context, anyhow};
use chrono::{DateTime, Utc};
use domain::{Deal, DealsResponse, ErrorBody, LoginRequest, LoginResponse, Position};
use reqwest::{RequestBuilder, Response, StatusCode};
use uuid::Uuid;

#[derive(Clone)]
pub struct Api {
    http: reqwest::Client,
    base: String,
    token: Option<String>,
}

impl Api {
    pub fn new(base: &str) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(8))
            .build()
            .expect("reqwest client");
        Self { http, base: base.trim_end_matches('/').to_string(), token: None }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    fn auth(&self, rb: RequestBuilder) -> RequestBuilder {
        match &self.token {
            Some(t) => rb.bearer_auth(t),
            None => rb,
        }
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
        let resp: LoginResponse = check(res).await?.json().await.context("decoding login response")?;
        self.token = Some(resp.token.clone());
        Ok((self, resp))
    }

    pub async fn logout(&self) -> anyhow::Result<()> {
        check(self.auth(self.http.post(format!("{}/auth/logout", self.base))).send().await?).await?;
        Ok(())
    }

    pub async fn deals(&self, since: Option<DateTime<Utc>>) -> anyhow::Result<DealsResponse> {
        let mut req = self.http.get(format!("{}/deals", self.base));
        if let Some(s) = since {
            req = req.query(&[("since", s.to_rfc3339())]);
        }
        Ok(check(self.auth(req).send().await.map_err(connect_error)?).await?.json().await?)
    }

    pub async fn deal(&self, id: Uuid) -> anyhow::Result<Deal> {
        let res = self.auth(self.http.get(format!("{}/deals/{id}", self.base))).send().await?;
        Ok(check(res).await?.json().await?)
    }

    pub async fn positions(&self) -> anyhow::Result<Vec<Position>> {
        let res = self.auth(self.http.get(format!("{}/positions", self.base))).send().await.map_err(connect_error)?;
        Ok(check(res).await?.json().await?)
    }

    /// A client with no overall timeout, for the long-lived event stream.
    pub fn stream_client(&self) -> reqwest::Client {
        reqwest::Client::builder().connect_timeout(Duration::from_secs(8)).build().expect("reqwest client")
    }
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

/// Turn non-2xx responses into readable errors using the server's `{error}` body when present.
async fn check(res: Response) -> anyhow::Result<Response> {
    let status = res.status();
    if status.is_success() {
        return Ok(res);
    }
    let msg = res
        .json::<ErrorBody>()
        .await
        .map(|b| b.error)
        .unwrap_or_else(|_| status.canonical_reason().unwrap_or("request failed").to_string());
    Err(match status {
        StatusCode::UNAUTHORIZED => anyhow!("invalid username or password"),
        _ => anyhow!("{msg}"),
    })
}
