# dashboard

Everything a widget needs natively: opened recordings, data products, the host/viewer protocol and the data service that answers the frontend. No map pixels are drawn here and no live game memory is read.

Opened metadata carries the replay's terminal partial marker separately from branch gaps. The verified prefix remains readable; the marker keeps the failure reason and an optional attempted hour. An unknown failed hour adds no inferred gap. Recording-library summaries read only the validated replay trailer, leaving frame blocks to the normal source-open path.

Readable older recordings keep unchanged field prefixes. Products count a named refusal only when they need an appended field that was not recorded; ordinary names, colours, flags and scopes remain usable. Layout/meaning changes still withhold tables. See [replay compatibility](../replay/README.md#compatibility-and-recovery).

## Layout

| Module | What it does |
|---|---|
| `source.rs` | One opened recording: replay bytes, metadata, resolved game content and each product's per-source cache. |
| `catalogue.rs` | Content facts every product shares, read once per source, folders and game language: the localisation (the same parse country naming uses, from `content::localisation`; every text goes through the shared game localiser in `installed_content::localisation`; a refusal, such as a scripted expression or a control plain text cannot show, is kept apart from a missing key and noted by reason and key for the product warnings) and interface sprites, plus the texture decoding, within a budget, and PNG encoding every image reply uses. |
| `triggers.rs` | The one trigger evaluator: raid, combat-tactic, scripted-localisation, equipment-graphics pool and adjacency-rule triggers decided from one recorded hour. Each key is dispatched as `CTrigger::LoadKey` does (`installed_content::trigger_keys`), each coded trigger checked against its class's accepted scopes first. Only the children LoadKey adds count: a key that names nothing adds none, `else`/`else_if` attach to the last `if` added, and `World::has_children` is the emptiness test every consumer reads. A key names a character by the content's character templates; scope keys resolve through `scope.rs`, and deeper links use the caller's `EventScopes` graph when supplied; flattened callers still refuse a link they cannot describe. Tag aliases resolve as `AliasResolve` does (`triggers/aliases.rs`). Script variable operands (`check_variable`, `has_variable`, `var:` keys, variable counts, country and state operands, arrays and tag alias variables) are evaluated as `CScopedVariable::Evaluate` does over the recorded `ScriptVariables` stores (`triggers/variables.rs`); unreproduced built-in game variables, random draws, unsupported temp writers and outside-context residual temps are refused by name. A combat scope's own triggers come from its caller (`Combat`). A caller without a country uses `Scopes::empty`, the default event scope: no carried objects, and THIS/ROOT/FROM/PREV all empty. Scope-masked triggers are false there; country-tag scopes resolve normally over the recorded World. A context trigger targeting an empty scope is false by its own mask, including an empty ROOT block. What the recording cannot answer is `Eval::Unknown` with its reason. |
| `scope.rs` | Script scope targets resolved over one recorded hour (`RecordedScopes`), including the faction leader from `FactionMembers` order for triggers and scripted text, with `_CountriesForOriginalTags` derived from the branch (`scope/original_tags.rs`). |
| `metadata.rs` | Branch ranges, gaps and the first readable country catalogue, and the helpers every product shares: frame checks, a tag's country this hour (`tagged_country`: found, absent, or unresolved when the links cannot tell which country holds it; `tagged_countries` gives every tag's in one pass), a raw link's canonical slot (`link_slot`) and which hours read a daily table (`slot_read`: its cadence slot hour, or a timed walk within its slot on a forced full hour; CountryIntel requires its own measured walk and is no evidence for its slot peers, as in the fresh-save oracle; a product dating values by their read uses it). |
| [cryptology.rs](src/cryptology.rs) | Native cipher thresholds, full-decryption predicate and progress meter frames, shared with viewer raid tooltips. |
| `products/` | One module per data product, registered once in `products/mod.rs`. Helpers products share sit beside them, such as `resources.rs` (recorded resources in the game's order, named and pictured through the content stack for every resource product). |
| `read_runs.rs` | A daily table's keyed values as runs over the hours each read covers, with the spans where the table is known at all. A recorded zero is a run, told apart from a key the read did not list; a value read but not computable is a run with no amount (unknown, never zero or part of a sum). Products turn daily tables into runs here. |
| `products/equipment.rs` | Equipment facts every equipment product shares: units per week by the game's weekly formula (none without a positive cost, where the game shows only a placeholder), a variant's name (`GetDisplayName`), every variant's names over a branch with where they are known (`VariantCatalogue`: a gap, a due variant read that did not land, or an hour between reads whose variant table is not complete ends them; each read also fingerprints its name, modules and recorded upgrades, so a later branch's other design under the handle stays apart; `names` dates each by its read and never carries one back), an equipment type's archetype, database place and domain through the recorded content stack (`archetype_of`), that content's equipment types read once per catalogue for every equipment product (`units`), the mark keying a type content does not resolve (`UNRESOLVED_MARK`, and `UNRESOLVED_ARM` for its arm) and native reader diagnostics naming each file and definition (`equipment_warnings`). |
| `ranking.rs` | The top N countries at every hour a value changes, for every product that ranks countries (Factories, Equipment produced): a country with no value, or an unknown one, is not ranked, never as zero. |
| `service.rs`, `service/` | The service facade and its private modules for admission and leases, source and loader jobs, and the bounded response pump. |
| `transport.rs` | Data commands, the generic product request and fragmented replies. |
| `protocol.rs` | Control messages between the Tauri host and the viewer process. |
| `map_settings.rs`, `config.rs`, `content.rs` | Map preferences, game folders and content verification. |
| `bin/dashboard-bridge.rs` | Development-only stdio adapter for browser UI work. |
| `bin/export-types.rs` | Generates the TypeScript contract. |

Border-event descriptions share the evaluator's caller-owned `EventScopes` graph and active variable session. Scope switches preserve ROOT, FROM and PREV links and both native seed words. Description-only targets are stored by scope identity: saving follows ROOT, lookup follows FROM, and outer completion clears only terminal-FROM tooltip targets and global tooltip targets. `has_event_target` reads saved targets only. Scripted-localisation conditions and variable operands use the same graph and session. `num_owned_states` reads `CountryCounterInputs`; `has_template` compares native display-name bytes, resolving the recorded fallback key through the recording language and template country. Template names retain their recorded daily freshness. `any_owned_state` derives nonempty owned membership from `States` and `Provinces`; order-sensitive predicates refuse the unrecorded native roster order.

## Products

A product turns a source plus typed parameters into a typed reply. History products cover a whole branch, so playback reads them locally. Decryption uses exact daily linear runs checked against every hourly read, with separate tag runs and no values across gaps; it keeps this whole-branch contract. Hour-specific artwork and formula details use separate requests through the frontend's transient path, outside the history cache. To add one:

1. Add `products/<name>.rs` implementing `Product` (a key, `CONTENT_DEPENDENT` (whether any reply field reads game content), `Params`, `Reply`, optional `validate`, and `load`). Keep per-source caches in `source.state::<YourState>()`. Read localisation and sprites through `catalogue::catalogue`, never by parsing them again.
2. Register it in the `PRODUCTS` list in `products/mod.rs`.
3. Regenerate the contract and add the widget folder under `ui/src/widgets/`.

The generated contract also exports the products declared content-dependent; only those frontend cache keys include folder/readiness changes. The generated `CONTENT_PRODUCTS` list is the authority on which products those are. The service, transport and host route generic requests by key; widgets and their data helpers choose the product. Replies are serialized with the frontend's lease identity so they cannot be attributed to another recording. Products read verified recorded content only; missing localisation or artwork stays null.

Current products: national focus order, decryption, research slot history, country flags, country styles (each country's engine-built name and recorded map colour at an hour), country groups (human players, recorded factions with their members and leaders (the member at order zero), and war pairs, as runs of consecutive recorded hours), country balances (political power or command power: exact balance runs and net drops per country), party history (popularity, ruling runs and elections per country, with content ideology colours), idea history (each idea a country held, joined to its content category, slot, effects and artwork, with how the game formats each effect's modifier (`idea_format.rs`: the recorded modifier definitions, else content's scripted ones); `politics_content.rs` reads the Politics content once per catalogue), country stability (displayed stability, war support and maximum command power computed from recorded inputs with the engine's arithmetic, plus an hour's formula terms, tree lines with their source identity or recorded text, and an explicit remainder), stability parts (each of those parts of one country over the branch, as runs, known only where recorded), resource balance (one country's recorded resource pools and trade deals as hourly runs, with content's resource names and icons), equipment stockpile (one country's recorded logistics surplus or shortage per equipment archetype with the stock the same record counted, the hours it was AI-controlled, and its recorded stock and weekly production per variant, grouped by content's archetypes), trade flows (every recorded resource trade deal, or the chosen countries', as seller-buyer-resource flows over the branch, with the hours their deals are known and those known only in part, where a read listed deals it could not attribute; bounded by a run budget that refuses rather than cuts), equipment produced (every country's units delivered from the exact per-line output, each row's last count kept so deleted lines still count, as a running total or the week or month before, grouped by content's archetypes, exact variant or domain, for chosen countries or the top N, the reply delta coded, with the hours at least one requested country has an exact total (including zero) and no requested country with a piece at that hour has an unknown total (before ranking), and its size counted before it is built), land ownership, country fuel (one country's tank level and capacity at each daily read, and the fuel gained and burned in the 24 hourly updates before it, each update counted once and only while the game updated it) and subjects (every overlord–subject relation as runs of consecutive recorded hours, with the engine's start hour and the subject's current autonomy level through each run, levels named and ordered by content's autonomy states), and country factories (the three factory groups hour by hour, as whole factories the game shows, with their content building types named, for up to 30 chosen countries or the top N ranked here at every hour, each cut to its hours in the top N, the reply's size counted before it is built), world tension (each daily read of world tension and of every country's contribution to it, and the threat sources as events with the game's own text) and autonomy (each subject's level, overlord, score and move-down and move-up scores, hour by hour, with the levels ordered and named by the content's autonomy states), and production lines (one country's recorded lines at each daily read with their efficiency, the units each delivered as the collector counted them, each produced variant's names and icon as the game's production row shows them, and each organisation's name and icon; `production-lines` sends each line's factory steps and final count, `production-line-detail` its reads one 30-day window at a time, so neither reply grows with the campaign, and each is measured with `transport::json_len` and refused past its bound before it is sent). `equipment.rs` and `equipment_icon.rs` hold what equipment products share: units per week, variant names, domains and the production row's icon rule. A product about a whole branch takes `BranchParams`. `catalogue::display_text` turns a string the engine built for display (a stability tree line, a threat's reason) into plain text, and the construction log (every building and railway line of every country's construction queue in a branch: when it started, the levels its target's hourly building record shows it built, and whether it finished, was removed or left unknowably, each known only from the read that shows it, with content's building names, queue icons and state names), and construction spending (one country's construction progress from the same cached scan: the IC each project's `produced` and the new levels its building's rises confirm show between its reads, per stretch (a line can repair, then build), running totals per repair or new kind, building type and state, with every stretch the reads do not establish counted by reason and repairs refused as not measured). Their rules live beside their code and in the matching widget README. `metadata::LinkTags` names a scan's raw country links by tag hour by hour, rebuilt only when the country tables change.

The [`production-spending` product](src/products/production_spending.rs) supplies
one country's domain → content equipment type → exact design hierarchy as
bounded fixed5 running totals over recorded hours. It shares Equipment produced's
single per-source branch scan and cache in
[`equipment/production.rs`](src/products/equipment/production.rs), including
producer attribution, retained deleted-line output, ship-refit selection,
naval and railway-gun delivery rules, recorded design names and content resolution.
No country is excluded by player or AI status. Histories and replies refuse
their budgets rather than truncate; large totals use decimal strings of checked
128-bit integers.

Its unit is **Delivered IC at read cost**: each new whole delivered unit in
`ProductionLineOutput` is multiplied once by the matching producer, variant and
organisation's `ProductionLines.cost_raw` at that read. Earlier units are never
repriced, and progress awaiting delivery is excluded. Summing recorded speed
would not measure exact factory output: military `ProduceEquipment` at
`0x140F596C0` and naval production at `0x140F59ED0` replace speed on their instant
paths. Nor does a census establish the historical delivery price:
`SetEquipmentVariant` at `0x141926570` can change design and cost between passes,
and `RefreshProductionState` at `0x141926DB0` refreshes the MIO-adjusted cost;
ordinary delivery reads it at `0x141921B80` / `0x141921C10`. The widget therefore
states the read-cost valuation explicitly.

A new delivery without a matching cost is **Not known**, including a refit
removed in the finishing pass (`0x140F5CA10`). A deleted line's final publication
with no new units needs no cost and retains its earlier exact value. An inexact
output row, a lost final publication, an explicit or implicit recording gap,
an observation restart, or a change of country identity ends what can be known;
later reads never invent the lost amount or resume an exact campaign total.
Unresolved equipment keeps its recorded token in the shared unresolved arm;
the country's independent total can still be exact. See the
[widget guide](../../ui/src/widgets/production-spending/README.md) for playback,
settings and display rules.

The `occupied-states` product supplies the whole branch's active occupied-state
inventory as equal-value runs, with separate daily read dates and gaps. Daily
occupation joins freeze the read's controller and country links; other hours
project only owner/name changes. It uses
the shared state-name precedence and recorded occupation-law content. The
unrecorded country-level law pointer prevents guessing a default; displayed
garrison strength also needs unrecorded equipment allocation/requirements.
The native garrison amount and failure marker are recorded; the bar is known
only for the failure flag or complete zero assigned manpower. Those
limitations remain explicit in the [widget guide](../../ui/src/widgets/occupied-states/README.md).

The `occupation-trend` product shares that cached branch scan and its daily
joins. It supplies every occupier's readable-state mean, using the native
unweighted fixed5 arithmetic over recorded scalars, with covered/held counts,
separate read dates, identity segments and gaps. Chosen countries and top-N
requests use shared ranking and `MAX_PRODUCT_REPLY_BYTES`; no country, entry,
scan or run count cap applies. Top requests refuse zero or a simultaneous tag
selection. Country collaboration is optional tooltip context,
joined from the occupied country's relation toward its occupier. The shared
reader builds one unambiguous country index for both occupation consumers and
keeps it through hourly balance changes; unchanged projections extend endpoints
without cloning labels or context. Numeric-only replies omit collaboration and
merge its additional boundaries. See its
[widget guide](../../ui/src/widgets/occupation-trend/README.md) for the distinction
between these read-derived means and the game's unrecorded cached group means.

The Experience widget reads [`country_experience.rs`](src/products/country_experience.rs): banked army, navy and air experience at each read, and their net change from a read one day earlier in the same unbroken country stretch. Its [widget guide](../../ui/src/widgets/experience/README.md) describes the display and missing-data rules. Multi-country line products share selection validation in `products/mod.rs`.

Equipment losses (one country's losses as cause, equipment type and design, each a running total: the Combat Log's reasons and recovered equipment for land, SunkShips, SunkConvoys and lend-lease sunk and sent, with air reported unknown; hours the game did not log the country's land losses are unknown, not zero; each cause lists the hours its record was not read, and lend-lease the exchanges that ended after their last read, whose totals are lower bounds from then; a Combat Log retention define below 1 makes land unknown by name; cause names the executable does not name come from the content's interface files). Daily values change only at their recorded read hours. Recorded gaps, failed blocks and the unread remainder of failed frames have explicit unknown coverage and unread spans. CombatLogState explains missing native land records and never selects capture; the product also checks division orders-group membership, since country-gate admission alone does not prove logging. A mixed census is Partial with the number of that country's divisions outside orders groups: the game kept losses of divisions in armies, not of those divisions. The native equipment writer also tests the runtime feature mask recorded by SimulationFeatures: a closed feature gate is NotLogged, missing writer inputs stay Unknown, and every captured amount is still counted. Combat Log observation checks the requested country's slice reads before accessing its sorted row ranges. The product selects the hourly DivisionCensus counts instead of Divisions. The collector accumulates every country's counts in its existing authenticated division core projection, preserving the writer's raw expeditionary or roster owner; aliases resolve through CountryLinks. Coverage is dated by each hourly census read, including equal successful reads; consecutive hours merge only for equal states and equal ungrouped counts, and an unavailable hour is Unknown. Missing country rows, unread counts or unresolved nonempty owners remain Unknown. Wire decoding and reconstruction of the selected tables remain part of the scan cost. The ignored release test `equipment_losses_observer_scale` measures observation of shared decoded observer frames; `equipment_losses_observer_scale_including_decode` keeps the same major-sized division workload in the wire recording but omits Divisions from the selected decode, and separately measures the Combat Log/SliceReads decode. Live detour cost remains separate. Its rules live beside [the product code](src/products/equipment_losses.rs).

Special projects adds per-country project episodes, native prototype progress and localized completed-iteration counts, exact or bounded dates, and shared character names and portraits.

The Decryption widget reads [`decryption.rs`](src/products/decryption.rs): sparse pair runs with same-hour thresholds, native progress, bonus states, each current attempt's observed start, dated breaks and capture coverage for every recorded pair independently of player status. Its [widget guide](../../ui/src/widgets/decryption/README.md) describes the display and missing-data rules. Gaps interrupt reads within an episode.

The Intel products [`espionage_log.rs`](src/products/espionage_log.rs) and
[`agency_comparison.rs`](src/products/agency_comparison.rs) include every
country. Operation episodes retain recorded launch dates, civilian preparation,
read-time execution progress and phase indices, assignments and bounded endings; missing reads break
runs. Completion does not identify an outcome because the game's result text,
bonus and operative-detection operands are not recorded. Agency comparison
uses the agency screen's stored active count and maximum, rather than its
recruitment admission ceiling, and localises defence through the content's
formatter. Acquired upgrades are **Not recorded**; `upgrades_at_read` is the
single adapter for their future capture. Agency values compress independently
of the roster counts and their shared daily read dates. Both products refuse oversized replies
and share [`intelligence_content.rs`](src/products/intelligence_content.rs)
for content names, artwork and agency text. Top-N selection and run clipping
use [`ranking.rs`](src/ranking.rs), also used by Factories. Their UI rules live
in the [operation log](../../ui/src/widgets/espionage-log/README.md) and
[agency table](../../ui/src/widgets/agency-comparison/README.md) guides.

Nuclear arsenal reads [country_nukes.rs](src/products/country_nukes.rs): every
country's whole stockpile as the native top bar displays it, with native type
ordinals named through recorded tokens and content. The queued-drop `ready`
counter is a different quantity. [Raids](src/products/raids.rs) shares one cached
branch scan between the [Raid log](../../ui/src/widgets/raid-log/README.md) and
[Raid tally](../../ui/src/widgets/raid-tally/README.md), preserving observed dates,
target controllers, phase/outcome localization and bounded observed totals.

## Service

`Service` owns sources by canonical path and revision, hands the frontend independently releasable leases, and runs product and open jobs on a small loader pool. Requests for the same product on the same source serialise on that product's cache; different products and sources run in parallel. Replies are bounded, fragmented and cancellable at every step. The map process shares the same sources through `open_map`. [transport.rs](src/transport.rs) owns the retryable refusal catalogue, including the browser bridge and native command queue capacity messages; the generated frontend constants and desktop queue read that catalogue. A capacity refusal decides no country fact and is retried.

A refusal for want of room says nothing of the product: the frontend tries it again. Other errors remain failures until an explicit retry or a content/configuration change; they never become answered empty facts. Every such refusal, the service's and the hosts' alike (the desktop's admission, its queue to the native service and its map controls, and the browser bridge's admission), starts with one of the named constants in `transport::TRANSIENT_ERROR_NAMES`, which the contract exports with `TRANSIENT_ERRORS`. A new queue or capacity limit adds its constant there and builds its message from it, never from its own text; `transport::transient_error` and the frontend's `transientError` classify by those constants alone.

## Types

Rust owns the generated frontend types. Use the shared [build commands](../../AGENTS.md#build-commands):

```powershell
bash tools/build-slot.sh "dev/dashboard" "cargo run -p dashboard --bin export-types"
bash tools/build-slot.sh "dev/dashboard" "cargo run -p dashboard --bin export-types -- --check"
```

Generation updates `ui/src/generated/dashboard.ts`; do not hand-edit it. Follow with the checks in [AGENTS.md](../../AGENTS.md).

## Rules

Preserve recorded values exactly and keep missing, unavailable and zero distinct. Never bridge a gap or carry a value across missing hours. Opening with an outdated expected revision is refused. A changed revision uses a separate source and caches; product replies identify the lease, revision and product key they answer.

The initial country selection uses LocalPlayer at the first readable country catalogue (including an observer's selected country), joined through that frame's CountryLinks. The human roster does not determine the recording client.

Country flag shields and resource icons bind templates through `installed_content::gui::native` over the complete recorded content stack, then use the native window-local button lookup and inherited sprite. The catalogue caches the GUI registry and the configured user's `settings.txt` texture decode result once alongside the sprites. A missing or unreadable settings file leaves the catalogue available, with localisation in the recorded language and a warning; every item needing an unknown decode setting reports that reason. Focus, technology, idea and portrait artwork share the shields' and resource icons' [native texture resource path](../installed-content/src/gui/native/resources.rs), including PNG decoding and `force_pow2_textures`. A file past the game's own 16 MiB `load_file` limit is refused as the game refuses it, and an image whose decoded texels (after `force_pow2_textures` padding) would exceed the dashboard's budget (`catalogue::MAX_TEXELS`, sized from the largest sprite texture across the installed mod corpus) is refused by name from its header, before it is decoded. One frame helper uses the native integer extent and fractional pitch over the padded texture; sprite artwork takes the sprite's own frame count and direction, and an animated, masked or custom-effect sprite is refused by name. Missing candidates, undecodable files (which the game also draws without a texture), undefined reads, cube maps and device-dependent outcomes retain their distinct reasons in product warnings. A refused shield returns a warning and no flags. Country flag Targas keep their separate `installed_content::flags::decode` path.

Scripted text uses [scripted_text.rs](src/scripted_text.rs) and the shared [recorded trigger evaluator](src/triggers.rs). Built-in functions read the sampled party leaders, gender, manpower operands, agency, old country names and balance-of-power operands; content-defined entries select their first passing text alternative. Missing inputs and unsupported trigger or description forms remain named refusals. State template keys come from the recorded content layers. Text the game stored already localised or as a literal, without its key (a created agency's name, a state's custom or owner name, a country's retained old names), is exact only in the recording client's language: it is shown when the viewer's language is that one and refused by name otherwise. A caller with its own event scope passes it with the scopes it links to and its script variables (`EventScopes`): each bracket starts from it, ROOT, FROM and PREV move to the linked scope, and a content-defined entry is evaluated, and its text expanded, in the scope the game binds for it. A character it promotes is a character scope, where `tag`, `skill`, `has_trait` and `has_character_flag` read the recorded character inputs through [character_scope.rs](src/character_scope.rs), which the tactic evaluator shares; any other trigger there is checked against its scope mask first, and one that passes but is not reproduced is refused by name. A link or scope object the caller cannot describe is refused only where a promotion or a reached condition reads it. Native dispatch addresses and exact refusal reasons live beside each property.

GetPowerBalanceModDesc renders the cached daily then weekly source values, with ordered static modifier keys localised through the recorded content stack. Its numeric and tree formatting is shared with the viewer in [modifier_text.rs](src/modifier_text.rs).

Active-range modifier and changed-rule descriptions use authored balance ranges
from [installed-content](../installed-content/src/power_balance.rs), recorded modifier definitions
and the same value formatter. Wrappers and custom tooltips expand in a fresh
country scope. Feature gates use the recorded simulation mask; missing masks and
ambiguous range identities remain named, counted refusals.
Activation/deactivation and the viewer's border-war event tails use one
[effect description dispatcher](src/effect_description.rs). Native `effect_keys`
class identities make aliases share a dispatcher row; unknown keys and unported
classes are counted by name. Domain rows are grouped in
`effect_description/dispatch.rs`; the core's module documentation defines the
shared porting entry points. It ports the reached
country/state leaves, numeric fixed5 values, faction and doctrine descriptions,
targeted decisions and timed ideas, with the native conditional/context/state-list
builders. Own lines use three spaces per level; `IndentNonEmptyLines` uses one.
`modifier_description_with` exposes raw `FormatDescription` text; `modifier_list_with`
adds the native scoped expansion. `ModifierListOptions` maps requested precision,
outer colour, include/exclude category masks, numeric-line separator, fixed5 scale,
forced plus signs and colour inversion. Defaults keep the existing formatting
arguments. The separate prefix applies only to numeric lines; custom tooltip
entries add LF only when needed. Zero visibility stays definition-controlled,
after native getter clamping and wrapping fixed5 scaling. The options documentation
maps every native argument and gives the trait, advisor and scientist call values.
Context layout repeats native description probes instead of counting rendered
newlines; a failed probe retains its named leaf causes once. Unattached conditional
continuations are named refusals. State leaves require the complete state roster
before applying native invalid-state behavior. Generic builders and probes share
the native own-key projection: an empty own description still visits effect children.

The shared scope iterator builder lists at most four targets, or the first
three plus the native others text when five or more pass the limit. It describes
the body in the first target unless individual scopes are requested. Random
selectors and subset removal advance the caller's native scope seed, including
singleton random picks. Individual descriptions and their tooltip use one copied
scope across targets, carrying its RNG and targets. A random-selector layout probe
advances the same copy before rebuilding; its refusals count only once.
Prioritized random states retain roster order and fall
back to the full roster only if no prioritized state passes. Country, faction, subject, state, character, scientist
and script-array rosters retain their native recorded or content order. Missing
country neighbor/occupied rosters, division roster order and scientist
activity produce named refusals rather than invented values. Owned/core/controlled
states and army/navy/operative leaders derive recorded membership; only two or
more passing targets need the missing order. Owned membership is checked against
the recorded count. Original-tag lists
refuse ambiguous order; party-leader selection refuses the unrecorded current
role ideology; without the mandatory ideology predicate it returns empty.

MIO selectors use the recorded roster and reference handles,
the shared evaluator's organisation scopes, and the content's own or directly
inherited visibility trigger, including deleted inherited values. Their variable
store is not recorded; dependent conditions retain a named refusal.

Character-list military sections refuse their unrecorded
assignment references; advisor-only lists use the recorded roles and traits.

Scripted-effect bodies load through the recorded stack's registered templates;
calls describe the template regardless of the execution boolean. Recursive
calls into different THIS scopes expand up to a counted 64-level port backstop;
re-entry into an active template in the same THIS scope remains a named recursion
refusal. Meta text uses the native effect reader after
case-sensitive parameter substitution.

Random/list descriptions share the
existing evaluator, variable session and seeded graph; every weighted member
renders, including zero weights, with both seed words restored per member.
For-each-array descriptions use the first decodable object only when a tooltip
is authored. Collection descriptions resolve named collections and country/state
constants through content, use the first successful representative with fresh
native scope seeds, and expand their header in the caller's environment.
Zero-reach classes remain named, counted refusals.

Variable descriptions share the trigger evaluator's operand parser, arithmetic
and description session. Persistent scalar effects evaluate RIGHT then LEFT
only for an authored tooltip; temporary scalar effects execute before that
description. Temporary clamp and modulo execute and emit no text. Array,
round/randomize and persistent builder descriptions remain native empty setters.
Effect division reads `if_zero` from TLS even for a nonzero divisor; unlike
trigger division, a zero divisor stores that fallback rather than preserving
the destination (1403554E0 / 1405389B0).
Global description targets use the separate tooltip registry and existing
completion cleanup; a combined scope saves its state before its country.
Temporary country scorers load through the recorded content stack, reuse shared
CScriptTargets/MTTH evaluation and retain native highest/sorted tie order.
Array target filters run before the next indexed value is evaluated. Temporary
random ranges refuse the unrecorded stored-variable seed except for native
reversed-range no-ops. Temporary supply truck/train totals remain named,
counted recording gaps; unknown supply types store native zero. Scorer dynamic
tails and state-target modes are named refusals with no reached temporary
scorer form in the audited corpus.

Hidden effects build and discard text while retaining failures.

The D2 empty family dispatches all assigned native classes to empty own text
(`140176EE0` / `1402E9B40`), including the four empty build overrides. Setters
never execute during description. `create_dynamic_country` and targeted
`force_disable_resistance` share the generic child walk (`140532780`) and
multiple-description probe (`140533D80`); their children describe once while
their childless forms describe nothing. Railway and targeted resistance
descriptions use the same core traversal after their own text.

The D2 states family dispatches its 43 assigned classes. State/country changes,
capital and state naming, resistance/compliance, shared slots, supply, province
navigation/controller and resource rights use native operands, names, keys,
expansion and LF rules. Invalid targets retain each native empty or error branch;
zero state operands remain zero. `occupied` reads `StateResistance.country`,
including inactive rows. Descriptions require complete recorded rosters.

All scalar-state rows, including claim and transfer, use core `state_target`;
literal zero stays zero (`1403A1950` -> `140533BE0` -> `14052FA10`). In state
scope `capital` selects the owner's capital; in country scope it keeps zero.
Core `country_target` uses native effect selectors: linked states promote their
recorded `States.owner` (`14052B6A0`, state +0xC8); owner/controller/occupied
read the current state, while capital/random keep country zero. Saved targets
probe terminal-FROM local tooltip, local saved, global tooltip, then global saved
(`1405302E0`), reading country +0x0C independently of state +8. Invalid nonzero
linked states remain refused. Projected character, leader and province graph
targets retain named refusals pending core saved-target liveness plumbing.
State saved targets read +8 directly (`14052FA10`), without resolving an unused
country or retaining caller scope through a context projection. Repeated block
targets with selectors retain a named auxiliary-reader refusal: their native
sticky +0x24 flags (`140530BB0`) are not reconstructed from the final field.

The effect content cache also reads declared building names, resource icon frames
and `NSupply::MAX_RAILWAY_LEVEL` once per stack; its define read is shared with
idea content. Reader notes and independent gaps are retained. Construction,
offsite and random provincial buildings, explicit building damage/repair,
typed state building removal, building-resource changes, resource additions,
railway guns and unfiltered explicit railway paths use that content. Resource
changes distinguish the icon frame from the recorded resource database index.
One-province railway paths are accepted; unfiltered paths do not inspect interior
province IDs. Building names keep their declared case and native `_plural` key.

Province VP, rename and reset effects share the distinct effect display accessor
(`140E69320`); ordinary scripted `GetName` retains `140E67930` behavior. Effect
display returns a nonempty raw override even when its localisation is empty,
then tries VP text and the state name. Country adjectives reuse country naming.
Of the 43 states classes, 42 have an output or native empty branch; state category
is the sole all-form refusal, with no reader for database `1413AFAC0`.

Named, counted form refusals remain for generic child traversal, zero
colored-percent formatting, marked country names, tag-selected building targets,
provincial removal's unexposed land flag, strategic-location templates and
computed/filtered railway paths (`140E7E560`). Differing building declarations
and unresolved signed resource frames retain content refusals. State-less
province display needs the unrecorded strategic-region pointer and instance
name; country-zero naming/adjective/capital inputs also remain absent. Native
null dereferences, uninitialised operands and unsupported auxiliary forms are
refused precisely. No recording format or acquisition changes are made.

Effect conditions and values share the trigger evaluator and its full scope
graph. Description-only targets save through terminal ROOT and look up through
terminal FROM. Separate immediate/option calls preserve caller storage but clear
only terminal-FROM/global tooltip targets at each completion. Ordinary setters
do not execute while being described. Faction subject order, template fallback
names and targeted-decision seeds read their replay operands at each table's
existing cadence; decision definition seeds derive from the authored identifier.
Content databases are cached per stack, with independent reader failures refused
only by dependent leaves. Referenced event titles load by ID and never recursively
describe their event bodies. Corpus-only effect classes, bound custom tooltip
forms and nonempty timed-idea research/equipment/trait forms remain named,
counted refusals; doctrine technology operands are
also refused by name rather than dropped from a folder-only description.
Character effects resolve names and roles through the cached content stack and
recorded character/leader rosters. Operative-event titles use the ordinary event
catalogue; missing caller inputs and nested option readers requiring an unavailable
scope helper remain named, counted refusals.
Balance callers keep their strict
refusal policy; event tails retain known native lines alongside refusal counts.
Neither balance effect entrypoint is called by the installed localisation corpus.

MTTH evaluation tests obtain weight fields through the native tactic reader, so the modifier's own `factor` and `add` operands are read before its trigger conditions (140545820).

Scripted entries restart ProcessString at depth zero and reject repeated effective contexts; growing histories and fresh-context nesting are refused at our 10,000-level backstop on a dedicated large-stack thread.
Effect descriptions retain that caller's entry history, fresh-context depth and
large-stack state. Their evaluator and World are acquired only when an operand
or condition needs them, so custom tooltip text can render without those inputs.
Balance context headers use the viewing country's perspective.

`opinion@target` evaluates its country target in the native callback scope and reads the directed cached opinion from RelationOpinions, at that table's daily slice cadence. A missing nonzero row in a complete table means zero; country zero uses an unrecorded reserved relation slot and remains a named refusal. `num_owned_controlled_states` counts nonempty owned states whose recorded controller is the same country, including country aliases. Both callbacks precede stored keys; unavailable inputs remain unknown. MIO and special-project variable callbacks, outside-context residual temps and other unreproduced built-ins remain named, counted refusals.

Trigger-local temp assignment, addition, subtraction, multiplication, division, clamp, round and modulo execute in order over one evaluation's store ([variables/temps.rs](src/triggers/variables/temps.rs)). Nested checks share it; the next outer check clears it (1405354A0 / 140544190 / 140535830). A destination is inserted before its RHS, then read after the RHS and any alias conditions. Modulo (14047B280) instead caches the floored, signed32 target before its RHS, rounds the divisor to signed32, skips a negative target's RHS and preserves RHS mutations for a nonpositive divisor. The selected localisation's `random_list` weights and seed use that same evaluator. Arithmetic wraps at 64 bits before fixed5 division, round sends halves away from zero, zero division preserves the destination, and clamp applies max before min. Existing native trigger/effect readers supply the parsed items; no parallel reader is used. Core `CExpression` arithmetic is supported; other operators and repeated nonliteral operand reads remain named refusals. The separate `[?...]` text formatter remains unported.

`set_temp_variable_to_random` remains a named, counted refusal: its TLS scoped-variable RNG (140537810, called by 14047B530) is initialized by 1405354A0 from unrecorded CVariables+8 words and scope/root hashes (140536DF0 / 140529EE0). The native effect reader's ambiguous bare scalar also retains its counted `Deferred effect` refusal (14047EF60 / 140481030). `randomize_temp_variable` is effect-only (registrar 14001BE20), so direct trigger bodies keep native invalid-key dispatch. Its own description (140176EE0, invoked by 140532780) is empty and never executes the setter; an actual effect-execution consumer is not ported. Its executor (1403452E0) draws from CEventScope+0x0C/+0x10 via 14039D050 or 1403A82C0 / 1424DB9D0, with the constructor seed derivation used by `random_list`. Neither random form uses the global game RNG. No outside-context residual temp history is inferred.

Explicit province text contexts use recorded province names, state ownership and perspective, with terrain from the recorded content stack. The native event scope cannot carry a province, and province text objects do not bind scripted entries.

`random_list` selection follows the native scoped seed and wrapping integer pick in [scripted_text/random.rs](src/scripted_text/random.rs). Default and promoted scopes derive their seed from their native constructors. Weights and explicit seed operands use [triggers/scripted_localisation.rs](src/triggers/scripted_localisation.rs): the native weight function applies every modifier without testing its conditions, accumulates additions and factors separately, and stops on a zero factor. A single member bypasses weights and seed; a nonpositive total returns empty text. `num_of_controlled_factories` reads the three stored group-head levels from hourly CountryFactories, sums them with native i32 wrapping and converts the result to fixed5. An unavailable table or absent country row is unknown by name. The content corpus does not evaluate recording-dependent operands.

Saved-global event targets resolve from the recorded registry in native first-match order. Scope keys, country operands, scoped variables and tag aliases use their respective native lookup paths; missing, cleared and dead targets decide as invalid. Current consumers construct fresh scopes; event-chain consumers must supply their runtime target arrays when introduced. Live object kinds without a reproduced evaluator remain named refusals; countries, states, characters and character-backed leaders are supported in triggers. Scripted-text promotions use their separate native selector for countries, states and characters; a missing or invalid name leaves the text context unchanged. Other text object kinds remain named refusals.

`has_dynamic_modifier` reads the current country's or state's recorded modifier container. Its optional `scope` operand uses the shared scoped-variable evaluator: a country target compares country aliases, a state target compares the stored state ID, and an empty target tests only the first matching entry for an empty scope. Explicit targets scan every entry of the definition. Presence ignores the entry's enabled state and remaining duration. Definition names come from the recorded native definitions and the trigger body from the recorded content stack; missing tables and unresolved target links remain named refusals.

The [Scoreboard product](src/products/scoreboard.rs) is a table of requested
shared per-country measures, with a single registry for labels, units, exact
formatting. Its defaults are returned by the product; country selection has no
product count cap. Factory counts use the Factories scan and count helper;
Stability and War support use their single-stat histories; Nukes and Equipment
produced reuse their existing cached scans. Political and Command power share
the country identity/balance scan in
[country_history.rs](src/products/country_history.rs) with Country measure.
That cache retains identities for every country and materializes each balance
field only when requested. Native values stay numeric internally, and adding
another field shares existing histories through Arc while scanning only the
missing field. Stability histories also share immutable per-country cache entries
without evicting a large table when another widget asks for a new country.
Owned territory is the recorded native owned-state roster count, which excludes
zero-province state templates exactly as the native roster writer does. An
unavailable or ambiguous row remains unknown rather than becoming zero.

The Military leads' Army size, Planes, Ships and Casualties columns each have
one unavailable adapter supplying both availability and values, pending their
shared measures. Unavailable measures cannot supply top-N membership. The table
ranks over every identity-present country, places missing values last in either
sort direction, and cuts requested values to top-membership spans using the shared
ranking helper. Ranking updates ordered membership only for countries whose
value changes; supplying read dates are merged in a forward walk only for
returned countries. Manual country picks follow their recorded native identity
through a tag rename. Values and supplying read dates are separate; gaps and
identity changes end runs. Replies refuse the shared product budget rather than
truncating. The UI sends one request for the chosen countries or top N;
only the shared reply-size budget refuses an oversized Scoreboard response.
No live capture or new recording field is required.
