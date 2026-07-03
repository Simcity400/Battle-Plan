# Event State Derivability Audit

Lane: `event_timeline_change_log_live`
Build target for next probes: `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`

This lane is constrained by the current recorder architecture: the production
extractor is polling-based. It does not install inline hooks, and direct call
observation is out of scope for this discovery lane. Event-like replay facts are
therefore capturable only when they are visible as:

- polled state deltas that persist long enough to be sampled; or
- pending/queue state that exists in memory before the transition fires.

Transient calls with no persistent memory effect are rejected for this
architecture unless a future owner explicitly opens hook instrumentation.

## Existing Delta Coverage

The master contract already assigns the durable state that explains most
event-like behavior:

| Source | Domain | What it already gives the replay |
| --- | ---: | --- |
| `focus_live` | 12 | Current focus, continuous focus, completed/progress fields as currently encoded. |
| `tech_live` | 13 | Active research slots and completed technology state. |
| `decisions_live` | 16 | Base decision status and timed-decision rows. |
| `focus_research_decisions_depth_live` | 45 | Derived transition-depth rows from ordered focus/research/decision compact frames, the shared active targeted-timed country-target subset, and fail-closed residual badges. Numeric targets remain open/blocked in the master contract. |
| `scripted_state_flags_variables_live` | 51 | Country/global/state variables and token/date/value flags, including mod/scripted state that persists as state. |

For a replay, these sources already support start/finish observation at the
recording cadence for any transition that changes these persistent fields. The
current production cadence is hourly-or-better, so "exact tick inside the hour"
is not currently a captured replay fact unless another production domain records
sub-hour state.

## Direct Focus, Research, And Decision Events

Current status: `ROOT_DISCOVERY_OPEN` in the master contract because no direct
start/finish/cancel hook domain is accepted.

Derivable now:

- Focus start: a `focus.current`/continuous focus appears or changes in domain
  12, and domain 45 can emit the ordered transition marker from frame deltas.
- Focus finish: a completed-focus token/count increases, and domain 45 can
  produce exact closure when compact focus rows include completed-token vectors.
- Focus switch/stop: a previous current focus disappears or changes in ordered
  domain 12 frames; domain 45 badges unresolved details rather than guessing.
- Research start/switch: `(tag_db_id, slot_index)` changes active technology in
  domain 13; domain 45 derives slot progress/speed/ETA rows from deltas.
- Research completion: completed-technology state changes in domain 13, with
  transition timing bounded by the polling cadence.
- Timed-decision start/stop/day-delta: domain 16 timed-decision rows and domain
  45 decision transition rows cover persisted active rows.
- Targeted timed country targets: the shared domain-45 contract accepts
  `CDecisionStatus+0xE8`, vtable `0x27D5038`, `record+0x20` country target for
  the exact matched country-target subset. Numeric targets are not accepted by
  the shared contract and remain open/blocked.

Residual missing semantics:

- Exact intra-hour tick of focus/research/decision starts and completions.
- Cancel versus completion disambiguation when a stop occurs without an exact
  completed-token delta in the same polled frame.
- Focus pause state outside the rejected `CFocusStatus+0x00..+0x7F` range.
- Numeric targets, unmatched targeted-timed transition rows, numeric target
  semantic classification, cooldown/day/state fields, and broader body fields.
- Direct engine callback identity and call arguments.

Replay relevance judgment:

- Exact intra-hour callback timing is not replay-relevant for the current
  hourly-or-better replay unless a UI requirement demands sub-hour event
  playback.
- Cancel versus complete is replay-relevant only when the resulting persistent
  state cannot disambiguate it. If the completed vector changes, the replay can
  show completion. If not, the replay should badge the stop/switch as inferred
  rather than claim a cancel hook.
- Decision body/cooldown residuals remain replay-relevant when they affect
  visible decision availability or target context and are not already expressed
  by domain 16/45/51.

Draft rejection text for direct hooks:

> Direct focus/research/decision hooks are rejected for the polling production
> architecture unless a future hook lane is explicitly authorized. The replay
> already observes start/finish/switch behavior from persisted focus, research,
> and decision deltas within one game hour. Remaining hook-only semantics are
> exact intra-hour callback timing and ambiguous cancel-versus-complete cases;
> these should remain inferred/badged unless a replay UI requirement proves they
> change visible playback.

Proof-backed rejected candidates from prior focus/decision work:

- `direct_decision_event_hook_by_named_on_action_string`: rejected because exact
  `on_decision`, `on_decision_taken`, `decision_taken`,
  `on_timed_decision_complete`, and `on_decision_timeout` string anchors were
  absent.
- `focus_completed_callback_names`: rejected because exact
  `on_focus_complete`, `on_focus_completed`, and
  `on_national_focus_complete` string anchors were absent.
- Static text-xref anchors `on_tech_research_completed`,
  `on_research_complete`, `on_research_complete_limit`,
  `complete_national_focus`, and `uncomplete_national_focus`: rejected because
  all five xrefs are registration/catalog arguments in function
  `0x82CB0..0x11896F`, loading the string into `r8`, a registration key into
  `edx`, a registry pointer into `rcx`, then calling `0x24AFC10`. They are not
  runtime callbacks and provide no object argument contract.

## On-Action And Effect Runtime State

Current status: `ROOT_DISCOVERY_OPEN`; no broad accepted event hook domain.

Derivable now:

- If an on-action/effect mutates country/global/state variables or flags, domain
  51 captures the persisted scripted state.
- If an effect changes one of the existing production domains, the replay sees
  the resulting state delta in that domain.
- If an effect schedules a delayed event, the pending delayed-event queue is a
  real persistent state target and remains discoverable by queue probes.

Missing:

- Transient call identity when the effect leaves no persistent state.
- Exact effect execution order inside one polling interval when multiple fields
  change before the next sample.

Rejection rule:

> A runtime on-action/effect candidate is rejected for polling capture when the
> candidate only identifies a transient call and no persistent state, queue row,
> or replay-visible durable delta exists. Such a candidate cannot be captured by
> the current production architecture without inline hooks.

No candidate is rejected merely because a fixture is currently empty. Empty
fixtures stay blocked, not disproven.

## Mission And Decision Scripted State

Current status: `ROOT_DISCOVERY_OPEN`.

Derivable now:

- Base decision visibility/status/timed rows are domain 16.
- Transition-depth and the accepted targeted-timed country-target subset are
  domain 45.
- Scripted mission/decision variables and flags that persist in country,
  global, or state scope are domain 51.

Genuinely missing:

- Separate mission-instance timer/container rows if the engine stores them
  outside `CDecisionStatus` and outside domain-51 variables/flags.
- Cooldown/body fields not validated in the current `CDecisionStatus` vectors.
- Same-frame body fields for unmatched transition rows.

New probe scope:

- The mission probe scans validated `CCountry` and `CDecisionStatus` roots for
  mission/decision-like vector containers not already covered by the known
  decision lists.
- It excludes domain-51 variable/flag roots from promotion because those are
  already production captured.

## Scripted GUI Open/State

Current status: `FIXTURE_BLOCKED` in the master contract through scripted-state
residual fixture contracts.

Replay relevance judgment:

- The server-side replay does not render the original client's scripted GUI
  panels.
- Replay-relevant scripted-GUI effects are the variables/flags or other
  persisted state changes they cause; those are already domain 51 when stored in
  country/global/state variables or flags.
- A GUI open/selected-client state is not a replay extraction target unless it
  affects future server-side game state and is persisted separately from domain
  51.
- `scripted_gui_random` remains fixture-blocked because it may be persistent
  causal state, but it must be validated with a positive fixture before any root
  is accepted.

Proof-backed rejected candidates:

- `game_defs/out/scripted_guis.json` as live GUI open-state root: rejected
  because it is a static definition map, not per-country live state.
- `countries.*.variables` as complete scripted-GUI causal/open state: rejected
  as complete root because variables cover GUI-bound values but not
  `scripted_gui_random` or open/selection state.
- Current `countries.*.scripted_gui_random` fixtures: blocked, not rejected,
  because recursive save inventory proves the raw leaf exists but current
  parsed fixtures have zero positive rows.

Draft rejection text:

> Scripted GUI open-panel state is rejected as non-replay-relevant for the
> server-side replay unless a positive fixture proves it drives persisted
> server-side state not already captured by variables/flags. Static GUI
> definitions and ordinary variable/flag changes are not separate open-state
> roots. `scripted_gui_random` remains a positive-fixture blocker, not a
> validated root and not a discarded candidate.

## Delayed Events And Event Queues

Current status: `FIXTURE_BLOCKED`, but this is the one event-like row here with
clear persistent state potential.

Why this remains a real candidate:

- Pending delayed country/state/global events are scheduled state that should be
  readable before firing.
- Melted saves can expose `countries.<id>.delayed_event` rows with event,
  countdown/date, originator, and scope fields once a positive fixture exists.
- Polling can capture this queue while paused or during normal hourly sampling;
  no inline hook is required.

Rejected proxies:

- `saved_event_target` as delayed-event validation proxy: rejected because it is
  top-level world saved-scope data, not per-country delayed-event queue rows.
- `save_oracle/output/events.json` `pending_events` in the current fixture:
  rejected only as the current positive fixture because it is empty, not as a
  general queue concept.

New probe scope:

- Build a save oracle from the 1943 IRE fixture and any controlled
  delayed-event fixture.
- Rediscover event/queue/mission/decision RTTI vtables from the live `hoi4.exe`
  image.
- Walk bounded candidate containers from `CGameState` and validated `CCountry`
  roots.
- First-pass live probing compares bounded container shape, owner labels, row
  counts, and RTTI interest against the save oracle. It does not decode event,
  originator/scope, or time fields.
- A second-pass decoder must use the first-pass candidate addresses to prove
  owner, event token/name/id, originator/scope, and countdown/due-time fields
  against the save oracle before any root can be promoted.

Promotion rule:

> Do not promote a delayed-event root until a positive save oracle and live
> candidate match on owner, event identity, originator/scope, and remaining
> time or due date. A zero-row fixture is blocked, not a rejection.
