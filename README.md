# NBB Treasury System

A reconciliation-first golden record for treasury deals. Observations from the front office
(Finastra Kondor), execution (Bloomberg), the back office (FIS Quantum) and the desk's own bookings
are ingested into one append-only log, matched into golden deals, tracked per source through
executed → booked → confirmed → settled, held to expected-lag SLAs, and surfaced to each desk with
its breaks and provenance. Nothing is ever written back to a source system. Entitlement is enforced
by Postgres row-level security, not by application code.

Phase 1 built the foundation (database, hardened login, a first native client, live updates). Phase 2A built
the reconciliation core with synthetic feeds that reproduce real failure modes. Phase 2B added the
intraday liquidity ladder with synthetic nostro statements, a booking ticket with amendments, and the
deal history view. Phase 2C turned the client into a dashboard (overview, tabs,
attention panel, light and dark mode) and added the ALM workbench: LCR, NSFR, NII and NIM, stress
testing, variance attribution, an interest rate differential calculator, the repricing gap and funding
concentration, on a synthetic banking book with the bank's limits as a versioned parameter set.
Phase 2D rebuilt the interface as a web client in TypeScript served by the same server at `/app`
(the same screens, richer tables and charts, printing and CSV export) and wrapped it in a small
Windows desktop application with its own icon. Phase 2E redesigned those pages
around the user's work (a command palette, keyboard selection, an attention-first overview, side
sheets instead of modals, a break queue, toasts and skeletons), split them into chunks that load on
demand, hardened the server's responses and the desktop shell, and retired the native egui client:
the React pages are now the whole interface. Phase 2F added the FX desk's risk
and P&L view (positions, P&L, the forward ladder, counterparty exposure, VaR, versioned limits set
by the treasury head, breach sign-off, an off-market check on the ticket) and the rest of the ALCO
toolkit (the liquidity maturity ladder, early-warning indicators with the contingency plan's stage,
balance-sheet ratios, the annual report's ±200 bp test, and a signed-off ALCO pack). Phase 2G, the
current state, publishes ALM's daily FTP curves to every desk from its Excel workbook and brings in
the PMU desk: the bank's bond portfolio with its hedging swaps and repo funding, accrued daily at
each purchase's effective interest rate and charged FTP at the rate locked on its trade date, in five
pages that replace the desk's eight-tab Power BI report (`docs/pmu-desk-spec.md`). The
implementation plan with what is blocked and what is not lives in `docs/implementation-plan.html`.

## Layout

| Path | What |
|---|---|
| `crates/domain` | Types shared by everything: deals, sources, breaks, provenance, API bodies, events, the business-day calendar, the banking book and the ALM parameters and results |
| `crates/alm` | Pure ALM maths: contractual schedules, LCR, NSFR, curves and Basel shocks, repricing gap, NII projection, EVE, stress, attribution, IRD, funding concentration |
| `crates/pmu` | Pure bond-portfolio maths: coupon schedules and day counts, price and yield, the effective interest rate, the daily accrual and amortisation of each purchase lot, FTP locked at purchase, swap carry and valuation, repo interest, DV01 and rate shocks, cash flows and the income forecast |
| `crates/server` | axum HTTP + SSE API over Postgres. Binary `treasury-server`: `migrate`, `seed`, `serve`, `gen-dev-cert`, `ftp-import` |
| `crates/recon` | Reconciliation core (normaliser, matcher, SLA, pipeline, synthetic world, synthetic banking book). Binary `treasury-recon`: `serve`, `synth`, `rebuild`, `nostro`, `alm-synth`, `status` |
| `web/` | the web client: Vite, React and TypeScript pages served by the server at `/app`; `e2e/` holds the Playwright checks and the screenshot runner |
| `web/src-tauri` | the Windows desktop shell (Tauri 2): the web client in a window of its own, with a tray entry and a remembered server; built on Windows only |
| `assets/` | the bundled typeface, Inter (OFL 1.1) as a static build with tabular figures, the favicon and the theme bootstrap script; served by the server under `/app` |
| `DESIGN.md`, `PRODUCT.md` | the visual system and the product brief it serves |
| `docs/pmu-desk-spec.md` | the PMU desk's eight Power BI tabs as described by the desk, the decisions taken, the proposed definitions, where each tab went, and the open questions |
| `db/migrations` | `0001_init` (identity, deals, RLS, login hardening), `0002_recon` (event log, source state, breaks, feeds, recon role), `0003_liquidity` (cash flows, nostro statements, amendments, history access), `0004_alm` (banking-book snapshots, market data, versioned parameters, income by month, activity feed), `0005_risk` (versioned FX limits, breach sign-offs, ALCO commentary and sign-off), `0006_ftp` (the PMU desk, FTP publications), `0007_pmu` (books, bonds, purchase lots, swaps with their hedges and unwinds, repos and collateral, repo quotes, bond prices), `0008_dealing_desks` (deals only on the FX and money-market desks, each with its own products), `0009_pmu_booking` (PMU purchases, sales, repos and reverse repos, closing and rolling, Kondor ids, through definer functions for the desk's bookers), `0010_ftp_uploads` (the FTP upload log: every workbook dropped, kept with who dropped it, and which publication replaced which) |
| `db/init` | Docker-only bootstrap: the three roles and the database, passwords from `.env` |
| `scripts/` | `smoke.sh` (API check), `verify.sh` (end to end from a clean database), `web-setup.cmd` and `build-web.cmd` (web client), `build-desktop.cmd` (desktop shell and installer) |

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
a dot or a word. One typeface, Inter (OFL, bundled under `assets/fonts` as a static build with
tabular figures on by default), in three weights carries the hierarchy. Metrics sit in a stat strip
rather than a grid of tiles, tools and filters are chips, counts are badges, and every table, card
and gauge comes from the same few components in `web/src/components/` styled by the tokens in
`web/src/styles/tokens.css`, so both themes are designed rather than inverted.

The **ALM** tab is a tool strip: Dashboard, LCR, NSFR, NII / NIM, EVE, Stress, Attribution, IRD, Gap,
Funding and Setup. Each tool shows the bank's saved assumptions on the left, editable as an unsaved
what-if, and the result on the right with its thresholds. Setup starts a draft from the active version
or the current what-if and saves it as a new version with a note; any user on the ALM desk may save,
the server validates, the database function versions and audits it, and every open client is told
over the live stream. Version 1 holds Basel III defaults; the CBB LM-11 and LM-12 calibration and the
bank's own limits are to be confirmed, which is why every factor is a parameter.

**One rate shock for every rates tool.** Under the tool strip sits the rate shock bar, the only
place a shock is chosen: none, one of the six Basel shapes, or a custom size in basis points aimed
at any of the repricing gap's nine buckets (all of them is a parallel move; fewer move only the
rates at those tenors, as a step at each bucket edge). Gap, NII, EVE and Stress all run under it, so
switching tabs shows the same shock from each side, and each result carries it in its heading and
on a printout. It is a simulation: it lives in the browser session, is never saved, and the
dashboard, the ALCO pack and the official figures never see it. The liquidity tools keep the bar in
place and say it does not reach them. Under a shock the Gap tab adds the classic gap estimate of
the effect on twelve-month NII (each bucket's gap times the move times the part of the year left
after it reprices; the balances themselves do not move). The EVE tab shows the full revaluation by
bucket and currency beside the duration gap and its estimate,
`ΔEVE ≈ −[D_A − (L/A) × D_L] × A × Δy / (1 + y)`, which is given only for a parallel move because
that is what the formula assumes. NII shows the projection under three balance-sheet assumptions
side by side: constant and run-off (the two static views) and dynamic, where cohorts are replaced
and balances grow at a yearly rate per side or per product, the cash growth takes or brings going
through the balancing book. Deposits with no maturity take a behavioural profile per product: a
life in months, a core share (the rest reprices overnight) and a yearly decay of the core (zero
runs it off evenly); a product with neither set behaves as before, so saved parameter versions
give the same figures.

**Trial hedges.** Beside the shock, the bar holds trial hedges: pay-fixed and receive-fixed
swaps, fixed-rate funding and fixed-rate investments, each with a currency, notional, tenor and a
fixed rate that defaults to the curve at that tenor. A hedge is two cohorts in a `HEDGE` book (a
fixed leg and a floating leg, `crates/alm/src/hedge.rs`) added to the balance sheet for that
request only: nothing is booked, the result is never cached, and the notionals stay out of total
assets and earning assets. Gap, NII, EVE and Stress show the figures with the hedges and one line
saying what they changed against the unhedged run; the liquidity tools do not see them.

**Saved scenarios.** The simulation on screen (the shock, the what-if assumptions and the trial
hedges) can be kept under a name from Scenarios on the bar and opened again. The assumptions are
stored as changes against the bank's parameters, so a scenario applies to whichever version is
active when it is opened. A scenario is its owner's alone until they share it with the desk
(`alm_scenarios`, migration 0011: row-level security shows a user their own and the shared ones,
and every write goes through a definer function); a shared one may be put in the ALCO pack, where
it is listed under Interest rate risk and marked as a simulation. The Stress tab lays the current
simulation and every visible saved scenario side by side (`POST /alm/compare`), each recomputed on
the snapshot in use. The pack's own tables always ask for the bank's figures: no shock from the
bar, no hedges.

**Every lever acts.** Each assumption on a tool's rail changes that tool's result (checked one by
one against the live engine). A lever with nothing in the snapshot to act on, such as a factor for
a maturity band no balance falls into, is drawn dashed with "nothing in this snapshot"; hovering
any other lever shows the balance it applies to (`POST /alm/coverage`, read from the LCR and NSFR
calculations' own lines, so it counts exactly what the ratios count). NII: deposit betas are ALM
assumptions per product, or product and segment (`CURRENT_ACCOUNT.RETAIL` beats
`CURRENT_ACCOUNT`), with a default for products not listed; they act only when rates move. The
constant balance sheet switch, when off, runs the book off: a cohort earns or pays nothing after
maturity and the cash it releases or takes is placed or funded at the scenario's one-month rate (the
BALANCING book), with NIM on average earning assets. NSFR: cash and central-bank reserves take their
own factor (0%), not the Level 1 securities factor. The rate floor binds only when a shock takes a
rate below it, which at NBB's rate levels takes a custom shock of several hundred basis points.

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

## FTP curves and the PMU desk

**FTP.** ALM publishes the day's funds transfer pricing curves from its Excel workbook. On the ALM
tab's FTP tool the workbook is dropped in; the server reads it (`calamine`), in either of two
layouts (one sheet with a `Tenor` column and a column per currency code, or one sheet per currency
named by its code; an `As of` cell sets the date), names every bad cell by sheet and reference,
reads rates written as fractions as percent, and compares it with the curves in force (currencies
added or missing, tenors added or dropped, points moved by more than 25 bp). Nothing is stored until
Publish, which needs a note; a corrected workbook replaces the day's earlier publication and both
are kept.

**FTP upload log.** Every workbook dropped is stored as it arrived (bytes, size, SHA-256 computed by
the database) with who dropped it and when, published or not. Publishing names the dropped file:
the server reads the stored file again and publishes exactly what it holds, so the log's file and
the curves always agree, and only the person who dropped a workbook may publish it. A publication
that replaces the day's earlier one records on the earlier one when it was replaced and by which
publication (and so by whom). The FTP page shows the log to every desk (`GET /ftp/log`), newest
first, each entry with its file to download (`GET /ftp/upload/{id}/file`, audited); ALM's publish
page shows the same log plus the workbooks dropped and never published, which other desks cannot
read (row-level security), and says before publishing whose curves for that day will be replaced. The currencies are whatever the workbook holds (BHD, USD, AED and SAR today), so a curve
is added or retired by the workbook, not by code. Every desk reads the curves in force on the FTP
page and in the status bar; ALM is reminded when a business day passes without a publication. The
template download is the curves in force in the wide layout, ready to edit. For a file that arrives
in a folder: `treasury-server ftp-import <workbook.xlsx> --note "..." [--as-of D] [--user alm1]`.

**PMU desk.** The Portfolio Management Unit's bond portfolio, for the PMU desk, ALM and the head.
The purchase lot is the base record (book, bond, trade and settlement dates, face, price, Kondor
ID); a position is every open lot of a bond in a book. The `pmu` crate accrues each lot day by day:
its amortised cost starts at what was paid and grows at the effective interest rate fixed at
purchase, falling by each coupon and the redemption, so it reaches zero exactly at maturity; the
day's income splits into the coupon accrual and the amortisation. The FTP charge is the book value
at the rate for the lot's term from the curve published on its trade date, locked for life
(floating bonds re-lock at each reset). Swaps linked to the lots they hedge carry their legs daily
and are valued on one curve; repos carry their interest. DV01 and the ±50, ±100 and ±200 bp shocks
revalue every bond at its yield moved and every swap on its curve moved. The forecast runs the
portfolio off to year end and three calendar years ahead, or reinvests maturing face at today's
yield for the same tenor. The proposed definitions (carry, net NII, accounting NII) are on the
pages and in the spec, to be confirmed with the desk.

The five pages: **Overview** (the six headline figures with their move since yesterday, by book,
NII by month booked and forecast, the maturity profile now and at year end, the repo book, the
largest holdings), **Positions** (one tree from book to bond to lot with holding, income and risk
column sets, filters, a period for income, CSV, and a bond's lots, swaps, repos and cash flows),
**Income** (a period picker, the parts of income as a waterfall, each day, by bond and by book, and
the forecast), **Risk and hedges** (DV01 by bucket, shocks by book, hedge pairs, duration and daily
accrual by portfolio, hedged and unhedged by currency and by maturity, yield by maturity year, the
unhedged holdings) and **Funding and repo** (repos by time left, funding raised and repaid,
collateral, quotes over SOFR with their age, receipts, swap settlement cash, full-year income by
portfolio). The portfolio loads once per business date and data stamp; each page computes in under
0.1 s. Attention items: a held currency with no FTP curve, lots without a Kondor ID, repos maturing
within seven days, large unhedged positions, repo quotes more than two business days old.

**Booking.** The PMU desk's bookers (`pmu1`; ALM and the head read only) buy and sell bonds and
book repos from the PMU strip. The bond ticket finds a bond by ISIN, name or issuer, or sets up an
ISIN the system does not know (its check digit verified, its terms entered once). It prices the
trade from a clean price or a yield for the face and settlement date (T+2 business days):
accrued, consideration in local currency and BHD, the effective rate, the FTP rate a purchase will
lock and the carry after it, DV01, duration, and the distance from the market (beyond 15 bp a tick
is needed). A sale draws on the oldest lots first, or on lots the dealer picks, and shows the gain
realised on each against amortised cost; face pledged to a repo cannot be sold. The repo ticket
books a repo against bonds whose face is free over the whole term (the cash currency first, cash
from the market value less the haircut, the rate prefilled from the counterparty's latest quote
over SOFR) or a reverse repo lending cash against the counterparty's bond; the repo book closes a
repo early or rolls it. A booking counts at once and is provisional until it carries a Kondor deal
id, attached on the ticket or later; holdings count from the trade date, accrual from settlement,
and realised gains are kept apart from NII. The server recomputes the preview at booking and the
database functions check the face is free under row locks.

A user who sees no dealing desk (a PMU user) gets no Blotter, Breaks or Liquidity tab and no feed
health: their Overview leads with the bond portfolio. Deals are booked on the FX and money-market
desks only, each with its own products; the ticket offers nothing else and the database refuses
anything else (`0008`), whatever booking rights a user holds on another desk.

**Synthetic portfolio.** Until the Kondor feed and Bloomberg prices are connected, `alm-synth`
also writes daily FTP publications (the interbank curve plus a term premium; never over a day ALM
published) and, once, a portfolio of about forty bonds (Bahrain, GCC and supranational sovereigns,
sukuk, banks and corporates, with `SYN` ISINs), about 120 purchase lots, payer swaps on most fixed
dollar holdings with some unwinds, repos with their collateral, and daily prices and repo quotes,
extended to the business date on every run.

## Web client and desktop app

The interface is `web/`: a Vite, React and TypeScript project built from `DESIGN.md` (the tokens are
CSS variables in `web/src/styles/tokens.css`, both themes designed). The Rust server serves the built pages at `/app` and redirects `/` there; every deep link
such as `/app/alm/lcr` falls back to the page so the router inside it takes over. The pages carry no
data: they sign in against `/auth/login`, hold the bearer session for the tab, send the passive
header on background polls (feeds every 5 s, attention 15 s, activity 30 s, the ALM overview 15 s),
read the live event stream with `fetch` so the bearer header travels, and send one real request a
minute while the user is active. A deal or break event refetches the lists, an ALM event the
parameter-driven views, a resync everything; lists stay fresh for a minute in between, so moving
between pages never waits on the network.

How the pages work for the person using them: the Overview opens with "Needs attention", the
server's list of what is wrong on the user's desks, each row a link, before any figure. A command
palette (Ctrl K or `/`) jumps to a page, an ALM tool, an action or any deal or break by reference,
counterparty or pair; `g` then a letter changes page; `?` lists the shortcuts. In the blotter and
the breaks the selected row is in the address (shareable, back-button safe), the arrow keys move
it, Enter opens it, Esc closes it; a View menu chooses columns and row density and the page
remembers both; filter chips carry counts; empty states say why nothing is there. The booking
ticket is a sheet beside the blotter, not a dialog over it, so the book stays in view; the breaks
page is a queue that moves to the next open break after an acknowledge, dismiss or link. The ladder
draws the cumulative balance with today marked and a horizon switch. The ALM assumptions rail
closes to give results the width, and a what-if lists every changed parameter against the bank's
version with a revert on each. Every action answers with a toast, every first load with a skeleton,
and a banner says when the live stream is reconnecting. Tables are virtualised, sortable and
resizable with CSV export; charts have hover tooltips; the ALM pages print as the ALCO pack. The
trading pages and the ALM workbench are separate chunks fetched when their tab is hovered, so the
first load is the shell and the Overview.

A design critique on 4 November (`.impeccable/critique/`) scored the client 27 of 40 and drove a
second pass: the Overview separates what needs a decision from system health and folds three stale
feeds into one line; the bell counts only what is new since it was last opened; the four numbers
the head signs (LCR, NSFR, worst EVE, ten largest depositors) sit in a hero row with their standing
in words; the top bar is tabs, search, one feeds pill, the bell and the user menu (theme and server
live in the menu); the ALM assumptions are explained, collapsible groups with the bank's value shown
as a ghost beside any edit; Setup lists exactly what a new version changes, with a revert on each
line, and will not save an unchanged set or one without a note; dismissing a break asks once more;
`a` acknowledges the selected break and `/` goes to the page's search; every failed load offers
"Try again"; small text is 12 px. A second critique (30 of 40) drove one more pass: Shift-click,
Ctrl-click and Ctrl A pick several breaks, which can be acknowledged or dismissed with one note in
batches; the ALM results dim and say so while a recompute runs, with "Try again" on a failure; the
balance-sheet charts share one surface instead of four cards; the bell groups system health the way
the page does; the ladder opens on the days with flows. Tooling runs on Windows where Node is installed (`node_modules`
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

Then on Windows, either open `http://localhost:8787/app/` in a browser (after `scripts\build-web.cmd`)
or install `dist\NBB-Treasury-Setup.exe` (built by `scripts\build-desktop.cmd`; the raw build output
lives in `C:\dev\targets\`, outside the OneDrive folder) and point it at `http://localhost:8787`.
Log in as `fx1`, `mm1`, `alm1` or `head1` with password `password`. `./scripts/stop-dev.sh` stops
the services.

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

132 Rust tests. PMU and FTP: 8 hand-worked engine cases (a par purchase earns its coupon, accrued
interest under each day count, price and yield inverting and DV01 against duration, a premium bond
amortised to par exactly at maturity, FTP locked to the trade-date curve, a par swap worth nothing
and its carry with an unwind, the parts of income adding up with hedged face and the tree, the
forecast running off or reinvesting), 5 on the workbook reader (both layouts, fractions, bad cells by
reference, the template read back), 3 over HTTP for publishing (preview, publish, supersede, rights; the upload log with who replaced whom, drafts hidden from the desks, the file downloaded as dropped)
and 4 for the portfolio (a hand-built bond, swap and repo; who may see it; deals only on the dealing desks; the whole booking flow), plus 3 engine cases for sales and repos and the ISIN check digit. FX risk: 9 hand-worked cases (revaluation of a direct and a cross deal, covered
interest parity and discounting, the shorthand position, the P&L split, VaR and expected shortfall,
no VaR on pegged currencies, counterparty add-on and settlement peak, the off-market measure, the
forward ladder); the liquidity ladder's two views; the early-warning thresholds and stage; and 3
over HTTP (a hand-worked book with desk scoping, who may set limits and sign off breaches, the ALCO
pack's commentary and sign-off). Recon: 18 unit tests (tolerances, normaliser, matcher, SLA maths, generator determinism
and scenario coverage), 12 pipeline scenarios on a fresh database each (three-way match, reversed
quote within tolerance, mismatch then correction, missing after SLA then late arrival, orphan then
manual link, ambiguity, duplicates at every layer, amend before new, cancel propagation, internal
anchor and merge, poisoned event, rebuild) and 3 on the synthetic banking book. Domain: 2 on the
calendar. ALM: 24 hand-worked cases (Basel caps, an LCR, inflow cap and currency conversion,
schedules by amortisation profile, curves and shocks with the floor, NSFR factors and bands, gap
slotting and limits, NII by cohort type, deposit betas by product and segment, NII on a balance sheet running off, cash needing no stable funding, what each lever acts on, EVE on the shocked curve, a custom shock aimed at chosen buckets through the curve, EVE and NII, the gap's income estimate, the duration gap against the full revaluation, deposits split into a decaying core and an overnight rest, a dynamic balance sheet funding its growth, a trial swap starting at par and taking the rate risk it is sized for, stress across scenarios with the
survival horizon and FX, attribution adding up exactly, covered interest parity, funding
concentration). Server: 14 phase-1 tests, 7 on the reconciliation privilege boundary, 3 on the
reconciliation endpoints, 4 on the liquidity endpoints, 4 on the ALM privilege boundary, 6 on the
ALM endpoints and 1 on the browser hardening headers.

The web client adds 19 Vitest cases (the formatting, chart, IRD, rate-shock, trial-hedge and saved-scenario helpers, the ports checked against the Rust crate's
cases) and 51 Playwright checks on the installed Edge, among them NII's betas, default beta, run-off and growth moving the figures, one rate shock chosen once and followed across Gap, EVE, NII and Stress, deposit behaviour moving the gap, a trial hedge changing every rates tool and no liquidity tool, a scenario saved, reopened, shared with the head and listed in the pack, idle levers marked, the FTP page and ALM's
template-drop-publish round trip, a second workbook replacing the first with the log naming who replaced whom, the five PMU pages (the tree's expand and filters, the period in
the address, a bond's detail, the forecast switch, old quotes flagged; buying by ISIN, setting up a
new bond, selling chosen lots, a repo closed early and a reverse repo), the FX risk page, the limits
page as a dealer and as the head, the ticket's off-market check, and the ladder, early-warning and
ALCO pack tools: sign-in and its errors, the ALM entitlement
of the tab and the address, the overview strips per desk and the attention list, booking a deal from
the side sheet and its toast, filters, provenance, keyboard selection with the address following, the
positions panel preference, a break's actions, the ladder drill-down, every ALM tool's headline, a
what-if edit with its change list and reset, the assumptions rail, a Setup draft, the command palette
and the page keys, and a console watch for Content Security Policy violations across the pages.

`./scripts/verify.sh` runs the whole thing from a clean database and checks the behaviour over HTTP,
including a rebuild and, when `web/dist` exists, the pages at `/app`. `cargo audit` checks the
lockfile against RustSec.

## Windows builds

```
scripts\build-web.cmd            # web client     -> web\dist, served at /app
scripts\build-desktop.cmd        # desktop shell  -> dist\NBB-Treasury-Setup.exe and dist\NBB-Treasury.exe
```

For the bank's build of the shell, set `TREASURY_FIXED_SERVER=https://treasury.nbb.internal` before
`build-desktop.cmd`: the shell then opens that server and nothing else, never asks for an address
and has no change-server menu. Every build refuses to navigate anywhere but its server and its own
first-run page. The server adds a Content Security Policy and the other browser hardening headers to
every response (HSTS once it speaks TLS or sits behind a terminator that says so). Code signing of
the installer is the bank's step.

## Not yet built

Real Kondor, Bloomberg and Quantum adapters (waiting on NBB's answers about replica access and
database engines), real nostro statement feeds, the real banking-book extract and market-data feed
behind the ALM workbench, the CBB calibration of the ALM parameters, OIDC against the bank's
directory, settlement-date SLAs, allocations, code-signing; for the PMU desk, swap (hedge)
booking, corrections and cancellations of a booking, the Kondor and Bloomberg feeds behind the
portfolio (matching bookings automatically, static data for new bonds), and a real copy of ALM's
FTP workbook to confirm the reader against. See `docs/implementation-plan.html` for
the full picture.
