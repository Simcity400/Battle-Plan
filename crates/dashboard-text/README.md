# dashboard-text

The dashboard's text interpreter and the content facts it reads. It evaluates recorded triggers and scopes, renders scripted localisation and native effect descriptions, formats modifiers, and applies the game's name rules. It also holds the content catalogue and the recording metadata helpers that products and map tooltips share.

Faction screens reuse the bound-localisation interpreter and `ScriptedText::expand_faction`, whose faction environment holds the explicit faction and the selected country's event scope. Native goal names and descriptions therefore share the existing faction properties, promotions and recorded-input refusals.

The catalogue supplies war participation's four lend-lease factors from the complete evaluated content stack, including mod overrides.

Occupation badge descriptions share effect sessions and the raw modifier formatter. Their active, non-detailed description shows the definition name, disable effects, eligible state modifiers and authored tooltip, retaining independent fragments and counted refusals. The caller supplies the recorded state or controller root and its controller/occupied-country scope links.

Nothing here names a product or a source. The [dashboard](../dashboard/README.md) loads one catalogue per source, folders and presentation, and re-exports every module at its old `dashboard::` path. The map's tooltip crates (viewer-foundation, viewer-world, viewer-combat, viewer-counters, viewer-counter-products and viewer-raids) depend on this crate and [dashboard-core](../dashboard-core/README.md) instead of the whole dashboard, so editing a product doesn't rebuild them.

| Module | What it does |
|---|---|
| [triggers.rs](src/triggers.rs), [scope.rs](src/scope.rs), [character_scope.rs](src/character_scope.rs) | Recorded-hour trigger evaluation, scope links, aliases and character operands. |
| [scripted_text.rs](src/scripted_text.rs), [effect_description.rs](src/effect_description.rs) | Scoped localisation and native effect descriptions, sharing the trigger/variable evaluator. |
| [modifier_text.rs](src/modifier_text.rs), [faction_names.rs](src/faction_names.rs), [unit_names.rs](src/unit_names.rs) | Shared modifier formatting and native stored/generated names. |
| [party_names.rs](src/party_names.rs) | The ruling party naming ladder shared by party histories and scripted text. |
| [division_history.rs](src/division_history.rs) | The game's description keys, date text and arguments for a division's recorded career entries, shared by the map's division tooltip and the service record. |
| [idea_names.rs](src/idea_names.rs) | Country-specific idea names: an idea's selected localisation key or its generated advisor name. |
| [identity.rs](src/identity.rs) | Character and country identity operands names are chosen by: script tokens, templates, tags, looks and civil-war state (the predicate's inputs indexed once an hour for every country, with the fields it reads). |
| [variant_names.rs](src/variant_names.rs) | Equipment variant names as the game writes them, and the operands that order variants. |
| [air_wing_names.rs](src/air_wing_names.rs) | Retained air wing names and the authenticated active-raid projection older prefixes use. |
| [flag_choices.rs](src/flag_choices.rs) | Flag asset candidates per recorded country at one hour, shared by the flag product and map tooltips. |
| [cryptology.rs](src/cryptology.rs), [naval_battle_name.rs](src/naval_battle_name.rs), [state_resources.rs](src/state_resources.rs) | Native helpers shared by products and viewer tooltips. |
| [catalogue.rs](src/catalogue.rs) | Shared lazy localisation, content databases, native GUI/sprite lookup, texture decoding and PNG replies; the per-source loader is in the dashboard. Recorded facts read the recorded stack; sprites, GUI, textures and native-window text read its presentation, drawing what the visual mods cannot supply from the recording, named on the consumer's own refusals ([dashboard](../dashboard/README.md#widget-presentation)). Sprites draw through [catalogue/sprite.rs](src/catalogue/sprite.rs). Masked shields draw through [catalogue/shield.rs](src/catalogue/shield.rs) as the game draws them: the flag through the shield's own effect and mask, then the overlay through the program the game's code names. |
| [metadata.rs](src/metadata.rs) | Branch bounds, gaps, initial country catalogue, frame checks, country-link resolution and supplying-read detection. |

The metadata helpers can date every table's supplying read together
(`slot_reads`): cadence groups use the recording's format, and consumers of
one hour with the same timed tables reuse its flags. CountryIntel's measured
walk dates only itself. Individual `slot_read` calls follow the same rule.

In an industrial organisation's scope, `has_mio_size` reads that organisation's recorded size (`IndustrialOrganisationDetails`, by its full reference) and compares it as the game's integer comparison triggers do, the class `num_of_controlled_states` belongs to: `<` and `>` are strict, `=` is equality, and a variable operand is truncated to whole units. Its input declares that one row's size field. Every other trigger in an organisation's scope is still refused by name.

## Own refusals

The catalogue's text refusal tally (`Catalogue::refusal_lines`) is shared by
every consumer of the source, at whatever pace each runs. A consumer whose output
must not depend on the others runs its calls inside `catalogue::own_refusals`
with its own `OwnRefusals`: every text refusal noted on that thread meanwhile, and
every lazy reader it uses (scripted localisation, state templates, faction
templates), joins its tally, and inside the scope `refusal_lines` reports that
tally in the same form and order, with only the warnings of the readers it used
(read again if this process has not read them yet). A cached result retains what
its read refused and credits it to every scope that uses it, and scripted text
expanded on a larger stack credits its thread's notes back to the caller, so who
read first decides nothing. Artwork-effect and uncovered-read diagnostics,
which any consumer's read adds to, stay the resolved stack's live diagnostics:
a scope's lines leave them out, and a prepared query reply's content warnings
gain the current ones where it is served
(`Source::append_stack_warnings`), as product replies gain the read set.

## Evaluation reuse

An evaluation run under `triggers::inputs::capture` returns the inputs it read:
each recorded query on the hour's shared state (`World`, `RecordedScopes`,
recorded country names, the scripted-text helpers, the Clock seed when randomness
is drawn) with a digest of its answer, and each raw table it read whole, by view
identity, or by the fields it reads of every row (`inputs::fields`; the
character scope's tables, `character_scope::FIELDS`). Queries run their bodies
unrecorded, and shared state built lazily inside an evaluation is built
unrecorded, so an input is always the query's answer, never the reads behind
it. `Inputs::confirm` rereads every input on a later hour; when all read the
same, the evaluation's result is reused. Leader visibility reuses this way.

Each input also declares what its value is a function of (`Key::dependence`):
some fields of some rows (a key range) of some tables, their row keys and
availability, and for an ongoing relation the hours its rows end. A read of one
country's rows of a table keyed by raw country (its completed focuses, parties
and their leaders, characters, ideas, technologies, modifiers, appearance,
nukes, its Politics row) declares those rows and the rows whose country does
not resolve, which a read refuses on, with every link's slot
(`TableRead::owned_by`): another country's rows changing leave it unchanged.
World queries declare theirs beside the queries (`triggers/world/dependence.rs`),
scope queries in `scope.rs`, scripted texts' own reads and country names in
`scripted_text/dependence.rs`. Every world query, names read and text read is
declared but a date property, a function of the hour itself; a scope query not
declared in `scope.rs` is undeclared. Given what changed since the hour the inputs last read
their values (`inputs::Since`: a `FrameChanges`, a `BlockChange::Changed` hour
or a quiet span of `Replay::read_block_changes` through `BlockTables` and
`QuietTables`), `Inputs::confirm_since` takes an input none of whose declared
reads changed as read again without reading it, and rereads only the others;
`Inputs::unchanged` says whether no input needs reading, so a family can
confirm an evaluation without building the hour's trigger world, and
`Inputs::advance` carries one across a quiet span.
`Inputs::keys` exposes the recorded keys in read order for measured rejection
diagnostics; inspecting them does not confirm or advance an evaluation.

Country scope declarations include tag and alias lookup, roster and diagnostic
names. Tag substitution declares countries' slots and tags, country links,
owned-state counts and the recorded original-tag buckets, including the reserved
country's count. Alias bodies come from the resolved content stack; their
evaluation records its own reached inputs.

Check mode (always in tests, `BATTLE_PLAN_REUSE_CHECK` in preparation) evaluates every
reused result anyway and counts any difference, and rereads every input a
declaration let through: a changed value is a declaration that misses a read,
counted by `inputs::missed_dependences` and failing the test that finds it.
Tests read each declared query on frames that differ only outside its
declaration, and check that check mode reports each one declared to read
nothing (`triggers/inputs_tests.rs`, `scripted_text/dependence_tests.rs`).

A country's completed focuses, parties and party leaders, characters and
active ideas are read from that country's rows of the hour's table (rows
ascend by country in a decoded table), with the scan's verdicts and refusals,
rather than from an index of every row.

The `test-support` feature compiles the catalogue and scope fixtures (`catalogue::tests`, `catalogue::texture_tests`, `scope::tests`) and the test-only instrumentation, such as read counters and synthetic scripted environments, for the tests of crates above this one. Only dev-dependencies enable it.

Scalar supplying-read queries reuse the same per-hour timed-table projection as
`slot_reads`, retaining direct answers for scheduled reads and CountryIntel.
The cache checks the format, hour and every table's timed/un-timed evidence;
`slot_reads:refresh` labels an evidence change that rebuilds its flags.

`CountryKeys::same_changes` carries a previous hour's projection through recorded
country deltas that keep its keys, slots and tags. Without that evidence it
compares the full key sequence. `kept_changes` checks only the row count and
recorded masks; a caller deferring its full comparison retains any failed proof
until that comparison. `country_keys:changed_fields` labels the mask check.

The create_wargoal effect description reads the complete recorded unused inventory and target occurrences, including direct-subject matching and claimed-state containment. A reached absent inventory, unresolved reference or ambiguous definition occurrence is refused by name.

War-goal descriptions use the definition's declared name. Duplicate-type identity is refused only when a matching unused goal covers the requested targets; absent or non-covering goals remain decidable.
The shared recorded trigger kit includes Divisions for promoted unit scopes and CountryCommandPower for country owner operands, preserving them in product-selected reads.
`effect_description::doctrine_benefits` shares bound prose, unit-stat, modifier
and effect descriptions with existing technology and effect consumers. Missing
operands refuse only their fragment and produce counted reasons.
Project scopes resolve persistent variables and seeds from Projects.runtime. Project and pending-choice outputs share one renderer for country, scientist, facility-state, unlock and equipment/unit bonus bodies. Explicit project assignments retain null or unavailable context instead of invoking completion-effect fallback selection.


`effect_description::doctrine_benefits` shares bound prose, unit-stat, modifier
and effect descriptions with existing technology and effect consumers. Missing
operands refuse only their fragment and produce counted reasons.
Project scopes resolve persistent variables and seeds from Projects.runtime. Project and pending-choice outputs share one renderer for country, scientist, facility-state, unlock and equipment/unit bonus bodies. Explicit project assignments retain null or unavailable context instead of invoking completion-effect fallback selection.
`template_facts` shares recorded template display names and plain layout/settings/restriction, country staff/garrison and HQ sections between division hovers and the Army size breakdown. Names and definitions resolve from recorded operands and the supplied content stack. Valid sibling facts remain visible when another field is unavailable; each refused field carries its own diagnostic into the product or hover. Hover callers supply field-level text validation before names and definition spellings join their sections; unsupported text withholds only that fact. Model-read refusals keep category/country operands, and an unreadable HQ front keeps the deployment context and pools.
Scripted map callbacks share this evaluator's calculated stability and war support,
war, AI-control, claim, neighbor, home-area, tech-sharing and appointed-advisor/idea
trait predicates. Country/core/state rosters retain native order and duplicates;
scope-array loops decode elements lazily. State target filters run on the initiator
with FROM set to the target. Root checks, array operands and target filters share
one temporary-variable context for each selection; state selectors ignore the
country-only state_target marker. Already accepted states skip subsequent filters,
while rejected candidates can be retried. Built-in global country/state arrays, state roster
arrays, owner/controller, population, resistance/compliance and state building-level
callbacks use recorded operands, as do recorded stability modifier totals. Expression
clamp evaluates its lower then upper bound and takes the lower comparison arm first.
Resistance/compliance callbacks and Occupied require their table's own current-hour
acquisition. A carried daily value has no writer certificate and is refused by name;
the evaluation's freshness dependency is checked again even when rows stay equal.
Other built-ins remain named refusals when their native operands are unavailable.
An effect-driven map refresh runs inside its caller's active temporary-variable
context, so nested callbacks do not begin with zeroed output channels. `Temps::Inherited`
keeps unknown initial values unknown even inside checks; explicit local writes and
seeds remain readable. Unwritten reads, including RNG, refuse the unrecorded caller
temporary store. The viewer proves each forced callback independently, conservatively
refusing dependencies on earlier targets instead of reconstructing transient script
execution from an event ordinal. Standalone callbacks retain fresh-context semantics.
Colour callbacks use a conservative rule: any reached unavailable trigger, value,
iterator or scope input refuses that target, even when a later unconditional
assignment would overwrite every colour output. This accepted limitation avoids
painting a colour that uncertain control flow could change. The same rule applies
during layer target selection: the viewer uses the evaluator's colour-input
capture around both selectors and their root/per-target filters, refusing the
whole layer when its selected set is uncertain. Refusals are created
by the accessor whose own read finds an unavailable table,
field or acquisition, with the table/field and a scope only for scoped input.
Errors carry those identities through aliases, scope dispatch, Boolean groups,
conditional loading, operands and temporary variables. Context changes only their
message. No wrapper observes messages or creates a substitute input identity.
The callback collects source identities independently of its Boolean result,
deduplicates them before formatting, and counts each affected target once per input.
Global inputs such as LexicalTokens remain one input across country scopes.
Genuine unsupported evaluator/content forms have their own category. There is no
implicit conversion from a source failure to String: only text rendering
boundaries explicitly format it. Cached and lazy errors retain their sources; reading a cached failure replays those identities,
while an unused lazy RNG failure does not refuse unrelated output. Deferred alias
candidates replay failures only when visited. Invasion collections retain both
known orders and unresolved contributors; a known matching order proves an
existential query, while a nonmatch refuses if an unresolved contributor could
change it, even when the known collection is nonempty. Positive-reference lookups record the failing table's typed refusal before a
caller can filter or default a miss. State/province row views guard referenced
keys and incomplete enumeration; character-token indexes retain lexical failures
through warm reads. Authenticated nulls and complete-container membership absence
remain distinct from a missing referenced object. Membership absence requires a
complete source on every read: warmed country-tag and live-character indexes
cannot authenticate an omitted token from a truncated or unavailable table.
Scope loading replays every unavailable name classification before inspecting
the parsed scope, including nested chain targets and classification that falls
through to a keyword. Reusing loaded items and warmed name indexes preserves
those refusals. Raw-country resolvers record a typed CountryLinks refusal when a nonnull
reference cannot resolve, even if a caller filters the returned None. Lazy world,
scope and resource indexes retain these failed joins separately from their values
and replay them on every use, including warm and nested caches. Character-name
indexes retain raw owners and resolve them at lookup. Query reuse digests include
failed-join identities, so a known nonmatch cannot be reused after its join becomes
unavailable. Filtering a failed entry may conservatively refuse an otherwise
proved match; the owner accepts this over-refusal. Zero remains a valid null or
reserved reference and does not itself create a missing-positive-link refusal.
Evaluator consumption replays existing identities before Boolean masking; a test
assertion runs first and fails if an accessor forgot to replay its cached error. Ordinary trigger
evaluation retains its three-valued Boolean rules. Technology-sharing
queries require a complete country-local roster witness even for a group name
absent from the recorded token dictionary.

State hover text resolves GetContinent through the controller's original-tag
localisation ladder, GetCapitalVictoryPointName through the captured cached index
and authenticated authored province order, and Occupied through the recorded
resistance country. A missing cache or changed state membership refuses that text.
The shared `stability` module re-exports `replay-data`'s arithmetic for triggers
and stability products, and adds the captured CountryCommandPower lookup with
its recorded-input refusals.

Unit effect descriptions retain the selected full unit handle when a carrier lookup refuses at the presentation boundary. The unit existence witness still precedes owner/template lookup and operand evaluation; an absent template still precedes the unit-name read.

An explicitly refused template settings payload reports only the settings section as unavailable. Layout, model, category, country and display-name facts retain their independent availability.

`subunit_requirements` owns canonical authored and recorded local-clone equipment
requirements shared by army products and model consumers. `subunit_files` and
`table_access` retain the same file order, token identities and partial-table
semantics at this lower layer; viewer-foundation re-exports their old paths.
Model scoring and model-country selection remain in viewer-world.
