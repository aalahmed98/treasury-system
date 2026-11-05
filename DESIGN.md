# NBB Treasury client: visual system

The web client reads this file as its design system. Tokens live in `web/src/styles/tokens.css` as
custom properties (one set per theme) and the components in `web/src/components/` wear them; feature
code never holds a colour or a size of its own.

## Scene and theme

An ALM analyst at NBB head office at 08:30, two 24-inch monitors under office light, Excel open
beside the client. Light is the default and the primary design. Dark is a selected set for the
dealing room and evening work: warmer and lighter than a toolkit's default dark, with depth from
surface lightness rather than shadow.

## Colour

Strategy: **restrained**. Warm-tinted neutrals (OKLCH hue 30, toward the NBB red, chroma 0.002 to
0.014), one accent (ink: near-black on light, near-white on dark) for selection, the active tab and
primary actions, and a fixed status set. NBB red appears only in the wordmark, so red keeps meaning
breach on every data surface.

| Role | Light | Dark | Use |
|---|---|---|---|
| canvas | `#f6f3f2` | `#0e0a0a` | page behind everything |
| surface | `#fefcfc` | `#161312` | top bar, cards, tables |
| surface_alt | `#faf7f6` | `#1e1b1a` | side panels, striped rows, status bar |
| raised | `#fefdfd` | `#242120` | popups, menus, windows |
| hairline | `#e2dddc` | `#2c2827` | borders, separators, gridlines |
| border | `#c9c2c1` | `#474140` | hover outline, axis |
| hover_wash / active_wash | `#f1ebea` / `#e8e1e0` | `#282322` / `#322c2b` | interactive states |
| text / text_2 / muted / faint | `#201917` / `#59504f` / `#786f6d` / `#a59c9b` | `#ebe7e6` / `#ada6a5` / `#867e7d` / `#5b5352` | primary, secondary, labels, disabled |
| ink / ink_text | `#261d1b` / `#fcf9f9` | `#e8e3e2` / `#16100f` | selected chip, primary button |
| selection | `#ece2e0` | `#3a302f` | selected table row |
| good / good_soft | `#00792b` / `#ddf6e0` | `#4ec36f` / `#15301b` | comfortable, confirmed, inflow |
| warn / warn_soft | `#a26500` / `#ffedcb` | `#f0b135` / `#3b2b0d` | watch, provisional, stale |
| bad / bad_soft | `#be2323` / `#ffe6e2` | `#f66d67` / `#47211e` | breach, break, outflow, error |
| info / info_soft | `#1f68bc` / `#e1f0ff` | `#60aaf3` / `#142a41` | booked, ambiguous, links |
| brand | `#d62223` | `#ed4b43` | the wordmark only |
| violet / magenta | `#654db6` / `#b53c7f` | `#a495f0` / `#f080b8` | orphan / duplicate break kinds |

Chart series (validated, fixed order, never cycled): light `#2a78d6 #eb6834 #1baf7a #eda100 #e87ba4
#008300`, dark `#3987e5 #d95926 #199e70 #c98500 #d55181 #008300`. Status colours never stand alone: a
dot or a word sits beside them.

## Typography

One family: Inter (OFL), bundled as `assets/fonts/InterTreasury-*.ttf`, a static build with tabular
figures on by default so every column of numbers aligns. Three weights carry the hierarchy (regular,
medium, semibold).

| Style | Size | Weight | Use |
|---|---|---|---|
| hero | 32 | semibold | the one number a tool is about |
| title | 20 | semibold | page title |
| stat | 24 | semibold | stat-strip values |
| heading | 15 | semibold | card and panel titles |
| body / button / monospace | 13 | regular / medium / regular | text, controls, figures |
| label | 12 | medium | table headers, form labels |
| small | 12 | regular | metadata, hints |
| eyebrow | 11 | medium, caps, +0.6 tracking | section headers |

## Spacing and shape

4-point scale: 4, 8, 12, 16, 24, 32. Item spacing 8 × 6, button padding 10 × 5, card padding 14,
panel padding 16 to 20, section gap 24. Radii: controls 6, cards 8, popups 10, badges full. One
hairline stroke everywhere; light mode has one soft shadow on popups, dark mode none.

## Components

- **Stat strip**: one surface with vertical hairlines between cells (label, value, sub line with a
  status dot) instead of a grid of identical tiles. Clickable cells wash on hover.
- **Card**: surface, hairline, radius 8, a heading and content; never a card inside a card.
- **Chip**: pill; selected = ink fill and ink text, rest = text_2 with hover wash. Tool strip,
  scenario picker, filters.
- **Tab**: text with a 2 px ink underline when active; counts as badges, not parentheses.
- **Badge**: soft tint and strong text of its tone, with a dot.
- **Table**: 22 px rows, striped with surface_alt, header in label style, figures right-aligned in
  tabular digits, selected row in selection.
- **Limit gauge**: 8 px track in hairline, fill in the standing colour, limit tick in text.
- **Charts**: thin marks, 2 px gaps, hairline baseline, labels in muted, values in text_2.

- **Attention list**: one surface, a heading row on `surface_alt`, then rows of dot, title, detail,
  age and a chevron; the first thing on the Overview, and a calm one-line empty state when nothing
  needs the user.
- **Command palette**: a `raised` panel over a dimmed page; one input, results grouped by eyebrow,
  the active row in `hover_wash`, key hints as `kbd` caps. Opened by Ctrl K or `/`.
- **Keyboard hint (`kbd`)**: `surface`, hairline border with a 2 px bottom, radius 4, 11 px medium
  in `text_2`.
- **Toast**: `raised`, hairline, radius 8, the popup shadow, a tone dot, a title, an optional detail
  and one link; bottom right above the status bar; gone after six seconds, ten for an error.
- **Skeleton**: a still block in `hover_wash` where text will be. Nothing shimmers; nothing moves
  on load.
- **Empty state**: a tone dot, a medium title, a small line that says why, and at most one action.
- **Sheet**: a form in the side column, under its own heading row, above the positions; the page
  behind it stays live. Used for the booking ticket instead of a modal.
- **Banner**: one line under the top bar on `warn_soft` while the live stream is reconnecting.
- **Hero strip**: the stat strip with 32 px values and the standing in words, for the numbers
  someone signs; a plain strip of the supporting figures sits under it.
- **Feeds pill**: one pill in the top bar with the worst feed's dot and a count of stale sources;
  the detail (each source, its age, its counts) is a popup.
- **Assumption group**: a collapsible `details` with an eyebrow summary, a one-line help text in
  `text_2` and, when closed, the count of inputs inside. An edited input shows the bank's value as a
  `warn_soft` ghost that puts it back.
- **Limit row**: dot, label, figure, "of" limit, an 8 px gauge, then the percentage used and the
  standing in words; the same row on the risk page, the ALM limits and the ALCO pack.
- **Breach run**: consecutive days a limit stood breached are one line (limit, span, days, worst,
  limit, sign-off), never a row per day.
- **Market check**: a small line under the ticket's rate with the market outright for the value
  date and the distance in basis points; beyond tolerance it turns `bad` and a confirmation appears.
- **Save box**: the one surface for a bank-wide write: the list of changes, the note, a full-size
  primary button named for the version it creates, and one sentence of consequence.
- **Stacked bars**: two series in one bar per category (hedged and unhedged, pledged and free,
  coupons and principal), a 2 px surface gap between the segments, a legend, the total on top
  where there is room.
- **Forecast bars**: the same bar chart with forecast categories at 40% opacity and a legend that
  names both styles; the tooltip says "forecast". Booked and projected never share one style.
- **Axis labels**: a chart thins its category labels to every second or third when they would
  collide, rather than overlapping them.
- **Tree table**: the positions register as one table: book rows on `surface_alt` in 600, bond rows
  indented with a twisty, lot rows indented further in `text_2` at 12 px. Books start open, bonds
  closed; "Show every lot" and "Bonds only" set the depth at once.
- **Detail overlay**: a `raised` panel with the popup shadow fixed to the right of the page, so the
  table under it keeps its width; closes with its own button. Used for a bond's lots, swaps, repos
  and cash flows.
- **Drop zone**: a dashed `border` box that washes on hover and turns `info_soft` while a file is
  over it; the preview below it lists errors (bad, with sheet and cell), warnings and changes before
  anything is stored.
- **Ticket sheet**: the PMU tickets use the detail overlay, 460 px wide: chips for the kind of
  trade at the top, a search that lists matches as plain rows, a `surface_alt` card naming the bond
  chosen, a label-and-field grid, then the server's preview as a right-aligned key-value list,
  blocking problems in `bad` and warnings in `warn` above one primary button named for what it books.
  Off-market needs a tick beside the button, as on the FX ticket.
- **Period picker**: preset chips (month, quarter, year to date, last 30 days) and two date inputs;
  the period lives in the address so a view can be shared.

## Motion

State changes only, 100 ms, ease out. Nothing animates on load.

## In the web client

The same system, in CSS and React. Every token above is a custom property in
`web/src/styles/tokens.css` (`--canvas`, `--surface`, `--text-2`, `--good-soft`, `--s1` to `--s6`
and so on), defined on `:root` for light, redefined for the OS dark preference unless the page is
stamped light, and again for an explicit `data-theme="dark"` stamp. The type scale is the `.t-*`
classes in `web/src/styles/base.css`; the components (stat strip, card, chip, tab, badge, dot,
table, gauge, popups) are `web/src/styles/components.css` with their React counterparts in
`web/src/components/`. Charts are plain SVG in `web/src/components/charts.tsx` with the same marks,
labels and series order. The desktop shell's first-run page uses the same tokens inline.
