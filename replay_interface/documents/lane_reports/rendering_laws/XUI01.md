# XUI01 — complete effective-VFS GUI definition admission

Status: **PROMOTION_READY_DEFINITION_ADMISSION**

Pinned authority: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

XUI01 now owns both the exhaustive retained-playset census and the production definition-loader gate. It does not claim runtime visibility, population, interaction, focus, localization, or pixels; those remain with their exact downstream XUI owners.

## Frozen production result

| Surface | Exact result |
|---|---:|
| active `.gui` files | 454 |
| definition-admitted files | 454 |
| rejected definition files | 0 |
| base `interface/**/*.gui` files | 453 |
| enabled integrated-DLC files | 1 (`interface/mtg_music.gui`, DLC023) |
| window definitions | 9,155 |
| definition-admitted windows | 9,155 |
| rejected window definitions | 0 |
| `containerWindowType` definitions | 9,072 |
| `windowType` definitions | 83 |
| parsed elements | 27,135 |
| parse failures | 0 |
| definition-only windows with downstream runtime owners | 190 across 77 files |

The production entrypoint is `game_ui::definition_admission::load_frozen_definition_catalog`. It requires the exact 454-path namespace, rejects missing/unexpected/case-colliding paths, verifies every byte count and SHA-256, parses every file, verifies every per-file window count and both aggregate counts, then admits only explicit construct schemas. `replay_app::gui_definition_admission::load_product_gui_definitions` enumerates the live mod-aware VFS, mounts the selected integrated-DLC definition, and runs this gate during product startup whenever `--game` is present.

## Construct schemas and fail-closed boundary

All 49 shipped case-preserved construct families have explicit definition schemas. The schemas cover windows, containers, sprites/backgrounds, instant text, buttons, collections, scroll geometry, layout properties, browser geometry, checkbox geometry, dropdown geometry, edit-box geometry, overlap-collection geometry, and text-box geometry. Generic parser retention is no longer confused with unsupported definition semantics.

Browser, checkbox, dropdown, edit-box, overlap-collection, and stateful text-box definitions are file-derived and therefore admitted. Their behavior remains fail-closed under exact downstream owners:

| Definition family | Runtime owner |
|---|---|
| browser | XUI26 |
| checkbox | XUI04 |
| dropdown | XUI02, XUI04 |
| edit box | XUI06, XUI25 |
| overlapping collection | XUI03 |
| stateful text box | XUI06 |

The corpus contains 190 windows with at least one such definition-only dependency across 77 files. The full per-window identities and owner lists are frozen in `window_census.json`. Unknown future construct families still reject their narrow owning window; they are never coerced into an empty widget.

## Corrected provisional six-file finding

The first implementation pass reported 448/454 files and 8,965/9,155 windows. The 190 window rejections were intentionally eliminated by adding the explicit definition schemas above. The apparent six rejected files were not the complete owning-file inventory: the 190 windows span 77 files, while only five one-window files had no other admitted window and one valid file contained zero windows.

The exact corrected six rows are `interface/browser.gui` (XUI26), `interface/ingamelobby.gui` (XUI06/XUI25), `interface/messagesettings.gui` (XUI04), `interface/playeraiprefs.gui` (XUI04), `interface/widgets/valueinputwidget.gui` (XUI06/XUI25), and the valid zero-window `interface/destroyedshipswindow.gui`. They remain in `promotion_contract.json` as a retained implementation finding, not as residual blockers.

## Historical census correction

The earlier 133-file/2,009-container matrix remains refuted as a complete census. The pinned namespace has 131 shallow files plus 323 nested files, dominated by 268 `interface/equipmentdesigner/**` files. The complete recursive set contains 9,072 containers, so the old shallow method omitted 7,074 active container definitions.

## Admission law

1. Resolve GUI files through the ordered effective VFS and selected integrated-DLC layer.
2. Require exact path, byte-count, SHA-256, per-file window-count, and aggregate-count agreement before definition admission.
3. Admit only explicit definition schemas; unknown future constructs reject the owning window.
4. Never interpret definition admission as runtime state, interaction, ordering, text output, or pixel admission.
5. Carry exact downstream XUI owner IDs for definition-only runtime semantics.
6. Treat inactive optional DLC as valid absence and selected missing/drifted content as fatal namespace drift.

The focused corpus gate passes 3/3 and proves 454/454 files plus 9,155/9,155 windows. The full `game_ui` suite and `replay_app` compilation are also part of the freeze sequence.

## Binary provenance

XUI01 makes no new executable claim and started no Ghidra operation. Function address, job ID, artifact path, and Ghidra manifest hash are therefore **not applicable**. The implementation and offline corpus evidence are pinned by `evidence_manifest.json`.

## Freeze result

The effective namespace and production definition-loader layer are ready to promote. Runtime manager, collection, widget-state, text, graphics, specialized-data, and final-composition admission remain with their downstream XUI packets. `PROGRAM_STATUS.md` was not edited.
