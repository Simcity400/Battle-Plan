# dashboard

Everything a widget needs natively: opened recordings, data products, the host/viewer protocol and the data service that answers the frontend. No map pixels are drawn here and no live game memory is read.

## Layout

| Module | What it does |
|---|---|
| `source.rs` | One opened recording: replay bytes, metadata, resolved game content and each product's per-source cache. |
| `catalogue.rs` | Content facts every product shares, read once per source, folders and game language: the localisation (the same parse country naming uses, from `content::localisation`; every text goes through the shared game localiser in `installed_content::localisation`; a refusal, such as a scripted expression or a control plain text cannot show, is kept apart from a missing key and noted by reason and key for the product warnings) and interface sprites, plus the texture decoding, within a budget, and PNG encoding every image reply uses. |
| `triggers.rs` | The one trigger evaluator: raid, combat-tactic, scripted-localisation, equipment-graphics pool and adjacency-rule triggers decided from one recorded hour. Each key is dispatched as `CTrigger::LoadKey` does (`installed_content::trigger_keys`), each coded trigger checked against its class's accepted scopes first. Only the children LoadKey adds count: a key that names nothing adds none, `else`/`else_if` attach to the last `if` added, and `World::has_children` is the emptiness test every consumer reads. A key names a character by the content's character templates; scope keys resolve through `scope.rs`, and a FROM read through ROOT, FROM or PREV is refused outside the combat scope, since those copy their own scope's links. Tag aliases resolve as `AliasResolve` does (`triggers/aliases.rs`). Script variable operands (`check_variable`, `has_variable`, `var:` keys, variable counts, country and state operands, arrays and tag alias variables) are evaluated as `CScopedVariable::Evaluate` does over the recorded `ScriptVariables` stores (`triggers/variables.rs`); built-in game variables, saved event targets, random draws and temps a trigger-side setter may have written are refused by name. A combat scope's own triggers come from its caller (`Combat`). A caller without a country uses `Scopes::empty`, the default event scope: no carried objects, and THIS/ROOT/FROM/PREV all empty. Scope-masked triggers are false there; country-tag scopes resolve normally over the recorded World. A context trigger targeting an empty scope is false by its own mask, including an empty ROOT block. What the recording cannot answer is `Eval::Unknown` with its reason. |
| `scope.rs` | Script scope targets resolved over one recorded hour (`RecordedScopes`), with `_CountriesForOriginalTags` derived from the branch (`scope/original_tags.rs`). |
| `metadata.rs` | Branch ranges, gaps and the first readable country catalogue, and the helpers every product shares: frame checks, a tag's country this hour (`tagged_country`: found, absent, or unresolved when the links cannot tell which country holds it), a raw link's canonical slot (`link_slot`) and which hours read a daily table (`slot_read`: a walk of any table of its slot was timed, as the fresh-save oracle dates daily tables; a product dating values by their read uses it). |
| `products/` | One module per data product, registered once in `products/mod.rs`. |
| `service.rs`, `service/` | The service facade and its private modules for admission and leases, source and loader jobs, and the bounded response pump. |
| `transport.rs` | Data commands, the generic product request and fragmented replies. |
| `protocol.rs` | Control messages between the Tauri host and the viewer process. |
| `map_settings.rs`, `config.rs`, `content.rs` | Map preferences, game folders and content verification. |
| `bin/dashboard-bridge.rs` | Development-only stdio adapter for browser UI work. |
| `bin/export-types.rs` | Generates the TypeScript contract. |

## Products

A product turns a source plus typed parameters into one compact reply covering a whole branch, so frontend playback is a local lookup and never a per-hour request. To add one:

1. Add `products/<name>.rs` implementing `Product` (a key, `Params`, `Reply`, optional `validate`, and `load`). Keep per-source caches in `source.state::<YourState>()`. Read localisation and sprites through `catalogue::catalogue`, never by parsing them again.
2. Register it in the `PRODUCTS` list in `products/mod.rs`.
3. Regenerate the contract and add the widget folder under `ui/src/widgets/`.

The service, transport, host and frontend route by key and never name a product. Replies are serialized with the frontend's lease identity so they cannot be attributed to another recording. Products read verified recorded content only; missing localisation or artwork stays null.

Current products: national focus order, research slot history, country flags, country styles (each country's engine-built name and recorded map colour at an hour), country groups (human players, recorded factions with their members, and war pairs, as runs of consecutive recorded hours), country balances (political power or command power: exact balance runs and net drops per country), party history (popularity, ruling runs and elections per country, with content ideology colours), idea history (each idea a country held, joined to its content category, slot, effects and artwork, with how the game formats each effect's modifier (`idea_format.rs`: the recorded modifier definitions, else content's scripted ones); `politics_content.rs` reads the Politics content once per catalogue), country stability (displayed stability, war support and maximum command power computed from recorded inputs with the engine's arithmetic, plus an hour's formula terms, tree lines with their source identity or recorded text, and an explicit remainder), stability parts (each of those parts of one country over the branch, as runs, known only where recorded), resource balance (one country's recorded resource pools and trade deals as hourly runs, with content's resource names and icons), land ownership and country fuel (one country's tank level and capacity at each daily read, and the fuel gained and burned in the 24 hourly updates before it, each update counted once and only while the game updated it). Their rules live beside their code and in the matching widget README.

## Service

`Service` owns sources by canonical path and revision, hands the frontend independently releasable leases, and runs product and open jobs on a small loader pool. Requests for the same product on the same source serialise on that product's cache; different products and sources run in parallel. Replies are bounded, fragmented and cancellable at every step. The map process shares the same sources through `open_map`.

## Types

Rust owns the generated frontend types. Use the shared [build commands](../../AGENTS.md#build-commands):

```powershell
bash tools/build-slot.sh "dev/dashboard" "cargo run -p dashboard --bin export-types"
bash tools/build-slot.sh "dev/dashboard" "cargo run -p dashboard --bin export-types -- --check"
```

Generation updates `ui/src/generated/dashboard.ts`; do not hand-edit it. Follow with the checks in [AGENTS.md](../../AGENTS.md).

## Rules

Preserve recorded values exactly and keep missing, unavailable and zero distinct. Never bridge a gap or carry a value across missing hours. A changed recording revision invalidates every reply.

The initial country selection uses LocalPlayer at the first readable country catalogue (including an observer's selected country), joined through that frame's CountryLinks. The human roster does not determine the recording client.

Scripted text uses [scripted_text.rs](src/scripted_text.rs) and the shared [recorded trigger evaluator](src/triggers.rs). Built-in functions read the sampled party leaders, gender, manpower operands, agency, old country names and balance-of-power operands; content-defined entries select their first passing text alternative. Missing inputs and unsupported trigger or description forms remain named refusals. State template keys come from the recorded content layers. Text the game stored already localised or as a literal, without its key (a created agency's name, a state's custom or owner name, a country's retained old names), is exact only in the recording client's language: it is shown when the viewer's language is that one and refused by name otherwise. A caller with its own event scope passes it with the scopes it links to and its script variables (`EventScopes`): each bracket starts from it, ROOT, FROM and PREV move to the linked scope, and a content-defined entry is evaluated, and its text expanded, in the scope the game binds for it. A character it promotes is a character scope, where `tag`, `skill`, `has_trait` and `has_character_flag` read the recorded character inputs through [character_scope.rs](src/character_scope.rs), which the tactic evaluator shares; any other trigger there is checked against its scope mask first, and one that passes but is not reproduced is refused by name. A link or scope object the caller cannot describe is refused only where a promotion or a reached condition reads it. Native dispatch addresses and exact refusal reasons live beside each property.
