# Country balance widgets

The shared widget behind [Political power](../political-power/README.md) and [Command power](../command-power/README.md): one recorded fixed-point balance per country over time, from the `country-measure` product. Each widget kind supplies a `MeasureConfig` (which measure, and how to name it); the look, settings (`options.ts`) and data helpers (`measureData.ts`) are shared, so a change reaches both.
