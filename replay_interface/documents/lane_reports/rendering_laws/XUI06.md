# XUI06 — text and localisation renderer

Status: **PROVEN — promotion-ready strict admission law and production integration**

This packet is `XUI06`, not `UI06`. The existing `UI06` scrollbar packet was
not modified.

## Exact result

The production compositor now resolves every localisation key or literal text
template through one fail-closed admission path before glyph composition.
Source precedence is:

1. an exact engine-formatted runtime substitution recorded for the draw;
2. the effective localisation key selected for the recording language;
3. nested static `$KEY$` localisation;
4. a typed unavailable result, never a renderer-side guess.

Unformatted `$KEY$` recursion is exact and cycle/depth checked. A token with a
Clausewitz format suffix is keyed by its complete spelling (`VAL|%0`,
`COUNT|+0`, and so on) and is admitted only when its already-formatted result
is supplied. The same rule applies to `[data functions]` and dynamic
`§[colour functions]`. Thus percentage, precision, sign, abbreviation, date,
duration, plural, gender, article, case and adjective selection cannot drift
through a second implementation in the replay renderer.

Static `§<code>` escapes remain intact until the existing file-derived
`core.gfx` colour table is applied. Static bitmap-font layout continues to use
the authored glyph page/id, offsets, baseline, advance and pair kerning.
Missing glyphs remain diagnostics; there is no substitute box or fabricated
fallback.

The strict rejection set is explicit: missing localisation, missing runtime
variable, formatted value without its exact result, missing data function,
missing dynamic colour, inline icon without sprite layout, cycle, depth
overflow, and unterminated token. `Compositor` records these under
`unresolved_text_inputs` and suppresses only the affected draw.

## Production integration

`game_ui/src/text_law.rs` implements the resolver and typed failure contract.
`replay_core/src/runtime_text.rs` admits the exact A25 `runtime_text` carrier,
preserving absent, unobtained, and measured-empty states and validating final
bytes, source identity, producer RVAs, and duplicate semantic keys.
`ComposeOptions.runtime_text` carries exact formatted variables, data-function
strings and dynamic colour codes. `compose.rs` applies the resolver to both
localisation keys and literal/override templates before calling the bitmap-font
renderer. Captured runtime glyph and inline-icon layouts are consumed directly
when present. The module is public through `game_ui/src/lib.rs`.

Focused tests prove nested static resolution, preservation of section-colour
codes, exact runtime formatted/data-function admission, identity-guarded A25
setter results, strict carrier admission, and rejection of format guesses,
cycles, duplicate semantic rows, invalid hashes, dynamic colours and inline
icons without their required inputs.

The current focused replay-core and game-ui suites are rerun as part of this
freeze; their exact counts and hashes are recorded below. The real-install
acceptance probe remains the file-derived font/localisation authority.

- `cargo test -p game_ui --test xui06_text_renderer`: **3/3 pass**.
- `cargo test -p replay_core runtime_text`: **4/4 pass**.

## Corpus exhaustion

The mechanical verifier joins the installed a729 files, the effective
mod-over-vanilla extractor output, and production source:

- executable SHA-256 exactly
  `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`;
- 139,950 effective English keys from 238/238 files across `vanilla` then
  `AMSB - DEV`;
- 42 installed `common/scripted_localisation` files containing 1,529
  `defined_text` definitions;
- 206 installed vanilla English YML files and 379 BMFont files;
- 18,218 `$...$` tokens, 31,091 `[...]` tokens, 7,928 formatted variable
  tokens, 1,844 inline-icon tokens, and 13 dynamic-colour tokens in the
  installed English corpus.

This corpus proves that runtime formatting and evaluation are material input
classes, not hypothetical edge cases.

## Exact runtime carrier and data presence

Extractor row **A25** is frozen in
`extractor/documents/dossiers/A25_RUNTIME_TEXT_INPUTS.json`. It now supplies
compact domain 76, the `hoi4-runtime-text-v1` handoff schema, typed
`WorldFrame::runtime_text()` admission, and the following exact inputs:

- engine-selected scripted-localisation/plural/grammar result;
- exact output for the complete formatted substitution token;
- evaluated data-function string and dynamic colour;
- runtime-selected wrap, truncation, ellipsis, clip and alignment result when
  it is not reproducible from authored literal layout;
- fallback-font runs and inline-icon sprite/frame/baseline/advance placement.

This is a complete data boundary, not a partial renderer law. Static/nested
strings are promotion-ready without a carrier. A runtime-dependent draw is
promotion-ready when its authenticated A25 record is present; absent or
unobtained data rejects with the exact typed reason, while measured-empty stays
distinct. RenderDoc glyph quads can validate placement but cannot replace
semantic text/source identity.

The separate XUI11 politics-tooltip dependency is likewise registered as
extractor row **A24** and
`extractor/documents/dossiers/A24_POLITICS_TOOLTIP_INPUTS.json`; it is not
hidden in this rendering packet.

## Incidental findings retained

- The effective localisation JSON contains case-distinct keys (for example
  `AUS_ELIN` and `AUS_elin`), which Windows PowerShell's case-insensitive
  `ConvertFrom-Json` rejects as duplicates. The verifier uses a case-sensitive
  JSON parser and preserves both keys.
- The older tooltip wrapper in `hud.rs` is a whitespace heuristic. XUI06 does
  not promote it as an engine wrapping law; runtime-selected wrap/truncation is
  admitted only through A25.
- The existing generic markup parser already tokenises `£` icons so they do
  not leak into glyph measurement. XUI06 now makes the missing inline sprite
  placement a typed draw rejection instead of silently dropping the icon.

## Provenance and frozen tuple

Authority is `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

This is a FILE/corpus/production-admission packet. Ghidra function address,
job ID and Ghidra artifact path are **N/A** because no new binary claim or
Ghidra operation was used. The normative artifact is
`replay_interface/re_evidence/rendering_laws/XUI06/production_contract.json`.

- Production contract SHA-256:
  `3224759b851bdfd261328c8f1b79f9c306e6d660b3325b2e2f19b0fdb1e04626`.
- Resolver SHA-256:
  `9f34435e4cac2214935773409faf77e1825c56a5c6b67cb76c76ac8601368373`.
- Compositor SHA-256:
  `c2b5d883a06dd669003cd12f2444eb75b519553f6e89f26d4091120060342a76`.
- Focused test SHA-256:
  `6aa030ef68eb8fa7c57ba6920ded8423803c39910c8a3d1af048ba5f19572aa0`.
- Typed A25 replay admission SHA-256:
  `50810052110616707b64bc71fd89f4eaaae5512127b00fce5c39e81be51654bb`.
- Verifier SHA-256:
  `d3c7255bcafca350e00c1c7af9e10e91e426b6377dc981f77a7673b31427215b`.
- Verification SHA-256:
  `bce1536a9fbaa6552bd6ca436f918496107dcb34d078c0ea88a71e075bab15ac`
  (**42/42**, byte-identical across consecutive runs).
- Evidence manifest SHA-256:
  `76085c703e73f5f51efb228d0fbf97feaea859a7c3c6dab4cf5e1fd791d17bd8`
  (**12/12** artifact hashes match).
- Extractor A24 contract SHA-256:
  `c27175c4243502884c96909ca55cacd02bc4e5f8a86f8cebae334d275eb84355`.
- Extractor A25 contract SHA-256:
  `a1ef3d952b62a9a05cb0303e872b58f8821fe21db7ba409309a7a2b0e1eeb1b6`.

No `PROGRAM_STATUS.md`, canonical UI packet, collector, Ghidra artifact, or
adversarial-review surface was modified.
