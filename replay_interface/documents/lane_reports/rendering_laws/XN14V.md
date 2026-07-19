# XN14V — deterministic weather production fixtures

Status: **PROVEN - PRODUCTION FIXTURE CORPUS FROZEN**

Pinned binary: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

XN14V freezes a deterministic positive and negative fixture corpus for the already-frozen XN14C/P production boundary. It does not change, restate, or weaken the upstream schema. Authored values come from hash-pinned XN14A game-file extraction; retained carrier observations come from hash-pinned XN14B late-load captures. Only sequence/update indices, stable instance IDs, and packet ordinals are fixture-only structural values.

## Positive closure

The corpus covers eleven production domains with zero residuals:

- code zero with a null weather token and every ordered position explicitly empty;
- exact one-big/two-small position coverage from strategic region 8;
- sandstorm big instantiation plus both small positions explicitly empty, proving no vector fallback;
- primary/fading entity-to-effect joins and primary-then-fading ordered effect IDs;
- rain-heavy lightning and shared-cloud definitions;
- blizzard storm and storm-cloud definitions;
- all five camera branches (`near_hidden`, `near_fade`, `full`, `far_fade`, `far_hidden`), including the asymmetric primary-write rule and unconditional fading writes;
- monotonic spawn/activate/deactivate/stop-emission/destroy lifecycle with parent-before-child and child-before-parent teardown;
- CPU effect identity as the packet semantic join, never shared-atlas texture identity;
- rejection of serialized capture-local RenderDoc ResourceIds;
- checkpoint and cold-seek equality.

The production specimen contains four state/membership cases: `code_zero_null`, `rain_heavy_lightning_cloud`, `blizzard_storm_cloud`, and `sandstorm_no_small_fallback`. Every case emits all three region-8 positions in source-line order. Every instantiated member is checked against the exact authored variant, entity, particle registration, and definition chain; every explicit-empty member must carry null variant/entity fields and an empty effect-ID array.

## Exact retained carrier anchors

The carrier rows are deterministic first matches in the frozen XN14B late-load anchor. They are capture cross-checks, not semantic identity sources and not substitutes for production packet bytes.

| Case | Pass/frame/pass draw | Chunk | Instances | Texture payload FNV-1a64 | VS / PS FNV-1a64 | Classification |
|---|---|---:|---:|---|---|---|
| lightning/rain | `6 / 1864 / 1846` | `4141` | `154` | `69b7cf7c57c1d712` | `edcd722913015e88` / `abe38fd19ed78391` | exclusive weather carrier |
| lightning cloud | `6 / 1876 / 1858` | `4288` | `17` | `741c5a7f3fc7bdcf` | `b88e161b8297ce4b` / `abe38fd19ed78391` | shared atlas, CPU join required |
| blizzard storm | `6 / 1899 / 1881` | `4614` | `45` | `ca90e58d3fa33ef9` | `edcd722913015e88` / `de579ae13f1dfa35` | exclusive weather carrier |
| blizzard cloud | `6 / 1933 / 1915` | `4989` | `20` | `b4ee9596703831b1` | `b88e161b8297ce4b` / `de579ae13f1dfa35` | shared atlas, CPU join required |

Each compact anchor also carries a deterministic SHA-256 of its complete retained blend/depth/raster/sampler/CB/SRV/target/viewport state. The fixture builder recomputes those digests from the frozen source on every run.

## Fail-closed corpus

All 29 negative fixtures reject with fixed error codes. They cover unknown schema/binary/code; code-token and flag-priority disagreement; code-zero instantiation; missing/reordered positions; sandstorm small fallback; unknown variants/entities; missing, duplicated, or reordered primary/fading effect joins; invalid visibility branch/alpha/playback/write behavior; shared-atlas relabeling; capture-local ResourceId transport; missing packet joins; orphaned/reordered/invalid lifecycle operations; and checkpoint/cold-seek drift.

This is a validator corpus, not a source of visual constants. No arbitrary packet bytes, transforms, alpha values, textures, shaders, or pipeline states were invented. The visibility results are derived mechanically from the exact XN14A piecewise laws and encoded as final IEEE-754 single-precision bit patterns.

## Inherited executable provenance

XN14V issued no duplicate Ghidra jobs. Its executable claims inherit the complete frozen tuples used by XN14C/P:

| Function address / role | Job ID | Artifact path | Manifest SHA-256 |
|---|---|---|---|
| `0x140f0bd60` region transition/type/expiry | `20260717T010100Z-xn14a-weather-state-transition-workers-b3770842` | `C:\Users\Example\ghidra_agent_exports\20260717T010100Z-xn14a-weather-state-transition-workers-b3770842\evidence\func_140f0bd60.decompile.c` | `97ee87421ce17726a486eb4529cb7422cb70d2b1942fe927bee335c9f97929e8` |
| `0x14166ec50` membership/visibility/playback producer | `20260717T012439Z-xn14a-weather-region-getter-all-callers-e6227833` | `C:\Users\Example\ghidra_agent_exports\20260717T012439Z-xn14a-weather-region-getter-all-callers-e6227833\evidence\func_14166ec50.decompile.c` | `fd99de197bf7699319a522e0d2e68dbd63a36d9a778ded8ca368597d9679d9c8` |
| `0x1424dbe60` wall-clock seed source | `20260717T004246Z-xn12b-particle-prng-seed-init-min-14613a5d` | `C:\Users\Example\ghidra_agent_exports\20260717T004246Z-xn12b-particle-prng-seed-init-min-14613a5d\evidence\func_1424dbe60.decompile.c` | `faabb041c2eaa89bdc9a93a81535073aa5189d0e09a9322f46ea613b8b9ad1d5` |
| `0x1423dd730` packet emission | `20260717T004108Z-xn12b-particle-instance-render-update-co-75672612` | `C:\Users\Example\ghidra_agent_exports\20260717T004108Z-xn12b-particle-instance-render-update-co-75672612\evidence\func_1423dd730.decompile.c` | `b8482288abf63d70128ddd097186c3cba5c7e9e8eb48d906df3cd5b00670192d` |
| `0x14227f420` bind/draw submit | `20260717T004345Z-xn12a-cpdxparticleobject-update-callers-98b14e19` | `C:\Users\Example\ghidra_agent_exports\20260717T004345Z-xn12a-cpdxparticleobject-update-callers-98b14e19\evidence\func_14227f420.decompile.c` | `a64495ae47dfaaa2a6f752ca1b5d96ec1f764041b68074ea8e79d88cea51f6d1` |

## Evidence and verification

- `replay_interface/re_evidence/rendering_laws/XN14V/build_fixture_corpus.py`
- `replay_interface/re_evidence/rendering_laws/XN14V/fixture_corpus.json`
- `replay_interface/re_evidence/rendering_laws/XN14V/verify_xn14v.py`
- `replay_interface/re_evidence/rendering_laws/XN14V/verification.json`
- `replay_interface/re_evidence/rendering_laws/XN14V/manifest.json`

Run `python replay_interface/re_evidence/rendering_laws/XN14V/build_fixture_corpus.py`, then `python replay_interface/re_evidence/rendering_laws/XN14V/verify_xn14v.py`. The verifier rebuilds the entire corpus from the frozen inputs, checks all positive domains, executes every negative mutation, and emits the manifest only after all results match.

Incidental finding retained: the late-load capture has exclusive rain/storm carriers but both selected cloud anchors are non-exclusive shared-atlas draws. This independently reinforces the frozen CPU-identity rule. There is no data, capture, Ghidra, or human-input blocker. No upstream packet, collector, rendering implementation, or `PROGRAM_STATUS.md` file was edited. No adversarial review was run.
