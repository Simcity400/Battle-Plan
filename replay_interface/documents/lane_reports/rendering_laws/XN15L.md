# XN15L - authenticated live render-submit joins

Status: **IMPLEMENTATION MECHANICALLY FROZEN; LIVE E2E RELEASE GATE OPEN**

Authority: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

This packet owns extractor/live-hook evidence and portable promotion seams. It does not write or replace `PRODUCTION_NON_UI_PROMOTION.md`.

## Frozen observer laws

### Exact particle definition-to-atlas observation

- `0x142281b00` builds two equal-length, ordered per-subsystem arrays on the
  resolved particle type: shader-program indices at type `+0x48` (count
  `+0x54`) and `0x14`-byte atlas rows at type `+0x60` (count `+0x6c`). Each
  atlas row is the exact texture runtime index, f32 half-pixel width/height,
  and authored `x/y` columns/rows. The half-pixel numerator is proven as
  `0.5f`, not `1.0f`.
- Packet `+0x60` is the flattened subsystem selector already retained as
  `RawParticlePacket.packet_ordinal`. `0x14227f420` uses that same selector for
  both arrays before binding program, texture, constants and the final draw.
  Multi-atlas selection is therefore ordered and duplicate-preserving;
  rows/columns alone are not an identity.
- `0x1423f17e0` joins the authored texture path bytes to the runtime texture
  index, while admitted H4CT VFS payloads join that logical path to the exact
  active mod/DLC winner. Production must authenticate the actual program and
  bound texture at the draw, retain both definition and texture winners, and
  fail closed on any selector, constant, program, texture or winner mismatch.
  No vanilla atlas catalog or captured texture substitute is permitted.
- The complete receipt and implementable producer API are frozen in
  `replay_interface/re_evidence/rendering_laws/XN15L/PARTICLE_RUNTIME_OBSERVATION_2026-07-19.md`.

### Exact effective-VFS texture-to-D3D resource join

- `0x1423f17e0` keys the engine texture cache by the exact authored logical
  path. Its miss path is now proven end to end:
  `0x1423f2ff0 -> 0x1423f3320` resolves the live VFS file object,
  `0x1423f2d50` reads and decodes the selected bytes, and
  `0x1423fd6d0 -> 0x1423ffc90 -> 0x1423fb4f0` creates the native texture.
- `0x1423fb4f0` calls ID3D11Device vtable slot 5 `CreateTexture2D` and returns
  the exact `ID3D11Texture2D*`. `0x1423ffc90` stores it at wrapper `+0x08`,
  creates an SRV from the same resource through device slot 7, and stores the
  SRV at wrapper `+0x00`. The existing SRV `GetResource` observation therefore
  supplies an exact pointer-equality join to `RawTextureResourceSnapshot`.
- The source side remains the authenticated effective-VFS winner/open receipt,
  including container SHA-256 plus exact member offset/count/range SHA-256 for
  archive winners. Production must correlate the same open/stream/data/range
  to the creation event, require SRV-resource equality, and validate the D3D
  descriptor and every ordered subresource against the admitted winner bytes.
  Runtime pointers and cache indices are stripped from portable output.
- The complete proof, producer API, and per-address Ghidra provenance are
  frozen in
  `replay_interface/re_evidence/rendering_laws/XN15L/VFS_TEXTURE_RESOURCE_JOIN_2026-07-19.md`.

### Live program and final-draw authentication

- `0x142400520` binds the high-level shader program and stores it at render-context `+0x220`. The hook keeps a fresh range-local association for particle/entity joins and a per-context bound-program cache for static/post-process draws.
- `0x1423f4ec0` returns the engine program index. The authenticated cache accessor at `0x1423f5d60` proves the exact successful-index join: `shader_cache + 0x20 + program_index * 0x38`. The live hook records that transient program pointer with the lookup receipt and later requires the same pointer at `0x142400520`; negative indices use a global fallback and therefore fail closed for exact lookup authority.
- `0x1423fb3e0` resets the high-level render-state caches. Map, post-process, and present phase transitions also clear the extractor cache, so a stale program cannot cross a phase boundary.
- Exact final wrappers are `0x1423fe970` (`Draw`), `0x1423fe9b0` (`DrawIndexed`), `0x1423fe9f0` (`DrawInstanced`), and `0x1423fea40` (`DrawIndexedInstanced`). Runtime particle/entity traffic is accepted only within its authenticated producer range; other final draws are classified as static/post-process and require the per-context binder association.
- The a729 installation is one 32-target, signature-gated hook set. A sentinel mismatch, invalid pointer read, overflow, missing program join, phase mismatch, duplicate, gap, or incomplete cycle fails the drain/promotion.
- The retained RenderDoc census covers 1,266 authenticated runtime draws: 1,239 entity and 27 particle. All have non-null VS/PS; all have null GS/HS/DS/CS and no stream-output targets. Those nulls are authenticated only at the matched final draw.

### Entity identity is selector-native

- `0x14227e5f0` performs the case-sensitive named-entity registry lookup (FNV-1a seed `0x811c9dc5`), and `0x142278710` passes the resolved definition to `0x142277df0`. The returned entity retains that definition at entity `+0x40`. This authenticates the runtime registration token to the effective-VFS `.asset`/`pdxmesh`/`.mesh` winner chain; it does not assign render-list semantics.
- `0x1422644c0` proves the exact draw-selection contract. `pass_selector` selects the outer list header at `selected_list_root + pass_selector * 0x18`. Each `0x50` outer row supplies the entity-group ordinal and entity pointer. That ordinal indexes both the mesh-owner `+0x48` vector and the entity `+0xb8` binding vector, each with a `0x18` header. The inner ordinal pairs a `0x108` mesh-draw row with a `0x60` material-binding row.
- `alternate_list` selects the regular or alternate outer-list root and admits a row iff it equals `bool(entity[0x78] != 0 || mesh_draw[0x7d] != 0)`. For an ordinary entity variant, the shader-program index is material-binding dword `[pass_selector + 3]`; for a nonzero entity variant it is the dword at `+0x2c`. The resolved program, exact buffers/resources/state, and the indexed draw are then bound. No submesh name, LOD name, material name, damage name, or selection name is read or produced on this path.
- Therefore the exact portable identity is the effective-VFS entity asset and mesh content keys plus the pointer-free native selector tuple: entity registration token, entity-group ordinal, mesh-draw-list ordinal, entity-variant byte, mesh-draw-variant byte, signed pass selector, alternate-list byte, and bind-renderer-target byte. It is joined to the captured program and D3D bindings. Process pointers remain transient join keys only. Assigning `submesh_index`, `lod_index`, `standard`, `undamaged`, or `unselected` semantics without separate authored evidence would fabricate information and is forbidden.

### Exact clear and unified order

- `0x1423fca30` is the combined clear wrapper. Bit 0 executes `ClearRenderTargetView` first through the D3D context vtable slot at `+0x190`, using the render-target object view at `+0x10` and the explicit color or default `{0,0,0,1}`.
- Flag bits 1/2 then execute `ClearDepthStencilView` through vtable slot `+0x1a8`, using the depth/stencil view at render-target object `+0x18`, depth `1.0f`, and stencil `0`. The hook emits color and depth/stencil events in this native order.
- The exact frame envelope is map scene `0x140b481a0`, post-process `0x140ff3450`, then present `0x1420b6530`. Operations are legal only inside map or post-process; every frame must contain all six begin/end boundaries in order.
- Each completed hook frame is frozen at authenticated `PresentEnd` with the verbatim `QueryPerformanceCounter` tick, the `QueryPerformanceFrequency` captured and hashed into the early hook epoch, process-start nonce, hook-epoch nonce, and the exact monotonically increasing completed-Present ordinal for `global_order`. Neither QPC cadence nor global order is synthesized from `frame_sequence`; non-increasing ticks, ordinal overflow, or an epoch/frequency mismatch rejects the frame.
- `promote_unified_render_execution_order` is the sole portable merge. It consumes every runtime particle/entity output and every `StaticMapPassProgram` draw/clear exactly once. Missing, extra, repeated, unresolved, or type-mismatched operations reject the frame. It never infers order from catalog position or file names.

### Exact render-target registration and present-output identity

- `0x1420fddd0` is the exact window target producer. On success it calls
  `IDXGISwapChain::GetBuffer` with buffer ordinal zero and
  `IID_ID3D11Texture2D`, then calls `CreateRenderTargetView` with a null view
  descriptor and stores the resulting RTV at D3D backend `+0x40`. The backend
  device is at `+0x20`, swap chain at `+0x30`, ready marker at `+0x25c`, and
  the renderer resolves the backend at `+0x268`.
- A signature-gated post-return hook emits only after nonnegative `HRESULT`,
  ready marker one, and a non-null RTV. It invalidates the prior recreation
  generation and registers the transient RTV pointer as semantic kind
  `SwapChainBackBuffer`, ordinal zero. The portable receipt contains the
  actual texture/RTV descriptors, recreation generation and authenticated
  creation order, never a pointer.
- `0x1420fda10` is the generic texture/SRV/optional-RTV producer. Its source
  descriptor stores the allocated wrapper at `+0x70`; the wrapper stores the
  primary texture at `+0`, SRV at `+0x8`, and RTV at `+0x10`. Descriptor byte
  `+0xc` bit `0x2` gates RTV creation. Successful targets from this path are
  registered as opaque descriptor/order identities; no role or filename is
  inferred.
- The portable generic enum is
  `OpaqueRenderTarget { attachment_ordinal }`, using observed OM color-slot
  order at use. The existing `NamedRenderGraphResource` spelling is forbidden
  for this evidence because the engine registration emits no name. Likewise,
  `MainSceneColor`, `MainSceneDepth`, and `PostProcessColor` remain forbidden
  without a distinct proven engine semantic producer.
- Draw and clear observations resolve the currently bound RTV through this
  transient registry. Missing, stale, repeated, or generation-mismatched
  registration fails the authority join. This supplies the exact
  render-target registration/output-target producer previously listed as an
  XN15L residual.

The complete evidence and failed-job ledger is
`replay_interface/re_evidence/rendering_laws/XN15L/RENDER_TARGET_REGISTRATION_2026-07-19.md`.

### Exact hourly Present selection

- `0x140abeaa0` reads the current raw game hour from `CGameState+0x468`; the process root slot is image-base RVA `0x3316e90`. The slot is bound before hook installation, and each completed framed Present stable-reads the same pointer/hour into a pointer-free receipt. Menu/loading Presents with no active render frame are ignored rather than poisoning completeness.
- The collector selects exactly the latest completed Present for the authenticated target hour at or before its QPC cutoff. Candidate identities are sorted by QPC/global/frame order and hashed; the selected ordinal is `candidate_count-1`. A missing runtime join is a bounded `PendingIncomplete`, while unrelated runtime-only rows cannot block selection.
- Earlier same-hour candidates and ready stale/after-cutoff rows are exact-retired only after all joins, authority identities, epochs, and receipt hashes validate. Late same-hour Presents after sealing are retired at publication and folded into a fixed-size rolling SHA-256/count telemetry record. No per-Present audit queue remains; the paused-hour stress test retires more than `MAX_SUBMITS` without overflow.
- Installed live/static authority registries require exact ordered observation IDs for every discarded group. Even an empty group must prove the pending row is absent; only hook-only tests with both registries absent may use the empty shortcut.

### Strict d80 closure consumer and live lookup observer

- Compact domain 80 remains `production_map_content_snapshot_live`, schema `extractor.production-map-content-snapshot.v1`; it is distinct from d77 `ProductionRenderFrame`.
- `promote_resolved_map_content_index` turns an already authenticated effective-VFS winner inventory plus complete role dependency closures into the portable d80 snapshot. XN15L does not build that inventory or those closures in normal production.
- All 11 production roles are covered: terrain, water, rivers, borders, trees, fixed objects, rails, map names, unit models, lighting, and post-process. The test fixture proves 11 observations, zero omissions, and byte-stable encode/decode.
- The generic PdxMesh producer is not guessed into one role. It can promote separately as fixed objects and unit models only when each role has its own complete authenticated static dependency closure.
- Validation binds every `UsedContentKey` to the authority manifest and exact effective-VFS winner, verifies the producer/role law and closure digest, and rejects malformed, stale, incomplete, duplicate, or noncanonical inputs. There is no runtime filename classifier: five proven registration roots wrap the engine lookup and record its actual logical path/effect/program result. PdxMesh remains closure-only because the proven helper does not itself call the engine lookup.
- The extractor and replay layer remain independently distributable. Only the portable `.battleplan` content manifest/domain payload crosses the boundary; process pointers and hook state never do.

## Normal-production implementation closure and release disposition

The XN15L-owned normal producer path is now wired and mechanically frozen:

- `EarlyRenderProductionSession::latch_update` establishes the atomic target-hour/Present cutoff before heavy world or VFS reads. Publication gaps remain bounded and paused-hour/late-publication state is compacted rather than accumulated.
- The live hook emits the exact particle, selector-native entity, direct program/source-open, render-target registration, and effective-VFS texture-resource evidence described above.
- `EarlyRenderProductionSession::finalize_update` invokes `populate_exact_authority_observations` before strict selected-only finalization. The caller requests only exact admitted H4CT ranges, creates every live/static authority observation through the strict constructors, and rejects an incomplete content, program, texture, or target join. There is no non-test raw-population or caller residual left in this packet.
- The detailed source hashes, test receipts, and final producer provenance closure are frozen in `replay_interface/re_evidence/rendering_laws/XN15L/PRODUCTION_HOOK_COLLECTOR_FREEZE_2026-07-19.md`.

The remaining live gate is now executable as one autonomous transaction:
`extractor/native/scripts/Invoke-XN15LComposedAcquisition.ps1` owns the clean
early launch, prepared product-modset admission, d77 capture, independent replay
relocation, playback VFS validation, normal product composition, and native
D3D11 readback. `verify_xn15l_composed.py` independently recomputes the selected
frame/Present hashes and exact VFS/resource/view/unified-submit joins before it
admits the immutable pixel receipt. No manual semantic sidecar, calibration, or
human fixture labeling remains; see `XN15L/AUTONOMOUS_COMPOSED_GATE.md`.

This is not yet a production-readiness claim. Recorder-host/live-HOI4 end-to-end validation and the downstream production-render compose check remain mandatory release gates. Until they pass, scheduling/promotion state is not advanced by this report and `PROGRAM_STATUS.md` remains unchanged.

The retained RenderDoc census proves only the optional-stage/stream-output null law for its 1,266 matched final draws. RenderDoc resource identifiers and process pointers are capture-local and are not a reusable production catalog. Any unavailable or incomplete exact producer causes the frame/domain to be omitted or rejected; no vanilla fallback, captured-bundle substitution, or fabricated draw is permitted.

## Ghidra provenance

Every relied-on executable finding records its address, job, artifact path/hash, and job manifest hash.

The weather registry/factory rows use exact-10 replacement root
`SUP-02-D1-P1`: actual `shard-03`, normalized request SHA-256
`46efa0aeb27817c3956f8a6b668cd95f4d61ad789389c2fc11aefdeb55b97a46`,
identity-manifest SHA-256
`cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe`,
job-receipt SHA-256
`be93c49db0d0a5d7b44ddfd36bf19af38604a919c4474879447144f58d5a7a9a`,
and exact-10 pool-status SHA-256
`df67a663059a645bf8f039f34659fed76274a7259af2fc137a2a60b5e8e41395`.
The earlier 2026-07-18 weather entity exports are discovery history only.

| Address / finding | Job ID | Artifact | Artifact SHA-256 | Manifest SHA-256 |
|---|---|---|---|---|
| `0x142400520` binder; `0x1423fb3e0` cache reset | `20260718T050055Z-xn15-live-render-wrapper-implementations-68de09ce` | `C:\Users\Example\ghidra_agent_exports\20260718T050055Z-xn15-live-render-wrapper-implementations-68de09ce\evidence\func_142400520.decompile.c`;<br>`C:\Users\Example\ghidra_agent_exports\20260718T050055Z-xn15-live-render-wrapper-implementations-68de09ce\evidence\func_1423fb3e0.decompile.c` | `9f61f8c2b7e659fe0bfa363a49bbb7026a22ab951a37cfba0234a6d3adcb0462`;<br>`4e8465554d359c7b5ba638ce12e908963eed82caf0165df7977a319f525807a8` | `2f53a83153cf594dba5384b4424f33789bec3dbab0052a5eea0667ef1c948fb6` |
| `0x142400520` hook sentinel | `20260718T060943Z-xn15-a729-shader-program-binder-hook-sen-e918b868` | `C:\Users\Example\ghidra_agent_exports\20260718T060943Z-xn15-a729-shader-program-binder-hook-sen-e918b868\datdump.txt` | `1ecf66ae443791dff62ba055efdcaac73e32810522f3c8844188faff1f147630` | `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` |
| `0x1423fe970` non-indexed final draw | `20260718T053155Z-xn15-live-particle-final-draw-and-dynami-1f10f06b` | `C:\Users\Example\ghidra_agent_exports\20260718T053155Z-xn15-live-particle-final-draw-and-dynami-1f10f06b\evidence\func_1423fe970.decompile.c` | `020f5b043c46a69d824069bf49d2e960126fc9d9b84fa35cfd0f5a3172671688` | `5f8151427cafd6b89bd824adebda07a1f6d988dd61156455d34a8feac6c30c87` |
| `0x1423fe9b0` indexed final draw | `20260718T043411Z-xn15-live-entity-final-draw-target-v1-a3f2b785` | `C:\Users\Example\ghidra_agent_exports\20260718T043411Z-xn15-live-entity-final-draw-target-v1-a3f2b785\evidence\func_1423fe9b0.decompile.c` | `f69e886b37c9709d0db3ffd1f2e296f5fef652d8582bc556fae56c7eccc4fcde` | `125cea24e6ef541536a2b10faf54a6e00469c9182ada777302078a11adec134b` |
| `0x1422644c0` exact entity selector-native traversal and draw binding | `20260718T145822Z-xn15l-entity-render-selector-native-cont-3c40f2e3` | `C:\Users\Example\ghidra_agent_exports\20260718T145822Z-xn15l-entity-render-selector-native-cont-3c40f2e3\evidence\func_1422644c0.decompile.c` | `45bc2290199dda4a5c69c84f4aaca7d2c55230ce8a16b130035b0938094f989b` | `f6313554e1e2121bd80881bf9f484c823523928b0994bbad2f85e5f98093dcfb` |
| `0x14227e5f0`, `0x142278710`, `0x142277df0`: named registry lookup, named factory, definition-backed instance factory | `20260719T223856Z-wave2-r10-sup-02-d1-p1-ea60087c` / actual `shard-03` | `<local-path>`;<br>`<local-path>`;<br>`<local-path>` | `e0d1de209dc5e05a802459e62728d55a38ee7802fa416a16a022395b1531e73c`;<br>`102de8d042162939ea2baa68c5945d428c90ec53cc7d825a0f296936d56e6292`;<br>`655d63fefd2610ec5fda27f75dea7bc34bb69fd4841f832f37ec9aaf418f3680` | `d50e88d325dc098c7e676de879563e99de6b689d6ab58e5cb094320b1c0903e4` |
| `0x142273fb0` entity instance construction and definition retention | `20260719T223856Z-wave2-r10-sup-02-d1-p1-ea60087c` / actual `shard-03` | `<local-path>` | `6bc8d1149a2a9ce76da3e24663f57410a4931d50c1a84f912ae10c9c69960963` | `d50e88d325dc098c7e676de879563e99de6b689d6ab58e5cb094320b1c0903e4` |
| `0x1423fe9f0`, `0x1423fea40` instanced final draws | `20260718T073219Z-xn15l-final-draw-wrapper-neighborhood-46096f2c` | `C:\Users\Example\ghidra_agent_exports\20260718T073219Z-xn15l-final-draw-wrapper-neighborhood-46096f2c\evidence\func_1423fe9f0.decompile.c`;<br>`C:\Users\Example\ghidra_agent_exports\20260718T073219Z-xn15l-final-draw-wrapper-neighborhood-46096f2c\evidence\func_1423fea40.decompile.c` | `32015b2d9c222f7b0e372a179c8cb82622fbc56347313f4a38c8a69196b92b90`;<br>`0ef69050306bce7e3e9aa85d8c604e0c893d998c60da88f199314a06ef46e11c` | `4ab20d72637d35582433f56ca8ec51531a50f412d79d66b1d0932672803afd25` |
| `0x1423fe9f0`, `0x1423fea40` instanced final-draw sentinels | `20260718T073321Z-xn15l-instanced-draw-wrapper-sentinels-0a7cf262` | `C:\Users\Example\ghidra_agent_exports\20260718T073321Z-xn15l-instanced-draw-wrapper-sentinels-0a7cf262\datdump.txt` | `e37ff5aa295506e1d27f83cfd848a04b23653d55913668e17d0e434ddf8bddda` | `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` |
| `0x1423fca30` combined color/depth/stencil clear | `20260718T075701Z-xn15l-render-clear-wrapper-identities-3dd27aac` | `C:\Users\Example\ghidra_agent_exports\20260718T075701Z-xn15l-render-clear-wrapper-identities-3dd27aac\evidence\func_1423fca30.decompile.c` | `aeb6b97b9e8f8a852d6e9d5a4fe562c1b80892d146becca28217993831fd722c` | `0dcb44fd16e93376392c2843b54d71e1e2b9f3cde8b345f10ccec53ade13f106` |
| `0x1423fca30` hook sentinel | `20260718T075723Z-xn15l-render-clear-wrapper-sentinel-be263ed3` | `C:\Users\Example\ghidra_agent_exports\20260718T075723Z-xn15l-render-clear-wrapper-sentinel-be263ed3\datdump.txt` | `6d4aa8c67a6f8ca2fa699bbde8994b96a0f14d39be0399008da1bd459ae8b8d9` | `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` |
| `0x1427115a0` default RGBA; `0x14270e270` default depth | `20260718T080141Z-xn15l-render-clear-default-values-19e035e7` | `C:\Users\Example\ghidra_agent_exports\20260718T080141Z-xn15l-render-clear-default-values-19e035e7\datdump.txt` | `905072a26023e36120eabb663b7e4f78f4b6e3ca0f5109a58b977bd19bb4454a` | `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` |
| `0x140b481a0` complete map-scene order root | `20260718T075411Z-xn15l-map-scene-complete-order-root-56bf08c1` | `C:\Users\Example\ghidra_agent_exports\20260718T075411Z-xn15l-map-scene-complete-order-root-56bf08c1\evidence\func_140b481a0.decompile.c` | `7f8ac48964e8cbfb06bab728b622cfd5d470e91cea1aaee696e82934f41a4e18` | `95874ba5d3fba58f140890ef98503f240928f644cf3f15a4d882ef1fba3fd251` |
| `0x140ff3450` post-process order root | `20260718T072133Z-xn15l-postprocess-orchestrator-order-roo-58d04981` | `C:\Users\Example\ghidra_agent_exports\20260718T072133Z-xn15l-postprocess-orchestrator-order-roo-58d04981\evidence\func_140ff3450.decompile.c` | `aec4798090118441f4552c142c56e765aa4db4f30ec984d443d2ca160854ffec` | `57b21b765b84ac23248bb9bf9fe1fd74609d03b4020df0895dcf6d890151cd48` |
| `0x1420b6530` present root | `20260718T072134Z-xn15l-present-string-xref-root-11f93e7b` | `C:\Users\Example\ghidra_agent_exports\20260718T072134Z-xn15l-present-string-xref-root-11f93e7b\evidence\func_1420b6530.decompile.c` | `9422f6a52bfe79ba118e0056785c6e12b774379e812ca3ed3b7fc483c8389ced` | `237df1fd7f656ab7dd8682e268ae9995436552844baaae2d07d7a60e82c3037e` |
| `0x140b481a0`, `0x140ff3450`, `0x1420b6530` phase sentinels | `20260718T072733Z-xn15l-map-post-present-hook-sentinels-2e7e284d` | `C:\Users\Example\ghidra_agent_exports\20260718T072733Z-xn15l-map-post-present-hook-sentinels-2e7e284d\datdump.txt` | `1301a39afcef62e05225f26d374131632023953e259e9825a12e85da7309ed91` | `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` |
| `0x1423f4ec0` successful engine shader/effect lookup | `20260718T065652Z-xn15l-engine-shader-effect-lookup-8ad679e2` | `C:\Users\Example\ghidra_agent_exports\20260718T065652Z-xn15l-engine-shader-effect-lookup-8ad679e2\evidence\func_1423f4ec0.decompile.c` | `541efd607875b2d33d0b31d4300025a4efe88fb8ec5f7c2ce08b6e1a3ce94e99` | `1e5a23ec3b272067e3ed3314e1b815a8eb4b77b3e0045a56cf7d60fd0b398826` |
| `0x1423f5d60` exact successful-index to `0x38`-byte program entry; negative global fallback excluded | `20260718T044320Z-xn15-live-entity-shader-program-lookup-v-8f109985` | `C:\Users\Example\ghidra_agent_exports\20260718T044320Z-xn15-live-entity-shader-program-lookup-v-8f109985\evidence\func_1423f5d60.decompile.c` | `e682e92ac0f7776c153cecfe44d0577dbce644c2a8eae9f5f25dd6f7af2f684c` | `4bf26c3ccc65591c1eaf1cb1e86b4ed09b00974c34c0b42ddf6bf05748b937b3` |
| `0x141261100` map-effect registration root | `20260718T065445Z-xn15l-map-effects-registration-root-53ffe3de` | `C:\Users\Example\ghidra_agent_exports\20260718T065445Z-xn15l-map-effects-registration-root-53ffe3de\evidence\func_141261100.decompile.c` | `5c1e4e991c813e6181ae9b499701a934d62ddbea859eb02800e744b6b02f9858` | `3436b30e16be63593f6d4688c95fc2fdbfe9db932ce0d28497ede799553822fa` |
| `0x14159b9b0` railroad registration root | `20260718T065447Z-xn15l-railroad-shader-registration-root-8a53ffc8` | `C:\Users\Example\ghidra_agent_exports\20260718T065447Z-xn15l-railroad-shader-registration-root-8a53ffc8\evidence\func_14159b9b0.decompile.c` | `3e626ee67354824916f80e030f624801ddbfca2659e7493dc8d9f455af947e0f` | `4da77046061d420010fe66ecb013d1cc61719d0f33d7ba3da9367db85d06bd5a` |
| `0x140b40890` map-name registration root | `20260718T065447Z-xn15l-mapname-render-root-0c1f02ea` | `C:\Users\Example\ghidra_agent_exports\20260718T065447Z-xn15l-mapname-render-root-0c1f02ea\evidence\func_140b40890.decompile.c` | `8d8ae79b13d4e98f41fe39d541b969462d322f55039f213fdee93c95aae8b9be` | `b2e8b27885de7b786a94198d6c327cd7b2c4823238e454e4846771372f4bd613` |
| `0x140ff1410` restore/bloom registration root | `20260718T065448Z-xn15l-restore-bloom-postprocess-root-1c36f5f0` | `C:\Users\Example\ghidra_agent_exports\20260718T065448Z-xn15l-restore-bloom-postprocess-root-1c36f5f0\evidence\func_140ff1410.decompile.c` | `11b4041035694fbc286988a057b974fa9289307585a6d979e6dc976d4f095f1a` | `10ae209aeb4802a8b9bcc969b7820ebff568bee30f1d85e3973cc58b57604189` |
| `0x14126f590` LUT-blend registration root | `20260718T065449Z-xn15l-lut-blend-postprocess-root-8a6b0b07` | `C:\Users\Example\ghidra_agent_exports\20260718T065449Z-xn15l-lut-blend-postprocess-root-8a6b0b07\evidence\func_14126f590.decompile.c` | `4b3829d10b3aa73be02e8a71116b5acf7a0df2be2e54626eef0d5988b641ca4c` | `89e0d2b1bb381b09ed54104e5761dda6bb0dcee0943eb61e157082fc5c4fb33b` |
| `0x1423f4ec0` lookup plus `0x141261100`, `0x14159b9b0`, `0x140b40890`, `0x140ff1410`, `0x14126f590` hook sentinels | `20260718T070257Z-xn15l-map-lookup-and-root-hook-sentinels-c9506392` | `C:\Users\Example\ghidra_agent_exports\20260718T070257Z-xn15l-map-lookup-and-root-hook-sentinels-c9506392\datdump.txt` | `6b3f04ea5fe2c3f02a8415e3f771976f3e58ca5d6f22e40900cd006ef2183f1d` | `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` |
| `0x14224ba50` generic PdxMesh registration helper | `20260718T072852Z-xn15l-second-pdxmesh-registration-root-841bfe28` | `C:\Users\Example\ghidra_agent_exports\20260718T072852Z-xn15l-second-pdxmesh-registration-root-841bfe28\evidence\func_14224ba50.decompile.c` | `4d522b35da89a9325aa97815c11dab8da9ea4e1fed3048ec1303b0c7a4d7c7f3` | `62541baa6359d069515cdd2df49cf7f25fa2c1ec94467993e489e414363aead9` |

| `0x140abeaa0` Present-hour producer; root slot image-base RVA `0x3316e90`, raw hour at `CGameState+0x468` | `20260717T101342Z-a29-effective-ahead-producer-candidates--f8665dfd` | `C:\\Users\\Example\\ghidra_agent_exports\\20260717T101342Z-a29-effective-ahead-producer-candidates--f8665dfd\\evidence\\func_140abeaa0.decompile.c` | `f4a0760325858c61040c3f071c557290d9232356ee4ad434e11d8774a0c700ef` | `89137474708ad3c772421278a73bafdb91cd888f488356a1d3e9e7fd71a836d1` |

## Discovery ledger

The following completed jobs were useful negative-space evidence and are preserved rather than discarded:

| Addresses / finding | Job ID | Artifact | Artifact / manifest SHA-256 |
|---|---|---|---|
| `0x1423ffab0`, `0x1423ffc90`, `0x1423ffdb0`: target initialization and command queue, not clear wrappers | `20260718T074908Z-xn15l-render-clear-wrapper-neighborhood-1dd218bd` | `C:\Users\Example\ghidra_agent_exports\20260718T074908Z-xn15l-render-clear-wrapper-neighborhood-1dd218bd\evidence\_FUNCTION_EVIDENCE_MANIFEST.json` | `1218539f20624594292c7a0a886c81702974aa2c9d901aa4671621311197bb74` |
| `0x1423fcae0`: adjacent destructor/helper, not a second clear path | `20260718T075701Z-xn15l-render-clear-wrapper-identities-3dd27aac` | `C:\Users\Example\ghidra_agent_exports\20260718T075701Z-xn15l-render-clear-wrapper-identities-3dd27aac\evidence\func_1423fcae0.decompile.c` | artifact `285bc3eb161d3d3ef10fae36f8715d3ec524af12e6ca101a4183432f4a889ffe`; manifest `0dcb44fd16e93376392c2843b54d71e1e2b9f3cde8b345f10ccec53ade13f106` |
| `0x1423fd9f0`, `0x1423febe0`, `0x1423fdb80`, `0x1423fec10`, `0x1423fdcd0`, `0x1423fddc0`, `0x1424005a0`, `0x142400fe0`, `0x1423ff2d0`, `0x1423fdd10`: renderer state/destructor wrappers | `20260718T075105Z-xn15l-render-dispatch-identities-2-d6de33ad` | `C:\Users\Example\ghidra_agent_exports\20260718T075105Z-xn15l-render-dispatch-identities-2-d6de33ad\evidence\_FUNCTION_EVIDENCE_MANIFEST.json` | `ae4eab2f9030bd5e2d49ece473312500838a5313ef0f09edc74a1beee605117b` |
| `0x142400920`, `0x1423fc920`: shader/buffer identities | `20260718T075241Z-xn15l-render-dispatch-identities-1c0-bb5720fb` | `C:\Users\Example\ghidra_agent_exports\20260718T075241Z-xn15l-render-dispatch-identities-1c0-bb5720fb\evidence\_FUNCTION_EVIDENCE_MANIFEST.json` | `1406355a1418c86141f0b7da983c6f3b488dcfe3e5a364f0cb8ab74b308ce07d` |
| `0x1423fde50`, `0x1424013e0`: shader/buffer identities | `20260718T075245Z-xn15l-render-dispatch-identities-1c1-d6c2c827` | `C:\Users\Example\ghidra_agent_exports\20260718T075245Z-xn15l-render-dispatch-identities-1c1-d6c2c827\evidence\_FUNCTION_EVIDENCE_MANIFEST.json` | `bbfe763468ac59548c578d4843c33e7f0d5b52a64f8f30cabbe3543228e05815` |
| `0x1423ff2e0`, `0x1423fdbc0`: shader/buffer identities | `20260718T075244Z-xn15l-render-dispatch-identities-1c2-1fdae844` | `C:\Users\Example\ghidra_agent_exports\20260718T075244Z-xn15l-render-dispatch-identities-1c2-1fdae844\evidence\_FUNCTION_EVIDENCE_MANIFEST.json` | `b9c46c1a3ad7920a7bfdd549caddaf4e52e9bdbfc6735e389aba6da6c847fbd0` |
| `0x1424002a0`, `0x1423fda30`: shader/buffer identities | `20260718T075254Z-xn15l-render-dispatch-identities-1c3-336813ac` | `C:\Users\Example\ghidra_agent_exports\20260718T075254Z-xn15l-render-dispatch-identities-1c3-336813ac\evidence\_FUNCTION_EVIDENCE_MANIFEST.json` | `36c5871820c9331c3ac341629944e5c2e73fbdda5782dafd6dcf892fd404f232` |
| Pointer table including `0x1430a7728 -> 0x1423fca30` | `20260718T075003Z-xn15l-render-dispatch-complete-pointer-t-9e2b3f03` | `C:\Users\Example\ghidra_agent_exports\20260718T075003Z-xn15l-render-dispatch-complete-pointer-t-9e2b3f03\datdump.txt` | artifact `ba90281c13ebf17c70cfa65bc5b2630acec83945afa78bdd71a02a2b29f3b59b`; identity manifest `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` |

Failed exploratory jobs produced no artifacts and are not proof: `20260718T075105Z-xn15l-render-dispatch-identities-1-588ba821` (`0x1423fcfb0` is not a function), `20260718T075225Z-xn15l-render-dispatch-identities-1b-27f14087` (`0x1423fca20` is not a function), `20260718T072609Z-xn15l-d3d-clear-final-operation-discover-3ed3292c` (regex failure), and `20260718T072135Z-xn15l-d3d-present-wrapper-root-f9a4e69f` (`0x142423bf0` decompile failure).

The following older jobs remain unused by XN15L because they were not part of
the final mechanical revalidation set. Their artifacts are retained, but no
artifact semantics or hashes from them support a claim or enter an evidence
manifest: `20260718T091834Z-xn15l-d77-shader-creation-and-compile-ro-42232380`,
`20260718T091900Z-xn15l-d77-shader-root-0-b92deebb`,
`20260718T092200Z-xn15l-d77-shader-root-1-60a7387b`,
`20260718T092501Z-xn15l-d77-shader-root-2-8c32f0ab`,
`20260718T092502Z-xn15l-d77-shader-root-3-5330dd54`, and
`20260718T092648Z-xn15l-d77-dx11-creation-string-anchors-2dc5f19a`.

The three later VFS-open jobs were mechanically revalidated on the current
authoritative 20-shard pool after `ghidra_status` confirmed `ready=true`, the
pinned a729 program/image base, and executable SHA-256. `ghidra_list_jobs` was
called before artifact reads. They are now valid discovery/provenance evidence:

| Address / finding | Job ID | Artifact path | Artifact SHA-256 | Manifest SHA-256 |
|---|---|---|---|---|
| `0x140a53290` open-consumer | `20260718T130406Z-xn15l-authority-vfs-open-candidates-fdb67dfb` | `C:\Users\Example\ghidra_agent_exports\20260718T130406Z-xn15l-authority-vfs-open-candidates-fdb67dfb\evidence\func_140a53290.decompile.c` | `e7cc28fbe6189a785bd9b85f0299f718ea642536c18d51f7b0fa7e5828331a76` | `f0eb5d8a32c8486c398fe862b38487baf499fbbd0753b621d125ef7d19c57dac` |
| `0x1424f1b80` asset append/open consumer | `20260718T130406Z-xn15l-authority-vfs-open-candidates-fdb67dfb` | `C:\Users\Example\ghidra_agent_exports\20260718T130406Z-xn15l-authority-vfs-open-candidates-fdb67dfb\evidence\func_1424f1b80.decompile.c` | `b7694683e4c192224648d2272df78060399813a8cd2a8ee1f41ad4dbe090009a` | `f0eb5d8a32c8486c398fe862b38487baf499fbbd0753b621d125ef7d19c57dac` |
| `0x1424cb7d0` VFS-open error producer | `20260718T130406Z-xn15l-authority-vfs-open-candidates-fdb67dfb` | `C:\Users\Example\ghidra_agent_exports\20260718T130406Z-xn15l-authority-vfs-open-candidates-fdb67dfb\evidence\func_1424cb7d0.decompile.c` | `b06fb89d7f65e14aad3709711a219e6ff14ee0cd2d77419e3cb4332446251d7c` | `f0eb5d8a32c8486c398fe862b38487baf499fbbd0753b621d125ef7d19c57dac` |
| `0x1424ca2f0` archive-file constructor/open path | `20260718T130406Z-xn15l-authority-vfs-open-candidates-fdb67dfb` | `C:\Users\Example\ghidra_agent_exports\20260718T130406Z-xn15l-authority-vfs-open-candidates-fdb67dfb\evidence\func_1424ca2f0.decompile.c` | `830737f2a4283a5d66a129b7a5c4376300e6e435d92065a6c20b9fe183f35a4e` | `f0eb5d8a32c8486c398fe862b38487baf499fbbd0753b621d125ef7d19c57dac` |
| `0x140a53290`, `0x1424f1b80`, `0x1424cb7d0`, `0x1424ca2f0` VFS/open string anchors and xrefs | `20260718T130257Z-xn15l-authority-vfs-open-producer-anchor-00427e53` | `C:\Users\Example\ghidra_agent_exports\20260718T130257Z-xn15l-authority-vfs-open-producer-anchor-00427e53\string-xrefs.txt` | `1a9243a631601275456cc4d20f9cbaf7565d1b41ea534fd1dd7a9ca9761530b9` | `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` |
| `0x1424ca2f0` constructor sentinel bytes | `20260718T130730Z-xn15l-vfs-open-constructor-sentinel-6f1642f4` | `C:\Users\Example\ghidra_agent_exports\20260718T130730Z-xn15l-vfs-open-constructor-sentinel-6f1642f4\datdump.txt` | `980c08a5e23d9988fa8268f55df1c2873baa02e35f54c34f6a217e7568ad212e` | `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` |

The dedicated revalidation receipt is retained at
`replay_interface/re_evidence/rendering_laws/XN15L/GHIDRA_REVALIDATION_2026-07-18.md`.

## Mechanical verification

- `cargo test --locked -p production_render_submit_live_lane --lib`: **55/55 passed twice** on the frozen source.
- `cargo test --locked -p production_render_catalog_live_lane --lib`: **2/2 passed** after the runtime-content migration.
- `cargo check --locked -p collector`: **passed twice** after the normal live/static caller was wired (warnings only).
- Portable production-render core: **15/15 passed twice**, with the full core suite also passed twice; receipt SHA-256 `33dbb85d7b2b68af480d5a0ba203d6be92443b063b3230b04e61b2270c17ca5b`.
- `cargo fmt -p production_render_submit_live_lane -- --check`: **passed**.
- `cargo test -p compact_contract --lib`: **passed twice**.
- `verify_xn15l.py`: **passed twice** with a byte-identical v2 omission-state receipt.

Frozen source hashes and the compact release gate are recorded in `replay_interface/re_evidence/rendering_laws/XN15L/PRODUCTION_HOOK_COLLECTOR_FREEZE_2026-07-19.md` and `replay_interface/re_evidence/rendering_laws/XN15L/manifest.json`.
