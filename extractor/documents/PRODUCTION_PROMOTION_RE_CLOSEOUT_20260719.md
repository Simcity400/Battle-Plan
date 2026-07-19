# Extractor and replay production-promotion RE closeout

Date: 2026-07-19
Repository: ``
Decision rule: exact-or-fail; no inferred zeros, visual tuning constants, fabricated descriptors, or unverified sidecar substitutions.

## Promotion decision

The offline reverse-engineering and retained-evidence pass is exhausted for the current repository, installed a729 binary/installed content, Ghidra project, save corpus, RenderDoc corpus, and harness captures. The extractor and replay code are substantially hardened and their deterministic test layers pass, but the combined product is **not yet eligible for production promotion**. Promotion remains fail-closed on the live observations below and on the engineering/authority work listed separately.

This document is the consolidated authority for what still needs a human-coordinated game state, capture, environment, or product decision. A row is not a request to guess a value: it is a request to create the stated controlled observation so the value can be derived mechanically.

## Validation completed centrally

- Extractor debug workspace, all targets: build and doctest pass; 2,609 nextest tests passed and 13 skipped.
- Extractor release workspace, all targets: build pass; 2,609 nextest tests passed and 13 skipped.
- Replay debug workspace, all targets: build and doctest pass; 1,824 nextest tests passed and 81 correctly skipped by the CI evidence/soak filter.
- Replay release workspace, all targets: build pass; 1,824 nextest tests passed and 81 correctly skipped by the CI evidence/soak filter.
- Replay serialized soak tier: two tests passed in 91.960 seconds.
- Installed-game `game_ui` acceptance binary: pass, exit zero.
- Python/offline suites: coverage audit 54 passed after refreshing the 447-field schema snapshot; game definitions/assets 206 passed and 20 skipped; save oracle 17 passed; production-script tests 14 passed; derive 48 passed and 8 skipped; models 98 passed and 18 external-corpus tests skipped; replay Python 275 passed and 24 skipped; RenderDoc tooling 84 passed; Ghidra tooling 12 passed; capture harness 12 passed; C07 2 passed.
- Production capture validation and acceptance-ladder PowerShell self-tests: pass.
- M02 verifier: 140 checks passed. M03 combined verifier: 337 checks passed. The combined promotion verifier then stops fail-closed at O04's absent W20 integrity index.
- CAP6 retained verifier: 98 checks passed with the current B8G8R8A8 target correctly rejected against retained R8G8B8A8 evidence, producing zero counter draws.
- O04 reviewed evidence restoration: 410 authenticated files, 79,019,327 bytes; inventory SHA-256 `dea4b2b45270c0095a6d9a3228215cab800024ef0b37c98a22f5d4b8b00bb667`.
- Final G3 recount is deterministic across two consecutive reads: 95 register rows (`76 PROVEN`, `17 DATA_BLOCKED`, `1 HUMAN_CAPTURE_REQUIRED`, `1 QUEUED`); the active base register is 49 proven plus one human-capture row. The recursive freeze is stale with 265 findings: 94/294 frozen source hashes current, 199 stale, one missing; zero of seven tool pins and zero of seven authority-container pins current; seven source additions and one removal.

## Human-coordinated inputs still required

### Capture boundary, orders, economy, and runtime text

| ID | Required controlled input | Exact acceptance purpose |
|---|---|---|
| H-XUI27 | One same-present engine UI observation paired with an authenticated replay `WorldFrame`, content manifest, and settings | No retained session pairs the engine presentation and replay presentation at the same state; this is the terminal replay UI parity observation. |
| H-X1-X8 | Two paused army-order fixtures: empty/inactive and active lifecycle | Prove producer identity, empty-state handling, creation/update/removal ordering, and the exact order schema without substituting save-only state. |
| H-B1 | Paused market fixtures with nonempty and empty market state | Prove market root and exact absence semantics. |
| H-X4 | Paused production fixture exposing the production root and domain 614 | Close the remaining production-root/offset join. |
| H-A25 | Exact `CInstantText` glyph and inline-icon capture | Derive runtime glyph placement and icon participation; installed scripts and retained captures do not contain the object join. |
| H-A31 | Controlled national-focus GUI state matrix | Join focus model state to rendered focus presentation. |
| H-A37 | By Blood Alone division-designer state | Resolve the designer-only producer/presentation branch. |
| H-A38 | Controlled scripted-GUI state | Join evaluated scripted GUI values to their visible presentation owner. |
| H-A28-A36-A39-A40 | The pinned JSON/input-SHA capture matrices for active abilities, ace/character presentation, and controller/read-only states | Each dossier defines the exact state transitions and hashes; no existing corpus contains the required discriminators. |
| H-POPUP | One pinned-a729 materialized event window with title, description, multiple options, scoped/formatted text, owner, and context | Authenticate the event-window/textbox-owner classifier now that independent title, description, and per-option contexts are wired end to end. |

### Combat, naval, air, supply, and strategic state

| ID | Required controlled input | Exact acceptance purpose |
|---|---|---|
| H-X2-LOSS | Active land combat with nonzero casualty and equipment loss | Locate and authenticate the live loss carrier; the retained stream cannot distinguish unavailable from a genuine zero. |
| H-X2-WIDTH | Synchronized land-combat-width producer observation | Join the width source to the same combat generation. |
| H-X2-END | Terminal land combat covering winner/retreat plus overrun or destroyed outcome | Prove end-state producer and teardown semantics. |
| H-NAVAL-1..5 | Five naval matrices specified by `A8_A9_B6_HUMAN_INPUT_REQUIREMENTS.json` | Close mission, combat-side, anti-air, terminal, and membership discriminators that retained lifecycle evidence cannot supply. |
| H-AIR | Paused active air combat | An exhaustive 33-melt, 1.777-GB census contains no serialized active-air-combat block. |
| H-PEACE | Unresolved active peace conference | The same exhaustive save census contains no nonempty active peace conference. |
| H-SUPPLY | One bounded production root for each `CCountrySupplySystem` | Replace preflight-only RTTI heap discovery with a production-safe ownership route. |
| H-CONVOY | One convoy battle transitioning to terminal loss | Join owner/killer/count to route/client attribution, which terminal saves discard. |
| H-ACE | Same-tick raw-memory and save fixture for one ace lifecycle transition | Cross-authenticate alive, kill type, handled state, and save representation. |
| H-NUKE | Controlled nuke count and owner lifecycle | Validate the exact live reader and ownership transition. |
| H-DYNMOD | One timed dynamic-modifier lifecycle | Validate raw expiry/duration fields and remaining daily ticks without inventing absolute expiry. |
| H-SNOWMUD | Temporal snow/mud capture across changing ground conditions | XN16 proves one-frame capture only; temporal recording remains unproven. |
| H-RAID | Paused raid matrix containing preparing, in-progress, and ended children | Validate ordered parent/child equality and lifecycle transitions. |

### Intelligence, operations, decisions, and temporal capture

| ID | Required controlled input | Exact acceptance purpose |
|---|---|---|
| H-A10-A11-A16-A30 | Prepared-operation, intel-network, live-intel, GH01, and GH05 controlled states | Close the remaining producer/consumer joins in the diplomacy/intelligence lane. |
| H-A3-P3-X12 | D57 lifecycle, sub-hour hook, and speed-case fixtures | Prove exact temporal ordering and speed behavior across the live capture boundary. |
| H-G1-C07 | Four isolated T1/S1 control/discriminator launches, including two full-next-frame RenderDoc captures | Prove tree-buffer to IA-slot/draw joins and shadow-caster behavior with exactly one changed variable per pair. |
| H-XN15L | Early-launch production capture followed by replay composition/pixel comparison | Close the launch-time renderer state that cannot be rescued by late attach. |
| H-B4-B5-B7 | One paused D04 session: 441 B4 rows with convoy sum 2,810; all 958 B5 rows with typed offsets in `+0x70..+0x120`; 440 B7 parents and 167 ordered children including the dummy and nine non-ended raids | Establish live-memory/save equality for the already-mined offline oracles. |

### Map, counters, post-effects, and environment

| ID | Required controlled input | Exact acceptance purpose |
|---|---|---|
| H-CAP6-1 | Camera-height plus DPI/window-mode sweep | Derive continuous counter anchor and scale law. |
| H-CAP6-2 | Stack sizes around grouping thresholds | Derive slot order and aggregation thresholds. |
| H-CAP6-3 | Allegiance, country-color mode, and selection matrix | Derive exact tint and selection-state rules. |
| H-CAP6-4 | Simultaneous counter badge states | Derive badge precedence and layer ordering. |
| H-CAP6-5 | Enemy counters across FoW/intel levels | Derive hidden/partial presentation behavior. |
| H-RG-1 | Unclamped HDR plus consecutive-frame luminance observations | Close temporal grading/exposure behavior. |
| H-RG-2 | One-variable wide-bloom-width discriminator | Derive bloom width without visual tuning. |
| H-RG-3 | Height-fade and region-edge crossings | Derive exact spatial fade boundaries. |
| H-RG-4 | Visibly nontransparent political secondary-map producer case | Prove the secondary-map blend/producer path. |
| H-ONEDRIVE | A genuine dehydrated OneDrive placeholder replay store | Exercise metadata-only traversal without hydration; 47,621 files were scanned locally and zero placeholders exist. |
| H-MODSET | Selected installed-mod matrix and one current seven-role static bundle per chosen mod set | Authenticate effective-VFS bundle generation before publication; retained bundles have deferred token joins and the old D04 bundle is schema v2. |

### Live acceptance and fault behavior

| ID | Required controlled input | Exact acceptance purpose |
|---|---|---|
| H-SINK-OK | One production capture with nonzero host-write samples and terminal sole-pipe reconciliation | Validate real sink latency, monotonic wall time, manifest acceptance, and graceful finalization. |
| H-SINK-FAIL | One injected sink-open or mid-stream write failure | Validate authenticated `collector_sink_failure.json` publication end to end. |
| H-F6-PAUSE | One attempt-bound paused capture/fold/extraction proof | Exercise the paused proof once its producer wiring exists. |
| H-SOAK | Six-hour product soak on the target/weak-PC profile | Validate rollover, heartbeat, memory, storage growth, and replay continuity under deployment conditions. |

## Remaining engineering and repository-authority work

These are **not human reverse-engineering requests** and must not be misreported as such:

1. CAP6's captured-map D3D11 upload/draw seam is wired and fail-closed. It still needs an authenticated current-target descriptor and an exact per-frame carrier for content, modulation, complete draw order, and CAP6-H1 through H5 before it may draw.
2. The G3 lexical classifier is diagnostic only and is not valid promotion proof. Occupation stripes now use the exact canonical law, and authenticated D80 operations take precedence over the legacy renderer. Shadow, buildings, rails, and trees still require authenticated producer/consumer or typed semantic-to-D80 joins; they must not be marked wired by token references. Camera requires the exact C02 current/target binary32 updater; its separate wheel height-to-speed target law remains untraced.
3. The offline product runtime now has durable append tickets, rollover propose/drain/seal/open/ack/header barriers, dormant paired-runner heartbeat, and graceful H4CT close before H4CR drain. Activation remains disabled until concrete live adapters, CID1 proof, and the active legacy spawner's hard-kill/finite-stdout boundaries are replaced or authenticated.
4. The popup multi-slot context contract and authenticated one-shot weather-worker fault tooling are implemented. Only their named live observations remain; no further placeholder producer engineering is required before acquisition.
5. O04's two reviewed static-object trees are restored. Recover the exact W20 RDC integrity index (392,964 bytes; SHA-256 `9715a81dababc7c6d888a957ec102e73f2d5cc01b3cd93d2f995facd56d23e4d`) or reacquire and independently authenticate the same corpus. The verifier correctly stops there.
6. Refresh the full recursive XUI27 authority chain only after all source changes settle. Run two clean identical replay-owned passes, update the independent golden only from equal results, run terminal verification, then repeat the G3 recount.
7. The strict package-runtime boundary now provides deterministic manifests, signing admission, atomic active/previous pointers, rollback, locks, Windows-safe paths, and reparse-point refusal. Product owners must still select the production signature algorithm/key custody and rotation/revocation policy, installer/update entry point, trust-store distribution, and tray/window surface.
8. Pure Cargo/cache scratch totals about 17 GB and can be regenerated. The two mixed audit trees contain superseded intermediate source even though they contain no unique production evidence; archive that source history before deleting those mixed trees.

## Closed without further human input

- M02 and M03/A48 production contracts.
- A24, A29, A33/A34 core mechanics, d75 fired-event lifecycle, and the pre-folded decoder seam.
- Portable replay strict JSON, archive/VFS, path portability, integrity, and rollback-index hardening.
- Sink telemetry and typed failure contracts at the source/offline-test layer.
- Dynamic popup title/description/per-option context transport and fail-closed replay admission.
- Default-off authenticated F6 synthetic fault injection and secure proof tooling.
- Offline rollover, durable-ticket, heartbeat, and graceful terminal-control foundations.
- Strict replay package manifest, admission, promotion, and rollback runtime foundations.
- Exact occupation-stripe CPU law and authenticated D80-before-legacy renderer precedence.
- Save-oracle fixture coverage and the retained D04 special-project census.
- Naval mission enum and retained combat-lifecycle masking.
- A17 exact stockpile reader, A18 raw timed-modifier semantics, and A20 temporal validator implementation.

## Promotion gate

Production promotion requires all of the following:

1. Central debug and release batteries remain green after the final authority refresh.
2. O04, XUI27, and G3 recursive verification report current - not stale or substituted - evidence.
3. Every applicable human-input row above produces an authenticated packet and passes its packet-local verifier.
4. Product integration, rollover, sink-failure, packaging, and soak gates pass.
5. The final recount contains no unowned, fabricated, stale, or silently unavailable production field.
