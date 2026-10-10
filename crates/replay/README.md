# replay

The shared recording contract: rows, tables, frames, sample provenance, branches, spools and `.battleplan` containers. Recording and playback agree here; consumers must not invent meanings for the bytes.

## Sources of truth

| File | Responsibility |
|---|---|
| [lib.rs](src/lib.rs), [format.rs](src/format.rs) | Current writer version and format admission. |
| [rows.rs](src/rows.rs) | `TableId`, `define_tables!`, typed row/table dispatch, `RowCodec` and `Recorded`. |
| [rows/compatibility.rs](src/rows/compatibility.rs) | Proven wire/meaning history, lifetimes, appended fields and nested records. |
| [cadence.rs](src/cadence.rs), [sliced.rs](src/sliced.rs) | Scheduling, carry rules, owner slices and read hours. |
| [codec.rs](src/codec.rs), [value.rs](src/value.rs), [fixed_point.rs](src/fixed_point.rs) | Bounded encoding, native representations and exact arithmetic. |
| [frame.rs](src/frame.rs), [frame/](src/frame/) | Updates, baselines, keyframes, sample origins and frame wire encoding. |
| [container.rs](src/container.rs), [branches.rs](src/branches.rs) | Indexed compressed blocks, seek/read APIs and branch identity. |
| [spool.rs](src/spool.rs), [witness.rs](src/witness.rs) | Versioned CRC records, durability and terminal failure evidence. |
| [identity.rs](src/identity.rs), [fixed_layout.rs](src/fixed_layout.rs) | Content identity and consumers of fixed modifier/token layouts. |
| [acceptance.rs](src/acceptance.rs) | Synthetic capture/recovery campaign; `DOMAIN_TABLES` and direct fixtures cover every table exactly once. |

Code owns versions, TableIds, field counts, lifetimes and cadence; keep catalogs out of prose. Shared rules live in [AGENTS.md](../../AGENTS.md).

## Values, availability and freshness

Preserve native IDs, signed quantities, fixed-point words, ordered occurrences and stored text losslessly. Encoding is explicit little-endian, never Rust memory layout. Reject malformed, out-of-range or noncanonical input. Compression/deltas may reduce size, not precision; collector-reachable buffers use engine's checked growth.

Resolve raw country references through the frame's `CountryLinks`; slots can be reused. Preserve raw IDs and leave unresolved links unknown. Stored names obey the recording client's language; token spelling cannot reconstruct missing cached text.

A frame belongs to one hour/branch. Missing, truncated and unavailable tables are not complete empty tables. Losing a complete baseline requires a snapshot before deltas resume. Equality and freshness differ:

- Successful equal acquisition can encode `Unchanged` while advancing sample origin.
- Carried tables retain their read hour. Failed, truncated or skipped acquisition is never a fresh complete sample.
- Sliced tables replace only acquired owners and can mix read hours. Use `SliceReads`/`sliced::read_hour`, not the containing frame's hour.
- Startup/load/branch/gap transitions preserve full-refresh/baseline rules. A self-contained keyframe encodes held rows, including carried ones; it is not a full acquisition.

Preserve these distinctions through inspection, analytics and save joins. Sibling timing is not freshness proof; scheduling/merge rules live in [cadence.rs](src/cadence.rs) and [sliced.rs](src/sliced.rs).

`SharedView::sampled_at` certifies complete acquisitions with that table's own nonzero read timing. Equal timed reads advance it without copying rows; unchanged carries retain it. Untimed replacement snapshots/deltas, carried keyframes, failures and local mutation leave it unknown. Uncharged shared reads require their producer-specific provenance. Row equality and allocation identity exclude the sample hour.

A witnessed native world invalidation splits branch continuity even when stored clock and load timestamp values agree. The collector supplies this fact to `BranchTracker`; the existing branch controls preserve it without replacing native clock values.

`FrameDecoder::for_tables` fixes its selection for its lifetime; omitted tables are unavailable. `Replay::read_block_selected` validates all updates before discarding unselected ones. Both keep ordinary baseline/recovery rules. `Replay::read_block_parallel` preserves sequential frames/first error and requires caller-owned spare workers; collector and recorder read sequentially.

## Row contracts

Keep keys, field order, native evidence, census/coverage and save joins beside each row. Start with [rows.rs](src/rows.rs), [extended_rows.rs](src/extended_rows.rs) and its [domain modules](src/extended_rows/). Dedicated contracts include [combat_rows.rs](src/combat_rows.rs), [land_rows.rs](src/land_rows.rs), [modifier_breakdown_rows.rs](src/modifier_breakdown_rows.rs), [naval_mission_rows.rs](src/naval_mission_rows.rs), [production_modifier_rows.rs](src/production_modifier_rows.rs), [politics_rows.rs](src/politics_rows.rs), [peace_rows.rs](src/peace_rows.rs) and [event_feed_rows.rs](src/event_feed_rows.rs). Playback arithmetic lives in [eta_derive.rs](src/eta_derive.rs) and [naval_mission_derive.rs](src/naval_mission_derive.rs).

The [weather boundary packet](src/extended_rows/naval_weather.rs) retains native snow/mud planes and refresh operands. Its refresh identity is an equality witness, never playback animation time. Historical absence remains distinct from a known empty optional packet.

Event feed actions keep event dates separate from supplying reads; incomplete history can retain known actions without proving absence. Rocket totals survive site removal with independent spending/delivery coverage; coverage-only rows add no amounts. [Air events](src/extended_rows/air_ledger.rs) retain signed corrections and native dates; zero health change is not a bombing event.

Identity stores ordered layer hashes and coverage policy, not assets or machine paths. [installed-content](../installed-content/README.md) resolves content. Older supported policies retain their verification roots; newer-only reads produce uncovered-read notes, and unsupported policies are refused.

Finalization uses two streaming spool passes and compresses blocks before the shared decoder size bound. Split hours become self-contained keyframes with original metrics and sample hours; an oversized frame or a split with a truncated table is refused by name. Allocation and I/O failures abort finalization and retain the spool. Repeated finalization compares complete decoded values, availability and metrics before leaving an existing replay unchanged.

## Compatibility and recovery

Update affected readers, writers and consumers together when bytes or recorded meanings change. A branch's format is provisional **main + 1**. Allocate new TableIds densely after main's catalog; reconcile/renumber provisional additions at merge. Never reuse or shift published IDs, including retired entries.

Writers emit only the current format. Older recordings still open when today's readers decode unchanged contracts correctly within the proven [compatibility history](src/rows/compatibility.rs). Refuse changed layouts/meanings at their affected scope: a table becomes unavailable by name; incompatible file headers/framing refuse the file. Age alone must not reject an unchanged contract. Add no migrations, translations or old row decoders.

Appended fields decode through the source prefix and delta-mask width. `Recorded::NotRecorded` differs from native empty, zero or absence, including `Recorded::Value(None)`. Only dependent features receive named, counted refusals; readable prefixes remain usable. Opaque nested records retain source-format context even when the parent field count is unchanged. Inspection counts recorded bytes and nested gaps without rewriting rows.

Older air-history, naval-base and construction-output prefixes remain readable with their absent scenario channels and native oracle witnesses reported as field gaps. The changed technology occurrence keys require named whole-table refusals for research inventories and technology definitions.

Lifetimes determine timing entries/updates. Absent new tables are not recorded, not empty; retired tables retain IDs/framing for named discard. Append requires current format. Check each spool record's version/contract before decoding; incompatible old spools stay on disk, refused for finalization/append and never re-encoded. Recovery keeps CRC-valid prefixes, preserves existing good files and never joins branches through probable parentage.

A validated terminal failure produces a replay prefix with a visible `Partial` control. Only known attributable unrecorded attempted hours become `Gap`s; missing context invents no hour, clock or branch. A prepared identity-only witness is not failure evidence. Validate terminal identity/context and committed/pending spool bounds in [witness.rs](src/witness.rs) and [recorder recovery](../recorder/src/replay_io.rs). Partial notices cannot make coverage complete.

Spool writes stay ordered. Periodic durability may run off-thread; segment creation, forced boundaries and detach are synchronous barriers. Check pending errors before writes/idle polls and join workers before destroying sinks. Publish complete-record bounds before durability; promote baselines only after complete encoding. Collector allocation-failure diagnostics must not allocate; see [codec.rs](src/codec.rs) and [engine allocation](../engine/src/allocation.rs).

[Transport packets](src/extended_rows/transport_mission_inputs.rs) retain the physical navy domain, nullable ledgers and authenticated receivers. Same-hour numerical views preserve independent failures and native short circuits; convoy stock, spotting, damage, air normalization and carrier fill use ordered recorded inputs and content-defined operands. Historical absence stays distinct from current refusal and native null.

## Naval modifier breakdowns

Naval trees retain native source/child order, formatting gates and root values while pruning zero subtrees. Each requested accumulator is walked once at native depth -1; failed roots/arenas withhold trees rather than publish partial ones. Labels obey recording language. These trees have no save counterpart; evidence/reuse limits live in [modifier_breakdown_rows.rs](src/modifier_breakdown_rows.rs) and [the engine reader](../engine/src/naval_ship_modifier_breakdown.rs).

## Capture exceptions and limitations

- `DivisionEtaInputs` is hourly with approved five-hour operand reads. Use `sample_hour`/`valid_at`; seeking needs no earlier witness state. See [movement rows](src/extended_rows/movement.rs).
- `RaidSpeedInputs` reads transport speed separately from daily variant stats because flying raids reach it hourly: the exception to one reader per stored value. See [raid rows](src/extended_rows/raids_projects.rs).
- Native war groups have no native group ID or participant join/leave dates. Derived continuity must preserve unobserved stretches; see [war rows](src/extended_rows/war_overview.rs) and [the wars product](../dashboard/src/products/wars/README.md).
- Cumulative output starts with branch observation, not line creation. Respect observation start, exactness and coverage; a linked partner's unknown share is never zero. See [economy rows](src/extended_rows/economy_construction.rs).
- Event application time can differ from frame time. Complete empty batches certify only their capture scope; missing batches/partial intervals/lost drains never prove zero. Domain rows own these contracts.
- [fixed_layout.rs](src/fixed_layout.rs) owns fixed-key/token families and named refusals. Verify complete recorded dictionaries and registered spelling/layout; dynamic keys remain content-driven.

Fixed modifier-name authentication requires initialized executable metadata and reports a family-scoped refusal when it is unavailable. Raw recording histories remain readable. Its name-dependent tests initialize from `BATTLE_PLAN_GAME_DIR`; format and index-membership fixtures use synthetic data without an installation.

Research inventories and definitions key each native template occurrence by database index. Slot inputs append exact occurrence identity with historical absence retained by `Recorded`; equal handles cannot establish assignment continuity. The inventory also retains the native serializer's applied-bonus-count inclusion operand.

## Add or change a table

1. Document native key, exact representations, ordering, observation boundary, completeness and refusals beside the row. Add it to its domain module with `RowCodec`/`row_codec!`, existing lossless value codecs and validation.
2. Register it in `define_tables!` in [rows.rs](src/rows.rs), including dispatch/size handling. Keep provisional IDs dense. Appended fields retain keys, prefix order/kinds and historical mask framing; use `Recorded` for historical absence.
3. Update [rows/compatibility.rs](src/rows/compatibility.rs) for lifetimes, fields, nested records and layout/meaning changes, and the current writer version referenced by [lib.rs](src/lib.rs). Retain retirement framing; add no legacy decoders.
4. Wire scheduling/carry in [cadence.rs](src/cadence.rs), sliced owner reads/merges in [sliced.rs](src/sliced.rs), and fixed-key/token dependencies in [fixed_layout.rs](src/fixed_layout.rs).
5. Wire acquisition/capacities/conversion in [collector](../collector/README.md) and affected frame/container/spool handling, [recorder](../recorder/README.md), [viewer](../viewer/README.md) and [dashboard](../dashboard/README.md) consumers together. New inputs need a save comparison or named limitation.
6. Add synthetic fixtures and fault-campaign coverage in [acceptance.rs](src/acceptance.rs). `DOMAIN_TABLES` plus direct fixtures must cover each table exactly once. Exercise snapshots/deltas, unchanged/carried freshness, missing/truncated reads, baseline recovery, malformed input and readable older prefixes as applicable.
7. Run scoped [AGENTS.md checks](../../AGENTS.md) and relevant [fresh-save/load and smoke checks](../recorder/README.md#inspect-and-compare). Reconcile provisional version/IDs against current main at merge.

## Checking changes

Use existing round-trip, delta/baseline, compatibility, provenance, branch, recovery and fault-campaign tests with synthetic fixtures. Exercise affected consumers together; decoding success does not certify capture or display fidelity. Follow [AGENTS.md](../../AGENTS.md) and the [live-check guide](../recorder/README.md#live-checks-and-performance). Documentation-only changes need link/diff checks.
