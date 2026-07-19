# XUI24 — mod-added UI discovery

**Status: PROMOTION READY — PRODUCTION DISCOVERY FAIL CLOSED**

XUI24 consumes the staged UI14 engine-ordinal receipt and UI15 materialized-DLC namespace directly. Loose mods are ordered highest priority first by the recorded unique `engine_mount_ordinal`; staging or launcher-array order is irrelevant. Materialized DLC follows by its required unique UI15 ordinal, then vanilla. Every staged descriptor identity and SHA-256 must match, `_ui15_dlc/` is a guarded namespace, and missing content roots, missing/duplicate ordinals, unbound receipt rows, or namespace misuse reject admission.

The effective VFS uses the first case-folded relative-path winner. The frozen UI14/UI15 `replace_path` law is exact and non-recursive: it removes lower files whose immediate parent equals the declared directory, while the declaring layer may re-add files. A declaration for `interface` therefore removes lower `interface/base.gui` but not `interface/nested/base.gui`.

Discovery recursively enumerates effective `interface/*.gui`, `interface/*.gfx`, `localisation/english/*.yml`, and `common/scripted_localisation/*.txt`. Every effective GUI enters the shared XUI01 construct classifier, so mod-added panels and overrides are discovered beyond the vanilla census; an unknown widget construct rejects the entire discovery result. Sprite and English-localisation registration use the same effective resolver.

The installed a729 game-file census covers 42 scripted-localisation files, 1,529 `defined_text` blocks, 30,368 `text` blocks, and 30,467 `localization_key` occurrences. Its exact direct grammar is `trigger`, scalar `localization_key`, and 22 observed `random_list` blocks. Weighted rows are numeric objects containing exactly one scalar `localization_key`; the only other observed member is scalar `seed`. XUI24 inventories all candidates, accepts this complete installed grammar, and rejects parse recoveries, anonymous fields, unknown structures, malformed weights, and definitions without a candidate. Replay never evaluates triggers or random selection: the recorded engine-selected runtime value remains the only production value.

Focused tests prove mod-added panel discovery, overrides, sprite/localisation/scripted-localisation registration, exact-parent `replace_path`, two-mod precedence independent of staging order, UI15 DLC namespace registration and mod-over-DLC precedence, missing/duplicate DLC ordinals, namespace misuse, installed `random_list` admission, and fail-closed unknown/empty widget or script forms. The full `game_ui` package is also a required verifier gate.

## Binary and capture provenance

| Function address | Job ID | Artifact path | Artifact SHA-256 | Manifest hash | Disposition |
|---|---|---|---|---|---|
| `0x1420610f0` | `20260716T090431Z-l02-cgraphicalmap-vtable-methods-v1-fb61dea1` | `C:/Users/Example/ghidra_agent_exports/20260716T090431Z-l02-cgraphicalmap-vtable-methods-v1-fb61dea1/evidence/func_1420610f0.decompile.c` | `656eb082a31ec705f1b76ddc0a8ab0a82c8e6fb7e1b17ed739bb980b571c56c9` | `54d207d339ad009b25dbebe32645ce225ab97857451a708150a2b1c84cd44841` | Single-MOD descriptor parser/inserter; capture binds the caller return RVA and descriptor ordinals. |
| `0x1420665a0` | `20260716T090431Z-l02-cgraphicalmap-vtable-methods-v1-fb61dea1` | `C:/Users/Example/ghidra_agent_exports/20260716T090431Z-l02-cgraphicalmap-vtable-methods-v1-fb61dea1/evidence/func_1420665a0.decompile.c` | `d0cf422e52862671288e3c33fc905c6d3eba9035c1a931a5453d514786e50c15` | `54d207d339ad009b25dbebe32645ce225ab97857451a708150a2b1c84cd44841` | Preserves supplied DLC descriptor-vector order; not used to infer MOD order. |

The two fully bound UI14 captures reverse launcher `enabled_mods` order while retaining the same engine descriptor/mount result and winner SHA-256 `bbeebd879e1dff6918546dc0c179fdde505f2a21591c9a9c96e36b054ec5af83`:

- `a_then_z`: `C:/tmp/hoi4_rendering_law_capture/20260717T000006Z-ui14-a-then-z`; capture manifest `9590ef7ce452ab259eba885e9a7487bac3f8996b5933ae6645ce78320419e3e5`; active config `4c5a36528a399532ae37f45cf14a78476980b738a0bef15dc3def779cc1dadfe`; ready `1248e4ce61383dd42bb8057d0165e8eeb5b33a97343cde12065dc020a0affbdf`; trace `38fbfc16eba3a557bb465f64b113e6bd70e098af401e8f03be0579f318ed353a`.
- `z_then_a_repro`: `C:/tmp/hoi4_rendering_law_capture/20260717T000007Z-ui14-z-then-a-repro`; capture manifest `e106185f58fcb3a42216fb14a76967ba14b3233c516179b1bd72ce0198c62bd9`; active config `d336346e9f9920c55432e8ec71aba86bb5922534826c0fce15e83dd69af15fa4`; ready `b56ce0124c04d05a4867dcbd244cc493530119734806af2de739081904499309`; trace `06e471fcb85ba61c074399708c05876e0726a18bebee760d3adf9e3e81d5dc21`.

UI15 contributes the materialization boundary: installed `dlc003.zip` is 2,461,198 bytes, SHA-256 `3619c316ed8a10a81be6bb6ddec946e0be10889b4a84a72a2fc583186b67fb57`, with 27 deflated members materialized byte-for-byte. Archive mods remain rejected upstream because no installed a729 archive-mod case attests them; XUI24 admits only directory mods and already materialized UI15 DLC roots.

The former shared XUI02 integration residual is resolved. Its activation-source test now checks non-empty unique source identities, complete definition accounting, and direct-plus-inherited expansion invariants instead of a stale fixed source count. XUI24 therefore runs the complete `game_ui` package without exclusions. Two independent verifier passes must produce byte-identical verification snapshots before promotion.

**Production admission: true. Implementation residuals: none. Data residuals: none. Human input required: none.**
