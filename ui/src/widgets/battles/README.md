# Shared battle rows

`BattleLog` composes the style-5 Event feed (`charts/log/LogList`) and Scoreboard
(`charts/table/Table`). It accepts caller-supplied wording, side labels, measures,
country looks, empty text and menus. It contains no domain-specific facts.
`onVisibleRows` reports drawn rows for lazy names and flags, including countries
dated independently inside one row. The caller filters countries and time before
rendering through the dashboard country matching helpers. A server-ranked top N
may use `groups.countryMatchFilter` before ranking; logs filter locally.

The wire shape is owned by `products/battles.rs`. `battleRows.ts` imports and
re-exports the generated dashboard types; its validators check those types:

```ts
BattleRow {
  id: string;
  place: { name: string | null; hour: number };
  start: { hour: number; bound: 'exact' | 'by' } | null;
  end: { hour: number; bound: 'exact' | 'by' } | { unknown: string } | null;
  sides: BattleSide[];
  winner: { side: number } | { unknown: string };
}
BattleSide {
  label: string;
  countries: { tag: string; hour: number }[];
  countriesUnknown?: string; // caller-supplied reason for an incomplete country list
  counts: BattleValue[];
  losses: BattleValue[];
}
BattleValue = { label: string; value: number; atLeast?: boolean }
  | { label: string; value: null; unknown: string };
```

The same shared module owns `BattleWindow { firstHour, lastHour }` (recording
hour bounds) and `BattleWarning { hour, reason, count }`. Land and Naval import
these generated types; there are no Land-specific copies of either shape.
The lead's next regeneration should replace the stale `LandBattleWindow` and
`LandBattleWarning` names with these two shared exports.

A null start means start not recorded; a null end means ongoing. An unknown
end keeps the battle visible with the caller's reason (for example, Ending not
recorded), without claiming it remains ongoing. Dates with `by`
stay upper bounds in tooltips and row text. The log dates and sorts end events by
their observation hour. The table date shows the start, or the known end when the
start is missing, in the log kit's short format. Live pins ongoing rows
first and reuses the shared `live-dot` pulse, respecting reduced motion.
Right-click Jump uses `battleJumpHour`: start, else `place.hour`; an absent-read
end is never a useful Jump destination. Stable row IDs preserve keyboard focus and tooltips as rows move; the kits own scrolling,
motion, highlighting, menus, rank markers and bars.

Biggest-battles choices use the caller's count/loss label as a string key, never
an array index. `battleMeasure` sums that label across every side; a missing side
value keeps the sum unknown. Count and loss labels must be unique across both
lists of each side; ambiguous duplicates refuse the row. `BattleMeasure` can
instead supply a caller's own measure and format. Values are finite numbers,
including fractions (Rust `f64`, TypeScript `number`). Land roster counts fit
exactly in that domain. Length is supplied by each domain product; the shared
module assumes no duration convention for `place.hour`. No helper derives an outcome.

`battleTooltip` lists every supplied country, participant-uncertainty reason, count and loss, its dated
bounds and the supplied winner. The shared tooltip fitter retains the complete
accessible description when the visible tooltip must be shortened. Country
facts are requested only through `battleCountryRequests` for visible rows;
opaque tags and separate observation hours remain distinct.

The Rust wire values live in `crates/dashboard/src/products/battles.rs`, so no
consumer imports types from a domain product. `BattleMeasure.format` receives
both the value and its row, allowing a caller to date a peak without putting
that convention in the shared module. The log sorts only here, newest first;
products supply unsorted histories. The table ranks the caller's supplied set.

`place.hour` dates only the place name: callers may use a stable naming hour such
as battle start. Every country's own `hour` still dates that fact separately.
`countriesUnknown` means the side's country list is incomplete, even when known
countries are also present; it is rendered in the row and tooltip. A caller
filtering by country must keep such a battle because it may involve that country.
Land-specific `readHour` dates its counts without overloading the naming hour.
`BattleLog` enables the log kit's scroll anchoring, so releasing an off-screen
recording window keeps the first visible row in place.

`BattleLog.note` can be a string or a row function, so a caller can date the
read supplying its counts separately from the stable place-name hour.

`BattleLog.formatLoss(value, row, side, tooltip)` lets callers format dated or
provisional losses in both views and their tooltips without extending the wire
row. `BattleMeasure.tooltipFormat` can give a measure its fuller dated text;
without either hook, existing value formatting is preserved. The caller owns
loss read hours and the meaning of terminal values. Neither hook changes labels
or numeric ranking values.

`BattleLog.compactSides` supplies the count label, separator and pluralised count
formatter for a single flag/count line. Flags fit the measured available width;
`+n` represents the remaining flags, with the complete list in the tooltip. No
participants are dropped. Without this option the detailed side renderer remains
available. Log rows keep measures in their tooltip. Table cells show short numeric
values or a dash for unknown values, with their reason and fuller measure text in
tooltips; `BattleMeasure.hint` supplies a header explanation. The Battle column
flexes, while the kit measures date and numeric columns from their content.
