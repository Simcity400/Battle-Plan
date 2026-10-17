# installed-content-sound

Authored audio definitions and their sample files. [`sound`](src/sound.rs) reads the sounds, sound effects, falloffs, categories and compressors of `sound/**/*.asset` and the `NSound` defines; [`combat_sounds`](src/combat_sounds.rs) reads the battle ambience layers of `sound/combat_sounds/*.txt` and the `NSound` battle defines; [`music`](src/music.rs) reads the music tracks of `music/**/*.asset`, the song lists of `music/**/*.txt` with their stations and `chance` bodies, and the user's enabled stations; [`wav`](src/wav.rs) decodes RIFF WAVE samples in the encodings the game's loader opens (PCM, 32-bit float and G.711 A-law and mu-law directly; the IMA and Microsoft ADPCM files mods ship through Symphonia), reporting the sample format SDL's WAVE loader gives the game and the float values SDL's converter reads from it and [`ogg`](src/ogg.rs) Ogg Vorbis tracks, whole or streamed a packet at a time (Symphonia 0.6, MPL-2.0, with only its `ogg`, `vorbis`, `wav` and `adpcm` features). The viewer and viewer-audio depend on this crate directly; `installed-content` does not re-export it, so recorder builds never compile the decoder. Shared parsing, identity and authored foundations come from [installed-content-core](../installed-content-core/README.md); playback belongs to [viewer-audio](../viewer-audio/README.md).

Resolve every authored input through the recorded `ResolvedLayers` content stack, including folders, archives, DLC, mods and replace_path. Sound assets and samples are outside the recording's hash coverage: they are read with `read_uncovered` and join the stack's uncovered-read diagnostics.

## Rules

- Asset files load in the recursive load order (files before subdirectories, each level in byte order). A sound's `file` is relative to its asset file's folder.
- The first sound, sound effect or falloff of a name is kept; later ones are refused by name. Effect sounds and falloffs and category members resolve after every file is read; a later category takes an effect over, and a repeated category name adds to the same category, a later `compressor` block replacing the earlier one. A sound's `priority` (default 1) ranks its voices in the mixer.
- Missing references are named, counted refusals: a missing falloff leaves the effect unattenuated, a missing sound leaves its choice out, an effect in no category plays outside every category.
- Defaults follow the engine's readers: falloff minimum 0, maximum 100, height scale 1, logarithmic; effect volume 1, `max_audible` unlimited with `silent` overflow, playback rate 100%, `random_sound_when_looping` on; compressor gains 0 dB, ratio 1 (a zero ratio is 1), enabled. Playback-rate values are percentages; compressor gains are decibels held linear.
- A song list's `music_station` names the station of the songs after it in that file; a song before any station, or naming no track, is refused by name. `chance` bodies are read as mean-time bodies with the load point's trigger names and kept as data; their evaluation belongs to the consumer.
- A music track (`music/**/*.asset`) reads `name`, `file` and `volume`; any other field, or a block where a scalar belongs, is refused by name.
- Unknown fields, malformed values and unsupported WAVE forms (other compressed encodings, 64-bit float, more than two channels) are refused by name and counted.

Runtime metadata tests depend directly on `engine-executable`, keeping the native reader and replay chains out of audio builds and test builds.

## Tests

Synthetic tests live beside the readers and decoders, with layered fixtures from the core; the song-list test reads trigger names, so it needs `BATTLE_PLAN_GAME_DIR`. `installed_sounds_resolve_and_decode` and `installed_music_resolves_and_decodes` are ignored and read-only: set `BATTLE_PLAN_GAME_DIR` and run `cargo test -p installed-content-sound installed -- --ignored --nocapture`. See the [installed-content contracts](../installed-content/README.md) and [project rules](../../AGENTS.md).
