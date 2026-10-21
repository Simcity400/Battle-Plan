# collector-map-tables

The ordinary map part's single-file tables: the battle-sound list (BattleSounds: the battles the game's battle ambience holds, in the order its per-frame pass visits them), map facilities, naval bases and saved event targets. The host owns the part's dispatch, loans and table assembly.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-map-tables` and `cargo clippy -p collector-map-tables --all-targets -- -D warnings` for its synthetic checks.

The naval-base census also supplies authenticated bases to the naval-management observer. It publishes shared repair-priority, dockyard-allocation and ordered-queue tables beside NavalBases; unchanged bases cause no additional repair read.

Recorder acquisition defers changed repair snapshot collection/sorting to FIFO
decode through the naval-management store loan. The base and country-text reads
and all receiver admission remain in their original acquisition order; ordinary
direct captures still settle synchronously. Decode requires no native reader.
