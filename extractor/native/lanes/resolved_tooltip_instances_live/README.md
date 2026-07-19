# A26 resolved tooltip instances

Status: `PROMOTION READY — EXACT A729 PRODUCER, SCHEDULABLE d72, FROZEN FOLD`

The extractor-side carrier and exact a729 producer are implemented. `src/a729_hook.rs` signature-gates the generic tooltip controller at RVA `0x002A4080`, reuses A25's sole shared final-setter and deleting-destructor detours, and emits copied values only. It never places process pointers in replay. Schedulable domain 72 contains the exact frame key, final engine rectangle, viewport, producer identity, ordered final text fragments, three-state row availability, producer-manifest authentication, and a complete-frame record/count contract. `recorder_host::world_fold` admits it under `_r2_resolved_tooltip_instances_live`, preserves authenticated measured-empty frames, rejects wrong-hour attachment, and emits no inferred layout or text semantics.

RenderDoc, OCR, static localisation, replay-side Clausewitz evaluation, cursor-offset rules, inferred edge flips, and reconstructed modifier breakdowns remain rejected as production sources. The controller supplies its already-resolved position after the engine's own anchor, flip, margin, viewport, and clamp branches. Final setter text is split only on literal newline boundaries and carried as `resolved` fragments so A26 does not guess header/value/breakdown/scripted authoring semantics. An authenticated frame with zero visible instances is measured-empty; a missing frame is unobtained.

## Mechanical result

- Focused hook/carrier tests: `6/6` pass, including exact round trip, authenticated measured-empty, completeness/key/geometry/order rejection, row-availability rejection, unauthenticated-source rejection, and newline-only final-fragment preservation.
- `cargo metadata --no-deps --format-version 1`: pass after consolidating the lane manifest to one dependency table.
- `cargo check -p resolved_tooltip_instances_live_lane --lib`: pass with the Windows hook, A25 shared-observer dependency, MinHook, and exact a729 sentinels compiled.
- Recorder integration: focused domain-72 fold tests pass `2/2`, covering all instance/row/provenance fields, authenticated measured-empty, and wrong-hour fail-closed behavior.
- Compact registry: domain 72 is `validated_slice`, schedulable, and mechanically identical between `compact_contract` and `domain_registry.json`.
- Production handoff schema: `resolved_tooltip_instances` declares the same frame, key, rectangle, row, availability, authentication, and completeness fields.

## Retained RenderDoc findings

The retained captures authenticate placement and chrome only; they do not supply semantic row identities or a general producer.

| Capture | Constant-buffer translation | Independently derived screen rectangle |
|---|---:|---:|
| `hoi4_frame37048.rdc` | `(0.2739583552, -0.5166666508)` | `[1223, 671, 297, 148]` |
| `hoi4_frame54162.rdc` | `(0.2895833552, -0.9259259105)` | `[1238, 876, 291, 164]` |

Both RDCs, all four retained vertex blobs, both constant-buffer ZIPs, and both selected constant-buffer entries are rehashed by the verifier. At byte offsets 48/52, the translation values reproduce the rectangles using `x=(tx+1)*960` and `y=(1-ty)*540-height`. The capture oracle also establishes a 54-vertex, 18-pixel-border dialog chrome built from `gfx/interface/tiles/tiles_dialog.dds`; this is useful validation evidence but is outside the semantic producer contract.

## Exact producer implementation

The live implementation supplies, for every admitted frame:

1. the exact instance-key set and completeness record;
2. each final engine-resolved rectangle and viewport;
3. stable producer record identity;
4. every final evaluated text fragment in strict newline order, including dynamic/scripted breakdown text without inventing semantic classes; and
5. explicit `resolved`, `not_applicable`, or `data_blocked(field)` availability.


## Ghidra report tuple

- Function address: `0x1402A4080`
- Job ID: `20260717T113739Z-a25-ctextbox-vtable-identity-accessors-7ddacf03`
- Artifact path: `evidence/func_1402a4080.decompile.c` (SHA-256 `a1838600f7f016606ed6287926ef47d5b836a5501bc83ef59467c5dd61a1c071`)
- Ghidra evidence-manifest SHA-256: `2bc2bc389cfcadb3903ad6c79a03645a5fe4b7102feac1eb982f89e6aabac9c5`
- Manifest SHA-256: `89897f3ac71c7cf8c81c4238bbeabfcb81146efefbc3363f568109323844df2a`

