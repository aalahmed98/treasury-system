# NBB Treasury System

A reconciliation-first golden record for treasury deals. Observations from the front office
(Finastra Kondor), execution (Bloomberg), the back office (FIS Quantum) and the desk's own bookings
are ingested into one append-only log, matched into golden deals, tracked per source through
executed → booked → confirmed → settled, held to expected-lag SLAs, and surfaced to each desk with
its breaks and provenance. Nothing is ever written back to a source system. Entitlement is enforced
by Postgres row-level security, not by application code.

Phase 1 built the foundation (database, hardened login, native client, live updates). Phase 2A, the
current state, built the reconciliation core with synthetic feeds that reproduce real failure modes.
Phase 2B (liquidity ladder, booking ticket, history view) comes next.

## Layout

| Path | What |
|---|---|
| `crates/domain` | Types shared by everything: deals, sources, breaks, provenance, API bodies, events |
| `crates/server` | axum HTTP + SSE API over Postgres. Binary `treasury-server`: `migrate`, `seed`, `serve`, `gen-dev-cert` |
| `crates/recon` | Reconciliation core (calendar, normaliser, matcher, SLA, pipeline, synthetic world). Binary `treasury-recon`: `serve`, `synth`, `rebuild`, `status` |
| `crates/client` | egui desktop client (Windows `.exe`): blotter with match status, provenance, breaks, feed freshness |
| `db/migrations` | `0001_init` (identity, deals, RLS, login hardening) and `0002_recon` (event log, source state, breaks, feeds, recon role) |
| `db/init` | Docker-only bootstrap: the three roles and the database, passwords from `.env` |
| `scripts/` | `smoke.sh` (API check), `verify.sh` (end to end from a clean database), `build-windows.cmd` |

## How reconciliation works

```
feeds -> source_events (full image per observation, append-only, deduplicated by hash)
            |  treasury-recon: one batch per transaction, one savepoint per event
            v
   source_deals (latest image per source ref, link to a golden deal)
   deals       (golden record: economics owned by the highest-precedence source, derived status,
                confirmed flag, match status)
   breaks      (MISSING, MISMATCH, ORPHAN, AMBIGUOUS, DUPLICATE; idempotent by key)
   feed_status / recon_status (heartbeats and the worker's clock: visible freshness)
```

**Matching is deterministic and tiered.** An observation is attached by its own reference, then by
a cross reference in either direction, then by economics: product, side, pair, trade date and
notional (within 0.01%), with a strict pass (exact rate, counterparty, value date) before a relaxed
one (0.5 pip or 1 bp, alias-equal counterparty, value date within one business day). Several
candidates are an AMBIGUOUS break, never a guess. A second record from a source already linked to
the deal is a DUPLICATE. After every link, unlinked records in the same bucket are retried, so the
outcome does not depend on arrival order. Amends apply only when newer than what was applied; an
older image is recorded as STALE. Internal bookings are self-identifying and absorb a provisional
golden deal created earlier from an external source.

**Per-source lifecycle.** Golden status is the furthest lifecycle any linked source reports;
CANCELLED only from a source with cancel authority (Kondor, or the booking itself while nothing
else has seen it). `confirmed` is set when the back office reaches CONFIRMED. Field disagreements
with the owning source become MISMATCH breaks that resolve themselves when a later image agrees.

**SLA monitor.** Expectations are directional and per hop (Bloomberg→Kondor 5 min, internal→Kondor
10 min, Kondor→Quantum 30 min, plus grace). A source expected and not seen by its deadline is a
MISSING break that resolves itself on arrival, recording how late it was. Unlinked records from a
source that cannot originate deals become ORPHAN breaks after a grace window. A feed whose heartbeat
has gone stale does not generate MISSING breaks.

**User actions.** Acknowledging is direct. Linking and dismissing are appended to the event log as
USER events and applied by the worker in order, so they are auditable and replayable. A dismissal
stands until the observation changes. Rights: `can_resolve` on the desk (bookers and the treasury
head by default).

**Rebuild.** `treasury-recon rebuild` wipes links, breaks and external-origin deals and replays
the log. Deals, links and observation-derived breaks come back identical. Known limitation: a user
action on a break that was raised by an SLA tick cannot be replayed, because that break is re-raised
with a new id; the rebuild reports how many such actions were skipped so they can be re-applied.

## Security model

Three database roles, none a superuser:

- `treasury_owner` owns the schema and runs migrations, seeding and tests.
- `treasury_app` is the API role. It reads deals, reconciliation rows and feed status through
  desk-scoped policies, books internal deals, and can only act on breaks through definer-rights
  functions that check `can_resolve`. It cannot read credentials, write entitlements, or touch the
  event log, source state or breaks directly.
- `treasury_recon` is the worker role. It appends to the event log, owns source state and breaks,
  creates and annotates external-origin deals, and may never change the economics of an internal
  booking or reach identity tables.

Everything from phase 1 still holds: argon2id passwords in a separate table, hashed session tokens
with absolute and idle expiry, per-username lockout and per-IP throttling, event streams that close
on logout, plaintext HTTP only on loopback, audit rows with client IP and request id. See
`db/migrations/0001_init.sql` and `0002_recon.sql` for the policies and grants.

## Run it (WSL / Linux dev loop)

```bash
cp .env.example .env                      # dev passwords for the local Docker database
docker compose up -d --wait               # Postgres 16 on 127.0.0.1:5434
cargo run -p server -- migrate            # as treasury_owner
cargo run -p server -- seed               # users and entitlements only
cargo run -p recon -- synth --mode replay --speed 20000 --days 5   # a synthetic world through the pipeline (~40 s)
cargo run -p recon -- serve               # the worker, for anything that arrives from now on
cargo run -p server                       # the API on 127.0.0.1:8787
cargo run -p client                       # the desktop client (WSLg); log in as fx1 / password
```

Seeded users (password `password`): `fx1`, `fx2` (book and resolve FX), `mm1` (book and resolve
MM), `alm1` (reads FX, MM, ALM), `head1` (reads everything, resolves anywhere).

`treasury-recon synth` modes: `replay` (simulated clock from the first event, at `--speed`),
`catch-up` (everything up to now at once, then live), `realtime` (a world for today, fed as it
happens; add `--keep-serving`). `--dump file.json` writes the generated observations instead of
feeding them; `--config synth.toml` overrides the built-in world, scenario mix and per-feed lag,
drop, duplicate, reorder, rounding and naming behaviour. `treasury-recon status` prints the worker
clock, feed freshness, deals by match status and breaks by kind.

Reset everything: `docker compose down -v`, then the steps above again.

### TLS

```bash
cargo run -p server -- gen-dev-cert
TLS_CERT=certs/dev-server.pem TLS_KEY=certs/dev-server-key.pem cargo run -p server
cargo run -p client -- https://localhost:8787 --ca certs/dev-server.pem
```

## Tests

```bash
set -a; source .env; set +a
cargo test --workspace
```

58 tests. Recon: 20 unit tests (calendar, tolerances, normaliser, matcher, SLA maths, generator
determinism and scenario coverage) and 12 pipeline scenarios on a fresh database each (three-way
match, reversed quote within tolerance, mismatch then correction, missing after SLA then late
arrival, orphan then manual link, ambiguity, duplicates at every layer, amend before new, cancel
propagation, internal anchor and merge, poisoned event, rebuild). Server: 14 phase-1 tests, 7 on
the reconciliation privilege boundary, 3 on the reconciliation endpoints.

`./scripts/verify.sh` runs the whole thing from a clean database and checks the behaviour over HTTP,
including a rebuild. `cargo audit` checks the lockfile against RustSec.

## Windows .exe

```
scripts\build-windows.cmd
C:\dev\targets\treasury\release\treasury-client.exe http://localhost:8787
```

## Not yet built

Phase 2B: intraday liquidity ladder, booking ticket, deal history view. Later: real Kondor,
Bloomberg and Quantum adapters (waiting on NBB's answers about replica access and database
engines), OIDC against the bank's directory, settlement-date SLAs, allocations, code-signing.
