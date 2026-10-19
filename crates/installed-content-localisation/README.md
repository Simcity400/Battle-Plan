# installed-content-localisation

Language-file merge order, substitutions, controls and numeric formatting. The implementation and its tests live in [`localisation`](src/localisation.rs); the same public paths are exported through `installed_content::localisation`. Shared parsing, identity and authored foundations come from [installed-content-core](../installed-content-core/README.md).

Resolve every authored input through the recorded `ResolvedLayers` content stack, including folders, archives, DLC, mods and replace_path. Preserve native order, lookup identity, duplicate handling, recovery and named/counted diagnostics. Keep evaluation and rendering in consumers.

Localisation arguments retain their native numeric types. `Float32` carries raw single-precision bits, multiplies percentages before promotion, and prints decimals with ties-to-even rounding. Its compact and trim directives have no effect; sign controls use the original value's comparisons, including signed zero and NaN. The byte and text APIs both return a named refusal for Float32 precision 7–9: the native formatter has formats only for 0–6 and otherwise reads an uninitialized format pointer. Refusals occur only for demanded operands, and recovery notes deduplicate repeated refusals. Raw byte results still preserve invalid UTF-8 for the native text decoder.

## Tests

Synthetic tests live beside the reader. Integration tests of independent readers live in `installed-content`; layer fixtures come from the core through dev-dependencies. `test-support` forwards the existing core test constructors and retains reader test gating. Runtime metadata tests initialize `BATTLE_PLAN_GAME_DIR`; ignored installation tests remain read-only. See the [installed-content contracts](../installed-content/README.md) and [project rules](../../AGENTS.md).
