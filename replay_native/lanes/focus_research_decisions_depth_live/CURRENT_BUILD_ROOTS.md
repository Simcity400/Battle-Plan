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
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/decision_body_candidate_gate_20260701_v1/decision_body_candidate_gate.json`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/decision_body_candidate_gate_20260701_v1/DECISION_BODY_CANDIDATE_GATE.md`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/integrated_depth_gate_20260701_v1/depth_gate_manifest.json`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/residual_blocker_gate_20260701_v1/residual_blocker_gate.json`
- `replay_native/lanes/focus_research_decisions_depth_live/proofs/residual_blocker_gate_20260701_v1/RESIDUAL_BLOCKER_GATE.md`

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
- object `tag_db_id` matched the table index for `433` rows and mismatched for
  `6` rows; the depth body probe uses this as root evidence, not as a final
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
- `114` rows had active focus or progress
- `46` national-current rows
- `67` continuous-current rows
- `439` completed vectors valid
- sampled `CFocusStatus+0x20..+0x37`; `+0x38` is known progress

Verdict: focus pause flag is not validated. There is no current-build
`paused=true` ground truth in this live sample, so the production domain must
keep pause fail-closed and omit/badge it as unknown. Do not synthesize a boolean
from the candidate window.

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

Verdict: direct completed-focus token closure is still blocked in the current
compact production path. The integrated proof ran with `compact_country_frame =
true`, and the current shared collector compact focus read omits the completed
focus vector in compact mode. Completion rows can therefore only infer a token
from the previous active focus unless the shared compact focus reader starts
emitting bounded completed tokens or a lane-specific collector hook emits exact
completed-token deltas.

## Decision Bodies

Validated decision status root:

- `CCountry+0xFA0 -> CDecisionStatus`
- `CDecisionStatus` vtable `module+0x27D50F0 = 0x7FF6AD7C50F0`
- `CDecisionStatus+0x08` owner country back-pointer
- `439` decision status rows valid

List/body probe results:

| List | Offset | Entries | Sampled | Record vtable RVA | Definition token verdict |
| --- | ---: | ---: | ---: | --- | --- |
| `decisions_taken_bucket` | `0x40` | `67` | `67` | `0x27D4F00` | rejected timed-definition layout |
| `active_targeted_decision` | `0x88` | `5470` | `4897` | `0x27D4F00` | rejected timed-definition layout |
| `active_timed_decision` | `0xA0` | `86` | `82` | `0x27D4FD0` | existing timed body mostly validates; gate failed for at least one sampled row |
| `active_targeted_timed_decision` | `0xE8` | `3427` | `1403` | `0x27D5038` | definition pointer valid, target details unproven |
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

The lane-local v4 probe and candidate gate reran against PID `45880` on the
same `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`
build. The probe sampled:

- `56/56` `decisions_taken_bucket` records at vtable `0x27D4F00`.
- `5270/5452` `active_targeted_decision` records at vtable `0x27D4F00`.
- `1620/2786` `active_targeted_timed_decision` records at vtable
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

Remaining blocker: `active_targeted_timed_decision` still validates
`record+0x10 -> CDecisionDef` on `1620/1620` sampled rows, but the exact target
body field remains blocked. The next useful evidence is a same-frame oracle row
keyed by owner tag, decision token, and known target country/state/province for
records with vtable `0x27D5038`. Until then, keep targeted/cooldown target
details fail-closed and do not emit partial target fields.

The residual blocker gate also rejects a definition-agnostic universal target
layout for `active_targeted_timed_decision`: candidate country-tag offsets are
sparse or constant across sampled records (`record+0x20` appears as a valid tag
on only `490/1620` rows; `record+0x50` is the constant tag `15:SPR` on
`1620/1620` rows; country-pointer candidates at `+0x60/+0x70` appear on only
`6/1620` and `1/1620` rows). The remaining plausible path is definition-aware or
variant target decoding, which needs the same-frame oracle or static decision
target-type metadata.

Lane-local badge:

- status flag bit `1`
- badge kind `2`
- source domain `16` (`decisions_live`)
- evidence code `2`
- detail `decision_target_fields_fail_closed_body_layout_unvalidated`

## Direct Transition/Event Markers

No current-build focus/decision/research engine event callback or transition
function was validated in this slice.

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
zero skipped hours and no source/partial/worker/bridge failures, but the residual
blocker gate remains `NOT COMPLETE` because all three fail-closed badges are
still present and exact completed-focus token closure did not occur in compact
production.
