# XUI25 — display/language/input variants

**Status: PROVEN — PROMOTION-READY EXACT VARIANT ADMISSION AND LAUNCH CARRIER**

XUI25 implements both the production selection boundary and its launch-bound
carrier. `recorder_host` reads the corroborated user directory's `settings.txt`
twice under the existing metadata/process-start stability bracket. One
byte-identical read supplies language, physical viewport, `min_gui`, exact GUI
scale bits, refresh rates, fullscreen/borderless mode, and a settings-file
SHA-256. Fold/repack, replay ingest, and the typed world view validate and carry
the exact object into `game_ui`.

The exact variant key is physical viewport width/height, `min_gui`, display
mode, GUI-scale IEEE-754 f32 bits, recorded localisation language, and input
mode. Each admitted envelope also binds a source record and evidence-manifest
hash, producer-resolved logical-layout SHA-256, complete physical-pixel-oracle
SHA-256, exact font file/atlas/ordered-glyph-set hashes, final half-open
physical-pixel clips, and—when applicable—the exact post-resize transition and
keyboard/controller focus owner/traversal. Unsupported keys reject before
composition. The current promoted input family is pointer dispatch
(`MouseKeyboard`); keyboard-only and controller variants reject explicitly.

## Evidence disposition

The complete admitted matrix currently contains exactly the labelled 1920x1080,
GUI-scale 1.0, English, pointer-family UI control. A second 2752x1152 viewport
is established by camera and research evidence, but not as a complete all-UI
pixel oracle. It is therefore not admitted. The retained metadata
census contains no complete non-English fallback-glyph or non-pointer focus
traversal matrix. Those are not guessed: runtime formatted strings and
fallback-font/inline-icon runs remain A25-owned, while non-pointer input is an
explicitly unsupported product variant rather than a fabricated traversal.

This is a complete renderer/data-boundary disposition, not a partial layout
law. File-derived anchors and root sizing remain usable through the frozen UI
laws; XUI07 supplies half-open clip intersection; XUI02 supplies authenticated
window ownership. A32's launch display/language carrier is now implemented;
the exact typed key must be present for admission.

## Mechanical gates

- Exact-key selection: viewport, `min_gui`, display mode, GUI-scale bits, language and input mode.
- No-nearest-resolution/scale and no-language-fallback negative gates.
- Every admitted key binds exact logical-layout and physical-pixel-oracle hashes.
- Post-resize transition must bind exact previous/current keys and a layout hash
  equal to the selected envelope's static logical-layout hash.
- Clips are non-empty half-open rectangles wholly inside the physical viewport.
- Font file, glyph-atlas and ordered-glyph-set hashes are mandatory; empty glyph
  runs reject.
- 2752x1152 and every other unlabelled resolution, aspect, UI scale, language,
  glyph set, resize state or input variant reject until their own exact bindings
  exist.
- Keyboard/controller variants reject as unsupported; shortcuts are not promoted as traversal.

Recorder modset tests are **27/27**; fold/repack shape gates are **2/2**;
replay source tests are **10/10**; world-timeline integration is **28 passed, 0
failed, 1 environment-only ignored**. Focused UI tests are **14/14**. The latest
complete library run is **120 passed, 0 failed, 1 ignored**; the promotion gate
requires at least the frozen 116-test baseline because other replay lanes add
independent library tests concurrently (the installed-file test is exercised by
production verification).
The contract verifier also checks frozen upstream laws, the A32 separation
contract, source tokens, and complete artifact hashes.

The deterministic replay-only freeze is bound to these exact artifacts:

- production source: `40b5de7011fdaec2ebab910329e1f97546aec9d9009b68e583789be188e4cc55`;
- focused tests: `3a2745c2173a0a03fd88b8ac6503c127c37b7158c5fdb412b313c8a7e232935b`;
- production contract: `73a33129b1bb1521a7c140c1d2687c65e40db85bb087196d62e8e612fcd00ed0`;
- verification, both final runs: `95f8e4c46438127f4a944fa643a25b1034a90b2d3ecd51360a2c18f4324e99fd`;
- evidence manifest: rebuilt after this report is frozen; its own hash is
  intentionally not self-embedded.

## Binary provenance

| Function address | Job ID | Artifact path | Artifact SHA-256 | Manifest hash | Disposition |
|---|---|---|---|---|---|
| `0x14225ffb0` | `20260717T100012Z-xui25-a32-launcher-settings-write-path-a703304a` | `C:/Users/Example/ghidra_agent_exports/20260717T100012Z-xui25-a32-launcher-settings-write-path-a703304a/evidence/func_14225ffb0.decompile.c` | `15805f42700a40d78e29086e7f71517c18ca009f0081f24d3f250aa8394ae1f1` | `d350dfe52a4d7718f821c332e2703f46f1be5c467f987e1246384264a4f998d2` | Writes exact fullscreen/windowed/borderless_fullscreen modes and WIDTHxHEIGHT resolutions. |
| `0x14225eab0` | `20260717T100020Z-xui25-a32-launcher-settings-read-path-406688ef` | `C:/Users/Example/ghidra_agent_exports/20260717T100020Z-xui25-a32-launcher-settings-read-path-406688ef/evidence/func_14225eab0.decompile.c` | `a6aa6ebffc343542aecbe8a1c625713ae93ce4c5d32bb9154d67449373aa8981` | `98cdcb855d49ac5c9f5be75e895c914e9273c635fad94bcffeb209b45a4a0066` | Reads modes 0/1/2, two-integer resolution and refreshRate. |
| `0x142215160` | `20260717T100026Z-xui25-a32-borderless-shortcut-semantics-2310ee0e` | `C:/Users/Example/ghidra_agent_exports/20260717T100026Z-xui25-a32-borderless-shortcut-semantics-2310ee0e/evidence/func_142215160.decompile.c` | `3229001f4fe202ec2b585d27d4291a4faafd6f1d1a8bd16eef8922be21be3bff` | `eecccd5c70866fa4195d4779edac8b15608de6ee6620045e0f590e0f19992bdf` | Command-line modes map exactly to 0/1/2 and bind a resolution pair. |
| `0x141378220` | `20260717T100026Z-xui25-a32-borderless-shortcut-semantics-2310ee0e` | `C:/Users/Example/ghidra_agent_exports/20260717T100026Z-xui25-a32-borderless-shortcut-semantics-2310ee0e/evidence/func_141378220.decompile.c` | `55487aa2ca348ee6c30a08f6b441ea73e24d2c4c6d2f4847c5e20a3c5f83680f` | `eecccd5c70866fa4195d4779edac8b15608de6ee6620045e0f590e0f19992bdf` | Shortcut names bind GUI actions/widgets, not traversal order. |
| `0x140082cb0` | `20260717T095737Z-xui25-a32-display-scale-settings-anchors-6e55e24d` | `C:/Users/Example/ghidra_agent_exports/20260717T095737Z-xui25-a32-display-scale-settings-anchors-6e55e24d/string-xrefs.txt` | `91f793f0fb2dc86b58d3744965fceb6ee4e6c316462d60e85f09334d6bc7a4fa` | `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` | One settings producer references gui_scale, min_gui, fullScreen, borderless and refreshRate. |

The large `0x140082cb0` export failed at jobs
`20260717T100000Z-xui25-a32-engine-display-settings-loader-3ef64cf4` and
`20260717T100959Z-xui25-a32-engine-display-settings-loader-488043c3`; the
successful string-xref artifact is retained and no unexported structure claim
is made. Machine-readable provenance is frozen in `ghidra_provenance.json`.

Incidental finding retained: `settings-layout.json` declares the three display
modes and ten launcher languages, while SDL supplies resolution choices
dynamically; no hard-coded supported-resolution list is justified. The
extractor ledger also currently contains duplicate A29 and A31 identifiers;
XUI25 therefore uses the next unused identifier, A32, and does not rewrite
other agents' rows.

**Implementation residuals: none. A32 launch-bound display/language identity is
integrated. Non-pointer traversal is explicitly unsupported, not data-blocked.
A25 remains the owner only for runtime-selected text and fallback-font or
inline-icon runs.**
