# dashboard-text

The dashboard's text interpreter and the content facts it reads. It evaluates recorded triggers and scopes, renders scripted localisation and native effect descriptions, formats modifiers, and applies the game's name rules. It also holds the content catalogue and the recording metadata helpers that products and map tooltips share.

Nothing here names a product or a source. The [dashboard](../dashboard/README.md) loads one catalogue per source and folders, and re-exports every module at its old `dashboard::` path. The map's tooltip crates (viewer-foundation, viewer-world, viewer-combat, viewer-counters, viewer-counter-products and viewer-raids) depend on this crate and [dashboard-core](../dashboard-core/README.md) instead of the whole dashboard, so editing a product doesn't rebuild them.

| Module | What it does |
|---|---|
| [triggers.rs](src/triggers.rs), [scope.rs](src/scope.rs), [character_scope.rs](src/character_scope.rs) | Recorded-hour trigger evaluation, scope links, aliases and character operands. |
| [scripted_text.rs](src/scripted_text.rs), [effect_description.rs](src/effect_description.rs) | Scoped localisation and native effect descriptions, sharing the trigger/variable evaluator. |
| [modifier_text.rs](src/modifier_text.rs), [faction_names.rs](src/faction_names.rs), [unit_names.rs](src/unit_names.rs) | Shared modifier formatting and native stored/generated names. |
| [party_names.rs](src/party_names.rs) | The ruling party naming ladder shared by party histories and scripted text. |
| [idea_names.rs](src/idea_names.rs) | Country-specific idea names: an idea's selected localisation key or its generated advisor name. |
| [identity.rs](src/identity.rs) | Character and country identity operands names are chosen by: script tokens, templates, tags, looks and civil-war state. |
| [variant_names.rs](src/variant_names.rs) | Equipment variant names as the game writes them, and the operands that order variants. |
| [air_wing_names.rs](src/air_wing_names.rs) | Retained air wing names and the authenticated active-raid projection older prefixes use. |
| [flag_choices.rs](src/flag_choices.rs) | Flag asset candidates per recorded country at one hour, shared by the flag product and map tooltips. |
| [cryptology.rs](src/cryptology.rs), [naval_battle_name.rs](src/naval_battle_name.rs), [state_resources.rs](src/state_resources.rs) | Native helpers shared by products and viewer tooltips. |
| [catalogue.rs](src/catalogue.rs) | Shared lazy localisation, content databases, native GUI/sprite lookup, texture decoding and PNG replies; the per-source loader is in the dashboard. |
| [metadata.rs](src/metadata.rs) | Branch bounds, gaps, initial country catalogue, frame checks, country-link resolution and supplying-read detection. |

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
availability, and for an ongoing relation the hours its rows end. World queries
declare theirs beside the queries (`triggers/world/dependence.rs`), scope
queries in `scope.rs`; a query not declared there, and every names or text
read, is undeclared. Given what changed since the hour the inputs last read
their values (`inputs::Since`: a `FrameChanges`, a `BlockChange::Changed` hour
or a quiet span of `Replay::read_block_changes` through `BlockTables` and
`QuietTables`), `Inputs::confirm_since` takes an input none of whose declared
reads changed as read again without reading it, and rereads only the others;
`Inputs::unchanged` says whether no input needs reading, so a family can
confirm an evaluation without building the hour's trigger world, and
`Inputs::advance` carries one across a quiet span.

Check mode (always in tests, `BATTLE_PLAN_REUSE_CHECK` in preparation) evaluates every
reused result anyway and counts any difference, and rereads every input a
declaration let through: a changed value is a declaration that misses a read,
counted by `inputs::missed_dependences` and failing the test that finds it.
Tests read each declared query on frames that differ only outside its
declaration (`triggers/inputs_tests.rs`).

A country's completed focuses, parties and party leaders, characters and
active ideas are read from that country's rows of the hour's table (rows
ascend by country in a decoded table), with the scan's verdicts and refusals,
rather than from an index of every row.

The `test-support` feature compiles the catalogue and scope fixtures (`catalogue::tests`, `catalogue::texture_tests`, `scope::tests`) and the test-only instrumentation, such as read counters and synthetic scripted environments, for the tests of crates above this one. Only dev-dependencies enable it.
