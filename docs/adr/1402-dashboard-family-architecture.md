# Dashboard family architecture: source plans, bounded read views, and installed pictures

Dashboard widgets follow one architecture from the recording to the screen so every family shares playback discipline, time safety, interaction soundness, and exact-read verification without per-widget orchestration or duplicate worker pools.

```
recording
  -> Rust products          facts, units, coverage, reasons, dated identities, recorded order, totals and ranks
  -> family source plan     one declaration per widget view: what it reads, its coverage mode, and its time shape
  -> bounded read view      only facts evidenced at or before displayed hour H reach a family
  -> family projection      pure: (readView, options) -> model
  -> lane worker            layout + paint; publishes one picture: pixels + hit targets + action keys + audit rows
  -> family host (main)     installs the picture whole or not at all; pointer, keyboard, scroll, focus, tooltips
  -> shell (React)          frame, title, settings, pickers, pop-up detail host
```

The map keeps its native renderer; game windows (such as the equipment designer) render through the native off-screen renderer inside the shared pop-up host.

## One owner per job

1. **Drawing a widget body (`ui/src/draw/` via `ui/src/families/host/`).** Every dashboard body renders on the worker lanes. There are no main-thread canvas charts, no second worker pool, and no DOM-drawn data bodies. Every family registers with an open `FamilyId` string and shares cost-weighted per-instance lane placement (`widgetId|declarationKind|instanceId` with explicit `kind`).
2. **What is on screen (`InstalledPicture`).** Each installed picture is an immutable tuple committed atomically by `Page.install`: `id`, `owner`, `generation` (`{ seek, reset, binding }`), `factRevisions`, `validInterval`, `through`, branded `hour` (`DisplayedHour`), `viewport` (width, height, DPR, scroll, optional chart `transform`), options and theme revisions, `actionKeys`, `accessibleDescription`, and `auditRows`. Pointer clicks, Enter/Space activation, context menus, pinned tooltips, accessible descriptions, and exact-read audits all resolve against the **installed** tuple, never against live React props or newer data. Pointer coordinates stay in client space and convert through the installed viewport geometry (`clientPointToInstalled`), so a stationary click on a held reply during a resize still hits what is visually under the cursor. Changing `owner` clears the installed picture in both `Page` and `Lane` immediately and rejects any late presentation or hit carrying the previous owner.
3. **Asking for and following data (`ui/src/families/plan.ts`, `broker.ts`, `host/useFamilyPlan.ts`).** A widget view declares `sources(ctx, view) -> SourcePlan` and `project(readView, options) -> model`. Plans declare plain serialisable parameters across seven time shapes: `point`, `range`, `branch-to-hour`, `trailing-window`, `pixel-binned`, `exact-hover`, and `paged-rows`. `planSignature` compares plans cheaply without invoking closures; `FamilySourceBroker` deduplicates concurrent requests by produced parameters (`planQueryKey`), drops stale slot completions, and aborts on unmount.
4. **Coverage of partly read hours.** Reflecting the query layer's coverage rule, every plan entry declares either `mode: 'keyed'` (explicit keys, independent per-key cells, row-level coverage allowed) or `mode: 'population'` (`all`, `top`, `least`, `parts`, or `total`, computed in Rust with table-level coverage). `compileFollowPlan` rejects row-level coverage on population forms and `top`/`least` on keyed forms. Families never rank, sum, total, or assign category order from partial client data.
5. **Time safety (`ui/src/families/boundedView.ts`).** Families read only through `BoundedReadView`, which enforces nothing-after-`H` across panels, answers, live cells, pixel-binned series, approximate spans, missing spans, marks, hovers, paged row sources, and `who(kind, key, atHour?)`, throwing `FutureDataError` on any read past `H`. Advancing `H` costs O(what changed), never O(history): closed bins, runs, missing spans, approximate spans, and marks before `H` pass through by reference via binary-search index bounds (`BoundedSlice<T>`), and quiet advances inside `validInterval` reuse the cached view with zero visits and zero allocations (`BoundedSeriesCursor`). A bucket straddling `H` is cut from witnessed prefix runs or points before any range, scale, total, order, label, or hit is computed; when no witnessed prefix exists, the straddling bucket is omitted and marked `straddling: 'loading'`.
6. **Identity and art seams (`IdentityProvider`, `ArtProvider`).** `readView.who(kind, key, atHour?)` and `readView.art(handle)` give every family one time-bounded path to dated entity identities (holder, tag, name, colour, flag choice at a row's own observation hour or at `H`) and mod-stack art results, registering pending demands with the broker so the family wakes when they settle.
7. **Body states and trailing gaps (`ui/src/dashboard/model.ts`, `regionTime.ts`, `regionGate.ts`, `families/host/FamilyBodyState.tsx`).** Host body states (loading, outside, not recorded with reason, refused with reason, failed with retry) live in one place. Campaign-mode windows (`resolveWindow`) and end pins (`recordingEndTime`) resolve to `lastRecordedBranchHour(branch)` so recordings ending in a trailing gap settle on their last recorded hour instead of hanging in `outside` or `loading`.
8. **Verification (`ui/src/families/host/auditBinding.ts`).** `createPlanAuditVerifier` and `createBoundAuditReader` re-run the exact same `project(readView, frozenOptions)` on an independent exact read at the installed picture's hour, never using streaming `Facts` or `TileCache` as their own oracle.

## Invariants

- Register each widget kind, its views, sources, time shape, coverage mode, country policy, settings, and audit projection together.
- Never read after `H`; clip history and cut straddling buckets before computing scale, totals, order, labels, or hits.
- Paint and interact exclusively from the installed picture; carry `(installedId, owner, subject, hour)` on every action.
- Steady playback frames perform zero React commits, zero DOM layout reads, and zero history copies; quiet ticks inside `validInterval` perform zero projections and zero paints.
- Delete superseded renderers, schedulers, and orchestration hooks in the same PR that replaces them.

[Source plan](../../ui/src/families/plan.ts) · [Bounded read view](../../ui/src/families/boundedView.ts) · [Family broker](../../ui/src/families/broker.ts) · [Family plan hook](../../ui/src/families/host/useFamilyPlan.ts) · [Pointer binding](../../ui/src/families/host/pointer.ts) · [Audit binding](../../ui/src/families/host/auditBinding.ts) · [Widget family contract](1398-widget-family-contract.md)
