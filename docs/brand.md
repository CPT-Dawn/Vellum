# Vellum brand

Midnight Bloom: overlapping sheets of smoked teal glass on a near-black
canvas, lit by one neon-mint stroke. Keep it quiet, dark and precise. The
mint is the only thing that glows.

## Assets

| File | Use |
| --- | --- |
| [`assets/brand/icon.svg`](../assets/brand/icon.svg) | App icon, avatar, AUR, crates.io, favicon. Square, self-contained tile, legible at 32 px. |
| [`assets/brand/lockup.svg`](../assets/brand/lockup.svg) | Icon plus `vellum` wordmark, horizontal. Transparent background. Dark surfaces only. |
| [`assets/brand/banner.svg`](../assets/brand/banner.svg) | README header, 1280×400, on the midnight-canvas gradient with the tagline. |

The wordmark is lowercase `vellum` set in Inter Display Medium (SIL OFL)
and converted to outlines, so the SVGs render the same without the font
installed. The tagline is Inter Regular, also outlined.

## Palette tokens

| Token | Hex | Role |
| --- | --- | --- |
| `ink` | `#0B0D14` | Deepest background; TUI base |
| `canvas` | `#0E1019` | Midnight-canvas gradient midpoint; panels |
| `canvas-hi` | `#1A1B2E` | Gradient top; raised surfaces, overlays |
| `glass` | `#4E9EA2` | Smoked teal glass, used translucent (40–55% alpha) |
| `glass-deep` | `#1B4549` | Lower end of the glass gradient |
| `mint` | `#8DE3DC` | The single focus highlight and the logo stroke |
| `mint-hot` | `#E6FFFC` | Highlight core inside the glowing stroke only |
| `blue` | `#7AA2F7` | Selection (Tokyo Night blue) |
| `text` | `#C0CAF5` | Primary text and the wordmark (periwinkle) |
| `subtext` | `#A9B1D6` | Secondary text, metadata, tagline |
| `muted` | `#565F89` | Borders, disabled items, inactive hints |
| `rose` | `#F7768E` | Error |
| `orange` | `#FF9E64` | Warning |
| `green` | `#9ECE6A` | Success |

The midnight-canvas gradient runs top-left to bottom-right:
`#15162A` → `#0E1019` (55%) → `#090A10`, with soft radial blooms of `mint`
(about 13% alpha) and `blue` (about 12% alpha).

## TUI theme roles

Midnight Bloom is the default theme on truecolor terminals.

| Role | Token | Notes |
| --- | --- | --- |
| Background | `ink` | Leave unset under `theme = "terminal"` |
| Panel / overlay background | `canvas-hi` | Help overlay, palette, Rotation panel, toasts |
| Border, inactive | `muted` | |
| Border, focused pane | `mint` | |
| Focus (cursor, active input, search match) | `mint` | The only mint on screen at a time, ideally |
| Selection (selected list row, targeted Monitor) | `blue` | Always paired with bold or reverse; never color alone |
| Primary text | `text` | List items, headings |
| Secondary text | `subtext` | Metadata strip, footer key hints, header details |
| Dim text | `muted` | Disabled actions, placeholders |
| Key glyph in hints | `mint` | Description in `subtext` |
| Error | `rose` | Toasts, inline preview errors |
| Warning | `orange` | |
| Success | `green` | "Applied to DP-1" toasts |
| Rotation running indicator | `green` | Stopped shows in `muted` |

On 16- or 256-color terminals, fall back to the nearest ANSI colors: cyan for
`mint`, blue for `blue`, white for `text`, bright black for `muted`, and red,
yellow and green for error, warning and success. With `NO_COLOR`, use
bold, reverse and underline only.

## Glow

The glow belongs to the mint `v` and nothing else. It is built in four
layers, bottom to top:

1. `mint`, wide stroke (14 units on a 128 grid), Gaussian blur σ 5, 45% opacity.
2. `mint`, 10 units, blur σ 1.6, 60% opacity.
3. `mint`, 8 units, solid core.
4. `mint-hot`, 2.4 units, 85% opacity highlight.

Round caps and joins throughout. Do not add glow to the wordmark, the glass
sheets or UI text. In the TUI, glow is implied by the single mint focus, not
imitated with extra characters.

## Usage rules

- Use the icon tile as is. Don't recolor, rotate, crop the tile, or remove the sheets.
- Keep clear space around the icon and lockup of at least a quarter of the icon's width.
- The lockup is for dark backgrounds (`ink`, `canvas` or darker). On light backgrounds use the icon alone or the banner.
- Minimum sizes: icon 16 px (32 px preferred), lockup 120 px wide.
- Don't retype the wordmark in another font or weight; use the outlined SVG.
- Rose, orange and green mean error, warning and success. Never use them decoratively.
- The full Tokyo Midnight Bloom design document is the maintainer's and is not committed. This page is the condensed reference.
