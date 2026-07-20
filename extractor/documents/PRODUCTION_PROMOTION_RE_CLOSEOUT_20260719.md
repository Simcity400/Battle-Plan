# Extractor and replay production-promotion RE closeout

Date: 2026-07-19
Repository: ``
Decision rule: exact-or-fail; no inferred zeros, visual tuning constants, fabricated descriptors, or unverified sidecar substitutions.

## Promotion decision

The offline reverse-engineering and retained-evidence pass is exhausted for the current repository, installed a729 binary/installed content, Ghidra project, save corpus, RenderDoc corpus, and harness captures. The extractor and replay code are substantially hardened and their deterministic test layers pass, but the combined product is **not yet eligible for production promotion**. Promotion remains fail-closed on the live observations below and on the engineering/authority work listed separately.

This document is the consolidated authority for what still needs a human-coordinated game state, capture, environment, or product decision. A row is not a request to guess a value: it is a request to create the stated controlled observation so the value can be derived mechanically.

Reconciled 2026-07-20: the canonical machine register now has 49 external-input
rows, of which 29 are immediately schedulable, plus 31 non-human residual
dispositions. The retained-evidence disposition for every one of the 32 rows
that was formerly immediate is recorded in
`dossiers/RETAINED_EVIDENCE_IMMEDIATE_HUMAN_INPUT_CENSUS_20260720.json`.

The clean exact-ten replacement pass is also exhausted: 43 root calls produced
authenticated artifacts, two isolated calls to `0x140082CB0` produced the
expected terminal decompiler failure with no artifact and no retry, and there
were zero unexpected failures. The final strict consumer audit binds all
110/110 successful claim-consumer rows. Exact-ten receipt completion replaces
discovery custody only; it does not activate any independent live, semantic,
collector, or human gate described below.
The reconciliation SHA-256 is
`8882a5996d8fd1dc6de5a58a45d5f2e2c5c248421cfd0ca6d539cbde5f67c7a6`;
the final consumer-ledger SHA-256 is
`fa2122bad7009953f87f1621900abc3075668d1a452e4cb80b226e6ed59224ec`.

## Validation completed centrally

- Extractor debug workspace, all targets: build and doctest pass; 2,609 nextest tests passed and 13 skipped.
- Extractor release workspace, all targets: build pass; 2,609 nextest tests passed and 13 skipped.
- Replay debug workspace, all targets: build and doctest pass; 1,824 nextest tests passed and 81 correctly skipped by the CI evidence/soak filter.
- Replay release workspace, all targets: build pass; 1,824 nextest tests passed and 81 correctly skipped by the CI evidence/soak filter.
- Replay serialized soak tier: two tests passed in 91.960 seconds.
- Installed-game `game_ui` acceptance binary: pass, exit zero.
- Python/offline suites: coverage audit 54 passed after refreshing the 447-field schema snapshot; game definitions/assets 206 passed and 20 skipped; save oracle 17 passed; production-script tests 14 passed; derive 48 passed and 8 skipped; models 98 passed and 18 external-corpus tests skipped; replay Python 275 passed and 24 skipped; RenderDoc tooling 84 passed; Ghidra tooling 12 passed; capture harness 12 passed; C07 2 passed.
- Production capture validation and acceptance-ladder PowerShell self-tests: pass.
- M02 verifier: 140 checks passed. M03 combined verifier: 337 checks passed. The prior combined promotion pass stopped fail-closed at O04's absent W20 integrity index; that exact 392,964-byte index is now recovered with its provenance receipt and must be consumed by the next root verification pass.
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
| H-POPUP | The pinned-a729 XUI21 matrix covering five popup classes, four resolution paths, repeated/same-hour firings, and capture-start pending state | Authenticate the implemented fire/materialisation/resolution hooks, zero-loss completeness telemetry, d75 lifecycle rows, and exact A25 title/description/option owner joins. |

### Combat, naval, air, supply, and strategic state

| ID | Required controlled input | Exact acceptance purpose |
|---|---|---|
| H-X2-LOSS | One instrumented nonzero manpower/equipment update across two authoritative hours of the same active combat side | Authenticate the exact side scalar and pointer-free equipment-map entries. Retained `SDamageDone` attribution is already exact but is a different carrier. |
| H-X2-WIDTH | Two exact `0x141299610` calls bracketing one D55-authenticated reserve-to-committed transition | Join every committed-wrapper `+0x170` return and final sum to the same combat generation and hour; inspect mutable roots only if direct sum equality fails. |
| H-AIR | Paused active air combat | An exhaustive 33-melt, 1.777-GB census contains no serialized active-air-combat block. |
| H-PEACE | Unresolved active peace conference | The same exhaustive save census contains no nonempty active peace conference. |
| H-CONVOY | One convoy terminal loss plus a no-loss control using the engineered exact `0x141962378` pre-collapse hook and pinned exact-ten authority receipt | Authenticate `CFEXMember+0xD0` client resolution to live source/destination state while owner/killer/count still exist; terminal saves irreversibly discard that join. Exact-ten static custody is closed; this branched gameplay observation is the sole remaining convoy input. |
| H-ACE | Same-tick raw-memory and save fixture for one ace lifecycle transition | Cross-authenticate alive, kill type, handled state, and save representation. |
| H-NUKE | Controlled nuke count and owner lifecycle | Validate the exact live reader and ownership transition. |
| H-DYNMOD | One timed dynamic-modifier lifecycle | Validate raw expiry/duration fields and remaining daily ticks without inventing absolute expiry. |
| H-SNOWMUD | Temporal snow/mud capture across changing ground conditions | XN16 proves one-frame capture only; temporal recording remains unproven. |
| H-RAID | Paused raid matrix containing preparing, in-progress, and ended children | Validate ordered parent/child equality and lifecycle transitions. |

`H-X2-END` is retired: the two exact terminal writers (`0x140CCC7F0`
normal and `0x140CCC100` alternate/overrun) are now joined through their sole
`0x140CC9F10` record allocator to the owning D21 combat pointer before deletion.
D55 v6 carries the authenticated pointer-free terminal projection keyed by
`(combat_id,generation)` and fails closed on every hook, byte, queue, or teardown
mismatch. The exact-10 replacement receipt and pool/function/byte/layout hashes
are pinned in the X2 closeout and hook module, so terminal writer activation is
enabled; the separate width and current-loss activation latches remain false.

`CSupplySystem+0x108` per-country owner vector, its exact alignment with
`CGameState+0x310`, and the `CCountrySupplySystem+0x70/+0x78` backlinks. Domain
44 production kind 5 now emits the exact stored `disrupted_supply` table and
fails closed on every roster, identity, backlink, table, and value mismatch.

`H-A10` is retired: OPIN v3 now carries exact operation definition, owner,
country/state target, preparing/prepared/running/finished status, phase,
current-phase progress, start/duration, and assigned operative identities. The
remaining reciprocal-assignment and UI-controller joins belong to A30, not to a
new operation-semantics fixture.

`H-A9-MEMBER-LEAVE` and `H-A9-TERMINAL-LOSS` are retired by
`extractor/native/lanes/naval_combat_live/A8_A9_B6_HUMAN_INPUT_REQUIREMENTS.json`.
D50 tombstones mechanically cover each vanished member independently of D49
parent survival; `CFEXMember+0xE0 == 0x20` is exactly `FEX_STATE_SUNK` and is
consumed as the terminal sunk row. Controlled leave/loss states remain useful
regression validation only, and generic tombstones must remain untyped.

`H-A8-ACTIVITY` is not a human fixture. All named naval replacement roots now
have exact-ten custody, but the resulting evidence still does not prove one
common D56 receiver/base for the conflicting `+0x4A8/+0x4B8/+0x4C0`
observations. Live `mission_state`, `repair_progress_milli`, and task-force
`retreat_state` therefore remain mechanically rejected and absent. The save
retreat Boolean and D50 member retreat states are separate carriers, not
aliases; another human transition cannot repair the missing semantic join.

### Intelligence, operations, decisions, and temporal capture

| ID | Required controlled input | Exact acceptance purpose |
|---|---|---|
| H-A11 | Controlled per-state network-strength transition plus the same-frame owner, reciprocal-assignment, and controller membership/order/visibility projection | Four-lane owner-target intel levels and network-scoped national coverage are already exact; validate only the remaining per-state/network-controller carrier after its wiring exists. |
| H-A16 | Frozen subject-autonomy discriminator states | Close the remaining subject-autonomy direction and ratio semantics. |
| H-A30 | Exact agency/operative/operation/cipher/intel/network reciprocal-assignment and controller matrix after producer wiring | Validate the remaining XUI14 presentation-owner, membership, ordering, and visibility leaves without re-requesting the now-exact A10 operation runtime state. |
| H-A3-P3-X12 | P3 campaign-identity matrix plus X12 within-hour transition actions after hook engineering | A3 raw-speed mapping and pause orthogonality are already exact for pinned a729; this alias now covers only P3 identity arms and X12 ordered, gap-free sub-hour transitions. |
| H-XN15L | Early-launch production capture followed by replay composition/pixel comparison | Close the launch-time renderer state that cannot be rescued by late attach. |
| H-G2-PIXEL | One authenticated close-zoom engine MainScene/map-target readback and portable-replay render at the same render boundary, hour, physical viewport, graphics settings, language, DLC/mod VFS, camera, and POV | Retain both 32-bit BGRA planes and admit a reviewed golden only when the receipt-bound gate reports `mismatch_pixels=0` and `max_channel_delta=0`; typed-state hashes or receipts without pixels are insufficient. |
| H-B4-B5-B7 | One paused D04 session: 441 B4 rows with convoy sum 2,810; all 958 B5 rows with typed offsets in `+0x70..+0x120`; 440 B7 parents and 167 ordered children including the dummy and nine non-ended raids | Establish live-memory/save equality for the already-mined offline oracles. |

`H-C07-T1` and `H-C07-S1` are retired by
`replay_interface/re_evidence/rendering_laws/C07/C07_HUMAN_INPUT_REQUIREMENTS.json`.
The retained 4,781-cell T1 scope and the 129-frame L02/L03/W23 S1 execution
chain are authorized and wired. A one-cell T1 discriminator and labelled
single-caster S1 pair are optional automated/golden validation. Complete-pool
tree replacement remains fail-closed for the other 95,275 cells pending an
authenticated visibility/model/LOD carrier; that is non-human producer work,
not permission to reissue `H-G1-C07`.

`H-A23-REWARD-HISTORY` is retired by
`extractor/native/project_progress_live_lane/SPECIAL_PROJECT_LIFECYCLE_REWARD_RE_20260719.json`.
The exact lifecycle bytes, per-project reward-history vector/row/context layout,
serialized fields, and writer ordering are closed. The exact-10 card, including
`SP-01`, is authenticated and pinned; production `PRHF` emission/fold/replay is
active under promotion-manifest SHA-256
`33d72cfe1cfb7e11123d83dcb5cba80c1f9a1f6b5dd0027809864a1a42a4a4a2`.
`H-A23-PROGRAM-LIFECYCLE` is narrowed to the genuine 26/26 d65 owner-equality
gate. `H-X9-SPECIALIZATIONS` remains active and still requires its four distinct
nonzero same-tick handle/value join.

### Map, counters, post-effects, and environment

| ID | Required controlled input | Exact acceptance purpose |
|---|---|---|
| H-CAP6-H1 | Camera-height plus DPI/window-mode sweep | Derive continuous counter anchor and scale law. |
| H-CAP6-H2 | Stack sizes around grouping thresholds | Derive slot order and aggregation thresholds. |
| H-CAP6-H3 | Allegiance, country-color mode, and selection matrix | Derive exact tint and selection-state rules. |
| H-CAP6-H4 | Simultaneous counter badge states | Derive badge precedence and layer ordering. |
| H-CAP6-H5 | Enemy counters across FoW/intel levels | Derive hidden/partial presentation behavior. |
| H-RG-H1 | Unclamped HDR plus consecutive-frame luminance observations | Close temporal grading/exposure behavior. |
| H-RG-H2 | One-variable wide-bloom-width discriminator | Derive bloom width without visual tuning. |
| H-RG-H3 | Height-fade and region-edge crossings | Derive exact spatial fade boundaries. |
| H-RG-H4 | Visibly nontransparent political secondary-map producer case | Prove the secondary-map blend/producer path. |
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

1. CAP6's exact-ten static replacement roots are closed, but they do not supply an event-scoped GUI-paint-to-selected-Present receipt. The captured-map D3D11 upload/draw seam remains fail-closed as `UNAVAILABLE_ZERO_DRAW`; activation stays false until the authenticated live owner/path-to-draw carrier and CAP6-H1 through H5 observations exist.
2. The G3 lexical classifier is diagnostic only and is not valid promotion proof. Occupation stripes now use the exact canonical law, and authenticated D80 operations take precedence over the legacy renderer. Shadow, buildings, rails, and trees still require authenticated producer/consumer or typed semantic-to-D80 joins; they must not be marked wired by token references. Camera requires the exact C02 current/target binary32 updater; its separate wheel height-to-speed target law remains untraced.
3. The offline product runtime now has durable append tickets, rollover propose/drain/seal/open/ack/header barriers, dormant paired-runner heartbeat, and graceful H4CT close before H4CR drain. Activation remains disabled until concrete live adapters, CID1 proof, and the active legacy spawner's hard-kill/finite-stdout boundaries are replaced or authenticated.
4. The XUI21 fire/materialisation/resolution graph, multi-slot context contract, and authenticated one-shot weather-worker fault tooling are implemented. XUI21 now needs its named exhaustive runtime coverage matrix, not additional producer discovery; weather fault acceptance remains a separate named live observation.
5. Domain 81's exact-ten owner, enumeration, authentication, and frame-law authority is closed. The v2 encoder, recorder fold, authority accounting, and replay-ingest latches are enabled under promotion-manifest SHA-256 `32388518a63eaa22ea6290e82fb3677a5515039e937334ebc2c2d26a0c0bc6b1`; the active collector still emits payload v1 because the authenticated fail-closed double-read hub collector remains unimplemented. No human capture can close that code-owned residual.
6. GH04's exact-ten topology body is pinned and existing national coverage remains implemented. Per-state strength stays unavailable until that body is semantically integrated; `H-A11` remains downstream of that wiring.
7. The weather named-token, definition, and entity-selector bridge is exact-ten closed. Active weather remains false pending authenticated initial/per-update visibility, update ownership, and selected-Present effect-output coverage or the filed controlled-camera equivalent.
8. Refresh the full recursive XUI27 authority chain only after all source changes settle. Run two clean identical replay-owned passes, update the independent golden only from equal results, run terminal verification, then repeat the G3 recount.
9. The strict package-runtime boundary now provides deterministic manifests, signing admission, atomic active/previous pointers, rollback, locks, Windows-safe paths, and reparse-point refusal. Product owners must still select the production signature algorithm/key custody and rotation/revocation policy, installer/update entry point, trust-store distribution, and tray/window surface.
10. Pure Cargo/cache scratch totals about 17 GB and can be regenerated. The two mixed audit trees contain superseded intermediate source even though they contain no unique production evidence; archive that source history before deleting those mixed trees.

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
- Exact D45 focus-pause state and both decision target-body/cooldown families.
- Naval B6 exact side positioning/anti-air producer, signed fixed-point arithmetic, compact carrier, fold, and replay projection; its eight-row capture card is retired as RE authority.
- Static-artifact S1 mandatory seven-artifact admission and fail-closed validation.
- W20's exact 392,964-byte RDC integrity index and provenance receipt.
- The original 243-file `restore_gate_1` capture, recovered byte-for-byte; RG-H1 through H4 remain separate discriminator captures.
- D81 exact province-anchor and reciprocal-rail validated slice, exact-ten hub owner/enumeration authority, and enabled hub-v2 carrier/replay latches; the live double-read collector and active payload-v2 switch remain fail-closed non-human work.
- A10 exact OPIN v3 operation runtime semantics; its former prepared/running fixture is retired, while A30 controller and reciprocal-assignment leaves remain separate.
- Replay rendering-law B10 territory key (province controller) and authenticated mapname CB1 discriminator; remaining controller projection/token emission is integration, not human RE. This is distinct from extractor B10/A23 `CProgram` owner validation.
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
