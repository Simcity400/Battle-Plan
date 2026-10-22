# installed-content

Rust readers for the installed content used by recording, playback and save comparison. This crate exports the shared reader paths and owns integration tests of independent readers. [installed-content-core](../installed-content-core/README.md) owns content identity, shared parsing/dispatch, catalogues and map/asset foundations; the reader crates below own independently used content domains. [installed-content-save](../installed-content-save/README.md) owns text-save projections. It does not read live game memory.

Read the [project rules](../../AGENTS.md) before changing a reader. Consumers should use these shared readers rather than parse the same files again or add a Python runtime dependency.

## Layout

| Area | Entry points and responsibility |
|---|---|
| Content stack | [installed-content-core](../installed-content-core/README.md): identities, verification and overlays, native archives, installed-mod discovery and presentation mounts. |
| Text readers | [installed-content-core](../installed-content-core/README.md): byte-preserving data and balanced trees, the script lexer and persistent reader, database file order and number conversions. |
| Script dispatch | [installed-content-core](../installed-content-core/README.md): load-point names, shared trigger, tactic, effect and modifier body readers, name identity and unread suffixes. |
| Names and text | [installed-content-localisation](../installed-content-localisation/README.md), [installed-content-country-names](../installed-content-country-names/README.md), [installed-content-scripted-localisation](../installed-content-scripted-localisation/README.md), and scripted keys and defines in [installed-content-core](../installed-content-core/README.md). |
| Authored catalogues | Domain modules such as equipment, ideas and characters in [installed-content-core](../installed-content-core/README.md) and [installed-content-events](../installed-content-events/README.md) own database and lookup rules. [lib.rs](src/lib.rs) exports the modules and shared typed diagnostics. |
| Artwork and layout | Native GUI, meshes, particles and shaders in [installed-content-core](../installed-content-core/README.md); [installed-content-gfx](../installed-content-gfx/README.md). |
| Image decoding | [installed-content-image](../installed-content-image/README.md): native bitmap decoders and image representations, re-exported as `image`. |
| Map and movement | The map, province graph and movement in [installed-content-core](../installed-content-core/README.md), [installed-content-states](../installed-content-states/README.md), and the land/naval path and raid crates. |
| Preferences | [installed-content-settings](../installed-content-settings/README.md): read-only local preferences. |

Frame-free readers depend on the content foundations, executable metadata and identity types without depending on native readers or replay. The `installed-content` facade also exports readers of recorded rows, so frame-free consumers such as shader-pipeline depend on `installed-content-core` directly.

Internal consumers depend directly on the libraries that own their content
modules, including `installed-content-image` for image types. This facade retains
cross-reader integration tests and the module index. Shader-pipeline uses
`installed-content-core` without pulling in recorded rows or native readers.

[installed-content-sound](../installed-content-sound/README.md) (sound and music definitions, `NSound` defines, WAVE and Ogg Vorbis samples) is not re-exported: only the viewer and [viewer-audio](../viewer-audio/README.md) depend on it, so its audio decoder stays out of the recorder's builds.

A catalogue retains definitions, source identity and diagnostics. Duplicate handling, case sensitivity, defaults and enumeration order follow its native database, not a universal merge rule. Reuse catalogues and stack-scoped caches. Presentation and its shared [catalogue](../dashboard-text/src/catalogue.rs) belong to [dashboard](../dashboard/README.md).

Idea lookup shares the lexer's ASCII case identity and selects the first matching database entry; rereading a held group replaces that entry in place. Advisor lookup, ambiguity and coverage share its native folded string identity. Retain authored spellings for names and assets; localisation and sprite names keep their own case rules. Equipment graphics contexts retain a null archetype separately from a real empty equipment name: the null object's sentinel hash cannot match an authored empty pool key, while a real empty name can.


## Reader crates

| Crate | Responsibility |
|---|---|
| [installed-content-localisation](../installed-content-localisation/README.md) | Language merge order, substitutions and controls. |
| [installed-content-country-names](../installed-content-country-names/README.md) | Recorded country naming and selection rules. |
| [installed-content-unit-path](../installed-content-unit-path/README.md) | Land/transfer paths and selected movement text. |
| [installed-content-naval-path](../installed-content-naval-path/README.md) | Naval region/province route searches. |
| [installed-content-raids](../installed-content-raids/README.md) | Raid types, naming, targets and tooltips. |
| [installed-content-weather](../installed-content-weather/README.md) | Weather entities, thermal intervals and temperature content. |
| [installed-content-states](../installed-content-states/README.md) | Authored state history and membership. |
| [installed-content-equipment-graphics](../installed-content-equipment-graphics/README.md) | Graphics pools, equipment contexts and map/state joins. |
| [installed-content-settings](../installed-content-settings/README.md) | Read-only local game/display settings. |
| [installed-content-regions](../installed-content-regions/README.md) | Strategic regions and ordered weather periods. |
| [installed-content-division-names](../installed-content-division-names/README.md) | Division and raid name groups. |
| [installed-content-events](../installed-content-events/README.md) | Event declarations and ordered options/effects. |
| [installed-content-focus](../installed-content-focus/README.md) | Focus file order, parse waves and definitions. |
| [installed-content-scripted-localisation](../installed-content-scripted-localisation/README.md) | Scripted text alternatives, triggers and random lists. |
| [installed-content-gfx](../installed-content-gfx/README.md) | Graphics descriptors and GUI/sprite registration. |
| [installed-content-abilities](../installed-content-abilities/README.md) | Commander ability definitions and adjusters. |
| [installed-content-province-area](../installed-content-province-area/README.md) | Province area in native map cells. |
| [installed-content-naval-combat-defines](../installed-content-naval-combat-defines/README.md) | Typed naval combat define operands. |
| [installed-content-state-categories](../installed-content-state-categories/README.md) | State-category identities and modifiers. |
| [installed-content-wargoals](../installed-content-wargoals/README.md) | War-goal definitions and naming. |

## Recorded content stack

Resolve every content fact through `ResolvedLayers`: **vanilla, DLC, then mods in recorded order**. Later layers win file lookup; `replace_path` hides lower layers under its path. File overlays do not determine database duplicate rules. Support folder/archive layers and the engine's built-in virtual roots.

`gather` prepares a new inventory. `native_mod_order` follows dependencies and descriptor path bytes, not launcher list order. Verification requires the collector to match the live successful search path and active DLC/mod rosters. Duplicate display names that prevent proving mount order leave the session unverified.

`resolve` uses recorded IDs, order, hashes and coverage policy, independently of current launcher selections. Machine paths and descriptor locations stay outside portable identity. Caches must preserve verification on hits and misses; changed, added or removed covered files are errors.

Folder layers stamp, look up and hash their files on the machine's cores; digests reach the cache in file order, and a failing file keeps the digests of the files before it. The persistent content-hash cache is written only when entries change or an absent/unreadable cache needs rebuilding. An unchanged valid cache remains byte-for-byte intact. Verification and read statistics still cover every selected file; successful hashes persist even when a later layer refuses resolution. Cache write errors remain failures whenever a write is needed.

Use stack listing/read APIs. `script_files::paths` and `ResolvedLayers::directory_files` list direct files in merged filename byte order. Follow each loader's suffix and recursion rules; recursive loaders generally read files before subdirectories.

`ResolvedLayers::recorded_source_files` returns physical inputs in recorded layer
order for prepared-file size and modification-time checks: descriptors, each
folder's inventoried files, and archive paths once per mount. It does not read
their payloads or change native listing/lookup order.

Consumers retain derived caches through `ResolvedLayers::derived`: clones share
one resolved stack, another stack or mount roster has independent entries, and
the presentation stack has its own. Cached reads retain the stack's shared
diagnostics. Archive lookups verify every mounted record on each open; covered
payloads still verify on every read. Prefix reads retain only header bytes while
streaming verification of the full covered payload. Readers keep content inputs
immutable during one resolved load.

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

Static keyword, modifier, trigger, effect, equipment, rule and AI metadata comes from `engine_executable::executable_tables`, initialized explicitly from the selected installation's hash-verified executable. The process shares one immutable table set. Content binding validates the selected installation before using it; parsers do not guess an installation or fall back to bundled names. Authored definitions still use `ResolvedLayers` and the load point's `KeyNames`.

Built-in variable names come from the same installation's `documentation/dynamic_variables_documentation.md`. The reader extracts declaration headings by scope, excludes examples, preserves exact spelling and checks sorted names against the supported semantic digests. Missing, changed or duplicate declarations refuse initialization. Mod documentation does not replace executable metadata. Global, country, state and character variable stores use these names; the character scope is documented as `unit_leader`.

Retain conditions and effects as data. Evaluation and descriptions over recorded state belong to [dashboard triggers](../dashboard-text/src/triggers.rs) and their consumers.

## Localisation, defines and scripted text

[Localisation](../installed-content-localisation/src/localisation.rs) uses language headers and native line syntax, not YAML or filename-derived languages. Ordinary files in layer/path order keep the first key; `replace/` files run afterwards and overwrite, even over ordinary files in later layers.

Use `Localisation::localise`, `localise_with` or `localise_bytes` for substitution/formatting, and `pieces` for stored controls. Do not interpret raw values with a consumer's colour or substitution grammar. Keep native missing-key behaviour and absent/empty distinctions. `format_scripted_variable` shares numeric formatting with scripted text.

Scripted localisation retains ordered alternatives, triggers and random-list operands; ScriptedKey retains conditional choices. Consumers supply recorded scopes and shared evaluation with each caller's selection rules. Plain text and evaluated `[...]` text are different paths: use dashboard's catalogue and [scripted text](../dashboard-text/src/scripted_text.rs) helpers. Missing runtime operands are named refusals.

[Defines](../installed-content-core/src/defines.rs) runs stack scripts in native recursive order on vendored PUC Lua 5.1, including arithmetic, functions and loops. Its sandbox excludes machine-access libraries/file loaders and bounds execution and memory. Script failure refuses the reader; it never substitutes vanilla values. Use its typed getters for numeric, string and boolean defines. Resolved stacks share one immutable evaluated state after repeating the ordinary verified script reads in native order; independent script loads use `Defines::new`.

## Asset contracts

`ResolvedLayers::covered`/`covered_path` test the recording's hash policy; `content.rs` owns its roots. Ordinary reads outside coverage require `report_uncovered_reads()`: retain the shared `UncoveredReads` handle and refresh diagnostics with `append_to` after lazy reads. Explicit `read_uncovered` consumers keep asset-specific warnings and contribute their attempted uncovered paths, including missing candidates, to that shared handle when reporting is enabled. Both paths verify covered bytes, including covered folder aliases. Do not expand coverage incidentally.

Name uncovered bytes in diagnostics and machine-readable output, including assets only newer policies cover. Loading a file does not prove hash verification.

Use [gui::native::resources](../installed-content-core/src/gui/native/resources.rs) for sprite/font registries, texture candidates and frame geometry; [image::native](../installed-content-image/src/native/mod.rs) supplies native decoding. Pass the client's `settings::SettingsKeys::texture_decode`: `force_pow2_textures` changes extents, sizing and UVs. Distinguish native failure from undefined/device-dependent output. Sprite names are exact bytes; refused declarations are not absent names.

Preserve palettes and image origins, authored mip chains, cube faces, glyph metrics, mesh inheritance, skeleton order and attachment transforms. Resource roles determine linear or sRGB sampling. The approved BC7-only exception reads logical DDS extents, the complete declared mip chain and header colour space; its unsupported forms remain named refusals in [load_dds.rs](../installed-content-image/src/native/load_dds.rs). Other formats keep native decoding.

[PdxShader](../installed-content-core/src/pdxshader/program.rs) preserves native effect/include order and hashes resolved inputs. Missing sources or unsupported declarations never select an unrelated effect. Compilation, GPU caches and bindings belong to [shader-pipeline](../shader-pipeline/README.md).

Artwork uses `program::read_reported`: ordinary uncovered-source reads join the shared read warnings, and missing-include notes join the stack's lazy shader diagnostics. Reporting consumers retain both after rendering.

## Visual mods

A user can show any installed mod's artwork over a recording that was not made with it. `visual::installed` lists every mod the game could mount: each descriptor directly in the user folder's `mod` directory, and each item of the Workshop folder in the game's own Steam library, mounted in place from its `descriptor.mod` as the launcher mounts it (a `path` there is ignored; an `archive` names a file in the item's folder). A user descriptor of a Workshop item and descriptors naming one source are one mod. Selection ids are `workshop/<item>` or `mod/<file>` and are matched against this catalogue, never joined to a path; a selection keeps its name so a mod no longer installed is still named. A descriptor, source, Workshop folder or listing that cannot be read is listed by name with its problem, and the rest still list. Nothing is downloaded, enabled or written, and there is no eligibility check: a mod's other files never keep its artwork from being shown.

`visual::apply` verifies nothing new and changes no recorded layer. It mounts the selection as ordinary folder or archive layers in `ResolvedLayers::presentation`: the recorded layers, then the selected mods in `native_mod_order` (dependencies, then descriptor path bytes; a cycle shows none of the selection), then the built-in layers the engine mounts above every mod. Each mount keeps its descriptor's `replace_path` for lookups and listings alike. The engine mounts one mod of a display name: a selected mod named as a recorded mod it is not stays hidden, and of selected mods sharing a name the first in mount order is shown, each by name. A mod is mounted as the game mounts it: symbolic links are not followed, an archive keeps names that differ only in case apart, and a folder or archive name in another case than a lower layer's stays its own file, since each layer's lookup stays native; recorded layers still refuse these, which verification cannot order. A mod that cannot be mounted with the others is left out by name and the rest still mount. Mounted layers share the recorded layers' inventories rather than copying them. A selected recorded mod mounts again at its new precedence and still verifies its recorded bytes; another mod's bytes are not verified and are counted per mod in the read diagnostics (`UncoveredReads::overlay_warning`), apart from uncovered recorded reads. An empty selection mounts nothing. What cannot be mounted is a named warning in the returned plan, and the rest still mount.

A visual mod shows only under the presentation roots `gfx`, `interface`, `localisation` and `map/terrain`, and in the world normal map (`map/world_normal` as `.dds`, `.tga` or `.bmp`); every other path, its files and its `replace_path` included, resolves from the recorded layers alone, in lookups and listings alike. A reader handed the presentation stack therefore reads recorded scripts, history and map geometry, and only artwork, interface, fonts, entities, shaders and text from the mods. `presentation_defines()` is the same stack with the mods' `common/defines` showing too: graphics consumers evaluate it as a second, natively merged defines state (recursive file order, replacement, arithmetic and functions as `defines` runs them), while simulation consumers keep the recorded defines. `Defines::with_graphics_of` builds what graphics consumers read: the recorded defines with the merged state's `GRAPHICS_SECTIONS` (graphics, aircraft graphics, interface, front end, map modes, map icons), every simulation section recorded, and a graphics section or value the mods retype or delete kept recorded and named. Each table is copied once, so a table a define state shares stays shared and one that holds itself is copied as the same cycle.

Map geometry, simulation inputs and every recorded fact read the recorded stack itself; artwork consumers read `presentation()`, whose memos and derived caches are its own. They load through `ResolvedLayers::artwork`, called on either stack: artwork a mounted mod cannot supply (a file or loader its reader refuses, or a lower file its `replace_path` hides) is loaded from the recorded layers instead (`without_overlays` below a presentation stack, `recorded_only` for the recorded stack) and named with why (`UncoveredReads::overlay_refusals`), so a mod never keeps a recording from opening. A loader that reads items apart from the rest (a tree kind, an arrow texture, an aircraft rig) reads each through `ResolvedLayers::artwork_item`: an item the mods break comes from the recorded layers, named, and one neither stack gives is refused by a stable name (`note_refused`, read back through `refused` on the stack or its `UncoveredReads`) for the map's fallback. Which consumers read it is listed in the [viewer guide](../viewer/README.md#campaign-appearance); the dashboard's widgets read it too ([dashboard](../dashboard/README.md#widget-presentation)).

Every such fallback is named and counted by one line (`content::overlay_refusal_line`), which the map's `UncoveredReads::overlay_refusals` and each widget's reply share. A consumer that shares one stack with others reports its own: `content::observe_overlay_refusals` returns, with what it runs, every `note_overlay_refusal` on its thread meanwhile, whichever stack noted it, and a memo of artwork loaded once replays what its load fell back on to each later user (`content::credit_overlay_refusals`). `ResolvedLayers::with_own_overlay_diagnostics` is a stack sharing every inventory, memo and cache whose visual mod diagnostics start empty, so a presentation mounted on it reports apart from every other. `Defines::graphics_for` gives the graphics defines an artwork consumer of a stack reads, naming what it keeps recorded, for the map and widgets alike.

`visual::revision` digests what a selection mounts now (mount order, sources, descriptor stamps and the stamp of every file the mods show) from metadata alone; `apply` records it in its plan as what it mounted: a mod that changes while it mounts is mounted again, so the revision before and after the mount agree. A map that loaded with another revision is stale: a mod was installed, updated or removed since.

## Native GUI reader

`gui::native::read_layers` shares context across interface files and runs the native post-load pass. Use its registry and local/recursive lookup helpers, preserving template ownership. Layout, construction and lifetime belong to the [viewer](../viewer/README.md).

Dependent lookups/draws retain element and subtree refusals for undefined reads, missing inputs or inheritance that cannot finish. Viewport-dependent fields require `read_layers_at`/`read_at` with the native GUI viewport. Sprites/fonts share the lexer, variables and number conversions.

## Map movement and raids

Map readers share definition rows, raster loading, geometry and adjacencies. Resolve file names/dimensions from the stack; record positions, authored IDs and terrain ordinals are distinct. Map area uses raster cells without physical-area calibration.

[weather/temperature.rs](../installed-content-weather/src/weather/temperature.rs) resolves what playback's temperature pass reads: `common/weather.txt`'s temperature settings, window and terrain modifiers, each province record's activity, neighbours, terrain and last-listing strategic region, and each region template's weather periods ([regions.rs](../installed-content-regions/src/regions.rs)). [weather.rs](../installed-content-weather/src/weather.rs) also gives the rules of a province weather record's modifier rebuild: the thermal and ground snow intervals and `snow_gain_max`.

World interfaces supply recorded movement/raid facts. Missing admission inputs stay unknown; do not infer port sea access or runtime caches from nearby assets. [raids/target.rs](../installed-content-raids/src/raids/target.rs) shares target/name joins. The [raid tooltip contract](../viewer-raids/src/map/raids/tooltips/README.md) owns consumer and capture rules.

[Selected movement searches](../installed-content-unit-path/src/unit_path/selected.rs) use recorded current-hour world operands and preserve native neighbour order and caller-specific admission. Missing runtime facts remain refusals.

## Refusals

Keep absence, valid emptiness, native recovery, parse failure and unsupported content distinct. Preserve producer-assigned `DiagnosticKind` and named, counted diagnostics. Refuse unsupported fields/classes/definitions with file/line context; never drop them or substitute vanilla defaults.

`Read::misreads` records recovery; `Read::unsettled` means consumption cannot be followed. `Deferred effect` warnings do not prove complete reads. An unread suffix can alter later objects: use database coverage tracking to refuse dependent lookups, including absence or earlier definitions. This uncertainty is separate from hash coverage.

Technology description catalogues retain every indexed declaration in native order, while case-insensitive name lookup selects the first. The technology database keeps each technology's research folders in authored order, a later definition replacing an earlier one's; `read_folders` reads `common/technology_tags` folder definitions in directory order with their `available` triggers, keeping a repeated folder's first definition and refusing the repeat by name.

## Adding or changing a reader

1. Identify the loader's file order, lexer mode, field and lookup rules. Describe their behavior beside code.
2. Extend the owning module/shared dispatch. Use `ResolvedLayers`, load-point names and native conversions.
3. Preserve order, defaults, redefinitions and required operands; propagate diagnostics and uncertain suffixes through lookups.
4. Add synthetic regressions for changed behaviour, layered replacements and reached mod forms. Reuse helpers (`test-support` exposes cross-crate fixtures) and update affected consumers together.

[Selected transport](../installed-content-unit-path/src/unit_path/selected/transport.rs) preserves early returns, per-occurrence rounding and independent output buffers; its message law asks for each operand only when a native branch reaches it.

## Checking changes

Follow [AGENTS.md](../../AGENTS.md) for crate-scoped formatting, clippy and affected tests. Run the owning reader packages together with `installed-content` for their integration tests. Layer changes need order, folder/archive, `replace_path`, identity-failure and changed-file cases. [mod_layers_tests.rs](src/mod_layers_tests.rs) provides synthetic stacks; other tests live beside readers.

Tests that exercise runtime keyword or registry metadata call `engine_executable::executable_tables::initialize_for_tests()`. Set `BATTLE_PLAN_GAME_DIR` to the supported installation for those tests; an unset variable or incompatible installation is an explicit failure. Fixtures supply synthetic authored content while the immutable metadata comes from that installation. Equipment fixtures initialize this metadata before loading authored content, and shader parsing tests initialize it before using the shared lexer. Tests limited to table decoding and byte lookup use synthetic data without an installation.

Real-game tests are ignored and read-only. Set `BATTLE_PLAN_GAME_DIR` to the game folder; select the owning reader package and run `cargo test -p installed-content-core <filter> -- --ignored --nocapture` for core readers from the repository root. Example filters: `every_shipped_gui_file_reads`, `every_shipped_gfx_file_reads`. Check each test's environment: corpus audits may require `BATTLE_PLAN_WORKSHOP_DIR`; special projects use `BATTLE_PLAN_SP_GAME`/`BATTLE_PLAN_SP_WORKSHOP`. Missing variables can skip a test, so confirm it read files.

Compare relevant DLC/mod diagnostics with the [viewer corpus workflow](../viewer/README.md#mod-corpus-check); identity support alone does not prove reader coverage. Asset changes follow [rendering rules](../viewer/README.md#rendering-rules); integration follows the [recorder fresh-save/smoke workflow](../recorder/README.md). Commit synthetic inputs; keep installed content, captures and audits outside Git under [shared storage rules](../../AGENTS.md).
