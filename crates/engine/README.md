# engine

The boundary between HOI4's native memory and the recorder. This crate owns the supported executable identity, native layouts and offsets, guarded readers, table walkers and hook site descriptors. Replay encoding belongs to [replay](../replay/README.md), hook installation and acquisition to [collector](../collector/README.md), and presentation to the consumers.

Shared rules and checks live in [AGENTS.md](../../AGENTS.md). Use the [hoi4re guide](../../tools/re/README.md) when investigating native behaviour.

## Source map

| Area | Start here |
|---|---|
| Executable identity and sampling operands | [build.rs](src/build.rs), [cgame_state.rs](src/cgame_state.rs) and the collector's [hook.rs](../collector/src/hook.rs). |
| Guarded copies, exact-field gathers and accounting | [memory.rs](src/memory.rs), [seh_read.c](src/seh_read.c) and [read_counts.rs](src/read_counts.rs). |
| Walk results, checked storage and prepared reads | [walk.rs](src/walk.rs), [allocation.rs](src/allocation.rs) and [read_plan.rs](src/read_plan.rs). |
| Native containers and strings | [cpdx_array.rs](src/cpdx_array.rs), [msvc_tree_node.rs](src/msvc_tree_node.rs) and [msvc_string.rs](src/msvc_string.rs). |
| Country identity and native order | [ccountry_tag.rs](src/ccountry_tag.rs), [country_memory.rs](src/country_memory.rs), [country_roster.rs](src/country_roster.rs) and [original_tag_lists.rs](src/original_tag_lists.rs). |
| Full reference handles | [creference_registry.rs](src/creference_registry.rs); shared registry counts bound storage, not one reference type's population. |
| Modifier names, definitions and fixed tokens | [modifier_keys.rs](src/modifier_keys.rs), [modifier_keys/builtins.rs](src/modifier_keys/builtins.rs), [modifier_definition.rs](src/modifier_definition.rs) and [fixed_layout.rs](src/fixed_layout.rs). |
| Struct layouts and field evidence | The corresponding module under [src](src/); [lib.rs](src/lib.rs) lists the public modules. |
| Hook boundaries and operands | Domain site modules, for example [line_output_sites.rs](src/line_output_sites.rs), [peace_finalizer_site.rs](src/peace_finalizer_site.rs) and [event_feed_sites.rs](src/event_feed_sites.rs). |
| Save comparisons | [installed-content save readers](../installed-content/src/save/) and [recorder oracle comparators](../recorder/src/oracle/). |

Offsets, widths, native functions and field-specific limitations stay beside the reader. Stored fields, calculated getters and renderer caches are different sources; a similar save value does not establish equivalence.

Fixed modifier keys and lexer tokens require the live layouts to match their native registration names; a mismatch refuses fixed keys by name. Dynamic definitions size and resolve from live data.

## Reader safety

- Use `Memory` and the guarded read helpers for every native span. Check nulls, address arithmetic, lengths and the reader's vtable, reference-type, identity and ownership requirements. A readable address alone does not authenticate an object.
- Preserve native order, repeated occurrences, signed widths and arithmetic. Deduplicate only where native behaviour permits it. Scope shared reads to the selecting table, identity and occurrence; address equality alone is insufficient.
- Preserve the first independent failure and each consumer's failure boundary. A short or failed read cannot become a complete row, empty table or zero. Distinguish known absence, unavailable data and capacity truncation, counting unread occurrences as the walker requires. Do not retry to hide a failure.
- Read passively at the collector's established boundary. Never call game getters, refresh caches, wait for memory to change or infer freshness beyond the native writers. Registry walks require a quiescent boundary.
- Copy everything deferred decoding needs into owned storage during acquisition. Deferred code must not follow game pointers. Native string descriptors, including ace portrait descriptors, authorize immediate acquisition only; copy their payload before deferring work.
- Reuse buffers and plans only within a proved identity, ownership and invalidation lifetime. Reloadable defines, weather and mutable definitions are not automatically session-static; a sizing hint is not a recorded value or validation of a later read.

Size variable capture storage from live demand. Use [allocation.rs](src/allocation.rs) for potentially large or live-sized buffers, reserve a known batch before filling it, and keep callback reads within caller-owned capacity. Preserve the existing growth boundaries when filtering determines the output count. Allocation failure must retain its terminal `OutOfMemory` distinction through the collector callback, including readers that otherwise return a static refusal or optional value. See the collector guide for accepted standard-library allocation residuals.

Preparing a read plan is fallible; executing it does not allocate. Its controls must cover every owned operand that selects later addresses. Hints run only after those controls pass and acquire no bytes. A stopped plan reports its completed prefix and attempted failure so continuation need not reread them. Valid geometry alone proves neither dependencies nor object lifetime.

## Adding a layout or reader

1. Establish the native writer, getter or serializer with `C:\Tools\hoi4_mcp\hoi4re.cmd fn|asm|data|callers <addr>`. Derive widths, signedness, ownership, gates and arithmetic from that evidence. Inspect instructions when the decompile omits an argument or operation; distinguish virtual addresses from RVAs. Old readers are research leads.
2. Keep one source file per native struct and register it in [lib.rs](src/lib.rs). Cite the native address and what it establishes beside each constant or operation; keep a longer field audit beside the module when needed. Save established names, fields and notes through hoi4re.
3. Implement guarded reads with the existing containers, gathers and caller-owned arenas. Preserve independent failures and native reach: skipped branches should not acquire operands. Resolve content through [installed-content](../installed-content/README.md); built-in modifier names and fixed tokens use the witnessed layouts above, while dynamic definitions size and resolve from live data.
4. Add synthetic-memory cases that fail on the old behaviour: aliases, invalid identities, short reads, overflow, cycles where relevant and insufficient capacity. Follow AGENTS.md for scale and local checks, and the [recorder guide](../recorder/README.md) for the affected capture oracle and smoke workflow. Update the collector, replay and consumers together when the recorded meaning changes.

## Adding a hook site

Put the site's RVA, exact instruction sentinel, operand layouts and caller evidence in its domain module. Establish whether it is a function entry, CALL or interior instruction, and document the register/stack ABI, original continuation and safe observation phase. Pre/post readers must reauthenticate identities when the native call can replace or free objects.

Verify both [the pinned executable](src/build.rs) and the site's bytes before installation. A matching sentinel alone proves neither executable support nor hook, lifetime or unwind safety. An interior site must resume its relocated instruction and original continuation correctly; it cannot be invoked as a function. Collector owns patching, dispatch, teardown and runtime coverage. Keep engine operands separate from replay event IDs and never publish a native pointer as history identity.

## Native distinctions to preserve

[world_lifetime_sites.rs](src/world_lifetime_sites.rs) authenticates current-world invalidation markers before reset, deletion and construction publish or retire data. These markers supply occurrence changes to collector; they do not infer successful load completion from a clock or singleton address.

- [War overview](src/cwar_relation/war_overview.md) builds viewpoint-dependent temporary groups of bilateral relations. These have no stable native war-object identity or stored participant join dates.
- [Peace settlements](src/peace_settlement.rs) need the pre-destruction finalizer boundary: a conference can appear and disappear between hourly samples, and finalization destroys its action history.
- [Equipment destruction](src/equipment_destruction.rs) retains signed pre/post pool differences, including gain corrections and supplemental writers; requested removal does not prove actual removal.
- [Air loss](src/air_loss_sites.rs) and [building damage](src/building_damage_sites.rs) retain native bookkeeping and signed health changes, including gains above a full level, independently of returned budgets.
- [Event feed](src/event_feed_sites.rs) reauthenticates action roles and reciprocal war links after native execution; world lifetime comes from the collector's reset/construction signal, not from these reads.
- Cached rates and tooltip inputs retain native gates, stale values and source order. Their formula and lifetime evidence belongs in the corresponding reader; save serialization may omit those operands.

## Strategic-action operands

[carmy/strategic_action.rs](src/carmy/strategic_action.rs) supplies the current ladder glyph and ETA rail operands with their native gates and effective modifier queries. These consumers retain their own acquisition and cadence; carried ETA cannot supply current glyph inputs. A query failure withholds that row's ladder glyph. The save omits the effective modifiers, while serialized route, target and strategic truth remain independently comparable.

## Measurement

Measure acquisition with the current [read counters](src/read_counts.rs) and their definitions. Decoding an owned copy is not another native read. [Unit-scale fixtures](tests/support/unit_world.rs) and the opt-in scale tests model native walks; synthetic timings do not establish gameplay slowdown or live hook correctness.
