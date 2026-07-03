# Wave 2 Event Hooks Probe Runsheet

Lane: `event_timeline_change_log_live`
Target process: `hoi4.exe`
Required executable SHA-256:
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`

All probes are read-only and stdlib-only. They must be run with:

```powershell
$env:PYTHONPATH = "extractor/memory_reader"
```

Every live probe opens `hoi4.exe`, reads its executable SHA-256, and exits
fail-closed with code `2` plus a JSON report if the hash is not exactly the
required value. A missing `hoi4.exe` is not a validation result.

## Output Directory

Use a fresh run directory:

```powershell
$run = "extractor/native/runs/wave2_event_hooks_870ebf_20260703_v1"
New-Item -ItemType Directory -Force $run
```

`extractor/native/runs/` is gitignored. Copy only the summarized findings back
into lane docs after review; do not commit raw run output unless the
orchestrator explicitly asks.

## 1. 1943 IRE Delayed-Event Save Oracle

The orchestrator must supply the exact 1943 IRE save path. If the fixture is the
current autosave, use the default personal save path shown below; otherwise
replace it with the exact fixture file.

```powershell
python extractor/memory_reader/events/build_delayed_event_oracle.py `
  --save "C:\Users\Example\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV\save games\autosave.hoi4" `
  --out "$run/ire_1943_delayed_event_oracle.json"
```

Expected output:

- JSON `schema = event_delayed_event_save_oracle/v1`.
- `source.save_date` records the save date.
- `summary.country_delayed_event_count` and
  `summary.top_level_pending_event_count` are reported.
- If the 1943 IRE save has no `countries.*.delayed_event` rows, this is a
  blocker for root validation, not a rejection of delayed-event queues.

Success criteria:

- The command exits `0`.
- The JSON parses.
- For root validation, `summary.country_delayed_event_count > 0` is required.

Failure criteria:

- The save cannot be read or melted.
- The oracle has zero country delayed-event rows and no controlled delayed-event
  fixture is provided. In that case, keep delayed events `FIXTURE_BLOCKED`.

## 2. Delayed-Event Live Queue Probe

Run only while the same save/session is loaded in the current-build game.
Paused is fine.

```powershell
python extractor/memory_reader/events/probe_870ebf_delayed_event_queues.py `
  --oracle "$run/ire_1943_delayed_event_oracle.json" `
  --out "$run/delayed_event_live_probe_ire_1943.json" `
  --max-countries 512 `
  --scan-country-bytes 0x4000 `
  --scan-gamestate-bytes 0x2000
```

Expected output:

- `process.sha256` equals the required SHA.
- RTTI rediscovery summary for class names containing event/queue/action terms.
- Candidate vectors from bounded `CGameState` and `CCountry` scans.
- `summary.status` is one of `candidate_found_needs_fixture_match`,
  `blocked_no_positive_oracle`, `candidates_rejected_no_event_like_rtti`, or
  `no_candidate_found`.

First-pass success criteria:

- If the oracle has positive delayed-event rows, at least one candidate must
  have event-like RTTI interest and a plausible owner/row-count relationship to
  the positive oracle.
- Candidate rows should be tied to bounded `CGameState` or validated `CCountry`
  owner scans.
- No root is promoted from this probe. `candidate_found_needs_fixture_match`
  only means "candidate ready for second-pass field decoding."

Status matrix:

- `candidate_found_needs_fixture_match`: at least one event-like RTTI candidate
  was found with a positive oracle. Proceed to the second-pass field decoder.
- `blocked_no_positive_oracle`: the save oracle has zero country delayed-event
  rows. Keep delayed events `FIXTURE_BLOCKED`; do not reject queue existence.
- `candidates_rejected_no_event_like_rtti`: a positive oracle exists and raw
  vector/header candidates were observed, but none had event-like RTTI in the
  sampled pointer rows. Record each candidate as rejected for this RTTI route;
  do not promote.
- `no_candidate_found`: a positive oracle exists but no bounded candidate
  vector/header survived the first-pass scan. Keep root discovery open.

Failure/rejection criteria:

- SHA mismatch: fail closed, no evidence.
- Positive oracle exists but the first pass returns
  `candidates_rejected_no_event_like_rtti` or `no_candidate_found`: keep row
  open and record the candidate/rejection reason.
- Candidate is only `saved_event_target` or empty top-level `pending_events`:
  reject as proxy.
- Zero positive oracle rows: blocked, not rejected.

## 2B. Delayed-Event Field Decoder Second Pass

This step is not implemented by the first-pass script above. It is required
before promotion.

Required inputs:

- A positive delayed-event oracle with `countries.<target_id>.delayed_event`
  rows.
- A first-pass candidate report with `summary.status =
  candidate_found_needs_fixture_match`.
- Candidate header address, owner kind, owner tag-db id or world/state scope,
  data pointer, and row count from the first-pass JSON.

Second-pass success criteria:

- Decode owner country tag-db id or explicit world/state scope.
- Decode event name/token/id and match `ssfv_delayed_probe.2` for the controlled
  fixture, or the save-oracle event for the 1943 IRE fixture.
- Decode originator and scope and match the save oracle.
- Decode remaining hours/days/months or absolute due date within one game hour
  of the save evidence.
- Prove the row disappears or the candidate becomes empty after the event fires.

Second-pass failure criteria:

- Any required field cannot be decoded or joined to the oracle.
- The row only matches `saved_event_target` or top-level `pending_events`.
- The candidate requires `STATUS_PARTIAL_DATA` or `STATUS_SOURCE_UNAVAILABLE`
  for already validated domain-51 variables/flags.

## 3. Controlled Delayed-Event Fixture

If the 1943 IRE save has no positive delayed-event rows, run a disposable
fixture before re-running steps 1 and 2.

Required operator action:

- Enable a dev-only fixture event namespace `ssfv_delayed_probe` in a
  disposable local mod or branch.
- The fixture must define `ssfv_delayed_probe.1` so its immediate effect
  schedules exactly one delayed country event:

```the game
country_event = {
  id = ssfv_delayed_probe.1
  is_triggered_only = yes

  immediate = {
    country_event = {
      id = ssfv_delayed_probe.2
      days = 3
    }
  }
}
```

- The fixture must define `ssfv_delayed_probe.2` as the event that fires after
  the delay. Its visible body can be minimal; the important part is that it is
  schedulable and visible after firing.
- Start a single-player current-build session as a stable country such as GER,
  pause on `1936.1.1.1`, and record that country's tag-db id from the accepted
  `CCountry` path.
- Fire the exact console command:

```text
event ssfv_delayed_probe.1 GER
```

- Save immediately after scheduling.
- Keep the game paused and run the oracle and live probe against that save.
- Advance until the event fires, save again, and run the oracle/probe after
  firing as the disappearance/control phase.

Expected before-fire oracle rows:

- `countries.<target_id>.delayed_event`
- `countries.<target_id>.delayed_event.event = ssfv_delayed_probe.2`
- `countries.<target_id>.delayed_event.days = 3` or equivalent positive
  `hours`/`months` countdown for at least `72` hours
- `countries.<target_id>.delayed_event.originator` equals the target country or
  the explicitly recorded origin country id
- `countries.<target_id>.delayed_event.scope` contains a country scope matching
  the target country

Acceptance:

- Before-fire oracle has exactly one positive save row for
  `ssfv_delayed_probe.2` under the target country.
- First-pass live probe produces `candidate_found_needs_fixture_match`.
- Second-pass field decoder matches target owner, event id/name/token,
  originator/scope, and remaining time/due date within one game hour.
- After-fire oracle/live candidate loses that row.

## 4. Mission/Decision Residual Probe

Run during the same current-build session. This probe intentionally excludes
variables/flags because domain 51 already captures them.

```powershell
python extractor/memory_reader/events/probe_870ebf_mission_decision_state.py `
  --out "$run/mission_decision_state_probe.json" `
  --max-countries 512 `
  --scan-decision-status-bytes 0x220 `
  --scan-country-bytes 0x1800
```

Expected output:

- Known `CDecisionStatus` lists are summarized separately and marked
  `already_captured_or_badged_by_domain_16_45`.
- Unknown mission/decision-like vector candidates outside the known lists are
  emitted with RTTI names, row counts, owner samples, and rejection/blocker
  status.

Success criteria:

- A separate mission-instance timer/container candidate exists only if it is
  outside known decision vectors and outside domain-51 variables/flags.
- A candidate must have stable owner joins to validated countries and a
  mission/decision/timer-like row vtable or field shape.

Failure/rejection criteria:

- Candidate duplicates known `CDecisionStatus` vectors at `+0x40`, `+0x88`,
  `+0xA0`, `+0xE8`, `+0x130`, or `+0x148`: reject as already covered/badged.
- Candidate is only a country/global/state variable or flag root: reject as
  already captured by domain 51.
- Candidate has no owner join or no stable row identity: keep blocked, do not
  promote.

## 5. Scripted GUI Decision

No live probe is scheduled for GUI open panels in this wave.

Acceptance for rejection text:

- Treat client GUI open/selected state as non-replay-relevant unless a fixture
  proves server-side persisted state not already captured by domain 51.
- Keep `scripted_gui_random` fixture-blocked until a positive
  `countries.*.scripted_gui_random` fixture exists.

## 6. Compile Gate

Before any live session, compile the scripts:

```powershell
python -m py_compile `
  extractor/memory_reader/events/probe_common.py `
  extractor/memory_reader/events/build_delayed_event_oracle.py `
  extractor/memory_reader/events/probe_870ebf_delayed_event_queues.py `
  extractor/memory_reader/events/probe_870ebf_mission_decision_state.py
```

Pass: command exits `0`.
Fail: fix syntax/import errors before live probing.
