# PMU desk: dashboard specification

The Portfolio Management Unit (PMU) runs the bank's bond portfolio: the bonds, the interest rate
swaps that hedge them, and the repos that fund them. Today the desk works from a Power BI report
with eight tabs. This file records those tabs as the desk described them, the questions still open,
and how the tabs map onto the pages we will build. Keep it up to date as answers come in.

All amounts are in BHD unless a column says local currency. DV01 is in BHD per basis point.

Status: specification complete (8 of 8 tabs). Being built as Phase 2G (plan: FTP curves, the PMU
accrual engine, then the five PMU pages).

---

## The eight Power BI tabs

### Tab 1. Desk overview

Cards
- Portfolio size (BHD)
- Net MTM (BHD)
- MTD bond carry (BHD)
- Net DV01 (BHD per bp)
- MTD NII (BHD)
- Portfolio yield (%)

Charts and tables
- **Portfolio by book (BHD).** AFS BHD, AFS USD, HTM, and any further book users create.
- **Interest earned, month by month (BHD).** Asset EIR income, funding cost, net NII.
- **Net DV01 by book (BHD per bp).**
- **Maturity profile (BHD).** Hedged against unhedged.
- **Position concentration.** Each position with its portfolio (book), face value and yield.
- **Repo book.** Repo amount, available face (BHD), active repos, average repo rate, average
  haircut, weighted maturity in days.

### Tab 2. Accrual and income

Cards (BHD)
- Bond carry
- Coupon
- Amortisation
- FTP funding
- IRS carry
- Accounting NII

Charts and tables
- **Income by position (BHD).** Rows expand and collapse by bond and ISIN. Columns: coupon,
  amortisation, FTP, IRS, bond carry.
- **What makes up bond carry (BHD).**
- **Income earned each day, selected period (BHD).** The tab needs a period picker.
- **Bond carry after FTP, by bond (BHD).**

### Tab 3. Portfolio outlook

Cards
- Year-end bond face (BHD)
- Current weighted bond yield (%)
- Current weighted modified duration
- Year-end hedged face (BHD)

Charts and tables
- **Current weighted bond yield by maturity year.**
- **Table by maturity year.** Columns: portfolio, maturity year, held face (BHD), all-bond yield,
  unhedged yield.
- **Current hedged and unhedged face by currency (BHD).** A pie in Power BI.
- **Current modified duration by portfolio.** Columns: portfolio, weighted average modified
  duration, current market value (BHD).
- **Year-end maturity buckets, hedged and unhedged (BHD).**

### Tab 4. Position register

Cards
- Held face (BHD)
- Market value (BHD)
- Net MTM (BHD)
- Purchase lots (count)
- Kondor ID (%)
- Daily accrual after FTP (BHD)

Charts and tables
- **Position register.** Columns: bond, book, Kondor ID, maturity, trade date, held face (BHD),
  purchased face (local), weighted average price, net MTM (BHD), lots, EIR in bp (weighted).
- **Portfolio by book (BHD).** Same as tab 1.
- **Hedged and unhedged face by currency (BHD).** Same as tab 3.

### Tab 5. Risk and hedges

Charts and tables
- **Hedge pairs, all books (BHD; DV01 in BHD per bp).** Columns: bond/ISIN, bond DV01, IRS DV01,
  bond MTM, IRS MTM, net MTM, closed notional (local).
- **DV01 by maturity bucket (BHD per bp).** Bond DV01, IRS DV01 and net.
- **Parallel rate shock, estimated P&L (BHD).** Scenarios down the side, books across.
- **Daily accrual after FTP, by book (BHD).**
- **Fully unhedged holdings.**

### Tab 6. Liquidity and repo

Cards
- Repo amount (BHD)
- Available face (BHD)
- 7-day repo due (BHD)
- Active repos (count)
- Average repo rate (%)
- 12-month receipts (BHD)

Charts and tables
- **Repo book: funding and repayments (BHD).**
- **Collateral by book (face, BHD).**
- **Coupon and principal receipts, next 12 months (BHD).**
- **Hedge settlement cash.** Kept separate from income.
- **Full-year income by portfolio (BHD).**

### Tab 7. Repo outlook

Charts and tables
- **Active repo amount by remaining maturity (BHD).**
- **Latest repo quotes: spread over SOFR (bp).**
- **Year-end hedge maturity buckets (BHD).**
- **Year-end unhedged maturity buckets (BHD).**

### Tab 8. Income forecast

Cards
- Full-year NII (BHD)
- Remaining-year NII (BHD)
- Full-year bond carry after FTP (BHD)
- Year-end bond face (BHD)

Charts and tables
- **Monthly NII: actual plus forecast (BHD).**
- **Forecast net income (BHD).** A table gives the partial-year dates.
- **Forecast net income by calendar year.** Future periods only.
- **Full-year income by portfolio (BHD).** Same as tab 6.

---

## Shared requirement: the day's FTP curves

ALM publishes the funds transfer pricing (FTP) curves for the day, and every desk sees the same
curves: PMU charges its bonds' funding with them, and the other desks can read them.

- One curve per currency. Today: **BHD, USD, AED, SAR**.
- The set is not fixed: ALM can add a currency's curve or retire one without a code change.
- Each day's publication is a dated, audited record, so the income of any past day can be
  recomputed with the curves that applied that day.
- **How ALM publishes:** ALM drops an Excel workbook holding every curve into the app (or the
  command line). The app reads it, shows a preview with any errors and the changes since the last
  publication, and ALM publishes it with a note. The currencies are whatever the workbook holds.
- **Rate charged to a bond:** locked at purchase (matched maturity). Each purchase lot takes the
  rate for its term from the curve published on its trade date and keeps it for life; floating
  bonds re-lock at each coupon reset.

## Decisions

| Decision | Choice |
|---|---|
| FTP source | ALM's Excel workbook, previewed and published in the app |
| FTP rate on a bond | Locked at purchase, matched maturity |
| Income forecast | Portfolio runs off by default; a switch reinvests maturing face at today's yield for the same tenor |
| Where the data comes from | Synthetic now, shaped for a Kondor feed later; PMU booking tickets in a later phase |

## Definitions (proposed; to confirm with PMU)

| Figure | Definition |
|---|---|
| EIR income | Coupon accrual + amortisation (amortisation = EIR income − coupon accrual) |
| FTP funding | Book value (amortised cost) × locked FTP rate × days / basis |
| Bond carry (after FTP) | Coupon + amortisation − FTP |
| IRS carry | Receive-leg accrual − pay-leg accrual on the swaps linked to the bond |
| Net NII (management) | Bond carry + IRS carry; the "interest earned by month" and monthly NII |
| Accounting NII | Coupon + amortisation + IRS carry − actual repo interest (no FTP: FTP is internal) |
| Net MTM | Bond: (clean price − amortised cost price) × face; swap: present value; net is the sum |
| DV01 | P&L for a 1 bp rise, by full revaluation |
| Hedged face | The smaller of the face and the active linked swap notional |

---

## Figures that appear on more than one tab

| Figure | Tabs |
|---|---|
| Portfolio by book | 1, 4 |
| Hedged and unhedged face by currency | 3, 4 |
| Maturity buckets, hedged and unhedged (now and at year end) | 1, 3, 7 (twice) |
| Full-year income by portfolio | 6, 8 |
| Year-end bond face | 3, 8 |
| Net MTM | 1, 4 |
| Daily accrual after FTP | 4 (card), 5 (by book) |
| Repo summary | 1, 6 |
| Monthly NII (actual) and monthly NII (actual plus forecast) | 1, 8 |

---

## Proposed pages (to be confirmed in the plan)

| Page | Question it answers | Built from |
|---|---|---|
| 1. Overview | How is the desk today? | Tab 1 |
| 2. Positions | What do we hold, and how is each holding doing? | Tabs 4, 2 (income by position), 5 (hedge pairs, unhedged), 1 (concentration) |
| 3. Income | What have we earned, and what will we earn? | Tabs 2, 8 |
| 4. Risk and hedges | What happens if rates move? | Tabs 5, 3 |
| 5. Funding and repo | How are we funded, and what cash is coming? | Tabs 6, 7 |

Every element of the eight tabs must be traceable to one of these pages. The checklist below is
ticked as each one is built.

### Coverage checklist

- [ ] T1 cards: portfolio size, net MTM, MTD carry, net DV01, MTD NII, yield
- [ ] T1 portfolio by book
- [ ] T1 interest earned by month (EIR, funding, net NII)
- [ ] T1 net DV01 by book
- [ ] T1 maturity profile, hedged and unhedged
- [ ] T1 position concentration
- [ ] T1 repo book summary
- [ ] T2 cards: carry, coupon, amortisation, FTP, IRS carry, accounting NII
- [ ] T2 income by position, expandable by bond and ISIN
- [ ] T2 what makes up bond carry
- [ ] T2 income each day, selected period
- [ ] T2 bond carry after FTP by bond
- [ ] T3 cards: year-end face, weighted yield, weighted modified duration, year-end hedged face
- [ ] T3 yield by maturity year
- [ ] T3 table by maturity year (all-bond and unhedged yield)
- [ ] T3 hedged and unhedged face by currency
- [ ] T3 modified duration by portfolio
- [ ] T3 year-end maturity buckets, hedged and unhedged
- [ ] T4 cards: held face, market value, net MTM, lots, Kondor ID %, daily accrual after FTP
- [ ] T4 position register
- [ ] T5 hedge pairs
- [ ] T5 DV01 by maturity bucket
- [ ] T5 parallel rate shock by book
- [ ] T5 daily accrual after FTP by book
- [ ] T5 fully unhedged holdings
- [ ] T6 cards: repo amount, available face, 7-day due, active repos, average rate, 12-month receipts
- [ ] T6 repo funding and repayments
- [ ] T6 collateral by book
- [ ] T6 coupon and principal receipts, next 12 months
- [ ] T6 hedge settlement cash
- [ ] T6 full-year income by portfolio
- [ ] T7 repo by remaining maturity
- [ ] T7 repo quotes, spread over SOFR
- [ ] T7 year-end hedged and unhedged maturity buckets
- [ ] T8 cards: full-year NII, remaining-year NII, full-year carry after FTP, year-end face
- [ ] T8 monthly NII, actual plus forecast
- [ ] T8 forecast net income with partial-year dates
- [ ] T8 forecast net income by calendar year
- [ ] T8 full-year income by portfolio

---

## Open questions

| # | Question | Answer |
|---|---|---|
| 1 | Kondor ID (%): the share of positions (or face) matched to a Kondor deal? | |
| 2 | Closed notional: swap notional unwound early, or swaps that matured? | |
| 3 | Parallel shock scenarios the desk uses (±50, ±100, ±200 bp?) | |
| 4 | Forecast: maturing bonds reinvested at today's yield, or the portfolio runs off? | Runs off, with a reinvest switch |
| 5 | Restructure the FX view into the same five-page shape alongside PMU, or after? | |
| 6 | "FTB" on tab 2 read as FTP | |
| 7 | Do the definitions above match how PMU calculates each figure? | |
| 8 | A real copy of ALM's FTP workbook, so the reader matches its layout | |
