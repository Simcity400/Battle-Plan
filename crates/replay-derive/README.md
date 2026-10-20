# replay-derive

Playback calculations over recorded rows: air regions, mission counts, sortie
efficiency/aircraft counts and their base/country joins, attack, directed wars,
movement and arrival times, naval missions and dominance, sparse mine levels and
regional control/share, power balances, subject access, port relations, ordered
naval repair selection and shared visibility branches.
`fixed_layout` and `modifier_registry` bind the executable metadata those
calculations require. Content facts remain supplied by the recorded stack's
readers; this crate introduces no content defaults. `aircraft_count` preserves
fixed5 wrapping, per-product truncation and the ordinary final clamp; recorded
efficiency overrides replace the law unclamped. `air_mission_sources` owns the
registry and volunteer mission-country joins. Viewer paths re-export them; content
define loading stays in the callers.

Depends on [replay-data](../replay-data/README.md),
[replay-frame](../replay-frame/README.md) and the native libraries it actually
uses. It has no compressed recording, spool or filesystem publication dependency,
so calculations and recording I/O can compile independently.

Run `cargo test -p replay-derive`; installation-dependent tests authenticate
metadata from `BATTLE_PLAN_GAME_DIR`. Shared rules live in [AGENTS.md](../../AGENTS.md).

`naval_repairs::Sources` shares demand/capacity and assigned-dockyard selection
through one ordered eligibility traversal. Assigned selection preserves queue
ordinals and full ship handles, including repeats, and stops before demanding
an unused suffix. Text control markers remain with viewer presentation.
