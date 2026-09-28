# installed-content

Rust readers for the installed content used by recording, playback and save comparison. This crate owns content layers, identity verification, asset parsing and text-save parsing. It does not read live game memory.

## Responsibilities

Use the existing parsers under [src](src/) rather than adding a second parser in a consumer. Clausewitz text, descriptors, localisation and saves should keep consistent parsing rules; binary assets use their corresponding loaders. The application should not acquire a Python runtime dependency to parse installed content.

Resolve content in recorded order: vanilla, DLC and mods. Support folder/archive layers and honor `replace_path`. Do not sort layers into a new order or let a convenient local file silently override the recorded content.

Content identity uses the versioned coverage policy and layer hashes. A local cache can avoid redundant hashing, but it must not weaken verification. Machine paths are local configuration, not portable replay identity.

## Refuse mismatches clearly

The viewer must refuse missing or mismatching recorded content. An unverified identity is also refused unless the user explicitly enables `--allow-unverified-content`. That override does not waive hash checks. Its warning shows in the viewer's diagnostics mode and in the machine-readable outputs, and exported and captured images keep their UNVERIFIED CONTENT banner (owner decision, 2026-09-25).

Keep absent files, unknown data and parse failures distinct from valid empty values. Preserve exact fixed-point and integer representations for save comparisons. Do not fill missing runtime caches from guesses just because a text save cannot represent them; the oracle should state the limitation.

## Asset contracts

Loading an asset does not prove its bytes were verified. [content.rs](src/content.rs) owns the versioned coverage roots. `ResolvedLayers::read_uncovered` can load visual assets outside those roots, but those bytes are not hash-verified and the viewer must say so: in diagnostics mode and in the machine-readable outputs (owner decision, 2026-09-25). Covered paths still use normal verification. Do not remove this warning or expand the coverage policy incidentally during a rendering change.

Preserve palette indices and image origin in [BMP](src/image/bmp.rs), and authored mip levels, cube faces and original extents in [DDS](src/image/dds.rs). The viewer chooses linear or sRGB sampling for each resource's role; there is no correct blanket conversion for all textures.

[Mesh and entity loading](src/mesh.rs) preserves field-by-field clone inheritance, subobjects, skeletons and attachment transforms. Unlabelled subobjects are not interchangeable with LODs. [Bitmap fonts](src/fonts.rs) and [localisation](src/localisation.rs) retain authored metrics, colour runs and glyph assets rather than approximating them with a system font.

## Checking changes

Use existing parser and synthetic input tests plus the [shared local checks](../../AGENTS.md). Changes affecting layer resolution must exercise order, replacement and identity failures. Asset interpretation changes also need review against the [viewer rendering rules](../viewer/README.md#rendering-rules).

Localisation verifies and parses independent files with a fixed small worker pool, then merges results in the original layer/path order. Replace-file precedence and duplicate diagnostics remain unchanged; filenames never substitute for language-header parsing.

Vanilla with the relevant DLC is the established first validation path; mod identity support is not proof that every mod combination has been tested. Live captures and large reference assets stay outside source control.

## Local cosmetic overrides

[content/visual.rs](src/content/visual.rs) reads installed mod descriptors using the shared parser, including folder and archive sources. Eligibility uses the installed game's checksum_manifest.txt independently of the replay's broader content hash policy. A checksum-sensitive file conservatively excludes a mod; this is a file-policy check, not a live engine checksum comparison. Unsupported manifest forms are refused rather than guessed.

Cosmetic eligibility and renderer compatibility are separate: the initial overlay supports existing DDS/TGA/PNG/BMP artwork under gfx and map/terrain. Shader programs, GUI layouts, non-artwork files and replace_path mods are reported as unavailable instead of silently partially applied. File listing is bounded and cancellable; asset bytes are read only while constructing a selected map. The selector does not download Workshop content or edit the game's enabled-mod list.

Original layer hashes are verified before presentation overrides. Explicit selections remove supported recorded cosmetic layers from the presentation copy and overlay selected artwork in order; gameplay layers remain intact. Missing/invalid selected assets fail visibly. These overrides carry their own diagnostic warning and are not represented as recorded artwork.

[settings.rs](src/settings.rs) provides bounded, read-only access to the configured user folder's settings.txt through the shared Clausewitz parser. The desktop and map use this for inherited preferences; it never modifies the game file or reads live game memory.

For the map UI scale, `settings::Display` reads settings.txt's `graphics` block and then the launcher's pdx_settings.txt, in the engine's order: the scale is clamped while settings.txt is read, pdx_settings.txt entries then change the mode, monitor and size in file order, and the window and interface layout follow the native rules cited beside the code. Absent keys keep the engine's constructor values and pdx values read as its `atoi` does; a present value the reader cannot parse is refused. The game's command-line display overrides are not visible and not applied. `settings::desktops` lists the monitors in SDL's display order with their physical modes and effective DPI, which is why this crate depends on `windows-sys` on Windows; it refuses a thread that is not per-monitor DPI aware and a monitor whose DPI cannot be read, and it walks every display adapter, so callers read it off the frame path.

`focus.rs` reads national/shared/joint/continuous focus presentation definitions with the shared parser and recorded layer ordering. It reports malformed files individually. It also reads authored costs and literal focus-day defines; scripted values remain unavailable rather than executing Lua. Conditional icon scripts remain unavailable unless a static icon is authored. `image/rgba.rs` expands the shared DDS reader's BC1/2/3 blocks for bounded UI artwork, preserving alpha and cropping partial blocks; native map uploads retain their compressed path.

[defines.rs](src/defines.rs) evaluates every `common/defines/**/*.lua` of the stack on vendored PUC Lua 5.1, the game's runtime, in the native load order (byte-ordered names, files before subdirectories). The sandbox has no `io`, `os`, `package`, `debug` or file loaders and bounds the instruction count; a failing script refuses the reader rather than falling back to vanilla values. `fixed5` and `int32` reproduce the native fixed-point and integer getters; `text` reads string defines such as entity names. [flags.rs](src/flags.rs) is the engine's flag decoder, shared by counters, the combat UI and the dashboard: exact define-sized uncompressed Targas, always read bottom-up. `ResolvedLayers::uncovered_from_base` tells whether an uncovered file is still the base game's own.

Coverage policy 2 adds every `.asset` and `.gfx` below `gfx`: the engine registers entities, meshes and particles from all of them, so a mod that changes one changes the content identity. Meshes, animations and textures stay uncovered. [mesh.rs](src/mesh.rs) `Roster` reads every descriptor in recorded (layer, path) order and names repeated or cross-file-cloned entries, which consumers refuse; `TextureIndex` is the engine's `gfx/models` basename index for material textures, and a texture it cannot resolve refuses the mesh. [buildings.rs](src/buildings.rs) reads building types and spawn buckets in the engine's database order and names the buckets a differing repeated declaration makes ambiguous. `slot_index` resolves `map/buildings.txt` slot names folding ASCII case, as the engine does; buckets whose names differ only in case are ambiguous too.

`ResolvedLayers::directory_files` lists one directory as a database loader does: no recursion, byte order of names. [train_gfx.rs](src/train_gfx.rs) reads the supply-train database, the graphical-culture registry and equipment archetypes with the engine's merge and lookup rules.

[weather.rs](src/weather.rs) resolves the `common/weather.txt` `weather_effects` entities to particles; the snow ceiling and permanent snow are recorded, not derived. [particles.rs](src/particles.rs) indexes `pdxparticle` objects from every `.gfx` below `assetviewer_interface`, `interface` and `gfx`, and particle blocks by `name` from every `gfx/particles/**/*.asset`; a name declared twice is refused. `ResolvedLayers::descriptor_paths` lists effective descriptor files whatever the coverage policy.

[movement.rs](src/movement.rs) reads the static province movement map once from the files `map/default.map` names: native ordered neighbours (raster scan plus authored adjacencies, shared with `RailwayAdjacency`), each neighbour's fixed5 distance, crossing kind and arrow crossing, and each province's terrain flags and ordinal. `common/terrain/*.txt` is one database, read once by `terrain::read_definitions` in native order (`directory_files`): a redeclared category appends another entry, a lookup takes the first case-insensitive match, a graphical terrain's `type` resolves against the categories read before it and the last palette writer wins; river kinds use the define index ranges. A missing `adjacencies` key or file means no authored adjacencies, as in the engine. Unsupported content (definition IDs that differ from record positions, unknown kinds, non-8-bit rivers) is refused with a reason, and a neighbour whose sampling fails is listed as unavailable rather than guessed. The native evidence is in the module comments.

[resources.rs](src/resources.rs) reads each strategic resource declaration's icon frame from `common/resources` in load order, keeping a repeated token as another declaration as the game does; which resources exist and their order come from the recording.

`technology.rs` reads unambiguous equipment unlocks from authored technology definitions for generic name/description fallback. Multiple or scripted unlocks remain unavailable; later definitions without an equipment link clear earlier links. Resolution uses the shared parser and recorded layer ordering.

Politics content readers share [script_files.rs](src/script_files.rs) (a content folder's files in layer then path order; a file that ends inside an open block is closed as the game closes it, with a warning, via `parser::parse_unclosed`, which the interface reader also uses; an unparsable file is left out with a warning) and [modifiers.rs](src/modifiers.rs) (modifier blocks kept as authored: numbers, `yes`/`no` flags and the file's own `@variables`; `hidden_modifier` values are kept apart because they apply without showing; anything else, such as `[?value]`, stays `Unsupported` with its text):
- [ideas.rs](src/ideas.rs): `common/idea_tags` categories (slots, character seats, type, `politics_tab`, ledger, costs) and `common/ideas` ideas (group, law/designer/default, name override, picture, modifiers, targeted modifiers, equipment and research bonuses, traits, costs). A later definition of a token replaces the earlier one, with a warning. Conditions and effects are not evaluated.
- [characters.rs](src/characters.rs): advisor roles by idea token, country leader roles by (character, ideology), names and portraits by (category, size). Conditional `instance` alternatives are used only where they agree; disagreeing roles are left out and reported. The viewer's headquarters portraits use the same parser.
- [leader_traits.rs](src/leader_traits.rs): `common/country_leader` traits (country leaders and advisors) with their modifiers, equipment bonuses and targeted modifiers.
- [ideologies.rs](src/ideologies.rs): ideology groups, their colours (`{ r g b }` or `rgb { }`; `hsv` stays unavailable) and sub-ideologies.
- [modifier_definitions.rs](src/modifier_definitions.rs): scripted modifiers' display fields. Built-in modifiers are defined in the executable; their display comes from the recording.
