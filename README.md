# NBB Treasury System

Position-keeping and intraday-liquidity decision support for treasury desks. Phase 1 proves the
foundation: a central Postgres database with mock deals, desk entitlements enforced by row-level
security, login, and a native desktop client that shows each user only their desks' deals, live.

## Layout

| Path | What |
|---|---|
| `crates/domain` | Types shared by server and client (deals, desks, users, API bodies, events) |
| `crates/server` | axum HTTP + SSE server over Postgres. Binary `treasury-server` with `migrate`, `seed`, `serve` |
| `crates/client` | egui desktop client. Binary `treasury-client`, builds to a Windows `.exe` |
| `db/migrations` | Schema, RLS policies, NOTIFY trigger, positions view |
| `scripts/` | `smoke.sh` (API check), `verify.sh` (end to end), `build-windows.cmd` |

## How entitlement works

The server connects to Postgres as `treasury_app`, a role that owns nothing and cannot bypass
row-level security. Every request that touches deals runs inside a transaction that first sets
`app.user_id`; the RLS policies on `deals` compare that against `user_desk_access`. An unset
identity sees nothing. Booking requires `can_book` on that desk. The server never filters deals in
application code, so a bug in a handler cannot leak another desk's data.

Live updates: a trigger on `deals` calls `pg_notify`; the server fans the (id, desk) out to SSE
subscribers whose entitlements include that desk; the client then fetches the deal by id through the
normal RLS-checked read path.

## Run it (WSL / Linux dev loop)

```bash
cp .env.example .env            # already done once
docker compose up -d            # Postgres 16 on localhost:5433
cargo run -p server -- migrate  # as treasury_owner
cargo run -p server -- seed     # 5 users, 500 deals
cargo run -p server             # serves on 0.0.0.0:8787
cargo run -p client             # native window (WSLg) -> log in as fx1 / password
```

Seeded users (password `password`): `fx1`, `fx2` (book FX), `mm1` (book MM), `alm1` (reads FX, MM,
ALM), `head1` (reads everything).

Reset the data: `docker compose down -v`, then migrate and seed again.

## Tests

```bash
set -a; source .env; set +a
cargo test -p server
```

Each test gets a fresh database created from the migrations and checks the RLS matrix, booking
policy, session expiry, token hashing, the HTTP login flow and NOTIFY on insert.

`./scripts/verify.sh` runs the whole thing end to end including a live-event check.

## Windows .exe

From PowerShell or cmd on Windows (needs Rust 1.95+ MSVC toolchain, already installed):

```
scripts\build-windows.cmd
C:\dev\targets\treasury\release\treasury-client.exe http://localhost:8787
```

The server can keep running under WSL. If `localhost` does not reach it from Windows, enable
mirrored networking in `%USERPROFILE%\.wslconfig` (`[wsl2]` / `networkingMode=mirrored`) or use
the WSL IP from `hostname -I`.

The client takes the server URL as its first argument or from `TREASURY_SERVER`.

## Not in phase 1

Kondor / Bloomberg / Quantum adapters, the matching engine, OIDC login against AD or Entra, TLS,
deal amendment, Power BI replica. See the design notes in the plan for the agreed direction.
