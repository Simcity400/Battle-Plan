# XUI18 — power balance

**Status: PROVEN — REPLAY PROMOTION READY, EXACT A37 PRODUCER INPUT REQUIRED**

XUI18 now has a production-owned, fail-closed path from the measured power-
balance view and effective game-definition catalogs to the shipped panel
presentation. Position, sides, ranges, modifiers, decisions, thresholds,
animation state, DLC gating, ordering, and visible composition all have an
exact disposition. No renderer or integration law remains partial.

## Exact static and presentation laws

- The effective `common/bop` catalog contains **12 balances, 34 sides, and 120
  ranges**. Raw decimal threshold strings, authored side/range order, static
  modifiers, activation/deactivation blocks, source file, initial value, side
  identity, icon token, and decision-category identity are preserved.
- The effective English catalog resolves **152/166** balance/side/range IDs.
  The 14 unresolved IDs are exactly the eight IRE-family IDs and six
  non-production `_test` IDs; runtime-visible strings therefore use the
  existing A25 text producer instead of guessed localisation.
- The current effective VFS contains **24 CZE, 23 FIN, and 46 ITA** BOP
  decisions. BRA, DEN, ETH, IRE, PRC, SWE, and SWI BOP categories are
  measured-empty in this playset; the replay never invents definitions absent
  from the effective catalog.
- The effective `common/bop` source authors **46**
  `set_power_balance_gfx` invocations, and all **46** are now materialized in
  `catalogs.json`. The former count of 39 was parser loss: repeated sibling
  keys were collapsed in two CZE and five FIN sites. Those seven commands now
  survive as source-ordered arrays while single commands retain the existing
  object shape. FIN also contains the scripted
  `GFX_bop_FIN_[GetLeaderBopIcon]_*` token, so authenticated runtime effective
  icons still override authored side icons at composition.
- There are **68 shared threshold boundary points**. At equality the replay
  admits only the producer-reported active range; it never chooses a range by
  first/last match or floating-point nudging.
- `progress_minmax.shader` paints the interval between the fixed midpoint and
  `CurrentState=(value+1)/2`: the front texture is selected exactly when
  `(u<0.5 && u>=CurrentState) || (u>=0.5 && u<=CurrentState)`. The four
  stable/moving left/right bar sprites are selected from authenticated runtime
  state, not inferred from `trending_side` alone.
- The vanilla window is entitlement-independent. It is visible only when the
  current country occurs in a measured `world.power_balances[].countries` row;
  no active row means hidden, not unavailable.

The shipped window is 550x600 at shown position `[45,0]`, with hidden/show
position `[545,0]`. Show is a 300-authored-unit decelerated transition and hide
is the reverse accelerated transition with deferred hiding. The bar is
360x16 at `[95,125]`; marker offset is `[-12,-6]`; range-splitter offset is
`[-6,-4]`. Decisions begin at `[0,200]` in one 502x40 column, with vertical
step 41, wheel factor 40, and smooth scrolling. XUI03 supplies the frozen
position-animation law and A28 supplies exact dynamic collection
materialisation/scroll state.

## Production implementation

- `game_ui::power_balance` parses the frozen catalog strictly and joins it to
  typed `PowerBalanceView`, `DecisionSet`, and one authenticated A37 supplement
  per active balance. It verifies country/balance identity, unique contiguous
  display ordinals, exact sides/range, finite normalized value, complete
  modifier and decision lists, decision membership/order, effective icons,
  bar state, indicator frame, trend text, and resolved visible text.
- `game_ui::compose` supports runtime sprite-token overrides and the exact
  midpoint-interval `progress_minmax` shader path while leaving ordinary
  progress bars unchanged.
- Portable GUI role schema v2 keeps the exact authored hierarchy, parent,
  widget type, and same-type sibling ordinal. Identifiers are case-sensitive:
  a729's direct siblings `powerbalanceview/background` (panel chrome) and
  `powerbalanceview/Background` (decision chrome) are distinct roles. Exact
  duplicate selectors still reject, while a valid effective mod winner may
  supply different sprite and texture identities without a vanilla allowlist.
- Multi-balance ordering is producer-authenticated. Threshold equality,
  effective icon selection, decision visibility/order/cost/timer/target,
  modifier display order, stable/moving state, trend cadence/magnitude, and
  runtime text all reject when absent or incomplete.
- Focused tests cover the 12/34/120 frozen catalog, exact join and geometry,
  modifier/decision order, base-game visibility, every producer rejection,
  threshold equality, the min/max shader, and panel animation.
- Parser gates cover a hermetic repeated-sibling block and the complete real
  effective catalog, whose authored/materialized GFX invocation count must be
  exactly 46 with the CZE-two/FIN-five preservation distribution.
- The repository move to `` exposed a stale layout assumption:
  the legacy derived user-data root becomes `C:/`, yielding an 11-balance
  vanilla-only rebuild. The catalog CLI now accepts explicit `--vanilla`,
  `--mod`, `--userdata`, and `--out` arguments; the frozen artifact was rebuilt
  through that path with the effective `test.mod` layer and retains all 12
  balances.

## Separated extractor input

Only the irreducible runtime leaves in
`extractor/documents/dossiers/A37_XUI18_POWER_BALANCE_INPUTS.json` remain
upstream: source identity, country/balance/display ordinal, active side/range,
  effective icons, exact politics-launcher visible/text/icon/glow tuple,
  stable/moving bar state, range-indicator frame, final
  range-indicator position, trend, complete active modifiers, complete ordered
  decision subset, and their
resolved-text references. Existing `world.power_balances` and static catalogs
remain authoritative for their current fields; A25 owns resolved text, A28
owns collection materialisation, and XUI19 owns general decision state. This
separation is also recorded as the single concise A37 row in
`extractor/documents/RE_REMAINING.md`.

Retained UI03/UI06 capture contact sheets do not contain an open power-balance
panel. That creates no capture residual: shipped GUI/GFX/shader files and the
already-frozen UI03/UI06 engine contracts close the replay-owned presentation
law, while runtime-selected leaves are explicitly carried by A37/A25/A28.

## Adjacent country-politics launcher

`interface/countrypoliticsview.gui` also declares the launcher container
`ruling_party_info/power_balance_button` at `[320,-62]`, with
`show_power_balance`, `show_power_balance_glow`, `power_balance_percentage`,
`power_balance_levels`, and code-overridden `power_balance_ICON` children. Its
source SHA-256 is
`811051a83ef087b87be4de9b226fb831f6d6d8f20cd440371619501b5710c183`.

This launcher sits inside XUI11's explicitly owned `countrypoliticsview`
composition boundary, not the opened XUI18 `powerbalanceview` panel. XUI02's
prior inventory classified the launcher root as
`UNRESOLVED_ROOT_NO_RUNTIME_PRODUCER`. The exact percentage/levels strings,
code-selected icon, and glow visibility are now explicit A37/XUI18 producer
fields consumed by XUI11. The exact selector scans engine balance order: only
display ordinal 0 is launcher-visible and carries the three references plus
glow state; every later ordinal carries an explicit hidden/empty tuple. Neither
renderer derives these values from the static placeholder text or icon.

## Mechanical verification

- Focused XUI18 tests: **12/12 passed** within the root-owned centralized
  Final/All run.
- Complete replay workspace, including every `game_ui` acceptance and
  integration target plus doctests: pass under the same centralized run.
- The deterministic XUI18 verifier consumes that exact central attestation and
  reruns the two packet-owned parser gates without invoking Cargo itself.

## Binary provenance reused from frozen UI03

No new Ghidra operation was issued. The animation and render-tree claims reuse
these already-frozen binary packets for `hoi4.exe 1.19.2.0.a729`, image base
`0x140000000`, SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

| Function address | Job ID | Artifact path | Artifact SHA-256 | Manifest hash |
|---|---|---|---|---|
| `0x1422b01e0` | `20260716T052828Z-ui03-container-state-scan-b0000-b0fff-8277b271` | `evidence/func_1422b01e0.decompile.c` | `914673aac7e7075024d7b23e660fb5d140ff24d4b8f10c28ec2f21ce7290a221` | `5a5848e827371cd48996853196a37f88324f31c099da065c0314722d37b6de43` |
| `0x1422af780`, `0x14254c2a0`, `0x14254b3d0` | `20260716T054324Z-ui03-animation-easing-helper-exact-v1-8931dd93` | `evidence/func_1422af780.decompile.c` | `83875abe91704844dfa8686d2446884815586a00c03231144aa9893d247b1253` | `7da8758aa9e0a978f9120381dfc25ab4f218191a9b0a2fed6214d9d1d5c057d8` |
| `0x1422284b0` | `20260716T043945Z-ui07-guiobject-event-slot10-lower-neighb-5f5329bd` | `evidence/func_1422284b0.decompile.c` | `3f493758f90858ea0537997e41d7c8f6df77aacb62bfe52309d0afa597495ed4` | `0b1f8e644087b46b5dfbd69252b2160d73ab56b500905a7923fc6a097ab0722d` |
| `0x1422998c0` | `20260716T043945Z-ui07-guiobject-event-slot10-lower-neighb-5f5329bd` | `evidence/func_1422998c0.decompile.c` | `e69c895747e1c57dcf02f3d093c0b89496fe8803d5021dd894b2bfb24d2cf000` | `0b1f8e644087b46b5dfbd69252b2160d73ab56b500905a7923fc6a097ab0722d` |

## Evidence disposition

The deterministic contract is
`replay_interface/re_evidence/rendering_laws/XUI18/promotion_contract.json`;
all incidental findings, including the 46 authored/materialized GFX commands,
seven preserved duplicate siblings, 68 shared boundaries, scripted FIN token,
measured-empty categories, unresolved localisation families, and the
code-positioned range-indicator anchor, are retained in `all_findings.json`.

**Non-data residuals: zero. Human input required: none. Adversarial review and
PROGRAM_STATUS edits were intentionally not performed.**
