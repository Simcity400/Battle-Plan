# collector-slow-tables

The Slow job's single-file country tables: country factories, country tactic unlocks, faction theatres, logistics availability (from the collector-logistics balance observer) and modifier definitions. The host owns the Slow job's dispatch, loans and table assembly.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-slow-tables` and `cargo clippy -p collector-slow-tables --all-targets -- -D warnings` for its synthetic checks.

Doctrine state shares the CountryTacticUnlocks status-table baseline. Prepared
lock-free receiver marks select coalesced status/folder, track, bank, progress,
source-gain and equipment reads on the parked Slow worker; quiet hours only hand
off owned journals. Parent inventories settle before marked children, and lost
marks rebuild the baseline. Prepared gain operands and expiry counts defer row
reduction to Slow; grant append/load snapshots first settle earlier expiry counts.
Cost-add/consumption marks select the final native cost inventory, and effective
daily gain keeps its exact pre-store operand independently of unit-source gain.
Spool conversion follows no pointers and shares unchanged replay tables.

Cost rows retain terminal uses, names, factors, folders and ordinals. The worker
copies their marked inventory directly, without a private consumption reducer.
