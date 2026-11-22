# 1393. Viewer render crates

## Decision

The viewer builds each render layer in its own crate, with `viewer-world` composing
the passes and retaining scene state. `viewer-scene-base` owns shared shader
bindings, the world pass, map wrap calculations, the camera margin and the
projection ID allocator. Terrain owns the choice of which terrain modes read
the shadow buffer. The order pass takes the scene's interface flag as a boolean,
and the border passes read `Regions` from `map-display-data`. Unit history owns
the mesh facing offset used by both motion history and model placement. Render
crates depend on the layers they consume, and cross-layer tests live with the
consumer that can exercise both sides of the interface.
