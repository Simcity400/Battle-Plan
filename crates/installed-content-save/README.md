# installed-content-save

Text-save readers and serializer projections for the [recorder oracle](../recorder/README.md). The shared [installed-content parser](../installed-content/README.md#parser-contracts) owns syntax and exact decimal conversion; this crate interprets the state fields the recorder captures. Content inputs resolve through the recorded `installed_content::ResolvedLayers` stack, including installed mods. These readers do not read live game memory or change installed content.

Read the [project rules](../../AGENTS.md) before changing a reader. Consumers use `installed_content_save` directly.

## Layout

| Area | Entry points and responsibility |
|---|---|
| Save projection and map membership | [lib.rs](src/lib.rs): dates, exact values, captured state, content joins and partial state membership. |
| Country emissions | [countries.rs](src/countries.rs): one owning save tree and authenticated physical-country projection. |
| Field readers | Domain modules such as [units.rs](src/units.rs), [construction.rs](src/construction.rs), [intel.rs](src/intel.rs) and [naval_history.rs](src/naval_history.rs) retain serializer output and ordering. |
| Exact decimals | [installed-content parser/decimal.rs](../installed-content/src/parser/decimal.rs): `Decimal` and `FIXED_SCALE`, used by this crate's public value types. |

## Save contracts

Saves follow native omission, ordering, duplicate and representation rules. Preserve exact integers, fixed-point values and optional presence; do not invent unsaved caches. Partial state membership distinguishes assigned, unassigned and unknown provinces independently of resource failures. The [recorder oracle](../recorder/README.md) owns comparison limits and finalization.

An omitted state owner certifies the native signed nonpositive class and also suppresses the state controller. Province control retains absent ownership for those states, preserves each explicit positive province controller and keeps omitted controllers absent. A missing state or unknown membership remains a separate join failure or exclusion.

`SaveDocument` projects serializer output: a bare `=fixed5` directly inside any saved `variables` block retains an empty literal key, distinct from an anonymous value or an omitted map. Other tree readers keep their existing grammar. This preserves the writer's state; native text-load recovery consumes that malformed assignment differently and does not round-trip the empty name.

Saved technologies retain ordered repeated keys because the native serializer filters each instance independently.

[Saved transfer inputs](src/counter_inputs.rs) keep source province and ordered combat references separate from the target. [Mission cache reads](src/naval_missions/spotting_cache.rs) use recorded lexical keys and the authenticated physical-country projection; omission constrains only the nonpositive writer branch. Air pools require their saved definition token. Orders-group and plan names distinguish an omitted key from emitted empty first-NUL text.

The [template save reader](src/units.rs), state, province and faction names, and ace callsigns preserve key presence separately from bytes: emitted empty text differs from an omitted key. Comparisons follow each native stored-length gate and first-NUL prefix. Unconditional deployment and air-group names require the saved key even when its text is empty.

## Checking changes

Follow [AGENTS.md](../../AGENTS.md) for scoped formatting, clippy and affected tests. Synthetic tests live beside the readers; retain omission, order, duplicate, representation and layered-content cases when changing behavior. Integration follows the [recorder fresh-save oracle](../recorder/README.md#fresh-save-oracle); never waive a save difference. Keep saves, recordings and captures outside Git.

The read-only installed-state-history test is ignored by default. Set `BATTLE_PLAN_GAME_DIR` to the supported installation; it initializes shared executable metadata explicitly. From the repository root, run:

```powershell
cargo test -p installed-content-save vanilla_state_history_reads_without_diagnostics -- --ignored --nocapture
```
