# Rebuild acceptance coverage

The production product registry and the widget registry define the inventory.
The prepared-slice [native harness](../crates/dashboard/acceptance.md) compares
actual requested focus, produced-equipment, country-balance and map checkpoint
outputs with retained production readers. The [browser harness](../ui/e2e/README.md)
compares actual configured widgets, their complete registered semantic models,
visible text and RGBA pixels. Both keep recordings, installed content and evidence
outside Git. A listed test or capture is an entry point, not a validation result.

## Current checks and remaining comparison scope

| Family | Direct prepared checks | Other exact comparisons required |
|---|---|---|
| National focus | Hour/order/progress/daily gain/native holder, repeated completion ordinals, dated Display/Text rows, titles/PNG bytes, table refusals and all requested page rows. | Actual whole-campaign/follow/pin view, empty and literal titles, referenced static metadata, independent text/icon refusals, inline details and expansion state/motion, Jump and keyboard traversal. Shared legacy/prepared model schemas must preserve every referenced field. |
| Research and special projects | Production readers remain the oracle; no research prepared query is exercised by the native slice harness. | Slots and completions/read bounds, repeated technology tokens, progress/bonus/modifier details, country/foreign scopes, complete ordered histories, technology artwork and disclosure controls. |
| Produced equipment | Arbitrary included subsets, duplicates/missing selectors, type/variant/domain, total/day/week/month, refits, units/IC, exact wide signed values, independent equipment amounts, knownness, paging and order. | Actual race/mix/lines/parts/drill adapters, global bounds, every offscreen ranked row, historical membership and selector histories, per-country folds/residuals, dated variant names and holder-bound pins. Passive raw models with different shapes cannot certify semantic parity by dropping fields. |
| Other production/equipment | Existing lines, stockpile, spending and losses products remain oracle inputs. | Line/design/organisation/refit identity, deleted-line finals, Rocket and equipment independent coverage, retained counts/IC windows, all breakdowns, empty/null variants, unresolved type attribution and exact tooltip operands. |
| Political/command power | Exact raw intervals, native identity/segment start, drops and through-hour pages, width summary extrema/continuity, complete mark clusters and balance-only keyboard dates. | Attributed one-off payments, daily accounting/deficits/native wrapping, reserves, maximum, lost gain, group membership, income/spending drills, supplying dates and picks. Approved width geometry summaries are compared independently from exact values and events. |
| Country identity/looks/groups/diplomacy | Native holders are compared at reached slice hours. Other histories remain oracle inputs. | Tag aliases/replacement holders, client LocalPlayer/observer choice, dated names/flags/styles and all presentation refusals, human/faction/war rosters, hour/end/ever membership, subjects/autonomy and faction totals. |
| Census and scores | Country counts in preparation/checkpoint fixtures exercise live sizing. | Army/supply/air/navy/factories/nukes/experience/scoreboard/game-score histories and rankings, zero versus unknown, same-holder clipping, every original tie rule and independent field knownness; unit leaders/portraits and air aces/portraits retain their actual ordered rosters. |
| Politics/history/text | Existing stability/parts, ideas, parties, politics ledger, peace deals, world tension and event-feed readers remain the oracle. | Stored/scoped text and empty strings, native event versus confirming/read dates, duplicates and original ordering, complete payment/outcome inventories, all details and named counted content refusals. |
| Territorial/resources/construction | Existing territory/breakdown, occupied states/manifest/trend, land owners/place names, resource balance, trade and construction readers remain the oracle. | Exact ownership and read coverage, origin/recipient/source dates, sparse pair/resource inventories, signed spending/resource operands, construction/deletion observations, native place-name ladders and empty text. |
| Fuel | Existing balance/awards/award history/consumption/demand/demand history/demand stack remain the oracle. | Independent recorded versus derived fields, held supplying reads, exact occurrence keys/segments, recipient/topology multiplicity, reason-gated unknown spans and every requested child/history row. |
| Combat and losses | Checkpoint reconstruction includes DivisionStatus counters. Other products remain oracle inputs. | Division lifecycle/manifest/totals, war casualty pairs/breakdown, land battle/manifest/ranking/course/terrain, naval battle/manifest/ranking/details/losses, ships sunk/titles, equipment losses, military air ledger, air superiority and raids. Preserve native comparison keys, signed wide counts, side/read identity, duplicates and independently unavailable parts. |
| Intelligence | Existing decryption, agency comparison, espionage log, operation description, placements and roster remain the oracle. | Complete potentially quadratic pairs/rosters, exact native byte extents and known empty text, operation/placement identity and publication dates, independent numeric/text/picture refusals. |
| Native map | Actual prepared checkpoint bytes and replay decoder compare Campaign/Clock/DivisionStatus rows, availability, sampled provenance, exact target/gap and bounded seek/step reads. Private viewer tests exercise its real checkpoint attachment. | Retained map tables/builders, transitions/routes/GUI clocks, displayed-hour ownership, skipped-hour history, workers/tooltips, resources, standalone/embedded views, picking/occlusion/settings/exports and real GPU pixels/timestamps. |
| Shell and lifecycle | Real preparation/restoration/reuse and optional appended-tail block counts; source/revision/product echoes. | Open/rebind/unload, pending carry, stale cancellation, default/manual background work, section progress/refusals, Clear while open, library/storage/settings, virtual lists/menu accessibility, exact input-to-painted-frame and idle CPU. |

The browser requires each matrix case's named model providers. A missing provider,
incomplete full row inventory, changed request/action, failed source or deadline
is an error. Exact differences remain differences until their semantic cause is
resolved. Captured raw prepared and legacy models are retained when their shapes
differ. Successful JSON validation, SSR adapters or pure chart tests do not
establish executable recording or native presentation parity.

## Synthetic 5,000-country and long-history checks

Central Rust entry points present in the integrated slice include:

```text
cargo test --locked -p dashboard prepared_focus::tests::sequential_observations_match_history_at_scale_and_resume
cargo test --locked -p dashboard prepared_production::tests::arbitrary_subsets_keep_signed_wide_sums_ties_request_order_and_refits_at_5000_countries
cargo test --locked -p dashboard prepared_measure::tests::five_thousand_countries_opaque_tags_and_ambiguous_links_have_no_cap
cargo test --locked -p dashboard prepared::pipeline::tests
cargo test --locked -p dashboard prepared_measure_query::tests
cargo test --locked -p viewer map_checkpoints::tests
cargo test --locked -p viewer hours::tests::source_worker_seed_attaches_committed_checkpoints_and_clear_keeps_exact_hour_ownership
```

The pipeline checkpoint test feeds 5,000 countries through actual domain state
publication and persisted resume. Its source block cursor/new-block checks belong
to the metadata-only append policy; they do not count every kernel read. Prepared
measure tests use long intervals and logarithmic ordinal reads; mark pages retain
duplicate rows. The replay checkpoint fixture independently preserves
`NotRecorded`, native absence, unavailable/truncated states and acquisition dates.

Frontend entry points include `preparedFocusData.test.ts`,
`preparedFocusParity.test.ts`, `RaceChart.remote.test.ts`,
`preparedMeasureQuery.test.ts`, `chartSummary.test.ts` and `markSummary.test.ts`,
plus the shared VirtualList complete/sparse 5,000-row traversal checks. The browser
review fixture captures 5,000 synthetic rows and exact pixels in a retained Job.
The matrix parser preserves a supplied 5,000-country selection without a harness
cap; this parser check is not a renderer or performance result.

Remaining families need differential fixtures at 5,000 countries and their actual
live scale vectors: high-churn intervals, complete ranks, pair multiplicity,
opaque/long tags, replacements, gaps, independent read cadences, missing fields,
late content refusals, ordered duplicates and complete multi-page outputs. Those
fixtures use synthetic content. Installation-dependent checks read
`BATTLE_PLAN_GAME_DIR`; no installed game assets become fixtures in Git.

Length checks use the same live entity scale and requested viewport across a
short/reference input and readable long input. Preparation alone walks accepted
new blocks. Reopen/query/seek/step memory and read counts are measured separately;
the native harness's old oracles and their history caches are included in its
process samples and cannot establish a prepared-only memory bound. Format-102
inputs require their separate reader integration. An incomplete generated long
file is not an accepted input.

## Measurement targets and provenance

| Operation | Target and measurement scope |
|---|---|
| Prepared lookup | Release query p99 ≤2 ms; typical ≤0.3 ms per bounded page/viewport. First in-process and repeated timings remain separate; OS-cold requires a separately controlled run. Include content-file stamp cardinality, manifest/page checks, JSON serialization and reply bytes. |
| Produced subset/rank | X06 ≤20 ms computation/≤50 ms first complete ranked page at 5,000 countries ×32 types; one country ≤10 ms for 1,024 parts. Complete global prerequisite work is measured before output paging. |
| Exact totals/details | X04 ≤20 ms computation/≤50 ms first exact page for 160,000 qualifying contributions. Preserve unknown counters, signed values and every requested child row. |
| Map seek/step | Exact seek ≤50 ms; next hour's table state ≤1 ms; sustained default-view playback ≥60 hours/s. Native displayed-frame, transitions/clocks and GPU work are additional evidence. |
| Prepared reopen | Warm OS-cache interactive open ≤300 ms; cold ≤1 s. Include the actual metadata restoration path and its transient private memory, not just a manifest lookup. |
| Preparation | Target ≤10 s per in-game year on the specified 16-core NVMe/GPU desktop. Record accepted cursors, newly prepared blocks, per-section failures, disk bytes and peak working set; constant artwork/changing-progress cases expose redundant page rewrites. |
| UI/idle | Input to first complete paint ≤8.3 ms at 120 Hz; main-thread work ≤4 ms/frame; idle ~0% CPU. Software-rasterized screenshot parity does not measure these targets. |

Targets are not pass claims. Every run records source/commit, selected release
executable, exact recording/content inputs, case matrix, prepared/native seed
provenance, cache condition, sample count, complete diffs and failures. A prepared
seed reduces setup cost but cannot replace a fresh source/model/screenshot capture.
No approved geometry change permits rounding exact operands, reordering records,
filling unknown values or suppressing a named refusal.
