# XN13F — final XN13 production-promotion compose gate

Status: **PROVEN — PRODUCTION PROMOTION COMPOSE READY / FAIL CLOSED**

XN13F is the final admission gate for the complete XN13 production package. It
admits the owner-authored XN13P handoff, XN13V deterministic fixtures, and XN13S
storage contract as opaque immutable tuples. It does not restate, merge, or
re-serialize their schemas and performs no adversarial review.

Authority: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

## Admitted owner tuples

| Owner | Purpose | Owner tuple SHA-256 | Owner manifest SHA-256 | Verification SHA-256 |
|---|---|---|---|---|
| XN13P | production handoff | `9185da30cb005e4adc3e045e1a8f522f50e3f6f74b9d361c435bcfbab81a27ff` | `c90ba224063f200a3ff3af11cee5b85d92430d43546a851f2e4ffa32da6929c2` | `e7c3fc104e620d00e047adf8b4e792f56d28eada149df5dececb3db4d9170d6b` |
| XN13V | deterministic promotion fixtures | `c2c4b609f782e58df04ea136a7b73c5c9f84ffcf8b8d3d5283d80e8291073415` | `591c4c1cdb049bbd5c2075669865b6f5a55c5c62d0b357d6775441d481119626` | `f70287107e92497e4ddb5fcaf1eed210c05f4dd148008bc1f7dd84a805b8a01e` |
| XN13S | production storage contract | `66384ee6e3fb90418f36bcfb68d6e5d53aa800f39060bd8d15d2299c31779335` | `31189335a1bba162b8a0f4f67611541402d4f1483570d1e8dd721c940225daab` | `94d248689a489533174185b3c4d313d4ad495ce697f3ed4bdeb7b6cc15f5b946` |

Each tuple contains its owner report, owned contract/corpus, verifier, PASS
verification, manifest, and every additional artifact enumerated by that
manifest. XN13F re-hashes every file and requires the manifest's byte count,
path, and digest to match. All three owner dispositions are ready/PASS, share
the binary authority above, and declare zero residuals.

## Fail-closed compose law

Admission order is exactly `XN13P`, `XN13V`, `XN13S`. The verifier rebuilds the
compose contract from the live immutable owner tuples, requires canonical byte
equivalence with the frozen contract, then validates physical file hashes and
the per-owner aggregate tuple hashes.

Thirteen independent rejection probes prove exact failure for:

- a component-only contract without an owner manifest;
- a missing report or verifier;
- a manifest that omits its owned contract;
- owner artifact hash drift;
- draft or partial dispositions;
- failed owner verification or a residual;
- binary-authority mismatch;
- a missing or duplicate owner; and
- changed compose order.

The current verifier result is **PASS**, with 3/3 complete owners admitted,
13/13 rejection probes passing, and zero residuals.

## Incidental findings

The transition from incomplete to admissible was exercised directly. XN13S's
standalone `storage_contract.json` correctly failed with
`E_OWNER_MANIFEST_MISSING`; it became admissible only after its report, verifier,
PASS verification, evidence manifest, and manifest-listed artifacts were all
present and hash-consistent.

Owner manifests legitimately use two container keys (`files` in XN13P/XN13S,
`artifacts` in XN13V). XN13F normalizes only path, SHA-256, and byte count for
admission; it does not treat that surface difference as permission to rewrite
owner content.

## Federated Ghidra provenance

XN13F adds no binary claim. The decisive owner-boundary tuples are federated
unchanged. Each row records the required function address, job ID, artifact
path, and manifest hash.

| Function address / role | Job ID | Artifact path | Manifest SHA-256 |
|---|---|---|---|
| `0x14227b9a0` XN13I state-token lookup | `20260717T023441Z-xn13b-entity-state-event-runtime-2ced8259` | `C:/Users/Example/ghidra_agent_exports/20260717T023441Z-xn13b-entity-state-event-runtime-2ced8259/evidence/func_14227b9a0.decompile.c` | `eb46e2e95d1b7067f58139783c8fcd61c48536bfe74e1218ec0fe22134ddb725` |
| `0x14227ac70` XN13I node/attachment transform | `20260717T023459Z-xn13b-entity-visibility-node-transform-42ca1847` | `C:/Users/Example/ghidra_agent_exports/20260717T023459Z-xn13b-entity-visibility-node-transform-42ca1847/evidence/func_14227ac70.decompile.c` | `016d5dd925d197b6dd232b359cf059b8c6a8363c86898d42e0bbba76cb11602b` |
| `0x14165d5c0` XN13P/O05 building selection | `20260716T041136Z-o05-runtime-desired-count-validity-speci-7b04be6e` | `C:/Users/Example/ghidra_agent_exports/20260716T041136Z-o05-runtime-desired-count-validity-speci-7b04be6e/evidence/func_14165d5c0.decompile.c` | `2e744d3b5e94f2af4ad72e0ca1259c4808d3d0dc0684854dca3189626953e438` |
| `0x14127b2e0` XN13P/O05 building update/order | `20260716T052901Z-o05-mapbuilding-production-update-loop-v-ea7ac957` | `C:/Users/Example/ghidra_agent_exports/20260716T052901Z-o05-mapbuilding-production-update-loop-v-ea7ac957/evidence/func_14127b2e0.decompile.c` | `bcca6ce2eb8491d2df2c8a0562d14c29dcd7bc4486d1fff019235c42879cbb6b` |

## Promotion artifacts

| Artifact | SHA-256 |
|---|---|
| `replay_interface/re_evidence/rendering_laws/XN13F/build_compose_contract.py` | `e7501ac89442d1c93b74ae0092d3fb561e3adc62dc3a6946ce25eeca64838803` |
| `replay_interface/re_evidence/rendering_laws/XN13F/verify_xn13f.py` | `999f37e46f2df3ff3bbf02cb93c240bee0c440ef0ad0124eebfe8e7e408b966e` |
| `replay_interface/re_evidence/rendering_laws/XN13F/compose_contract.json` | `e35d87972f0373f6173e442cd08dcaead4954ed4a9e4ba58981e957ab2f4a236` |
| `replay_interface/re_evidence/rendering_laws/XN13F/verification.json` | `61112ff55a41ab136a75873251d41897720e34094b7cfb51ab59c790b70406ee` |

Residuals: **none**. XN13F has no data, capture, RE, or human-input blocker.
