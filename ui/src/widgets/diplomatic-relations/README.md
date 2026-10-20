# Diplomatic relations

**What it shows:** Every chosen country's total opinion of every other at the shown hour, as a matrix (rows hold the opinion of the columns), both directions independent, on a signed colour ramp from theme grey over the recorded extremes. A cell opens that pair's opinions of each other over the campaign, which pins as its own widget ("Diplomatic relations: Pair history").

**Data it reads:** `diplomacy-state` (`view: 'opinions'`) at the shown hour and (`view: 'pair'`) for a pair's history. Drawn with the heat kit's [matrix mode](../../charts/heat/README.md#matrix).

**Settings:** Countries (default: the Human players group; any selection, no cap); **Order**: by faction then name, or by name; Show (Matrix or Pair history); Breakdown opens. Moment widget: Follow or Pin.

**Notable behaviour:**

- A known owner's opinion of a country it does not list is exact zero; an owner whose read cannot prove its state leaves its row hatched as unknown. Self-pairs are recorded state.
- There is no fixed opinion range: the colour domain is the recorded values' own extremes, and the exact signed value is in the tooltip. No modifier breakdown is shown: none is recorded.
- A pair history names and colours its two countries as they were at the hour the pair was chosen, read at that hour itself, so a later holder of either tag never lends its look. A name still being read says so; one the recording lacks reads unavailable.
