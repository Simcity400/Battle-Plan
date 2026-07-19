# XN16W — `live_trees_measure` synchronized water+rivers recovery

Status: **PROVEN RETAINED-CORPUS ABSENCE — exact human capture required**

XN16W exhausted the retained native bundle ledger and every known raw RenderDoc
store for a water+rivers submission synchronized to the authenticated
`live_trees_measure` terrain/trees frame. No admissible candidate exists. This is
not a partial reverse-engineering result: the absence claim is complete and
mechanically verified. The production gate remains fail-closed because the two
required source payloads are genuinely absent.

## Exact result

- Authority: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, executable
  SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.
- Retained terrain identity: bundle SHA-256
  `0425152b54ffe75418dd95c95ab0de27faed4db109c437c0162f797350a2656d`,
  active draw 6, CB0 FNV-1a64 `619059c647782be4`, VS/PS
  `e5a0acbee3ae07a2` / `ce5e5185d46e214b`.
- Retained trees identity: bundle SHA-256
  `41af03bbbeadc73b775ea6d6e64d8b4348371d4c136f1a352707c663b7a54c04`,
  active-draw CB0 FNV-1a64 `9c2d9a7336c291e4`, VS/PS
  `02bd512fa7acd53f` / `277b7b5e8964214f`.
- Raw census: 214/214 RDCs, 69,298,844,274 bytes, 214 unique direct mines and
  215 mine variants. The scan inspected 2,966 primary and 2,982 all-variant
  416-byte scene-CB writes. Terrain CB matches: zero. Trees CB matches: zero.
- The terrain shader pair is present in 213 frames and the trees shader pair in
  146 frames. These carriers are recorded, but none has the exact retained
  scene-CB identity. Shader presence therefore cannot authenticate the frame.
- Filename/session-label hits for `live_trees`, `trees_measure`, `water_pass`,
  or `rivers_pass`: zero. The closest raw RDC timestamp is still
  80,922,876,522,500 ns outside the retained preservation-mtime window; those
  mtimes are preservation metadata, not a substitute for scene authentication.
- The retained ledger contains five water and five rivers bundles, but the
  `live_trees_measure` directory contains exactly the terrain and trees bundles.
  Its water and rivers paths are absent.

The normative result and complete per-RDC/path/hash/mine-artifact ledger are in
`replay_interface/re_evidence/rendering_laws/XN16W/absence_manifest.json`.
`verify_xn16w.py` passes 35/35 checks with status
`PASS_PROVEN_RETAINED_CORPUS_ABSENCE_CAPTURE_REQUIRED`.

## Authenticated replacement frame

The exact-a729 `20260717T000008Z-production-complete-frame` round is preserved as
an incidental useful finding. It contains complete terrain, water, rivers and
trees bundles, 773 manifest artifacts, and passes its production renderer and
three pass validators. It is not admitted for this gate: terrain/water CB0 is
`fa02e482948dd2ba` and rivers/trees CB0 is `0ef8464072d2312f`, so neither matches
the retained frame's `619059c647782be4` / `9c2d9a7336c291e4` identities. Its
capture-local RenderDoc ResourceIds are never transported into the retained
frame.

## Exact capture card

One capture round at the unique `live_trees_measure` camera/state must contain
terrain, water, rivers and trees. It must authenticate the terrain and trees
CB/shader identities above and retain exact ordered draw arguments, shader
bytecode/reflection, all IA/CB/SRV/sampler bindings and payloads, pre-draw and
post-pass output-merger payloads, RTV/DSV clear provenance, the ordered event
stream, exact executable identity and artifact hashes. Cross-frame,
cross-camera, ResourceId-based, inferred or visually tuned substitution is
forbidden.

## Provenance and frozen tuple

This is a raw/native capture recovery lane, so Ghidra function address and job
ID are both **N/A**; no Ghidra artifact was used. Artifact paths and hashes are
fully enumerated by the evidence manifest.

- Normative result: `absence_manifest.json`, SHA-256
  `b0ba436fb195c152094df60b5f06783c07f45bca20f03bf6b88216efca660b0d`.
- Verification: `verification.json`, SHA-256
  `0f3cfd77c1d2b8517c9ce1bf9a1e0322a52694d90174aeb2e688bc5073a28434`.
- Evidence manifest: `evidence_manifest.json`, SHA-256
  `977c3fc6079634b59daa98f9826a56e7c765da844164f8f0d86d18d24fdcc368`.

XN16W changes no renderer, extractor, collector, shared integration file, or
`PROGRAM_STATUS.md`.
