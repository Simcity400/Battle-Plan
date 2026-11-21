# 1301. Drive entity lights with the played event clock

An entity's light reads authored curves on the event clock and applies the event transform to the resulting position. A hidden event emits no light. The render pass combines linear RGB and evaluated intensity. All registered light definitions come through the content descriptor roster.

Light parameters accept both joined and spaced curve references. The shared parser returns the spaced curve name as the next anonymous scalar, so the light reader takes that operand when the value ends at the comma.

[Entity light loader and evaluation](../../crates/viewer-world/src/map/entity_effects.rs)
