# 1301. Drive entity lights with the played event clock

An entity's light reads authored curves on the event clock and applies the event transform to the resulting position. A hidden event emits no light. The render pass combines linear RGB and evaluated intensity. All registered light definitions come through the content descriptor roster.

Light and weather curves share the 64-sample Catmull-Rom lookup table with extrapolated virtual endpoints and `minValue`/`maxValue` remapping, zero-filling tables with fewer than two knots, normalizing non-positive durations to 1.0, wrapping repeating times modulo `start + duration`, matching `life_abs`, `life`, and `system` time modes case-sensitively with any other scalar string keeping the default `spawn` birth-time mode, and applying case-insensitive `add`, `mul`, and `abs` operations. Unsupported curve operations, malformed or nested curve operands, non-finite remap or knot scalars, odd-count knot operands, or non-increasing knot domains fail with a named refusal; on entity lights, the affected light is withheld with its named refusal so valid lights and the replay map still open.

Light parameters accept both joined and spaced curve references. The shared parser returns the spaced curve name as the next anonymous scalar, so the light reader takes that operand when the value ends at the comma.

[Entity light loader and evaluation](../../crates/viewer-weather-effects/src/entity_effects.rs) · [Shared curve table](../../crates/viewer-weather-effects/src/weather_effects.rs)
