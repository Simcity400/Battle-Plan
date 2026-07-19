# Remaining work to production

**Updated: 2026-07-19.** The single deduped backlog. Detail lives in the
per-half ledgers (linked); this list is complete at the item level — if it is
not here (or in `SEAM_CONTRACTS.md`), it is not open. Check items off here;
update ledgers as usual.

## 1. Extractor

Detail: `extractor/documents/MASTER_EXTRACTION_CONTRACT.md` (§A/B/D/E/F/P/S/X)
and `extractor/documents/RE_REMAINING.md`.

- **Live/fixture-blocked extraction (§A):** A3 speed-cycle d57, A5 ace events,
  A6 disrupted supply, A7 convoy-combat loss, A8 naval missions, A9 naval
  combat lifecycle-end, A10 running operations, A11 spy-network/intel-map,
  A13 authoritative scripted-GUI state, A16 subject autonomy, A17 nukes,
  A18 dynamic modifiers, A19 delayed events, A20 weather drift, A23 CProgram
  owner-tag gate.
- **Deep cracks (§B):** B1 market stockpile iterator, B2 (contradiction —
  master says promotion-ready, RE_REMAINING says DEEP RE REQUIRED; reconcile,
  see §5), B3 garrison equipment (partly superseded by A35), B4 sunk-convoys
  paused acceptance, B5 war threat/instigator, B6 naval anti-air equality,
  B7 raids paused acceptance.
- **Committed-code fixes (§D):** D11 sink-intent observability; D12 d55
  zero-silent-gap/fold correctness — extractor half OPEN (preserve ended
  combats after d21 removal, unreadable/torn-vs-genuinely-empty side detail,
  per-side completeness, end/rebaseline fold gates) plus replay half =
  X-ENDEDCOMBAT (see seam).
- **Save-family dispositions (§E):** E12–E29 (18 open).
- **Build/ops (§F):** F1b one failing `replay_decoder` fixture test (blocks
  S4), F3 Windows capability gates, F5 truthful acceptance telemetry, F6 full
  acceptance ladder rungs 1–6.
- **Product pillars (§P):** P1 auto-attach product closure (packaging, tray,
  graceful stop, heartbeat), P2 duplex activation (collector H4CT/CID1
  client, durable sink/stdout replacement, live peer/deadline/cancellation/
  liveness proof, rollover — then flip
  `AUTO_ATTACH_DUPLEX_ACTIVATION_ENABLED`), P3 campaign-identity full matrix
  (collector sequence-0/CFA1 barrier unapplied), P4 stitching production call
  site, P5 dormant stitch-input hardening.
- **Static/oracle (§S):** S4 (blocked on F1b), S7 atlas generator
  independence residual, S8 exhaustive save-oracle fixtures (peace-conference,
  naval-battle, land/air-combat, special-project, restored outputs), S9 live
  static-bundle admission (never produced live).
- **Coverage sweeps (§X):** X1a–c, X2a–d, X3, X4 deployment queues, X8 order
  membership, X9 projects, X10 direct hooks, X11 expected-partial narrowing,
  X12 sub-hour timing.
- **OWED-PRODUCER tooling** (prerequisite for acceptance gates; register in
  `extractor/documents/LIVE_ACCEPTANCE_RUNBOOK.md`): fault harness, pause
  prover, ladder wiring,
  affinity application, idle-cost harness, D57 emission-comparison harness,
  CID1 collector barrier, P4 stitch call site, P1 packaging/tray UI.
- **2026-07-14 session-close residue** (from the archived
  `extractor/documents/archive/CODEX_DISPATCH_QUEUE.md`; never dispositioned):
  dlcmeta r9 REVISE,
  foldwave1 REVISE, d57-gate r2 REVISE, c1stageb r7 REVISE, stageb-impl gate,
  boundary-auth + stitching re-gates.

## 2. Replay layer

Detail: `replay_interface/documents/lane_reports/rendering_laws/PROGRAM_STATUS.md`.

- **XUI27 final UI composition** — pixel parity for every panel family,
  modal/tooltip combination, map-under-UI state, localization variant, and
  unavailable-data case. QUEUED, never run. The last replay-side promotion
  gate.
- **Adversarial release-review wave** — by program policy ALL adversarial
  review was deferred until after freeze; it has never run.
- **Reissue the finalization receipt** with stable SHAs after the fix wave
  (current receipt drifted same-day; not a freeze authority).
- **G1/C7 close-zoom wiring:** the 7 close-zoom law modules are lib.rs-only —
  wire into `ProductionMapPassGraph`/`d3d11_replay`; rerun the full-ladder
  RenderDoc parity (≤0.03 at all heights incl. h=50).
- **Robustness fix wave (in flight 2026-07-19, Fable workflow):** F4
  SHA-mismatch fail-closed hole; blocker-registry coverage beyond main.rs +
  snow/mud env-override registration; receipt_sha256 into a golden gate; D3D
  device-removed handling; container open cost (O(entries²) manifest loop,
  full decompress+CRC at open, whole-file RAM residency, O(hours) frame
  scan); container-spec §4.3 NFC path-collision rejection;
  `EntryRec.verified` wiring. Status will be updated here when the wave
  lands.
- **Capture-gated residuals:** M02/M03 human-capture cards (the two
  HUMAN_CAPTURE_REQUIRED rows on the board); recapture of the LOST
  `restore_gate_1` to re-validate grading/bloom/political-v2 laws. (The
  event-compose external gates and O04 city draw are CLOSED — XN16/O04
  PROVEN on the board. The archived plan's CAP1–CAP11 asks must be verified
  against the board before scheduling any; several closed via the XN/XUI
  program — see §5.)
- **G3 fidelity recount** — the archived FIDELITY_LEDGER totals were
  self-flagged "PENDING FRESH RECOUNT" and were never recounted; fold a fresh
  recount into PROGRAM_STATUS (or record the recount here) before release.
- **Phase H** — swap a real recording into the replay path and rerun the
  G-gates end-to-end (explicitly deferred until extractor completion).

## 3. Seam (both directions)

See `SEAM_CONTRACTS.md`: ~15 extractor-side input contracts blocking XUI
packets (A24–A40 namespace), 2 replay-side asks (X-DECODER adoption,
X-ENDEDCOMBAT incl. the confirmed `_merge_two_snapshots` union defect), and
the legacy-row dedup pass that nobody has run.

## 4. Owner-run / owner decisions

- **Live acceptance sequences 1–5**
  (`extractor/documents/LIVE_ACCEPTANCE_RUNBOOK.md`): ~36 the game launches + 6
  idle windows; sequence 5 needs a second machine + second player. Runbook
  must first be reconciled with the 07-19 promotions (§5 below).
- **Six open runbook questions** (runbook "Open questions", need owner
  answers before their gates run): D04 save availability, affinity target,
  player input during graded rungs, naval-fixture steering, D57 harness
  shape, fixture-variance verifier task.
- **X-DECODER architecture decision:** adopting
  `extractor/native/replay_decoder` in `replay_core` conflicts with
  `REPLAY_ARCHITECTURE.md` (replay_core must carry no extractor-library
  dependency). Decide: move/neutralize the decoder (e.g. under
  `portable_replay/`) or amend the architecture. Blocks stream-only
  containers (~72% size cut).
- **F1 performance contract:** name the reference weak-PC/GPU class and
  frame-time budget; then the headless bench gets built.
- **Replay-loader TOCTOU decision** (user-gated; two non-production
  finalizers).
- **Rescue branches** (local-only, from
  `archive/GIT_CONSOLIDATION_2026-07-19.md`): `rescue/weather-leaf-soundness`
  (run its 3 tests vs master — master may have REGRESSED the open-vocab
  weather-leaf masking fix), `rescue/acceptance-telemetry-f5` (port or
  descope), `rescue/replay-layer-checkpoint` (parity review, then drop).
- **Trigger the adversarial release-review wave** once XUI27 + fix wave are
  in.

## 5. Documentation debt (doc says X, reality is Y)

- MASTER_EXTRACTION_CONTRACT: reconcile the domain table with the registry
  (doc claims 31 rows = "all"; `domain_registry.json` has 71 entries incl.
  d70–d80); fix stale header date.
- B2 contradiction: master §B says promotion-ready, `RE_REMAINING.md` says
  DEEP RE REQUIRED — one of them is stale; reconcile with evidence.
- A27 vs board: extractor marks A27 delivered; the board still lists XUI13
  DATA_BLOCKED on A27 — either replay-side consumption or a board refresh is
  owed.
- CAP1–CAP11 (archived REPLAY_PRODUCTION_PLAN §4): verify which capture asks
  remain open against the board; record survivors in §2, drop the rest.
- LIVE_ACCEPTANCE_RUNBOOK: OWED register still lists the 8 promoted producers
  (A1/A2/A4/A12/A14/A15/A21/A22) as missing; refresh against the 07-19
  promotions.
- RE_REMAINING A26 says d72 unschedulable; registry has d72
  `validated_slice, schedulable=true` — reconcile.
- AUTO_ATTACH_DESIGN header claims "no product code exists yet" — false;
  update status line.
- HOI4REPLAY_CONTAINER_SPEC: add a spec-vs-code delta note (or close the
  deltas via the fix wave and note conformance).
- Legacy seam dedup pass (SEAM_CONTRACTS §3).
