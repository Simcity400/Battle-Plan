# XN13S — Production replay storage and exact deduplication contract

**Status: PROVEN — STORAGE CONTRACT READY / FAIL CLOSED**

XN13S freezes the lossless storage boundary for the already-proven XN13I entity/ground presentation and XN13P building presentation handoffs. It is ready to promote into the production replay layer as a schema contract. It does not select a compression codec and does not change the extractor, collector, renderer, XN13P handoff, or XN13V fixtures.

## Frozen inputs

| Input | SHA-256 |
|---|---|
| XN13I report | `a4ff7a8c38210cfef61d34085add658779b4c513a7866f95f9019d877e516812` |
| XN13I integration contract | `5d55c81a395b98f778947dcf3b758b382163b47d0a024c434ad873ead71616fb` |
| XN13I manifest | `c16f512b09a0ae10fe270ba0c92ee1938f69882c568e1c9b338b7eefacf49f49` |
| XN13I verification | `7357ae5494a3c4d9e693a8186d29acfba64d5cfcc78d07a790d0b933b7613cc8` |
| XN13P report | `9beba756b78032024114c82356b78b7ec6aa4cb500414a3344a334f3d178c6fd` |
| XN13P production handoff contract | `abac9c3de7083775952df485c01c114bd453030da2dec7387d003107be50f9ed` |
| XN13P evidence manifest | `c90ba224063f200a3ff3af11cee5b85d92430d43546a851f2e4ffa32da6929c2` |
| XN13P verification | `e7c3fc104e620d00e047adf8b4e792f56d28eada149df5dececb3db4d9170d6b` |

Both upstream verifiers pass, their evidence is stable across two runs, and their residual sets are empty. XN13S adds no unresolved law.

## Exact serialization law

All integers use declared fixed widths and little-endian order. Floating-point values are stored as raw IEEE-754 bits, without decimal conversion or NaN normalization. Strings are exact upstream UTF-8 bytes with no case folding, Unicode/path normalization, or inserted terminator. Every record begins with an eight-byte `record_kind_u16`, `record_version_u16`, `payload_length_u32` header.

`BlobRefV1` is exactly 44 bytes:

```
sha256[32] | raw_length_u64[8] | collision_ordinal_u32[4]
```

SHA-256 and length select a candidate bucket only. Two payloads share an entry only after exact byte-for-byte equality. Different bytes in the same digest/length bucket receive distinct collision ordinals in canonical first-observation order. A digest is a locator and integrity check, never semantic identity or proof of equality. Stable IDs remain stable numeric IDs.

Blob entries serialize independently in ascending `(sha256 bytes, raw_length_u64, collision_ordinal_u32)` order. This never changes semantic record order. Eligible immutable payloads include exact strings, 3,200-byte palettes, immutable matrices, ordered authored-event bytes, XN12B particle packet bytes, 18,448-byte tiled-light submissions, and canonical checkpoint components.

## Record and ordering law

The six version-1 families are `frame`, `entity_presentation`, `building_presentation`, `state_transition`, `effect_light`, and `checkpoint`.

- Frames preserve increasing frame sequence.
- Entity rows preserve exact XN13I population order. Their pass variants remain `color_a`, `light_depth`, `main_hdr`; each is a one-byte `NotSkinned` tag or a one-byte `Skinned` tag plus a `BlobRefV1` to exactly 3,200 bytes.
- Building rows preserve O05 source insertion order. Each row owns one exact 64-byte world matrix; its three pass draws refer to `building_instance_id_u64` instead of repeating the matrix.
- State, event, effect, light, particle-packet, pass, draw, and lifecycle occurrences preserve engine order. They are never removed merely because their encoded bytes match. Only immutable payload bytes may be shared.
- One tiled-light payload is exactly 18,448 bytes: 16-byte grid constants, 16,384-byte index map, and 2,048-byte data map.
- Checkpoint epochs remain explicit and increasing. Set-like active populations sort by stable numeric ID, maps sort by canonical encoded key bytes, and ordered lifecycle/draw/packet arrays retain semantic order. A decoded checkpoint plus forward records must be byte-identical to linear playback.

Catalog references may avoid repeating immutable authored bytes, but never replace the actual trigger sequence/update/tick, stable IDs, attachment, or lifecycle operation.

## Exact retained-section accounting

Logical bytes below exclude filesystem, compression, allocator, container, and transport overhead. They are section-local comparisons, not a whole-replay size forecast.

| Retained section | Exact content-addressed / structural bytes | Inline baseline | Exact reduction |
|---|---:|---:|---:|
| XN13I pose palettes, including 1,239 mode tags | 502,775 | 615,639 | 112,864 |
| XN13P building matrix/reference section | 4,400 | 9,600 | 5,200 |

The palette calculation uses 1,239 draws, 192 skinned consumers, 1,047 explicit `NotSkinned` consumers, 152 byte-unique palettes, 3,200 raw bytes per palette, and 44 bytes per blob entry/reference. Its components are 1,239 tag bytes + 486,400 unique raw bytes + 6,688 blob-entry metadata bytes + 8,448 consumer-reference bytes. The inline baseline is 1,239 tag bytes + 614,400 repeated raw bytes. Exact byte-equality sharing avoids 128,000 raw duplicate payload bytes before reference metadata.

The building calculation uses 50 rows and 150 pass draws: `50*64 + 150*8 = 4,400` versus `150*64 = 9,600`. This is structural sharing by building-instance identity; it does not claim the 50 matrix byte strings are content-unique.

The frozen XN13P census also contains 358 visual events, 357 particle events, 11 resolved lights, one light-only event, 281 semantic static objects, and four admitted fixed objects. Those are recorded findings, not a size extrapolation: runtime lifecycle occurrence counts stay explicit.

Unknown production cardinalities remain symbolic:

- tiled light: `distinct_payloads*(44+18448) + references*44`;
- generic blobs: `sum(44+raw_length for each distinct exact blob) + references*44`;
- checkpoint: eight-byte record header + declared scalar widths + five 44-byte component references + 32-byte atomic digest, with component bytes counted in their blob entries.

No compression ratio, tuned threshold, dictionary benefit, or unmeasured whole-file reduction is claimed.

## Fail-closed boundary

Promotion must reject any upstream hash drift; unknown record kind/version/tag; width, endian, or raw-float mismatch; string normalization; palette/matrix/tiled-light length mismatch; digest-only deduplication; collision aliasing; stable-ID replacement; semantic occurrence removal; record/pass/lifecycle/checkpoint reordering; incomplete runtime trigger data; checkpoint cold-seek mismatch; or logical-byte omission/double counting.

## Frozen Ghidra provenance

No new Ghidra extraction was needed: XN13S serializes already-frozen XN13I/P fields and retains their producer-boundary evidence.

| Function | Role | Job ID | Artifact path | Manifest SHA-256 |
|---|---|---|---|---|
| `0x14227b9a0` | XN13I state-token lookup boundary | `20260717T023441Z-xn13b-entity-state-event-runtime-2ced8259` | `C:/Users/Example/ghidra_agent_exports/20260717T023441Z-xn13b-entity-state-event-runtime-2ced8259/evidence/func_14227b9a0.decompile.c` | `eb46e2e95d1b7067f58139783c8fcd61c48536bfe74e1218ec0fe22134ddb725` |
| `0x14227ac70` | XN13I node/attachment transform boundary | `20260717T023459Z-xn13b-entity-visibility-node-transform-42ca1847` | `C:/Users/Example/ghidra_agent_exports/20260717T023459Z-xn13b-entity-visibility-node-transform-42ca1847/evidence/func_14227ac70.decompile.c` | `016d5dd925d197b6dd232b359cf059b8c6a8363c86898d42e0bbba76cb11602b` |
| `0x14165d5c0` | XN13P/O05 building selection boundary | `20260716T041136Z-o05-runtime-desired-count-validity-speci-7b04be6e` | `C:/Users/Example/ghidra_agent_exports/20260716T041136Z-o05-runtime-desired-count-validity-speci-7b04be6e/evidence/func_14165d5c0.decompile.c` | `2e744d3b5e94f2af4ad72e0ca1259c4808d3d0dc0684854dca3189626953e438` |
| `0x14127b2e0` | XN13P/O05 building update/order boundary | `20260716T052901Z-o05-mapbuilding-production-update-loop-v-ea7ac957` | `C:/Users/Example/ghidra_agent_exports/20260716T052901Z-o05-mapbuilding-production-update-loop-v-ea7ac957/evidence/func_14127b2e0.decompile.c` | `bcca6ce2eb8491d2df2c8a0562d14c29dcd7bc4486d1fff019235c42879cbb6b` |

## Promotion artifacts

- `replay_interface/re_evidence/rendering_laws/XN13S/storage_contract.json`
- `replay_interface/re_evidence/rendering_laws/XN13S/size_accounting.json`
- `replay_interface/re_evidence/rendering_laws/XN13S/build_size_accounting.py`
- `replay_interface/re_evidence/rendering_laws/XN13S/verify_xn13s.py`
- `replay_interface/re_evidence/rendering_laws/XN13S/verification.json`
- `replay_interface/re_evidence/rendering_laws/XN13S/evidence_manifest.json`

**Residuals: none.**
