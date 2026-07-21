# XUI25 — display/language/input variants

**Status: STATIC PRODUCTION COMPLETE; LIVE POINTER WITNESS REQUIRED**

XUI25 implements the launch-bound display/language carrier and a fail-closed
product key check. `recorder_host` reads the corroborated user directory's `settings.txt`
twice under the existing metadata/process-start stability bracket. One
byte-identical read supplies language, physical viewport, `min_gui`, exact GUI
scale bits, refresh rates, fullscreen/borderless mode, and a settings-file
SHA-256. Fold/repack, replay ingest, and the typed world view validate and carry
the exact object into `game_ui`. That portion is integrated.

The Wave5 production callsite gap is closed. The injected collector samples
the foreground the game client in the normal capture loop and joins an earlier QPC
sample to a later selected-Present sample only when the same client extent's
in-client pointer positions differ. The v2 materializer binds executable, process id,
process-start nonce, hook epoch, both Present QPC values, current selection
receipt, client extent and both positions to the frozen authority manifest.
The receipt is bound into `InProcessRenderAdmission` before d77 authority and
UI-pass assembly. Stationary, unfocused, keyboard-only and controller sessions
remain unsupported and produce no carrier.

The exact variant key is physical viewport width/height, `min_gui`, display
mode, GUI-scale IEEE-754 f32 bits, recorded localisation language, and input
mode. The library-only `VariantCatalog` envelope additionally binds a source record and evidence-manifest
hash, producer-resolved logical-layout SHA-256, complete physical-pixel-oracle
SHA-256, exact font file/atlas/ordered-glyph-set hashes, final half-open
physical-pixel clips, and—when applicable—the exact post-resize transition and
keyboard/controller focus owner/traversal. Unsupported keys reject before
composition. Both reconstructed-HUD and selected-Present product adapters now
call `ExactKeyVariantContract`, a named mechanically narrow selector that admits
only the single independently proven key. It deliberately does not claim the richer layout/pixel/font/clip/resize
`VariantCatalog` envelope. The only intended current family is pointer
dispatch (`MouseKeyboard`); keyboard-only and controller variants reject
explicitly and remain unsupported.

## Evidence disposition

The complete admitted matrix currently contains exactly the labelled 1920x1080,
GUI-scale 1.0, English, pointer-family UI control. A second 2752x1152 viewport
is established by camera and research evidence, but not as a complete all-UI
pixel oracle. It is therefore not admitted. The retained metadata
census contains no complete non-English fallback-glyph or non-pointer focus
traversal matrix. Those are not guessed: runtime formatted strings and
fallback-font/inline-icon runs remain A25-owned, while non-pointer input is an
explicitly unsupported product variant rather than a fabricated traversal.

This is not yet a complete production renderer/data-boundary disposition.
File-derived anchors and root sizing remain usable through the frozen UI
laws; XUI07 supplies half-open clip intersection; XUI02 supplies authenticated
window ownership. A32's launch display/language carrier is now implemented;
the exact typed key and an authenticated same-launch pointer-family receipt
must both be present for admission.

## Mechanical gates

- Exact-key selection: viewport, `min_gui`, display mode, GUI-scale bits, language and input mode.
- No-nearest-resolution/scale and no-language-fallback negative gates.
- The library catalog requires logical-layout and physical-pixel-oracle hashes,
  while the product HUD is explicitly limited to its independent single-key
  `ExactKeyVariantContract` and makes no catalog-envelope claim.
- Post-resize transition must bind exact previous/current keys and a layout hash
  equal to the selected envelope's static logical-layout hash.
- Clips are non-empty half-open rectangles wholly inside the physical viewport.
- Font file, glyph-atlas and ordered-glyph-set hashes are mandatory; empty glyph
  runs reject.
- 2752x1152 and every other unlabelled resolution, aspect, UI scale, language,
  glyph set, resize state or input variant reject until their own exact bindings
  exist.
- Keyboard/controller variants reject as unsupported; shortcuts are not promoted as traversal.

The frozen receipt historically recorded recorder modset tests at **27/27**; fold/repack shape gates at **2/2**;
replay source tests are **10/10**; world-timeline integration is **28 passed, 0
failed, 1 environment-only ignored**. Focused UI tests are **14/14**. The latest
complete library run is **120 passed, 0 failed, 1 ignored**; the promotion gate
requires at least the frozen 116-test baseline because other replay lanes add
independent library tests concurrently (the installed-file test is exercised by
production verification).
Those results prove the isolated carrier and catalog behavior from that frozen
slice. Root-owned central Final/All validation has passed and is hash-bound by
the current contract; one real pointer observation remains required before the
production manifest can become promotion authority.

The current static closure is bound to these exact canonical-LF identities:

- variant-admission source: `56d092d0e1f2dc8a6161b589b58674845c68441eef1cdef984fa5c008c5e6f6e`;
- focused tests: `c7eeffb6a386eafd079eb4af79613a423445c448b6a4d7849ec2f96c6d052958`;
- production contract: `6f7503a7471ce292f9453b3212e3de3c96736d52d4d6fe3c8121a2b3356b75e0`;
- central Final/All attestation: `fde868c505f42421370aa08262806b67ce6d1c39842e9efdb5876836e29ac727`;
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

**Implementation residual:** acquire one real foreground in-client pointer
movement from a capture-loop sample to a later selected-Present boundary, and
refreeze the manifest from that receipt. A32 launch-bound
display/language identity remains integrated. Non-pointer traversal is
explicitly unsupported, not inferred.
