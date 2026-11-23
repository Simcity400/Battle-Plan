# Log widget family

Log widgets (chronological feeds, histories, and ledgers) share one stateless tail contract on the native side and one canvas-painted worker family on the UI side, upholding the five family properties in [ADR 1398](1398-widget-family-contract.md) and the five-state value contract in [ADR 1397](1397-family-value-contract.md).

## Native tail contract

Every log widget subscribes to a stateless, exact `tail(hour, params, limit)` dated panel returning one shared `LogTailReply` envelope (`columns`, `baseOrdinal`, `firstOrdinal`, `total`, newest-first `rows`, exact `facetTotals`, `missing`, and `warnings`) without touching `dashboard-follow::host`.

- **Exactness and statelessness.** `Reader::tail` and `Reader::total` share one `slice` core with `Reader::page` in `crates/dashboard-query/src/log.rs`. `LogTailReply` omits the moving query `hour` (the `PNL2` frame carries it), so `dashboard-follow`'s `panel::product::Delivery` automatically emits zero-byte `STANDS` updates on quiet hours.
- **Exact facet totals.** `countFacets` computes exact counts up to `hour` (including the current partial bin), eliminating stale bin-rounded counters such as `division-log`'s vanished diagnostic.
- **Inline enrichment.** Domain tail products (`division-log-tail`, `market-ledger-tail`) share one `enrich_rows` path with their paged counterparts (`division-log`, `market-ledger`). `division-log-tail` resolves province names inline through a `Source`-scoped `PlaceCache` (cleared in `forget`) whose output is verified equal to uncached resolution, eliminating per-row `place-names` IPC loops from playback.
- **Bounded work.** `limit` (`K`) is sized from the viewport height (`tailLimitForViewport`, floor 64 rows), so per-step native work, IPC payload, projection, layout, and paint depend only on `K` and never on total log length.

## UI log family (`ui/src/families/log/`)

- **One projection for drawing and audit.** Each log widget registers one pure projection `(sources, hour, options) => LogModel` with `registerLogProjection`. The worker lane (`familyLogLaneView`) and the main-thread exact-read audit (`FamilyLogHost` via `AuditSnapshotStore`) run the same projection and layout on the same visible slice, comparing canonical five-state values and row order (`ordered: true`).
- **Host-owned playback discipline.** `FamilyLogHost` paints onto a worker-owned `OffscreenCanvas` (row layout implemented; card layout declared as an extension point). Facet summary pills and the accessibility mirror update in the DOM without triggering React commits (`0` commits during playback), and `install()` performs zero DOM layout or style reads. `useShownHour()` reports `null` before first paint and the painted `model.hour` thereafter.
- **Cadence and pause.** Mirror (~1 Hz) and tooltip (≤4 Hz) updates share `CadenceGate` and `TrailingFlushScheduler` from `ui/src/families/cadence.ts`, with an immediate flush on pause.
- **Scroll-back and jump to latest.** Scrolling away from the top freezes the displayed hour and snapshot without shifting the user's rows, displaying `Jump to latest` (or `{N} new · Jump to latest` as live events arrive). Scrolling past the `K`-row tail loads older slices on demand via the existing paged product (`division-log`, `market-ledger`). Pausing at the top fetches nothing.
- **Widget standard visuals.** Row heights follow R6 (`28/22 px` single-line, `44/36 px` two-line); missing/unrecorded values render hatched placeholders (`drawFormattedValue`) and never silently default to zero or another category; recording gaps render inline as hatched `Not recorded · {span}` rows (R13); filter chips occupy one horizontally scrollable row (R14); tooltips carry a date line (T1) and pin with `P` (T10).

[Log model](../../ui/src/families/log/model.ts) · [Log layout](../../ui/src/families/log/layout.ts) · [Log painter](../../ui/src/families/log/painter.ts) · [Log lane view](../../ui/src/families/log/laneView.ts) · [Log host](../../ui/src/families/log/FamilyLogHost.tsx) · [Native log tail](../../crates/dashboard-base/src/products/log_page.rs) · [Widget family contract](1398-widget-family-contract.md)
