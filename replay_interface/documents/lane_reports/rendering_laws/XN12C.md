# XN12C — retained particle draw-to-asset correlation

Pinned target: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

Status: **PROVEN — promotion-ready**. The retained RenderDoc corpus, shipped
entity/event registrations, particle definitions, subsystem textures, shader
variants, fixed render state, ordering, and production identity rule are closed.
There are no XN12C capture-, data-, Ghidra-, or promotion-blocked fields.

## Promotion ledger

| ID | Proven law |
|---|---|
| XN12C-01 | The shipped static graph contains 503 exact definition/subsystem chains spanning 146 unique definitions, 180 registrations, 511 subsystem texture references, and 18,238 entity particle-event rows. XN12A reports zero parse errors, unresolved event particles, unresolved registration types, or unresolved texture paths. |
| XN12C-02 | The retained corpus contains 198 mined frames, all with exact filename-matched source RDCs. Exactly 88 frames carry immutable extracted texture contents and 110 were re-mined directly from their RDCs. The three RDC roots contain 199 captures because `hoi4_vanilla_frame3088546.rdc` has no compact mine and independently contains no SIMPLEPARTICLE draw. No mined frame is source-less or unscannable. |
| XN12C-03 | All 198 source-backed frames contain 24,647 exact asset-identified particle draws and 817,248 instances. Every extracted texture payload joins a shipped particle DDS; no exact-content draw is unmatched. The corpus observes 26 payload hashes from the 57 shipped particle DDS identities. |
| XN12C-04 | Particle draw identity is the exact six-index SIMPLEPARTICLE packet with VS constant-buffer sizes 64/16/96 and one of the two proven VS hashes: billboard `b88e161b8297ce4b`, no-billboard `edcd722913015e88`. The obsolete `non_building_particle` label is not used. |
| XN12C-05 | The recovered `captures` root supplies exact source RDCs for all 38 formerly generic vanilla mines: 28 particle-bearing frames with 2,160 draws / 205,886 instances and 10 zero-particle frames. All 38 were direct-converted; every one of the 2,160 draws now joins by texture bytes from its own RDC. Cross-capture RenderDoc ResourceId transport is forbidden because ResourceIds are capture-local opaque handles, not authenticated asset identities. |
| XN12C-06 | The terminal legacy analysis now contains zero legacy frames and zero excluded draws. All 24,647 retained draw rows are production-identity eligible exact payload joins; there is no residual or training/fixture exclusion bucket. |
| XN12C-07 | A static chain ID exactly binds definition source/name, subsystem index, shader, and texture to every registration and entity-event trigger row. Texture identity alone is not effect or trigger identity because textures are shared. Production therefore records stable effect/definition identity in the XN12B post-simulation packet stream; replay must never infer a semantic trigger from texture. |
| XN12C-08 | Fixed state is alpha `SrcAlpha/InvSrcAlpha`, additive `SrcAlpha/One`, pre-alpha `One/InvSrcAlpha`, RGB write mask, depth enabled/write-off/`LESS_EQUAL`, back cull for billboard and no cull otherwise, linear/wrap sampling. Packets remain in engine order after map objects and before arrows/restore. |

## Production extractor/replay contract

At the proven `0x14227f420` submit boundary, serialize the XN12B
post-simulation packet stream in engine order. Each packet carries stable
effect/definition, material, pipeline, texture and bucket identities; exact
matrix and ordinal; exact live 48-byte instance records; atlas and fixed-state
selectors; and billboard camera/projection constants. The correlation table is
the validation oracle: exact texture payloads must join the shipped asset graph,
but a texture match must never replace the packet's stable semantic identity.

The 2,160 draws formerly lacking mined texture contents now carry source-local
payload hashes from their recovered RDCs. `legacy_join_analysis.json` remains as
a terminal mechanical proof that no source-less frame or excluded draw remains.

## Evidence and mechanical closure

- `correlation.json`: 198-frame exhaustive packet inventory and exact content joins.
- `raw_rdc_enrichment.json`: 110/110 direct RDC re-mines with texture subresources and draw state.
- `legacy_join_analysis.json`: zero-row terminal proof after complete source-RDC recovery.
- `summary.json`: compact promotion contract and per-frame/per-texture census.
- `evidence_manifest.json` SHA-256: `0bba65bf085a642b2a9164a552a6c1f1c01b075e888056e9789fc3f9e614cf32`.

Run:

`python replay_interface/re_evidence/rendering_laws/XN12C/verify_xn12c.py`

## Ghidra provenance

XN12C required no new Ghidra operation: retained captures plus shipped assets
close its correlation scope. The producer endpoint used by the promotion
contract is inherited from XN12B and recorded here with the required full tuple:

| Address / role | Job ID | Artifact path | Manifest SHA-256 |
|---|---|---|---|
| `0x14227f420` bind/draw/ribbon submit | `20260717T004345Z-xn12a-cpdxparticleobject-update-callers-98b14e19` | `C:\Users\Example\ghidra_agent_exports\20260717T004345Z-xn12a-cpdxparticleobject-update-callers-98b14e19\evidence\func_14227f420.decompile.c` | `a64495ae47dfaaa2a6f752ca1b5d96ec1f764041b68074ea8e79d88cea51f6d1` |

The complete inherited function tuple ledger remains machine-readable in
`XN12B/mechanical_result.json`; XN12C does not duplicate unrelated producer
internals.
