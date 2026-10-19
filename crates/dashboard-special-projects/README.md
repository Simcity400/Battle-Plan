# dashboard-special-projects

The special projects product: each country's project facilities, progress,
prototypes and completions. Its behaviour is documented beside the code in
[products/special_projects.rs](src/products/special_projects.rs) and in the
[special projects widget guide](../../ui/src/widgets/special-projects/README.md).

This is a product crate above [dashboard-base](../dashboard-base/README.md), laid
out like [dashboard-politics-ledger](../dashboard-politics-ledger/README.md): the
module keeps its dashboard path, `products::special_projects`, and the code names what it uses
at the crates that own it ([dashboard-base](../dashboard-base/README.md),
[dashboard-text](../dashboard-text/README.md) and [dashboard-core](../dashboard-core/README.md)). The [dashboard](../dashboard/README.md)
re-exports the module at `dashboard::products::special_projects`, registers it,
and prepares and queries it in its pipeline.

The block pre-pass reads country presentation and trigger tables only when
the bound content has a scoped project name. Unscoped names read the project,
program, history, character, portrait, dictionary and country identity tables.
The extract finds its next completion deadline again only when history changes
or the previous deadline is reached; every due completion is still observed.

`cargo test -p dashboard-special-projects` builds and runs only this product's
tests. The `test-support` feature compiles its test oracles and instrumentation
for the dashboard's tests.

Current requests name an hour. Running cards before history reaches that hour use the frame, the preparation observer and its episode/presentation builders. Starts and completed/cancelled history remain preparing. The separate `current` reply cannot enter History lanes or expose a fabricated start date.

Current-card selections include exact holders witnessed in the requested frame, including countries not yet reached by the identity prefix.

Country identity comparisons follow recorded country field masks: balance-only
deltas and equal acquisitions keep the previous key projection. Snapshots,
identity edits and membership changes use the complete key comparison.

At-hour details in Current show scientist activity, skills, traits and dated daily injury recovery; dated facility lead/support assignments and delayed timers; pending reward choices; project variables and seeds; and authored project/reward outputs. Sliced projects require a continuous supplying owner read. Effects share the interpreter and the actual selected assignment; missing current scientist context is refused instead of choosing another scientist.

Current facility details derive the exact speed product in native tag and assignment order, with building, supply, acquired-resource, skill, country, state, trait and injury factors. The daily observation also exposes specialization breakthrough progress and lead/support contributions. Scientist roster visibility evaluates the retained role conjunction at its own source origin in character scope at its supplying date; empty conjunctions are true even for generated roles. Injury, visibility and breakthrough supplying observations are independent of program-roster availability. Chosen reward options show their recorded date and scope context; pending option outputs include effect-driven costs.

Current project cards restore progress from the independently retained value series before using the shared episode builder, including a newly selected holder before identity preparation.

Dictionary revisions follow recorded change evidence. A decoder handoff does not invent a dictionary read, while a recorded equal dictionary read still advances its revision.
ScientistDaily publishes injury, retained visibility-conjunction state and template origin at daily slot 4, projected from the existing scientist acquisition. Scientists and ScientistSkills keep their hourly roster. Off-slot reads skip the daily table; replay carries its last observation, and the detail panel labels its supplying date. Independent daily read refusals remain unavailable. No native read or census is added.
The supplying-read marker carries equal authenticated values between changes.
Unknowns and reactivation take ordinary observation. Structural project reads
compare held numeric values; publication materializes their endpoints after
the hour's changes are observed.
