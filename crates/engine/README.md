# engine

The boundary between this project and HOI4's native memory. This crate owns the supported executable identity, native layouts, offsets, guarded reads and table walkers. It does not own replay encoding or presentation.

## Start here

Use the source map below to follow a field from native memory to its consumer. Read the [reverse-engineering guide](../../tools/re/README.md) before investigating a native field.

The pre-rewrite implementation is retained at Git tag `archive/pre-rewrite-2026-09-09` and `<local-path>`. Use it as a research lead, not as proof that an old offset or interpretation is correct.

## Source map

| Question | Start here |
|---|---|
| Which executable and sampling boundary are supported? | [build.rs](src/build.rs), [cgame_state.rs](src/cgame_state.rs) and the collector's [hook.rs](../collector/src/hook.rs). |
| How are memory reads bounded and failures represented? | [memory.rs](src/memory.rs), [walk.rs](src/walk.rs) and the relevant struct's reader. |
| How do country references resolve? | [ccountry_tag.rs](src/ccountry_tag.rs), [country_memory.rs](src/country_memory.rs) and the [replay identity rules](../replay/README.md#values-availability-and-freshness). |
| Where is a native row captured and encoded? | Collector [acquisition](../collector/src/acquisition.rs) and [extended packs](../collector/src/extended/), then replay [rows](../replay/src/rows.rs), [extended rows](../replay/src/extended_rows.rs) and [format](../replay/src/format.rs). |
| What do the raid map tooltips read? | [raid_facilities.rs](src/raid_facilities.rs), [raid_air_scores.rs](src/raid_air_scores.rs), [raid_intel_inputs.rs](src/raid_intel_inputs.rs) and [raid_tooltip_operands.rs](src/raid_tooltip_operands.rs); each separates stored values from the tooltip's on-demand arithmetic. |
| What can a save validate? | [Text-save parsers](../installed-content/src/save/) and the recorder's [oracle comparators](../recorder/src/oracle/); keep unsupported fields explicit. |
| Where is a reader's field evidence, validation and live-verification status? | The reader's own module docs (`//!`), beside its offsets; for example [research_inputs.rs](src/research_inputs.rs) or [country_stability.rs](src/country_stability.rs). |

The struct files hold offsets, native widths and evidence. Do not recreate the old registry, table-count or format-version catalogs here. A stored field, a calculated getter and a renderer cache are different sources; a similar-looking save value does not make them interchangeable.

## Adding or changing a reader

Built-in modifier names live in [modifier_keys/builtins.rs](src/modifier_keys/builtins.rs), with each native registration callsite; the fixed-key constants in [modifier_keys.rs](src/modifier_keys.rs) take their names from that catalog. Hard-coded static lexer tokens are listed in [fixed_layout.rs](src/fixed_layout.rs). Its check confirms, through the witnessed lexer dictionary, that every built-in row spells its script name, that the gap row after the built-ins is default-constructed, and that each token is within the static range and spells its name. A mismatch anywhere in the built-in layout fails every fixed key by name. Dynamic definitions remain sized and resolved from live data.

Keep one source file per engine struct. Put a short evidence comment beside each constant: the native function or structure that establishes it and how it was confirmed. There is no arbitrary file-count cap.

Derive widths, signedness, ownership and meaning from native writers, serializers and established evidence. An old reader is a lead, not proof. Look functions up through `hoi4re`; save established names, fields and notes before reporting the result. If a decompile drops an argument or arithmetic, inspect the indexed instructions rather than guessing. Distinguish virtual addresses from image-relative RVAs.

Guard every native read. Preserve source order, identity checks, meaningful pointer occurrences, bounds and the first independent failure. A failed or short read must not become a complete row, an empty table or a zero value. Never call engine functions to obtain a value, wait for memory to change, or retry to hide a failure.

The collector may decode owned copies after the callback. Those copies must be sufficient: deferred code must not follow live game pointers. Shared acquisition is useful only when each consumer retains its own correctness and failure behavior. Address equality alone does not identify a table or read occurrence: keep raw/owned lookups correctly scoped, including empty and overflow paths.

## Reuse and measurement

Keep cached plans and copied buffers within their proved lifetime. Reloadable defines, weather structures and mutable definitions are not automatically session-static. Reuse native inputs only when identity, ownership and invalidation are established.

[read_plan.rs](src/read_plan.rs) executes fresh exact reads with ordered comparisons of the owned values that selected later addresses. The native executor uses compact checked descriptors; geometry outside that representation keeps the ordinary ordered executor. Hints run only after their selecting controls pass and never acquire a value. A stopped plan exposes its completed prefix and attempted failure so a producer can continue without rereading it. Bounds validation alone does not prove a plan's dependency coverage or object lifetime.

Use the instrumentation present on the checked-out branch and its actual counter definitions. Do not count decoding an owned copy as another native read, or report synthetic fixture savings as measured gameplay speed.

Test readers with the existing synthetic-memory fixtures, including aliases, invalid pointers, short reads and capacity limits. Run the local checks in [AGENTS.md](../../AGENTS.md); a new capture domain also needs collector/replay integration and an oracle comparison or an explicit limitation.

Unit-count fixtures live in [tests/support/unit_world.rs](tests/support/unit_world.rs). Run `cargo test -p engine --release --test unit_scale -- --ignored --nocapture --test-threads=1` through the build slot; `BATTLE_PLAN_UNIT_SHAPE=design|stress` selects one population per process for memory measurement. `BATTLE_PLAN_UNIT_VALIDATE=1` checks complete native walks without timing repetitions. The separate `unit_scale_templates` test extends the model-input fixture; it retains that benchmark's in-memory fastest-pass methodology. Synthetic guarded reads measure acquisition work, not gameplay slowdown.

## Historical evidence

The [archived domain index](#) preserves old registry-to-source mappings, exclusions and investigation results. It is not current coverage or acceptance: its double-read windows and phase gates are superseded by [AGENTS.md](../../AGENTS.md). Keep new field evidence beside the reader, not in another running porting log. For the hourly-sample versus later-save limitation, see the [recorder guide](../recorder/README.md#inspect-and-compare).

The [naval mission input reader](src/naval_mission_inputs.rs) reads the native strike/invasion range and convoy-efficiency chain, authenticated embedded missions, stored navy ledger headers and supply-hub lists, and raw country/fleet-leader/HQ accumulators. Native reader/writer addresses stay beside the fields; the build-fixed modifier registrations are in [modifier_keys.rs](src/modifier_keys.rs).

[saved_event_targets.rs](src/saved_event_targets.rs) reads the saved event targets a save writes, in native order; its module docs hold the fields, liveness bits and refusals.

[fixed5.rs](src/fixed5.rs) owns the native logarithm, exponential and power series
(1424DAA00, 1424DA930, 1424DAB50), shared by movement and naval mission derivation.
