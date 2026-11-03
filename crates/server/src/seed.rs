//! Development users, credentials and entitlements, written by the owner role (the app role has no
//! such rights). Deals are not seeded here: they arrive through the reconciliation pipeline
//! (`treasury-recon synth`) or are booked through the API.

use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;

use crate::auth::password;

pub const DEV_PASSWORD: &str = "password";

/// (desk code, can_book, can_resolve)
struct SeedUser {
    username: &'static str,
    display_name: &'static str,
    home: &'static str,
    access: &'static [(&'static str, bool, bool)],
}

const USERS: &[SeedUser] = &[
    SeedUser { username: "fx1", display_name: "FX Trader One", home: "FX", access: &[("FX", true, true)] },
    SeedUser { username: "fx2", display_name: "FX Trader Two", home: "FX", access: &[("FX", true, true)] },
    SeedUser { username: "mm1", display_name: "MM Trader One", home: "MM", access: &[("MM", true, true)] },
    SeedUser {
        username: "alm1",
        display_name: "ALM Analyst",
        home: "ALM",
        access: &[("FX", false, false), ("MM", false, false), ("ALM", false, false)],
    },
    SeedUser {
        username: "head1",
        display_name: "Head of Treasury",
        home: "HEAD",
        access: &[("FX", false, true), ("MM", false, true), ("ALM", false, true), ("HEAD", false, true), ("UNASSIGNED", false, true)],
    },
];

pub fn user_id(username: &str) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_OID, format!("treasury-user:{username}").as_bytes())
}

/// `owner` must be the schema owner. The app pool is accepted for call-site compatibility.
pub async fn run(owner: &PgPool, _app: &PgPool) -> anyhow::Result<()> {
    let desks: HashMap<String, i16> = sqlx::query_as::<_, (i16, String)>("SELECT id, code FROM desks")
        .fetch_all(owner)
        .await?
        .into_iter()
        .map(|(id, code)| (code, id))
        .collect();
    let desk = |code: &str| desks[code];

    let hash = password::hash(DEV_PASSWORD)?;
    for u in USERS {
        let id = user_id(u.username);
        sqlx::query(
            "INSERT INTO users (id, username, display_name, home_desk_id) VALUES ($1, $2, $3, $4) \
             ON CONFLICT (username) DO UPDATE SET display_name = EXCLUDED.display_name",
        )
        .bind(id)
        .bind(u.username)
        .bind(u.display_name)
        .bind(desk(u.home))
        .execute(owner)
        .await?;
        sqlx::query(
            "INSERT INTO user_credentials (user_id, password_hash) VALUES ($1, $2) \
             ON CONFLICT (user_id) DO UPDATE SET password_hash = EXCLUDED.password_hash, updated_at = now()",
        )
        .bind(id)
        .bind(&hash)
        .execute(owner)
        .await?;
        for (code, can_book, can_resolve) in u.access {
            sqlx::query(
                "INSERT INTO user_desk_access (user_id, desk_id, can_book, can_resolve) VALUES ($1, $2, $3, $4) \
                 ON CONFLICT (user_id, desk_id) DO UPDATE SET can_book = EXCLUDED.can_book, can_resolve = EXCLUDED.can_resolve",
            )
            .bind(id)
            .bind(desk(code))
            .bind(can_book)
            .bind(can_resolve)
            .execute(owner)
            .await?;
        }
    }
    tracing::info!(count = USERS.len(), "users seeded (development password: see README)");
    Ok(())
}

