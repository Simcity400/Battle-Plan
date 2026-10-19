# viewer-foundation

Air range consumers share the current wing/ace/country range law. Its frame-backed source indexes canonical country terms and ace handles lazily once per preparation. Equal ace observations share a handle; conflicting owners or operands refuse that handle's range.

Enemy template display-name resolution requires an admitted settings payload for each candidate's recorded roster position. An explicit acquisition refusal yields a named refusal rather than guessing first-match order.

Shared contracts for the native map viewer: GUI layout and lifecycle, windows and atlases, text and tooltip drawing, texture upload and GPU targets, camera and province sampling, visibility, recorded-table access and bounded workers.

The [camera](src/map/camera.rs) is the game's look-at camera or a [free pose](src/map/camera/pose.rs): an eye with heading, pitch, roll and its own lens. Every camera answers the same questions: its matrices and rays, the ground its [view volume](src/map/camera/frustum.rs) reaches within a depth and band of heights, its six planes, and map-icon projection, which drops anchors behind a free eye. [Province admission](src/map/province_admission.rs) adds a free eye's near plane and a far plane at its object range (the content's `DRAW_MAP_OBJECTS_CUTOFF`) and takes its seam views from the copies it reaches; [picking](src/map/province_pick.rs) walks a free eye's ray through the height cells; [`HeightField::surface_height`](src/map/province_map.rs) is the drawn terrain's surface for anything standing on it. Map icons stand in a free eye's world ([free_icons.rs](src/map/free_icons.rs)): owners tag their quads with the icon's anchor and its kind's cutoff, and the presentation reads each icon's distance (`Camera::icon_distance`, the game camera's height otherwise), tests one sight line to its anchor over the drawn surface ([terrain_sight.rs](src/map/terrain_sight.rs), block maxima walked down to the surface only where the line passes close), and ranks, scales and fades the icons; the [sprite pass](src/map/sprites.rs) draws them nearest last, scaled about their anchors, and draws a fading icon offscreen to blend it in as one image ([sprites/fade.rs](src/map/sprites/fade.rs)).

The crate also owns shared engagement and retained-icon history reads, shared recorded movement queries, selected-neighbour and province-clock reads, movement timing and the hourly ETA cache, province geometry, terrain and survival arithmetic, wing range and aircraft model selection, unit handle namespaces, air participant/reach/sortie joins, settled unit and task-force motion, occupant order, label names/anchors, common ribbon content and mesh data, building facts and authored trigger operands. Immutable trigger names are shared across completed feature snapshots. Counter, combat, raid and world callers retain their own admission and presentation policies; a map mode's choices for them arrive as one `mode_visibility::Visibility`, with every mode's `NMapIcons` priority tables. Shared drawn counter/selection observations, authored GUI fallback, attrition and port strike-status operands, counter input, action, status, railway-gun and partial naval-count folds serve grid rendering and hover text from the same recorded operands.

Content comes from installed-content and recorded tables from replay. The crate has no dependency on viewer or its feature implementations. The host owns ordered asset loading, aggregate view state, documents and control of the recording-wide engagement pass.

[Resource origins](src/resource_origins.rs) answer what the resources map mode reads of an hour: which states carry a resource icon, whose origin each resource reads under resource rights, and each resource's nominal and delivered amounts from the recorded origins, rights, oil factors, temporary resources and the shared state resource operands; the resource icons and the resources mode's hover both use them.

Finalized stack-anchor lookup is shared by counter grids and selected movement products, preserving authored slots, victory-point inheritance and seeded defaults.

Synthetic GUI, table, raster, localisation and GPU fixtures used by other viewer crates are available through `test-support`. Follow the workspace [working rules](../../AGENTS.md).

Stored modifier source labels recover through their recorded language's content keys in `recorded_labels`; combat and occupation hovers share the same display-language policy.
Map visibility reuses replay's recorded country access and intel fractions,
including the reserved-country packets, with equipment detail queries. Province
fog geometry and map-specific admission remain in this crate.

Deployed panel and counter explanations share held-membership, effective source-tree, state-context and native mode-zero war operands. Border enemy power is shared by province explanations and fresh deployed attrition. Shared content loading follows the recorded stack.
The [enemy-template ledger query](src/map/visibility/template_reveal.rs) derives observer-specific basic disclosure, standard versus fake template selection and fuzzy counts from recorded country intel, relations, deployed template references and clock operands. Thresholds come from the recorded content stack. This adds no capture and remains separate from the province tooltip's visibility gates.

The enemy-template query resolves each grouped display name through source-country, effective-country and target-country rosters in native roster order. Fuzzy counts use that resolved template identity, including localisation collisions supplied by mods.

The current sortie-efficiency law is shared by aircraft scenarios and Air mission hovers. Ribbon content also resolves the radar symbol's runtime definition, texture and graphics defines, with per-item artwork fallback.

Building site content preserves the first listed province for every state/base kind independently of the position finder, which may skip that province. Only ambiguities that can change the first province refuse its answer.

Current base-to-region Air reach is shared by assignment stripes and transport orders. It preserves the region template split rectangles, centre prefilter, signed wrapped distances and state versus hosted origin rules.

Concrete ribbon draws identify Air map controls independently of battle-plan display families, while sharing the same type buckets and geometry.

Map symbol content includes the Air transfer progress symbol, with its installed technique, texture, size and graphics colour. Its orientation is supplied by relocation state rather than a random presentation clock.

Air detection reads retained executing mission masks/counts independently of selected mission icons and graphical sortie counts. Country, wing and ace normalization applies recorded modifier metadata and feature eligibility, native fixed-point arithmetic and final clamps. Zero execution and zero weighted aircraft avoid unused normalization demand.

Air mine-target admission shares the complete sparse Navy mine vector and per-country completeness witness with Navy map mode. Only a witnessed omitted in-range value means zero; unavailable tables, missing witnesses and out-of-vector regions refuse admission.

Regional mine readers require both complete sparse-table coverage and each navy's vector witness before interpreting omitted amounts as zero. Regional naval control and explanatory ratios retain their distinct directed relation and roster laws.

Native progress drawables retain normalized float-setter state separately from integer-setter state. Float ratios clamp before double scaling and float half rounding; their rounded value is rendered directly, including authored step counts above exact float integer precision. Integer assignments retain their own upper clamp, and drawable replacement resets either state.

Regional Navy dominance source products share a complete-ledger certificate across the counter and province-hover features. A complete sparse map gives omitted regions zero sources; global and region-local failures retain their names.

Recorded naval modifier labels share one prepared localisation index across combat windows and Navy mission controls. Recoverable source names follow the display language; composed, unmatched or ambiguous names retain counted refusals. Drawing reads no content.

Air night and weather normalization have separate entry points as well as their combined detection product. Combat-stat consumers demand night terms only after the full province night-factor threshold, preserving unused modifier and carrier-source failure boundaries.

Frame-backed ace lookups index full handles lazily once because AceRangeInputs orders country before handle. Identical repeated observations share their row; conflicting observations refuse only that handle by name. Range, normalization and patrol consumers reuse the same prepared index.

World-name draw operands retain optional connected strategic-area province membership beside cached glyphs for per-view pointer admission. Mode visibility retains political-mode identity separately from the reusable icon priority table, so port grids can choose their native comparator without guessing the current mode from artwork priorities. Neither value is recording data.

Occupation markers use the ordinary native map-icon projection, pooled window lifecycle and control hit geometry. Their authored parent and modifier-row templates join the shared window asset demand. The type's pool begins with 96 windows and uses the common queued growth; the size is an initial allocation, not an admission cap.
