# Roots And Integration Notes

## Exact Roots Already Usable

These roots are already present in the shared DLL collector and are the only
sources used by the lane-local proof stack:

- Focus:
  - `CCountry+0x1370 -> CFocusTree` for `focus_tree`.
  - `CCountry+0x1380 -> CFocusStatus`.
  - `CFocusStatus+0x10` current national focus pointer.
  - `CFocusStatus+0x18` current continuous focus pointer.
  - `CFocusStatus+0x38` progress days fixed point.
  - `CFocusStatus+0x40/+0x48/+0x4C` completed-focus vector data/capacity/size.
- Research:
  - `CCountry+0xF60 -> CTechnologyStatus`.
  - `CTechnologyStatus+0xA0/+0xA8/+0xAC` research-slot vector data/capacity/count.
  - `CResearchSlot+0x08` active tech handle, `+0x10` owner status,
    `+0x18` active `CTechnology`, and `+0x40` slot index.
  - `CResearchSlot+0x18 -> CTechnology+0x198` moving live progress numerator.
  - `CTechnology+0x160 -> CTechnologyTemplate+0x3E0`, scaled by the current
    build's base-cost scalar, for base required points.
- Decisions:
  - `CCountry+0xFA0 -> CDecisionStatus`.
  - `CDecisionStatus+0x08` owner country back-pointer.
  - Compact-vector headers at `CDecisionStatus+0x40`, `+0x88`, `+0xA0`,
    `+0xE8`, and `+0x130` for cooldown/target/timed status counts.
  - Active `CTimedDecision` rows from `CDecisionStatus+0xA0` with
    definition pointer, days remaining, and state enum.

## Current-Build Probe

- Current 870EBF live root evidence is in `CURRENT_BUILD_ROOTS.md` and proof
  output under
  `replay_native/lanes/focus_research_decisions_depth_live/proofs/current_roots_20260701_v5`.

## Remaining Fail-Closed Gaps

- Focus paused state is still unresolved. The current-build probe sampled
  `439` valid focus status rows, including `114` active/progress rows, but no
  `paused=true` ground truth. The v2 focus pause candidate gate rejects the full
  `CFocusStatus+0x20..+0x37` byte window as the paused-boolean home:
  `13` direct byte-shape rejections plus `11` structural qword-overlap
  rejections. The wider v2 focus pause gate scanned
  `CFocusStatus+0x00..+0x7F`, excluding that prior window, and rejects `100`
  additional outside-window byte candidates. The static v2 gap gate rejects the
  final `0x3C..0x3F` bytes by proving `2` CFocusStatus vtable methods use
  qword-sized access at `+0x38` and `0` scanned methods access the gap directly.
  The bounded fixture watcher observed `2` unique game hours and records the
  required paused=true oracle contract for any future root outside
  `CFocusStatus`. This lane does not synthesize a pause boolean and emits badge
  kind `1` with status flag bit `0`.
- Completed focus token closure is a validated slice when compact focus source
  rows carry the completed-token vector. The exact-completed production proof
  has `1` exact completed-token row, `0` inferred rows, `0` unresolved rows,
  zero skipped hours, and verified `.battleplan` output. Without the
  orchestrator hunk in `SHARED_RESIDUAL_PATCH_PLAN.md`, compact focus rows can
  still only infer a completion token from previous-current deltas.
- Decision targeted/cooldown record bodies are narrowed:
  - non-timed targeted/cooldown records at vtable `0x27D4F00` reject the
    timed-definition layout;
  - `decision_to_re_enable` at vtable `0x27D4E60` rejects the timed-definition
    layout;
  - targeted-timed records at vtable `0x27D5038` validate
    `record+0x10 -> CDecisionDef`;
  - targeted-timed `record+0x20` validates as the country target `tag_db_id`
    for the exact save/live matched country-target subset: `1061` live rows
    across `180` decision tokens.
  - targeted-timed `record+0x24` validates as the numeric target id for the
    exact save/live matched `target=<number>` subset: `2285` rows, with `4`
    live-only and `3` save-only transition rows.
  - lane-local domain 45 compact schema/decoder/gate coverage is proved for
    those rows using `decision_target_count` plus target rows carrying owner tag,
    target id, source kind, provenance flags, and decision token.
  This lane records count/status transitions and active timed-decision rows, and
  leaves unmatched transition rows, numeric target semantic classification, and
  exact cooldown/day/state body detail fail-closed with badge kind `2` and
  status flag bit `1`.
- The v2 decision-body candidate gate rejects exact offsets that should not be
  retried as production target fields on build `870ebf`:
  - timed-body `record+0x10 -> CDecisionDef` for
    `decisions_taken_bucket`, `active_targeted_decision`, and
    `decision_to_re_enable`;
  - `active_targeted_decision+0x54/+0x78` and
    `decisions_taken_bucket+0x54/+0x78` as exact target tags;
  - `active_targeted_timed_decision+0x50` as an exact target tag;
  - `active_targeted_timed_decision+0x20/+0x24/+0x28/+0x30/+0x34/+0x5C/+0x60/+0x64/+0x70/+0x74/+0x78/+0x7C`
    as universal exact country-target tag fields;
  - `active_targeted_timed_decision+0x60/+0x70` as universal exact
    country-target pointer fields.
  The v3 full-decision variant gate also rejects `49` definition-local country-
  tag slices in the scanned `record+0x20..+0x7C` i32 window. It leaves `50`
  definition-local candidates as `CANDIDATE_NOT_PRODUCTION` unless covered by
  the save-oracle target proofs. The save-oracle proofs validate `record+0x20`
  for exact matched country-target rows and `record+0x24` for exact matched
  numeric-target rows; the remaining target-body blocker is a same-frame oracle
  for unmatched transition rows, numeric target semantic classification, and
  cooldown/day/state encodings, not another retry of the rejected offsets.
- The event-hook candidate gate rejects the named decision-event string route
  and the named focus-completion callback string route for build `870ebf`.
  The static-contract gate rejects all `5` remaining string/xref anchors as
  registration/catalog arguments rather than runtime callbacks. The rejected
  anchors are `on_tech_research_completed`, `on_research_complete`,
  `on_research_complete_limit`, `complete_national_focus`, and
  `uncomplete_national_focus`. Each xref lives in function
  `0x82CB0..0x11896F`, which has `10697` string references, loads the string
  into `r8`, loads a registration key into `edx`, sets a global registry
  pointer in `rcx`, and calls `0x24AFC10`. Direct event hooks remain
  fail-closed with badge kind `3` and status flag bit `2` until a non-
  registration callback root with a live argument contract is validated.

## Badge Gates

The lane-local compact payload now includes explicit badge rows. For the current
build, the decoder fails closed if a required fail-closed status flag is omitted
or if a fail-closed status flag lacks the matching badge row. Badge
source/evidence/detail tokens are exact, not advisory:

- kind `1`: focus pause flag unavailable, current-build probed, no
  `paused=true` ground truth.
- kind `2`: targeted/cooldown decision target fields unavailable, current-build
  probed, timed-definition layout rejected for non-timed bodies.
- kind `3`: direct engine event hooks unavailable, replay transitions derived
  deterministically from compact frame deltas.

## Final Patch Plan

See `INTEGRATION_PATCH_PLAN.md` for the exact DLL/Rust patch plan. No shared
entrypoint changes are made by this lane until the orchestrator grants an
integration slot.
