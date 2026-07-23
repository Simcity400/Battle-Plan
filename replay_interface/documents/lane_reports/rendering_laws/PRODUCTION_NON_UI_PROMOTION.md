# Production non-UI replay promotion

Status: **IMPLEMENTED; 2026-07-19 PROMOTION RECEIPT HISTORICAL; CURRENT CENTRAL VALIDATION/REFREEZE PENDING**

The promotion disposition and counts below describe the 2026-07-19 source set.
They do not claim current PASS after the container, recording-open, prefold, and
product-route changes. Missing runtime observations remain optional fail-closed
feature-activation evidence and omit only their named pixels.

Pinned target: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

This report is only for the replay renderer. The extractor remains independently distributable. Its only product handoff is the portable `.battleplan`; replay_app never loads or calls extractor code. Debug capture bundles are a separate evidence route and are not a product dependency.

## Promotion disposition

There is no ambiguous `PARTIAL` state in this packet. A route is either promoted, debug-only, or omitted for one named authenticated-data requirement.

| Route | Disposition | Exact result |
|---|---|---|
| Portable startup | PROMOTED | `--recording` opens and validates without `--bundle` or `--capture`. Those switches are optional debug/evidence inputs only. |
| Playback content binding | PROMOTED | Immediately after open, playback reconstructs only recorded required layers, validates the portable content manifest, and constructs an admission-bound `GameFileResolver` before renderer consumers. |
| Effective-VFS reads | PROMOTED | `read_admitted_used_content` requires exact full `UsedContentKey` equality and rechecks winning source, byte length, and SHA-256 on reread. Path-only admission, vanilla fallback, changed overrides, and unrecorded installed files reject. |
| Display submit and draw order | REPLAY CONSUMER PROMOTED | `display_submit_index` is the only display selector and `RenderSubmit::ordered_draws` plus `ordered_render_operations` are the only admitted orders. Unknown, duplicate, omitted, first/last-guessed, family-concatenated, or incomplete joins reject. Producer scheduling and generation of a new portable carrier belong to the later extractor phase. |
| Typed submitted-draw executor | REPLAY CONSUMER PROMOTED | replay_app reconstructs admitted shader/resource bytes, VS/PS and optional GS/HS/DS/CS, texture/SRV resources, IA buffers and draw calls, scene/draw CBs, samplers, full blend/depth/stencil/rasterizer state, viewport/scissor, and exact output descriptors. Stale RenderDoc pointers and prebuilt workstation sidecars remain forbidden. |
| Stream-output state | IMPLEMENTED END-TO-END; PIXELS STATIC-CATALOG GATED | The carrier and executor preserve exact sparse slots, stable buffer identity/capacity reuse, byte offsets including the append sentinel, and authenticated empty/null binding. The retained 1,239 entity plus 27 particle draws authenticate GS/HS/DS/CS and stream output as null at their matched final draws; any future non-null observation requires its exact carried state and otherwise rejects. |
| Capture-independent static map | REPLAY CONSUMER PROMOTED; INPUT CARRIER DEFERRED | The replayer has deterministic admission, exact supported geometry interpreters, an exact native MainScene target owner, and opaque sealed static programs. Existing retained recordings without programs omit them; creating a new populated carrier is an extractor-phase task. An empty plan or blank target is never called rendered terrain/water/rivers/trees. |
| Portable static-map D3D contract | IMPLEMENTED PRODUCT ROUTE; PIXELS DATA-GATED | Normal product playback prepares and validates the typed D3D plan on every displayed hour. Every static program is an opaque sealed object identified by its whole-program `payload_sha256`; `pass_token` is diagnostic only and no fixed terrain/water/etc. role classifier exists. Its sole manifest consumer is `production_map.static_program/<payload_sha256>`. `ordered_content_keys` is authoritative: the renderer clears its prior-frame cache, eagerly opens only the selected program keys through the admitted VFS, and requires exact equality with every nested `UsedContentKey` referenced by authenticated operations, including every exact render-target initialization `UsedContentRange`. Unknown, duplicate, unused, extra, missing, or uncovered content/operations reject. Inline and admitted target initialization, persistent targets, nonzero mips, and MRT descriptors are validated; admitted ranges enter D3D receipts and native creation reads their already-loaded bytes only through the exact sealed consumer binding. `ordered_render_operations` is the sole cross-static/runtime order. Scene CBs, resource byte ranges, output descriptors, and deterministic receipts fail closed. Supported pass-scoped raw VB and raw VB+IB interpreter payloads reopen exact admitted bytes, validate length/hash/draw bounds, and materialize exact geometry before native execution; unknown interpreter grammars omit only their named static program. The normal replay app creates/caches `NativeProductionD3dBackend` on its own device/context, attaches the current app RTV, executes the unified frame, and accepts visible output only when an authenticated operation names `SwapchainBackBuffer`; this occurs before the HUD. Resize releases the AddRef'd backend before `ResizeBuffers` and forces a frame rebind. Feature-local native omission clears only the map surface and preserves transport/HUD. Current real recordings still carry no complete admitted static programs, so product pixels remain omitted on those files. |
| MainScene presentation | IMPLEMENTED PRODUCT ROUTE; CARRIER DATA-GATED | The normal app calls `NativeProductionD3dExecutor` and attaches its app-owned back-buffer RTV only for an authenticated carried output naming `SwapchainBackBuffer`. The carried map -> post-process -> present envelope and final-draw joins are validated exactly. Normal replay_app omits the route when domain 77 render-catalog or domain 80 effective-VFS role inputs are unavailable. Raw MainScene copy remains proof-only and is never used by the product path. |
| Snow/mud, political runtime textures, tiled lights | IMPLEMENTED; BASE-TARGET GATED | Exact upload/update laws are wired into the validated captured-map debug route. They become product-visible only with an authenticated capture-independent static target; stale prior-hour reuse rejects. |
| Captured terrain/water/rivers/trees pass bundles | DEBUG_ONLY | Existing captured D3D11 passes remain available for evidence and regression work. They are not a portable production route. |
| M02 combat indicators | REPLAY CONSUMER PROMOTED | A-E law/data closure is complete. The portable selected-Present UI pass carries the exact typed M02 rows, full draw frontier, VFS-bound content, exact draw packets, and authenticated interleave; the native executor preserves the recorded swapchain draw ordinal. |
| M03 order/movement arrows | REPLAY CONSUMER PROMOTED | The A48-labelled transition/draw carrier, stable-key join, exact comparator inputs, VFS content, cross-family order, and native execution are admitted without inferred subtype or ordering. |
| O04 cities | REPLAY CONSUMER PROMOTED | The exact generic entity carrier and runtime executor preserve the retained 81-city/243-draw consumer join and authenticated output/order state. |
| Selected/integrated DLC precedence collisions | PROMOTED | Exact a729 mount-planner precedence is implemented and fail-closed: integrated DLC uses the physical descriptor-parent mount and bytewise enumeration; selected DLC/mod planning preserves priorities 0/10000, dependency lowering, and stable descriptor launcher-ID ordering; later layers win; archive and exact-parent `replace_path` behavior are bound. Extractor passed 176/20 twice, map-viewer 827/1 twice, and the portable binder final gate is green. Dossier SHA-256 `223fe31ee2e81491772cbbbbb683d60ed6c672b77e2704ecd39eceeb83626551`; evidence SHA-256 `4c538d0a711ecb4d0ba904ff9f18264c4bf07a343393e4c19b5d09f16fa489a3`. |

User-removed scope remains absent: tree camera-height fade, border chain direction/texture-V, diplomacy fill RGB, and resource fill.

## Exact executor findings

- Shader programs are not VS/PS-only. Optional geometry, hull, domain, and compute stages are explicit authenticated carrier fields; `None` alone means null.
- The retained-RDC census observes GS/HS/DS/CS and stream-output null for all 1,239 entity plus 27 particle draws examined. This supports null for that retained corpus only; the live hook must still authenticate the per-draw binder/catalog row before scheduling pixels.
- Every programmable stage is cleared at the draw boundary before the carried stages are bound, preventing a static map shader from leaking into a submitted draw.
- Stream-output targets use the exact carried sparse slots, buffer identities/capacities, and offsets; only an authenticated empty list binds null.
- Direct-visual and weather rows are stable semantic joins to entity/particle outputs, not duplicate draw families.
- All game-file-backed shader, entity, mesh, texture, and buffer-range payloads reopen through the exact admission-bound VFS key at execution time.
- Static programs cannot bypass their opaque sealed identity: the whole-program hash owns one exact `production_map.static_program/<payload_sha256>` consumer, `pass_token` is never used for discovery, and duplicate/unknown/unused/extra/missing ordered content keys, invalid byte ranges, and uncovered outputs reject.
- Exact target initialization is included in that closure. Every nested admitted range is validated and receipted; native target creation consumes the sealed `ProductionMapResource` bytes and rejects a missing, foreign, changed, or ambiguous consumer binding instead of reopening by path.
- Supported raw VB and raw VB+IB interpreter tokens materialize exact admission-loaded bytes to `StaticGeometrySource::Exact`; hash/length/draw-range drift rejects, while an unknown grammar yields a feature-local static-pass omission rather than guessed geometry.
- `ordered_render_operations` is the sole cross-static/runtime sequence; family concatenation or inferred pre/post buckets are forbidden. Render-graph color targets and prepared-target SRVs are exact carried resources. The product app executes this sequence on its own D3D11 device before the HUD; a raw MainScene copy is proof-only, and only a carried operation naming `SwapchainBackBuffer` may make the route visible.
- Malformed carriers reject. Missing feature carriers omit only that feature; a previously bound hour is never silently reused.

## Provenance

No new Ghidra operation was issued by this production integration. It consumes these frozen upstream records:

| Function / role | Job ID | Artifact path | Manifest SHA-256 |
|---|---|---|---|
| `0x14124b710` SnowMud owner scope and CPU producer | `20260719T220305Z-wave2-r10-addr-03-05-1df2b63f` (`ADDR-03-05`, actual `shard-03`) | `<local-path>` | exact-10 manifest `492e712cd283a75b9b6357eb42f4bc98137d1ab138cfa857164f8d353f675c28`; artifact `6a8acb1cd322e54ebf473e808905019c3b2af878dad3a65c742bbed80cb820bb` |
| `0x142400e40` SnowMud CPU preupload helper | `20260719T220305Z-wave2-r10-addr-03-05-1df2b63f` (`ADDR-03-05`, actual `shard-03`) | `<local-path>` | exact-10 manifest `492e712cd283a75b9b6357eb42f4bc98137d1ab138cfa857164f8d353f675c28`; artifact `f1fa4242316dbd01954367c42644ae56149f212ea361fc08eaddc25a693fef08` |
| `0x14227f420` particle bind/draw submit | `20260717T004345Z-xn12a-cpdxparticleobject-update-callers-98b14e19` | `C:\Users\Example\ghidra_agent_exports\20260717T004345Z-xn12a-cpdxparticleobject-update-callers-98b14e19\evidence\func_14227f420.decompile.c` | `a64495ae47dfaaa2a6f752ca1b5d96ec1f764041b68074ea8e79d88cea51f6d1` |
| `0x1423dd730` particle instance update / canonical packet producer | `20260717T004108Z-xn12b-particle-instance-render-update-co-75672612` | `C:\Users\Example\ghidra_agent_exports\20260717T004108Z-xn12b-particle-instance-render-update-co-75672612\evidence\func_1423dd730.decompile.c` | `b8482288abf63d70128ddd097186c3cba5c7e9e8eb48d906df3cd5b00670192d` |
| `0x1422644c0` entity VB/layout/IB/material/palette submit root | `20260716T084950Z-l03-shadow-model-list-producer-v1-bbb15676` | `C:\Users\Example\ghidra_agent_exports\20260716T084950Z-l03-shadow-model-list-producer-v1-bbb15676\evidence\func_1422644c0.decompile.c` | `5c3e7986b4350f612b2eb908ef1a6fa533cda1523ac9b2a42f91621a21da3e4f` |
| `0x14166ec50` weather membership/visibility/playback producer | `20260717T012439Z-xn14a-weather-region-getter-all-callers-e6227833` | `C:\Users\Example\ghidra_agent_exports\20260717T012439Z-xn14a-weather-region-getter-all-callers-e6227833\evidence\func_14166ec50.decompile.c` | `fd99de197bf7699319a522e0d2e68dbd63a36d9a778ded8ca368597d9679d9c8` |

The two SnowMud rows close static hook custody only. They do not activate the
new H/H2 transport: that lane remains false until two authenticated live H/H2
pairs, including a content change, pass its separate admission contract.

The complete XN15 ledger is `replay_interface/re_evidence/rendering_laws/XN15/evidence_ledger.md`, SHA-256 `2d48349125008e7db36ab5af887bb7261c8831a44dedae4099e38efe0dd42527`.

The complete live-submit/d80 provenance table and negative-space discovery ledger are maintained in `replay_interface/documents/lane_reports/rendering_laws/XN15L.md`. Its final refreeze records why domains 77 and 80 remain reserved instead of promoting a sidecar or stale-pointer producer.

The complete M02 thirteen-function provenance table is frozen in `replay_interface/documents/lane_reports/rendering_laws/M02.md` and its adjacent evidence manifest; the exact DLC precedence seventeen-function provenance table is frozen in `extractor/documents/dossiers/DLC_EFFECTIVE_PRECEDENCE_RE.md`. Those upstream tables record each function address, job ID, artifact path, artifact SHA-256, and manifest SHA-256; this packet references rather than abbreviates them.

## Historical verification — replay-only refreeze 2026-07-19

All checks used retained portable/fixture inputs. No extractor source/test, the game process, live capture, or adversarial review participated.

- `map_viewer` production family: 60/60 passed twice.
- Static-program consumer filters: renderer 13/13, interpreter 5/5, and D3D/native 18/18 passed twice. The native suite performs real D3D11 offscreen draw/readback and exact sealed-consumer target-initialization checks.
- Normal replay_app product D3D route: 4/4 passed twice. This includes an exact-red app-target pixel proof plus M03-only and O04-only `SwapchainBackBuffer` presentation recognition.
- M02/M03/O04 replay consumer join: passed twice. M02 closes 140 checks; M03/A48 closes 337 checks; O04 preserves the exact 81-city/243-draw retained consumer join.
- Static consumer relocation/VFS verifier: passed twice with byte-identical receipt SHA-256 `8bd849ce8374d8c82d2ca666fab138db8a96f4aec0dc10dd9d03b9a1b2c29860`. It covers relocation, mod override, `replace_path`, optional unused DLC, exact sealed-program validation, and exact raw VB/VB+IB materialization.
- A replay-side frozen semantic `production_map.province_raster` dimension gate was removed from both windowed and headless product execution. It had incorrectly suppressed unrelated M03/O04/runtime draws. Carried target/geometry/viewport/output descriptors are now authoritative; each operation still fails closed on every exact `UsedContentKey` it actually consumes.
- No production consumer contains a frozen vanilla path, catalog, map dimension, or fallback asset default. Test-only logical paths remain fixtures, not product defaults.

The historical machine-readable contract and twice-stable verifier receipts remain under `replay_interface/re_evidence/rendering_laws/PRODUCTION_NON_UI_PROMOTION/`. They are not current freeze authority; current central validation and content-addressed refreeze are pending. Producer completion and a new populated end-to-end recording are optional acceptance evidence rather than a prerequisite for fail-closed base-product operation.
