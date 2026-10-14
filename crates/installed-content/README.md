# installed-content

Rust readers for the installed content used by recording, playback and save comparison. This crate owns content layers, identity verification, authored catalogues, assets and the shared text parser. [installed-content-save](../installed-content-save/README.md) owns text-save projections. It does not read live game memory.

Read the [project rules](../../AGENTS.md) before changing a reader. Consumers should use these shared readers rather than parse the same files again or add a Python runtime dependency.

## Layout

| Area | Entry points and responsibility |
|---|---|
| Content stack | [content.rs](src/content.rs): identities, verification and overlays. [physfs_zip.rs](src/content/physfs_zip.rs): native archives. [visual.rs](src/content/visual.rs): explicit cosmetic overlays. |
| Text readers | [parser.rs](src/parser.rs): byte-preserving data and balanced trees. [parser/native.rs](src/parser/native.rs): script lexer and persistent reader. [script_files.rs](src/script_files.rs): database file order. [crt.rs](src/crt.rs), [fixed5.rs](src/fixed5.rs) and [parser/decimal.rs](src/parser/decimal.rs): number conversions. |
| Script dispatch | [key_names.rs](src/key_names.rs): load-point names. [trigger_keys.rs](src/trigger_keys.rs), [combat_tactics](src/combat_tactics.rs), [effect_keys.rs](src/effect_keys.rs) and [modifiers.rs](src/modifiers.rs): shared body readers. [tokens.rs](src/tokens.rs): name identity. [coverage.rs](src/coverage.rs): unread suffixes. |
| Names and text | [localisation.rs](src/localisation.rs), [country_names.rs](src/country_names.rs), [scripted_localisation.rs](src/scripted_localisation.rs), [scripted_keys.rs](src/scripted_keys.rs) and [defines.rs](src/defines.rs). |
| Authored catalogues | Domain modules such as [equipment.rs](src/equipment.rs), [ideas.rs](src/ideas.rs), [characters.rs](src/characters.rs) and [events.rs](src/events.rs) own database and lookup rules. [lib.rs](src/lib.rs) indexes all modules and defines typed diagnostics. |
| Artwork and layout | [gui/native.rs](src/gui/native.rs), [gfx.rs](src/gfx.rs), [image](src/image/mod.rs), [mesh.rs](src/mesh.rs), [particles.rs](src/particles.rs) and [pdxshader](src/pdxshader/mod.rs). |
| Map and movement | [map.rs](src/map.rs), [states.rs](src/states.rs), [province_graph.rs](src/province_graph.rs), [movement.rs](src/movement.rs) and the land/naval path and raid modules. |
| Preferences | [settings.rs](src/settings.rs): read-only local preferences. |

A catalogue retains definitions, source identity and diagnostics. Duplicate handling, case sensitivity, defaults and enumeration order follow its native database, not a universal merge rule. Reuse catalogues and stack-scoped caches. Presentation and its shared [catalogue](../dashboard/src/catalogue.rs) belong to [dashboard](../dashboard/README.md).

Idea lookup shares the lexer's ASCII case identity and selects the first matching database entry; rereading a held group replaces that entry in place. Advisor lookup, ambiguity and coverage share its native folded string identity. Retain authored spellings for names and assets; localisation and sprite names keep their own case rules. Equipment graphics contexts retain a null archetype separately from a real empty equipment name: the null object's sentinel hash cannot match an authored empty pool key, while a real empty name can.

## Recorded content stack

Resolve every content fact through `ResolvedLayers`: **vanilla, DLC, then mods in recorded order**. Later layers win file lookup; `replace_path` hides lower layers under its path. File overlays do not determine database duplicate rules. Support folder/archive layers and the engine's built-in virtual roots.

`gather` prepares a new inventory. `native_mod_order` follows dependencies and descriptor path bytes, not launcher list order. Verification requires the collector to match the live successful search path and active DLC/mod rosters. Duplicate display names that prevent proving mount order leave the session unverified.

`resolve` uses recorded IDs, order, hashes and coverage policy, independently of current launcher selections. Machine paths and descriptor locations stay outside portable identity. Caches must preserve verification on hits and misses; changed, added or removed covered files are errors.

The persistent content-hash cache is written only when entries change or an absent/unreadable cache needs rebuilding. An unchanged valid cache remains byte-for-byte intact. Verification and read statistics still cover every selected file; successful hashes persist even when a later layer refuses resolution. Cache write errors remain failures whenever a write is needed.

Use stack listing/read APIs. `script_files::paths` and `ResolvedLayers::directory_files` list direct files in merged filename byte order. Follow each loader's suffix and recursion rules; recursive loaders generally read files before subdirectories.

Consumers retain derived caches through `ResolvedLayers::derived`: clones share
one resolved stack, another stack or mount roster has independent entries, and
cosmetic overlay selection starts fresh. Cached reads retain the stack's shared
diagnostics. Readers keep content inputs immutable during one resolved load.

Paths are bytes (`LayerPath`): archives retain PhysFS name identity; folders use Windows lookup. Use byte-path APIs where required. The shared sanitizer and layer readers own normalization and case rules; decoding archive names or independently normalizing requests changes native identity.

Missing or mismatching recorded content is refused. The viewer's explicit `--allow-unverified-content` override permits an unverified recording identity, with diagnostics and an image banner; it does not waive hashes or accept an unsupported coverage policy. Supported older policies verify their own roots without rewriting identity.

## Parser contracts

Script content uses `parser::native`, the loader's lexer mode and each field's `Shape` or shared typed reader. Native token consumption, operators, variables, recovery and block endings differ from balanced trees. Use `Item::keyword`/`keyword_is`: quoted keys are not keywords; bare keywords use the lexer's fold.

Keep the separate tree grammar for saves and descriptor consumers that use it. Save brace mismatches are corruption. Do not add guessed variable evaluation or recovery to tree-based content readers. Numbers stay lexical bytes until native conversion; script fixed5 and save decimal parsing are separate paths.

Triggers share `combat_tactics::Shaper` and the trigger registry; effects share `effect_keys`; modifiers share `modifiers`. Pass the native load point's `KeyNames`: later registrations must not change earlier reads. Reuse memoized names and common keyword/dynamic-token tables.

Focus catalogues share the complete load-point `KeyNames` across file parsers.
Their layer/path order determines duplicate precedence and diagnostic order.
Reads verify sequentially in waves sized by available hardware concurrency;
independent files parse concurrently within each wave and merge in original
order. The first I/O error stops before any later path is read. A single-file
wave parses directly, and every inventoried focus file remains eligible.

Static keyword, modifier, trigger, effect, equipment, rule and AI metadata comes from `engine::executable_tables`, initialized explicitly from the selected installation's hash-verified executable. The process shares one immutable table set. Content binding validates the selected installation before using it; parsers do not guess an installation or fall back to bundled names. Authored definitions still use `ResolvedLayers` and the load point's `KeyNames`.

Built-in variable names come from the same installation's `documentation/dynamic_variables_documentation.md`. The reader extracts declaration headings by scope, excludes examples, preserves exact spelling and checks sorted names against the supported semantic digests. Missing, changed or duplicate declarations refuse initialization. Mod documentation does not replace executable metadata. Global, country, state and character variable stores use these names; the character scope is documented as `unit_leader`.

Retain conditions and effects as data. Evaluation and descriptions over recorded state belong to [dashboard triggers](../dashboard/src/triggers.rs) and their consumers.

## Localisation, defines and scripted text

[Localisation](src/localisation.rs) uses language headers and native line syntax, not YAML or filename-derived languages. Ordinary files in layer/path order keep the first key; `replace/` files run afterwards and overwrite, even over ordinary files in later layers.

Use `Localisation::localise`, `localise_with` or `localise_bytes` for substitution/formatting, and `pieces` for stored controls. Do not interpret raw values with a consumer's colour or substitution grammar. Keep native missing-key behaviour and absent/empty distinctions. `format_scripted_variable` shares numeric formatting with scripted text.

Scripted localisation retains ordered alternatives, triggers and random-list operands; ScriptedKey retains conditional choices. Consumers supply recorded scopes and shared evaluation with each caller's selection rules. Plain text and evaluated `[...]` text are different paths: use dashboard's catalogue and [scripted text](../dashboard/src/scripted_text.rs) helpers. Missing runtime operands are named refusals.

[Defines](src/defines.rs) runs stack scripts in native recursive order on vendored PUC Lua 5.1, including arithmetic, functions and loops. Its sandbox excludes machine-access libraries/file loaders and bounds execution and memory. Script failure refuses the reader; it never substitutes vanilla values. Use its typed getters for numeric, string and boolean defines.

## Asset contracts

`ResolvedLayers::covered`/`covered_path` test the recording's hash policy; `content.rs` owns its roots. Ordinary reads outside coverage require `report_uncovered_reads()`: retain the shared `UncoveredReads` handle and refresh diagnostics with `append_to` after lazy reads. Explicit `read_uncovered` consumers keep asset-specific warnings and contribute their attempted uncovered paths, including missing candidates, to that shared handle when reporting is enabled. Both paths verify covered bytes, including covered folder aliases. Do not expand coverage incidentally.

Name uncovered bytes in diagnostics and machine-readable output, including assets only newer policies cover. Loading a file does not prove hash verification.

Use [gui::native::resources](src/gui/native/resources.rs) for sprite/font registries, texture candidates and frame geometry; [image::native](src/image/native/mod.rs) supplies native decoding. Pass the client's `settings::SettingsKeys::texture_decode`: `force_pow2_textures` changes extents, sizing and UVs. Distinguish native failure from undefined/device-dependent output. Sprite names are exact bytes; refused declarations are not absent names.

Preserve palettes and image origins, authored mip chains, cube faces, glyph metrics, mesh inheritance, skeleton order and attachment transforms. Resource roles determine linear or sRGB sampling. The approved BC7-only exception reads logical DDS extents, the complete declared mip chain and header colour space; its unsupported forms remain named refusals in [load_dds.rs](src/image/native/load_dds.rs). Other formats keep native decoding.

[PdxShader](src/pdxshader/program.rs) preserves native effect/include order and hashes resolved inputs. Missing sources or unsupported declarations never select an unrelated effect. Compilation, GPU caches and bindings belong to [shader-pipeline](../shader-pipeline/README.md).

Artwork uses `program::read_reported`: ordinary uncovered-source reads join the shared read warnings, and missing-include notes join the stack's lazy shader diagnostics. Reporting consumers retain both after rendering.

Explicit cosmetic overlays verify recorded layers first, then alter a presentation copy. Checksum-manifest eligibility is separate from renderer compatibility. Report unsupported content and warn that selected artwork is not recorded verification. The selector neither downloads content nor changes enabled mods.

## Native GUI reader

`gui::native::read_layers` shares context across interface files and runs the native post-load pass. Use its registry and local/recursive lookup helpers, preserving template ownership. Layout, construction and lifetime belong to the [viewer](../viewer/README.md).

Dependent lookups/draws retain element and subtree refusals for undefined reads, missing inputs or inheritance that cannot finish. Viewport-dependent fields require `read_layers_at`/`read_at` with the native GUI viewport. Sprites/fonts share the lexer, variables and number conversions.

## Map movement and raids

Map readers share definition rows, raster loading, geometry and adjacencies. Resolve file names/dimensions from the stack; record positions, authored IDs and terrain ordinals are distinct. Map area uses raster cells without physical-area calibration.

[weather/temperature.rs](src/weather/temperature.rs) resolves what playback's temperature pass reads: `common/weather.txt`'s temperature settings, window and terrain modifiers, each province record's activity, neighbours, terrain and last-listing strategic region, and each region template's weather periods ([regions.rs](src/regions.rs)).

World interfaces supply recorded movement/raid facts. Missing admission inputs stay unknown; do not infer port sea access or runtime caches from nearby assets. [raids/target.rs](src/raids/target.rs) shares target/name joins. The [raid tooltip contract](../viewer/src/map/raids/tooltips/README.md) owns consumer and capture rules.

[Selected movement searches](src/unit_path/selected.rs) use recorded current-hour world operands and preserve native neighbour order and caller-specific admission. Missing runtime facts remain refusals.

## Refusals

Keep absence, valid emptiness, native recovery, parse failure and unsupported content distinct. Preserve producer-assigned `DiagnosticKind` and named, counted diagnostics. Refuse unsupported fields/classes/definitions with file/line context; never drop them or substitute vanilla defaults.

`Read::misreads` records recovery; `Read::unsettled` means consumption cannot be followed. `Deferred effect` warnings do not prove complete reads. An unread suffix can alter later objects: use database coverage tracking to refuse dependent lookups, including absence or earlier definitions. This uncertainty is separate from hash coverage.

Technology description catalogues retain every indexed declaration in native order, while case-insensitive name lookup selects the first. The technology database keeps each technology's research folders in authored order, a later definition replacing an earlier one's; `read_folders` reads `common/technology_tags` folder definitions in directory order with their `available` triggers, keeping a repeated folder's first definition and refusing the repeat by name.

## Adding or changing a reader

1. Identify the loader's file order, lexer mode, field and lookup rules. Describe their behavior beside code.
2. Extend the owning module/shared dispatch. Use `ResolvedLayers`, load-point names and native conversions.
3. Preserve order, defaults, redefinitions and required operands; propagate diagnostics and uncertain suffixes through lookups.
4. Add synthetic regressions for changed behaviour, layered replacements and reached mod forms. Reuse helpers (`test-support` exposes cross-crate fixtures) and update affected consumers together.

[Selected transport](src/unit_path/selected/transport.rs) preserves early returns, per-occurrence rounding and independent output buffers; its message law asks for each operand only when a native branch reaches it.

## Checking changes

Follow [AGENTS.md](../../AGENTS.md) for crate-scoped formatting, clippy and affected tests. Layer changes need order, folder/archive, `replace_path`, identity-failure and changed-file cases. [mod_layers_tests.rs](src/mod_layers_tests.rs) provides synthetic stacks; other tests live beside readers.

Tests that exercise runtime keyword or registry metadata call `engine::executable_tables::initialize_for_tests()`. Set `BATTLE_PLAN_GAME_DIR` to the supported installation for those tests; an unset variable or incompatible installation is an explicit failure. Fixtures supply synthetic authored content while the immutable metadata comes from that installation. Equipment fixtures initialize this metadata before loading authored content, and shader parsing tests initialize it before using the shared lexer. Tests limited to table decoding and byte lookup use synthetic data without an installation.

Real-game tests are ignored and read-only. Set `BATTLE_PLAN_GAME_DIR` to the game folder; run `cargo test -p installed-content <filter> -- --ignored --nocapture` from the repository root. Example filters: `every_shipped_gui_file_reads`, `every_shipped_gfx_file_reads`. Check each test's environment: corpus audits may require `BATTLE_PLAN_WORKSHOP_DIR`; special projects use `BATTLE_PLAN_SP_GAME`/`BATTLE_PLAN_SP_WORKSHOP`. Missing variables can skip a test, so confirm it read files.

Compare relevant DLC/mod diagnostics with the [viewer corpus workflow](../viewer/README.md#mod-corpus-check); identity support alone does not prove reader coverage. Asset changes follow [rendering rules](../viewer/README.md#rendering-rules); integration follows the [recorder fresh-save/smoke workflow](../recorder/README.md). Commit synthetic inputs; keep installed content, captures and audits outside Git under [shared storage rules](../../AGENTS.md).
