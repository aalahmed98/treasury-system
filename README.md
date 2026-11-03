# NBB Treasury System

Position-keeping and intraday-liquidity decision support for treasury desks. Phase 1 proves the
foundation: a central Postgres database with mock deals, desk entitlements enforced by row-level
security, hardened login, and a native desktop client that shows each user only their desks' deals,
live.

## Layout

| Path | What |
|---|---|
| `crates/domain` | Types shared by server and client (deals, desks, users, API bodies, events) |
| `crates/server` | axum HTTP + SSE server over Postgres. Binary `treasury-server`: `migrate`, `seed`, `serve`, `gen-dev-cert` |
| `crates/client` | egui desktop client. Binary `treasury-client`, builds to a Windows `.exe` |
| `db/migrations` | Schema, RLS policies, triggers, history, login functions, grants |
| `db/init` | Docker-only bootstrap: creates the two roles and the database from `.env` passwords |
| `scripts/` | `smoke.sh` (API check), `verify.sh` (end to end), `build-windows.cmd` |

## Security model

**Two database roles, neither a superuser.** `treasury_owner` owns the schema and runs migrations,
seeding of users, and the tests. `treasury_app` is what the server connects as: it owns nothing,
cannot bypass row-level security, and holds only the grants at the bottom of the migration. It
cannot read password hashes, write users or entitlements, or touch the audit log and deal history
except by appending audit rows. Credentials live in `user_credentials`, reachable only through the
definer-rights function `auth_lookup`.

**Entitlement is decided in Postgres.** Every request that touches deals runs inside a transaction
that first sets `app.user_id`; the policies on `deals` compare that against `user_desk_access`. An
unset identity sees nothing. Booking requires `can_book` on that desk and the deal must be
attributed to the booker. Amending requires booking rights on the deal's own desk, and a trigger
forbids changing a deal's desk, reference or trader, or amending settled and cancelled deals. Every
version of every deal is journaled in `deal_history` by a definer-rights trigger.

**Login hardening.** Argon2id hashes, one verification per attempt whether or not the user exists,
per-username lockout after `LOGIN_MAX_FAILURES` (applied to unknown usernames too, so lockout
cannot enumerate accounts), and a per-IP limit on login attempts. Sessions are opaque random
tokens stored as SHA-256; they end at `SESSION_TTL_HOURS` or after `SESSION_IDLE_MINUTES` without
user activity. Event-driven refetches carry an `x-treasury-passive` header and do not count as
activity, so an abandoned workstation times out even on a busy desk. Every login outcome and
logout is audited with client IP and request id.

**Event streams follow the session.** An open `/events` stream closes immediately on logout and
within ten seconds of expiry, idle timeout, or an entitlement change (its desk filter is reloaded
on every check). Only ids travel on the stream; the client fetches each deal through the
RLS-checked read path.

**Transport.** Plaintext HTTP is accepted only on a loopback address; binding anywhere else needs
`TLS_CERT` and `TLS_KEY`. The client refuses to send credentials over plain http to a non-local
host, takes its server address from the command line or environment rather than a typed field, and
can pin a CA with `--ca` or `TREASURY_CA`.

Reviewed and tested; not yet done: OIDC against the bank's directory, certificate pinning by
fingerprint, a shared store for the per-IP limiter if the server is ever scaled out, signing the
Windows executable.

## Run it (WSL / Linux dev loop)

```bash
cp .env.example .env            # dev passwords for the local Docker database
docker compose up -d --wait     # Postgres 16 on 127.0.0.1:5434 (5433 is taken by a Windows PostgreSQL)
cargo run -p server -- migrate  # as treasury_owner
cargo run -p server -- seed     # 5 users, 500 deals
cargo run -p server             # serves plain http on 127.0.0.1:8787
cargo run -p client             # native window (WSLg) -> log in as fx1 / password
```

Seeded users (password `password`): `fx1`, `fx2` (book FX), `mm1` (book MM), `alm1` (reads FX, MM,
ALM), `head1` (reads everything).

Reset the data: `docker compose down -v`, then `up`, migrate and seed again. The role bootstrap in
`db/init` runs only when the volume is created.

### TLS

```bash
cargo run -p server -- gen-dev-cert          # writes certs/dev-server.pem and certs/dev-server-key.pem
TLS_CERT=certs/dev-server.pem TLS_KEY=certs/dev-server-key.pem BIND_ADDR=127.0.0.1:8787 cargo run -p server
cargo run -p client -- https://localhost:8787 --ca certs/dev-server.pem
```

The generated certificate is for development only. On a Windows-mounted folder its key file
cannot be made private by permissions; keep real keys elsewhere.

## Tests

```bash
set -a; source .env; set +a
cargo test
```

Each server test gets a fresh database created from the migrations. The suite covers the
entitlement matrix, the privilege boundary of the app role, booking and amendment policies, deal
history, session expiry and idle timeout, token hashing, lockout, per-IP limiting, the HTTP login
flow with audit fields, NOTIFY on insert, and the event stream's behaviour on logout, session
death and entitlement change. `./scripts/verify.sh` runs the whole thing end to end.

`cargo audit` (install with `cargo install cargo-audit`) checks the lockfile against RustSec.

## Windows .exe

From PowerShell or cmd on Windows (Rust 1.95+ MSVC toolchain):

```
scripts\build-windows.cmd
C:\dev\targets\treasury\release\treasury-client.exe http://localhost:8787
```

The server can keep running under WSL. If `localhost` does not reach it from Windows, enable
mirrored networking in `%USERPROFILE%\.wslconfig` (`[wsl2]` / `networkingMode=mirrored`). For a
server on another machine use `https://` and pass `--ca` (or set `TREASURY_CA`).

## Not in phase 1

Kondor / Bloomberg / Quantum adapters, the matching engine, OIDC login against AD or Entra, deal
amendment UI, Power BI replica. See the plan for the agreed direction.
