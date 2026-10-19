# replay-derive

Playback calculations over recorded rows: air regions, attack, directed wars,
movement and arrival times, naval missions and dominance, power balances, subject
access and shared visibility branches.
`fixed_layout` and `modifier_registry` bind the executable metadata those
calculations require. Content facts remain supplied by the recorded stack's
readers; this crate introduces no content defaults.

Depends on [replay-data](../replay-data/README.md),
[replay-frame](../replay-frame/README.md) and the native libraries it actually
uses. It has no compressed recording, spool or filesystem publication dependency,
so calculations and recording I/O can compile independently.

Run `cargo test -p replay-derive`; installation-dependent tests authenticate
metadata from `BATTLE_PLAN_GAME_DIR`. Shared rules live in [AGENTS.md](../../AGENTS.md).
