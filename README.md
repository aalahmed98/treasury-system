# NBB Treasury System

A reconciliation-first golden record for treasury deals. Observations from the front office
(Finastra Kondor), execution (Bloomberg), the back office (FIS Quantum) and the desk's own bookings
are ingested into one append-only log, matched into golden deals, tracked per source through
executed → booked → confirmed → settled, held to expected-lag SLAs, and surfaced to each desk with
its breaks and provenance. Nothing is ever written back to a source system. Entitlement is enforced
by Postgres row-level security, not by application code.

Phase 1 built the foundation (database, hardened login, native client, live updates). Phase 2A built
the reconciliation core with synthetic feeds that reproduce real failure modes. Phase 2B added the
intraday liquidity ladder with synthetic nostro statements, a booking ticket with amendments, and the
deal history view. Phase 2C, the current state, turned the client into a dashboard (overview, tabs,
attention panel, light and dark mode) and added the ALM workbench: LCR, NSFR, NII and NIM, stress
testing, variance attribution, an interest rate differential calculator, the repricing gap and funding
concentration, on a synthetic banking book with the bank's limits as a versioned parameter set.
Phase 2D, the current state, rebuilt the interface as a web client in TypeScript served by the same
server at `/app` (the same screens, richer tables and charts, printing and CSV export) and wrapped it
in a small Windows desktop application with its own icon; the native client stays until the web
pages are accepted. The implementation plan with what is blocked and what is not lives in
`docs/implementation-plan.html`.

## Layout

| Path | What |
|---|---|
| `crates/domain` | Types shared by everything: deals, sources, breaks, provenance, API bodies, events, the business-day calendar, the banking book and the ALM parameters and results |
| `crates/alm` | Pure ALM maths: contractual schedules, LCR, NSFR, curves and Basel shocks, repricing gap, NII projection, EVE, stress, attribution, IRD, funding concentration |
| `crates/server` | axum HTTP + SSE API over Postgres. Binary `treasury-server`: `migrate`, `seed`, `serve`, `gen-dev-cert` |
| `crates/recon` | Reconciliation core (normaliser, matcher, SLA, pipeline, synthetic world, synthetic banking book). Binary `treasury-recon`: `serve`, `synth`, `rebuild`, `nostro`, `alm-synth`, `status` |
| `crates/client` | egui desktop client (Windows `.exe`): overview, blotter with match status and provenance, breaks, liquidity ladder, ALM workbench |
| `web/` | the web client: Vite, React and TypeScript pages served by the server at `/app`; `e2e/` holds the Playwright checks and the screenshot runner |
| `web/src-tauri` | the Windows desktop shell (Tauri 2): the web client in a window of its own, with a tray entry and a remembered server; built on Windows only |
| `assets/fonts` | the bundled typeface for both clients: Inter (OFL 1.1) as a static build with tabular figures |
| `DESIGN.md`, `PRODUCT.md` | the visual system and the product brief it serves |
| `db/migrations` | `0001_init` (identity, deals, RLS, login hardening), `0002_recon` (event log, source state, breaks, feeds, recon role), `0003_liquidity` (cash flows, nostro statements, amendments, history access), `0004_alm` (banking-book snapshots, market data, versioned parameters, income by month, activity feed) |
| `db/init` | Docker-only bootstrap: the three roles and the database, passwords from `.env` |
| `scripts/` | `smoke.sh` (API check), `verify.sh` (end to end from a clean database), `build-windows.cmd` (native client), `web-setup.cmd` and `build-web.cmd` (web client), `build-desktop.cmd` (desktop shell and installer) |

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

## Liquidity ladder

`cash_flows_v` turns every live golden deal into signed cash flows: FX moves ccy1 by the notional on
the value date and ccy2 by notional times rate the other way; money-market deals move the principal
at the value date and return principal plus interest at maturity (ACT/360, ACT/365 for GBP). The view
runs with the caller's entitlement, so a desk trader's ladder is their desks' deals and the
treasury head's ladder is the bank's. `GET /ladder` groups the flows per value date, split confirmed
against provisional, and runs a cumulative balance from the latest nostro statement. Users entitled
to the ALM or HEAD desk also see nostro actuals and the difference per date, with drill-downs into
the flows and the movements behind any date.

Nostro statements and movements are written by the reconciler. Until the real MT942 / camt.052 feed
exists, `treasury-recon synth` (and `treasury-recon nostro --from --to`) generates them from the
expected flows of booked deals: an intraday and an end-of-day statement per account per business
day, movements with a small share missing or adjusted by fees, and a few customer and fee movements
treasury never booked.

## Booking, amendments and history

The client's booking ticket posts to `POST /deals`. `PATCH /deals/{id}` amends counterparty, rate,
notional, value date or maturity of an internal booking; `POST /deals/{id}/cancel` cancels it. Both
are limited by policy to internal bookings on desks the user may book on, and the database trigger
keeps identity columns and terminal deals frozen. Every change enters the reconciliation log, so an
amendment that disagrees with Kondor shows up as a mismatch. `GET /deals/{id}/history` lists every
version with the fields that changed and who changed them, through a definer function that applies
desk entitlement to the append-only history table.

## Dashboard and ALM workbench

The client opens on an **Overview** that is desk-aware: deals on the latest trade date, provisional
and unmatched counts, open breaks by kind, SLA breaches, feed health, the next five value dates of
expected cash, recent activity and the largest positions. ALM and treasury-head users also get the
balance-sheet block: LCR, NSFR, NII for the next twelve months and NIM, income month and year to
date against budget, the one-year cumulative gap, the worst EVE scenario and depositor
concentration, eight limit rows with gauges, a 30-day LCR trend, actual against budget by month and
the gap by bucket. The top bar carries the tabs, the feed ages, the attention bell (a server-computed,
severity-sorted list: SLA breaches, breaks awaiting a decision, stale feeds, deals unconfirmed beyond
a business day, large settlements on the next business day, limit breaches, parameter changes; each
item opens the right place) and the System, Light or Dark theme switch, which is remembered between
runs.

The look is a small design system, written up in `DESIGN.md` (the tokens and components) and
`PRODUCT.md` (who it is for and what it must never look like). Warm-tinted neutrals lean toward the
NBB red, which itself appears only in the wordmark so that red keeps meaning breach on every data
surface; one ink accent carries selection and the primary action; status colours always travel with
a dot or a word. One typeface, Inter (OFL, bundled under `crates/client/assets/fonts` as a static
build with tabular figures on by default), in three weights carries the hierarchy. Metrics sit in a
stat strip rather than a grid of tiles, tools and filters are chips, counts are badges, and every
table, card and gauge is drawn by the same few functions in `crates/client/src/ui/widgets.rs` from
the palette in `crates/client/src/ui/theme.rs`, so both themes are designed rather than inverted.

The **ALM** tab is a tool strip: Dashboard, LCR, NSFR, NII / NIM, Stress, Attribution, IRD, Gap,
Funding and Setup. Each tool shows the bank's saved assumptions on the left, editable as an unsaved
what-if, and the result on the right with its thresholds. Setup starts a draft from the active version
or the current what-if and saves it as a new version with a note; any user on the ALM desk may save,
the server validates, the database function versions and audits it, and every open client is told
over the live stream. Version 1 holds Basel III defaults; the CBB LM-11 and LM-12 calibration and the
bank's own limits are to be confirmed, which is why every factor is a parameter.

The maths lives in `crates/alm` and is pure: the server runs it on a banking-book snapshot (and
caches official results per snapshot, generation time and parameter version), the client runs the
IRD calculator on the latest curves itself. Analytics carry `f64`; every ratio with a possibly-zero
denominator is optional. The business date is the reconciler's published clock when it has one and
the wall clock otherwise, so a replayed world shows a coherent "today".

**Synthetic banking book.** Until the bank's own extracts arrive (loans, deposits and securities from
BaNCS or Finacle, curves and spots from Bloomberg, budget figures from Finance),
`treasury-recon alm-synth` (also run at the end of `synth`) generates an NBB-scale balance sheet of
about ninety cohorts across six currencies, snapshotted daily for the last 35 days and at the
previous 13 month-ends, with 400 days of daily curves, spots, twenty depositor groups and monthly
actual and budget income. It is deterministic by seed, written in bulk, and a snapshot becomes
visible only once complete.

## Web client and desktop app

The same screens as the native client, as web pages: `web/` is a Vite, React and TypeScript project
built from `DESIGN.md` (the tokens are CSS variables in `web/src/styles/tokens.css`, both themes
designed). The Rust server serves the built pages at `/app` and redirects `/` there; every deep link
such as `/app/alm/lcr` falls back to the page so the router inside it takes over. The pages carry no
data: they sign in against `/auth/login`, hold the bearer session for the tab, send the passive
header on background polls (feeds every 5 s, attention 15 s, activity 30 s, the ALM overview 15 s),
read the live event stream with `fetch` so the bearer header travels, and send one real request a
minute while the user is active, like the native client. A deal or break event refetches the lists,
an ALM event the parameter-driven views, a resync everything.

What the browser adds over the native client: virtualised, sortable, resizable tables with CSV
export, charts with hover tooltips, a print stylesheet for the ALM pages (the ALCO pack), keyboard
reachability, and no installation. Tooling runs on Windows where Node is installed (`node_modules`
lives under `C:\dev` through a junction, off OneDrive, like the cargo target directory):

```
scripts\web-setup.cmd          # once: dependencies
scripts\build-web.cmd          # type-check and build web\dist; the running server serves it at once
cd web && npm test              # Vitest: the formatting and IRD ports against the crate's cases
cd web && npm run e2e           # Playwright on the installed Edge against http://localhost:8787
cd web && npm run shots         # every page in both themes to web\e2e\shots for a layout review
```

The desktop application is a Tauri 2 shell: a few hundred lines of Rust that open `<server>/app/`
in the WebView2 engine Windows already ships, with the NBB icon in the taskbar, a tray entry (open,
change server, quit) and a remembered window. On first run it asks for the server address (or takes
`--server https://host` and `TREASURY_SERVER`); the pages then come from the server, so a desk never
reinstalls for an interface change. `scripts\build-desktop.cmd` produces `dist\NBB-Treasury-Setup.exe`
(per-user NSIS installer, no admin rights) and the portable `dist\NBB-Treasury.exe`. Code signing
and the bank's packaging are the bank's step; the WebView2 runtime is part of Windows 10 and 11.

## Security model

Three database roles, none a superuser:

- `treasury_owner` owns the schema and runs migrations, seeding and tests.
- `treasury_app` is the API role. It reads deals, reconciliation rows and feed status through
  desk-scoped policies, books internal deals, and can only act on breaks through definer-rights
  functions that check `can_resolve`. It cannot read credentials, write entitlements, or touch the
  event log, source state or breaks directly.
- `treasury_recon` is the worker role. It appends to the event log, owns source state and breaks,
  creates and annotates external-origin deals, writes the synthetic banking book and market data,
  and may never change the economics of an internal booking, reach identity tables or touch the
  ALM parameter set.

Bank-wide ALM data (the banking book, depositor groups, income, parameters) is readable only by
users entitled to the ALM or HEAD desk; market data is readable by every desk. Parameters are saved
only through `alm_parameters_save`, which checks the entitlement, validates the document, swaps the
active flag under a row lock and notifies clients. Recent activity comes through `recent_activity`,
a definer function that applies desk entitlement to the history and break tables.

Everything from phase 1 still holds: argon2id passwords in a separate table, hashed session tokens
with absolute and idle expiry, per-username lockout and per-IP throttling, event streams that close
on logout, plaintext HTTP only on loopback, audit rows with client IP and request id. See
`db/migrations/0001_init.sql` and `0002_recon.sql` for the policies and grants.

## Quick start

From a WSL terminal in the project folder:

```bash
./scripts/start-dev.sh      # database, migrations, seed, a synthetic world if the database is empty, reconciler, API
```

Then on Windows, either open `http://localhost:8787/app/` in a browser (after `scripts\build-web.cmd`),
install `dist\NBB-Treasury-Setup.exe` (built by `scripts\build-desktop.cmd`) and point it at
`http://localhost:8787`, or open the native `dist\treasury-client.exe` (built by
`scripts\build-windows.cmd`; the raw build output lives in `C:\dev\targets\`, outside the OneDrive
folder). Log in as `fx1`, `mm1`, `alm1` or `head1` with password `password`. `./scripts/stop-dev.sh`
stops the services.

## Run it step by step (WSL / Linux dev loop)

```bash
cp .env.example .env                      # dev passwords for the local Docker database
docker compose up -d --wait               # Postgres 16 on 127.0.0.1:5434
cargo run -p server -- migrate            # as treasury_owner
cargo run -p server -- seed               # users and entitlements only
cargo run -p recon -- synth --mode replay --speed 20000 --days 5   # a synthetic world through the pipeline, nostro statements included (~40 s)
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

### Development snapshots

Debug builds of the client accept a few environment variables so a layout can be checked from a
script, in either theme, without anyone at the keyboard. Release builds ignore them.

```bash
TREASURY_DEV_LOGIN=alm1:password TREASURY_SNAPSHOT=/tmp/overview.png \
TREASURY_SNAPSHOT_TAB=overview TREASURY_SNAPSHOT_THEME=light TREASURY_SNAPSHOT_AFTER=6 \
cargo run -q -p client
```

`TREASURY_SNAPSHOT_BELL=1` opens the attention popup, `TREASURY_SNAPSHOT_TOOL=stress` opens an ALM
tool (`TREASURY_SNAPSHOT_DRAFT=1` with `setup` starts a draft), `TREASURY_SNAPSHOT_SCREEN=login`
captures the sign-in screen without a session, `TREASURY_SNAPSHOT_SIZE=680x410` sets the window and
`TREASURY_SNAPSHOT_ZOOM=0.5` counters a scaled display (WSLg reports 2x). The window closes itself
once the PNG is written.

## Tests

```bash
set -a; source .env; set +a
cargo test --workspace
```

88 tests. Recon: 18 unit tests (tolerances, normaliser, matcher, SLA maths, generator determinism
and scenario coverage), 12 pipeline scenarios on a fresh database each (three-way match, reversed
quote within tolerance, mismatch then correction, missing after SLA then late arrival, orphan then
manual link, ambiguity, duplicates at every layer, amend before new, cancel propagation, internal
anchor and merge, poisoned event, rebuild) and 3 on the synthetic banking book. Domain: 2 on the
calendar. ALM: 14 hand-worked cases (Basel caps, an LCR, inflow cap and currency conversion,
schedules by amortisation profile, curves and shocks with the floor, NSFR factors and bands, gap
slotting and limits, NII by cohort type, EVE on the shocked curve, stress across scenarios with the
survival horizon and FX, attribution adding up exactly, covered interest parity, funding
concentration). Server: 14 phase-1 tests, 7 on the reconciliation privilege boundary, 3 on the
reconciliation endpoints, 4 on the liquidity endpoints, 4 on the ALM privilege boundary and 5 on the
ALM endpoints.

The web client adds 6 Vitest cases (the formatting and IRD ports, checked against the Rust crate's
cases) and 14 Playwright checks on the installed Edge: sign-in and its errors, the ALM entitlement
of the tab and the address, the overview strips per desk, booking a deal and finding it, filters,
provenance, a break's actions, the ladder drill-down, every ALM tool's headline, a what-if edit and
its reset, and a Setup draft.

`./scripts/verify.sh` runs the whole thing from a clean database and checks the behaviour over HTTP,
including a rebuild and, when `web/dist` exists, the pages at `/app`. `cargo audit` checks the
lockfile against RustSec.

## Windows builds

```
scripts\build-windows.cmd        # native client  -> dist\treasury-client.exe
scripts\build-web.cmd            # web client     -> web\dist, served at /app
scripts\build-desktop.cmd        # desktop shell  -> dist\NBB-Treasury-Setup.exe and dist\NBB-Treasury.exe
```

## Not yet built

Real Kondor, Bloomberg and Quantum adapters (waiting on NBB's answers about replica access and
database engines), real nostro statement feeds, the real banking-book extract and market-data feed
behind the ALM workbench, the CBB calibration of the ALM parameters, OIDC against the bank's
directory, settlement-date SLAs, allocations, code-signing, retiring the native client once the web
pages are accepted on the desk. See `docs/implementation-plan.html` for the full picture.
