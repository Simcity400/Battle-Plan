# Current-Build Root Probe

Lane: `focus_research_decisions_depth_live`.

This file records the current 870EBF live probe for depth fields that were still
fail-closed after the lane-local compact derivation stack. The probe is
read-only Python validation, not hot-loop capture.

## Proof Command

```powershell
python -m py_compile replay_native\lanes\focus_research_decisions_depth_live\tools\probe_current_roots.py

python replay_native\lanes\focus_research_decisions_depth_live\tools\probe_current_roots.py `
  --out-dir replay_native\runs\focus_research_decisions_depth_live_current_roots_20260701_v3
```

Proof files:

- `replay_native/runs/focus_research_decisions_depth_live_current_roots_20260701_v3/root_probe_manifest.json`
- `replay_native/runs/focus_research_decisions_depth_live_current_roots_20260701_v3/focus_pause_samples.jsonl`
- `replay_native/runs/focus_research_decisions_depth_live_current_roots_20260701_v3/decision_body_samples.jsonl`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/current_roots_20260701_v4/root_probe_manifest.json`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/current_roots_20260701_v4/focus_pause_samples.jsonl`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/current_roots_20260701_v4/decision_body_samples.jsonl`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/current_roots_20260701_v5/root_probe_manifest.json`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/current_roots_20260701_v5/focus_pause_samples.jsonl`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/current_roots_20260701_v5/decision_body_samples.jsonl`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/current_roots_20260701_v6_full_decisions/root_probe_manifest.json`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/current_roots_20260701_v6_full_decisions/focus_pause_samples.jsonl`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/current_roots_20260701_v6_full_decisions/decision_body_samples.jsonl`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/decision_body_candidate_gate_20260701_v1/decision_body_candidate_gate.json`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/decision_body_candidate_gate_20260701_v1/DECISION_BODY_CANDIDATE_GATE.md`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/decision_body_candidate_gate_20260701_v2/decision_body_candidate_gate.json`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/decision_body_candidate_gate_20260701_v2/DECISION_BODY_CANDIDATE_GATE.md`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/decision_body_candidate_gate_20260701_v3_full_decisions/decision_body_candidate_gate.json`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/decision_body_candidate_gate_20260701_v3_full_decisions/DECISION_BODY_CANDIDATE_GATE.md`
- `replay_native/runs/focus_research_decisions_depth_live_decision_variant_gate_20260701_v1/decision_variant_candidate_gate.json`
- `replay_native/runs/focus_research_decisions_depth_live_decision_variant_gate_20260701_v1/DECISION_VARIANT_CANDIDATE_GATE.md`
- `replay_native/runs/focus_research_decisions_depth_live_decision_variant_gate_20260701_v2/decision_variant_candidate_gate.json`
- `replay_native/runs/focus_research_decisions_depth_live_decision_variant_gate_20260701_v2/DECISION_VARIANT_CANDIDATE_GATE.md`
- `replay_native/runs/focus_research_decisions_depth_live_decision_variant_gate_20260701_v3_full_decisions/decision_variant_candidate_gate.json`
- `replay_native/runs/focus_research_decisions_depth_live_decision_variant_gate_20260701_v3_full_decisions/DECISION_VARIANT_CANDIDATE_GATE.md`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/decision_target_save_oracle_gate_20260701_v1/decision_target_save_oracle_gate.json`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/decision_target_save_oracle_gate_20260701_v1/DECISION_TARGET_SAVE_ORACLE_GATE.md`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/decision_numeric_target_save_oracle_gate_20260701_v1/decision_numeric_target_save_oracle_gate.json`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/decision_numeric_target_save_oracle_gate_20260701_v1/DECISION_NUMERIC_TARGET_SAVE_ORACLE_GATE.md`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/focus_pause_candidate_gate_20260701_v1/focus_pause_candidate_gate.json`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/focus_pause_candidate_gate_20260701_v1/FOCUS_PAUSE_CANDIDATE_GATE.md`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/focus_pause_candidate_gate_20260701_v2/focus_pause_candidate_gate.json`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/focus_pause_candidate_gate_20260701_v2/FOCUS_PAUSE_CANDIDATE_GATE.md`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/focus_pause_wide_candidate_gate_20260701_v1/focus_pause_wide_candidate_gate.json`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/focus_pause_wide_candidate_gate_20260701_v1/FOCUS_PAUSE_WIDE_CANDIDATE_GATE.md`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/focus_pause_wide_candidate_gate_20260701_v2/focus_pause_wide_candidate_gate.json`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/focus_pause_wide_candidate_gate_20260701_v2/FOCUS_PAUSE_WIDE_CANDIDATE_GATE.md`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/focus_pause_static_gap_gate_20260701_v2/focus_pause_static_gap_gate.json`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/focus_pause_static_gap_gate_20260701_v2/FOCUS_PAUSE_STATIC_GAP_GATE.md`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/focus_pause_fixture_watch_20260701_v1/focus_pause_fixture_watch.json`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/focus_pause_fixture_watch_20260701_v1/FOCUS_PAUSE_FIXTURE_WATCH.md`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/event_hook_candidate_gate_20260701_v1/event_hook_candidate_gate.json`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/event_hook_candidate_gate_20260701_v1/EVENT_HOOK_CANDIDATE_GATE.md`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/event_hook_static_contract_gate_20260701_v1/event_hook_static_contract_gate.json`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/event_hook_static_contract_gate_20260701_v1/EVENT_HOOK_STATIC_CONTRACT_GATE.md`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/integrated_depth_gate_20260701_v1/depth_gate_manifest.json`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/exact_completed_gate_20260701_v2/depth_gate_manifest.json`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/residual_blocker_gate_20260701_v1/residual_blocker_gate.json`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/residual_blocker_gate_20260701_v1/RESIDUAL_BLOCKER_GATE.md`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/residual_blocker_gate_20260701_v2/residual_blocker_gate.json`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/residual_blocker_gate_20260701_v2/RESIDUAL_BLOCKER_GATE.md`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/residual_blocker_gate_20260701_v3/residual_blocker_gate.json`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/residual_blocker_gate_20260701_v3/RESIDUAL_BLOCKER_GATE.md`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/residual_blocker_gate_20260701_v4/residual_blocker_gate.json`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/residual_blocker_gate_20260701_v4/RESIDUAL_BLOCKER_GATE.md`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/residual_blocker_gate_20260701_v5/residual_blocker_gate.json`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/residual_blocker_gate_20260701_v5/RESIDUAL_BLOCKER_GATE.md`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/residual_blocker_gate_20260701_v6/residual_blocker_gate.json`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/residual_blocker_gate_20260701_v6/RESIDUAL_BLOCKER_GATE.md`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/residual_blocker_gate_20260701_v7/residual_blocker_gate.json`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/residual_blocker_gate_20260701_v7/RESIDUAL_BLOCKER_GATE.md`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/residual_blocker_gate_20260701_v8/residual_blocker_gate.json`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/residual_blocker_gate_20260701_v8/RESIDUAL_BLOCKER_GATE.md`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/residual_blocker_gate_20260701_v9/residual_blocker_gate.json`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/residual_blocker_gate_20260701_v9/RESIDUAL_BLOCKER_GATE.md`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/residual_blocker_gate_20260701_v10/residual_blocker_gate.json`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/residual_blocker_gate_20260701_v10/RESIDUAL_BLOCKER_GATE.md`

## Build And Base Roots

Validated current build:

- `hoi4.exe` PID `45880`
- SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`
- module base `0x7FF6AAFF0000`

Validated roots:

- `module+0x3316E90 -> CGameState = 0x22BA5F48AD0`
- `module+0x33189C8 -> country_database = 0x22AA6935E00`
- `country_database+0x00 -> tag_table = 0x22A9CCD3C70`
- tag table prefix `---`, `GER`, `ENG`, `SOV` validated; `tag_count=474`, `active_count=440`, `row_count=440`
- `CGameState+0x310 -> country_table = 0x22BA5CB0FD0`
- `country_count=440`; vtable-valid country slots from tag ids `1..439`: `439`
- object `tag_db_id` matched the table index for `432` rows and mismatched for
  `7` rows; the depth body probe uses this as root evidence, not as a final
  country identity bridge replacement
- `country_vtable = module+0x27D0E90 = 0x7FF6AD7C0E90`

## Focus Pause

Base focus status root remains valid:

- `CCountry+0x1380 -> CFocusStatus`
- `CFocusStatus+0x10` current national focus pointer
- `CFocusStatus+0x18` current continuous focus pointer
- `CFocusStatus+0x38` progress raw
- `CFocusStatus+0x40/+0x48/+0x4C` completed vector data/capacity/size

Probe evidence:

- `439` focus status rows valid
- `106` rows had active focus or progress
- `97` national-current rows
- `1` continuous-current row
- `439` completed vectors valid
- sampled `CFocusStatus+0x20..+0x37`; `+0x38` is known progress

The lane-local focus pause candidate gate then scanned `439` live
`CFocusStatus` rows, with `114` rows carrying focus context and `325` rows
without current/continuous focus context. It rejected these canonical paused
boolean byte candidates in `CFocusStatus+0x20..+0x37`:

- `0x20`, `0x21`, `0x22`, `0x23`, `0x24`, `0x25`, and `0x28`, because their
  only nonzero values were not boolean (`112`, `226`, `43`, `199`, `43`, `2`,
  and `3`).
- `0x30`, `0x31`, `0x32`, `0x33`, `0x34`, and `0x35`, because every sampled
  row held non-boolean constant bytes (`112`, `209`, `5`, `174`, `246`, `127`);
  qword `+0x30` also classified as module-pointer-like storage on all `439`
  rows.

The v2 focus pause candidate gate rejects the remaining `11` all-zero bytes as
structural overlap with rejected qword storage:

- `0x26` and `0x27` overlap qword `+0x20`, which is pointer-like storage when
  nonzero (`0x22BC72BE270` in the live sample).
- `0x29..0x2F` overlap qword `+0x28`, which has a stable non-boolean scalar
  value (`0x3`) when nonzero.
- `0x36` and `0x37` overlap qword `+0x30`, which is module-pointer-like storage
  on all `439` sampled rows.

Verdict: the full `CFocusStatus+0x20..+0x37` byte window is rejected as the
paused-boolean home: `13` direct byte-shape rejections plus `11` structural
qword-overlap rejections. Focus pause remains blocked only because no alternate
pause root or same-build `paused=true` oracle is validated. The bounded watcher
proof observed `2` unique game hours with the candidate window already rejected
and records the required oracle contract (`game_hour_raw`, `tag_db_id`,
focus token, `paused_expected_true`, and source).

The wider v2 focus pause gate then scanned `CFocusStatus+0x00..+0x7F` while
explicitly excluding the already rejected `+0x20..+0x37` window. It sampled
`439` focus-status rows on the same `870ebf` build, with `112` rows carrying
focus context and `327` rows without focus context. It rejected `100`
outside-window byte candidates:

- `0x00..0x07` as object header/vtable storage; all `439/439` sampled rows
  held the module pointer `0x7ff6ad713d30`.
- `0x10..0x17` as `current_focus_ptr`, `0x18..0x1F` as
  `continuous_focus_ptr`, `0x38..0x3B` as `progress_raw_i32`,
  `0x40..0x47` as completed-vector data, `0x48..0x4B` as completed-vector
  capacity, and `0x4C..0x4F` as completed-vector size; every one of these
  known fields matched the root probe's validated row value on `439/439` rows.
- `0x08..0x0D`, `0x50..0x55`, `0x58..0x5A`, `0x5C..0x5E`,
  `0x60..0x65`, `0x68`, `0x6C`, `0x70..0x72`, and `0x74..0x7D` by direct
  non-boolean byte evidence.
- `0x0E`, `0x0F`, `0x56`, `0x57`, `0x7E`, and `0x7F` by pointer-qword
  structural overlap.
- `0x5B`, `0x5F`, `0x66`, `0x67`, `0x69`, `0x6A`, `0x6B`, `0x6D`,
  `0x6E`, `0x6F`, and `0x73` by aligned dword scalar overlap.

The static v2 gap gate then closes the remaining `0x3C..0x3F` bytes. It derives
the live `CFocusStatus` vtable RVA `0x2723D30` from the object header, scans
`28` vtable entries and `18` executable methods, finds `2` methods with
qword-sized access at `CFocusStatus+0x38`, and finds `0` methods with direct
access to `+0x3C..+0x3F`. Exact static evidence:

- method `0x2D8580`: `0x2D86B2 lea rdx, [rcx + 0x38]`;
- method `0x2DC4E0`: `0x2DC6A6 mov r8, qword ptr [rbp + 0x38]`.

Verdict: the bounded `CFocusStatus+0x00..+0x7F` range is rejected as the
paused-boolean home. Focus pause remains blocked on a same-build paused=true
oracle or a production root outside `CFocusStatus`.

Lane-local badge:

- status flag bit `0`
- badge kind `1`
- source domain `12` (`focus_live`)
- evidence code `1`
- detail `focus_pause_flag_fail_closed_no_paused_true_ground_truth`

## Direct Completed-Focus Token Closure

The integrated domain `45` gate over
`replay_native/runs/focus_research_decisions_depth_live_integrated_870ebf_20260701_v1/collector_stream.bin`
decoded `89` depth data records with zero parse errors. It saw:

- `9` focus event rows
- `0` exact completed-focus closure rows
- `1` inferred completed-focus closure row
- `0` unresolved completed-focus closure rows

Verdict after scope correction: direct completed-focus token closure is a
validated slice only when compact focus source rows carry the completed vector.
The lane-local Rust test `derives_exact_focus_completion_from_completed_token_delta`
proves exact closure from completed-token deltas. The exact-completed production
slice then produced `1` exact completed-focus row, `0` inferred rows, and `0`
unresolved rows over `144` depth frames with zero skipped hours and verified
`.battleplan` output. The shared collector hunk that enables compact focus rows
to carry completed vectors is recorded in `SHARED_RESIDUAL_PATCH_PLAN.md` for
orchestrator review.

## Decision Bodies

Validated decision status root:

- `CCountry+0xFA0 -> CDecisionStatus`
- `CDecisionStatus` vtable `module+0x27D50F0 = 0x7FF6AD7C50F0`
- `CDecisionStatus+0x08` owner country back-pointer
- `439` decision status rows valid

List/body probe results:

| List | Offset | Entries | Sampled | Record vtable RVA | Definition token verdict |
| --- | ---: | ---: | ---: | --- | --- |
| `decisions_taken_bucket` | `0x40` | `56` | `56` | `0x27D4F00` | rejected timed-definition layout |
| `active_targeted_decision` | `0x88` | `5452` | `5270` | `0x27D4F00` | rejected timed-definition layout |
| `active_timed_decision` | `0xA0` | `85` | `85` | `0x27D4FD0` | existing timed body mostly validates; gate failed for at least one sampled row |
| `active_targeted_timed_decision` | `0xE8` | `2786` | `1620` | `0x27D5038` | definition pointer valid, target details unproven |
| `decision_to_re_enable` | `0x130` | `1` | `1` | `0x27D4E60` | rejected timed-definition layout |

Rejected for direct emission now:

- Non-timed targeted/cooldown records at vtable `0x27D4F00` do not use the
  `CTimedDecision+0x10 -> CDecisionDef` layout. Their exact decision token,
  target, and cooldown body fields remain fail-closed.
- `decision_to_re_enable` vtable `0x27D4E60` also rejects the timed definition
  layout in this sample.

Narrowed but not complete:

- Targeted-timed records at vtable `0x27D5038` have
  `record+0x10 -> CDecisionDef`, and all `1403/1403` sampled records resolved a
  decision token at `CDecisionDef+0x120`.
- Integer candidates exist for targets. Example:
  `SOV_set_up_puppet_state_in_turkey` has `record+0x20 = 21`, which resolves to
  `TUR` through the current tag table. Other fields also contain valid tag ids,
  so `+0x20` is a candidate, not a production-safe exact target field yet.

## Decision Body Candidate Gate Refresh

The lane-local v5 probe and candidate gate reran against PID `45880` on the
same `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`
build. The probe sampled:

- `76/76` `decisions_taken_bucket` records at vtable `0x27D4F00`.
- `5349/5452` `active_targeted_decision` records at vtable `0x27D4F00`.
- `1891/3332` `active_targeted_timed_decision` records at vtable
  `0x27D5038`.
- `1/1` `decision_to_re_enable` records at vtable `0x27D4E60`.

The executable gate rejected `22` candidate layouts/offsets and reduced the
decision target/cooldown search space:

- `record+0x10 -> CDecisionDef` timed-body layout is rejected for
  `decisions_taken_bucket`, `active_targeted_decision`, and
  `decision_to_re_enable`: `0/56`, `0/5270`, and `0/1` sampled rows resolved a
  decision token or satisfied the timed body gate.
- `active_targeted_decision+0x54` is rejected as a universal target tag:
  `5268/5270` sampled rows looked like tag id `170:GAM`, so it is not present
  on every record and is not record-specific.
- `active_targeted_decision+0x78` is rejected as an exact target tag:
  `5270/5270` sampled rows were the constant tag id `15:SPR` across `439`
  owner countries.
- `decisions_taken_bucket+0x54` and `+0x78` are rejected as exact target tags:
  `56/56` rows were constant `170:GAM` and `15:SPR` across `34` owner
  countries.
- `active_targeted_timed_decision+0x50` is rejected as an exact target tag:
  `1620/1620` sampled rows were constant `15:SPR` across `439` owner
  countries.
- `active_targeted_timed_decision+0x20`, `+0x24`, `+0x28`, `+0x30`, `+0x34`,
  `+0x5C`, `+0x60`, `+0x64`, `+0x70`, `+0x74`, `+0x78`, and `+0x7C` are
  rejected as universal exact country-target tag fields because each is absent
  as a valid tag id on at least one sampled row. The strongest prior candidate,
  `+0x20`, only appeared as a valid tag on `490/1620` sampled rows.
- `active_targeted_timed_decision+0x60` and `+0x70` are rejected as universal
  country-target pointer fields because they resolved to country pointers on
  only `6/1620` and `1/1620` sampled rows.

Pre-oracle blocker: `active_targeted_timed_decision` validates
`record+0x10 -> CDecisionDef` on `1620/1620` sampled rows. That was enough for
a future shared body-token patch, documented in `SHARED_RESIDUAL_PATCH_PLAN.md`,
but the exact target/cooldown/day/state body fields remained blocked until the
save-oracle gates below validated `record+0x20` for matched country-target rows
and `record+0x24` for matched numeric-target rows. Keep cooldown/day/state
details, numeric target semantic classification, and unmatched transition rows
fail-closed.

The residual blocker gate also rejects a definition-agnostic universal target
layout for `active_targeted_timed_decision`: candidate country-tag offsets are
sparse or constant across sampled records (`record+0x20` appears as a valid tag
on only `490/1620` rows; `record+0x50` is the constant tag `15:SPR` on
`1620/1620` rows; country-pointer candidates at `+0x60/+0x70` appear on only
`6/1620` and `1/1620` rows). The remaining plausible path is definition-aware or
variant target decoding, which needs the same-frame oracle or static decision
target-type metadata.

The lane-local decision variant gate then groups the same `0x27D5038`
targeted-timed rows by decision definition token and active mod-set metadata.
The full v3 refresh samples `3384` definition-valid targeted-timed records
across `262` decision tokens and records:

- `50` definition-local candidate slices where a nonconstant tag-id offset
  covers every sampled row for that decision token; these are
  `CANDIDATE_NOT_PRODUCTION`, not accepted target fields.
- `49` sampled decision tokens whose scanned `record+0x20..+0x7C` i32 tag-id
  window has no nonconstant full-coverage country-tag candidate, rejecting a
  token-local country-target tag field in that scanned window.

This reduces the targeted-timed search space but does not complete decision
target extraction. The save-oracle gates below supply that evidence for matched
country-target rows and matched numeric-target rows only; cooldown/day/state
details, numeric target semantic classification, unmatched transition rows, or
a new live root that exposes target bodies directly remain open.

## Decision Target Save Oracle Gate

The v6 full-decision probe removes the previous sample cap for decision bodies:
`active_targeted_timed_decision` has `3384/3384` records sampled at vtable
`0x27D5038`, and every sampled row validates `record+0x10 -> CDecisionDef`.
The save-oracle gate then compares those live rows to the current autosave
targeted-decision rows. The save stores `3384` targeted decision rows in this
scope (`3381` `active_targeted_decision` rows plus `3`
`active_targeted_timed_decision` rows), matching the live vtable row count.

Validated root:

- `CDecisionStatus+0xE8` active targeted timed vector.
- record vtable RVA `0x27D5038`.
- `record+0x10 -> CDecisionDef`.
- `CDecisionDef+0x120` decision token.
- `record+0x20` country target `tag_db_id` for exact save/live matched
  country-target rows.

Proof result:

- live rows: `3384`;
- live rows with `record+0x20` tag candidate: `1094`;
- save country-target rows: `1096`;
- exact matched live country-target rows: `1061`;
- validated decision tokens: `180`;
- unmatched live country-target rows: `33`;
- unmatched save country-target rows: `35`;
- unmatched decisions: `5`.

Matched examples include `instantiate_collaboration`, `SPA_reassert_dominance`,
`JAP_jeweled_spear_claim_decision`, `COG_uranium_trust_invitation_decision`,
`nation_building`, and `BRA_invite_to_mercosul`. The five unmatched transition
tokens are `ITA_bop_send_support`,
`JAP_border_conflict_escalation_warning_MON`,
`JAP_tauran_border_conflict_warning_MON`,
`JAP_tauran_border_incident_forgotten`, and
`WTT_border_conflict_initiate_incident`.

This validates `record+0x20` as a production root for matched country-target
rows. It does not validate cooldown/day/state encodings or the unmatched
transition rows; those remain fail-closed.

## Decision Numeric Target Save Oracle Gate

The numeric target save-oracle gate compares the same `3384` live
`active_targeted_timed_decision` rows to save-side targeted decision rows whose
`target` is numeric. It validates the best live candidate as:

- `CDecisionStatus+0xE8` active targeted timed vector.
- record vtable RVA `0x27D5038`.
- `record+0x10 -> CDecisionDef`.
- `CDecisionDef+0x120` decision token.
- `record+0x24` positive i32 numeric target id for exact save/live matched
  `target=<number>` rows.

Proof result:

- save numeric target rows: `2288`;
- best offset: `record+0x24`;
- live positive rows at `record+0x24`: `2289`;
- exact matched numeric target rows by `(decision,target_id)`: `2285`;
- matched intersection ratio: `0.998689`;
- runner-up offset intersection rows: `7`;
- unmatched live numeric target rows: `4`;
- unmatched save numeric target rows: `3`.

The bounded mismatches are `CHI_bog_them_down`,
`CHI_declare_war_zone`, `CHI_demonstrate_our_resolve_decision`, and
`SPR_eliminate_guerrillas`, matching the same transition/stale-save pattern as
the country-target gate. This validates `record+0x24` as a production root for
exact matched numeric target rows. It does not prove whether every numeric id is
a state id, province id, or another decision-specific body id; semantic
classification and cooldown/day/state encodings remain fail-closed.

The lane-local compact schema now has a gated decision-target row for this
validated subset plus the numeric target subset. `src/lib.rs` uses the reserved
depth header prefix for `decision_target_count` and emits `tag_db_id`,
`target_id`, flags, source kind, and decision token. Rust test coverage proves
the extended compact `decisions_live` fixture derives one country target row and
one numeric target row, rejects invalid target metadata, and keeps badge kind
`2` present for unresolved body fields. This is schema/decoder/gate proof only;
shared production capture still requires the orchestrator patch in
`SHARED_RESIDUAL_PATCH_PLAN.md`.

Lane-local badge:

- status flag bit `1`
- badge kind `2`
- source domain `16` (`decisions_live`)
- evidence code `2`
- detail `decision_target_fields_fail_closed_body_layout_unvalidated`

## Direct Transition/Event Markers

No current-build focus/decision/research engine event callback or transition
function was validated in this slice.

The static event-hook candidate gate scanned
`<local-path> of Iron IV\hoi4.exe` with SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`. It
rejects:

- `direct_decision_event_hook_by_named_on_action_string`: no exact
  `on_decision`, `on_decision_taken`, `decision_taken`,
  `on_timed_decision_complete`, or `on_decision_timeout` string anchor exists.
- `focus_completed_callback_names`: no exact event-style
  `on_focus_complete`/`on_focus_completed`/`on_national_focus_complete` string
  anchor exists.

It recorded `5` static text-xref anchors (`on_tech_research_completed`,
`on_research_complete`, `on_research_complete_limit`,
`complete_national_focus`, and `uncomplete_national_focus`). The static
contract gate rejects all `5` as registration/catalog arguments rather than
runtime callbacks:

| Anchor | String RVA | Xref RVA | Function | Registration call |
| --- | ---: | ---: | --- | ---: |
| `on_tech_research_completed` | `0x2AA30C0` | `0x10C8E4` | `0x82CB0..0x11896F` | `0x24AFC10` |
| `on_research_complete` | `0x2AA6F30` | `0x11117A` | `0x82CB0..0x11896F` | `0x24AFC10` |
| `on_research_complete_limit` | `0x2ADE1F8` | `0x1074F3` | `0x82CB0..0x11896F` | `0x24AFC10` |
| `complete_national_focus` | `0x2AA2318` | `0x99AA4` | `0x82CB0..0x11896F` | `0x24AFC10` |
| `uncomplete_national_focus` | `0x2ADEA48` | `0x108015` | `0x82CB0..0x11896F` | `0x24AFC10` |

For every anchor, the xref instruction loads the string into Windows x64
argument register `r8`; the following local sequence loads a registration key
into `edx`, sets a global registry pointer in `rcx`, and calls `0x24AFC10`.
The containing function has `10697` string references, so these anchors do not
provide a runtime function body or focus/research/decision object argument
contract. The string/xref hook route has `0` remaining static anchors.

Replay transition markers remain deterministic when derived from compact
production frames:

- provenance domains: compact `focus_live` domain `12`, `tech_live` domain `13`,
  `decisions_live` domain `16`, plus frame-end domain `1000`
- state keys: focus by `tag_db_id`, research by `(tag_db_id, slot_index)`,
  decisions by `tag_db_id` and timed decision token
- event rows are emitted only from ordered previous/current frame deltas
- gate requires source parse success, depth decode success, monotonic record
  sequence, and zero skipped game hours for proof acceptance

Lane-local badge:

- status flag bit `2`
- badge kind `3`
- source domain `1000` (`frame_end`)
- evidence code `3`
- detail
  `direct_event_hooks_unavailable_transitions_derived_from_compact_frames`

The badge-aware lane proof from
`replay_native/runs/focus_research_decisions_depth_live_20260701_v2_badges`
derives:

- source frames `87`
- game hours advanced `86`
- skipped game hours `0`
- focus source rows `631`
- research slot rows `1005`
- decision country rows `397`
- timed decision rows `50`
- derived research ETA rows `743`
- derived decision status rows `397`
- derived decision event rows `50`
- fail-closed badge rows `261` (`87` frames times `3` required badges)
  - focus pause fail-closed badge rows `87`
  - decision target fields fail-closed badge rows `87`
  - direct event hooks unavailable badge rows `87`
- source/depth parse errors `[]`

The integrated production-shaped proof from
`replay_native/runs/focus_research_decisions_depth_live_integrated_870ebf_20260701_v1`
already verifies domain `45` through compact DLL/Rust `.battleplan` packing with
zero skipped hours and no source/partial/worker/bridge failures. The
exact-completed production slice from
`replay_native/runs/focus_research_decisions_depth_live_exact_completed_20260701_v1`
also verifies through `.battleplan` and closes one completed-focus token
exactly. The v10 residual blocker gate remains `NOT COMPLETE` because focus
pause, target/cooldown body details, and direct engine hooks are still blocked
and badged, while recording one validated targeted-timed country-target root,
one validated targeted-timed numeric-target root, one validated lane-local
compact target schema/gate,
the full `24` rejected `+0x20..+0x37` focus pause byte candidates, `100`
additional rejected outside-window focus pause byte candidates, `4` static
qword-overlap gap rejections, `49` rejected definition-local decision target
slices, and `2` rejected direct event-hook candidate routes plus `5` rejected
static event-hook anchors as additional search-space reduction.
