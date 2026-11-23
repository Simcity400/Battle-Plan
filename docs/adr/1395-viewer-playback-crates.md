# 1395. Viewer playback crates

## Decision

Layer implementations compile in the counters, forces, political and infrastructure crates, with their shared playhead host and layer contracts in `viewer-layer-core`.
The core owns the map entry registry, visibility handoff and scene content needed to install a layer's products.
The playback crate assembles the layers from the document and supplies their sources.
An `Overlaying` object lets the host drive the overlay at the existing points in a frame, a whole-hour catch-up and a reset.
The overlay gathers the layer inputs in their existing order, and joins and hands its counter products back before the same layers settle.
The host is generic over the combat reader and overlay products so that layer tests can drive the same host without linking the concrete overlay or combat window.
