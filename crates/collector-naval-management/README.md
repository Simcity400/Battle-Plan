# collector-naval-management

Stored naval repair, refit and reinforcement policy. Existing worker censuses supply authenticated
countries, fleets, task forces, ships and bases. Native writer notifications select changed objects;
quiet objects supply their held values without another native read. Reusable native list storage
follows each changed object's live demand. Replay snapshots are shared while their values hold.

The writer observer, capture and replay consumers preserve ordered composition preferences, repair
queues and full work-order identities. Content labels resolve through the recording's content stack.
Reader failures remain named table refusals. There is no hourly polling fallback when observation
cannot certify carry.

Rejoin destinations carry the raw source reference. Its current validity is derived from the
existing hourly task-force census in playback and the save projection: transferring a repair
detachment to another fleet can drop its parent relationship while retaining that reference,
so destination retirement must not depend on a later source notification.

Ship refit links likewise retain the stored full work-order identity. Native resolution only
selects progress acquisition; a missing work order never zeroes the captured link. Existing
work-order retirement removes its hourly NavalRefitProgress row and live receiver pointer.
Playback and oracle share the replay row's resolution rule over those complete hourly
production-line rows; daily ProductionLines supplies last-observed metadata only.

The collector's cross-family hook ownership audit consumes this family's test-support patch
inventory: 47 owned Tap spans and 19 method entries. The same native descriptor inventory also supports the attach-thread constructor audit in ordinary builds. Main's task-force destructor constant describes
the layout; this family owns its sole patch. Construction failures name the site, while batch
publication failures retain the family, candidate site names and original failure reason.

`hook_timing.families.naval_management_changes` counts every callback and samples with the
shared timing policy. Its mixed bucket includes taps and method detours; method originals
are timed separately. No extra hook or native read is needed for timing.

First-hour admission tests visit populated synthetic countries, fleets, task forces, ships and
bases through production capture sessions and snapshot handoff, then decode the spool's encoded
frames. They compare every column of all nine outputs at off-slot hours and after recovery.
Recovery changes native values at the same identities without writer marks, requiring epoch
reseeding. Lost observation produces named refusals before populated readmission. The row
columns are primitive values; native read failures refuse their table by name.

Follow [AGENTS.md](../../AGENTS.md), the [collector guide](../collector/README.md), and the
[recorder fresh-save oracle](../recorder/README.md#fresh-save-oracle).
