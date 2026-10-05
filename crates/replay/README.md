# replay

The shared recording contract. This crate owns tables, rows, frames, cadence, sample provenance, spool records, replay containers, branches and codecs. Recording and playback must agree here; consumers must not invent their own interpretation of the bytes.

## Sources of truth

Start with [src](src/), especially [format.rs](src/format.rs) for format support, [rows.rs](src/rows.rs) for the table catalog, [cadence.rs](src/cadence.rs) for scheduling and [acceptance.rs](src/acceptance.rs) for admission. Shared development policy lives in [AGENTS.md](../../AGENTS.md#keep-these-guarantees).

Rows preserve engine-native identifiers and numeric representations. Encoding is explicit and lossless. Reject malformed, out-of-range or noncanonical inputs rather than silently coercing them. Compression and deltas may save space; reduced precision may not.

## Values, availability and freshness

Country references in rows are not necessarily canonical slot numbers. Resolve raw references through the frame's `CountryLinks`; physical country slots can be reused. Preserve the recorded raw ID and do not turn an unresolved link into a guess about the current country.

A frame belongs to one hour on one branch. Tables can be snapshots, deltas, unchanged, truncated or unavailable. Missing data is not an empty successful table. After losing a complete baseline, a later successful table needs a snapshot before deltas can resume.

Value equality and sample freshness are separate:

- A fresh successful acquisition may equal the previous values and encode as `Unchanged`, while advancing its sample origin.
- A carried table retains the original sample hour, even though it appears in a later frame.
- Failed, truncated or skipped acquisition must not be marked as a fresh complete sample.
- Sliced tables (`Cadence::Sliced`) are read one owner slice per hour; owners are native country IDs, canonical country slots or native state IDs, one kind per family. Each hour replaces only the rows of the owners read at that hour, so one complete table mixes rows read at different hours. SliceReads holds each owner's read hour per table (`sliced::read_hour`); rows carry no hour, so an owner's unchanged rows stay unchanged when it is read again. The owner's read hour, never the frame's hour, is when its values were true. The keyed-table merge contract lives beside [TechnologyDefinitionRow](src/extended_rows/politics_research.rs) and in [sliced.rs](src/sliced.rs).

Keep those distinctions through encoding, decoding, inspection, analytics and oracle joins. Do not infer freshness from a sibling table or the current frame's hour. Startup and branch/load transitions retain the full-refresh behavior required by the format. Keyframes stay self-contained, but a recorder may build them from the complete rows it already holds instead of reading every table again: a carried table in a keyframe is a snapshot of its last read, not a new sample, and its table cost is zero.

`Replay::read_block_parallel` decodes a block's frame payloads on the caller's worker threads once their boundaries are known. It returns the same frames and the same first error as `read_block`. Only a caller that owns spare cores uses it, such as the viewer's playback read-ahead; the collector and the recorder read sequentially.

Consumers that need only a few tables can use `FrameDecoder::for_tables`. It applies the same baseline and recovery rules to those tables and reports omitted tables as unavailable. The selection is fixed for the decoder lifetime; ordinary `FrameDecoder::new` still reconstructs every table. `Replay::read_block_selected` complements this by retaining only selected updates during parsing. All updates still go through the same decoder and validation before unselected ones are discarded; frame ordering, bounds, keyframe and compression checks remain shared with the full reader. The decompression buffer reserves the bounded size from the block index once. Neither selection API changes the recording format.

## Row contracts

Field contracts, native representations, census rules and save joins live beside the rows:

| Domain | Row documentation |
|---|---|
| Country appearance, counters, portraits and local viewpoint | [map_presentation.rs](src/extended_rows/map_presentation.rs) |
| Parties, script stores, saved-global targets, research and stability | [politics_research.rs](src/extended_rows/politics_research.rs) |
| Characters, roles, advisors and agencies | [characters_intelligence.rs](src/extended_rows/characters_intelligence.rs) |
| Fuel, factories, logistics, production, construction, variants, industrial organisations, Combat Log equipment and lend-lease | [economy_construction.rs](src/extended_rows/economy_construction.rs) |
| Country and state manpower, and faction identity | [rows.rs](src/rows.rs) |
| Division census, ETA, invasion preparation and volunteers | [movement.rs](src/extended_rows/movement.rs) |
| Raids and equipment pools | [raids_projects.rs](src/extended_rows/raids_projects.rs) |
| Task-force motion, regions and combat/transfer vectors | [naval_weather.rs](src/extended_rows/naval_weather.rs) |
| Naval combat sides and air-base wing occurrences | [combat_rows.rs](src/combat_rows.rs) |
| Live naval modifier child trees | [modifier_breakdown_rows.rs](src/modifier_breakdown_rows.rs) |
| Naval mission range, ledgers and held modifier slices | [naval_mission_rows.rs](src/naval_mission_rows.rs) |

NavalCombatDetails is hourly and live-combat-only. Each plane records the sub-unit indices of its ordered all-wing handles; the viewer derives the distinct short names in its own language. The air-base name comes from the recorded state display inputs or the host combat ship's stored name. A historical wing that no longer resolves is refused by handle. Each side carries persistent weapon damage history and baseline health from CNavalCombatant. The optional `custom_sea` bytes are ordered little-endian signed words, one per effective strategic-template modifier. `custom_sea_error` is 0 for a complete vector and 1 for a guarded read failure, in which case `custom_sea` is absent; an empty present vector means the template declared no custom modifiers. The custom grid is sampled only for a live combat's region and at that combat's hourly cadence. The viewer resolves modifier names and descriptions from the effective content stack. The UI-idler paused bit is a viewer decision and has no replay field. NavalCombatShips adds the state-8 speed base and terrain movement operands; the nested NavalShipModifierSource layout also occurs in NavalCombatResultDetails.

## Naval modifier breakdowns

NavalModifierBreakdowns captures only accumulator/key trees reached by live naval hovers, hourly. Each distinct accumulator is walked once for all requested keys at native depth -1. Pack-owned tree arenas start empty and grow off-thread after shortages, which withhold the tree table. Projection prunes zero subtrees while preserving root values and native formatting gates. Stable accumulator/key rows keep ordered combat/side/member uses separate from nodes, so unchanged trees carry through replay deltas. The viewer borrows trees and uses in native source and child order. Names obey the recording-language rule; numeric lines whose native formatter omits names remain available in another viewer language. Trees, contributions and display labels are runtime-only, with no text-save counterpart. Capture-on-change is deferred to the performance round. Field contracts and native evidence live in the [row docs](src/modifier_breakdown_rows.rs) and [engine reader](../engine/src/naval_ship_modifier_breakdown.rs).

## Capture exceptions and limitations

`DivisionEtaInputs` publishes a complete moving-division table each hour, including carried rows. Its per-row `sample_hour` is the latest full operand read; consumers use `DivisionEtaInputsRow::valid_at` before deriving. A seek needs no earlier witness state. This owner-approved five-hour operand cadence is an exception inside the hourly table, not a new table cadence. See the [collector limitation](../collector/README.md) and [viewer fidelity limits](../viewer/README.md#fidelity-limits).

RaidSpeedInputs' `transport_speeds` are the one reader of variant stat 0x3e besides VariantStats, a documented exception to one reader per stored value: VariantStats is daily, while the game reads the stat every hour a raid flies and its writers are not proven daily-only.

RaidEquipment's save writes only the entries and nukes; the oracle compares each total exactly with its archetype's saved entries summed by `VariantDefinitions.archetype_token`, and states a limitation only where the total, a contributing entry or their sum reaches the clamp bound `L` (`±0x53E2D620FB40` raw, about 9.2e8 units), which `AddVariantQuantity` applies to a total and an entry separately. The one case it cannot see is a clamp that later unwound: adding ¾L to two variants of one archetype and removing one leaves a native total of ¼L below the entries' ¾L with nothing at the bound. That needs a pool that once held at least L of one archetype, and would surface as a reported difference.

AdvisorTokens' reconstructed save set is exact at a load hour; at any other hour a recorded token whose characters are in neither saved list is a runtime-only limitation, since reusing a dynamic country drops its character lists without erasing their tokens. See [AdvisorTokenRow](src/extended_rows/characters_intelligence.rs) for the saved-list join.

Built-in game variables are computed by the game on demand, not stored, and are not recorded. A registered leader outside the orders-group HQ portrait scope has no row; that is not evidence of an empty portrait. CountryFuel records ring sums rather than individual per-hour entries or the pending entry of the hour in progress.

Stored old country names and agency names are text in the recording client's language, which the content identity records.

InvasionPreparations' route path-cost operands have no save comparator; the oracle reports that limit explicitly. CountryFactories' group fields also have no text-save counterpart; the [oracle](../recorder/README.md#inspect-and-compare) compares levels only with saved building inputs.

LogisticsEquipment holds the daily update's stored balance; the Logistics screen recomputes it from live demand, so it can differ later in the day. The screen's category rows also add incoming lend-lease, which is not recorded. ProductionLines records the average efficiency and its maximum-factory clamp, but not the per-factory efficiencies.


ProductionLineOutput totals begin with observation on each recording branch; consumers must distinguish whole-life totals from totals since `since_hour` and consult `exact`. The table is unavailable for the rest of a branch once a tick has gone unobserved, never partial. See [ProductionLineOutputRow](src/extended_rows/economy_construction.rs) for delivery/organisation-credit differences and final-row publication rules.

ConstructionLineOutput's linked primary holds combined output; its partner's share is inexact and unknown, never an exact zero. Ownership comes from the construction census, so a line created and ended between censuses has unknown ownership. Ended totals survive until a written publication; lost observation withholds the table until a new branch. See [ConstructionLineOutputRow](src/extended_rows/economy_construction.rs) for the separate output and progress totals.

Cadence and acceptance remain defined in code. [naval_mission_derive.rs](src/naval_mission_derive.rs) ports the naval mission arithmetic; the fixed-layout family withholds all four mission-input tables when the registered modifier layout differs.

Combat Log equipment rows preserve the native monthly loss and recovery totals for every held country log. Country and feature gates, orders-group membership and pruning can limit what the game recorded; consumers must keep those losses unknown when coverage is unproven. Ended lend-lease exchanges can lose changes after their last daily read. See [CombatLogEquipmentRow and LendLeaseEquipmentRow](src/extended_rows/economy_construction.rs) and the hourly [DivisionCensusRow](src/extended_rows/movement.rs).

EquipmentDestruction holds only positive hourly removals by raw country,
stable cause and full design handle (VariantDefinitions identity).
Amounts are exact native fixed5 pre-minus-post pool quantities, including
rounding and clamping, aggregated within the hour and dated by the drain that
read them. Resolve type and design metadata through the existing recorded
variant tables and content stack. The hooked writers cover active land-combat
damage, naval transport destruction, immediate army damage, attrition, aircraft
destruction and garrison resistance. Cause values are independent of executable
addresses; the stable names live in the
[row contract](src/extended_rows/equipment_destruction.rs).
Unrecognized callers record explicit unknown, counted in collector status.
Source classifications and their executable evidence live in
[engine](../engine/src/equipment_destruction.rs).
These rows cover every country and divisions outside orders groups, but they
are not the Combat Log and do not claim unhooked causes such as convoy-pool or
lend-lease losses, scrapping or equipment captured/transferred between pools.
Two unhooked destruction writers are not covered: paradrop disruption
(`CArmy::ApplyParadropDisruption` -> `140C7AE10`) and the remaining equipment of
divisions destroyed in combat (`CLandCombatant::participant_lifecycle` `141299850`).
A complete empty hour means no removals through the hooked writers. Missing
is never zero: a first/branch interval is partial, and overflow, refusal or a
lost drain makes subsequent hours unavailable until a new branch. A text save
keeps remaining pools, not loss events, so it cannot certify their totals.

## Compatibility and recovery

This release reads and writes only the current format. Reject other replay versions at the header and incompatible spool records explicitly; do not infer or silently translate an older layout.

Every spool record carries the current recording version; recovery checks it before decoding the payload and keeps only valid CRC-checked prefixes. Replay finalization and append must not destroy an existing good file when interrupted. Preserve session/branch identity and never join branches through merely probable parentage.

A terminal collector allocation failure preserves the verified prefix as a normal replay. Its `Partial` trailer control carries the failing session, reason and optional attempted hour. Consumers open the prefix normally and keep the partial notice visible. Only a known, unrecorded attempted hour becomes an existing `Gap`, placed before detach; an idle failure invents no hour. A partial marker never makes a missing table complete.

The recorder prepares [terminal witness storage](src/witness.rs) before attach with a checksummed identity-only record. That prepared record is never terminal evidence. The collector opens it through a fixed native bootstrap before its first heap operation, then publishes its first failure through the retained handle using a fixed, versioned, little-endian buffer with a CRC. The witness binds process/session identity, optional campaign and branch/load context, the exact attempted hour when known and the last complete spool record watermark. The watermark carries its own campaign identity, independently of the attempted hour's campaign, so a campaign switch cannot discard the bound on an earlier spool. An in-flight write also carries its campaign's preceding watermark, preventing a later CRC-valid write from extending the verified prefix after failure. A first write with no preceding record certifies no prefix for that campaign. Every watermark requires its campaign identity. Absent optional payloads and reserved bytes must be zero; unsupported failure phase/code values are rejected even with a valid CRC. Missing, truncated or invalid witnesses are insufficient to finalize a failed capture.

Collector-reachable row, arena, decode, slicing and encoded-byte buffers use the shared checked-growth helpers in engine. Batch bounds cover following pushes and extensions; complete frame bytes precede baseline promotion. The spool publishes each complete record watermark before periodic durability work begins, and a complete frame clears the attempted hour. A later durability failure therefore keeps the frame and names no missing hour. Fixed Arc control blocks, paths, handles and thread setup retain the standard library APIs under the accepted collector allocation scope; live-sized payload copies remain checked. Recorder-to-DLL bootstrap uses the existing JSON config.

The collector installs the shared allocation-failure callback before bootstrap decoding and retains it through teardown. Static validation errors use a plain error kind while that callback is installed; recorder and playback readers retain their text diagnostics outside that host boundary. Allocation errors always use the plain out-of-memory kind.

File spools keep record writes ordered while allowing one periodic durability request per segment to run in the background. Pending errors are checked before subsequent writes and on idle polls; a failed segment is abandoned through the existing recovery path. Segment creation, forced boundaries and detach remain synchronous durability barriers. Every outstanding worker is joined before its sink is destroyed. On Windows the durability handle reopens the existing file through its handle, separately from the synchronous writer; no path-based replacement or extra capture-buffer queue is involved.

The replay stores content identity, ordered layer hashes and the coverage policy, not game assets or machine-specific paths. Asset resolution belongs in [installed-content](../installed-content/README.md).

## Checking changes

[fixed_layout.rs](src/fixed_layout.rs) lists which recorded tables and playback calculations use each fixed modifier key and static token, and checks playback keys against complete ModifierDefinitions and LexicalTokens. A missing row or dictionary, a registered gap or a different script spelling produces a named refusal. Expected names live only in engine; collector refusals use the existing unavailable-table encoding, so the format is unchanged.

Use existing round-trip, delta/baseline, malformed-input, version-rejection, provenance, branch and recovery tests. Follow the shared [change and validation rules](../../AGENTS.md#keep-these-guarantees) and [build commands](../../AGENTS.md#build-commands).

Province-hover border fields stay in existing hourly rows: StateFlags carries the raw active byte plus ordered attacker/defender country pairs only for active states; DivisionStatus carries optional defense/soft/hard attack caches for their class0 armies; Countries carries optional stored land-doctrine level for their province controllers. None denotes outside the current demand, never a failed selected read. Zero active states produces no new conflict pairs or optional scalar payloads. Standard field deltas clear and reintroduce these scoped values. Native layouts and validation live beside [StateFlagRow](src/extended_rows/politics_research.rs), [DivisionStatusRow](src/extended_rows/movement.rs) and [CountryRow](src/rows.rs).

DivisionStatus keeps the combat caches as an independent changed field. Its wider mask adds exactly one raw byte per changed row; an unchanged row emits none, and full snapshot rows have no changed-field mask. The ignored [size measurement](src/rows/border/tests.rs) uses the production row codec over quiet, rotating-subset and all-changed synthetic 24-hour streams, isolating the mask byte before comparing raw and Zstd sizes. It does not measure a complete recording or live change rates.

WeatherProvinces preserves the native signed fixed5 temperature and raw mud byte separately from display mud. Field deltas retain their full values; a displayed whole degree is not the stored temperature. SimulationFeatures keeps its optional suspension-byte pair distinct from a known zero pair, so a weather read failure cannot erase unrelated feature operands. The native evidence and exact field order live beside these rows in [naval_weather.rs](src/extended_rows/naval_weather.rs) and [map_presentation.rs](src/extended_rows/map_presentation.rs).

UnitLeaders appends the raw signed `exile_country` reference for the native kind-6 flag badge (`141C61C40`, `CUnitLeader+0xED4`), using its existing row-delta codec and cadence. LandCombatDetails appends optional `border_war` operands for `1413D69D0`: raw transfer byte and two ordered sides holding signed country/state IDs and exact win/lose event identifier bytes. Only border-war classes hold this value; ordinary combats carry absence. The optional aggregate participates in the existing changed-field mask and opaque codec. BorderConflicts and the table registry are unchanged.
