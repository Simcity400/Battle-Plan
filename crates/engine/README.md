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
| What can a save validate? | [Text-save parsers](../installed-content/src/save/) and the recorder's [oracle comparators](../recorder/src/oracle/); keep unsupported fields explicit. |

The struct files hold offsets, native widths and evidence. Do not recreate the old registry, table-count or format-version catalogs here. A stored field, a calculated getter and a renderer cache are different sources; a similar-looking save value does not make them interchangeable.

## Adding or changing a reader

Keep one source file per engine struct. Put a short evidence comment beside each constant: the native function or structure that establishes it and how it was confirmed. There is no arbitrary file-count cap.

Derive widths, signedness, ownership and meaning from native writers, serializers and established evidence. An old reader is a lead, not proof. Look functions up through `hoi4re`; save established names, fields and notes before reporting the result. If a decompile drops an argument or arithmetic, inspect the indexed instructions rather than guessing. Distinguish virtual addresses from image-relative RVAs.

Guard every native read. Preserve source order, identity checks, meaningful pointer occurrences, bounds and the first independent failure. A failed or short read must not become a complete row, an empty table or a zero value. Never call engine functions to obtain a value, wait for memory to change, or retry to hide a failure.

The collector may decode owned copies after the callback. Those copies must be sufficient: deferred code must not follow live game pointers. Shared acquisition is useful only when each consumer retains its own correctness and failure behavior. Address equality alone does not identify a table or read occurrence: keep raw/owned lookups correctly scoped, including empty and overflow paths.

## Reuse and measurement

Keep cached plans and copied buffers within their proved lifetime. Reloadable defines, weather structures and mutable definitions are not automatically session-static. Reuse native inputs only when identity, ownership and invalidation are established.

[read_plan.rs](src/read_plan.rs) executes fresh exact reads with ordered comparisons of the owned values that selected later addresses. The native executor uses compact checked descriptors; geometry outside that representation keeps the ordinary ordered executor. Hints run only after their selecting controls pass and never acquire a value. A stopped plan exposes its completed prefix and attempted failure so a producer can continue without rereading it. Bounds validation alone does not prove a plan's dependency coverage or object lifetime.

Use the instrumentation present on the checked-out branch and its actual counter definitions. Do not count decoding an owned copy as another native read, or report synthetic fixture savings as measured gameplay speed.

Test readers with the existing synthetic-memory fixtures, including aliases, invalid pointers, short reads and capacity limits. Run the local checks in [AGENTS.md](../../AGENTS.md); a new capture domain also needs collector/replay integration and an oracle comparison or an explicit limitation.

## Historical evidence

The [archived domain index](#) preserves old registry-to-source mappings, exclusions and investigation results. It is not current coverage or acceptance: its double-read windows and phase gates are superseded by [AGENTS.md](../../AGENTS.md). Keep new field evidence beside the reader, not in another running porting log. For the hourly-sample versus later-save limitation, see the [recorder guide](../recorder/README.md#inspect-and-compare).

`research_inputs.rs` captures current slot-local progress/cost operands and applied limited-use bonus IDs. `climited_use_tech_bonus.rs` owns the source layout and serializer evidence. The slot multiplier is assigned once and is not a current countdown. The reader performs no game calls or forecasting. Synthetic fixtures exercise short reads, identities, truncation, lazy bonus lookup and guarded batch counts; they do not establish live capture cost or fresh-save agreement.

`modifier_definition.rs` copies the registered rows of the engine's modifier definition table (script-name token, name and postfix localisation keys, `$ARG$` argument, flags, category mask, feature selector, precision); a row is registered when its own index equals its position and its token is nonzero, and a default gap row is skipped. The table header (data pointer, count) is the copy's validity control. `key_by_name` resolves a key index from a script name through these rows and the lexer's reverse dictionary, so readers need not hard-code key numbers; it reads every row, so callers resolve once per header. `ccountry_characters.rs` reads each country's appointed advisors (CCountryCharacters +0x58) in native order; a persistent advisor role alone does not mean appointment. Synthetic fixtures cover both readers; live capture and fresh-save agreement are unverified until run.

`country_stability.rs` copies each country's operands of GetStability, GetWarSupport and GetMaxCommandPower (bases, stored Q15 penalties, the cached coastal term, ruling-party popularity, the war predicates over the owned war relations, the interception ring sum, world tension, the loaded defines, the active language and the getter values of tracked modifier keys resolved by script name through `modifier_definition::keys_by_names_at`). It also copies every child of the country modifier tree with its exact tracked values, and replays RecalculateModifiers' source order for ideas, the ruling party's leader and ideology group, appointed advisors, the bound statics, dynamic modifiers and the scaled statics. Only children with a tracked value and sources with a tracked key take part in pairing, so sources and children without values (tooltip-only ideas, dynamic modifiers whose values are all zero, zero-valued faction children) cannot shift it. A child is paired only when its nonzero entries (and, where the clone copies it, its display text) equal one source's and the leftmost and rightmost order-preserving assignments agree; every other child stays unpaired with its display text. Synthetic fixtures cover pairing, ambiguity, drift, truncation, short reads and the fingerprint; live capture and the pairing coverage on real countries are unverified.

The hourly research modifier reader follows only assigned technology links from the same acquisition pass. It copies shared country speed/source trees once, slot-specific idea/script and design-team operands, XP boosts, matching production licences, sharing group operands and selected scientist-category bonuses. Sharing uses bounded direct template-index lookups for raw member levels and predecessor classifiers, not a technology census. Native group filters, doctrine flags and scientist category values remain separate inputs; analytics derives eligibility, contributor counts and the combined scientist contribution. Modifier metadata retains the native flags and feature operands so eligibility and clamping also belong to replay. Applied-bonus roster scans have a finite bound and stop once the requested IDs are resolved. The scientist source list records efficiency modifiers separately from each native cached category value; those values must not be summed as direct technology-speed contributions. Native formula evidence is beside the reader. Synthetic tests cover signed values, missing reads, output capacity, ownership, aliases, source cycles and conditional read cost. Fresh-save, live-rate and performance acceptance remain unverified while the game is left untouched.
