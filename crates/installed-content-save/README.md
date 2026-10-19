# installed-content-save

Faction save projections include sparse indexed faction-system accounts, variable seeds/literal entries, research membership/bonuses, minimum-upgrade and doctrine-folder selections, projection effects and faction/system extracted pools. Sparse sizes and indices remain sparse maps; repeated resource tokens and occurrence order are retained.

Text-save readers and serializer projections for the [recorder oracle](../recorder/README.md). The shared [installed-content parser](../installed-content/README.md#parser-contracts) owns syntax and exact decimal conversion; this crate interprets the state fields the recorder captures. Content inputs resolve through the recorded `installed_content::ResolvedLayers` stack, including installed mods. These readers do not read live game memory or change installed content.

`faction_goals` keeps the serializer's manifest, runtime goal order and status spellings, completed/cancelled history order, signed term slots and native goal/term dates. Runtime pending states are projected by the comparator through the serializer's cancelled branch; null historical occurrences and zero slots emit no saved token or slot. The unsaved display-progress cache is independent of these saved dates.

`faction_companions` retains saved research/manpower access booleans with key presence, ordered repeated rule tokens, full programme references including null and stale values, and the manpower writer's positive contributions in occurrence order. Nonpositive native manpower values remain captured but emit no saved occurrence.

Faction `manpower_pool` wrappers repeat: the persistent-object writer emits an empty block before the explicit contribution writer. Each wrapper appends its entries in save order, matching the loader; empty blocks never clear preceding entries. Multiple populated blocks and repeated country tags retain every occurrence. The shared manpower-entry reader validates every payload, while single-wrapper occupation/garrison fields retain their strict duplicate checks.

Read the [project rules](../../AGENTS.md) before changing a reader. Consumers use `installed_content_save` directly.

## Layout

| Area | Entry points and responsibility |
|---|---|
| Save projection and map membership | [lib.rs](src/lib.rs): dates, exact values, captured state, content joins and partial state membership. |
| Country emissions | [countries.rs](src/countries.rs): one owning save tree and authenticated physical-country projection. |
| Field readers | Domain modules such as [units.rs](src/units.rs), [construction.rs](src/construction.rs), [intel.rs](src/intel.rs) and [naval_history.rs](src/naval_history.rs) retain serializer output and ordering. |
| Exact decimals | The decimal parser in [installed-content-core](../installed-content-core/README.md): `Decimal` and `FIXED_SCALE`, used by this crate's public value types. |

## Save contracts

Saves follow native omission, ordering, duplicate and representation rules. Preserve exact integers, fixed-point values and optional presence; do not invent unsaved caches. Partial state membership distinguishes assigned, unassigned and unknown provinces independently of resource failures. The [recorder oracle](../recorder/README.md) owns comparison limits and finalization.

Air wings retain the optional `raid_instance` full reference under its native resolution omission gate; a missing key stays distinct from a malformed or duplicate saved link.

An omitted state owner certifies the native signed nonpositive class and also suppresses the state controller. Province control retains absent ownership for those states, preserves each explicit positive province controller and keeps omitted controllers absent. A missing state or unknown membership remains a separate join failure or exclusion.

`SaveDocument` projects serializer output: a bare `=fixed5` directly inside any saved `variables` block retains an empty literal key, distinct from an anonymous value or an omitted map. Other tree readers keep their existing grammar. This preserves the writer's state; native text-load recovery consumes that malformed assignment differently and does not round-trip the empty name.

Operatives retain the inherited leader writer's optional nonempty portrait `gfx`; an emitted empty value is refused.

Intel networks retain the independent coverage counts, controlled/core strength sums and maximum state strength. A network without states omits its parallel state lists; its national caches remain present independently. Partial projections retain absent caches for the oracle to report, without synthesizing zeros or reconstructing totals from state strengths.

Strategic operatives retain ordered recent propaganda efforts, including zero occurrences, and the independent weekly drift. Both channels use the same five-decimal Q15 save formatter as fuel and experience; the shared `FuelQuantity::parse(...).raw()` conversion recovers their unique native signed words exactly, including wide boundaries, and refuses decimals with no native preimage. No floating-point conversion or rounding is applied. Omitted empty history and omitted zero drift follow the serializer's gates.

Intel source projections preserve dense allied/cryptology positions, ordered contribution pools and prior-day values, static IDs/removal flags, dynamic pending/accumulator lists and held cheat/radar/token/network source references. Empty pools, signed keys, duplicates and keyed zero occurrences remain distinct.
Dynamic source pools normalize the serializer's `accumulator` list as pending additions and its `values` list as retained aggregate/decay results; their independent extents and ordered occurrences remain exact.

Saved technologies retain ordered repeated keys because the native serializer filters each instance independently.

The [combat-log reader](src/combat_log.rs) retains own and enemy equipment months, recoveries, all three manpower counters, template statistics and ordered repeated battle references. Shared battle records retain reference counts, side losses, captured and recovered equipment, modifier-hour occurrences, country tags and optional serializer presence separately from zero and empty values.
War relations retain both directional participation blocks independently of
their integer casualty counters. Their exact signed fixed-point components,
endpoint tags and ordered captured-province membership remain separate facts.
Omitted blocks retain explicit absence. The runtime unknown-casualty score has
no text-save field; the saved `casualties` component is the known-casualty score.
Same-side delivery histories retain all four exact quantities. Their enclosing block's omission proves native binary fixed-point zero for each quantity, which can include small nonzero raw values.

Country resources retain shared `delivery_routes` by receiver tag, including exact optional `from_state` and `to_state` values. Omitted endpoints, zero and an absent receiver entry remain distinct; duplicate receiver keys are refused.

[Saved transfer inputs](src/counter_inputs.rs) keep source province and ordered combat references separate from the target. [Mission cache reads](src/naval_missions/spotting_cache.rs) use recorded lexical keys and the authenticated physical-country projection; omission constrains only the nonpositive writer branch. Air pools require their saved definition token. Orders-group and plan names distinguish an omitted key from emitted empty first-NUL text.

[Patrol mission words](src/naval_missions/patrol_inputs.rs) bind target, progress and chance through recorded lexical keys and the same physical-country projection. The full target reference survives independently of current registry liveness. Progress and chance retain signed nonzero values; omission means zero. Explicit zero words, null target entries, duplicate fields, duplicate full identities and aliased bindings are refused.

The [template save reader](src/units.rs), state, province and faction names, and ace callsigns preserve key presence separately from bytes: emitted empty text differs from an omitted key. Comparisons follow each native stored-length gate and first-NUL prefix. Unconditional deployment and air-group names require the saved key even when its text is empty.

## Checking changes

Follow [AGENTS.md](../../AGENTS.md) for scoped formatting, clippy and affected tests. Synthetic tests live beside the readers; retain omission, order, duplicate, representation and layered-content cases when changing behavior. Integration follows the [recorder fresh-save oracle](../recorder/README.md#fresh-save-oracle); never waive a save difference. Keep saves, recordings and captures outside Git.

The read-only installed-state-history test is ignored by default. Set `BATTLE_PLAN_GAME_DIR` to the supported installation; it initializes shared executable metadata explicitly. From the repository root, run:

```powershell
cargo test -p installed-content-save vanilla_state_history_reads_without_diagnostics -- --ignored --nocapture
```

Manpower parsing retains the country recruitment ratio, exile manpower and maximum exile manpower; state available, locked and total pools; acquired ship and wing crews and required ship crews. Counter input parsing retains ordered sent-expeditionary references, including nulls and duplicates.

Air wings retain optional government_in_exile_tag separately from pool owner and logical override. Templates preserve the saved origin_type spelling and omission for the manpower exile predicate.

State resistance preserves scripted target additions in saved order, including omitted country selectors and emitted empty tooltips. Its stored target retains exact fixed5 zero omission. State and occupation-group active resistance/compliance definitions preserve serializer order, duplicates and emitted empty names, independently of active status; omitted lists certify zero occurrences. Occupation groups retain their saved retained-country key separately from the state's occupied-country link. A missing country occupation-status wrapper is unknown; a present wrapper without an occupation list certifies no groups. Diplomacy keeps the exile host independently of raw legitimacy.

War-goal projections preserve each country's object and available-reference order, complete state/liberation/puppet lists, expiry presence and justification activity ordinals, points, costs, dates, upfront-payment flag and threat. Omitted scalar fields retain their serializer gates.
Air wings preserve the unconditional saved reinforcement_setting byte, including zero and unusual byte values, separately from missing-key evidence.

Division presentation retains the stored experience value independently of active exercising status and national experience.

Division presentation retains both ordered history lists and their medal amount, with saved text/key presence, native omission predicates and embedded sunk-ship records. Each entry retains the required medal_count and inherit yes/no flags independently of the signed medal-store amount.

The commander save reader preserves the complete `trait_xp_factor` vector independently of `in_progress`, including repeated keys and signed fixed5 factors.

Division presentation preserves the optional old template reference during conversion.

The division's outer `army_history` holder contains sibling career `army_history` and `unit_medals` blocks; omission of that holder remains separate from empty lists. Held-officer experience and the retained history-panel counter preserve their independent save operands.
The governance doctrine reader retains ordered folders, tracks, completions,
commander contributions, cost reductions, mastery grants and equipment handles.
Anonymous empty cost/grant blocks retain their slots independently of valid
entries. Saved unit-source daily mastery is distinct from the unsaved effective
daily-gain cache.

Mastery-grant filters preserve the authored `track` token separately from the
optional numeric `track_index`; zero is a present index and omission stays absent.
Research save projection includes unused grant rosters, their signed counter/use/claim values and repeated eligibility restrictions. Special-project projection includes primary/supporting assignments and delayed timers, pending multiple-choice rewards and scoped variable values/seeds. Scientist projection includes the serializer's positive injury-recovery timer.

Faction advisor-slot projections preserve owner/advisor presence, optional change dates and the independent modifier/spymaster source names and numeric entries in native slot order.
Manpower parsing retains the country recruitment ratio, exile manpower and maximum exile manpower; state available, locked and total pools; acquired ship and wing crews and required ship crews. Counter input parsing retains ordered sent-expeditionary references, including nulls and duplicates.

Air wings retain optional government_in_exile_tag separately from pool owner and logical override. Templates preserve the saved origin_type spelling and omission for the manpower exile predicate.
Research save projection includes unused grant rosters, their signed counter/use/claim values and repeated eligibility restrictions. Special-project projection includes primary/supporting assignments and delayed timers, pending multiple-choice rewards and scoped variable values/seeds. Scientist projection includes the serializer's positive injury-recovery timer.
Combat Log projections retain ordered monthly membership, enemy amounts, manpower counters, templates and references, shared battle records and active observer descendants. Serializer omission remains distinct from a present zero or empty collection.
Division templates retain their exact occupied grid coordinates, recruitment and equipment settings, restriction flags, optional cap and equipment niche, obsolete date, parent, creator and origin, ordered variant exclusions, forbidden equipment tokens and explicit model choices. Required priority and equipment-switch keys retain their presence separately from values. Empty model text retains key presence for the writer's first-NUL string behavior.

The production save projection retains each country's templates_locked flag and the deployment wrapper's optional default_hq_template reference independently of an empty queue and an absent wrapper.


The orders-group save projection retains target-template references, deployment flags, the nearest-front province and headquarters deployment holders, including their priority, owner, ordered equipment pools, manpower and requisition state. Country template configuration retains the next headquarters allocation order.

Country template configuration parses the occupation distributable priority, preserving an absent wrapper and the native omitted default separately.
Combat Log projections retain ordered monthly membership, enemy amounts, manpower counters, templates and references, shared battle records and active observer descendants. Serializer omission remains distinct from a present zero or empty collection.
Division templates retain their exact occupied grid coordinates, recruitment and equipment settings, restriction flags, optional cap and equipment niche, obsolete date, parent, creator and origin, ordered variant exclusions, forbidden equipment tokens and explicit model choices. Required priority and equipment-switch keys retain their presence separately from values. Empty model text retains key presence for the writer's first-NUL string behavior.

Country template configuration parses the occupation distributable priority, preserving an absent wrapper and the native omitted default separately.
`naval_management` preserves required-key presence for naval policies and budgets, independent current/queued refit designs, rejoin references, ordered repair children and repair queues, and parallel composition lists. Unequal composition-array lengths and keyed list entries are refused.
Market save projection preserves listed pools, omitted normal price levels, both subsidy alternatives, all automation flags, pending sparse requests, independent contract quantity/price manifests, runtime/history scalars, in-transit equipment and exact contract variable/random state. Lend-lease terms retain all three schedules, dates and independent fuel scales.
Network auxiliary projections preserve local edges, vertex depths, previous sources, local metadata, national records and operative/location occurrences, plus subnetwork coverage, coverable counts, sums, detection operands, records, state/strength occurrences, core states and operative handles. Ordered duplicates and absent scalar inputs remain distinct.
The script_events reader preserves pending instance order, fired identity duplicates and each projected country's delayed-event vector. Iterative scope trees retain optional ROOT/FROM/PREV children, both random words in save order, all reference pairs, saved targets and unknown keys for exact comparison. Missing global registry blocks remain distinct from empty blocks.

The decision-status parser is shared by governance census comparisons and the retained event-feed oracle, preserving taken order, all five instance containers and ordered description random counters.

Flattened fleets, task forces and ships retain their enclosing country's tag, as bases do,
so oracle comparisons can apply the authenticated canonical emission scope to every object.

The air pack also projects each country's strategic_region_data blocks, retaining strategic_region and more_ground_crews_enabled with the native false omission. The Ground Crews oracle compares only pairs demanded by recorded regional missions.

Ship critical_damage wrapper presence retains the native nonempty-array gate, including an emitted empty wrapper when unnamed critical entries are skipped.

The air mission stop_training_at_max_xp flag follows its true-only serializer omission; demanded training wings compare it in the oracle.

Air wings preserve optional transfer_progress and transferring_to exactly. Native serializer omission stays distinct from zero; total relocation distance and direct incoming registration order are unsaved.

The naval history pack retains ordered standing accidents and mine reports. Mine reports preserve both serialized country occurrences in their original victim/mine-owner order; absent sunk flags mean false, while all unconditional operands retain missing-key failures.

The retained raiding-grid save view binds its spotting and region keys through caller-supplied recorded lexical spellings. It preserves negative spotting and independent omission, and follows the same authenticated physical country projection as the full save.

Fleet counter projection retains the saved strategic-region identities and order so the oracle compares the captured membership multiset with every saved occurrence.

Air bases preserve their ordered repeated countries containers, with exact fixed5 fuel_consumption and received values and zero omission. The saved tag stays separate from the native indexed country ordinal.
