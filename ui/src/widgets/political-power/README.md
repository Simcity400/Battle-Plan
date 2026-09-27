# Political power

Each country's political power balance over time, one line per country, with the hours its balance fell. The first widget built on the shared chart kit ([charts](../../charts/README.md)); it is the pattern for the other line-chart widgets.

## Data

- **Product `political-power`** (`crates/dashboard/src/products/political_power.rs`): `{branchId, tags ≤ 16}` → each country's balance as runs of the exact recorded fixed-point integer (`Countries.political_power_raw`, `decimals` places), and its net drops.
  - Runs break at a gap, a missing value (never zero) and a change of country identity.
  - A drop is a fall between two consecutive recorded hours of the same country, never across a gap.
- **Product `country-styles`**: each country's engine-built name and recorded map colour. It is read once per 30 game days during playback.
- **Product `country-groups`**, when the widget shows a group.

## Look and behaviour

The signed-off look is the balance line from mock-ups politics-1/2:
- a 2px line, with a light area fill when one country is shown;
- the exact current value in the header;
- spend ticks along the bottom;
- names and values at the line ends.

Hovering near a tick snaps to it. The tooltip lists each country's exact balance and any net spend that hour.

The chart kit supplies the rest: zoom that holds during playback, axis drags, live hover, and Jump to on right-click.

A group counted at each hour draws a country only while it belongs, with join (+) and leave (−) badges along the top in the country's colour.

## Settings

- **Time:** Live, Whole campaign, Pinned or Date range. While following playback the chart can grow to now (default), show the whole campaign with the playhead, or show a recent window.
- **Options:** spend ticks, area fill, names at line ends.
- **Axes:** value axis from/to, log scale.
- **Countries:** up to 16, or a group.

## Not recorded

- Daily political power income and what it was spent on need new capture (spec, 🆕). Until then spends are **net** falls in the balance, and the widget has no drill-down.
