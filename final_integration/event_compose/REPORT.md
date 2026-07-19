# Final mechanical renderer event/composition audit

Date: 2026-07-17  
Scope: terminal mechanical integration only; no adversarial review, Ghidra call, external-capture mutation, or `PROGRAM_STATUS.md` edit.

## Result

**PASS.** The current-workspace runner executes exactly 36 gates:

- 31 direct passes;
- one pinned known-negative capture excluded from acceptance;
- zero remaining post-port adapter gaps;
- four exact external-capture gates;
- zero unexplained acceptance failures.

The promoted L02, L03, O05, N02, M01, UI06, UI09, and UI15 artifacts remain mechanically consumed. All twelve exact T01 corpus gates pass. The machine result is `mechanical_result.json`.

Authority is the current shared production replay tree at repository HEAD `2069d17b707320fe2551bced1581ecea9b812d2f`. Because the tree is intentionally dirty, the result pins every consumed production source and promoted artifact by byte count and SHA-256.

## Static-SRV production closure

The former descriptor-only static-SRV blocker is closed and bound to three exact authorities:

| Authority | SHA-256 | Result |
|---|---|---|
| `replay_interface/native/map_viewer/src/frame_bundle.rs` | `c6919d0a0984b7640133292b15d61cbda5674aae1b144fafed9991a80109c1e5` | exact resolver-backed legacy SRV admission |
| `static_srv_resolver_join/verification.json` | `468035b76a967ad1c070967faa7d2199d0e2e1e20b4f7e452bf1e326beff932d` | PASS production adapter ready to promote |
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

`gate18_final` remains the pinned TerrainIDMap negative control. These are the only non-PASS gates. None is a post-port code or adapter gap, and no final-frame equality claim is made for them.

## Production and packet gates

- pass graph: 8/8;
- shadow law: 19/19;
- terrain LOD/stateful composition: 11/11 active tests;
- O05 fixed-object admission: 4/4;
- renderer build: PASS;
- all four preserved T01 corpora: LOD, vSnap, and byte-level mesh PASS;
- L02/L03/O05/N02/M01/UI06/UI09/UI15 promotion gates: PASS.

M03 still requires exact A48 producer input, and O04 still requires one synchronized frame that submits a city `WorldMatrix` draw. Their evidence verifiers pass, but this report does not relabel them as final replay parity.

## Freeze discipline

`manifest.json` is allowlist-built from the current 36 gate records. It freezes the result, 72 referenced logs, current runner/control files, current side results, all promoted packet artifacts, the static-SRV verification, and the trees oracle manifest. Superseded mesh/vSnap, T01, UI09, zero-test production, and earlier side-result artifacts remain explicitly excluded.

## Ghidra provenance

No Ghidra operation or new executable finding was used in this integration rerun. Function address: none; job ID: none; artifact path: none; manifest hash: none. Promoted packet manifests retain their packet-owned provenance.
