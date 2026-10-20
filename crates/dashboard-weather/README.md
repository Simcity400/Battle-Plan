# dashboard-weather

Prepared backend for Weather by region, above dashboard-base and dashboard-keyed.
The dashboard owns registration and the UI; this crate has no direct loader.

## Product and preparation

`products::regional_weather::RegionalWeather` (`regional-weather`) takes
`{branchId, firstHour?, lastHour?, regions?: string[],
show?: "conditions" | "modifier" | "temperature", modifier?: string, daily?: boolean,
view?: "history" | "provinces", region?: string, hour?: number}`. Region ids are canonical
unsigned decimal strings. An omitted selection includes all regions; an empty
selection requests no numeric series. `show:"conditions"` reads only flag names;
`show:"modifier"` requires a valid modifier id and returns only its names.
`show:"temperature"` returns only `|temp` names. Without
`show`, all measures are returned; a valid `modifier` is then unused. Region,
flag and modifier choices remain complete regardless of the series selection.
An auxiliary presence flag used to count a modifier refusal is never returned
in a modifier-only reply.

Prepare `dashboard_keyed::Scan<regional_weather::block::Observer>` with
`regional_weather::block::FAMILY`; the store is `regional-weather`.
WeatherRegions dates acquisition;
WeatherRegions and WeatherProvinces are observed independently. The watched fields
include every flag, raw modifier and witness, all four anchored temperature words
and the slot-zero temperature pass (seeds, suspended hours, cursor and checksum).
Province reads are watched even when equal: every reached hour rechecks that its
pass reaches that exact hour, rather than extending a stale packet as known.
Normal keyed block folding, quiet-hour supplying reads,
publication, continuation and accepted-prefix clipping apply unchanged.

Names (parts joined with `|`):

- `<regionId>|light_rain`, `heavy_rain`, `snow`, `blizzard`, `sandstorm`,
  `arctic_water`: independent recorded bytes. Zero is explicit; no precedence,
  dominant condition or subtraction. A missing region row has no flag runs.
- `<regionId>|mod:<modifier>`: signed raw fixed5 entries (100,000 units per unit),
  never effective country penalties. Modifiers are `air_detection`,
  `naval_detection`, `air_attack`, `air_defence`, `army_attack_factor`,
  `breakthrough_factor`, `army_speed_factor`, `army_attack_speed_factor`.
  Air detection needs its outer accumulator witness. The three map terms need
  `map_modifiers_known=true` and that scalar recorded. The four combat terms
  need the independent combat packet. Inside a successful witness, a missing
  key is explicit zero. Outside it, a missing run is unavailable, never zero.

- `<regionId>|temp`: the app's equal-province mean in **fixed5 degrees Celsius**
  (**100,000 raw units = 1 degree Celsius**, exported as `TEMPERATURE_SCALE`). The wire value
  is a signed decimal integer string, like the other keyed values. Sum the exact
  committed province words in `i128`, divide by the member count and truncate
  toward zero to fixed5 precision; no floating-point or game-aggregate claim.
  All member provinces must be known at that hour. Empty membership has no mean.
  A missing province/pass, incomplete table, content error, slot-count mismatch or
  failed checksum emits no temperature, never a partial average or held fallback.

Unavailable/truncated regional observations have no flag/modifier runs; complete
tables with no region row are absent. Temperature may remain independently known.
`window.known` spans hours where regional inputs or at least one exact temperature
were observed; individual missing temperature/modifier runs stay unavailable.
Known flags survive every independent modifier refusal.
No native per-record refresh date is invented: `window.reads` dates acquisition,
including carried rotating-window values, not when simulation changed a region.

## Reply

`{window, dayHours, regions:[{id,name}], flags:[{id,name}], modifiers:[{id,name}],
theatres?:[{id,name,regions:string[]}], temperature:string, warnings:string[]}`.
Every reply/nested wire type has TS declarations in `RegionalWeather::types()`.
`query(&dashboard_keyed::query::Query)` reads only selected region series and
returns the same shape when cold. The dashboard adds coverage/pending.

Without `daily:true`, the window retains exact coalesced **hourly runs**.
With it, the backend samples each series at each calendar day's last reached
**recorded** hour, using block/gap metadata, the requested last hour and accepted
preparation prefix. That value covers the whole calendar day clipped to the
requested/accepted window, including any earlier unrecorded hours that day.
The first day's sample may precede the requested first hour in the same day.
No values or reads extend into the pending tail. A wholly unrecorded day stays
a gap. Never search backward for known weather: an unknown endpoint leaves the
whole day unknown even when an earlier hour that day was known. Known-empty
region tables remain distinct from unknown weather and missing witnesses.

`known` and `reads` use the identical endpoint rule, with each day's supplying
read retained. Equal values on adjacent sampled days coalesce, including equal
reads; a missing day breaks a run. No daily mean or dense region-by-day matrix
is built. A single endpoint catalogue is reused for the selected series;
constant spans project in groups of consecutive days, not one value per cell.
The shared product reply-budget check retains the transport's 32 MiB limit.
Use `daily:true` and `show` for campaign-wide heat requests.

Daily sampling uses `dashboard_keyed::daily::Days::requested(...).sample(...)`
and its shared `day_start` helper. `dayHours` is `daily::DAY_HOURS`, the shared
`replay_data::cadence::STAGGER_PERIOD`, with the same Euclidean game-hour day
boundaries used by air-superiority. There is no new literal day length or
crate-local daily implementation. Shared keyed query helpers validate requests
and enforce cancellation and the complete-reply budget for both views.

Region definitions and names use installed-content-regions and localisation through the
verified recorded content stack, exactly the air-superiority naming path. Empty
localised names remain empty; missing names retain `Region <id>` with a counted
refusal. Condition names use the same native weather selectors as the map, from
that stack. Raw modifier titles use the reached recorded ModifierDefinitions
name key and argument through the shared text formatter. Missing names retain
clearly project-worded labels and counted warnings, without hiding numbers.
Region/name content successes and failures are cached per source/content/language.

Optional theatre choices are a **snapshot at the answered window's last known
weather hour** (before daily expansion), not historical/static region
ownership. One selected shared-cache frame supplies the recorded theatre/group/
wing joins and modifier title definitions. Complete tables prove empty theatre
membership; a broken join refuses the filter rather than publishing partial
membership. Names obey the recorded/display language rule. The numeric history
never decodes branch frames on request. The lead should use the existing dated
air-superiority choices if a historical theatre filter is required per cell.

## Exact temperature inputs and binding

`installed_content_regions::regions::read` reads effective strategic regions through
`ResolvedLayers::directory_files` and `read`: native directory/file order, folder
and archive layers, same-file overrides and `replace_path` are honoured. The
region's `native_provinces` vector removes duplicate/nonexistent province ids
using the content geometry's record roster. Overlapping region vectors each
retain the province; do not invert the province's final region backlink, which
selects only the last region for the simulation's period lookup.

`installed_content_weather::weather::temperature::read_with` supplies the shared exact
pass content: province slots, active geometry, ordered neighbours, terrain
ordinals/modifiers, weather settings/update-window size and regional periods.
Those success/failure results are cached once per source/content binding, using
the ordinary recorded-content verification/explicit unverified-content policy.
No authored content or simulation state is serialized into the catalogue.

One `replay_data::temperature::Memo` lives in each branch observer. `states`
starts from every dense slot's four anchored words plus slot zero's anchor hour,
cursor and pass entries. Each `Some(seed)` applies the exact shared native
fixed-point/wrapping pass; `None` preserves a suspended hour. The entire committed
temperature checksum must match before any aggregate is published. The recorded
anchors, seeds and checksum already supply all runtime inputs; **no recorder
change or new shared reader/accessor is needed**. The runtime memo is skipped by
serde and reconstructed from the next frame's existing anchor after resume.
Independent refusals are retained as named/count diagnostics and never hide flags.

**Content binding:** the observer implements `dashboard_keyed::Observer::bind`,
which the dashboard's keyed families call before every fold and block observation
(after a catalogue restore too) with the block's source and content. It is a no-op
for the same source/content binding. `Observer::bind(source, options, cancelled)`
is the flag-based equivalent for other drivers and tests. A reply whose observer
was never bound keeps the precise named missing-binding refusal and never claims
temperature available.
With a successful bind, `temperature:"available"` describes support; particular
hours/regions still lack `|temp` runs when their inputs refuse.

No night band or generic air penalty is emitted.

## Reached-hour province drill

`{branchId, view:"provinces", region:"<id>", hour}` uses the same product and
query entry point. `region` is a canonical unsigned decimal string; both region
and hour are required for this view. The hour must be within the branch.
Reply (no history window or preparation dependency):

`{view:"provinces", hour, readHour:number|null, region:string, provinces:[{id:string,
name:string|null, snow:string|null, mud:boolean|null, temperature:string|null}],
warnings:string[]}`. Both views' replies carry `view`, so the reply type is a
union tagged by the view that was asked for.

- `snow` is the recorded signed snow word, fixed5 (100,000 raw units per unit);
  neither the display snow byte nor snow plus permanent snow.
- `mud` is whether the game shows mud on the province, which it does whenever
  the recorded raw mud byte is nonzero (the viewer's province weather tooltip
  applies the same rule). The display mud byte never stands in for
  a missing raw byte.
- `temperature` is the reconstructed committed province value, fixed5 degrees
  Celsius, from the same checksum-validated temperature binding the observer
  uses. Each source keeps one drill memo per branch, reset when the content
  binding changes; a drill never alters prepared history. A missing pass, slot,
  checksum, table or temperature content nulls only temperature.
- Snow and temperature are exact decimal integer strings.

Membership comes from the recorded layered regions and province geometry through
the temperature path's province helper, independently of temperature settings,
so mods and replacement paths apply unchanged. A failed membership read or an
unknown region returns an empty roster with a named, counted warning; a region
that resolves empty has none.

One shared-cache frame request reads province weather and the place-name tables
at exactly the requested hour. The drill never scans the branch, reads another
block to fill an operand, falls back to an earlier hour or clips to the prepared
prefix. Without a usable frame the member ids come back with null fields and a
counted warning. `readHour` is the table's supplying read, or null when the frame
cannot prove one. Names use the shared at-frame place-name reader; an unresolved
name is null without discarding any other field. An incomplete weather table or
an absent row leaves snow and mud unknown, never zero. Each field otherwise
refuses on its own.

## Checks and integration

Run only `cargo test -p dashboard-weather` with
`CARGO_TARGET_DIR=lane-airnavy`. The mod/content test initializes
runtime metadata from `BATTLE_PLAN_GAME_DIR` and uses only synthetic layered fixtures;
the installation is read-only. Tests cover overlapping bytes, all independent
witnesses, known missing keys, signed extremes, absent/unknown, daily endpoints,
gaps, reads, flush/resume, prefix clipping, mod replacement/names, theatre joins,
cold contract, validation/cancellation, measure selection, daily known/read
sampling, unknown-only days, coalescing, negative hours and integer edges,
cached auxiliary reuse and 5,000 regions. Temperature adds complete/unknown
member averages, wide signed/rounded fixed5 arithmetic, actual layered membership
and replace-path overrides, checksum/pass refusals, unchanged-pass freshness,
suspended hours, branch
isolation, bound queries/daily/prefix/continuation, cached verified-content reads,
independent flags and a 5,000-region temperature inventory. A quiet ten-year
campaign with 5,000 regions stays compact without materializing daily values per region.
Province-drill regressions cover mod membership/localised names and replace paths,
raw snow/mud versus display values, unknown versus zero, independent field/content
refusals, exact-hour/shared-block-cache reads without history preparation, gaps,
parameter validation, nested wire types and cancellation.

The lead must add the crate dependency, register Product/types/content checks (`content_warnings` reuses the actual
region/condition/temperature content readers; modifier titles/theatres and
temperature pass evidence require recording rows),
register Keyed<block::Observer>/FAMILY and its refusal gate, route and cache the
`regional-weather` query and mark its prefix. No UI or shared registry is edited
here. No new capture or game-file writes are needed.
