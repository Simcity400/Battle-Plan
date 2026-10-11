# Fuel

## What it shows

Fuel level/capacity per country, with separate drills for current required consumption and completed-day gain/burn accounting.

## Products it reads

[`country-fuel`](../../../../crates/dashboard/src/products/country_fuel.rs), [award history](../../../../crates/dashboard/src/products/country_fuel/detail.rs) and [demand detail/history](../../../../crates/dashboard/src/products/country_fuel/demand.rs).

## Settings

Countries, campaign/follow/pin/range; playback/window/axes, gain/burn strip, capacity, fill, end labels, drill placement and Show.

## Notable behaviour

- Current required rates differ from completed-day debits/receipts. Keep native caches as stale as the game. Signed/zero rows remain selectable; geometry never proves a complete census.
- Days cover updates before their supplying read, with overlaps excluded. At capacity the game discards excess, so gain minus burn need not equal level change. Comparisons hide the strip.
- Preserve scales and BigInt fractions. Incoming lend-lease stays outside own-production gain; Other is inside burn. Actual attribution and legacy fallback retain different known parts; missing historical labels use plain unavailable names.
- The consumption drill groups selectable armies, fleets and air bases in a disclosure, with a separate By military branch disclosure. Demand keeps native roster occurrences/topology, including repeated/refused air sites. Keys bind original read, country/instance and occurrence; renames retain detail only while these match. Gaps, absent occurrences and resets stay unknown. Load wanted windows only.
- Picks bind a native country through its aliases; overlap choices hold the clicked date. Picking holds; Follow uses the clock. Pin saves actor, level/occurrence or recipient, basis and history extent; its header uses the displayed drill hour.
- Label refresh retains numbers/picks while masking old labels; source, actor or numeric changes invalidate it.

## Files

| File | Responsibility |
|---|---|
| [FuelWidget.tsx](FuelWidget.tsx), [fuelData.ts](fuelData.ts) | Overview, day accounting and exact values. |
| [FuelBreakdown.tsx](FuelBreakdown.tsx), [fuelDrill.ts](fuelDrill.ts), [fuelCountry.ts](fuelCountry.ts), [FuelCountryChip.tsx](FuelCountryChip.tsx) | Awards, pins and dated header. |
| [FuelConsumptionBreakdown.tsx](FuelConsumptionBreakdown.tsx), [FuelConsumptionStack.tsx](FuelConsumptionStack.tsx), [fuelConsumption.ts](fuelConsumption.ts) | Demand drill and history. |
| [definition.ts](definition.ts), [index.ts](index.ts) | Settings and registration; adjacent tests. |
