# Final mechanical renderer event/composition audit

Date: 2026-07-17  
Scope: terminal mechanical integration only; no adversarial review, Ghidra call, external-capture mutation, or `PROGRAM_STATUS.md` edit.

## Result

**PASS.** The current-workspace runner executes exactly 36 gates:

- 29 direct passes;
- three exact gate18 legacy diagnostics classified fail-closed as non-applicable;
- zero remaining post-port adapter gaps;
- four exact external-capture gates;
- zero unexplained acceptance failures.

The promoted L02, L03, O05, N02, M01, UI06, UI09, and UI15 artifacts remain mechanically consumed. Ten exact T01 corpus gates pass; the two gate18-only legacy diagnostics are classified below. The machine result is `mechanical_result.json`.

O04 runs only through its relocation-safe operational wrapper. The wrapper maps the historical checkout and W20 index to current authenticated authority, hashes all 199 indexed raw RDCs in the current invocation, and then executes the byte-authenticated reviewed verifier including the 81-city/243-draw production closure.

All eighteen Rust-owned gates are serialized through `extractor/native/scripts/Invoke-CentralRustValidation.ps1`: five targeted library-test gates, twelve explicit `Evidence`-tier corpus gates, and one targeted renderer build. The fifth targeted gate replaces R02's stale packet-freeze verifier with the current production `river_world_synthesis` contract. They use the repository-owned `.cargo-target`; this audit never invokes Cargo directly or creates a competing target directory.

Authority is the current shared production replay tree at repository HEAD `2069d17b707320fe2551bced1581ecea9b812d2f`. Because the tree is intentionally dirty, the result pins every consumed production source and promoted artifact by byte count and SHA-256.

## Static-SRV production closure

The former descriptor-only static-SRV blocker is closed and bound to three exact authorities:

| Authority | SHA-256 | Result |
|---|---|---|
| `replay_interface/native/map_viewer/src/frame_bundle.rs` | `418c871d9a67591a4fd94b9435a6f51a49321374203ceebb99ff349bae7ebd52` | exact resolver-backed legacy SRV admission |
| `static_srv_resolver_join/verification.json` | `8ab3e7b7153ad189190a41a0169d63d2d4cdd24e40f03e7ebedbdd951ca8bd56` | PASS production adapter ready to promote |
| `trees_measure_legacy_join/manifest.json` | `875047b57fb58e8a9faa4d8b62e21d29f04ab11be2fb6f0372e181c9b852af01` | 56/56 descriptor joins; exact source/capture oracle frozen |

The loader resolves legacy descriptors only through unique, descriptor-exact game-file sources and fails closed on missing, ambiguous, or drifted sources. No screen-derived, placeholder, or nearest-name texture is admitted.

## Canonical dependency-complete graphs

The FoW gates execute the full captured `terrain -> water -> rivers -> trees` graph. Terrain-only and ad hoc hashes are forbidden.

| Capture | Graph | Result | Final captured hash |
|---|---|---|---|
| FoW off observer | terrain -> water -> rivers -> trees | PASS | `1b554185e7b90769` |
| FoW on POV | terrain -> water -> rivers -> trees | PASS | `5a79c3503df3a519` |
| `live_trees_measure` | retained terrain -> trees only | exact external-capture dependency gate | none; runner refuses an incomplete graph |

`live_trees_measure` lacks `map_frame_water_pass_bundle.json` and `map_frame_rivers_pass_bundle.json`. The runner does not execute the incomplete graph and does not generate or accept a terrain-only hash.

## Remaining exact external-capture gates

| Capture | Exact gate |
|---|---|
| `live_FINAL_full` | rivers pass was exported active and has no required pass-close output-merger payload |
| `live_PAN2` | first depth-tested draw lacks captured RTV and DSV clear provenance |
| `live_PAN_chunks` | one preserved 23,091,200-byte water SRV payload denies read and ACL inspection; no bytes were substituted |
| `live_trees_measure` | canonical water and rivers pass bundles were not retained |

`gate18_final` is identity-pinned by its map-frame bundle, terrain-chunk library, and capture-event stream. Its 64/64 vSnap gate remains a positive result. The legacy LOD auxiliary-path omission, current mesh-synthesizer mismatch, and missing current shader/source provenance are three fail-closed non-applicable diagnostics; gate18 is not a final-frame or current mesh byte-equality oracle. These and the four external-capture gaps are the only non-PASS gates. None is a post-port code or adapter gap.

## Production and packet gates

- pass graph: 8/8;
- shadow law: 19/19;
- terrain LOD/stateful composition: 11/11 active tests;
- O05 fixed-object admission: 4/4;
- renderer build: PASS;
- the three complete preserved T01 corpora: LOD, vSnap, and byte-level mesh PASS;
- pinned legacy `gate18_final`: vSnap PASS plus the three fail-closed diagnostics described above;
- L02/L03/O05/N02/M01/UI06/UI09/UI15 promotion gates: PASS.

M03 still requires exact A48 producer input, and O04 still requires one synchronized frame that submits a city `WorldMatrix` draw. Their evidence verifiers pass, but this report does not relabel them as final replay parity.

## Freeze discipline

`manifest.json` is allowlist-built from the current 36 gate records. It freezes the result, 72 referenced logs, current runner/control files, current side results, all promoted packet artifacts, the static-SRV verification, and the trees oracle manifest. Superseded mesh/vSnap, T01, UI09, zero-test production, and earlier side-result artifacts remain explicitly excluded.

The UI15 repair uses the runner's fail-closed `--resume-failed-packet-gates` path. That path first authenticates the prior 36-gate result, every prior stdout/stderr receipt, all 34 untouched accepted gates and their promoted artifacts, current source/runner authority, the renderer binary, the static-SRV closure, and the pinned gate18 inputs. The only accepted-gate artifact drift it permits is M01's explicitly declared regenerable `verification_result.json`; its attestation, contract, Rust source, Rust test, and original gate logs must remain byte-exact. It also binds the preserved O04 stdout to the exact PASS side result for all 199 raw RDCs. Before rerunning M01 and the failed UI15 verifier, it archives both replaced gate records, both pairs of original logs, and byte snapshots of every replaced promotion artifact. If UI15 alone remains failed, the idempotent second stage authenticates that entire first-stage chain, requires the rerun M01 gate and all other accepted gates to remain exact, archives the current UI15 row/logs/artifacts under distinct stage-two names, and reruns only UI15. Rust, capture-corpus, renderer-build, and O04 gates cannot be resumed through either stage.

A separate fresh-audit repair mode is identity-locked to the 47,545-byte input result with SHA-256 `bd4c992508078e0a3305cd4a39832e4dd1c15387036f9e516267ffa609acc24c`: 26 direct passes, three gate18 diagnostics, four external-capture gates, and exactly the failed L02, L03, and UI09 packet rows. It authenticates every accepted row and the preserved O04 proof, archives those three failed rows/logs/artifacts with the `fresh-l02-l03-ui09-` prefix, and reruns only their lightweight packet verifiers. The manifest finalizer requires that exact input identity and receipt set; it does not generalize the exception to any other gate or audit state.

If those three verifiers and logs were already replaced but publication of `mechanical_result.json` was interrupted, recovery performs no rerun and overwrites no archived provenance. It requires all three prefixed old gate rows/logs/artifact snapshots to reproduce the frozen failed input, independently requires each current L02/L03/UI09 output and side result to be an exact PASS bound to its current promoted artifacts, reconstructs only those three PASS rows, and atomically publishes the same resume payload with `aborted_resume_recovered_without_rerun=true`.

## Ghidra provenance

No Ghidra operation or new executable finding was used in this integration rerun. Function address: none; job ID: none; artifact path: none; manifest hash: none. Promoted packet manifests retain their packet-owned provenance.
