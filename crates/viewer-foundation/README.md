# viewer-foundation

Shared contracts for the native map viewer: GUI layout and lifecycle, windows and atlases, text and tooltip drawing, texture upload and GPU targets, camera and province sampling, visibility, recorded-table access and bounded workers.

The crate also owns shared engagement and retained-icon history reads, shared recorded movement queries, selected-neighbour and province-clock reads, movement timing and the hourly ETA cache, province geometry, terrain and survival arithmetic, wing range and aircraft model selection, unit handle namespaces, air participant/reach/sortie joins, settled unit and task-force motion, occupant order, label names/anchors, common ribbon content and mesh data, building facts and authored trigger operands. Immutable trigger names are shared across completed feature snapshots. Counter, combat, raid and world callers retain their own admission and presentation policies. Shared drawn counter/selection observations, authored GUI fallback, attrition and port strike-status operands, counter input, action, status, railway-gun and partial naval-count folds serve grid rendering and hover text from the same recorded operands.

Content comes from installed-content and recorded tables from replay. The crate has no dependency on viewer or its feature implementations. The host owns ordered asset loading, aggregate view state, documents and control of the recording-wide engagement pass.

Finalized stack-anchor lookup is shared by counter grids and selected movement products, preserving authored slots, victory-point inheritance and seeded defaults.

Synthetic GUI, table, raster, localisation and GPU fixtures used by other viewer crates are available through `test-support`. `slow-tests` preserves the gating of installation and GPU tests. Follow the workspace [working rules](../../AGENTS.md).
