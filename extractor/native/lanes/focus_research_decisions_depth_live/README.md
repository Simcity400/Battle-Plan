# focus_research_decisions_depth_live
> Active lane note: this README is the only active lane-local documentation. Historical plans, proof reports, blocker notes, and registration packets for this lane are archived under ../../../documents/archive/native_lanes/focus_research_decisions_depth_live/. Current extraction completion status lives in ../../../documents/MASTER_EXTRACTION_CONTRACT.md.


Lane-local production stack for focus/research/decision depth records beyond the
already-proven base compact `focus_live`, `tech_live`, and `decisions_live`
domains.

This crate does not edit shared collector or recorder entrypoints. It consumes
the production compact H4CR stream from the DLL/Rust recorder, derives depth
transition rows in Rust, emits a lane-local compact stream, and gates that
stream with fail-closed decoder checks.

## Scope

- Focus transition markers from compact `focus_live` deltas:
  current/continuous start, stop, switch, completed-count increase, inferred
  completion token when the previous current focus is extractable, and progress
  regression/anomaly markers.
- Research slot speed/ETA rows from compact `tech_live` deltas:
  same-tag/same-slot/same-tech live progress deltas, observed raw speed per game
  hour, base required points, ETA hours, and rollover/regression markers.
- Decision transition rows from compact `decisions_live` deltas:
  active timed-decision start/stop/day-delta/state-change rows plus per-country
  status-vector counts for active timed, targeted, targeted timed, re-enable,
  alternate re-enable, and live timed rows.
- Fail-closed badge rows for fields absent from this historical derivation
  path's base domain-12/domain-16 inputs: focus pause, detailed target bodies,
  and direct engine event hooks. The integrated production collector does not
  use this lossy route for pause or decision bodies; it emits those exact
  fields directly into D45.

## Production Domain

The integrated production compact payload uses domain id `45` and domain name
`focus_research_decisions_depth_live`. Domain `44` is reserved for
`supply_logistics_attrition_live` and must not be reused by this lane.

## Proof

From the repo root:

```powershell
cargo test --manifest-path extractor\native\lanes\focus_research_decisions_depth_live\Cargo.toml

cargo run --manifest-path extractor\native\lanes\focus_research_decisions_depth_live\Cargo.toml --bin proof_from_stream -- `
  --input extractor\native\runs\compact_country_870ebf_speed5_20260701_v2\collector_stream.bin `
  --out-dir extractor\native\runs\focus_research_decisions_depth_live_20260701_v2_badges

cargo run --manifest-path extractor\native\lanes\focus_research_decisions_depth_live\Cargo.toml --bin gate_depth_stream -- `
  --input extractor\native\runs\focus_research_decisions_depth_live_integrated_870ebf_20260701_v1\collector_stream.bin `
  --out-dir extractor\native\lanes\focus_research_decisions_depth_live\proofs\integrated_depth_gate_20260701_v1 `
  --game-hours 48
```

The proof command writes:

- `depth_stream.bin`: derived H4CR compact stream containing one lane-local data
  record per source frame.
- `proof_manifest.json`: source frame/domain counts, skipped-hour gate, derived
  row counters including `badge_rows` and `badge_rows_by_kind`, and
  parse/gate errors.
- `sample_events.jsonl`: first derived rows for manual inspection.

The direct depth gate over the integrated domain `45` stream writes
`depth_gate_manifest.json`. Certification requires canonical H4CR flags and
sequence from `1`, consecutive open/closed hours, exactly one domain-45
`DOMAIN_DATA`/`DOMAIN_PERF` pair in every frame, known non-fatal PERF status,
and a 32-byte `FRAME_END` whose emitted count equals the independently observed
PERF roster, whose scheduled count matches emitted, whose reserved word is zero,
and whose status is known and non-fatal. Physical EOF must immediately follow
the final certified `FRAME_END`. Production DATA must carry the schema-v2 record
flag and the `FRD2` schema-v3/60-byte header. Schema v3 preserves the v2 field
offsets, adds authoritative raw compact DATA status at byte `56`, and begins rows
at byte `60`; byte `8` remains lane-local depth status. Legacy lane-local v1 and
statusless FRD2 v2 payloads cannot certify.
The explicit `--game-hours N` argument is authoritative and requires exactly
`N + 1` certified closed frames; under-length and over-length streams both fail,
and no manifest-derived duration fallback exists.

The shared FRD2 schema-v3 DATA decoder now exposes exact compact status authority,
including clean `Some(0)`, so the gate certifies only when DATA and PERF status
match exactly. Shared readers retain v1/v2 structural compatibility, but missing
status is never synthesized from PERF, `FRAME_END`, or the unrelated depth-status
word at byte `8`. The older named proof inputs and gate manifests are absent from
this checkout and their historical counts must not be treated as current
certification evidence.

The exact-completed production slice proof is gated separately:

```powershell
cargo run --manifest-path extractor\native\lanes\focus_research_decisions_depth_live\Cargo.toml --bin gate_depth_stream -- `
  --input extractor\native\runs\focus_research_decisions_depth_live_exact_completed_20260701_v1\collector_stream.bin `
  --out-dir extractor\native\lanes\focus_research_decisions_depth_live\proofs\exact_completed_gate_20260701_v2 `
  --game-hours 48

extractor\native\target\release\pack_replay.exe verify `
  extractor\native\runs\focus_research_decisions_depth_live_exact_completed_20260701_v1\native_session.battleplan
```

Historical notes reported `144` depth records, `1` exact completed-focus row,
`0` inferred rows, `0` unresolved rows, and `432` badge rows for that run. The
input and gate artifacts are absent from this checkout, so those counts are not
current certification evidence; the hardened gate must reject that old result
until D45 DATA status authority is added and a fresh exact-duration proof passes.
The shared collector hunk once proposed for that behavior is archived in
`../../../documents/archive/native_lanes/focus_research_decisions_depth_live/SHARED_RESIDUAL_PATCH_PLAN.md`.

Current-build root validation for fail-closed depth fields:

```powershell
python extractor\native\lanes\focus_research_decisions_depth_live\tools\probe_current_roots.py `
  --out-dir extractor\native\runs\focus_research_decisions_depth_live_current_roots_20260701_v3

python extractor\native\lanes\focus_research_decisions_depth_live\tools\probe_current_roots.py `
  --out-dir extractor\native\lanes\focus_research_decisions_depth_live\proofs\current_roots_20260701_v5 `
  --sample-limit-per-list 32

python extractor\native\lanes\focus_research_decisions_depth_live\tools\probe_current_roots.py `
  --out-dir extractor\native\lanes\focus_research_decisions_depth_live\proofs\current_roots_20260701_v6_full_decisions `
  --sample-limit-per-list 10000

python extractor\native\lanes\focus_research_decisions_depth_live\tools\analyze_decision_body_candidates.py `
  --probe-dir extractor\native\lanes\focus_research_decisions_depth_live\proofs\current_roots_20260701_v6_full_decisions `
  --out-dir extractor\native\lanes\focus_research_decisions_depth_live\proofs\decision_body_candidate_gate_20260701_v3_full_decisions

python extractor\native\lanes\focus_research_decisions_depth_live\tools\analyze_decision_variant_candidates.py `
  --probe-dir extractor\native\lanes\focus_research_decisions_depth_live\proofs\current_roots_20260701_v6_full_decisions `
  --with-static-defs `
  --out-dir extractor\native\runs\focus_research_decisions_depth_live_decision_variant_gate_20260701_v3_full_decisions

python extractor\native\lanes\focus_research_decisions_depth_live\tools\probe_decision_target_save_oracle.py `
  --probe-dir extractor\native\lanes\focus_research_decisions_depth_live\proofs\current_roots_20260701_v6_full_decisions `
  --save "C:\Users\Example\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV\save games\autosave.hoi4" `
  --out-dir extractor\native\lanes\focus_research_decisions_depth_live\proofs\decision_target_save_oracle_gate_20260701_v1

python extractor\native\lanes\focus_research_decisions_depth_live\tools\probe_decision_numeric_target_save_oracle.py `
  --probe-dir extractor\native\lanes\focus_research_decisions_depth_live\proofs\current_roots_20260701_v6_full_decisions `
  --save "C:\Users\Example\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV\save games\autosave.hoi4" `
  --out-dir extractor\native\lanes\focus_research_decisions_depth_live\proofs\decision_numeric_target_save_oracle_gate_20260701_v1

python extractor\native\lanes\focus_research_decisions_depth_live\tools\probe_focus_pause_candidates.py `
  --out-dir extractor\native\lanes\focus_research_decisions_depth_live\proofs\focus_pause_candidate_gate_20260701_v2

python extractor\native\lanes\focus_research_decisions_depth_live\tools\probe_focus_pause_wide_candidates.py `
  --out-dir extractor\native\lanes\focus_research_decisions_depth_live\proofs\focus_pause_wide_candidate_gate_20260701_v2 `
  --start-offset 0x0 `
  --end-offset 0x80

python extractor\native\lanes\focus_research_decisions_depth_live\tools\probe_focus_pause_static_gap_candidates.py `
  --wide-gate extractor\native\lanes\focus_research_decisions_depth_live\proofs\focus_pause_wide_candidate_gate_20260701_v2\focus_pause_wide_candidate_gate.json `
  --out-dir extractor\native\lanes\focus_research_decisions_depth_live\proofs\focus_pause_static_gap_gate_20260701_v2

python extractor\native\lanes\focus_research_decisions_depth_live\tools\watch_focus_pause_fixture.py `
  --focus-gate extractor\native\lanes\focus_research_decisions_depth_live\proofs\focus_pause_candidate_gate_20260701_v2\focus_pause_candidate_gate.json `
  --duration-s 12 `
  --sample-ms 500 `
  --min-game-hours 2 `
  --out-dir extractor\native\lanes\focus_research_decisions_depth_live\proofs\focus_pause_fixture_watch_20260701_v1

python extractor\native\lanes\focus_research_decisions_depth_live\tools\probe_event_hook_candidates.py `
  --out-dir extractor\native\lanes\focus_research_decisions_depth_live\proofs\event_hook_candidate_gate_20260701_v1

python extractor\native\lanes\focus_research_decisions_depth_live\tools\probe_event_hook_static_contracts.py `
  --event-gate extractor\native\lanes\focus_research_decisions_depth_live\proofs\event_hook_candidate_gate_20260701_v1\event_hook_candidate_gate.json `
  --out-dir extractor\native\lanes\focus_research_decisions_depth_live\proofs\event_hook_static_contract_gate_20260701_v1

python extractor\native\lanes\focus_research_decisions_depth_live\tools\analyze_residual_blockers.py `
  --probe-dir extractor\native\lanes\focus_research_decisions_depth_live\proofs\current_roots_20260701_v6_full_decisions `
  --depth-gate extractor\native\lanes\focus_research_decisions_depth_live\proofs\integrated_depth_gate_20260701_v1\depth_gate_manifest.json `
  --integrated-manifest extractor\native\runs\focus_research_decisions_depth_live_integrated_870ebf_20260701_v1\native_session_manifest.json `
  --focus-pause-gate extractor\native\lanes\focus_research_decisions_depth_live\proofs\focus_pause_candidate_gate_20260701_v2\focus_pause_candidate_gate.json `
  --focus-pause-watch extractor\native\lanes\focus_research_decisions_depth_live\proofs\focus_pause_fixture_watch_20260701_v1\focus_pause_fixture_watch.json `
  --focus-pause-wide-gate extractor\native\lanes\focus_research_decisions_depth_live\proofs\focus_pause_wide_candidate_gate_20260701_v2\focus_pause_wide_candidate_gate.json `
  --focus-pause-static-gap-gate extractor\native\lanes\focus_research_decisions_depth_live\proofs\focus_pause_static_gap_gate_20260701_v2\focus_pause_static_gap_gate.json `
  --event-hook-static-contract-gate extractor\native\lanes\focus_research_decisions_depth_live\proofs\event_hook_static_contract_gate_20260701_v1\event_hook_static_contract_gate.json `
  --completed-depth-gate extractor\native\lanes\focus_research_decisions_depth_live\proofs\exact_completed_gate_20260701_v2\depth_gate_manifest.json `
  --completed-integrated-manifest extractor\native\runs\focus_research_decisions_depth_live_exact_completed_20260701_v1\native_session_manifest.json `
  --decision-body-gate extractor\native\lanes\focus_research_decisions_depth_live\proofs\decision_body_candidate_gate_20260701_v3_full_decisions\decision_body_candidate_gate.json `
  --decision-variant-gate extractor\native\runs\focus_research_decisions_depth_live_decision_variant_gate_20260701_v3_full_decisions\decision_variant_candidate_gate.json `
  --decision-target-save-oracle-gate extractor\native\lanes\focus_research_decisions_depth_live\proofs\decision_target_save_oracle_gate_20260701_v1\decision_target_save_oracle_gate.json `
  --decision-numeric-target-save-oracle-gate extractor\native\lanes\focus_research_decisions_depth_live\proofs\decision_numeric_target_save_oracle_gate_20260701_v1\decision_numeric_target_save_oracle_gate.json `
  --event-hook-gate extractor\native\lanes\focus_research_decisions_depth_live\proofs\event_hook_candidate_gate_20260701_v1\event_hook_candidate_gate.json `
  --decision-target-compact-schema-proved `
  --integrated-replay-verified `
  --completed-replay-verified `
  --out-dir extractor\native\lanes\focus_research_decisions_depth_live\proofs\residual_blocker_gate_20260701_v10
```

See `../../../documents/archive/native_lanes/focus_research_decisions_depth_live/CURRENT_BUILD_ROOTS.md` for the validated/rejected roots,
`../../../documents/archive/native_lanes/focus_research_decisions_depth_live/INTEGRATION_PATCH_PLAN.md` for the domain-45 integration shape, and
`../../../documents/archive/native_lanes/focus_research_decisions_depth_live/SHARED_RESIDUAL_PATCH_PLAN.md` for residual-specific shared hunks that require
an orchestrator slot.

## 2026-07-17 A2/A12 promotion update

`A2_A12_RE_REPORT.md` and `A2_A12_RE_EVIDENCE.json` supersede the residual
claims below for focus pause, decision cooldowns, and target storage:

- focus pause is `CCountry+0x1380 -> CNationalFocusProgress+0xB0`;
- decision re-enable is `CCountry+0xFA0 -> CDecisionStatus+0x58`, with
  `CDecisionCooldown { definition:+0x08, days:+0x10 }`;
- decision removal uses the same record at `CDecisionStatus+0x70`;
- the targeted-decision union is `record+0x20` string/country or
  `record+0x24` numeric, while `record+0x28` is lifecycle state rather than a
  target-kind discriminator;
- target scope is joined from the static decision definition. A present,
  non-false `state_target` declaration makes the numeric value a state ID.
  The pinned game-file corpus has 278 such declarations and no
  `province_target` producer.

## 2026-07-19 residual cleanup

The promoted targeted-decision body also includes exact lifecycle
`record+0x28`, ignore `record+0x2C`, and days `record+0x30`. The serializer and
deserializer evidence for all three fields is recorded in
`A2_A12_RE_REPORT.md` and `A2_A12_RE_EVIDENCE.json`. Integrated production D45
sets the exact pause/targets/cooldowns status bits and emits only the remaining
direct-hook-unavailable badge. This lane-local proof derivation retains its
pause/target badges because its older base compact inputs do not carry those
new exact fields; it must not be mistaken for the production producer path.

## 2026-07-20 A31 national-focus presentation acquisition

Reserved compact domain 89 has a complete pointer-free NFP1 codec, collector
owner, recorder fold, typed replay view, and exact XUI10 admission route.
`A31_FRAME_ORDER_AND_FIXTURE_MATRIX_20260720.md` is the current acquisition
authority: exact frame callers update item state through `0x14137A780` before
the national/joint detail refresh at `0x14137A600`, so the evidence owner now
materializes only after that same-view pair. Use
`tools/verify_a31_acquisition_receipt.py` for each receipt and
`tools/verify_a31_fixture_matrix.py` for the stable/transition pair.
The final display and joint-country leaves are installed by the owner itself;
run the complete interactive pair through
`../../scripts/Invoke-A31NationalFocusFixturePair.ps1`.

No retained authenticated pair exists. Domain 89 remains reserved and
unscheduled; neither live-fixture nor production activation is enabled.

The direct-hook candidate census additionally rejects the remaining named
sources as effect validation, metrics formatting, static registration, or GUI
consumption. No callback RVA is guessed or promoted. A future direct-hook
promotion still requires an engine mutator/callback argument contract and
authenticated same-frame invocation observations.

The follow-up source-file/state-writer pass is recorded in
`DIRECT_EVENT_HOOK_RE_20260720.md` and `DIRECT_EVENT_HOOK_RE_20260720.json`.
It identifies static current-build seeds for focus completion, research
threshold crossing, ordinary/targeted decision taking, and timed-decision
lifecycle updates. They remain inactive because the retained corpus has no
authenticated entry/argument/same-hour-transition join; compact-frame
derivation and the direct-hook-unavailable badge remain production truth.

The following residual narrative is retained as historical evidence of the
search path; it is no longer the current A2/A12 RE status.

Current residual status is `NOT COMPLETE`: direct completed-focus token closure
is a validated slice when compact focus source rows carry completed vectors.
Focus pause has the full `CFocusStatus+0x00..+0x7F` bounded object range
rejected as the paused-boolean home. The original `+0x20..+0x37` window rejects
`24` byte candidates; the wider live v2 scan rejects the other `100`
outside-window candidates except for the `+0x3C..+0x3F` gap; and the static v2
gap gate rejects those final `4` bytes by CFocusStatus vtable-method qword
access evidence at `+0x38`. The bounded watcher still records the required
paused=true oracle contract for any future root outside `CFocusStatus`.
Decision body extraction is partially unblocked: targeted-timed country-target
and numeric-target rows now have validated target fields, while cooldown/day/
state details, numeric state/province semantic classification, unmatched
transition rows, and direct engine event hooks remain blocked or badged.

The decision target save-oracle gate validates a real target field for part of
the targeted-timed body work: for vtable `0x27D5038`, `record+0x20` is the
country target `tag_db_id` for the exact save/live matched country-target
subset. The proof compared the full `3384` live targeted-timed rows against the
current autosave targeted-decision rows, validating `1061` live country-target
rows across `180` decision tokens. It leaves `33` live country-target transition
rows plus cooldown/day/state body fields blocked. The numeric target
save-oracle gate validates `record+0x24` as the numeric target id for `2285`
exact save/live matched `target=<number>` rows; it leaves `4` live-only and `3`
save-only numeric transition rows plus semantic classification of the numeric
ids as state/province/body ids blocked. The full v6/v3 variant refresh sampled `3384` targeted-timed records
across `262` decision tokens and rejected `49` definition-local country-tag
slices.

The lane-local compact schema/decoder/gate now has a production-shaped target
row for those validated subsets. The depth payload uses the reserved header
prefix for `decision_target_count` and emits fixed target rows
`owner_tag_db_id, target_id, flags, source_kind, decision`. Rust tests prove
derivation from an extended compact `decisions_live` fixture, acceptance of the
country/`record+0x20` and numeric/`record+0x24` flags, and rejection of invalid
target metadata.
This is still not shared-production shipped until the orchestrator applies the
collector/recorder integration slot.

The event-hook candidate gate rejects the named decision-event string route and
the named focus-completion callback string route for this build. The follow-up
static-contract gate rejects all `5` remaining static text-xref anchors as
registration/catalog arguments, not runtime callbacks. Each anchor is inside
function `0x82CB0..0x11896F`, which has `10697` string references; the local
contract loads the anchor string into `r8`, loads a registration key into `edx`,
sets a global registry pointer in `rcx`, and calls `0x24AFC10`. No remaining
string/xref route has a runtime focus/research/decision object argument
contract, so direct hooks remain blocked.

## Integration Plan

The exact shared patch plan is in `../../../documents/archive/native_lanes/focus_research_decisions_depth_live/INTEGRATION_PATCH_PLAN.md`. Keep this lane
isolated until the orchestrator grants a shared integration slot.
