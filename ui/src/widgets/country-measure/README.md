# Country balance widgets

The shared widget behind [Political power](../political-power/README.md) and [Command power](../command-power/README.md): one recorded fixed-point balance per country over time, from the `country-measure` product. Each widget kind supplies a `MeasureConfig` (which measure, and how to name it); the look, settings (`options.ts`) and data helpers (`measureData.ts`) are shared, so a change reaches both.

Country choices use the shared selector without a smaller widget-specific limit.
Multi-country tooltips use shared viewport fitting, counting rows that do not fit
and preserving the complete read-out. Names and membership badges keep the
chart's existing sampled names. Command power keeps its established maximum-row
cutoff and dates carried maximums in the note when their rows are not listed.
