# installed-content

Rust readers for the installed content used by recording, playback and save comparison. This crate owns content layers, identity verification, asset parsing and text-save parsing. It does not read live game memory.

## Responsibilities

Use the existing parsers under [src](src/) rather than adding a second parser in a consumer. Clausewitz text, descriptors, localisation and saves should keep consistent parsing rules; binary assets use their corresponding loaders. The application should not acquire a Python runtime dependency to parse installed content.

Resolve content in recorded order: vanilla, DLC and mods. Support folder/archive layers and honor `replace_path`. Do not sort layers into a new order or let a convenient local file silently override the recorded content.

Content identity uses the versioned coverage policy and layer hashes. A local cache can avoid redundant hashing, but it must not weaken verification. Machine paths are local configuration, not portable replay identity.

## Refuse mismatches clearly

The viewer must refuse missing or mismatching recorded content. An unverified identity is also refused unless the user explicitly enables `--allow-unverified-content`. That override keeps a visible warning and does not waive hash checks.

Keep absent files, unknown data and parse failures distinct from valid empty values. Preserve exact fixed-point and integer representations for save comparisons. Do not fill missing runtime caches from guesses just because a text save cannot represent them; the oracle should state the limitation.

## Asset contracts

Loading an asset does not prove its bytes were verified. [content.rs](src/content.rs) owns the versioned coverage roots. `ResolvedLayers::read_uncovered` can load visual assets outside those roots, but those bytes are not hash-verified and the viewer must say so. Covered paths still use normal verification. Do not silence this warning or expand the coverage policy incidentally during a rendering change.

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

`focus.rs` reads national/shared/joint/continuous focus presentation definitions with the shared parser and recorded layer ordering. It reports malformed files individually. It also reads authored costs and literal focus-day defines; scripted values remain unavailable rather than executing Lua. Conditional icon scripts remain unavailable unless a static icon is authored. `image/rgba.rs` expands the shared DDS reader's BC1/2/3 blocks for bounded UI artwork, preserving alpha and cropping partial blocks; native map uploads retain their compressed path.

[defines.rs](src/defines.rs) evaluates every `common/defines/**/*.lua` of the stack on vendored PUC Lua 5.1, the game's runtime, in the native load order (byte-ordered names, files before subdirectories). The sandbox has no `io`, `os`, `package`, `debug` or file loaders and bounds the instruction count; a failing script refuses the reader rather than falling back to vanilla values. `fixed5` reproduces the native fixed-point getter. `ResolvedLayers::uncovered_from_base` tells whether an uncovered file is still the base game's own.

`technology.rs` reads unambiguous equipment unlocks from authored technology definitions for generic name/description fallback. Multiple or scripted unlocks remain unavailable; later definitions without an equipment link clear earlier links. Resolution uses the shared parser and recorded layer ordering.
