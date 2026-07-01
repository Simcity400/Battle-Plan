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
  `replay_native/runs/focus_research_decisions_depth_live_current_roots_20260701_v3`.

## Remaining Fail-Closed Gaps

- Focus paused state is still unresolved. The current-build probe sampled
  `439` valid focus status rows, including `114` active/progress rows, but no
  `paused=true` ground truth. This lane does not synthesize a pause boolean and
  emits badge kind `1` with status flag bit `0`.
- Completed focus token is exact only when a completed-token list is present in
  the source payload. In compact delta streams, the lane can infer a completion
  token from the previous current focus when the completed count increases and
  the current focus clears or changes; that row is flagged as inferred. The
  integrated compact domain `45` proof currently has `0` exact completed-token
  rows and `1` inferred completed-token row because compact focus reads omit the
  completed vector.
- Decision targeted/cooldown record bodies are narrowed:
  - non-timed targeted/cooldown records at vtable `0x27D4F00` reject the
    timed-definition layout;
  - `decision_to_re_enable` at vtable `0x27D4E60` rejects the timed-definition
    layout;
  - targeted-timed records at vtable `0x27D5038` validate
    `record+0x10 -> CDecisionDef`, but target fields remain unproven.
  This lane records count/status transitions and active timed-decision rows, and
  leaves exact target/cooldown body detail fail-closed with badge kind `2` and
  status flag bit `1`.
- The v4 decision-body candidate gate rejects exact offsets that should not be
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
  The remaining target-body blocker is a same-frame oracle for records with
  vtable `0x27D5038` or static decision target-type metadata for a
  definition-aware variant layout, not another retry of these rejected offsets.

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
