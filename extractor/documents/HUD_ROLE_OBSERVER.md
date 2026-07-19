# Authenticated a729 HUD semantic-role observer

Status: **PRODUCTION CONTENT ROUTE / FOUR INSTALLED ALWAYS-ON SURFACES CLOSED**

This is extractor documentation. It is separate from the rendering-law packet
reports and does not amend `PROGRAM_STATUS.md` or XUI27's frozen dossier.

## Result

The extractor has a signature-gated observer for the four proven a729 HUD
setup producers: topbar, minimap, map modes, and theatre selector, plus the
existing A27 designer refresh bridge for country-equipment, tank, and plane
designer families. Each hook
runs the original setup first, reads only the proven runtime-root field, and
queues one bounded, process-local root per surface. A core-HUD drain succeeds
only after all four roots are present and no read/callback failure has occurred.

Pointers cannot cross the replay boundary. The proven GUI definition getter
(`runtime_root+0x15D0`, virtual slot `+0x48`) supplies the exact authored root
name. Compact domain 79 carries only canonical `{surface, runtime_root_name}`
rows. The in-process effective-VFS emitter consumes that pointer-free identity
and resolves every admitted GUI root, sprite, texture, bitmap-font
descriptor/page, localisation source, define, or scripted consumer to the winning
`UsedContentKey` from the recording's live effective VFS. A foreign-VFS key is
rejected. An incomplete surface is omitted as a unit.

`HudRoleSnapshot` uses consumer names under `hud_role/v1/<surface>/...` and is
accepted only by the explicit recorder adapter. `HudRoleSnapshot::validate`
requires a complete, disjoint activated/observed/omitted partition, and the
recorder revalidates it at finalization. The normal recorder finalizer reads the
latest clean d79 frame, constructs the resolver from the exact recorded layer
roster and launch-corroborated install/profile roots, emits the closure, requires
every emitted key to equal the finalized source-inventory winner, and merges the
observations into the portable content manifest. Install/profile paths are never
serialized.

The installed effective-VFS census closes all four always-on surfaces twice
with identical results: topbar 658 bindings, minimap 77, map modes 475, and
theatre selector 7, with no missing roles. The emitter follows the production
UI loader's exact `.tga`/`.dds` alternate-extension rule and bitmap-font page
fallback order. These are physical-resolution laws inside the selected VFS,
not vanilla asset defaults. A synthetic designer activation also closes from
its exact runtime name without inventing a dependency on the four core roots.

No vanilla path, asset, layout, sprite, font, flag, catalog, mod roster, or DLC
selection is embedded by this lane. Exact HUD pixels are not claimed by this
content-admission result.

## Hook authority

Pinned program: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, executable
SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.
Ghidra was ready with 20 shards. Existing jobs were listed and reused; no
address was re-exported.

| surface | function / RVA | runtime-root field | job ID | artifact path | artifact SHA-256 | manifest SHA-256 |
| --- | --- | --- | --- | --- | --- | --- |
| topbar | `0x14188BA30` / `0x0188BA30` | `this+0x78` | `20260718T050440Z-xui27-topbar-controller-lookups-v1-c9501167` | `C:\Users\Example\ghidra_agent_exports\20260718T050440Z-xui27-topbar-controller-lookups-v1-c9501167\evidence\func_14188ba30.decompile.c` | `543b6ef4b167e05e7b1f1f0937019943740fd0b1f6fe3dde1b8c5f9ff26f2243` | `953d57d0f16f3ac9afedfac409bde89fb69d1597454f18ebf338861b477f35c9` |
| minimap | `0x1417B9A80` / `0x017B9A80` | `this+0x1490` | `20260718T050427Z-xui27-hud-controller-window-lookups-v1-f899f2b8` | `C:\Users\Example\ghidra_agent_exports\20260718T050427Z-xui27-hud-controller-window-lookups-v1-f899f2b8\evidence\func_1417b9a80.decompile.c` | `8f16027647710d1c4c1bf09eddf7fcf84237c3686441cd5fac1561f3912cdf80` | `4465eb2825c4c3e78b18c8dcea64d0607d2e8e4e1ab7766f4551c7542045ad0b` |
| map modes | `0x141563C80` / `0x01563C80` | `this+0x2ED8` | `20260718T052032Z-xui27-dynamic-mapmode-button-name-builde-ffb88c12` | `C:\Users\Example\ghidra_agent_exports\20260718T052032Z-xui27-dynamic-mapmode-button-name-builde-ffb88c12\evidence\func_141563c80.decompile.c` | `3cdb630c06280a2bf3a9775fe7eaf7b2b9b99e47eafcc68e6b2035cef64b71cf` | `6e34338355fb71ee85bab2e3fad6f5e32672c71d5d5a8f6da854724867041088` |
| theatre selector | `0x141878E40` / `0x01878E40` | `this+0x58` | `20260718T050427Z-xui27-hud-controller-window-lookups-v1-f899f2b8` | `C:\Users\Example\ghidra_agent_exports\20260718T050427Z-xui27-hud-controller-window-lookups-v1-f899f2b8\evidence\func_141878e40.decompile.c` | `fa8de0e2644ebc72cca3695835cc98395b546a49ab50c14d0a1786eddc6a1bee` | `4465eb2825c4c3e78b18c8dcea64d0607d2e8e4e1ab7766f4551c7542045ad0b` |
| runtime GUI root-name getter consumer | `0x1422A54E0` | `root+0x15D0 -> vtable+0x48 -> MSVC string` | `20260718T055840Z-hud-live-gui-object-identity-and-child-l-f73fdcf4` | `evidence/func_1422a54e0.instructions.txt` | `c759c64cfbc1ddc1dcd247de300588a8d9aac1b558a04dd4546e33b25f50e984` | `4f474dec5345a2e732b2b126c12005dc5cb8c4b9c7e88f61d4d29199b5edea1e` |
| designer refresh / family bridge | `0x1417797A0` / `0x017797A0` | `view+0xF3A0` | frozen A27 job | frozen A27 artifact | `f5ae...c054` | `e62d...ab65` |

Frozen identity authority:

- dossier SHA-256: `18d3ce7af46e983fb021fdf30f13b87eaabf694781ecdbf9b81afc42ce4be6f0`
- dossier manifest SHA-256: `3ab0d0da2ec19e50a6010242449d05f2ff9fb0e9a7394bc59b5ec28a35c1905c`

## Production boundary and remaining data needs

The observer, authenticated compact carrier, effective-VFS closure emitter,
exact-winner merge, and normal recorder finalizer route are production-ready for
content admission. Missing or incompatible roots remain deliberately omitted;
they receive no guessed fallback. The synthetic normal-finalizer test proves
four-surface admission, an exact mod GUI winner, same-machine repeatability,
relocated-install/profile repeatability, and three-surface admission when the
theatre root is incompatible.

There is no remaining data need for this lane's identity or content-admission
scope. Pixel composition and surface-specific render laws remain separate
renderer responsibilities; this receipt makes no pixel-fidelity claim.

## Mechanical verification

- `hud_role_observer_live_lane`: 11 passed, 0 failed.
- game UI HUD closure tests: 3 passed, 0 failed, 1 installed test ignored.
- installed effective-VFS gate: 1 passed, 0 failed, run twice; identical 658 / 77 / 475 / 7 binding counts.
- normal recorder finalizer E2E: 1 passed, 0 failed; 607 filtered out.
- finalizer E2E covers an exact mod winner, repeat, relocation, no sidecar, and incompatible-root omission.
- `git diff --check` for owned files: passed (line-ending notices only).
- dedicated lane target and lane-owned temporary fixture residue cleaned before handoff.

The machine-readable receipt is
`extractor/re_evidence/HUD_ROLE_OBSERVER/verification.json`.
