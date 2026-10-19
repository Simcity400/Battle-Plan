# collector-combat-log

Combat log capture: the kept CombatLog rows of the loaded session, read beside the daily slice, and the observer of the native log prune that withdraws pruned rows. The host owns dispatch, loans and table assembly.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

The retained-history [mutation and lifetime certificate](CERTIFICATE.md) covers
the allocation-free native event producer and its conversion reducer. Events stage
only reached months, template entries, references and settled battle records;
pruning removes prefixes and reference release certifies reusable battle slots.
Acquisition transfers reusable owned batches and settles only demanded slot
counts. It neither validates retained history nor projects replay rows. Ordered
conversion edits keyed held rows incrementally; complete encoder inputs are
materialised there, outside the hourly callback's acquisition join. Sequence
gaps require a full seed. A quiet batch touches no held rows.

Run `cargo test -p collector-combat-log` and `cargo clippy -p collector-combat-log --all-targets -- -D warnings` for its synthetic checks.
