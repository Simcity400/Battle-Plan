# Domain 57 Session Event Root-Discovery Runsheet

Lane: `session_multiplayer_metadata_live`
Build gate: `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`
Scope: lane-local probes only; no collector/recorder wiring.

## Current Lane Findings

- Existing validated roots remain unchanged:
  - `CGameState = u64[module+0x3316E90]`.
  - player-country roots: `CGameState+0x394` count, `+0x388` human byte array,
    `+0x310` country handles, `+0x358` TAG records.
  - pause roots: clock slots `module+0x33172E0` and `module+0x33172E8`,
    vtable `module+0x2952510`, paused byte `+0x6C1`.
- Existing schema/proof coverage already accepts domain-57 event rows:
  `hotjoin`, `tag_switch`, `chat`, `system_message`, `alert`, `notification`.
- The three contract rows are still `ROOT_DISCOVERY_OPEN`: no live pollable root
  or production-safe event source is validated for any of them.
- Preferred strategy is hook-free polling. A pure code-hook-only result is not a
  v1 acceptance; document it as rejected/deferred unless a shared hook
  infrastructure decision happens outside this lane.

## Probe Strategy

1. `probe_870ebf_session_rtti.py`
   - Discovers current-build MSVC RTTI names for chat/message/alert/lobby/session
     classes.
   - Derives vtable RVAs from the RTTI complete object locator.
   - Optionally scans heap objects whose first qword is a candidate vtable.

2. `probe_870ebf_session_event_roots.py`
   - Runs each sub-scope independently.
   - Uses RTTI-derived vtables to find live objects.
   - Samples MSVC `std::string` fields in candidate objects.
   - Searches for vector-like owners of object pointer arrays as pollable root
     candidates.
   - Optional `--expect-text` searches heap for a unique MP fixture chat/system
     text. This is a semantic oracle, not root acceptance by itself.

## Orchestrator Commands: 1943 SP Save

Run from the repository root:

```powershell
Set-Location 'C:\Users\Example\hoi4_wt\w3_session_hooks'
$runName = 'session57_hooks_sp_1943_' + (Get-Date -Format 'yyyyMMdd_HHmmss')
$runDir = Join-Path (Resolve-Path '.').Path "extractor\native\runs\$runName"
New-Item -ItemType Directory -Force $runDir | Out-Null
$env:PYTHONPATH = (Resolve-Path 'extractor\memory_reader').Path
Set-Location 'extractor\native\lanes\session_multiplayer_metadata_live'

python .\live_probe_session.py --expect-tag IRE --expect-name Ireland --expect-index 29 --expect-paused 1 --json (Join-Path $runDir '00_live_probe_session_strict.json')

python .\probe_870ebf_session_rtti.py --out (Join-Path $runDir '01_session_rtti_heap.json') --heap-scan --heap-scan-seconds 45 --max-heap-hits 512

python .\probe_870ebf_session_event_roots.py --out (Join-Path $runDir '02_session_event_roots_all.json') --scope all --heap-scan-seconds-per-scope 45 --ref-scan-seconds-per-scope 45 --owner-scan-seconds 8 --max-heap-hits-per-scope 384 --max-object-samples 24

python .\probe_870ebf_session_event_roots.py --out (Join-Path $runDir '03_alerts_notifications_deep.json') --scope alerts_notifications --heap-scan-seconds-per-scope 90 --ref-scan-seconds-per-scope 90 --owner-scan-seconds 12 --max-heap-hits-per-scope 1024 --max-object-samples 48
```

The strict live probe is included first to prove the running process/build and
validated session context did not drift before event-root probing.

## Decision Tree: Hotjoin / Tag-Switch Timeline

Accepted for a follow-up MP fixture only if:

- `00_live_probe_session_strict.json` has `"passed": true`.
- The relevant scope in `02_session_event_roots_all.json` has
  `summary.status = "pollable_container_candidate"`.
- Candidate owner/root is stable across a warm re-read and rerun.
- Later MP fixture performs a controlled join/leave or tag-switch and the same
  root shows a monotonic entry/count or state delta matching the action.

Rejected/deferred for v1 if:

- The only route is a send/handler function address or code-hook candidate.
- RTTI exists but no pollable owner/container is found.
- SP shows no event objects: this is not a rejection by itself because SP cannot
  generate hotjoin/tag-switch events.

Output classification:

- `pollable_container_candidate`: carry candidate to MP fixture.
- `live_heap_objects_observed_root_open`: not enough; needs owner/root.
- `rtti_only_no_live_objects`: fixture-blocked for SP.
- `no_current_rtti_candidate`: broaden RTTI keywords before rejecting.

## Decision Tree: Chat / System Messages

SP run can validate only container/root shape, not chat semantics.

Accepted after MP fixture only if:

- SP or MP root probe finds `summary.status = "pollable_container_candidate"` for
  `chat_system_messages`.
- In an MP fixture, send a unique message such as
  `D57_CHAT_PROBE_<timestamp>` and rerun:

```powershell
python .\probe_870ebf_session_event_roots.py --out (Join-Path $runDir '04_chat_mp_text_oracle.json') --scope chat_system_messages --expect-text 'D57_CHAT_PROBE_<timestamp>' --heap-scan-seconds-per-scope 90 --ref-scan-seconds-per-scope 90 --owner-scan-seconds 12 --max-heap-hits-per-scope 1024 --max-object-samples 64
```

- The JSON must show `summary.expected_text_observed = true`.
- The same pollable root must show a count/text delta matching the sent chat or
  system message. A raw heap text hit without a pollable root remains
  insufficient.

Rejected/deferred for v1 if:

- Chat text appears only as loose heap string hits with no stable owner/container.
- The only precise capture point is a code hook.
- SP run has no live chat objects and no root: fixture-blocked, not rejected.

## Decision Tree: Alerts / Notifications

The current SP wartime save should have active alerts, so this is the strongest
SP target.

### Follow-up Reachability Probe

The first RTTI heap strategy found alert/message/notification class names but no
live heap object starts. Pivot to reachability from validated roots:

```powershell
python .\probe_870ebf_alert_reachability.py --out (Join-Path $runDir '04_alert_reachability_vectors.json') --scan-bytes 0x10000 --child-scan-bytes 0x3000 --max-child-roots 512 --sample-elements 32 --warm-delay-ms 500 --expect-min-count 1
```

If the visible alert count is known, add:

```powershell
--expect-alert-count <visible_alert_count>
```

This probe derives real current-build RTTI names containing `essage`, `lert`,
`otification`, or `vent`, then scans `CGameState`, the human `CCountry` root,
and one-level child objects for vector-like `{begin,end,cap}` members whose
sampled elements point at those RTTI classes.

Final 2026-07-03 result: the orchestrator ran this reachability probe with
wartime alerts visible and it returned `no_reachable_vector_candidates`. The
final disposition is `REJECTED_DERIVED_UI_STATE`; see
`SESSION_EVENT_DISPOSITION.md`.

Historical acceptance criteria for a candidate, not met by the final
`04_alert_reachability_vectors.json` run:

- `04_alert_reachability_vectors.json` has
  `summary.status = "reachable_vector_candidates_found"`.
- Prefer candidates with `summary.strong_candidate_count > 0`.
- Candidate objects or container rows are stable after warm re-read.
- Element RTTI names are real alert/message/notification/event classes.
- Candidate `count` equals the visible alert count, or later fixture action
  shows a plausible delta.
- A second SP rerun after manually changing alert state, if available, shows a
  plausible delta. If no UI action is allowed, keep as candidate until a fixture
  can toggle an alert.

Rejected/deferred for this strategy if:

- The game visibly has active alerts but the deep alert probe returns only
  `rtti_only_no_live_objects` or no RTTI candidates. That rejects the current
  broad heap-vtable strategy, not the replay scope.
- The reachability probe finds no vector-like candidate from `CGameState`, the
  human `CCountry`, or one-level child roots.
- Candidate rows are loose heap objects with no owner/root.
- Candidate rows are definition databases or static UI templates rather than
  live alert state.

## Artifact Expectations

Each JSON artifact must contain:

- `build_gate_ok: true`.
- `session_context.ok: true`.
- process SHA equal to the build gate above.
- per-scope `summary.status` and `decision`.

Any mismatch means fail closed and do not infer offsets.

## After Orchestrator Feedback

If a sub-scope validates a pollable root:

- Add a lane-local extraction slice and mock tests in this lane only.
- Update `INTEGRATION_PACKET_HOOKS.md` with exact offsets, root chain, root
  evidence mask proposal, and artifact provenance.
- Do not wire collector/recorder or edit shared contract files in this lane pass.

If a sub-scope does not validate:

- Keep it `ROOT_DISCOVERY_OPEN`, `MP_FIXTURE_BLOCKED`, or
  `HOOK_ONLY_REJECTED_FOR_V1` exactly as supported by JSON evidence.
