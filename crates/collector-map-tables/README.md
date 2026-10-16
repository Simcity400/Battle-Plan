# collector-map-tables

The ordinary map part's single-file tables: the battle-sound list (BattleSounds: the battles the game's battle ambience holds, in the order its per-frame pass visits them), map facilities, naval bases and saved event targets. The host owns the part's dispatch, loans and table assembly.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-map-tables` and `cargo clippy -p collector-map-tables --all-targets -- -D warnings` for its synthetic checks.
