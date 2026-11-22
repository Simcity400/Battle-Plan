# 1305. Draw terrain at the recorded game's block detail

The viewer runs the terrain effects through the compiled content pipeline. A map whose dimensions cannot be tiled by the shader's 64-cell blocks is refused by name. The terrain cover is also the source of the map copies drawn by the other world layers.

## Cover and coasts

Select the coarsest block step allowed by the distance to the group's nearest corner at the top of the height range. Each group can coarsen only when all of its constituent blocks are far enough. The squared distance separates by axis, so each axis is evaluated once and the block distances are reduced in a pyramid. Partial groups at the map edge reduce only their own cells. At the whole-map view the game reaches step 16. At lower views the camera height, as well as horizontal distance, affects the step.

Cull map copies by intersecting their band of ground and object heights with the view volume. The far-plane reach alone includes copies that the view never touches. Keep the eye's own copy when no copy intersects so the frame has a scene block. Cull individual blocks against the same view and include the terrain-height margin needed by the lowest permitted map camera.

At a coarse step, one land sample could lift a whole sea cell above the water plane. Use the highest stored sample below the water plane as a cap for vertices that touch a cell containing water. The water effect then controls the per-pixel coast. Keep the finest blocks unchanged. Every drawn block touching a rim vertex uses the rule of the coarsest incident step, unless a finest block touches it. Shared vertices consequently have the same height. The east-west wrap keeps the sampled height because the original renderer does not snap across that edge. The cap follows the effect actually drawn, including when a visual substitution is refused.

## Shader and buffer contract

The content vertex stream uses signed 16-bit components for position, edge and direction. Height samples are signed 16-bit pairs, with the heightmap's top-down rows converted to the bottom-up terrain grid and the byte scaled by ten. The second height gives the vertex reached by an odd vertex on a snapped edge. The constant block starts with the shadow matrix and carries the origin, step and edge switches in south, north, west, east order. Overlay passes use the same draw's height stream and base vertex so they stand at its rendered height.

Prepare CPU height samples and GPU index shapes at load. A slab is bound as vertex data and as storage so a draw can instance patches that share both an index buffer and a slab. Group slab fill work into batches while builders are available. At a new cover, retain coast rims whose local neighborhoods did not change and reuse their slots before writing missing rims. Reuse a wrap view's bindings across cuts only when the block list and the identities of all reached rim slots in the same stream agree. [Terrain cut retention](1391-terrain-draw-reuse-across-cuts.md).

## Render reuse

Build each terrain technique's pipeline for its attachment formats and sample count. The receiver writes the shadow screen buffer, and applicable lit and underwater techniques sample it. If a visual replacement compiles but cannot be rendered, refuse it by name and rebuild all terrain modes against the recorded effect so their outputs belong together.

For recurring draws, retain bindings against persistent buffers and update the changed constant ranges. A render bundle is built only after the same draw list stays still long enough to amortize recording it. The pass supplies the blend constant before replaying a bundle.

The retained sea-bed target can be reused only when the compiled program's read constant bytes, sampled texture contents, block cover and destination match the last submitted pass. A texture with no described writer leaves the key unformed and causes a draw on every frame with a named reason. Commit the key after submission so a lost frame cannot certify the target. A skipped draw still counts as using its bindings, preventing an idle period from evicting the resources needed by the first changed frame.

[Terrain cover](../../crates/viewer-terrain/src/terrain.rs) · [Coastal caps](../../crates/viewer-terrain/src/terrain_coast.rs) · [Terrain pipeline](../../crates/viewer-terrain/src/terrain_shader.rs)
