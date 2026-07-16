# Master Extractor Completion Contract

The active top-level checklist for deciding whether the recording layer is done for a full 1:1
Battle Plan. This is the **only** active completion ledger; older plans/notes/packets are evidence
only under `extractor/documents/archive/`.

**Where things live (single-source, don't duplicate here):**
- Per-domain IDs / status / schedulability → `extractor/native/domain_registry.json` (code-authoritative mirror `compact_contract::ASSIGNED_COMPACT_DOMAINS`).
- Exhaustive per-datum census (every save path, per-country schema, authoritative runtime state,
  and explicitly excluded host-local presentation state) → `extractor/documents/EXTRACTION_ATLAS.md`.
- **What is cracked but not yet wired** → `extractor/native/runs/crack_fanout/CRACKED_LEDGER.md`.
- **What is NOT yet cracked** → the OPEN ITEMS REGISTRY below (the single working backlog).

This contract holds the normative rules (completion, gate, done-means), the open backlog, and a
coverage map. If every open item is closed or dispositioned, the extractor has no known missed
replay-relevant data.

**Authority rule:** domain status values in this document must be exact registry keys. The current
code vocabulary is `existing`, `production_complete`, `production_complete_base`,
`validated_slice`, `validated_partial_slice`, `validated_slice_open_gaps`, `active_integration`,
`reserved`, and `infrastructure`. Labels such as `SHARED_INTEGRATION_PENDING`, `SHARED`,
`PRODUCTION_SLICE_DONE`, and `ROOT_DISCOVERY_OPEN` are historical planning labels, not domain
statuses, and must not appear in the domain-status table. A registry status describes code
integration state; it does **not** by itself close a gameplay concept or waive the Production Gate.

**Production-data scope rule:** every save-serialized datum is in scope for semantic coverage, as
is authoritative host-side simulation state or an event needed to reproduce or explain the shared
multiplayer world. Stable identities, joins, timing, provenance, and completeness witnesses needed
to interpret that state are also in scope. Coverage may use one-time/static records, deltas, or
events rather than repeating unchanged values every hour.

**Host-local presentation exclusion:** STATUS: PROPOSED — owner ratification owed (2026-07-16); aligns
with the standing observer-interaction rulings. Camera/viewport state, selections, hover targets, open
screens/dialogs, local map-mode and layer toggles, outliner state, tooltips, drag state, and other
mouse/keyboard/UI preferences are explicitly out of production extractor scope. This remains true
when such state is reachable in the host process: it describes only the host client's presentation,
is not authoritative multiplayer world state, and cannot represent remote players' local clients.
The replay interface owns its own view state and reconstructs HOI4-like UI from captured facts.
These exclusions are not blockers, fixture needs, or RE targets and must not enter `.battleplan`.

## Current state (2026-07-15)

The registry and code contain a broad working extractor, but the extractor is **not** at production
closure. Wave 0 restored the completion authority and workspace baseline; Wave 1 implemented the
identified correctness repairs with synthetic/targeted gates. Neither wave supplies a live speed-5
acceptance proof.

- **Authority/build:** the master table mechanically matches all 31 non-legacy/non-infrastructure
  registry rows. The three F1a Cargo surfaces and seven dependency-free H4CR parsers are directly
  certified (F1a/F1c), and the maintained production-script suite executes all three package gates
  (F4). `scientists_live_lane` has also joined the main native workspace.
  The final integrated matrix is not wholly green: `replay_decoder` still has one fixture failure and
  `snapshot` still needs the Windows Developer Mode namespace capability (F1b/S4).
- **Correctness:** d61 read semantics, canonical failure masks, panic containment, schedule
  validation, total short-header parsing, and the extractor-owned shared stream-certification
  migration are implemented (D8). D45 FRD2 v3/60 now carries raw DATA status at `+56`, exactly
  reconciles it with PERF, and is offline-green. Final live proof remains F6.
- **Sink authority:** full-write/short-write/error handling and terminal poisoning are implemented.
  An hour is authoritative only after the complete record reaches the recorder pipe, and production
  acceptance requires that pipe to be the **sole configured sink**; a file+pipe mirror cannot prove
  the same record reached both destinations (D6/F2).
- **Static and speed admission:** d57 control authority is closed offline (D10). The runtime builder,
  exact v2 source inventory, ordered active-product DLC/mod layers, and all **seven** required static
  roles—including `technology_sharing_static`—are integrated and fail closed (S2/S5/D69). No fresh
  current-session D-0-4 seven-role bundle or speed-1-to-5 proof has passed production (A3/S9/F6).
- **Product composition:** auto-attach, campaign identity, stitching, H4CT framing/heartbeat, and the
  restrictive overlapped Windows control-pipe adapter have offline foundations. Explicit-mode
  CID1/CFA1 validation, durable receipt ordering, and recorder-main manifest admission are green
  offline; the collector's sequence-0/CFA1 producer barrier is still unapplied. Activation remains
  false, the active path remains Legacy, and no pillar has completed its live lifecycle gate (P1-P5).
- **Coverage:** fixture work, RE follow-ups, save/authoritative-runtime closure sweeps, atlas/oracle completion,
  and explicit low-value-tail dispositions remain open (A, B, E, S7-S9, and X).

Integrated foundations that remain valid evidence include the compact registry and readers, typed
fold coverage, per-domain malformed-payload degradation, auto-attach discovery/lease/journal logic,
campaign-identity grammar/resolver code, deterministic stitch planning, and v2 stitched-container
validation. None of those statements imply that the corresponding end-user product path is wired.
- **✔ OWNER DECISION RESOLVED 2026-07-14 — malformed-payload semantics = PER-DOMAIN DEGRADE (zero
  data loss).** Owner ruled zero data loss. A structurally-malformed/truncated DOMAIN_DATA payload
  now degrades **only its own domain** — the stream-boundary ingest gate (`stream_fold.rs`
  `drive_fold`) records a `malformed_payload` domain problem, marks the domain unknown, keeps the
  prior carried leaf for diagnostics, and **never applies the untrusted bytes** (so nothing can be
  smuggled into another domain) — while every other domain and hour keeps folding. **Framing / magic
  / version / header-length / payload-bound / storage-checksum / zstd corruption still fails closed**
  (it cannot be localized to one domain). New `WorldFold::record_malformed_domain_payload` routes
  through the existing `mark_fold_domain_unknown` machinery (+ sets d53's `seen_in_stream`/`known`
  safety signal, the only domain with the "modern-stream ⇒ unavailable-on-absence" pattern); new
  `FoldStreamStats.malformed_payload_records` counter; the 5 DOMAIN_DATA-payload boundary tests now
  assert per-domain degrade; the 2 acceptance oracles (`folded_malformed_d45_without_d12...` in
  `world.rs`, `production_encode_decode_fold_typed_unavailable...` in `world_timeline.rs`) are
  **un-parked and passing**. Consumer side unchanged — `reconcile_domain_problem_statuses`
  (DOMAIN_PROBLEM_VIEW_MAP 45→Focus, 53→Diplomacy) already downgrades any recorded domain problem.
  Lane `w12h-malformed-degrade`. Independent gate run: `recorder_host --lib` 421/0/2, `stream_fold`
  suite 34/34, `replay_core` 83/0 (+1 pre-existing ignore), both oracles pass. NOTE: the independent
  Codex adversarial gate could not emit a verdict — the Windows sandbox helper
  (`codex-windows-sandbox-setup.exe`) broke mid-session (`program not found`, 3 attempts incl. Codex's
  own review); substituted by orchestrator adversarial review (anti-injection proven by construction:
  the degrade branch advances by the full framed `record_len` and `continue`s before any apply, so an
  embedded record is never framed/counted/applied — the flipped swallow tests pin `data_records == 1`).
  Resumable review tid saved under `hoi4_lanes/_tids/` for a re-run once the helper is restored.

**Final user-gated acceptance proof:** a fresh speed-5 end-to-end capture proving the exact
production-profile roster emits at <100ms/frame with zero skipped/coalesced hours and no untolerated
status. It is the final live gate, not the only remaining engineering work. Current paused-D04
evidence cannot substitute for it.

## OPEN ITEMS REGISTRY — NOT YET CRACKED (single source of truth)

The one place to work through everything still uncracked. Cracked-but-not-wired lives in
`CRACKED_LEDGER.md`, not here. Check an item off only when cracked+validated (move it to the ledger)
or consciously dispositioned. Categories: **A** needs a user fixture; **B** needs another extractor
RE pass; **C** is an external replay-layer handoff and is not extractor implementation scope; **D**
is a committed-code correctness bug; **E** is an atomized low-value tail awaiting disposition;
**F** is build/acceptance infrastructure; **P** is product-session composition; **S** is static/oracle
closure; and **X** is a coverage-sweep decision. C items are recorded only to preserve the handoff —
they do not block extractor completion and must not be implemented or edited from the extractor lane.

### A. FIXTURE-BLOCKED — needs a stated live/save game transition
- [ ] A1. Hub motorization LEVEL/settings — toggle ≥2 hubs to distinct levels (d62 node membership done; per-level value needs variance)
- [ ] A2. Focus-paused bit — pause one national focus (d45; D04 has no paused variance)
- [ ] A3. Game-speed live acceptance — the d57 reader and strict control-file integration now bind
  the declared current build to `CGameState+0x4BC`, `raw 0..4`, and speed-5 raw `4`. Before live
  acceptance, retain a real speed-1-to-5 multivalue cycle plus pause/resume orthogonality evidence.
  The tracked machine-readable probe only
  contains paused `[1,1,1]` samples and explicitly leaves the speed flip unverified; synthetic or
  asserted control metadata is not live proof.
- [ ] A4. Air combat lifecycle: sortie lifecycle / kill attribution / last_killer (d42)
- [ ] A5. Ace kill_type / wounded variance (d46)
- [ ] A6. Convoy/port disrupted-supply PRESSURE (nonzero) — blockaded island / cut pocket (d44)
- [ ] A7. Convoy combat loss route/context — active convoy battle (d44)
- [ ] A8. Naval mission enum / home base / repair / retreat / route semantics (d56)
- [ ] A9. Naval combat lifecycle-end + member join/leave/sinking — battle teardown (d49-50)
- [ ] A10. Running-operation owner/target contrast (operative loc ≠ target) (d53)
- [ ] A11. Spy-network / intel-map percentages, same-tick (d53)
- [ ] A12. Decision numeric state/province targets + cooldowns (d45)
- [ ] A13. Scripted-GUI open/state (d51) — OPEN. Only the host-local widget-state portion (which
  panel is open, scroll/selection inside it) is excluded, and only under the *proposed*
  presentation-scope ruling that is still owed owner ratification (2026-07-16). The authoritative
  scripted-GUI state remainder is in scope and still owed: the standing owner scope ruling names
  country scripted-GUIs (GER_inner_circle, JAP_imperial_influence, RAJ_famine) as in-scope, and
  their authoritative scripted flags, variables, delayed events, and direct state-changing hooks are
  tracked via X10 and d51/d64 (d51-d64) and remain open
- [ ] A14. Construction repair semantics/speed — damaged building repairing (d48)
- [ ] A15. MP chat/messages, hotjoin, tag-switch — 2-player same-build MP (d57)
- [ ] A16. Subject autonomy CHANGE dynamics and `CSubjectRelation+0x60` ratio semantics — static
  levels are done; exercise a real autonomy change and compare the relation value before/after (d53)
- [ ] A17. Nukes leaf offset — a country with ≥1 built nuke; container
  `CCountry+0x1310→CNuke` is ready, but no production domain assignment exists yet (must not be
  misreported as divisions domain d20)
- [ ] A18. Dynamic-modifier per-instance value/expiry — a country carrying a timed modifier; id-set
  is done and the authoritative live lane is d67 `country_dynamic_modifiers_live`
- [ ] A19. Delayed-event `+0xC4` decrement-over-time + originator-vs-scope-country separation;
  static structure is done and the authoritative rooted lane is d64 `delayed_events_live`
- [ ] A20. Weather region-phenomenon-flags drift confirmation (rain_heavy/arctic_water 305/305 now) — unpaused drift (d24)
- [ ] A21. Finest per-entity combat killer label (per-convoy / per-sunk-event / per-wing) — one controlled combat (combat)
- [ ] A22. Expand cumulative air-loss statistics from the 6 value-pinned channels to the complete
  24-channel stat map and validate nonzero changes in a live air-combat fixture. Domain 61 is already
  committed, integrated, schedulable, and production-profile eligible; this is a field-expansion and
  live-semantic gate, not an integration task.
- [ ] A23. `CProgram` owner-tag live validation — current-build constructor/caller RE makes
  `CProgram+0x170` a strong u32 country/tag-handle candidate copied with `+0x18`, paired with the
  province at `+0x174`/`+0x1C`; the old `program_index` label is stale for the current executable.
  Do not integrate it yet. After every pre-live gate, an anchored paused-D04 read must prove both
  field-pair equalities and exact save-owner equality for all 26 programs, including unique
  resolution of the five aliases, with adjacent negative controls. Existing fail-closed drops stay.

### B. DEEP-CRACK FOLLOW-UP — static roots plus remaining inner/runtime gates

The current bounded query order and exact same-tick acceptance oracles are in
[`W14_MARKET_NAVAL_ROOT_RE.md`](dossiers/W14_MARKET_NAVAL_ROOT_RE.md). The 2026-07-15 bounded static
pass completed in the required order B4 → B7 → B1 → B6 → B2. None of the five items is complete:
B1/B2 still need inner-container RE, while B4/B6/B7 still need orchestrator-only paused-D04
acceptance and production integration. Do not interact with or inject into the paused game until the
static bundle and every other pre-live gate pass.

- [ ] B1. Market stockpile owner/iterator — save `countries/<TAG>/equipment_market` is rooted at the
  pointer at `CCountry+0xFB8`; the stockpile object is nested at `(*status)+0x38`. The global object at
  `CGameState+0x3E8` is a separate market manager, not the per-country stockpile. Recover the nested
  object's authoritative serializer/iterator, then require exact full-country row multisets rather
  than a superset or sample match
- [ ] B2. Convoys owned total — the country-side root is the embedded convoy object at
  `CCountry+0x1200` (vtable VA `0x1427D0C60`; known extent at least `0x88`). Recover its serializer/
  inner equipment iterator and the admitted-static variant-to-definition/archetype join. Runtime
  variant type `70` is generic and must not be treated as a convoy discriminator; acceptance requires
  exact per-variant sets/totals and explicit disposition of save countries lacking live roots
- [ ] B3. Garrison equipment id/type/amount — deeper equipment-bank (D5: 0 hits depth-4 BFS; oracle saved)
- [ ] B4. `sunk_convoys_history` live acceptance/integration — static root and iterator are closed:
  `CGameState+0x5A8` is the pointer-vector start and `+0x5B4` is the i32 count. Each
  `CSunkConvoyInfo` has extent `0x18` (not `0x20`), with month/count/killer/owner at
  `+0x08/+0x0C/+0x10/+0x14`. After the pre-live gates, reproduce the exact 441 unique rows with sum
  2810 in an orchestrator-only paused proof, then integrate through the single hot-file owner
- [ ] B5. War threat / instigator / hostility reasons — still unpinned. The validated d60 row
  substrate currently ends at `+0x70`, and the preserved generic static scan is not row-base-aware.
  After every pre-live gate, extend only the already-discovered d60 row windows to at most `+0x120`
  and extend the save oracle, then require all-row equality, useful variance, typed bounds, and
  adjacent negative controls. No new heap scan; if paused D04 lacks variance, remain fixture-blocked.
- [ ] B6. Naval anti-air live acceptance/integration — the current-build naval-side serializer writes
  save token `anti_air` directly from the 64-bit side member at `+0x150`; no ship/module aggregate or
  definition-derived formula is justified. Resolve the prior failed read as a side-root/raw-decoding
  issue, then match all eight D04 sides, including all three nonzero values, before integration
- [ ] B7. Raids live acceptance/integration — static root and both iterators are closed. The correct
  top root is `CGameState+0x3F0` (not `+0x3F8`); the manager's inline status array uses
  start/capacity/count/allocator at `+0xD8/+0xE0/+0xE4/+0xE8`, stride `0xB0`, and each status's child
  pointer vector uses `+0x18/+0x20/+0x24`. After the pre-live gates, require exactly the 158
  save-authoritative children under 440 country statuses plus explicit dummy classification
- [ ] B8. Construction building token/name — resolve numeric type id → token through an extractor
  static-definition map; only return to memory RE if the static mapping cannot be proved complete
- [x] B9. Subject ratio — merged into A16 so the same live transition proves both autonomy dynamics
  and `CSubjectRelation+0x60`; do not count as a second backlog item
- [x] B10. `CProgram` owner-tag offset — transferred to fixture item A23 because paused D04 is the
  required oracle; do not count as a no-fixture RE item

### C. EXTERNAL HANDOFFS — replay layer, explicitly out of extractor implementation scope

These are retained so extraction does not accidentally re-open rejected memory searches. They are
not extractor checkboxes, are not part of extractor "done", and must not cause edits under replay
directories from this lane.

- **C1 handoff.** World tension / average_major_ic / tension_scaling_base_country — derived decayed
  sum of the threat event list (D3)
- **C2 handoff.** Realized province temperature/water/snow — derived/non-resident (D6)
- **C3 handoff.** Outgoing diplomatic queue — transpose of incoming (no distinct structure; D4)
- **C4 handoff.** Market sibling pools produced/imported/used/to_export/base_export — derive from
  states + trade routes (MTRF)
- **C5 handoff.** Country factory/resource/consumer-goods UI counters — reserved d58; derive from
  state buildings/offmap + inputs
- **C6 handoff.** Replay-generated alerts/message presentation may be derived from authoritative
  facts. The host's alert list, popup state, and outliner expansion/scroll/filter state are local
  presentation excluded by A13/X13, not data to poll, derive as historical state, or extract

### D. COMMITTED-CODE FIXES — bugs found by discovery, apply during wiring
- [x] D1. d53 subject-relation reads 0 subjects on 870ebf — FIXED in W2 (2c3c1e98e): added the `CRelationStatus+0x288` pointer hop → 38/38.
- [x] D2. Naval positioning read emits a bogus constant 1.0 (`f32@NavalCombatSide+0x288`) — FIXED in W5 (1ac13f761): now fail-closed → None; d49 wire layout byte-identical.
- [x] D3. d61 positive absence vs read failure — tri-state read semantics implemented; true absence
  is clean while memory/layout/read failure emits `PARTIAL_DATA`
- [x] D4. Canonical failure mask — verification derives every failure bit from
  `COMPACT_STATUS_FAILURE_MASK`, with expected-partial cases tested separately
- [x] D5. Panic containment — affected reader workers convert panic to
  `WORKER_PANIC|PARTIAL_DATA`; no unwind crosses the frame coordinator
- [x] D6. Exact sink completion — full-write loops, short/zero/error failure, terminal stream
  poisoning, sink telemetry, and latch-after-delivery are implemented. Production certification
  additionally requires the recorder pipe to be the sole sink
- [x] D7. Schedule validation — missing schedules resolve from the registry production profile;
  empty, duplicate, malformed, unknown, or typoed schedules fail with `CONTROL_INVALID`
- [x] D8. Complete extractor-owned shared stream certification migration — canonical sequence,
  record semantics/flags, closed-hour/physical-EOF, FRAME_END, and DATA/PERF authority are enforced;
  the repaired dependency-free parsers and lane proof surfaces have direct certification
- [x] D9. Boundary safety — compact-header parsing is total for short buffers and snapshot-worker
  creation failure is contained without panic
- [x] D10. d57/control authority — one typed deny-extra control object, production latch, independent
  gate-file/running-executable binding, retained speed/pause transcript, exact one-row-per-frame
  evidence, and distinct `CONTROL_INVALID` reporting are implemented and certified offline
- [ ] D11. Sink-intent/failure observability — distinguish requested sinks from successfully opened
  sinks, fail production when an intended sink cannot open, and preserve terminal transport-failure
  evidence out of band because a poisoned sole stream cannot carry its own final failure probe
- [ ] D12. d55 zero-silent-gap/fold correctness — preserve ended combat events after d21 removes the
  battle from the active set; distinguish unreadable/torn side-detail and attribution containers from
  genuine empty attribution; add per-side completeness plus end/rebaseline fold gates

### E. DEPRIORITIZED / LOW-VALUE TAIL — disposition consciously (don't silently drop)
Save-backed but judged low replay value or redundant. Each row is independent; unchecked rows still
need an evidence-backed `crack-if-cheap`, `redundant`, `derived`, or `skip` disposition.

- [x] E1. `ai` — skip; AI implementation state is not an observed gameplay fact
- [x] E2. `ai_strategy` — skip for the same reason as E1
- [x] E3. Per-country `buildings` — redundant with the authoritative state/building capture
- [x] E4. `cached_navy_strength` — derive within extractor validation from captured OOB/ship rows;
  do not capture a stale cache
- [x] E5. `theatre` id counter — skip allocator state
- [x] E6. `navy` id counter — skip allocator state
- [x] E7. `land_combat` id counter — skip allocator state
- [x] E8. `industry_org` id counter — skip allocator state
- [x] E9. `instances` id counter — skip allocator state
- [x] E10. `id_counter_store` — skip allocator implementation state
- [x] E11. `debug_current_ref_id` — skip debug-only allocator state
- [ ] E12. `history` — decide whether any rows are not already covered by event/stat domains
- [ ] E13. `history_queue` — decide whether this contains unique replay-relevant transitions
- [ ] E14. `tokens` — enumerate token families and bind each to live/static coverage or rejection
- [ ] E15. `collaboration` — determine whether it represents observable occupation/diplomacy state
- [ ] E16. `claims` — reconcile against map/country claim coverage
- [ ] E17. `cores` — reconcile against map/country core coverage
- [ ] E18. `deployment` — reconcile against production/deployment queue work in X4
- [ ] E19. `reinforcement` — determine whether it adds reason/state beyond division equipment deltas
- [ ] E20. `upgrades` — reconcile against equipment/variant upgrade domains
- [ ] E21. `name_group` — decide static-definition join vs omit
- [ ] E22. `external_rules` — identify user-visible semantics or reject as engine configuration
- [ ] E23. `focus_tree` — reconcile static tree identity against live focus rows
- [ ] E24. `continuous_focus` — decide whether current selection/state is already captured
- [ ] E25. `preferred_tactic` — determine whether it is UI-visible player intent
- [ ] E26. `is_top_ic` — decide derived/session-context treatment
- [ ] E27. `major` — reconcile current dynamic major status with country/session rows
- [ ] E28. `capital` — reconcile against authoritative country/state ownership
- [ ] E29. `original_capital` — determine whether unique historical state must be captured

### F. BUILD, VALIDATION, AND OPERATIONS

- [x] F1. Named workspace baseline defects — proof binaries have unique target names,
  `FoldStreamStats` callers are current, and the Python oracle fixture matches required-output
  semantics; focused Wave 1 gates passed where reported
- [x] F1a. Certify the three originally manifest-local package surfaces:
  `project_progress_live_lane`, `scientists_live_lane` (now also a main-workspace member), and
  `lanes/occupation_resistance_compliance_live/replay_verify`. Run each manifest-local test/build and
  either add it to maintained automation or record a proof-backed retirement/integration decision
- [x] F1c. Certify the seven dependency-free H4CR parser surfaces after their shared
  `compact_contract/src/record_header.rs` migration: the two
  `army_oob_orders_battleplans_live` parsers, `division_state_movement_live/domain20_compact.rs`,
  `naval_oob_fleets_ships_live/domain28_29_compact.rs`, occupation d44/d52 gates, and
  `scripted_state_domain51_stream.rs`. `naval_missions_status_live` is Cargo-native, not an eighth
  direct-`rustc` parser; its stale README invocation is replaced by its manifest-local Cargo gate
- [ ] F1b. The integrated workspace check and almost all targets are green, including `fold_check`
  17/17, production scripts 46, `live_capture` 770 passed/6 skipped, and `game_defs` 162 passed/20
  skipped. Closure remains blocked by one `replay_decoder` fixture failure (20 passed/1 failed): the
  failing test is `replay_decoder::tests::perf_payload_and_announced_trailer_are_exact` (PERF
  FRAME_END domain-count certification against a stale fixture — fix in flight this wave). `snapshot`
  is NOT a blocker here: it is 39/39 green, and its Windows Developer-Mode namespace arm is an
  env-gated `#[ignore]` test (skipped, not red)
- [x] F2. Safe production-attempt implementation — unique staged DLL, one injection attempt,
  production latch, sole recorder-pipe authority, and one strict post-capture validator are wired;
  nonzero exit, incomplete roster/hour, bad flags/latch, truncation, decode/fold/container failure,
  static-bundle failure, or speed-proof failure is fatal
- [ ] F3. Run the Windows capability gates: Developer Mode symlink rejection, dehydrated OneDrive
  store, WMI live source, Job-object lifecycle, and multi-GiB streaming/container stress
- [x] F4. `native/scripts/Test-ProductionCaptureScripts.ps1` executes all three F1a Cargo surfaces
  sequentially with `--all-targets`; none of those offline package gates has an environment skip
- [ ] F5. Make acceptance telemetry truthful: verified game speed, p50/p99/max frame and sink time,
  skew, per-domain bytes, prewarm cost, attempts/coalesced/in-flight accounting, and exact roster
- [ ] F6. Run the acceptance ladder in order: synthetic faults, paused short proof, speed-5 short
  proof, five-minute full roster, six-core weak-PC run, six-hour soak, and full-game size projection

### P. PRODUCT SESSION COMPOSITION

- [ ] P1. Auto-attach product path — discovery/WMI/singleton/lease/journal supervision and both launch
  orders feed the production recorder with an exact capture id, canonical UUID session id, staged-DLL
  provenance, production profile, static bundle, and packaging. Product closure still requires the
  activation-disabled H4CT collector client, process-lifetime graceful-stop/recurring-heartbeat proof,
  complete Known Folder/OneDrive/`-userdir` discovery, and lifecycle/crash acceptance
- [ ] P2. Duplex H4CT activation — framing, deadline/cancellation channels, peer/capability admission,
  recurring-heartbeat primitives, restrictive overlapped Windows named pipes, explicit-mode CFA1,
  durable receipt ordering, the real paired H4CR/H4CT server, and the recorder-main one-shot-injection
  composition seam are built and focused-green. Activation remains deliberately false because the
  collector has no H4CT/CID1 client and the product permit is unmintable. Production durable-sink/
  stdout replacement, live peer/deadline/cancellation/liveness proof, and rollover remain open
- [ ] P3. Campaign identity — hash-bound static serializer/deserializer evidence in
  [`W15_CAMPAIGN_IDENTITY_STATIC_RE.md`](dossiers/W15_CAMPAIGN_IDENTITY_STATIC_RE.md) proves, for
  executable SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`,
  candidates `i32[CGameState+0x720]` (`game_unique_seed`) and an MSVC `std::string` object at
  `CGameState+0x728` (`game_unique_id`). Shared CID1 parsing/stateful validation and recorder-main
  explicit-mode manifest admission are offline-green, but the collector sequence-0/CFA1 barrier is
  unapplied and the active path remains Legacy. Do not add a production reader, build-profile
  fallback, CID1 activation, or live manifest authority until every matrix arm below passes.
  - **L0 paused D04:** exact running-image/root and same-hour brackets; one coherent
    `+0x720..+0x747` read; exact seed `1026046527` and id
    `a4b17fed-ed12-4361-9064-fb6576638110`; two complete identical reads bracketing the first proposed
    capture boundary. A 2/2 match only admits an exact-build candidate.
  - **S1:** same process, save and reload that exact save — pair equal; a lower/equal hour may only
    begin a new segment.
  - **S2:** reload an older save from a lineage whose pair was first measured — pair equal despite
    hour rollback.
  - **S3:** close the game and relaunch the same save — pair equal across process and d68-session change.
  - **S4:** rename/copy the measured save and load it — pair equal; path and filename are irrelevant.
  - **D1:** create two independent campaigns with identical build/modset/bookmark/countries — raw
    pairs and digests differ; any whole-pair alias blocks activation.
  - **D2:** load measured campaign A then measured campaign B in one process — identity changes before
    B's first world row and no row crosses the boundary.
  - **M1:** host and hotjoin client at the same game hour — complete pairs equal despite roster/role.
  - **M2:** host and observer at the same game hour — complete pairs equal without a controlled country.
  - **M3:** rehost the same save on another machine/player — complete pairs equal across machine,
    persona, path, and host-country changes.
  - **Failure matrix:** unreadable/malformed/torn input is explicit `unavailable`; one-component
    change is `identity_component_conflict`; a stable whole-pair change stops admission pending an
    acknowledged rollover; digest equality with unequal raw components fails closed.
- [ ] P4. Reload/relaunch stitching — wire existing planner and v2 container support into production:
  durable append-ticket ledger, pre-fold segmentation, fresh per-segment folds, identity seam,
  superseded-source retention, atomic publication, and crash/reload/exit lifecycle matrix. Stitch
  activation remains blocked until P3's complete identity matrix and boundary behavior pass
- [ ] P5. Harden dormant stitch inputs: empty recovery-segment transition coverage, full raw-source
  extent participation, strict digest grammar, and strict transition-evidence grammar

### S. STATIC ARTIFACT AND ORACLE CLOSURE

- [x] S1. All seven contract-required artifacts are mandatory and fail closed in production before
  injection and again before publication
- [x] S2. Catalog bytes are bound to the exact v2 source inventory and ordered override namespace;
  canonical role schemas, parsed-versus-hashed descriptor identity, source-gate hashes, and
  mixed-time resolver checks fail closed offline
- [x] S3. Cross-artifact localisation/source hashes are verified; inconsistent bundles are rejected
- [ ] S4. Publication/repack integrity code paths are closed: canonical verifier authority,
  consumed-byte identity, static bundle/index/container reconciliation, atomic last-good
  publication, and `replay_not_produced` failure semantics are implemented. This checkbox remains
  open until F1b's replay-decoder fixture and snapshot namespace-capability blockers clear
- [x] S5. The runtime builder handles ordered active mods plus integrated/active-product DLC layers,
  binds roots and fingerprints to authoritative inputs, excludes asset-only namespaces from
  definition parsing, and derives binding identity from ordered content
- [x] S6. The emitted-token/static join classifies every supplied identity as `resolved`,
  `explicitly_omitted`, or `unknown`, and bundle creation/verification requires `unknown=0`
- [ ] S7. Restore a reproducible extraction-atlas generator and remove any dependency on replay-layer
  schema as the source of extractor coverage truth
- [ ] S8. Make save-oracle validation exhaustive and fail-closed; restore all declared outputs and add
  peace-conference, naval-battle, land/air-combat, and special-project fixtures
- [ ] S9. The current paused-D04 process now has an exact admitted seven-artifact deferred bundle
  (`bundle_id=a1604f65ca79ba33a21bf20e1c6eebc0cf3dd65eb3ebf8066a4b1768ec1cc4d9`), bound to
  executable `870ebf…65ad`, zero active mods, 24 active DLCs, and 58,272 source rows. Closure still
  requires the capture-emitted token inventory to finalize with `unknown=0` and the exact finalized
  bundle to survive pre-publication revalidation in that production capture

### X. COVERAGE-SWEEP ITEMS MISSING FROM THE OLD OPEN TABLE

[`W13_RESIDUAL_COMBAT_PROJECT_RE.md`](dossiers/W13_RESIDUAL_COMBAT_PROJECT_RE.md) is the current
evidence/ordering packet for d54, d55, d65, d69, and multi-order membership. It preserves the
current-build do-not-retry list and requires static-bundle/pre-live closure before paused D04
corroboration, followed by separate mutation fixtures.

- [ ] X1a. d54 offline integration — carry cracked `COrderInstance+0x244` identity through d27/d54;
  rename/downgrade `assigned_units_status`, which currently proves only exact owning-army membership,
  not per-order divisions; split omission telemetry and re-adjudicate partial-fold acceptance
- [ ] X1b. d54 root work — direct empty/inactive order groups, activation/execution, stance/
  aggressiveness, and geometry beyond the validated type-5 target/non-type-5 path snapshot
- [ ] X1c. d54 fixture work — after roots/integration, prove stable-identity redraw/resize/delete and
  order-assignment add/remove/tombstone lifecycle
- [ ] X2a. d55 integrate already-cracked battle fields — tactic chain
  `side+0x110 → tactic_id+0x98/token+0x10/name+0x30` and side progress `+0x418`
- [ ] X2b. d55 root work — manpower/equipment losses have positive D04 save rows and need live roots;
  reserve count is captured but reserve-division identities remain open
- [ ] X2c. d55 fixture work — all-zero side `losses` scalar and retreat/overrun/outcome transitions
  need a changing battle fixture
- [ ] X2d. d55 dispositions — terrain is exactly derived; width formula remains unaccepted under the
  no-fudge rule; cracked planning/dig-in offsets belong in the division schema, not d55
- [ ] X3. d43 market — licensing and MTLL relation joins in addition to global owner work B1
- [ ] X4. d11/production — requested factories, deployment queues, and MIO joins
- [ ] X5. d48 construction — exact location and repair semantics in addition to static building identity
- [ ] X6. Static building and equipment-variant registries with deterministic live-id joins
- [x] X7. d69 tech-sharing bonuses and full group semantics — TSG2 v2 pairs validated country members
  with signed fixed-5 bonuses, carries four authority masks, and fails the entire frame on invalid
  logical members; static/runtime ordinal identity remains deliberately unjoined
- [ ] X8. d27 full division-to-multiple-orders membership — current d20 has one primary-order pointer
  and d28 has army membership, but no d27/d54 `(order_instance_id, division_id)` child table. Static
  RE must find the serializer/manager/reverse-index root; paused D04 later requires exact equality to
  all 3,776 save `scheduled_member` pairs over 2,038 divisions
- [ ] X9. d41/d65 projects — obtain controlled start/completion transition fixtures; resolve d65
  owner ambiguity through A23; capture fired reward history/option fields; and join breakthrough
  values to semantic names. d41 already emits started/completed lifecycle rows and breakthrough
  points with fold support
- [ ] X10. Direct decision/event/on-action hooks and mod-added state that hourly polling cannot preserve
- [ ] X11. Audit and explicitly narrow expected-partial scopes for d26, d27, d28, and d54; d54's
  current partials are omission/read/linkage failures, not the fixture-blocked semantics named by its
  tolerance, so sparse-fold identity must be proved or the domain/hour becomes unknown
- [ ] X12. Decide/capture day progress and sub-hour tick/event timing needed for transition fidelity;
  d57 currently emits scheduled current-state anchors, so speed changes between frames remain open
- [x] X13. Host-local presentation state disposition — selected objects, screens/dialogs, map mode,
  camera, hover targets, tooltips, outliner state, drag state, and local UI preferences are
  explicitly out of production extraction and `.battleplan`. The replay viewer creates its own
  presentation state from authoritative captured facts; no host-UI RE or replay-layer edit is owed

## Residuals & known limitations (consolidated — 2026-07-15)

The single roll-up of everything not yet closed. Detail lives where noted; nothing here is a silent
gap.

**USER-GATED — needs a live game and cannot be produced on synthetic fixtures:**
- Combined **speed-5 end-to-end proof** — every enabled production domain, hourly cadence, zero
  skipped hours, no untolerated failures (the standing engineering acceptance gate).
- **Cross-modset validation matrix** — the first non-vanilla captures ever; validates the modset
  resolver + all game-file joins against a real modded session.
- **d57 game-speed mapping** (registry A3) — a 10s tap at each speed 1–5; and **FACS v2 injected
  capture** to exercise the d53 tri-state against a live faction roster.
- **Static-bundle admission** (S9) — resolve the current live mod/DLC/load order, rebuild all seven
  artifacts, require `unknown=0`, and prove the same bundle at pre-injection and pre-publication.
- The **fixture-blocked backlog A1–A23** in the OPEN ITEMS REGISTRY above (each names its exact
  fixture + accepting gate).

**Platform (owner-confirmed 2026-07-14): Windows-only.** The `snapshot/dir_store` Linux/macOS code
is unsupported dead weight (Linux impl is cfg-gated and untested; macOS is a fail-closed stub). It
is **never** a pre-ship residual; a cleanup task to strip/stub it is filed. Do not re-list
"Linux/macOS run" as owed work.

**Merged foundations with remaining acceptance work:**
- **replay-loader:** the `.battleplan` container/loader is Windows-namespace-hardened and the
  **production streaming finalizer** verifies through the retained creation handle (no pathname
  reopen). The old claim that three TOCTOU tests are parked/ignored is stale: the active
  reparse-rebind tests exercise handle-bound verification. Remaining environment gates are Windows
  **Developer Mode** for the symlink-rejection capability test,
  and a real **dehydrated OneDrive** placeholder store for the Files-On-Demand tests. The static "no
  descendant pathname open" gate (F4) is an accepted **best-effort development lint**, not a runtime
  security boundary (cannot see renamed FFI beyond `#[link_name]`, macro-expanded, or generated code).
- **facs-wire:** with no certified `name_hash → display-name` resolver, the UI renders the
  authoritative **numeric** faction identity (cosmetic only — membership content is fully
  preserved). An owed **live-map integration test** lands when a production factions-map render
  consumer is built (none exists in-tree today).
- **autoattach:** the recording-proof residual is explicitly **out-of-threat-model**, not
  fail-closed — an in-process compromise or a truly-simultaneous equal-rights external injection is
  irreducible without an OS-level trust boundary.
- **duplex control:** H4CT framing, deadline/cancellation channels, peer/capability admission,
  heartbeat primitives, the restrictive overlapped Windows pipe adapter, explicit-mode CFA1, and
  write→flush→sync receipt authority are implemented and focused-green. Recorder-main IdentityV1
  file validation and manifest gating are dormant-green; the collector producer barrier is
  unapplied. Activation is false, the active path is Legacy, and production composition, the real
  H4CR backend/durable sink, live cancellation/deadline proof, and rollover remain P2/P3 work.

**Partially built product pillars (`PRODUCT_SESSION_SEMANTICS.md`):** auto-attach, campaign identity,
duplex control, and reload/relaunch stitching have merged offline foundations. Their open work is
production composition and live identity/process-lifecycle proof (P1-P5), not greenfield design.

**Deep-crack + external handoff:** B1-B8 remain open, but not at the same stage: B1/B2 still need
inner-container RE; B4/B6/B7 have static roots/fields and await pre-live closure, paused-D04
acceptance, and integration; B3/B5/B8 retain their separately stated work. B9-B10 were transferred
to A16/A23. C1-C6 are replay-layer handoffs and do not block or authorize extractor-layer work.

**Additional validation surfaces:** two Cargo packages remain outside the workspace
(`project_progress_live_lane` and `lanes/occupation_resistance_compliance_live/replay_verify`), while
`scientists_live_lane` is now a workspace member. All three original F1a surfaces and seven genuinely
dependency-free H4CR parsers have direct certification (F1a/F1c). Maintained production automation
runs all three package surfaces sequentially (F4), so they cannot disappear behind a main-workspace
sweep. `naval_missions_status_live` is Cargo-native; its old
bare-`rustc` README command was stale and must not be counted as an eighth parser.

## Completion Rule

A gameplay concept is not complete just because one compact domain passes. A row is **closed** only
when every replay-relevant field is either (1) **captured** through the production path
(`hoi4.exe → one injected DLL collector → typed compact records → Rust recorder → .battleplan`),
(2) **rejected with proof** (exact candidates tested, not to be retried without new evidence), or
(3) **blocked with proof** (an exact fixture/tooling/root wall documented, no unblocked work remains).
Python/saves/heap-scans support research and validation only — they never close a production row.

**Status legend:** use only the exact registry vocabulary listed in the Authority rule. Completion is
tracked independently: a concept is `captured`, `rejected with proof`, `blocked with proof`, or
`explicitly out of extractor scope`. `CONCEPT_COMPLETE` is a completion judgment, not a registry
domain status.

## Production Gate (required for every captured row)

DLL-only capture in the hot path · compact binary records (not JSON frames) · Rust recorder decodes ·
`.battleplan` verifies · recorder pipe is the sole configured authoritative sink · exact full-write
and hour-latch accounting · strict stream/header/payload/closed-hour certification · admitted and
unchanged seven-artifact static bundle · executable-bound d57 speed gate · hourly-or-better cadence at
verified speed 5 · zero skipped/coalesced game hours · no untolerated
source-unavailable/partial/worker-panic/bridge/control failure flags · proven under the recorder
`--production` profile (not an ad-hoc `--scheduled-domain` subset) · fail-closed on invalid layouts.

**Expected-partial policy:** the gate stays fail-closed. The only tolerated partials are the
domain/flag pairs in `compact_contract::COMPACT_EXPECTED_PARTIAL_TOLERANCES`, absorbing
`COMPACT_STATUS_PARTIAL_DATA` only for the named fixture-blocked sub-scopes — never source-unavailable,
control-invalid, or worker/bridge bits. The recorder manifest surfaces each applied tolerance
(domain, flags, reason, frame count).

## Domain status — registry reconciliation

This table includes every non-legacy/non-infrastructure registry row. Status text is copied exactly
from `domain_registry.json`; field and acceptance gaps are tracked by the item references above.
Domains 10-17, 20-23, and 26-40 remain `existing`; `clock` and `frame` are `infrastructure`.

| Domain | Status | What remains (→ registry ref) |
| --- | --- | --- |
| `country_identity` / 18 | `active_integration` | Built and schedulable but excluded by status from the production profile; promote only after D5-D8 and live proof |
| `weather_live` / 24 | `validated_slice_open_gaps` | Weather drift A20; realized values are C2 handoff |
| `projects_live` / 41 | `validated_partial_slice` | Started/completed rows and breakthrough points captured; controlled transitions, reward history/options, semantic joins X9 |
| `air_combat_losses_live` / 42 | `validated_slice_open_gaps` | Air lifecycle/attribution A4/A21 |
| `market_trade_live` / 43 | `validated_slice_open_gaps` | Market owner B1; licensing/MTLL X3 |
| `supply_logistics_attrition_live` / 44 | `validated_slice_open_gaps` | Hub/pressure/convoy fixtures A1/A6/A7; exact supply/attrition fields |
| `focus_research_decisions_depth_live` / 45 | `validated_slice_open_gaps` | Focus pause A2; decision numeric targets/cooldowns A12 |
| `air_variant_archetypes_live` / 46 | `validated_slice_open_gaps` | Ace variance A5 and remaining archetype labels |
| `characters_leaders_cabinet_live` / 47 | `validated_slice_open_gaps` | Advisor/appointee/portrait/trait revalidation and speed-5 proof |
| `construction_buildings_state_damage_live` / 48 | `validated_slice_open_gaps` | Building join B8; repair/location A14/X5 |
| `naval_combat_lifecycle_live` / 49 | `validated_slice_open_gaps` | Lifecycle-end fixture A9 and speed-5 proof |
| `naval_combat_members_live` / 50 | `validated_slice_open_gaps` | Member-change/sinking fixture A9 and speed-5 proof |
| `scripted_state_flags_variables_live` / 51 | `validated_slice_open_gaps` | Host-local scripted-GUI widget state is out of scope (A13); authoritative direct hooks/mod state remain X10; delayed events are d64 |
| `occupation_resistance_compliance_live` / 52 | `validated_slice_open_gaps` | Garrison equipment B3 and occupation tail reconciliation |
| `diplomacy_intel_operatives_live` / 53 | `validated_slice_open_gaps` | War B5; operations A10; intel A11; autonomy A16 |
| `army_battleplans_depth_live` / 54 | `validated_slice_open_gaps` | Validated target/path snapshot; identity/status/partial semantics X1a, roots X1b, fixtures X1c, membership X8 |
| `land_combat_detail_events_live` / 55 | `validated_slice_open_gaps` | Captured reserve count/committed/units + damage accumulators; exact integration/root/fixture/disposition gaps X2a-X2d |
| `naval_missions_status_live` / 56 | `validated_slice_open_gaps` | Mission fixture A8; anti-air B6 |
| `session_multiplayer_metadata_live` / 57 | `validated_partial_slice` | Speed mapping A3; MP fixture A15 |
| `country_economy_counters_live` / 58 | `reserved` | Non-schedulable; C5 replay handoff, not a live-domain root task |
| `templates_equipment_catalog_live` / 59 | `reserved` | Static/non-schedulable; division-template live join is already in d20; S1-S6 own artifact acceptance |
| `capitulation_war_state_live` / 60 | `validated_slice_open_gaps` | War threat/instigator B5 and lifecycle coverage |
| `air_loss_stats_live` / 61 | `validated_slice_open_gaps` | Read semantics D3; 24-channel expansion/live fixture A22 |
| `hub_motorization_live` / 62 | `validated_slice_open_gaps` | Per-hub level fixture A1 |
| `country_core_scalars_live` / 63 | `validated_slice_open_gaps` | Remaining scalar census and panic containment D5 |
| `delayed_events_live` / 64 | `validated_slice_open_gaps` | Decrement/originator fixture A19 |
| `special_project_programs_live` / 65 | `validated_slice_open_gaps` | Strong owner-tag candidate still needs the 26/26 live gate A23; reward history/options and semantic joins X9 |
| `occupation_state_extras_live` / 66 | `validated_slice_open_gaps` | Remaining occupation extras and field census |
| `country_dynamic_modifiers_live` / 67 | `validated_slice_open_gaps` | Timed modifier value/expiry A18 |
| `session_header_live` / 68 | `validated_slice_open_gaps` | Campaign identity P3 and session-header completeness |
| `tech_sharing_groups_live` / 69 | `validated_slice_open_gaps` | Offline TSG2/static contract is closed; current-session live acceptance remains S9/F6 |

## Gameplay coverage map (14 areas)

Base slices are `PRODUCTION_SLICE_DONE` unless noted; per-field detail is in `EXTRACTION_ATLAS.md`,
open items in the registry above.

| # | Area | Coverage | Open (registry refs) |
| --- | --- | --- | --- |
| 1 | Session / clock / stream / container | DONE (infra accepted) | game-speed value mapping A3; product composition P1-P4 |
| 2 | Static definitions & enrichment | DONE (loc/tech/focus/decisions/ships packed) | GFX/assets (unaudited, UI-owned); character defs (d47) |
| 3 | Country core & economy | DONE base (fuel/stockpile/production/market slice) | market owner B1; factory/resource counters C4/C5; licensing; MTLL |
| 4 | Politics / leaders / cabinet | DONE base | advisors/cabinet/characters (d47 revalidation) |
| 5 | Focus / research / decisions | DONE base | focus-paused A2; decision targets/cooldowns A12; direct hooks |
| 6 | Special projects / scientists | DONE base (+ D6 owner-join/reward cracked→ledger) | project lifecycle events; breakthrough |
| 7 | Divisions / army / OOB / land combat | DONE base (+E5 attribution committed; D20 manpower/losses) | battleplan lifecycle (d54); land-combat detail fields (d55) |
| 8 | Navy | DONE base | naval missions A8; combat lifecycle-end/members A9; convoy losses A7; positioning fix D2 / anti_air B6 |
| 9 | Air | DONE base | sortie/combat lifecycle A4; kill attribution A4/A21; ace death A5; reinforcement reason |
| 10 | Map / weather / construction / occupation | DONE base (+D5 created_date/occupation-law/garrison cracked→ledger) | construction repair A14; garrison equipment B3; weather internals A20/C2; terrain (UI) |
| 11 | Supply / logistics / attrition | DONE base | exact province supply; hub level A1; convoy/disrupted pressure A6; attrition reason |
| 12 | Diplomacy / wars / factions / intel | DONE base + d53 slices (+D4 queues/factions/subjects cracked→ledger) | war lifecycle B5; operations A10; intel-map A11; fix D1 |
| 13 | Scripted state / events / variables | DONE base d51 (+D6 delayed-events cracked→ledger) | authoritative on-action hooks and mod-added state X10; host-local scripted-GUI widget state is out of scope |
| 14 | Player / multiplayer / session | PARTIAL (pause + player rows; D3 session header cracked→ledger) | game-speed A3; hotjoin/tag-switch/chat A15; alerts derived C6 |

## Static artifacts packed by recorder

Deterministic JSON artifacts the production recorder admits only through the verified seven-artifact
bundle (`--static-bundle-manifest` plus mod/DLC/load-order binding documents), then packs as
`role="static_artifact"`. Loose `--static-*` flags are non-production compatibility inputs and cannot
satisfy production admission.

| artifact | bundle role | contents |
| --- | --- | --- |
| `templates_equipment_catalog` | `templates_equipment_catalog` | 306 templates / 1180 unit defs / 59 equipment defs (domain 59) |
| `localisation_join` | `localisation_join` | english loc-key → {name,raw,source} map + `resolved_joins` (focus/state/VP/country/cosmetic id→name); 139,950 keys, mod-over-vanilla |
| `idea_tech_loc_xref_static` | `idea_tech_loc_xref_static` | idea/tech token → loc-key cross-reference (6,522 ideas / 557 techs) |
| `decisions_static_definitions` | `decisions_static_definitions` | 9,525 decisions / 611 categories with costs/targets/cooldowns/visibility |
| `ship_definitions_static` | `ship_definitions_static` | 80 hulls / 116 modules |
| `gfx_icon_asset_manifest_static` | `gfx_icon_asset_manifest_static` | sprite/icon/flag manifest over vanilla+dlc+mod layers with texture hashes |
| `technology_sharing_static` | `technology_sharing_static` | technology-sharing group ids and static member/bonus definitions; runtime intern ordinals remain deliberately unjoined |

## Closure sweep (before calling the extractor done)

Run each, update the registry/coverage map: (1) **save-visible parity** — every save field maps to a
compact/static/event representation or a proof-backed blocker; (2) **observer-UI data sufficiency**
— walk every ledger/map-mode/tooltip/combat/production/diplomacy/intel/navy/air/army/project screen
and prove its factual contents can be populated from captured world data, without capturing the
host's selections, camera, hover, widget, or screen state; (3) **runtime-event sweep** — state
transitions invisible to hourly snapshots that need sub-hour event rows; (4)
**static-definition sweep** — every emitted token/id has a deterministic loc/def/asset join or an
explicit omitted reason; (5) **fixture backlog** — every fixture-blocked item (registry §A) names its
exact fixture + accepting gate; (6) **rejection ledger** — every `REJECTED_NO_RETRY` candidate listed
with offset/root evidence; (7) **combined speed-5 proof** — schedule every enabled production domain,
require hourly coverage, zero skipped hours, no untolerated failures, Rust decode, `.battleplan` verify.

## Done means

The extraction app is done only when every coverage-map area + registry item is `CONCEPT_COMPLETE`
(captured/rejected/blocked with proof) or explicitly out-of-scope with accepted proof, **and** the
final combined speed-5 proof passes with all enabled production domains, zero skipped game hours, no
untolerated failures, Rust decode, and `.battleplan` verification.
