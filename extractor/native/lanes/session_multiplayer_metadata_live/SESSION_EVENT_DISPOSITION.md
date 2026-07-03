# Domain 57 Session Event Disposition

Status date: 2026-07-03
Lane: `session_multiplayer_metadata_live`

This document records the final v1 disposition for the three residual session
event contract rows after two live SP attempts against the 1943 Ireland wartime
save with active alerts visible on screen.

## Evidence Read

Artifacts available in this worktree:

- `extractor/native/runs/session57_hooks_sp_1943_20260703/01_session_rtti_heap.json`
- `extractor/native/runs/session57_hooks_sp_1943_20260703/03_alerts_notifications_deep.json`
- `extractor/native/runs/session57_hooks_sp_1943_20260703/04_alert_reachability_vectors.json`

All three probes ran against the expected `870ebf` exe SHA and valid domain-57
session context:

- `01_session_rtti_heap.json`: `build_gate_ok=true`, process SHA
  `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`, and
  `game_hour_raw=60838485`
  (`01_session_rtti_heap.json:2`, `:18084`, `:18093`).
- `03_alerts_notifications_deep.json`: `build_gate_ok=true`, same process SHA,
  and `game_hour_raw=60838485`
  (`03_alerts_notifications_deep.json:11`, `:26`, `:5042`).
- `04_alert_reachability_vectors.json`: player-country roots were readable
  (`status="ok"`), the RTTI substring scan found `class_count=339` and
  `vtable_count=301`, and the final status was
  `no_reachable_vector_candidates`
  (`04_alert_reachability_vectors.json:33`, `:45`, `:4928`, `:5261`).

The negative evidence is now two independent live attempts:

- Broad RTTI heap strategy: `candidate_class_count=685`,
  `candidate_vtable_count=609`, `heap_vtable_hit_count=0`
  (`01_session_rtti_heap.json:18109`, `:18115`, `:18118`).
- Alert deep scope: no container candidates, zero heap vtable hits, no object
  samples, `rtti_only_no_live_objects`, and top-level
  `no_pollable_candidate_found`
  (`03_alerts_notifications_deep.json:31`, `:41`, `:48`, `:5028`, `:5056`).
- Reachability from validated roots: `CGameState` had 63 child roots and the
  human `CCountry` had 473 child roots, but both direct and child candidate
  counts were zero; summary candidate, strong, and warm-stable counts were zero
  (`04_alert_reachability_vectors.json:4932`, `:4933`, `:5080`, `:5086`,
  `:5087`, `:5234`, `:5263`, `:5266`, `:5267`).

The architecture signal points away from stored pollable alert state. The
matched RTTI names include `NInterfaceMessages` listener templates such as
`SScopedListener<...NInterfaceMessages...>` and `SEntityEventReader`
(`04_alert_reachability_vectors.json:48`, `:86`, `:116`). This is consistent
with an interface-message listener/dispatch path rather than a stable
game-state-owned alert list reachable from `CGameState` or player `CCountry`.

## Alerts / Notifications

Disposition: `REJECTED_DERIVED_UI_STATE` for v1 memory extraction.

Reason:

- The fixture had active wartime alerts visible, so this is not an MP-fixture
  limitation.
- Broad current-build RTTI discovery found many alert/message/notification
  classes, but no live heap object-start instances.
- A targeted reachability walk from validated `CGameState`, the human
  `CCountry`, and one-level child roots found no vector-like alert/message/
  notification/event container.
- RTTI evidence points to `NInterfaceMessages` listener/dispatch architecture,
  not stored pollable state.

Replay-layer implication:

- On-screen alerts are derived UI state. The replay should reconstruct
  alert-equivalent markers from underlying replay facts instead of recording
  alert UI rows directly.
- Examples of source domains for derivation:
  - wars/battles and combat pressure: domains 14, 16, 17, 55
  - focus/decision/mission-derived events: domain 45
  - production/equipment/resource pressure: domains 21 and related economy
    slices
- The derived replay UI can badge these as replay-derived alerts rather than
  live memory-extracted alert rows.

Future revisit:

- The exact future approach is code-hook instrumentation of the
  `NInterfaceMessages` dispatch/listener path.
- That is outside the current polling architecture and needs explicit user
  sign-off plus shared hook infrastructure. It should not be treated as a v1
  polling-lane task.

## Hotjoin / Tag-Switch

Disposition: `FIXTURE_BLOCKED_MP`.

Reason:

- The lane README records the `CGameLobby` / `CPlayerLobby` machinery as MP-only
  and states that hotjoin/chat/tag-switch cannot be validated from an SP fixture
  (`README.md:82`, `:84`, `:89`, `:90`).
- The compact schema is replay-proven: `Hotjoin = 1`, `TagSwitch = 2`, and the
  command-family source gate exists as `SOURCE_FLAG_COMMAND_HOOK_VALIDATED`
  (`src/lib.rs:34`, `:69`, `:70`).
- An SP save cannot create a join, leave, reconnect, or tag-switch event.

Required fixture:

- Live same-build MP session with at least two players.
- Controlled player join/hotjoin event.
- Controlled tag-switch or host/player country reassignment.
- Root to validate: MP session-slot/player array deltas plus the existing
  domain-57 human-country array deltas. Acceptance requires a stable pollable
  root/container or source whose rows change monotonically with the join and
  tag-switch actions.
- If the only precise observation point is a code hook, keep this deferred until
  shared hook infrastructure exists.

## Chat / System Messages

Disposition: `FIXTURE_BLOCKED_MP`.

Reason:

- SP cannot send or receive multiplayer chat, and system join/leave messages are
  tied to an MP session.
- The compact schema is replay-proven: `Chat = 5`, `SystemMessage = 6`, and the
  message-family source gate exists as `SOURCE_FLAG_MESSAGE_HOOK_VALIDATED`
  (`src/lib.rs:35`, `:73`, `:74`, `:696`, `:699`).
- A loose heap string or RTTI class is insufficient without a pollable message
  root.

Required fixture:

- Live same-build MP session with at least two players.
- Send a unique chat line, for example `D57_CHAT_PROBE_<timestamp>`.
- Generate at least one system message, preferably join/leave.
- Run the message probe with `--expect-text` for the unique chat line.
- Acceptance requires both text observation and a stable pollable root/container
  showing a count/text delta. Text observation alone remains insufficient.

## Schema Note

The existing event schema covers all three families. The unresolved gap is live
source/root validation for MP-only rows; alerts are rejected for v1 as derived UI
state rather than as direct memory-extracted rows.

- Kinds: `Hotjoin`, `TagSwitch`, `Chat`, `SystemMessage`, `Alert`,
  `Notification` (`src/lib.rs:69`, `:70`, `:73`, `:74`, `:75`, `:76`).
- Source-family gates: command, message, notification
  (`src/lib.rs:34`, `:35`, `:36`, `:690`, `:699`, `:708`).
