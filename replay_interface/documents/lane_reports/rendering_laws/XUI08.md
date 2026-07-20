# XUI08 — read-only designers and peace conference

## Disposition

**REPLAY LAWS PROMOTION READY; EXTERNAL ACTIVE CARRIER REQUIRED.**

`game_ui::readonly_design_peace` now provides the complete fail-closed replay
admission and immutable render model for division, equipment, tank, plane,
ship, and variant-upgrade designers, plus inactive and active peace
conferences. There are no replay-owned partial, tool-blocked, or data-blocked
fields. A40's active selected-designer/peace-graph carrier remains an external
producer dependency and is not represented as a replay defect.

## Designer law

An admitted designer snapshot is the lossless render model. It preserves every
producer-resolved display atom and exact row order. Admission requires:

- a complete inert snapshot with `controls_interactive=false`;
- exact catalog display order and selected design identity;
- exact selected owner, raw name, archetype, and active/obsolete lifecycle;
- exact selected module, numeric stat, and upgrade identity/order/payload;
- unique module slots, section/stat pairs, upgrade IDs, and division grid cells;
- valid provenance on every displayed row and nonempty optional tokens.

The six frozen chrome roots are:

| Surface | GUI file | Root widget |
|---|---|---|
| Division | `interface/divisiondesignerview.gui` | `countrydivisiondesignerview` |
| Equipment | `interface/equipmentdesignerview.gui` | `countryequipmentdesignerview` |
| Tank | `interface/tank_designer_view.gui` | `tank_designer_view` |
| Plane | `interface/plane_designer_view.gui` | `plane_designer_view` |
| Ship | `interface/equipmentdesignerview.gui` | `countryequipmentdesignerview` |
| Variant upgrade | `interface/equipmentupgradedesignerwindow.gui` | `equipment_upgrade_designer_window` |

## Peace-conference law

An authenticated inactive record is independently admissible. An active
conference must be complete and retains the full ordered phase/turn, score,
participant, allocation, contest, target, and result graph. Participant,
allocation, state, ship, and result identities are unique where required;
references resolve before admission; and each result exactly matches its
allocation's action, actor, beneficiary, target country, state IDs, and ship
IDs.

The conference identity is no longer an opaque producer string. Exact-a729
stores a registered `CReferenceObject` key as two `u32` words at conference
`+0x08`; word 0 selects the engine reference table and both words are compared
before resolution. Replay transports the pair pointer-free and requires the
canonical display identity `hoi4-ref:<object_type>:<object_id>`. The all-zero
sentinel, a resolved process pointer, or a mismatched display id rejects.

`action_kind` is the replay discriminator bound to the exact live controller
slot and GUI control. `take_states` is also the documented game-script action
type. Display ordinal and controller offset are separate laws: the authored GUI
places resource rights before war reparations, while the live controller stores
war reparations first.

| Display ordinal | Replay action kind | GUI widget | Live controller offset | Group |
|---:|---|---|---:|---|
| 0 | `take_states` | `take_state_action` | `+0x5190` | primary |
| 1 | `liberate` | `liberate_action` | `+0x5198` | primary |
| 2 | `puppet` | `puppet_action` | `+0x51A0` | primary |
| 3 | `force_government` | `force_government_action` | `+0x51A8` | primary |
| 4 | `take_navy` | `take_navy_action` | `+0x51B0` | primary |
| 5 | `demilitarize_zone` | `demilitarize_zone_action` | `+0x51B8` | stackable |
| 6 | `resource_rights` | `resource_rights_action` | `+0x51C8` | stackable |
| 7 | `war_reparation` | `war_reparation_action` | `+0x51C0` | stackable |
| 8 | `dismantle_industry` | `dismantle_industry_action` | `+0x51D0` | stackable |

## Frozen game and capture evidence

| Game GUI | SHA-256 |
|---|---|
| `interface/divisiondesignerview.gui` | `9f6ed489e62ad92d29ea163037770259fe7d5c2d5a2a0ee3d3d9b9a85b42d7a4` |
| `interface/equipmentdesignerview.gui` | `c1a6cf7c0b74f9f428cf5d50c3f422c5f675b38adbcef9c8c1e13070fa9dbc22` |
| `interface/equipmentupgradedesignerwindow.gui` | `45fc3879b0050caefc5db2375fcfc3a55a7aec8bcdd68cbf49ccd8972c059563` |
| `interface/tank_designer_view.gui` | `853df0257bbbc6666c640583a37aa3cfb1870419a9370069fcb5532b0932d561` |
| `interface/plane_designer_view.gui` | `6ef7b4924c5f4d3cbd1be71b46c0e31e86089b95eacb978fa01d5d40f18ed9fa` |
| `interface/peaceconferencewindow.gui` | `cc8a5e1060593ef9d763e8d8444f37020a689aac78f276a7899d6381fafc02bc` |

The retained design probe contains 649 named division templates with regiment
composition and 3,594 equipment variants: 1,401 land-other, 578 land-vehicle,
413 plane, and 1,202 ship records. Of those, 2,153 variants carry modules. The
authenticated intelligence probe contains exactly inactive peace state
`{ "active": false }`.

- design probe SHA-256: `ca4b6be63b4cf4bf1e3c7f978d236a4e9439795082a7571cda1e9a49da63e60f`
- inactive-peace probe SHA-256: `30f55f090dd47d16e63e1b9bf7edfc0c1041222d8876b432d05e7b63bef51c3f`

## External producer boundary

The read-only A40 contract remains frozen at
`extractor/documents/dossiers/A40_XUI08_READONLY_INPUTS.json`, SHA-256
`978139c87bc914e74af3d56f7d1b2cba8163fb7d4b460ad24c5afc9f2e4fa524`.
It must supply the active selected/open designer state, engine-resolved display
text and module availability, and the complete active peace graph. Without that
authenticated carrier, active or selected surfaces reject. A27's admitted live
design catalog and A25's producer-resolved localized text are also explicit
inputs; none is synthesized by replay.

Static identity RE is exhausted in
`extractor/documents/dossiers/A40/H_PEACE_PORTABLE_IDENTITY_CLOSEOUT_20260720.md`.
It closes the portable conference key and the claim/contest source-order
payload, but proves that `CPeaceAction` is a value object without a registered
id. The terminal-outcome xrefs also collapse to `0x140B6FF40`, a presentation
formatter over already-aggregated summary collections with no result or
allocation identity. Allocation identity across refresh and terminal result
binding therefore remain live-transition inputs; active production emission
stays disabled.

## Binary provenance

Pinned authority: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`,
SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

| Function | Job ID | Artifact | Artifact SHA-256 | Manifest SHA-256 |
|---|---|---|---|---|
| `0x141847980` | `20260717T110432Z-a40-peace-window-controller-roots-retry-eca38117` | `evidence/func_141847980.decompile.c` | `cb557f0ae20895da636614e5e65cbfee385002a344756788e1f494c0e6a5e2ca` | `74778064ea9a1f32ba19c1ab0c3235a0043fa65988cc8959a9bef8ada4295d86` |
| `0x14184FFB0` | `20260717T110304Z-a40-peace-action-turn-producers-3a638690` | `evidence/func_14184ffb0.decompile.c` | `81853907be9a570e61bd0adc16a04c87d2d3ef6f95af2313c7ef33f5444c1021` | `14445913ebd641dd6556c70955374d671b12c1a4fcef38cea009e52a8b28ae1c` |
| `0x1418522F0` | `20260717T110304Z-a40-peace-action-turn-producers-3a638690` | `evidence/func_1418522f0.decompile.c` | `d55ae3c4f38832a0deb42b4499686b43808212ed31e0d0445704263d0c7318a3` | `14445913ebd641dd6556c70955374d671b12c1a4fcef38cea009e52a8b28ae1c` |
| `0x141853780` | `20260717T110304Z-a40-peace-action-turn-producers-3a638690` | `evidence/func_141853780.decompile.c` | `c3c118f211c6053cad936fa3e39315dd1d60de3a40de7a0ca3a8f244e429da20` | `14445913ebd641dd6556c70955374d671b12c1a4fcef38cea009e52a8b28ae1c` |
| `0x140E29E00` | `20260717T110304Z-a40-peace-action-turn-producers-3a638690` | `evidence/func_140e29e00.decompile.c` | `e01ccb6350eadc5158978685133dcc43166b244db5296a7ea65dbfd0689e68c4` | `14445913ebd641dd6556c70955374d671b12c1a4fcef38cea009e52a8b28ae1c` |
| `0x140E29970` | `20260717T110304Z-a40-peace-action-turn-producers-3a638690` | `evidence/func_140e29970.decompile.c` | `dad2fae2dae3e8e3f828b392167a986efc570d699f048e1c10b2573dc2f9ab93` | `14445913ebd641dd6556c70955374d671b12c1a4fcef38cea009e52a8b28ae1c` |
| `0x140B6FF40` | `20260720T050217Z-w4-a40-peace-terminal-result-functions-f9a925e6` | `evidence/func_140b6ff40.decompile.c` | `cc6ca57447e52c2ea27293c61229c8e506feaf2934be320de55dd7c8e43bd2c5` | `71357eb95ff347afab566b15113589580553b0b2316f6f877915d31436a2c7f8` |
| `0x141DC3B90` | `20260717T110314Z-a40-designer-window-module-stat-producer-249a7729` | `evidence/func_141dc3b90.decompile.c` | `7204575302a7ff934cac6b6134fe5bb09c71f0d4cf553a6f3153ce6cd9101cb9` | `ac013804762b7a628217d731cbc6612047e8033fcbc8f699e7736e867f0a695b` |
| `0x141DC38A0` | `20260717T110314Z-a40-designer-window-module-stat-producer-249a7729` | `evidence/func_141dc38a0.decompile.c` | `8bfbad6f84b4f2ad0c05cec48f9c6d7736e0f7728b10d6d5bd2e2deb8662f542` | `ac013804762b7a628217d731cbc6612047e8033fcbc8f699e7736e867f0a695b` |
| `0x141769820` | `20260717T110352Z-a27-designer-grid-population-callers-9c115a8b` | `evidence/func_141769820.decompile.c` | `0096a93a24c4912b16de85d7ee579a27251b7288ff7452c63ca808fe65cdbc2c` | `e12cbbd037d94c33a81381b49021116dc1ca7af7325af9bbffe101acd5d82904` |
| `0x141492B70` | `20260717T105543Z-a40-designer-entry-row-producers-f122b2c0` | `evidence/func_141492b70.decompile.c` | `ba0b87bc64a9430fc5af38dc602b1d80ea0d8cb82a8f8d315ccae857c634f99d` | `fede7722aa8b31d5d044077b1aba688ef163e66a5a03da3cc964d7374f94fe5c` |

`0x14184FFB0` supplies the exact live controller-slot map, including the
resource-rights/war-reparation crossing. `0x1418522F0` proves the first-five
primary and next-four stackable grouping and runtime widget visibility law.
The remaining functions freeze conference identity/turn/score producers,
designer source order, row identity, and module availability.

## Production verification

The wave-4 `PeaceConferenceReference` admission change is pending the root
agent's centralized Rust validation. The historical results below predate that
change and are retained only as the prior baseline.

- implementation SHA-256: `cf7ada95db21432232d80b178a9f7b717d4b5832e131ede124289cbdbf86fc29`
- focused tests SHA-256: `ff40ce428474cc5ee9e0b7c1251626958016b13dde8217707509486f156573c9`
- focused gate: 14 passed, 0 failed
- full `cargo test -p game_ui --tests --quiet`: 120 passed, 0 failed, 1 ignored
- contract: 29/29, SHA-256 `4ce431a9ea5f66a8e8200b099958ee6e4d7feefcc68da85928df72d342645de6`
- verification: 16/16, SHA-256 `a88ddb42d937ed65f5c9b9fdc055eea8aa5dba8260dbb035180a0971219f83a1`
- manifest: 14 artifacts, SHA-256 `a06a862c21c57bc8ec5db20e33f12cbf14304fbe6bb9932fe923f901c2640ad7`

Two consecutive contract/verifier/manifest passes were byte-identical. XUI08
does not modify extractor code or `PROGRAM_STATUS`.
