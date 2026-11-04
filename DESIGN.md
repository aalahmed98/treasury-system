# NBB Treasury client: visual system

The egui client reads this file as its design system. Tokens live in `crates/client/src/ui/theme.rs`
(`Palette`, one per theme) and are applied to egui's `Style` once at start-up; feature code never
holds a colour or a size of its own.

## Scene and theme

An ALM analyst at NBB head office at 08:30, two 24-inch monitors under office light, Excel open
beside the client. Light is the default and the primary design. Dark is a selected set for the
dealing room and evening work: warmer and lighter than egui's default dark, with depth from surface
lightness rather than shadow.

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

One family: Inter (OFL), bundled as `crates/client/assets/fonts/InterTreasury-*.ttf`, a static build
with tabular figures on by default so every column of numbers aligns. Three weights carry the
hierarchy (regular, medium, semibold); egui's `strong` is not a weight, so weight is chosen by
family.

| Style | Size | Weight | Use |
|---|---|---|---|
| hero | 32 | semibold | the one number a tool is about |
| title | 20 | semibold | page title |
| stat | 24 | semibold | stat-strip values |
| heading | 15 | semibold | card and panel titles |
| body / button / monospace | 13 | regular / medium / regular | text, controls, figures |
| label | 12 | medium | table headers, form labels |
| small | 11 | regular | metadata |
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

## Motion

State changes only, 100 ms, ease out. Nothing animates on load.
