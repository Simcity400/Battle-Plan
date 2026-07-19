# XUI16 - occupation, resistance/compliance, garrison and alerts/logs

**Status: PROVEN - PROMOTION-READY, FAIL CLOSED**

XUI16 has a production replay boundary for occupation state values, occupation-law identity, garrison assignment/equipment, resistance-attack history, the controller-country-global garrison alert, shared map-mode inputs, authenticated row order and visible presentation. Missing or incomplete producer data rejects the surface; the replay layer does not invent empty equipment, log or alert rows, thresholds, text, sort order, law identities or garrison values.

## Proven replay law

- `replay_core::OccupationRow` retains every domain-52/domain-66 occupation and garrison carrier. Direct domain-66 law identity wins; domain 52 is used only when the direct value is absent.
- State admission requires exact measured resistance/compliance and speeds, occupier/occupied tags, recognized FNV law encoding and matching static law definition, garrison template, nonnegative manpower/requirement, unit-interval strength and a complete per-state equipment set.
- The save producer retains ordered current equipment, need/reinforcement rows and resistance-attack history. Equipment `(id,type)` joins the equipment-variant catalog; measured-empty collections remain distinct from absence. Sparse omitted attack manpower is exact zero (`sum([])`), not an incomplete row.
- `alert_not_enough_garrison` is a HIGH country-global alert registered in slot `0x3f`. Its predicate returns false on a strictly positive aggregate manpower deficit or any positive whole-equipment deficit after exact signed fixed-point division by `100000`; it is not a per-state row and state garrison ratios cannot synthesize its membership.
- Dynamic materialization/order is consumed from A28/XUI03 and runtime visible strings from A25. XUI16 does not duplicate or infer either producer.
- The shared occupation map mode parses the shipped noninitialized color and five resistance plus five compliance color stops. Sampling is linear between the two closest authored stops; an exact duplicate threshold selects the later authored stop.
- The compose seam binds shipped names for state name, selected-law text/icon, garrison requirement text and strength bar without renderer-side numeric clamping.

## Static and retained-data evidence

The frozen shipped files are `interface/countryoccupationview.gui`, `interface/garrisonlogview.gui`, `interface/countrystateview.gui`, `common/occupation_laws/occupation_laws.txt` and `common/defines/00_graphics.lua`. The GUI defines five sort controls, resistance/compliance tabs, country/state/no-resistance rows, law and garrison selection, the 1-12 month log control, activity/equipment tabs, and attack-row location/date/blocked/manpower/equipment fields.

The production parser was run over the 51,196,609-character `SPR_1942_melted.txt` at `1942.8.16.19`. It emitted 499 state-garrison rows, 1,241 current-equipment rows, 3,974/3,974 complete resistance-attack rows and 14,175 equipment-loss children. The snapshot fold joins equipment identities, distinguishes per-state law from controller-default fallback, exposes typed current/history carriers, and feeds typed equipment directly into XUI16.

Shipped `common/alerts.txt` defines `alert_not_enough_garrison` as `HIGH`; four `alerts_l_english.yml` keys define its instant/warning and aggregate missing-manpower/equipment text. Binary export proves the country-global registry slot and predicate/detail law: positive manpower shortages emit `alert_not_enough_garrison_delayed`; positive whole-equipment children emit `alert_not_enough_garrison_delayed_equipment`; the instant builder uses `alert_not_enough_garrison_instant` and appends `ALERT_RIGHTCLICK`. This closes the former per-state alert contract without RenderDoc or additional save data.

## Mechanical verification

- Deterministic evidence builder executed twice with identical contract, census and findings hashes.
- `replay_core` focused integration test: 1/1 passed.
- `game_ui` XUI16 tests: 2/2 passed, including complete admission and incomplete-producer rejection.
- Save parser/development-oracle tests: 2/2 passed, including sparse-zero and measured-empty rows.
- Replay store occupation suite: 27 passed plus five intentionally skipped real-output tests (32 total discovered).
- `map_viewer` occupation-map-mode test: 1/1 passed.
- Verifier checks executable authority, seven shipped-file hashes, law census, required GUI tokens, map stops, replay source gates, A35/A28/A25 separation and zero implementation residuals.

## Binary provenance

| Function address | Job ID | Artifact path | Manifest hash | Disposition |
|---|---|---|---|---|
| `0x1408970c0` | `20260717T114929Z-a35-garrison-strength-producer-functiona-6e318b08` | `C:/Users/Example/ghidra_agent_exports/20260717T114929Z-a35-garrison-strength-producer-functiona-6e318b08/evidence/func_1408970c0.decompile.c` (`2dab97e4b844aaaf2788e7b4be1b70950d08b2a0659da22808de68cce803e053`) | `9ffeb5e8bc1b2149f6d94047c8703b5fb7b02da0cc8b8b1279b45e645e3af777` | Important negative: graphics-define loader for `NInterface.GARRISON_STRENGTH_TO_SHOW_RED`, not the runtime value/alert producer. |
| `0x140aeeaa0` | `20260717T115443Z-a35-global-garrison-alert-three-xrefs-fbe31428` | `C:/Users/Example/ghidra_agent_exports/20260717T115443Z-a35-global-garrison-alert-three-xrefs-fbe31428/evidence/func_140aeeaa0.decompile.c` (`b241945ccaccfb2c170f0689aca7f9c186f193b1e5f1c93563e7baa0458cc750`) | `497e9abb701c996663de199ee29eef24314d522493d14c7a8e4c8da68685ce18` | Registry constructor: registers `alert_not_enough_garrison` in slot `0x3f` and loads `common/alerts.txt`. |
| `0x140afe8a0` | `20260717T115443Z-a35-global-garrison-alert-three-xrefs-fbe31428` | `C:/Users/Example/ghidra_agent_exports/20260717T115443Z-a35-global-garrison-alert-three-xrefs-fbe31428/evidence/func_140afe8a0.decompile.c` (`1c4f70f617bda04d9ed3ed9b8c98ce63b1465c273dab461c76934cf8cd7d56c3`) | `497e9abb701c996663de199ee29eef24314d522493d14c7a8e4c8da68685ce18` | Country-global predicate/detail builder: false on positive shortage; positive manpower deficit and equipment children after exact `/100000` signed fixed-point conversion. |
| `0x140b151c0` | `20260717T115443Z-a35-global-garrison-alert-three-xrefs-fbe31428` | `C:/Users/Example/ghidra_agent_exports/20260717T115443Z-a35-global-garrison-alert-three-xrefs-fbe31428/evidence/func_140b151c0.decompile.c` (`d0e1f194fb308d3160e1ac53084dd79f35ac3689368fd3a6c0094967bb802625`) | `497e9abb701c996663de199ee29eef24314d522493d14c7a8e4c8da68685ce18` | Instant-tooltip builder: current-country detail via `0x140afe8a0`, instant title, and `ALERT_RIGHTCLICK`. |
| `0x14030b010`, `0x1409925b0`, `0x1409929d0`, `0x140f80870`, `0x140992df0`, `0x140993210`, `0x140993000`, `0x140993630`, `0x140992be0`, `0x140993c60`, `0x140993840`, `0x140993a50` | `20260717T114427Z-a35-occupation-garrison-distinct-health--06668846` | same `string-xrefs.txt` | `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` | `RESISTANCE_TARGET` producer family; recorded for later use without claiming alert semantics. |
| N/A | N/A | `replay_interface/re_evidence/rendering_laws/XUI16/evidence_manifest.json` | refreshed by `verify_xui16.py` | File/save-oracle/replay-integration packet; no binary address is claimed for those save carriers. |

Representative Ghidra infrastructure diagnostics are frozen in A35: jobs `20260717T112613Z-a35-occupation-garrison-alert-anchors-v3-0c6812ad` and `20260717T112710Z-a35-occupation-garrison-alert-anchors-v4-95fc63c0`, artifact `identity.console.log`, hashes `d6c383d89399baf4d132cf773cbc8376691c31d14571360bdff552a6b031b7dc` and `78a9e6aa466932949abc9a3f0eec94a991905421d928d10341d29cba2feefa0e`; both failed before function resolution because the worker project lacked `hoi4.exe`. They are infrastructure evidence, not binary findings.

**Implementation residuals: none in the replay-owned XUI16 boundary. A35 equipment/history and the country-global alert law are promotion-ready. Exact shared inputs still consumed from separate owners are A28 collection materialization/order and A25 runtime text. No new capture or human data extraction is required.**
