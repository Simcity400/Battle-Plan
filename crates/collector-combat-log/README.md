# collector-combat-log

Combat log capture: the kept CombatLog rows of the loaded session, read beside the daily slice, and the observer of the native log prune that withdraws pruned rows. The host owns dispatch, loans and table assembly.

Prune status reports `observe_mean_ns` from every observed call, beside total
and maximum wall time. It includes scheduling delays but does not extrapolate
rare slow calls through the sampled hook timer.

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

Event storage also uses the existing worker-only active-combat operand census,
including its aggregate equipment entries. An empty retained-history seed must
not leave active writers with a single slot. The worker grows a drained lane
before writers resume; pending events defer replacement until the existing drain.
Status reports installed capacity beside complete required-entry demand. Genuine
loss still revokes the certificate until an explicit full seed; no reread fallback
or hidden loss recovery is added.

Run `cargo test -p collector-combat-log` and `cargo clippy -p collector-combat-log --all-targets -- -D warnings` for its synthetic checks.

`observer_projection` assigns active-observer occurrence labels from the existing
encoder baseline on the conversion lane. Collection insertion cannot relabel
other kinds or activity parents. Native acquisition, pool admission, duplicates,
refusals and hourly cadence are unchanged; no process-global identity cache is
kept. A missing baseline restarts labels from the complete current sample.
