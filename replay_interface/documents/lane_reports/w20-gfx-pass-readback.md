# W20 graphics pass-final replay readback

Status: **DIAGNOSTIC SEAM IMPLEMENTED; PIXEL PAIR NOT EXECUTABLE FROM RETAINED
REPLAY INPUTS**.

This lane changes no production pass order and introduces no fitted constant.
It performs no live capture and uses no Ghidra output. UI, panels, HUD, and
minimap work are out of scope.

## Grounded reason for the seam

The immutable Political post-carrier fix-v2 authority is commit
`22b6a82390432e8d04840428dbc9156213870813`. Its report states that all 22
eligible Political RDCs pass the retained-evidence gates and contain
event-scoped Terrain, Rivers, Water, and final MainScene boundaries, with
engine order `Terrain -> Rivers -> Water` in 22/22 frames. The report is
`C:\Users\Example\hoi4_lanes\w20-gfx-political-localize\replay_interface\documents\lane_reports\w20-political-postcarrier-localize.md`,
SHA-256
`682b36ce5a4f4bdfdf4fb85fe83f88f02b06c9095c4f63da92729ed604da65c1`.

Its retained corpus index is
`C:\tmp\w20-political-postcarrier-corpus-fix.json`, 76,981 bytes, SHA-256
`5bb91399b54f74bb86a372b5c25cef4b34b613aef33996dedeca184e230d53d6`.

The retained worst-frame engine manifest is
`C:\tmp\w20-postcarrier-boundaries-fix\postfx_frame161848\manifest.json`,
schema `hoi4-political-postcarrier-boundaries-v2`, 4,310 bytes,
SHA-256
`c880102193e52127b8deac5f8eb2c4461d2ff593f5895edfff1a680143b71737`.
It records four 1920x1080 `R16G16B16A16_FLOAT` exports, each 16,588,800
bytes, at exact event-scoped pass-final boundaries. The exporter used
last-write-before-draw state, not an end-of-frame substitute.

The v2 manifest binds those exports to source RDC
`C:\Users\Example\AppData\Local\hoi4_renderdoc\session_2026-07-10\postfx_frame161848.rdc`,
136,277,310 bytes, SHA-256
`bb7225e7c6d458b871b8f3c6bd1dc1d181db073fe713a832287224c6262836c6`.
The four export SHA-256 values, in event order, are:

- `terrain`: `55e17ccbfebd0459644c39c6726407f598abd1881fe1de13c92079c0f1a5c456`;
- `rivers`: `76d316a457bcccbe6b75a3e52383572bc65555d494980c96e3b400bddfc68f9e`;
- `water`: `54252c0114bc2bf7fee37e003d895977c63282af80fdfb921bcd7f8d264e2508`;
- `main_scene_final`: `26369cbc670b1d3c9721df596cc5e02107e08d553d5088b9027b551522a7e08d`.

The engine v2 manifest records width, height, format, byte length, and hashes;
it does not expose the source Texture2D's MSAA descriptor. This lane therefore
does not claim that missing source descriptor as mined fact. Its replay join is
narrower: the exported referee is one tight RGBA16F element per pixel, and the
diagnostic API refuses any retained-authority request or replay target other
than one mip, one array slice, and D3D11 single-sample `1:0`.

Replay still appends Water before Rivers in both prepared-map lists
(`d3d11_replay.rs:1273-1280` captured and `:1365-1372` synthesized). That is
the proven schedule mismatch. It is not yet proof that changing order improves
pixels, so this lane deliberately does not reorder production.

## Implemented diagnostic contract

`NativeD3d11TerrainRenderer::render_prepared_production_map_with_pass_final_diagnostics`
and
`render_prepared_synthesized_production_map_with_pass_final_diagnostics` are
explicit opt-in APIs. Normal captured, synthesized, and profiled rendering pass
no diagnostics session and remain on their prior paths.

The diagnostic entry point:

1. requires both Rivers and Water;
2. requires one mip, one array slice, `sample_count == 1`, and
   `sample_quality == 0` for both the retained export authority and replay
   target before rendering;
3. requires exact equality across width, height, mip count, array size, DXGI
   format, sample count, and sample quality;
4. renders the existing replay graph without changing its order;
5. uses the existing native D3D11 staging/readback path after Terrain,
   after the Rivers and Water passes at their actual execution positions, and
   after the complete graph as `main_scene_final`;
6. writes tight-row binary payloads plus a v2 `manifest.json` with the full
   target descriptor, row pitch, byte count, SHA-256, FNV-1a-64, boundary
   identity, and replay execution-order index;
7. rejects a runtime readback whose descriptor differs from the supplied
   engine-export descriptor, rejects duplicate boundary captures, and refuses
   to emit a complete manifest unless all four named boundaries were captured;
8. accepts only an absent or empty output directory. A stale manifest or any
   partial artifact blocks the rerun before rendering, and `manifest.json` is
   published only after all four boundary entries exist.

Boundary identity is explicit and independent of sequence position, so the
engine's `terrain/rivers/water/main_scene_final` exports can be paired by name
against replay even while their schedules differ.

## Typed fail-closed states

Code-visible preflight states:

- `missing_required_pass { pass: rivers }`;
- `missing_required_pass { pass: water }`;
- `retained_authority_not_single_sample { expected_target }`;
- `retained_authority_not_single_subresource { expected_target }`;
- `replay_target_not_single_sample { actual_target }`;
- `replay_target_not_single_subresource { actual_target }`;
- `target_descriptor_mismatch { expected_target, actual_target }`;
- `output_path_not_directory`, `output_directory_not_empty`, or
  `output_directory_unreadable`.

The helpers `native_d3d11_pass_final_readback_blockers` and
`native_d3d11_synthesized_pass_final_readback_blockers` expose these before a
GPU render. If any state is present, no pass is rendered and no successful
manifest is emitted.

Residual proof states:

```text
PoliticalPostCarrierReplayBoundaryPairNotExecuted
ProductionPassOrderChangeUnproven
```

The first closes only after the orchestrator invokes the applicable diagnostic
entry point on the exact-GB replay clone and compares all four payloads with
the retained engine exports. The second forbids changing product order until
the paired boundary result attributes the first unequal stage.

## Execution audit

The engine side is available and hash-bound, but the paired replay execution
is not currently runnable from this lane:

- no physical exact-GB clone is persisted at the W16 script's default
  `.scratch_gb_copy` path;
- the available `replay_app.exe` predates this seam; and
- no `replay_app` or `map_viewer` CLI call site invokes either new diagnostic
  entry point.

The prepared-map API also does not retain verified executable, bundle-file, or
GB-blob paths/hashes. Consequently this seam cannot itself bind a readback
manifest to those identities. The future executable caller must hash and
record them; caller-asserted strings are not accepted here as proof.

### 2026-07-15 resumption audit

The earlier statement that the source Wave-14 capture still existed is no
longer true and is superseded by this audit. The two immutable GB blobs and the
W16 copy script still exist, but both capture trees named by the recorded
scoreboard row are physically absent:

- `C:\Users\Example\AppData\Local\hoi4_captures\session_WAVE1_CAP3_parity_scoreboard_overlay_w14b`;
- `C:\Users\Example\AppData\Local\hoi4_captures\session_WAVE1_CAP3_parity_payload_join`.

The copy script is
`C:\Users\Example\hoi4_lanes\w16\analysis-parity-residuals\regate_exact_gb_carriers.py`.
Its default source is the first missing tree, so it cannot reconstruct the
replay inputs from the remaining GB blobs. A current map-bundle directory is
present at the recorded path (164 files, 532,069,452 bytes), but the recorded
command does not bind that directory to a manifest/hash set. It is therefore
not an exact replay-input authority.

The retained engine side was rehashed during this audit:

- the two retained RDC sessions contain exactly 88 and 72 RDC files;
- the source RDC, boundary manifest, corpus census, three mined artifacts, and
  all four RGBA16F exports reproduce every byte length and SHA-256 embedded in
  the v2 manifest;
- the exact export event/chunk identities are Terrain `265/2040`, Rivers
  `1853/3628`, Water `1898/3673`, and MainScene final `7208/8983`, all on
  resource `158434`;
- the recorded replay command is 832 bytes, SHA-256
  `af1cb05a0de6c23349e9078f590dedd4be57238bcb2ab436a0fc31ea954fad29`;
- the initial resumption audit observed a fallback release executable of
  7,544,832 bytes, SHA-256
  `7d0d025b42b4fc380c42574417d05d2d756dcb5d9a44080dcdd2c9e8f5d0df5a`,
  which predated the diagnostic seam; a later disk-cleanup recheck found both
  that fallback and the recorded command executable physically absent, so the
  current blocker is `replay_executable_missing` and the contract retains the
  former hash only as the unavailable expected identity;
- current main `75b02e7a0e9bbde01131eb9960520dfed09cb717` still has definitions/tests
  for the diagnostic methods but no `replay_app` or `map_viewer` executable
  call site.

`replay_interface/renderdoc/verify_pass_final_readback_authority.py` now
rehashes the retained engine authority and fails closed on all replay-side
paths and identities declared by
`w20-gfx-pass-readback-authority.json`. Its current expected result is not
ready, with the capture and capture-base paths missing, map-bundle identity
unbound, both recorded/fallback executables absent, executable invocation proof
unavailable, and no executable call site promoted. The
checked-in result is
`w20-gfx-pass-readback-readiness.json`. It never launches replay and cannot
change production order.

The bounded Mode-A fix removes caller booleans as proof. The verifier pins the
accepted engine-manifest byte length/SHA-256 in code, validates the exact
frame, descriptors, event/chunk/resource identities, strict event order,
terrain-consumer carrier last-write provenance, and export containment. Any
future capture or bundle identity must use a canonical
`hoi4-full-tree-identity-v1` manifest that enumerates every non-manifest file
with sorted relative path, byte length, and SHA-256; extra, missing, duplicate,
traversal, or symlink entries fail closed. This version intentionally has no
accepted executable invocation-proof format, so it always retains
`exact_replay_invocation_proof_unavailable`; setting caller-supplied
`diagnostic_capable` or `diagnostic_callsite_present` values cannot make it
ready.

The remaining exact execution blocker is still
`PoliticalPostCarrierReplayBoundaryPairNotExecuted`. No numeric
first-divergence metric or pixel-stage attribution exists.

## Verification

- Focused unit gate: `cargo test -p map_viewer pass_final_readback --lib`
  (9 passed, 0 failed).
- Full library gate: `cargo test -p map_viewer --lib`
  (970 passed, 0 failed, 27 ignored evidence-environment gates).
- Readiness-gate unit tests:
  `python -B -m unittest test_verify_pass_final_readback_authority.py`
  (9 passed, 0 failed).
- Exact retained/replay readiness invocation:
  `python -B verify_pass_final_readback_authority.py --contract C:\Users\Example\hoi4_lanes\w20-gfx-pass-readback\replay_interface\documents\lane_reports\w20-gfx-pass-readback-authority.json`
  (expected exit 2 until every typed replay-side blocker closes).
- Repository diff hygiene: `git diff --check`.

Completion cleanup removed only verified lane-contained disposable outputs:
`replay_interface/native/target` (2,129 files, 3,846,648,976 bytes) and any
lane-local RenderDoc Python `__pycache__` (absent at removal). A subsequent
disk-emergency audit found three files/59,405 bytes under
`extractor/native/runs/crack_fanout/live_revalidation/__pycache__`, but they
are tracked repository content and therefore are not disposable. They were
restored byte-exact from immutable candidate `52110ed0` and are excluded from
net cleanup. The retained RDC sessions,
`C:\tmp\w20-postcarrier-boundaries-fix`, retained mined artifacts, tracked
repository files, and shared immutable carrier blobs were not modified or
deleted by cleanup.

The fresh Mode-A reviewer separately audited the lane-contained
`replay_interface/renderdoc/__pycache__` and `replay_interface/native/target`
roots after review; both were absent, so reviewer cleanup was 0 bytes. It
retained the candidate lane, `C:\tmp\w20-postcarrier-boundaries-fix`, and both
retained RDC sessions unchanged.

No capture request is justified: the four engine boundary payloads already
exist in the retained corpus.
