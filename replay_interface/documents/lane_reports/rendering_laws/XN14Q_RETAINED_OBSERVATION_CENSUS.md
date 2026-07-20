# XN14Q retained weather observation census

Status: **retained observations exhausted; live sequence receipt required**

Authority: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

The machine-readable result is `replay_interface/re_evidence/rendering_laws/XN14Q/retained_observation_census.json`. This pass did not run the game, RenderDoc replay, Ghidra, a build, a test runner, or a verifier. It inspected the retained authority, exported capture products, native-run records, and current hook source.

## Result

No retained artifact contains the authenticated initial-state, per-admitted-update visibility, and selected-submit sequence observations required by `XN14C/promotion_contract.json`. The static weather law is already complete; the remaining gap is a live observation and transaction-ownership gap. Production must continue to fail closed for active weather.

The retained sources close different, non-interchangeable slices:

- The retained RDC and XN15L stage census prove 1,266 ordered draws: 1,239 entity and 27 particle draws, with null optional geometry/hull/domain/compute stages and no stream-output targets. They do not contain the CPU strategic region, weather update index, `view+0x184` scalar, post-update entity state, write masks, CPU effect membership, hook epoch, or collector commit.
- XN14B's four direct anchors contain 9, 74, 22, and 0 weather-candidate draws. They prove exact D3D11 packet composition and capture-local order. The zero late anchor is a useful negative, not authority for an empty weather stream. Shared atlases also prevent texture-only identity.
- XN14V is explicitly a fixture corpus. Its sequence, update, entity, effect, and packet identifiers are fixture-only and cannot be promoted as raw observations.
- The retained native-run tree contains 1,588 files, including 113 compact streams, 22 collector logs, and 177 session manifests. Its domain-24 records are gameplay region/province weather state. A textual census across 1,066 log/JSON/TXT/Markdown artifacts found no file containing any of the exact visibility/selected-submit field tokens searched by this pass. Those captures contain no close-zoom visual update or CPU-effect-to-selected-submit join.
- The current hook observes code changes, seed words, factory attempts, live pairs, child effect IDs, and a per-region code discriminator. It returns early when the airborne code is unchanged, has no visibility queue, records no weather update index or exact distance bits, and does not attach weather observations to the selected submit.

## Smallest admissible live boundary


Write occurrence must be observed, not guessed from value equality. While the exact weather call is current, capture calls to the three setter seams visible in the retained producer export:

- `thunk_FUN_142271b40`: alpha;

At producer exit, emit exact post-update active/alpha/playback state for each primary/fading member and the write masks formed from those dynamically observed setter calls. The retained caller export proves these seams, but their standalone prologs/signatures still require normal hook-authority revalidation before detouring them.

The latest fully completed weather update index must then be latched into the same atomic selected-Present transaction. Every active weather child effect ID must join to the complete particle/entity output cover for that selected submit in submit/pass/packet/draw order. An explicit zero-live-output result is admissible only when the authenticated CPU packet list proves it; RenderDoc absence is not enough.

Neither the current camera carrier nor the frozen formulas may fill missing observations. Camera eye data is not the exact `view+0x184` input; static branch laws do not prove post-update state; texture/technique does not prove CPU effect identity; and RenderDoc resource IDs remain capture-local.

## Remaining work and human boundary

The remaining engineering is mechanical but not closed by retained data: revalidate the three setter detours, add initial/checkpoint and per-update observation queues, sequence them into selected-Present ownership, and require complete effect-output cover. Active weather must remain fail closed until this is implemented and proven.

Only then is human input unavoidable: authorize one normal-production early-launch capture, approve the save/profile/modset/target hour plus an active-weather region and camera traversal across all five branches, and preserve the raw hook transcript, capture directory, and recorder output. Late attach, RenderDoc-only evidence, a sidecar, or a fixture cannot satisfy the gate.
