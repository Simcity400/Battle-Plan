# collector-slow-tables

The Slow job's single-file country tables: country factories, country tactic unlocks, faction theatres, logistics availability (from the collector-logistics balance observer) and modifier definitions. The host owns the Slow job's dispatch, loans and table assembly.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-slow-tables` and `cargo clippy -p collector-slow-tables --all-targets -- -D warnings` for its synthetic checks.

Doctrine state shares the CountryTacticUnlocks status-table baseline. Native writer
callbacks copy only changed statuses, folders, tracks or grant lists into owned,
coalesced journals. Effective daily gain is copied from the native store operands;
unit-source daily gain remains separate. Folder resets read only the folder
inventory; load completion also reads the other inventories. Each inventory
header is acquired once by its reader. Ordinary Slow jobs validate the fixed
observer epochs and swap prepared journals without reading doctrine objects or
growing storage. Baselines and structural insertions prepare journals for every
existing frame buffer. Grant expiry advances one carried country counter; grant rows keep signed
days and a counter anchor, so scalar expiry publishes no grant payload. Spool
conversion applies events and shares unchanged replay tables. Lost observation or a failed conversion refuses that hour and rebuilds the baseline at the next capture.

Cost consumption publishes prepared scalar occurrence events, with constant-size
folder authentication and a native result-count witness. Immutable reduction
names and folders remain owned; the spool worker derives use decrements,
removals and ordinal compaction. The first cleanup stays ordered, then folder
occurrences coalesce until an owner snapshot supersedes them. This adds no
inventory walk or allocation to the consumption hook or ordinary hourly job.
