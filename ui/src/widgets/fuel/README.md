# Fuel

## What it shows

Fuel level/capacity per country, with separate drills for current required consumption and completed-day gain/burn accounting.

## Products it reads

Prepared series queries for the [`country-fuel` family](../../../../crates/dashboard/src/products/country_fuel/prepared.rs), plus bounded [award detail/history](../../../../crates/dashboard/src/products/country_fuel/detail.rs) and [demand detail/history](../../../../crates/dashboard/src/products/country_fuel/demand.rs) products.

## Settings

Countries, campaign/follow/pin/range; playback/window/axes, gain/burn strip, capacity, fill, end labels, fuel decimals, drill placement and Show. Fuel is written in whole numbers unless Fuel decimals asks for up to 5 places ([fuelData.ts](fuelData.ts)).

## Notable behaviour

- Fuel and consumption charts read prepared width summaries and exact reached hours. One all-country observer prepares both. Completed-day accounting merges the observer's final 23 unsettled hours, so an appended daily-slot read can suppress an earlier off-slot day across a commit. Details retain their supplying-block and 720-hour window reads and obtain summary facts from prepared data.
- Dense gain/burn strips retain each summary bin's maximum and its supplying date. Hourly strips pass every supplying day through the existing strip renderer.
- Exact reads share reached and viewport hours, while each country's summary end dates stay local to that country. A reply missing requested dates stops the state or parity read instead of filling missing dates or continuing through stale pages.
- Unprepared fuel/capacity lines use the shared faded dashed drawing and become exact when zoomed to thirteen blocks. Exact reached amounts keep integer precision; completed-day accounting, consumption provenance and supplying-read navigation wait for accepted preparation.
- Mounted scalar summaries and reached reads refresh when accepted preparation advances. Exact picks remain independent of approximate chart geometry.
- Current required rates differ from completed-day debits/receipts. Keep native caches as stale as the game. Signed/zero rows remain selectable; geometry never proves a complete census.
- Days cover updates before their supplying read, with overlaps excluded. At capacity the game discards excess, so gain minus burn need not equal level change. Comparisons hide the strip.
- Preserve scales and BigInt fractions; only the written fuel rounds to the chosen decimals, while totals, shares, bars and order use exact amounts. A single army, fleet or air wing's own daily receipt or current consumption stays exact, since a fraction of one fuel would read as 0 or 1. Incoming lend-lease stays outside own-production gain; Other is inside burn. Actual attribution and legacy fallback retain different known parts; missing historical labels use plain unavailable names.
- The consumption drill groups selectable armies, fleets and air bases in a disclosure, with a separate By military branch disclosure. Demand keeps native roster occurrences/topology, including repeated/refused air sites. Keys bind original read, country/instance and occurrence; renames retain detail only while these match. Gaps, absent occurrences and resets stay unknown. Load wanted windows only.
- Picks bind a native country through its aliases; overlap choices hold the clicked date. Picking holds; Follow uses the clock. Pin saves actor, level/occurrence or recipient, basis and history extent; its header uses the displayed drill hour.
- Label refresh retains numbers/picks while masking old labels; source, actor or numeric changes invalidate it.

## Files

| File | Responsibility |
|---|---|
| [FuelWidget.tsx](FuelWidget.tsx), [fuelData.ts](fuelData.ts) | Overview, day accounting and exact values. |
| [FuelBreakdown.tsx](FuelBreakdown.tsx), [fuelDrill.ts](fuelDrill.ts), [fuelCountry.ts](fuelCountry.ts), [FuelCountryChip.tsx](FuelCountryChip.tsx) | Awards, pins and dated header. |
| [FuelConsumptionBreakdown.tsx](FuelConsumptionBreakdown.tsx), [FuelConsumptionStack.tsx](FuelConsumptionStack.tsx), [fuelConsumption.ts](fuelConsumption.ts) | Demand drill and history. |
| [preparedFuel.ts](preparedFuel.ts) | Prepared queries, bounded chart geometry, exact reached reads and browser parity values. |
| [definition.ts](definition.ts), [index.ts](index.ts) | Settings and registration; adjacent tests. |
