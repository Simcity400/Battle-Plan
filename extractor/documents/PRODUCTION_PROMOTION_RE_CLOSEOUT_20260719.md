# Extractor and replay production-promotion RE closeout

Date: 2026-07-19
Repository: ``
Decision rule: exact-or-fail; no inferred zeros, visual tuning constants, fabricated descriptors, or unverified sidecar substitutions.

## Promotion decision

The offline reverse-engineering and retained-evidence pass is exhausted for the current repository, installed a729 binary/installed content, Ghidra project, save corpus, RenderDoc corpus, and harness captures. The extractor and replay code are substantially hardened and their deterministic test layers pass, but the combined product is **not yet eligible for production promotion**. Promotion remains fail-closed on the live observations below and on the engineering/authority work listed separately.

This document records external controlled observations and product decisions.
It must not be read as saying that every row needs a person: after Wave 6, many
rows are autonomous early-launch, build/injection, or replay-pair runs. A row is
never a request to guess a value; its verifier derives the value mechanically.

Reconciled through the 2026-07-22 post-landing refreeze: the canonical machine
register now has 55 implementation-ready autonomous acquisition rows, zero
runtime-admitted receipts, zero user-action or unresolved-engineering rows, and
12 separate product decisions. H-AIR is retired outside the canonical set. CID1
M3's second physical machine is the sole separately listed genuine external
dependency. The historical execution-bundle grouping remains in
`dossiers/WAVE4_PROVISIONAL_HUMAN_INPUT_RECONCILIATION_20260720.json`; its
filename is retained for link stability. Root-owned Wave4 validation is now
complete: centralized Final/All and Release/All both passed with exit zero,
the production capture-script battery passed 79/79, and the combined Python
battery passed 471 with 44 skipped. Independent live, semantic, collector,
human, and product-decision gates remain fail-closed.
Wave 6 static RE and production wiring subsequently passed centralized
Final/All. Current human-versus-autonomous disposition is the canonical machine
register, not the historical section headings retained below.
The retained-evidence disposition for every one of the 32 rows
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
`f6cc6c85727ac45ec91250208affd2b9517b711d93382b88035bfdad3dfef65c`;
the final consumer-ledger SHA-256 is
`5407efc3147b48934c5df088cab34a4cfc57ca75b8d2c26192df3fc7de0cbf57`.

## Current Wave4 validation completed centrally

- Centralized Final/All: passed, exit zero.
- Centralized Release/All: passed, exit zero.
- Release artifact receipt:
  `.cargo-target/release/hoi4-production-prebuilt-artifacts.json`, SHA-256
  `f5f78fd251e6d3720ff69d5fd3e7904c452ae14fd792f5bbb0837e01ca388d0a`.
- Combined Python battery: 471 passed and 44 skipped.
- Production capture-script battery: 79/79 passed.

These results certify the current Wave4 source set. They do not activate any
independent live/capture, semantic, collector, soak, human-input, or product
choice gate recorded below.

## Historical baseline validation completed centrally

The results below predate the Wave4 edit set and are retained only as historical
baseline evidence. They are superseded for current-source validation by the
completed Wave 6 centralized Final/All battery recorded above.

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

## External controlled inputs still required

Unless a row explicitly requests a product choice or unavailable gameplay
steering, the root runner should attempt it autonomously with the existing
early-launch/capture harness before asking the user. Mechanical review or a
fail-closed verifier is not human semantic input.

### Capture boundary, orders, economy, and runtime text

| ID | Required controlled input | Exact acceptance purpose |
|---|---|---|
| H-XUI27 | One normal a729 same-Present session covering the frozen five-state UI matrix across 1920x1080 and 2560x1440, each paired with an authenticated replay `WorldFrame`, content/settings/input/VFS identity, and lossless engine plane | No retained session pairs the engine presentation and replay presentation at those same states; this is the terminal replay UI parity observation after the replay-only authority refreeze. |
| H-A25 | One visible base-game leader-ability tooltip containing ordinary text plus `command_power` or `clock_texticon` during the owner-scoped, same-Present GUI capture | Authenticate the exact mixed glyph/icon decisions, assets, destination/scissor, selected target, complete GUI frontier, recorded-input authority, and common owner/paint identity; retained CPU/GPU evidence does not contain that live object join. |
| H-A31 | Controlled national-focus GUI state matrix | Join focus model state to rendered focus presentation. |
| H-A29 | One active-research technology year/ahead-of-time tooltip during a pinned-a729 capture | Static RE, carrier, fold, replay, UI and centralized validation are complete. The machine-owned wrapper must drive an authenticated research-view selection/hover and fail closed if it cannot materialize branch 1/2; it must not delegate semantic selection to the user. Base-factor or template-year reconstruction remains forbidden. |
| H-A37 | Controlled By Blood Alone power-balance stable and cross-direction transition matrix | The panel/controller observer, PBP2 path, replay UI and owner wrapper are complete. This is an automatable live wrapper/fixture plus mechanical regrade; it needs no human semantic judgment. |
| H-A28-A36-A39-A40 | The pinned JSON/input-SHA capture matrices for active abilities, ace/character presentation, selected designers, and active peace state | A28 itself remains autonomous controller/root work. A36, A39, and A40 retain only the exact external states in the final execution bundle. |
| H-POPUP | The pinned-a729 XUI21 matrix covering five popup classes, four resolution paths, repeated/same-hour firings, and capture-start pending state | Authenticate the implemented fire/materialisation/resolution hooks, zero-loss completeness telemetry, d75 lifecycle rows, and exact A25 title/description/option owner joins. |
| H-A26 | One owner-run authenticated tooltip same-Present matrix covering hidden baseline, capture-start-visible, show/hide/destruction/owner changes, multiline dynamic final blob, and edge geometry at 1920x1080 and 2560x1440 | Prove exact owner, final post-layout blob, rectangle, selected-Present identity, d72 fold, typed replay, and real production-renderer HUD equality. Startup silence, newline-derived source rows, inferred availability, caller-stamped frame identity, and screenshots are rejected. |
| H-A27 | One exact-a729 session spanning at least three consecutive advancing replay hours with one real design rename/edit or obsolete/reactivate mutation | Prove the repeated whole-sweep reader seals every hour, emits exact upserts and absence-derived tombstones, reconstructs portable v3 deterministically, and admits d78 only at the matching hour. A coverage failure records the exact in-process-hook boundary; it never authorizes borrowed final state. |

### Combat, naval, air, supply, and strategic state

| ID | Required controlled input | Exact acceptance purpose |
|---|---|---|
| H-X2-LOSS | One instrumented nonzero manpower/equipment update across two authoritative hours of the same active combat side | Authenticate the exact side scalar and pointer-free equipment-map entries. Retained `SDamageDone` attribution is already exact but is a different carrier. |
| H-X2-WIDTH | Two exact `0x141299610` calls bracketing one D55-authenticated reserve-to-committed transition | Static RE, fresh exact-20 authority, and carrier/fold/replay/XUI20 wiring are complete. Retained evidence has 116 such transitions but zero active width-hook calls. Join every committed-wrapper `+0x170` return and final sum to the same combat generation and hour; inspect mutable roots only if direct sum equality fails. |
| H-X2-PANEL | One active land battle kept selected through a both-side combat-detail refresh | Exercise the already-active exact game materializer carrier. Require positive capture sequence, exact leader or exact-none status, canonical unique engine modifier order, simultaneous D55 combat generation/side binding, pointer-free fold, and independent replay regrade; screenshots and inferred side flags are rejected. |
| H-A33-SUNK | One disposable exact-a729 empty-history run in which the operator causes one naval sink | The automated probe must retain a complete baseline, same-thread D49 member epoch, SHP1 history, zero-loss receipt, fold/replay parity, and independent regrade. |
| H-B7 | One bounded read-only inspection of the paused D04 raid-system heap at `1944.2.11.18` | Authenticate exact parent/child identity, engine ordering, state and dummy classification against the retained oracle. Only an independently regraded `ACCEPTED_EXACT_440_PARENTS_167_CHILDREN` receipt may authorize a later d91 scheduling patch. |
| H-AIR | **RETIRED / CLOSED SCHEMA** | Exact A4/A21 writer, serializer, and consumer authority proves the durable region/date/wing aggregate and typed damage causes, with no persistent tactical combat id or opposed-wing pairing. The 33-melt/1.777-GB negative census is supporting evidence, not a reason to demand a nonexistent serialized fixture. |
| H-PEACE | One registered active conference lifecycle from pre-claim refresh through nonempty contest to terminal summary | Retain the loss-free raw trace and independent regrade. RE is closed negatively: allocation ids are refresh-scoped and terminal rows remain explicitly unjoined because exact-a729 exposes no allocation-result lifecycle identity. Acceptance authorizes later activation of that schema only; pointers, payload hashes, equality, and source order never manufacture a join. |
| H-CONVOY | One convoy terminal loss plus a no-loss control using the engineered exact `0x141962378` pre-collapse hook and pinned exact-ten authority receipt | Authenticate `CFEXMember+0xD0` client resolution to live source/destination state while owner/killer/count still exist; terminal saves irreversibly discard that join. Exact-ten static custody is closed; this branched gameplay observation is the sole remaining convoy input. |
| H-ACE | Same-tick raw-memory and save fixture for one ace lifecycle transition | Cross-authenticate alive, kill type, handled state, and save representation. |
| H-DYNMOD | One timed dynamic-modifier lifecycle | Validate raw expiry/duration fields and remaining daily ticks without inventing absolute expiry. |
| H-A19 | One uninterrupted delayed-event lifecycle covering retained decrement, execution, failed-trigger consumption, and definition cancellation, with one originator distinct from recipient | Authenticate the DEL2 paired hooks, shared dispatch join, ordered pointer-free reasons, corrected scope/originator fields, fold/replay equality, and fail-closed gap/unmatched-disappearance behavior. No human semantic annotation remains. |
| H-SNOWMUD | Temporal snow/mud capture across changing ground conditions | Wave 6 closes the exact CPU producer/upload boundary. The existing early-launch runner must record two real content-changing H rows and matching replay H2 readbacks; this is autonomous elapsed-game/capture evidence, not additional RE or human annotation. |

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
| H-A3-P3-X12 | P3 campaign-identity matrix plus one authorized X12 autonomous live run | A3 raw-speed mapping and pause orthogonality are already exact for pinned a729. The X12 hook/driver owns its baseline and ordered sub-hour mutations; this alias now covers only P3 identity arms and permission for the machine-run X12 receipt. |
| H-XN15L | Early-launch production capture followed by replay composition/pixel comparison | Close the launch-time renderer state that cannot be rescued by late attach. |
| H-G2-PIXEL | One authenticated close-zoom engine MainScene/map-target readback and portable-replay render at the same render boundary, hour, physical viewport, graphics settings, language, DLC/mod VFS, camera, and POV | Retain both 32-bit BGRA planes and admit a reviewed golden only when the receipt-bound gate reports `mismatch_pixels=0` and `max_channel_delta=0`; typed-state hashes or receipts without pixels are insufficient. |
| H-B4-B5-B7 | Three bounded paused D04 observations: B4 441-row/sum-2810 equality, B5 958-war-row save/memory equality, and B7 440-parent/167-child raid equality | Static laws and dormant carriers are closed. Each independent runner/regrader must bind the exact paused a729 process and its named retained oracle; accepted evidence does not silently enable scheduling, and every lane requires its separate reviewed activation patch. |

`H-C07-T1` and `H-C07-S1` are retired by
`replay_interface/re_evidence/rendering_laws/C07/C07_HUMAN_INPUT_REQUIREMENTS.json`.
The retained selected-submit T1 carrier and the 129-frame L02/L03/W23 S1
execution chain are authorized and wired. Exact producer RE plus hash-pinned
shipped inputs now supply all 296 immutable buckets / 99,056 cells, while the
generic exact-a729 consumers close visibility, palette/model, squared-distance
LOD, and ordered draws for every bucket. Production still admits per-frame
state only from one authenticated selected submit; files-only selection remains
fail-closed. A one-cell T1 discriminator, clean-launch buffer observation, and
labelled single-caster S1 pair are optional automated/golden validation, not
human RE and not permission to reissue `H-G1-C07`.

`H-A23-REWARD-HISTORY` is retired by
`extractor/native/project_progress_live_lane/SPECIAL_PROJECT_LIFECYCLE_REWARD_RE_20260719.json`.
The exact lifecycle bytes, per-project reward-history vector/row/context layout,
serialized fields, and writer ordering are closed. The exact-10 card, including
`SP-01`, is authenticated and pinned; production `PRHF` emission/fold/replay is
active under promotion-manifest SHA-256
`33d72cfe1cfb7e11123d83dcb5cba80c1f9a1f6b5dd0027809864a1a42a4a4a2`.
`H-A23-PROGRAM-LIFECYCLE` is narrowed to the genuine 26/26 d65 owner-equality
gate. `H-X9-SPECIALIZATIONS` is retired by fresh exact-20 writer/reader evidence:
the breakthrough tree key is the engine serializer token id, and production now
resolves it through a complete ambiguity-checked authenticated token-record census.
The 16-byte d41 carrier retains the raw key only inside compact delta tracking; fold strips
it before portable replay, and typed replay/UI require the closed enum plus a matching
current-hour D18 country witness. Unresolved handles fold null and remain unavailable.
See `X9_SPECIALIZATION_TOKEN_NAMESPACE_PROMOTION_20260720.json`.

### Map, counters, post-effects, and environment

| ID | Required controlled input | Exact acceptance purpose |
|---|---|---|
| H-CAP6-PRESENT | One central build/injection run of the Wave 6 final-D3D collector with counters visible | Accept only the pointer-free selected-Present receipt with `selected_present_sealable=true`, exact owner/root/wrapper/paint-root ancestry, complete draw frontier and immutable artifact hashes. This is autonomous machine acquisition; no operator classification is accepted. |
| H-CAP6-H1 | Camera-height plus DPI/window-mode sweep | Derive continuous counter anchor and scale law. |
| H-CAP6-H2 | Stack sizes around grouping thresholds | Derive slot order and aggregation thresholds. |
| H-CAP6-H3 | Allegiance, country-color mode, and selection matrix | Derive exact tint and selection-state rules. |
| H-CAP6-H4 | Simultaneous counter badge states | Derive badge precedence and layer ordering. |
| H-CAP6-H5 | Enemy counters across FoW/intel levels | Derive hidden/partial presentation behavior. |
| H-RG-H1 | Stationary over-0.5 HDR luminance convergence series | Close temporal grading/exposure behavior with authentic consecutive payloads. |
| H-RG-H2 | Separately admitted default and one-variable wide-bloom-width scenes | Derive bloom width without visual tuning and pass both exact bloom regrades. |
| H-RG-H4 | Separately armed single-frame transparent political control followed, after its completion, by a visibly striped F6 Resistance capture | Prove the secondary-map CPU producer/upload, same-submit D80 identity, distinct replay readback, and zero-diff route; one immediate two-frame request is invalid. |
| H-ONEDRIVE | One genuine OneDrive Files-On-Demand replay store observed with the provider enabled and then disabled against the same two dehydrated, index-listed snapshot identities | Exercise identity-bound hydration in the enabled arm and exact typed refusal without ordinary-file substitution in the disabled arm; 47,621 files were scanned locally and zero qualifying placeholders exist. The runner and independent receipt verifier own all selection, hashing, test, and interpretation work. |
| H-MODSET | Selected installed-mod matrix and one current seven-role static bundle per chosen mod set | Authenticate effective-VFS bundle generation before publication; retained bundles have deferred token joins and the old D04 bundle is schema v2. |

### Live acceptance and fault behavior

| ID | Required controlled input | Exact acceptance purpose |
|---|---|---|
| H-SINK-OK | **No separate input.** Every normal production Safe-launcher rung automatically runs `verify_sink_reconciliation.py --expect success` and binds its report into the capture-validation receipt. | Retain nonzero collector and host-write samples, monotonic wall time, terminal sole-pipe reconciliation, manifest acceptance, and graceful replay finalization without scheduling another launch. |
| H-SINK-FAIL | One `Invoke-SinkFailureAcceptance.ps1` early-launch arm (`sink-open-fault` or `sink-write-fault`) | Retain the recorder-authored pre-injection plan and authenticated `collector_sink_failure.json` v2 receipt. The arm, exact report matching, non-publication check, and regrade are automated; no manual fault construction remains. |
| H-F6-PAUSE | One fresh nonce-bound paused-short-proof attempt after the synthetic-fault predecessor | Exercise the fully automated paused proof; authenticated D57/replay state replaces visible acknowledgement. |
| H-SOAK | One uninterrupted six-hour product soak on the selected deployment profile | Require a >=21,600-second request, >=21,600,000,000 us wall and monotonic observations, and the selected-profile/manifest/plan-bound `six_hour_soak_acceptance_receipt.json`; validate rollover, heartbeat, measured memory/storage growth, and replay continuity against the selected profile policy. |

## Remaining engineering and repository-authority work

These are **not human reverse-engineering requests** and must not be misreported as such:

1. CAP6 Wave 6 closes the live owner/path/common-draw RE and wires the exact selected-Present final-D3D collector. The route remains fail-closed as `UNAVAILABLE_ZERO_DRAW` until one central build/injection run emits and mechanically accepts H-CAP6-PRESENT. H1 through H5 remain behavioral variation observations; they do not replace or authorize the owner/path receipt.
2. The G3 lexical classifier is diagnostic only and is not valid promotion proof. Occupation, SnowMud, rails and the Wave 6 shadow/tree paths now have exact typed or producer boundaries, but promotion still requires their named authenticated raw producer/consumer joins. Camera requires the exact C02 current/target binary32 updater and the G2 synchronized zero-delta pixel pair; no token or descriptor substitutes.
3. The offline product runtime now has durable append tickets, rollover propose/drain/seal/open/ack/header barriers, dormant paired-runner heartbeat, and graceful H4CT close before H4CR drain. Activation remains disabled until concrete live adapters, CID1 proof, and the active legacy spawner's hard-kill/finite-stdout boundaries are replaced or authenticated.
4. The XUI21 fire/materialisation/resolution graph, multi-slot context contract, and authenticated one-shot weather-worker fault tooling are implemented. XUI21 now needs its named exhaustive runtime coverage matrix, not additional producer discovery; weather fault acceptance remains a separate named live observation.
5. Domain 81's exact-ten owner, enumeration, authentication, and frame-law authority is closed. The authenticated fail-closed manager/type-`0x1a` double-read hub collector is implemented and activates the existing v2 encoder, recorder fold, authority accounting, and replay-ingest route only for a complete snapshot; v1 remains the explicit unavailable fallback. The promotion-manifest authority remains SHA-256 `32388518a63eaa22ea6290e82fb3677a5515039e937334ebc2c2d26a0c0bc6b1`. Central validation has passed; no human capture is required.
6. A11/A30 owner, assignments, recruitable/portrait/action identity, network proof and all seven XUI14 surface producers are integrated and centrally validated. Exact evidence shows no engine network collection/order for the product-authored summary. The remaining selected-Present matrix is autonomous machine evidence, not human input or unfinished semantic RE.
7. The weather named-token, definition, entity-selector and SnowMud CPU producer/upload boundaries are closed. Active weather remains false pending the existing early-launch runner's two content-changing H rows, matching replay H2 readbacks and selected-Present output evidence.
8. XUI27 replay-owned final composition is proven and centrally validated. Future source changes still require its ordinary content-addressed refreeze, but that bookkeeping is not a new RE or human-input gate.
9. The strict package-runtime boundary now provides deterministic manifests, signing admission, atomic active/previous pointers, rollback, locks, Windows-safe paths, and reparse-point refusal. Product owners must still select the production signature algorithm/key custody and rotation/revocation policy, installer/update entry point, trust-store distribution, and tray/window surface.
10. Pure Cargo/cache scratch totals about 17 GB and can be regenerated. The two mixed audit trees contain superseded intermediate source even though they contain no unique production evidence; archive that source history before deleting those mixed trees.
11. A38 exact-20 authority and d84 transport are complete, but the existing read-only probe and autonomous harness acquisition path have not yet been exhausted. It remains non-human machine work; no operator card is justified until those paths fail to materialize the required live frame.
12. A34's exact country-decision graph, DPN1 v2 transport, selected-Present join, fold, replay admission, heterogeneous UI model and centralized validation are complete. Nonempty/measured-empty runtime evidence remains mechanically acquired; caller-provided evidence is never promoted as an inferred production value.
13. C07 generic-building record layout, current grow-in presentation scale, pointer-free carrier/decoder/attachment, and replay executor are wired fail-closed. Retained objects and draw oracles do not contain the stable runtime-object/entity/output join, so generic promoted rows remain zero. Acquire one complete automated pinned-a729 selected-Present observation across the five roots named in `C07_GENERIC_BUILDING_PRODUCTION_WIRING_20260720.json`; no human semantic input is required and activation remains false.
14. A13's supported current global/country/state variables and flags are production-complete in d51. The carrier preserves exact token, DateHour, and value while deliberately refusing unauthenticated dictionary text. Delayed lifecycle belongs to A19/d64, scripted-GUI materialization belongs to A38/d84, and arbitrary engine scopes remain unavailable; none is a human A13 request.
15. A24's static producer graph, exact sentinels, reversible same-thread multi-tap evidence owner, PTI1 typed replay leaf, and fail-closed UI selector are complete. D73 remains reserved and unscheduled while `LIVE_PRODUCER_TAPS_COMPLETE=false`; the multi-case selected-Present matrix must collectively authenticate all six mutually exclusive families with zero loss before activation. No human semantic row is authorized.
16. A26 final-blob engineering and centralized validation are complete: the owner session, atomic selected-Present TTI2 carrier, fold, strict typed replay, actual production-renderer HUD consumer, matrix assembler/schema, and independent verifier are present. D72 deliberately stays reserved and unscheduled until H-A26 passes; stale historical manifests are not promotion authority.

## Closed without further human input

- M02 and M03/A48 production contracts.
- A24 static producer graph, evidence hook owner and replay binding, A33/A34 core mechanics, d75 fired-event lifecycle, and the pre-folded decoder seam. A24's live matrix and A29's observed-producer receipt remain separately tracked activation gates.
- A16 exact current-status producer law, current 15-root production re-audit, AUST transport, signed-score/adjacency and gap-safe fold, pointer-free replay/UI integration, and consecutive-witness AUST-aware fixture grading. Wave4 passed centralized validation; the current Wave5 hardening awaits the root-owned central pass. Its five-state live matrix is optional regression evidence only.
- A17 nuclear-stockpile semantic law and d83 transport. Authenticated live-source admission is machine engineering; a positive built-or-ready fixture is optional regression evidence only.
- Portable replay strict JSON, archive/VFS, path portability, integrity, and rollback-index hardening.
- Sink telemetry, exact IO classification, authenticated open/write fault arms,
  durable plan/receipt publication, and strict independent regrading. Only the
  named external launch observations remain.
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
- A13 supported current global/country/state variable and flag transport; unsupported arbitrary scopes are explicit absence rather than an inferred value or human-input request.

## Promotion gate

Production promotion requires all of the following:

1. Central debug and release batteries remain green after the final authority refresh.
2. O04, XUI27, and G3 recursive verification report current - not stale or substituted - evidence.
3. Every applicable human-input row above produces an authenticated packet and passes its packet-local verifier.
4. Product integration, rollover, sink-failure, packaging, and soak gates pass.
5. The final recount contains no unowned, fabricated, stale, or silently unavailable production field.
