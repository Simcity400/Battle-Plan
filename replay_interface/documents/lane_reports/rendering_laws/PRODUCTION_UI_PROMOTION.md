# Production replay UI promotion

Status: **PRODUCT ROUTE FROZEN AND TESTED; NON-AUTHENTICATED PIXEL ROUTES OMITTED**

Pinned target: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

This report covers the normal `replay_app` product route. The extractor and replayer are independently buildable and share no product-tree Cargo dependency. Their runtime interface is the portable `.battleplan` only; debug captures, process memory, extractor staging, installed defaults, and frozen vanilla catalogs are not product inputs.

There is no `PARTIAL` product disposition here. A surface is rendered only when its exact replay-carried display key, semantic runtime root, complete effective-VFS dependency closure, and winning content identities are admitted. Otherwise the surface is omitted as a unit; a malformed claimed binding rejects playback UI composition.

## Actual product route

`replay_app` opens the portable content manifest, reconstructs only its recorded required layers, validates exact winners, and creates an admission-bound resolver. `ProductUiRuntime::from_authenticated_hud_roles` then admits one all-or-nothing HUD route and restricts `AssetResolver` to the four recorded `hud_role/v1/*` consumer prefixes. Dynamic values come only from the typed `WorldFrame`.

| Surface | Required runtime root | Required winning GUI identity |
|---|---|---|
| topbar | `topbar` | `interface/topbar.gui` |
| minimap | `mini_map_interface` | `interface/minimap.gui` |
| map modes | `MapmodesInterface_Ingame` | `interface/mapmodes_interface.gui` |
| theatre selector shell | `theatre_selection_window` | `interface/theatreselector.gui` |

For every root, the GUI consumer's suffix must equal the admitted `UsedContentKey.payload_sha256`; its winning path must equal the table above; and every asset read must be bound beneath one of the four exact consumer prefixes. Missing or incompatible roots return normal omission. Duplicate or malformed claimed GUI bindings are errors.

The product compositor deliberately does not read or draw controller-unbound fragments: menubar `zoom_in`/`zoom_out`, `theatre_selection_entry` rows, or `core.gui` `ToolTip`. The broader probe compositor remains evidence-only. The product route does not construct the full XUI01 catalog, XUI02 window manager, XUI26 frontend manager, or unrelated panel definitions.

XUI25 remains exact rather than a default: composition requires a replay-carried display variant matching the supported 1920x1080 physical / 1280x720 `min_gui` / scale `1.0` / borderless fullscreen / `l_english` / mouse-keyboard key. An absent key or any other viewport omits UI without resampling.

## Complete product disposition

| Group | Disposition | Exact result |
|---|---|---|
| XUI24 | ROUTED | Exact loose/archive/integrated-DLC/selected-DLC/mod VFS winners and replay content admission back every product read. |
| XUI25 | ROUTED | The recorded exact display/language/input variant gates composition; there is no inferred/default variant or resize resampling. |
| XUI27 | ROUTED EXTENSION | The four runtime-root identities and their complete effective-VFS dependency closures make the HUD route admissible. |
| XUI09, XUI10, XUI13 | PIXEL CANVAS OMITTED | Their data/canvas laws are promotion-ready, but recordings do not authenticate the research/doctrine, focus, or designer surface activation plus semantic GUI-root binding. XUI13's canonical A27 catalog and d78 order are admitted; only its product surface authority is absent. |
| XUI01-XUI08 except routed packets, XUI11-XUI12, XUI14-XUI23, XUI26 | AUTHORITY/DATA OMITTED | Twenty-one packets remain absent from the product image. Their exact named carrier requirements are frozen in the adjacent machine manifest; no chrome-only panel, sample row, placeholder, guessed layout, or broad installed-catalog parse is shipped. |

The 21 omitted historical packets are `XUI01`, `XUI02`, `XUI03`, `XUI04`, `XUI05`, `XUI06`, `XUI07`, `XUI08`, `XUI11`, `XUI12`, `XUI14`, `XUI15`, `XUI16`, `XUI17`, `XUI18`, `XUI19`, `XUI20`, `XUI21`, `XUI22`, `XUI23`, and `XUI26`. The product test mechanically proves that these 21, the three canvas omissions, and routed `XUI24`/`XUI25` form an exact disjoint partition of XUI01-XUI26; XUI27 is tracked separately as the routed extension.

XUI12's typed production and stockpile successor data does not authorize pixels: the portable frame still lacks authenticated d71 `countryproductionlineview/item_grid` materialized slot rectangles, final clip, and scroll state. XUI05 lacks evaluated tooltip instances. The remaining exact carrier requirements are recorded per packet in `manifest.json` rather than expanded into speculative renderer behavior.

M02 combat-indicator carrier and executor laws are complete but its pixels remain omitted. The exact draw belongs at UI-pass ordinal 859/1167; the current flattened HUD has no authenticated mapping from every non-M02 ordinal to a product command/surface segment. Prepending, appending, or guessing an interleave is forbidden.

## Distribution and content boundary

Neutral HUD wire roles, exact launcher-equivalent VFS resolution, GUI/GFX/Clausewitz parsing, and HUD closure interpretation now live under `portable_replay/native`. Recorder-owned physical launch mounts and finalization live under `extractor/native/recorder_host`. `replay_app` has zero extractor/live-lane normal dependencies, and `recorder_host` has zero `replay_interface`, `game_ui`, `replay_core`, `map_viewer`, or `replay_app` normal dependencies.

Standalone extractor builds with `replay_interface` absent and standalone replayer builds with `extractor` absent passed twice each. HUD capture-to-relocated-manifest finalization passed twice. A27 relocation and neutral decoding preserved all 6,407 records. All boundary temporary roots were removed.

The installed a729 HUD closure closes twice with byte-identical counts and no missing roles: topbar 658 bindings, minimap 77, map modes 475, and theatre selector 7. The normal recorder finalizer proves an exact mod GUI winner, same-machine repeat, relocated install/profile roots, no sidecar, and feature-local omission of an incompatible theatre root.

An independent installed-product audit exposed and repaired a test-fixture defect: the synthetic recording admitted `gfx/interface` and fonts but omitted `gfx/minimap` and `gfx/texticons`. The restricted product resolver correctly rejected eight minimap textures and three XP icons. The fixture now records those exact dependency directories; fail-closed errors retain full missing/undecodable identities. Product admission was never weakened.

Integrated a729 DLC descriptors retain legacy `path="dlc/..."` values although their content is physically beside each `.dlc` descriptor. The VFS resolver now mounts integrated DLC at the descriptor parent while preserving ordinary archive/selected-DLC/mod behavior; the focused regression is green.

## Ghidra provenance

No new Ghidra job was issued by this production integration. It consumes these frozen XUI27 identity records; every row records the required address, job, artifact path, and job-manifest hash.

| Function / role | Job ID | Artifact path | Manifest SHA-256 |
|---|---|---|---|
| `0x14188BA30` topbar runtime root | `20260718T050440Z-xui27-topbar-controller-lookups-v1-c9501167` | `C:\Users\Example\ghidra_agent_exports\20260718T050440Z-xui27-topbar-controller-lookups-v1-c9501167\evidence\func_14188ba30.decompile.c` | `953d57d0f16f3ac9afedfac409bde89fb69d1597454f18ebf338861b477f35c9` |
| `0x1417B9A80` minimap runtime root | `20260718T050427Z-xui27-hud-controller-window-lookups-v1-f899f2b8` | `C:\Users\Example\ghidra_agent_exports\20260718T050427Z-xui27-hud-controller-window-lookups-v1-f899f2b8\evidence\func_1417b9a80.decompile.c` | `4465eb2825c4c3e78b18c8dcea64d0607d2e8e4e1ab7766f4551c7542045ad0b` |
| `0x141563C80` map-mode runtime root | `20260718T052032Z-xui27-dynamic-mapmode-button-name-builde-ffb88c12` | `C:\Users\Example\ghidra_agent_exports\20260718T052032Z-xui27-dynamic-mapmode-button-name-builde-ffb88c12\evidence\func_141563c80.decompile.c` | `6e34338355fb71ee85bab2e3fad6f5e32672c71d5d5a8f6da854724867041088` |
| `0x141878E40` theatre-selector runtime root | `20260718T050427Z-xui27-hud-controller-window-lookups-v1-f899f2b8` | `C:\Users\Example\ghidra_agent_exports\20260718T050427Z-xui27-hud-controller-window-lookups-v1-f899f2b8\evidence\func_141878e40.decompile.c` | `4465eb2825c4c3e78b18c8dcea64d0607d2e8e4e1ab7766f4551c7542045ad0b` |
| `0x1422A54E0` runtime GUI root-name getter consumer | `20260718T055840Z-hud-live-gui-object-identity-and-child-l-f73fdcf4` | `C:\Users\Example\ghidra_agent_exports\20260718T055840Z-hud-live-gui-object-identity-and-child-l-f73fdcf4\evidence\func_1422a54e0.instructions.txt` | `4f474dec5345a2e732b2b126c12005dc5cb8c4b9c7e88f61d4d29199b5edea1e` |

The complete 19-record XUI27 identity dossier is `extractor/documents/dossiers/XUI27_HUD_LOOKUP_IDENTITIES.md`; dossier SHA-256 `18d3ce7af46e983fb021fdf30f13b87eaabf694781ecdbf9b81afc42ce4be6f0`, manifest SHA-256 `3ab0d0da2ec19e50a6010242449d05f2ff9fb0e9a7394bc59b5ec28a35c1905c`.

XUI13 required no new Ghidra operation: function address `N/A`, job ID `N/A`. Current product artifacts are `portable_replay/native/snapshot/src/design_catalog.rs`, `replay_interface/native/replay_core/src/design_catalog.rs`, `replay_interface/native/game_ui/src/design_catalog.rs`, and `replay_interface/native/game_ui/tests/xui13_design_catalog.rs`; they are bound by this packet's machine manifest, SHA-256 `86329980977cffad8dcd2e1229e5d40ad343216566e361f3c00fa99e064a4bc5`.

The preserved upstream `XUI13/evidence_manifest.json` is non-authoritative for production after the neutral schema move: it still binds the old `replay_core::design_catalog` hash. It was not rewritten here. Production promotion instead hashes the current neutral schema, compatibility shim, admission code, and focused test directly.

## Final verification

- Targeted installed relocation: 1/1 passed, 665.41 s.
- Full locked `game_ui` passed twice. Each run had 380 passed and 2 ignored across 42 nonempty binaries; the library tier was 125 passed/2 ignored. `production_renderer` passed 11/11 in 504.78 s and 478.36 s.
- Full locked `replay_app` passed twice. Each run passed 223/223: 219 unit, 2 soak, and 2 F2 budget/trend gates. F2 completed in 158.51 s and 151.42 s.
- The nested locked/offline domain-53 regression passed after the neutral-schema lockfile refreeze.
- Normal Cargo trees are clean in both product directions; standalone source-absence builds passed twice.
- The production fallback guard passed in both full UI runs: no extractor staging, runtime user-dir discovery, frozen definition catalog, default game root, unrestricted `AssetResolver::new`, or unrelated installed UI scan is reachable from the product callers.
- XUI13's current neutral A27 schema, replay compatibility shim, product admission code, and focused test are direct machine-manifest artifacts. The stale pre-neutral-move packet manifest is explicitly excluded from production authority.

Machine-readable contract: `replay_interface/re_evidence/rendering_laws/PRODUCTION_UI_PROMOTION/manifest.json`, SHA-256 `86329980977cffad8dcd2e1229e5d40ad343216566e361f3c00fa99e064a4bc5`.
