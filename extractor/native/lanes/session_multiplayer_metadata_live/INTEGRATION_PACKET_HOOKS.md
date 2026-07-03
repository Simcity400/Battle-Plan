# Domain 57 Session Event Root Integration Packet

Lane: `session_multiplayer_metadata_live`
Domain id: `57`
Status: final v1 disposition for residual session-event rows.

This packet covers the three residual contract rows:

- Hotjoin/tag switches: player join/leave/tag-switch timeline.
- Chat/messages: multiplayer chat and system messages.
- Alerts/notifications: important-message log, popups, and visible alerts.

It does not replace `INTEGRATION_PACKET.md`, which documents the already
validated partial slice for player-country rows and pause state.

## Current Acceptance State

Nothing new integrates from this packet.

The already validated domain-57 production slice remains the separate
`INTEGRATION_PACKET.md` partial slice: human player-country rows and pause state.
No hotjoin/tag-switch, chat/system-message, alert, or notification rows are
accepted for production capture in this packet.

The probes written for this pass are:

- `probe_870ebf_session_rtti.py`
- `probe_870ebf_session_event_roots.py`
- `probe_870ebf_alert_reachability.py`
- shared helper `session_probe_common.py`
- run instructions in `PROBE_RUNSHEET.md`
- current fork disposition in `SESSION_EVENT_DISPOSITION.md`

## Final Disposition

After two live SP attempts with wartime alerts visible:

- Alerts/notifications: `REJECTED_DERIVED_UI_STATE` for v1 memory extraction.
  Evidence chain: broad RTTI heap scan found 685 classes / 609 vtables and zero
  live heap hits; alert deep scan found zero pollable candidates; reachability
  from validated `CGameState` and human `CCountry` roots found 339 classes / 301
  vtables but zero direct or child vector candidates. RTTI includes
  `NInterfaceMessages` listener/reader types, indicating transient dispatch
  rather than stored pollable state.
- Hotjoin/tag-switch: `FIXTURE_BLOCKED_MP`.
- Chat/system messages: `FIXTURE_BLOCKED_MP`.

## Existing Event Row Schema

The lane already defines the compact event row in `src/lib.rs`:

```rust
pub struct SessionEventRow {
    pub kind: SessionEventKind,
    pub flags: u32,
    pub actor_key_hash: u64,
    pub actor_name: String,
    pub text: String,
    pub tag_db_id: u32,
    pub value_i32: i32,
    pub related_value_i32: i32,
    pub command_sequence: u64,
    pub event_hour_raw: i32,
    pub event_source_id: u32,
    pub event_state: u32,
}
```

Kinds already assigned:

| Kind | Raw | Use |
| --- | ---: | --- |
| `Hotjoin` | 1 | player join/leave style event |
| `TagSwitch` | 2 | human/player changed controlled country |
| `Chat` | 5 | MP chat text |
| `SystemMessage` | 6 | MP system/lobby/session text |
| `Alert` | 7 | visible alert state |
| `Notification` | 8 | notification/important-message row |

Flags already assigned:

- `EVENT_FLAG_DIRECT_CAPTURE = 1`
- `EVENT_FLAG_UI_VISIBLE = 1 << 1`
- `EVENT_FLAG_HOST_COMMAND = 1 << 2`
- `EVENT_FLAG_TEXT_PRESENT = 1 << 3`

Source flags currently available:

- `SOURCE_FLAG_COMMAND_HOOK_VALIDATED` gates `Hotjoin`, `TagSwitch`, and the
  existing command-like kinds.
- `SOURCE_FLAG_MESSAGE_HOOK_VALIDATED` gates `Chat` and `SystemMessage`.
- `SOURCE_FLAG_NOTIFICATION_HOOK_VALIDATED` gates `Alert` and `Notification`.

The names say `HOOK`, but the preferred v1 implementation remains pollable
memory roots. If a pollable root validates, shared integration can either reuse
these family flags as "event source validated" or add clearer root-oriented flag
names in a later shared schema pass. This lane does not edit shared flags now.

## Field Mapping Proposal

### Hotjoin / Tag Switch

Proposed row mapping after an MP fixture validates a root:

- `kind`: `Hotjoin` or `TagSwitch`.
- `flags`: `EVENT_FLAG_DIRECT_CAPTURE`; add `EVENT_FLAG_UI_VISIBLE` only if the
  source is the visible/system-log row.
- `actor_key_hash`: stable player key hash if sourced; otherwise `0`.
- `actor_name`: display name if sourced; otherwise empty.
- `text`: visible system text if sourced; otherwise empty.
- `tag_db_id`: country slot when the event is tied to a country; otherwise `0`.
- `value_i32`: event subtype enum proposed lane-local:
  - `1` join
  - `2` leave
  - `3` reconnect
  - `4` tag-switch
- `related_value_i32`: prior tag id for tag-switch, otherwise `0`.
- `command_sequence`: monotonic source sequence if present; otherwise collector
  frame sequence.
- `event_hour_raw`: current domain-57 game hour.
- `event_source_id`: compact id assigned to the validated root/source.
- `event_state`: raw source state/subtype when stable; otherwise `0`.

### Chat / System Messages

Proposed row mapping after an MP fixture validates a root:

- `kind`: `Chat` for human chat, `SystemMessage` for system/lobby text.
- `flags`: `EVENT_FLAG_DIRECT_CAPTURE | EVENT_FLAG_TEXT_PRESENT`; add
  `EVENT_FLAG_UI_VISIBLE` if the root is the visible log.
- `actor_key_hash`: stable player key hash if sourced; otherwise `0`.
- `actor_name`: visible sender name if sourced; otherwise empty.
- `text`: UTF-8 visible chat/system text. Empty text is rejected by the existing
  payload gate for these kinds.
- `tag_db_id`: actor country when sourced; otherwise `0`.
- `value_i32`: channel/type enum if sourced; otherwise `0`.
- `related_value_i32`: target slot/tag for direct/team messages if sourced;
  otherwise `0`.
- `command_sequence`: source message id/sequence if sourced; otherwise collector
  frame sequence.
- `event_hour_raw`: current domain-57 game hour.
- `event_source_id`: compact id assigned to the validated message root.
- `event_state`: raw message state when stable; otherwise `0`.

### Alerts / Notifications

No production row mapping is accepted for v1. The existing schema remains
available for a future hook-backed implementation, but the polling lane rejects
direct alert/notification memory extraction.

If explicit hook work is approved later, proposed row mapping would be:

- `kind`: `Alert` for active alert state, `Notification` for queued/logged
  notification or important-message rows.
- `flags`: `EVENT_FLAG_DIRECT_CAPTURE`; add `EVENT_FLAG_UI_VISIBLE` for visible
  alert or popup rows; add `EVENT_FLAG_TEXT_PRESENT` when text/localization key is
  sourced.
- `actor_key_hash`: `0` unless a player/session actor is explicitly sourced.
- `actor_name`: empty unless a player/session actor is explicitly sourced.
- `text`: visible text or localization key if stable; otherwise empty for
  `Alert`, non-empty for `Notification` if emitted as text notification.
- `tag_db_id`: country slot if alert is country-scoped; otherwise `0`.
- `value_i32`: alert/notification type id if sourced; otherwise `0`.
- `related_value_i32`: severity/count/priority if sourced; otherwise `0`.
- `command_sequence`: source id/sequence if sourced; otherwise collector frame
  sequence.
- `event_hour_raw`: current domain-57 game hour.
- `event_source_id`: compact id assigned to the validated alert root.
- `event_state`: active/dismissed/read/raw state if stable; otherwise `0`.

## Offset And Provenance Table

Only already validated context roots are known today:

| Purpose | Root/offset | Provenance |
| --- | --- | --- |
| Build gate | exe SHA `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad` | Existing lane strict probe and sibling probe convention |
| CGameState | `u64[module+0x3316E90]` | `live_probe_session.py`, `live_probe_session_strict.json` |
| Current hour | `CGameState+0x468` | Existing lane probe context only |
| Pause clock | `module+0x33172E0`, `module+0x33172E8`, vtable `module+0x2952510`, bool `+0x6C1` | Existing validated partial slice |

Pending rows to fill from orchestrator JSON:

| Sub-scope | Root candidate | Required provenance before use |
| --- | --- | --- |
| Hotjoin/tag switches | `FIXTURE_BLOCKED_MP` | Requires same-build 2+ player MP fixture with controlled join/hotjoin plus controlled tag-switch or player-country reassignment; validate MP session-slot/player array deltas plus existing human-country array deltas. |
| Chat/system messages | `FIXTURE_BLOCKED_MP` | Requires same-build 2+ player MP fixture, unique chat text via `--expect-text`, and a stable pollable root/container with count/text delta. |
| Alerts/notifications | `REJECTED_DERIVED_UI_STATE` | Two active-alert SP attempts found no pollable memory root; broad RTTI heap scan and root reachability failed. Future revisit requires code-hook instrumentation of `NInterfaceMessages` dispatch/listener path with explicit user sign-off. |

Do not add offsets to build profile or shared collector until a pending row has
all required provenance.

## Fold And Replay Schema Proposal

For v1, do not fold new domain-57 session event rows. Replay alert equivalents
should be derived in the replay layer from underlying captured facts.

If future MP fixtures validate hotjoin/chat roots or future hook work validates
interface-message dispatch, fold under the existing domain sidecar until replay
schema has a first-class session event block:

```jsonc
"_r2_session_multiplayer_metadata_live": {
  "events": {
    "state": "known|unavailable",
    "rows": [
      {
        "kind": "chat|system_message|alert|notification|hotjoin|tag_switch",
        "hour_raw": 60828184,
        "actor_name": "",
        "country_tag": { "state": "known|not_sourced", "value": "IRE" },
        "text": { "state": "known|not_sourced", "value": "..." },
        "value_i32": 0,
        "related_value_i32": 0,
        "source_id": 0,
        "source_provenance": "domain57_probe_artifact_id"
      }
    ]
  }
}
```

Fold rules:

- Do not fabricate chat/system/alert rows from static definitions or UI strings.
- Do not emit alert/notification rows from the polling lane for v1.
- Represent alert-equivalent UI as derived replay state from underlying facts
  such as wars, battles, focuses, decisions, production, and logistics.
- Do not infer join/leave/tag-switch from player-country snapshots alone; that
  can be a derived replay annotation later, not a direct captured event.
- If a root validates but a frame has no rows, emit no rows and keep state
  `known`.
- If root validation fails, set the event family state to `unavailable` and emit
  no rows.
- Keep event rows ordered by `event_hour_raw`, then `command_sequence`, then
  source order.

## Rejection Rules

Reject or defer a candidate for v1 when:

- It requires patching code or installing a game hook.
- It is only a static RTTI class, definition database, localization key, or UI
  template.
- It is a live heap object set with no stable owner/container/root.
- It observes text in heap but cannot tie that text to a pollable message root.
- It cannot be repeated on the exact build gate.

## Next Update Required

This packet is ready as the lane ledger update:

- no new shared integration,
- hotjoin/tag-switch fixture-blocked by named MP fixture,
- chat/system-message fixture-blocked by named MP fixture,
- alerts/notifications rejected for v1 polling extraction as derived UI state.
