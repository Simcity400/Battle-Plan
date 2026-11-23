# 1305. Draw trade routes from recorded clients and content geometry

The viewer draws the viewed country's routes using the same mode gates as the game. Resolve references to a canonical country slot before testing equality. A zero reference has no country. Keep native list order for the routes and native exchange ordinals for displacement, even when recording rows were sorted by a different key.

| Layer | Recorded client | Route kinds | Convoys |
| --- | --- | --- | --- |
| Production | The country's own resource origins | Naval | Yes |
| Supply transfers | The country's supply transfer clients | Land and naval | Yes |
| Exports and imports | The country as exporter or importer | Land and naval | Yes |
| Lend-lease given and received | The country as giver or receiver | Naval | Yes |
| Market bought and sold | The country as buyer or seller | Land and naval | No |

Select the first native market contract with the same route kind and ordered endpoint states. Ignore country identity, port provinces and caches for grouping. A bad result on any grouped contract marks the line bad. Imports and exports use the importer for direction. Lend-lease uses the receiver for direction and the giver for membership. A matching canonical slot for the direction country and route country reverses those lines. The other layers use source-to-destination order.

Use the recorded route's bad flag and the content's colours. On a bad naval route, a sea region below the recorded delivery-efficiency threshold uses the hotspot variant and secondary colour. A region with no entry in the available efficiency table has full delivery efficiency. Each port, crossing and destination keeps its own place in the control sequence. A crossing takes the lesser of its neighboring province variants, so an isolated hotspot does not colour both boundary crossings. Subdivision propagates the greater adjacent variant. Defines supply either HSV or RGBA, and shader conversion preserves alpha.

Route search follows the recorded endpoints and the content's ordered province graph. Naval routes admit the recorded regions and the endpoint regions, while land routes use the recorded states. Restrict neighbors using their recorded permissions and refuse a crossing whose permission cannot be established. Keep frontier tie order and replace a predecessor only on a strictly lower cost. Use port positions for port land provinces, ordinary positions elsewhere, and insert the raw crossing controls before smoothing. A wrapping province-use counter is shared across all layers. Displace repeated controls according to that counter while preserving the initial anchor and all port anchors. The ribbon's scale, texture loop and perpendicular texture coordinates follow the content's arrow style. Empty or refused routes yield a named diagnosis rather than guessed geometry.

Cache route stops only while their inputs agree and drop unused entries after the next build. Repeat a refused search so its reason remains visible. Keep displaced controls for every convoy-carrying line that drew, even when its assigned count is zero. A count-only change can then rebuild the convoys from the recorded origin or client columns without searching or drawing the lines again. The full build resolves the convoy entity on first demand, so a later count-only change must rebuild the lines if it needs a model that was never resolved.

Convoys show only when the mode permits routes, units are shown and the camera is below the unit cutoff. Round the scaled assignment and clamp positive assignments to the authored count range. Resolve the viewed culture's entity before the content's default entity. The recording lacks the native convoy seed and history, so presentation begins from a fresh seed and follows the known motion law. It offsets interior controls by the outgoing segment delta without normalization and keeps the endpoints fixed. Five initial launch attempts check raw three-dimensional sphere overlaps across all active lines. Exact contact does not overlap, and the test uses the stored positions, including across the map wrap. An ended or blocked convoy sleeps for a random fraction of the authored time. A timer at exactly zero stays asleep and a convoy at exactly a segment end advances on the next update.

The Windows presentation counter passes two distinct writable `i64` locals to the synchronous performance-counter calls. Each call writes its scalar output before returning and does not retain the pointer.

[Route layers and retention](../../crates/viewer-unit-models/src/trade_routes.rs) · [Path search and ribbons](../../crates/viewer-unit-models/src/trade_routes/geometry.rs) · [Convoy presentation](../../crates/viewer-unit-models/src/trade_routes/convoys.rs)
