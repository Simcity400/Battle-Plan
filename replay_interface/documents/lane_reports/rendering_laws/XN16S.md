# XN16S — exact scene recipe for the two remaining XN16 captures

Status: **PROVEN PREPARATION — RECIPE FROZEN; CAPTURES STILL ABSENT**

**Current production disposition: PROVEN ADMITTED BY XN16Q.** The recipe remains historical-scene parity tooling, not a production prerequisite. Refreeze receipt `b0770adfbad10cea753f3cb4b937502889ec2b24dfb4940722e4836bc5958545`; verification `68409cbd92453c520929720d8ec6095154ac33af1fba4d1f42c8a5e5f1b67662` (88/88); manifest `aa7b15297e672d2fa651eb22e6d0ddbca6d226090d81e5b6a92e77c140b5b480`. Ghidra function/job/artifact/manifest: `null`.

XN16S freezes the exact-a729 game-state and suspended-launch recipe for the two inputs that XN16W and XN16R proved absent. It does not claim either capture exists, does not duplicate the XN16W2/XN16R2 admission validators, and does not weaken their gates.

## Frozen recipes

Both shots use the exact executable and continue-save environment recorded in `scene_recipe.json`: `hoi4.exe` SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`, `autosave.hoi4` SHA-256 `177e19d8fd67bb4a5b3bcd4e73dd2e1c0038940238ac05ea2840e0aafe7300c0`, no enabled mods, the frozen DLC/settings files, suspended early injection, and immediate root pass sequencing. The four moments are exactly terrain, water, rivers, trees. The evidence-derived limits are 8,192 draws/pass, 8 GiB/session, 30,000 ms capture work/frame, 1 GiB writer queue, 8,000 ms settle, 30,000 ms pre-resume, 240,000 ms dwell, and `frame-repeat` tree close. Terrain chunks, delayed arming, map-frame rounds, camera injection, and console injection are forbidden.

- `XN16S-R1` / `XN16W2_SYNCHRONIZED_TREES_MEASURE`: terrain draw 6 and water must share complete scene-CB0 FNV `619059c647782be4`; rivers and trees must share complete scene-CB0 FNV `9c2d9a7336c291e4`. The required shader pairs are frozen per pass. All four passes must come from one clean round and XN16W2 must return `ADMIT_EXACT_SYNCHRONIZED_TREES_MEASURE`.
- `XN16S-R2` / `XN16R2_FINAL_FULL_RIVER_PASS_CLOSE`: terrain and water bundles must equal the frozen `FINAL_full` SHA-256 anchors `77168591...f18d0` and `98e75ef1...6dcd2b`; every river draw must use shader pair `8eb490dfbd97857c/5be8efce15bc2c80` and complete scene-CB0 FNV `5866dccf4c7cb126`. At least 60 river draws, `pass_close`, and the source-local 1920×1080 format-10 one-sample 16,588,800-byte post-pass RTV are mandatory. XN16R2 must return `ADMIT`.

## Scene law recovered

`trees_measure` is not a single-camera scene. Its authenticated terrain/water CB0 uses camera position bits `4538e000/44480000/44af0000`; its rivers/trees CB0 uses `4529b6ef/43ae08ed/44c29959`. The historical launch naturally transitions between those profiles while the continued save initializes. Reproduction must therefore preserve the original immediate launch choreography and authenticate the full 416-byte CB0 after capture. Matching only position or matrices is insufficient because the same buffer also carries global time, FoW and day/night state.

The harness's `set_camera` path fails closed with `camera_object_unresolved`; `console_exec` also fails closed because its argument-vector/main-thread marshal is unresolved. Neither is an allowed substitute. A mismatched launch is discarded as a whole; passes may not be spliced across attempts.

The old `FINAL_full` run reached 60 serialized river draws but failed when 12,047 ms exceeded its 12,000 ms per-frame cap. The 30,000 ms limit is not a visual fudge: an authenticated later exact-a729 four-pass run closed terrain, water, rivers and trees with that value.

## Mechanical freeze

`verify_xn16s.py` passed **115/115** checks. It rehashes the executable, save/configuration, shipped game shaders, capture sources/binaries, prior bundles/logs/events, downstream validators and successful control; recomputes SHA-256 and FNV-1a64 for all four 416-byte reference CB0s; compares their raw `uint32` field layouts; and rejects forbidden command mechanisms.

- Recipe: `replay_interface/re_evidence/rendering_laws/XN16S/scene_recipe.json`, SHA-256 `3a7f499533e456dcb07e3765931c9aeb871b32620f0ee5570467e6eaa6eaeada`
- Verification: `replay_interface/re_evidence/rendering_laws/XN16S/verification.json`, SHA-256 `d7e7bca2d5abb3be6cbf1b17d8c771432ea7d6e78b307bf8ae8234b1349c5007`
- Evidence manifest: `replay_interface/re_evidence/rendering_laws/XN16S/manifest.json`, SHA-256 `c945bcb7e7c40d7257025dcf858dfa6ec25c28466c1e3d67001d5d58335129cc`

No collector, `PROGRAM_STATUS`, or adversarial-review surface was edited.

## Ghidra provenance

XN16S makes no new binary claim. Required tuple: function address `null`; job ID `null`; artifact path `null`; Ghidra manifest hash `null`. The reason is recorded in the recipe and evidence manifest. The XN16S evidence-manifest path and hash are recorded above.
