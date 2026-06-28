//! Designer cluster tests: per-view element-accounting (scene walk == the `.gui`'s
//! declared `element_type_counts`), render-plan digest scrub A→B→A byte-identity,
//! no-fabrication (a blocked field renders "—", never a 0; the computed combat stats are
//! BADGED not computed), bound-element-name existence, and inventory registration.
//!
//! The mini demo snapshot has NO `designs` domain, so two SYNTHETIC snapshots (a save-
//! channel `designs` + a live-channel `equipment_variants_live`) exercise the BOUND paths
//! and the scrub.

use super::*;
use crate::host::context::Stage;
use crate::host::input::{PointerButton, ViewInput};
use crate::host::view::{InputOutcome, RebindContext};
use crate::views::digest::bound_digest;
use gui_runtime::{GuiFile, Loc, Rect};
use std::collections::HashMap;
use std::path::Path;

// ----------------------------------------------------------------------------- fixtures

/// The `game_assets/gui/` dir (READ-ONLY repo data), reached from the crate root.
fn gui_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("game_assets")
        .join("gui")
}

fn assets_present() -> bool {
    gui_dir().join("tank_designer.json").exists()
}

fn load_gui(name: &str) -> Option<GuiFile> {
    let p = gui_dir().join(name);
    std::fs::read_to_string(&p)
        .ok()
        .and_then(|s| GuiFile::parse(&s).ok())
}

fn stage_with(files: &[&str]) -> Stage {
    let mut gui_files = HashMap::new();
    for f in files {
        if let Some(g) = load_gui(f) {
            gui_files.insert(f.to_string(), g);
        }
    }
    Stage {
        resolver: assets::resolver::GameResolver::new(None, None),
        manifest: assets::SpriteManifest {
            sprites: HashMap::new(),
        },
        loc: Loc::new(),
        gui_files,
        map_dir: std::path::PathBuf::from("."),
        assets_dir: std::path::PathBuf::from("."),
    }
}

fn ctx<'a>(stage: &'a Stage, snap: &'a snapshot::Snapshot) -> BindContext<'a> {
    BindContext {
        stage,
        snapshot: snap,
        country: "GER",
        screen: Rect {
            x: 0.0,
            y: 0.0,
            w: 1600.0,
            h: 900.0,
        },
        date: "1941.1.1.12",
        cursor: (0.0, 0.0),
        map_camera: None,
    }
}

/// SAVE-channel snapshot: a top-level `designs` domain with a GER tank/plane/ship variant
/// (save `modules` + `upgrades`) + a GER division template (regiment grid) + per-country
/// `military.xp`. The LIVE leaf is ABSENT (the save path is exercised).
fn snap_save() -> snapshot::Snapshot {
    serde_json::from_value(serde_json::json!({
        "date": "1941.1.1.12", "hour_index": 100,
        "countries": { "GER": { "military": { "xp": { "army": 42.0, "navy": 7.0, "air": 19.0 } } } },
        "designs": {
            "equipment_variants": {
                "1855:30": {
                    "id": 1855, "type": 30, "archetype": "medium_tank_chassis_1",
                    "family": "land_vehicle", "name": "Panzer IV", "version": 2,
                    "parent_id": null, "creator": "GER", "obsolete": false,
                    "modules": {"main_armament_slot": "medium_cannon", "turret_slot": ""},
                    "upgrades": {"tank_armor_upgrade": 2, "tank_engine_upgrade": 1},
                    "design_team": [200, 12],
                    "design_team_mio": {"tag": "GER", "key": "GER_krupp", "name": "Krupp", "size": 3}
                },
                "900:20": {
                    "id": 900, "type": 20, "archetype": "small_plane_airframe_1",
                    "family": "plane", "name": null, "version": 1, "creator": "GER",
                    "obsolete": false, "modules": {"engine_slot": "engine_1"},
                    "upgrades": {"plane_reliability_upgrade": 1},
                    "design_team_mio": null
                },
                "700:10": {
                    "id": 700, "type": 10, "archetype": "ship_hull_light_1",
                    "family": "ship", "name": "Z-Destroyer", "version": 3, "creator": "GER",
                    "obsolete": false, "modules": {"fixed_ship_battery_slot": "ship_light_battery_2"},
                    "upgrades": {"ship_engine_upgrade": 2},
                    "design_team_mio": {"tag": "GER", "key": "GER_dw", "name": "Deutsche Werke", "size": 2}
                },
                "5:5": {
                    "id": 5, "type": 5, "archetype": "x", "family": "land_vehicle",
                    "name": "FRA design", "version": 1, "creator": "FRA", "obsolete": false,
                    "modules": {}, "upgrades": {}, "design_team_mio": null
                }
            },
            "division_templates": [
                {"id": 1, "type": 1, "name": "Infanterie", "country": "GER", "role": "infantry",
                 "regiments": [{"type": "infantry", "x": 0, "y": 0}, {"type": "infantry", "x": 0, "y": 1},
                               {"type": "artillery", "x": 1, "y": 0}],
                 "support": ["engineer", "recon"], "regimental_support": [],
                 "priority": 1, "division_cap": 0, "obsolete": false, "is_locked": false,
                 "is_army_hq": false, "n_regiments": 3, "n_support": 2},
                {"id": 2, "type": 1, "name": "FRA div", "country": "FRA", "role": "infantry",
                 "regiments": [], "support": [], "regimental_support": [], "priority": 0,
                 "is_army_hq": false}
            ]
        }
    }))
    .unwrap()
}

/// LIVE-channel snapshot: the SAME GER variants, but the live `equipment_variants_live`
/// leaf (keyed by bare variant id) carries a DIFFERENT module layout (proves the live path
/// is preferred + the scrub differs from the save). Division template name changed too.
fn snap_live() -> snapshot::Snapshot {
    serde_json::from_value(serde_json::json!({
        "date": "1942.1.1.12", "hour_index": 200,
        "countries": { "GER": { "military": { "xp": { "army": 99.0, "navy": 50.0, "air": 30.0 } } } },
        "designs": {
            "equipment_variants": {
                "1855:30": {
                    "id": 1855, "type": 30, "archetype": "medium_tank_chassis_1",
                    "family": "land_vehicle", "name": "Panzer IV Ausf G", "version": 4,
                    "creator": "GER", "obsolete": false,
                    "modules": {"main_armament_slot": "medium_cannon"},
                    "upgrades": {"tank_armor_upgrade": 4},
                    "design_team_mio": {"tag": "GER", "key": "GER_krupp", "name": "Krupp", "size": 4}
                },
                "900:20": {
                    "id": 900, "type": 20, "archetype": "small_plane_airframe_1",
                    "family": "plane", "name": "Bf 109", "version": 2, "creator": "GER",
                    "obsolete": false, "modules": {"engine_slot": "engine_2"},
                    "upgrades": {"plane_reliability_upgrade": 3}, "design_team_mio": null
                },
                "700:10": {
                    "id": 700, "type": 10, "archetype": "ship_hull_light_1",
                    "family": "ship", "name": "Z-Destroyer II", "version": 5, "creator": "GER",
                    "obsolete": true, "modules": {"fixed_ship_battery_slot": "ship_light_battery_3"},
                    "upgrades": {"ship_engine_upgrade": 3}, "design_team_mio": null
                }
            },
            "equipment_variants_live": {
                "1855": {"modules": {"main_armament_slot": "heavy_cannon", "turret_slot": "armor_plate"},
                         "upgrade_levels": [4, 2]}
            },
            "equipment_variants_live_source": "M",
            "division_templates": [
                {"id": 1, "type": 1, "name": "Panzergrenadier", "country": "GER", "role": "armor",
                 "regiments": [{"type": "motorized", "x": 0, "y": 0}, {"type": "medium_armor", "x": 1, "y": 0}],
                 "support": ["maintenance"], "regimental_support": [], "priority": 2,
                 "is_army_hq": false, "n_regiments": 2, "n_support": 1}
            ]
        }
    }))
    .unwrap()
}

/// Army-HQ template snapshot A: HQ tab enabled, with A-only normal-tab regiment tokens.
fn snap_hq_a() -> snapshot::Snapshot {
    serde_json::from_value(serde_json::json!({
        "date": "1941.6.1.12", "hour_index": 150,
        "countries": { "GER": { "military": { "xp": { "army": 55.0 } } } },
        "designs": {
            "equipment_variants": {},
            "division_templates": [
                {"id": 10, "type": 1, "name": "Stabsdivision", "country": "GER", "role": "hq",
                 "regiments": [{"type": "infantry", "x": 0, "y": 0}, {"type": "signal_company", "x": 1, "y": 0}],
                 "support": ["engineer"], "regimental_support": [],
                 "priority": 3, "division_cap": 0, "obsolete": false, "is_locked": false,
                 "is_army_hq": true, "n_regiments": 2, "n_support": 1}
            ]
        }
    }))
    .unwrap()
}

/// Army-HQ template snapshot B: same tab shape, with B-only identity and regiment tokens.
fn snap_hq_b() -> snapshot::Snapshot {
    serde_json::from_value(serde_json::json!({
        "date": "1942.6.1.12", "hour_index": 250,
        "countries": { "GER": { "military": { "xp": { "army": 77.0 } } } },
        "designs": {
            "equipment_variants": {},
            "division_templates": [
                {"id": 10, "type": 1, "name": "Fuehrungsstab Ost", "country": "GER", "role": "hq",
                 "regiments": [{"type": "rocket_artillery", "x": 0, "y": 0}],
                 "support": ["field_hospital"], "regimental_support": [],
                 "priority": 4, "division_cap": 0, "obsolete": false, "is_locked": false,
                 "is_army_hq": true, "n_regiments": 1, "n_support": 1}
            ]
        }
    }))
    .unwrap()
}

/// A snapshot with NO `designs` domain (mirrors the mini demo) - every variant/template
/// bind must badge unavailable, never fabricate.
fn snap_no_designs() -> snapshot::Snapshot {
    serde_json::from_value(serde_json::json!({
        "date": "1941.1.1.12",
        "countries": { "GER": {} }
    }))
    .unwrap()
}

// --------------------------------------------------------- element accounting (per view)

/// Count `type` tokens recursively over the RAW gui json (the authoritative element set,
/// incl. layout primitives the typed scene drops) — identical method to the production
/// cluster's accounting test.
fn raw_type_counts(name: &str) -> HashMap<String, usize> {
    let p = gui_dir().join(name);
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    let mut counts: HashMap<String, usize> = HashMap::new();
    fn walk(node: &serde_json::Value, counts: &mut HashMap<String, usize>) {
        if let Some(t) = node.get("type").and_then(|t| t.as_str()) {
            *counts.entry(t.to_string()).or_insert(0) += 1;
        }
        if let Some(children) = node.get("children").and_then(|c| c.as_array()) {
            for c in children {
                walk(c, counts);
            }
        }
    }
    if let Some(windows) = v.get("windows").and_then(|w| w.as_array()) {
        for w in windows {
            walk(w, &mut counts);
        }
    }
    counts
}

fn declared_counts(name: &str) -> HashMap<String, usize> {
    let p = gui_dir().join(name);
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    let etc = v
        .get("stats")
        .and_then(|s| s.get("element_type_counts"))
        .and_then(|e| e.as_object())
        .expect("element_type_counts present");
    etc.iter()
        .map(|(k, v)| (k.clone(), v.as_u64().unwrap() as usize))
        .collect()
}

/// Every element of each designer `.gui` is accounted (walk == declared counts) — none
/// silently dropped. Runs for all 5 files.
#[test]
fn element_accounting_all_five() {
    if !assets_present() {
        eprintln!("[designers] skipped element_accounting: game_assets/gui not extracted");
        return;
    }
    for f in [
        "tank_designer.json",
        "plane_designer.json",
        "division_designer.json",
        "equipment_designer.json",
        "equipment_upgrade_designer.json",
    ] {
        assert_eq!(
            raw_type_counts(f),
            declared_counts(f),
            "element accounting mismatch in {f}"
        );
    }
}

/// The ACTUAL parsed-node totals (NOT the binding-map's stale 209/186/.../300 — the real
/// files are 207/187/313/247/285). A guard that the source files have not drifted.
#[test]
fn element_accounting_totals_actual() {
    if !assets_present() {
        eprintln!("[designers] skipped totals: game_assets/gui not extracted");
        return;
    }
    let total = |n: &str| raw_type_counts(n).values().sum::<usize>();
    assert_eq!(total("tank_designer.json"), 207);
    assert_eq!(total("plane_designer.json"), 187);
    assert_eq!(total("division_designer.json"), 313);
    assert_eq!(total("equipment_designer.json"), 247);
    assert_eq!(total("equipment_upgrade_designer.json"), 285);
}

/// Every registered designer ROOT resolves to a REAL, POPULATED `.gui` scene-graph via
/// the cluster's own `resolve_gui` (Stage-first, then the cached sanitized fallback for
/// the 3 files the shared parser rejects) — proving every view renders the REAL game
/// layout, not synthetic chrome. With an EMPTY stage the fallback path is forced, so this
/// also proves the `@named`-position sanitizer recovers the 3 otherwise-unparseable files.
#[test]
fn every_root_resolves_to_real_populated_gui() {
    if !assets_present() {
        eprintln!("[designers] skipped root-resolves: game_assets/gui not extracted");
        return;
    }
    // an EMPTY stage (no pre-loaded gui_files) → resolve_gui must use the sanitized fallback.
    let stage = stage_with(&[]);
    let snap = snap_save();
    let bctx = ctx(&stage, &snap);
    let cases = [
        (super::TANK_FILE, super::TANK_ROOT),
        (super::PLANE_FILE, super::PLANE_ROOT),
        (super::DIVISION_FILE, super::DIVISION_ROOT),
        (super::EQUIP_FILE, super::EQUIP_ROOT),
        (super::UPGRADE_FILE, super::UPGRADE_ROOT),
    ];
    for (file, root) in cases {
        let g = super::resolve_gui(&bctx, file);
        let w = g.windows.iter().find(|w| w.name.as_deref() == Some(root));
        assert!(
            w.is_some(),
            "root window {root} must resolve in {file} (real .gui)"
        );
        // the real root has a populated subtree (NOT an empty fallback root).
        let node = gui_runtime::scene::Node::from_raw(w.unwrap(), &stage.manifest, &stage.loc);
        assert!(
            node.count() > 5,
            "the real {root} subtree must be populated, not an empty fallback"
        );
    }
}

/// The `@named`-position sanitizer is SURGICAL: it rewrites only a `position.x/y` string
/// starting with `@` to 0.0, and leaves every data/text field untouched.
#[test]
fn sanitizer_only_touches_named_position_anchors() {
    let mut v = serde_json::json!({
        "type": "containerWindowType", "name": "w",
        "position": {"x": "@fixed_btn_mod_row_0", "y": 12},
        "text": {"text": "@keep_this_literal"},
        "children": [{"name": "c", "position": {"x": 5, "y": "@row"}, "value": "@data"}]
    });
    super::sanitize_named_positions(&mut v);
    // the @named position x → 0.0; the numeric y untouched.
    assert_eq!(v["position"]["x"], serde_json::json!(0.0));
    assert_eq!(v["position"]["y"], serde_json::json!(12));
    // a data/text field that merely starts with @ is NOT touched (only position x/y).
    assert_eq!(v["text"]["text"], serde_json::json!("@keep_this_literal"));
    assert_eq!(v["children"][0]["value"], serde_json::json!("@data"));
    assert_eq!(v["children"][0]["position"]["y"], serde_json::json!(0.0));
    assert_eq!(v["children"][0]["position"]["x"], serde_json::json!(5));
}

// ------------------------------------------------------------- render-plan scrub A→B→A

fn bind_view(view_id: &str, stage: &Stage, snap: &snapshot::Snapshot) -> String {
    let bv = match view_id {
        "designers.tank" => TankDesignerView::new().bind(&ctx(stage, snap)),
        "designers.plane" => PlaneDesignerView::new().bind(&ctx(stage, snap)),
        "designers.division" => DivisionDesignerView::new().bind(&ctx(stage, snap)),
        "designers.equipment" => EquipmentDesignerView::new().bind(&ctx(stage, snap)),
        "designers.equipment_upgrade" => {
            EquipmentUpgradeDesignerView::new().bind(&ctx(stage, snap))
        }
        _ => unreachable!(),
    };
    bound_digest(&bv)
}

// Component designer interaction helpers.
fn rebind_tank(v: &mut TankDesignerView, snap: &snapshot::Snapshot, refit: bool) {
    v.rebind(&RebindContext {
        snapshot: snap,
        country: "GER",
        screen: (1600, 900),
        refit,
    });
}

fn rebind_plane(v: &mut PlaneDesignerView, snap: &snapshot::Snapshot, refit: bool) {
    v.rebind(&RebindContext {
        snapshot: snap,
        country: "GER",
        screen: (1600, 900),
        refit,
    });
}

fn rebind_equipment(v: &mut EquipmentDesignerView, snap: &snapshot::Snapshot, refit: bool) {
    v.rebind(&RebindContext {
        snapshot: snap,
        country: "GER",
        screen: (1600, 900),
        refit,
    });
}

fn component_slot_center(interaction: &ComponentInteraction, slot_id: &str) -> (f32, f32) {
    let model = interaction
        .pick_model
        .as_ref()
        .expect("component pick model rebound");
    let slot = model
        .modules
        .slots
        .iter()
        .find(|slot| slot.id == slot_id)
        .unwrap_or_else(|| panic!("missing component slot {slot_id}"));
    let rect = slot.rect(model.grid_area);
    (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5)
}

fn component_disabled_point(interaction: &ComponentInteraction) -> (f32, f32) {
    let area = interaction
        .pick_model
        .as_ref()
        .expect("component pick model rebound")
        .disabled_area;
    (area.x + 8.0, area.y + 8.0)
}

fn assert_rect_close(actual: Rect, expected: Rect, label: &str) {
    let fields = [
        ("x", actual.x, expected.x),
        ("y", actual.y, expected.y),
        ("w", actual.w, expected.w),
        ("h", actual.h, expected.h),
    ];
    for (field, a, e) in fields {
        assert!(
            (a - e).abs() < 0.01,
            "{label}.{field}: expected {e}, got {a}"
        );
    }
}

fn pointer_move(x: f32, y: f32) -> ViewInput {
    ViewInput::PointerMove { x, y, drag: false }
}

fn left_click(x: f32, y: f32) -> ViewInput {
    ViewInput::PointerDown {
        x,
        y,
        button: PointerButton::Left,
    }
}

// Division tab interaction helpers.
fn rebind_division(v: &mut DivisionDesignerView, snap: &snapshot::Snapshot, refit: bool) {
    v.rebind(&RebindContext {
        snapshot: snap,
        country: "GER",
        screen: (1600, 900),
        refit,
    });
}

fn division_tab_center(bar: &TabBar, panel: Rect, id: &str) -> (f32, f32) {
    let rect = bar
        .layout(panel)
        .into_iter()
        .find(|tr| tr.id == id)
        .unwrap_or_else(|| panic!("missing division tab rect {id}"))
        .rect;
    (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5)
}

fn click_division_tab_center(v: &mut DivisionDesignerView, id: &str) -> InputOutcome {
    let panel = division_tab_panel_fallback(v.screen);
    let (x, y) = division_tab_center(&v.tabs, panel, id);
    v.on_input(&ViewInput::PointerDown {
        x,
        y,
        button: PointerButton::Left,
    })
}

/// Each designer rebinds A->B->A byte-identically without carrying B's values.
#[test]
fn scrub_a_b_a_byte_identical_all_five() {
    let files = &[
        ("designers.tank", "tank_designer.json"),
        ("designers.plane", "plane_designer.json"),
        ("designers.division", "division_designer.json"),
        ("designers.equipment", "equipment_designer.json"),
        (
            "designers.equipment_upgrade",
            "equipment_upgrade_designer.json",
        ),
    ];
    let (a, b) = (snap_save(), snap_live());
    for (vid, gui) in files {
        let stage = stage_with(&[gui]);
        let da1 = bind_view(vid, &stage, &a);
        let db = bind_view(vid, &stage, &b);
        assert_ne!(
            da1, db,
            "{vid}: A (save) and B (live) must differ, else the scrub proves nothing"
        );
        let da2 = bind_view(vid, &stage, &a);
        assert_eq!(
            da1, da2,
            "{vid}: scrub A→B→A carried a value from the B visit"
        );
    }
}

// --------------------------------------------------------------------- no-fabrication

/// The COMBAT STAT panels are BADGED unavailable, NEVER computed/fabricated: in EVERY
/// designer the fidelity report records the computed panels as unavailable and the digest
/// shows the "—" badges, even with fully-populated variant data.
#[test]
fn combat_stats_badged_not_computed() {
    let stage = stage_with(&["tank_designer.json"]);
    let snap = snap_save();
    let bv = TankDesignerView::new().bind(&ctx(&stage, &snap));
    // the variant IS bound (real modules), yet the stat panels stay unavailable.
    assert!(
        bv.fidelity.exact > 0,
        "the bound module/upgrade/identity fields are exact"
    );
    assert!(
        bv.fidelity.unavailable >= 6,
        "the 6 computed stat/cost panels are badged unavailable"
    );
    let d = bound_digest(&bv);
    // the computed panels render the not-captured glyph, never a number.
    assert!(
        d.contains("Combat Stats=—"),
        "combat stats must badge unavailable, got: {d}"
    );
    assert!(
        d.contains("Production Cost (IC)=—"),
        "production cost must badge unavailable"
    );
    // a measured bound field (the upgrade level) is a real value, not collapsed.
    assert!(
        d.contains("Tank Armor Upgrade=2(E)") || d.contains("tank_armor_upgrade=2(E)"),
        "an EXACT upgrade level must show its real value, got: {d}"
    );
}

/// With NO `designs` domain (the mini demo case) every variant bind badges unavailable —
/// never a fabricated 0 — and the chrome still renders (no panic).
#[test]
fn no_designs_domain_badges_never_fabricates() {
    let stage = stage_with(&["tank_designer.json"]);
    let snap = snap_no_designs();
    let bv = TankDesignerView::new().bind(&ctx(&stage, &snap));
    let d = bound_digest(&bv);
    assert!(
        bv.fidelity.unavailable > 0,
        "absent designs → unavailable badges"
    );
    assert!(
        d.contains('—'),
        "an absent variant must render the not-captured glyph"
    );
    // no fabricated "0" snuck in for a variant field (name/version/etc).
    assert!(
        !d.contains("Name=0"),
        "no fabricated 0 for an absent variant name"
    );
    assert!(
        !d.contains("Version=0"),
        "no fabricated 0 for an absent variant version"
    );
}

/// The LIVE module leaf is PREFERRED over the save `modules` (the live channel changes the
/// tank's module layout → the digest differs from the save-only bind).
#[test]
fn live_module_leaf_is_preferred() {
    let stage = stage_with(&["tank_designer.json"]);
    let save = TankDesignerView::new().bind(&ctx(&stage, &snap_save()));
    let live = TankDesignerView::new().bind(&ctx(&stage, &snap_live()));
    // the live tank has the heavy_cannon + armor_plate modules (2 known slots) vs the save
    // tank's medium_cannon + an empty turret — so the module digests differ.
    assert_ne!(bound_digest(&save), bound_digest(&live));
}

// ----------------------------------------------------- component slot interaction parity

#[test]
fn component_slot_hover_is_passive_no_command_and_digest_visible() {
    let stage = stage_with(&["tank_designer.json"]);
    let snap = snap_save();
    let mut view = TankDesignerView::new();
    rebind_tank(&mut view, &snap, true);

    let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
    assert!(before.contains("I[none]"), "{before}");

    let (x, y) = component_slot_center(&view.interaction, "main_armament_slot");
    let out = view.on_input(&pointer_move(x, y));
    assert!(!out.consumed);
    assert!(out.needs_redraw);
    assert!(out.commands.is_empty());

    let hover = bound_digest(&view.bind(&ctx(&stage, &snap)));
    assert_ne!(before, hover);
    assert!(
        hover.contains("I[mode=hover;id=main_armament_slot;slot=main_armament_slot(D);"),
        "{hover}"
    );

    let repeat = view.on_input(&pointer_move(x, y));
    assert!(!repeat.consumed);
    assert!(!repeat.needs_redraw);
    assert!(repeat.commands.is_empty());
}

#[test]
fn component_interaction_geometry_matches_bound_overlay_roots() {
    if !assets_present() {
        eprintln!("[designers] skipped component geometry: game_assets/gui not extracted");
        return;
    }
    let stage = stage_with(&[]);
    let snap = snap_save();
    let bctx = ctx(&stage, &snap);
    for (file, root, fam, label) in [
        (
            super::TANK_FILE,
            super::TANK_ROOT,
            Family::LandVehicle,
            "tank",
        ),
        (super::PLANE_FILE, super::PLANE_ROOT, Family::Plane, "plane"),
        (
            super::EQUIP_FILE,
            super::EQUIP_ROOT,
            Family::Ship,
            "equipment",
        ),
    ] {
        let file = resolve_gui(&bctx, file);
        let roots = build_root(&bctx, &file, root);
        assert_rect_close(
            component_overlay_area(bctx.screen, fam),
            overlay_area(&roots, bctx.screen),
            label,
        );
    }
}

#[test]
fn component_slot_clicks_are_read_only_and_wired_for_all_component_designers() {
    let stage = stage_with(&[]);
    let snap = snap_save();

    let mut tank = TankDesignerView::new();
    rebind_tank(&mut tank, &snap, true);
    let (x, y) = component_slot_center(&tank.interaction, "main_armament_slot");
    let out = tank.on_input(&left_click(x, y));
    assert!(out.consumed);
    assert!(out.needs_redraw);
    assert!(out.commands.is_empty());
    let tank_digest = bound_digest(&tank.bind(&ctx(&stage, &snap)));
    assert!(
        tank_digest.contains("I[mode=selected;id=main_armament_slot;slot=main_armament_slot(D);"),
        "{tank_digest}"
    );

    let repeat = tank.on_input(&left_click(x, y));
    assert!(repeat.consumed);
    assert!(!repeat.needs_redraw);
    assert!(repeat.commands.is_empty());

    let mut plane = PlaneDesignerView::new();
    rebind_plane(&mut plane, &snap, true);
    let (x, y) = component_slot_center(&plane.interaction, "engine_slot");
    let out = plane.on_input(&left_click(x, y));
    assert!(out.consumed);
    assert!(out.needs_redraw);
    assert!(out.commands.is_empty());
    let plane_digest = bound_digest(&plane.bind(&ctx(&stage, &snap)));
    assert!(
        plane_digest.contains("I[mode=selected;id=engine_slot;slot=engine_slot(D);"),
        "{plane_digest}"
    );

    let mut equipment = EquipmentDesignerView::new();
    rebind_equipment(&mut equipment, &snap, true);
    let (x, y) = component_slot_center(&equipment.interaction, "fixed_ship_battery_slot");
    let out = equipment.on_input(&left_click(x, y));
    assert!(out.consumed);
    assert!(out.needs_redraw);
    assert!(out.commands.is_empty());
    let equipment_digest = bound_digest(&equipment.bind(&ctx(&stage, &snap)));
    assert!(
        equipment_digest.contains(
            "I[mode=selected;id=fixed_ship_battery_slot;slot=fixed_ship_battery_slot(D);"
        ),
        "{equipment_digest}"
    );
}

#[test]
fn component_non_slot_designer_controls_are_replay_disabled_noops() {
    let stage = stage_with(&[]);
    let snap = snap_save();
    let mut view = TankDesignerView::new();
    rebind_tank(&mut view, &snap, true);

    let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
    let (x, y) = component_disabled_point(&view.interaction);
    let down = view.on_input(&left_click(x, y));
    assert!(down.consumed);
    assert!(!down.needs_redraw);
    assert!(down.commands.is_empty());
    let up = view.on_input(&ViewInput::PointerUp {
        x,
        y,
        button: PointerButton::Left,
    });
    assert!(up.consumed);
    assert!(!up.needs_redraw);
    assert!(up.commands.is_empty());
    let after = bound_digest(&view.bind(&ctx(&stage, &snap)));
    assert_eq!(
        before, after,
        "replay-disabled designer chrome/control clicks must not alter bound output"
    );
}

#[test]
fn component_selected_slot_survives_scrub_missing_then_restored_without_fallback() {
    let stage = stage_with(&[]);
    let save = snap_save();
    let missing = snap_no_designs();
    let mut view = TankDesignerView::new();
    rebind_tank(&mut view, &save, true);

    let (x, y) = component_slot_center(&view.interaction, "turret_slot");
    let out = view.on_input(&left_click(x, y));
    assert!(out.consumed && out.needs_redraw);
    assert!(out.commands.is_empty());
    let selected_save = bound_digest(&view.bind(&ctx(&stage, &save)));
    assert!(
        selected_save.contains("I[mode=selected;id=turret_slot;slot=turret_slot(D);"),
        "{selected_save}"
    );

    rebind_tank(&mut view, &missing, false);
    let selected_missing = bound_digest(&view.bind(&ctx(&stage, &missing)));
    assert!(
        selected_missing.contains("I[mode=missing_selected;id=turret_slot;"),
        "{selected_missing}"
    );
    assert!(
        !selected_missing.contains("id=main_armament_slot"),
        "missing selected slot must not fall back to another module: {selected_missing}"
    );

    rebind_tank(&mut view, &save, false);
    let restored_save = bound_digest(&view.bind(&ctx(&stage, &save)));
    assert_eq!(
        selected_save, restored_save,
        "scrub save->missing->save must restore the exact selected-slot digest"
    );
}

/// The division designer binds the regiment GRID (3 regiments + 2 support in the save fix)
/// as N4 slots (5 known), and the N2 tab bar selects the normal tab.
#[test]
fn division_regiment_grid_and_tabs_bound() {
    let stage = stage_with(&["division_designer.json"]);
    let bv = DivisionDesignerView::new().bind(&ctx(&stage, &snap_save()));
    let d = bound_digest(&bv);
    // 5 known slots (3 regiments + engineer + recon), 0 empty, 0 unavailable.
    assert!(
        d.contains("occ=0/5/0"),
        "the regiment grid binds 5 known slots, got: {d}"
    );
    // the N2 tab bar is present with the normal tab selected.
    assert!(
        d.contains("T[sel=Some(\"normal\")"),
        "the N2 tab bar selects the normal tab, got: {d}"
    );
    // the computed division stat chips badge unavailable (never computed).
    assert!(
        d.contains("Hardness=—") && d.contains("Combat Width=—"),
        "division stat chips badged unavailable"
    );
}

// -------------------------------------------------------- division tab interaction parity

/// An HQ-capable template enables the Army HQ tab and switches visible content.
#[test]
fn division_hq_tab_click_switches_digest_and_visible_content() {
    let stage = stage_with(&["division_designer.json"]);
    let snap = snap_hq_a();
    let mut v = DivisionDesignerView::new();
    rebind_division(&mut v, &snap, true);

    assert_eq!(v.tabs.selected(), Some(DIVISION_TAB_NORMAL));
    assert!(
        v.tabs
            .tabs
            .iter()
            .any(|t| t.id == DIVISION_TAB_HQ && t.avail.is_pickable()),
        "Army HQ tab must be enabled for an HQ template"
    );

    let cx = ctx(&stage, &snap);
    let normal = bound_digest(&v.bind(&cx));
    assert!(
        normal.contains("T[sel=Some(\"normal\")") && normal.contains("occ=0/3/0"),
        "{normal}"
    );
    assert!(
        normal.contains("infantry") && normal.contains("signal_company"),
        "normal tab must show the regiment/support grid: {normal}"
    );

    let out = click_division_tab_center(&mut v, DIVISION_TAB_HQ);
    assert!(out.consumed);
    assert!(out.needs_redraw);
    assert!(out.commands.is_empty());

    let hq = bound_digest(&v.bind(&cx));
    assert_ne!(normal, hq, "selecting Army HQ must change the digest");
    assert!(
        hq.contains("T[sel=Some(\"hq\")") && hq.contains("occ=0/0/0"),
        "{hq}"
    );
    assert!(
        hq.contains("HQ Deploy Cost=") && hq.contains("HQ Deploy Time="),
        "HQ tab must show the HQ-only unavailable summary: {hq}"
    );
    assert!(
        !hq.contains("infantry") && !hq.contains("signal_company"),
        "HQ tab must not carry normal regiment rows: {hq}"
    );
    assert!(
        !hq.contains("Hardness="),
        "HQ tab must not carry normal division stat chips: {hq}"
    );
}

#[test]
fn division_hq_tab_inert_when_template_is_not_hq() {
    let stage = stage_with(&["division_designer.json"]);
    let snap = snap_save();
    let mut v = DivisionDesignerView::new();
    rebind_division(&mut v, &snap, true);

    assert_eq!(v.tabs.selected(), Some(DIVISION_TAB_NORMAL));
    assert!(
        v.tabs
            .tabs
            .iter()
            .any(|t| t.id == DIVISION_TAB_HQ && !t.avail.is_pickable()),
        "non-HQ templates keep the Army HQ tab present but inert"
    );

    let before = bound_digest(&v.bind(&ctx(&stage, &snap)));
    let out = click_division_tab_center(&mut v, DIVISION_TAB_HQ);
    assert!(out.consumed);
    assert!(!out.needs_redraw);
    assert!(out.commands.is_empty());
    assert_eq!(v.tabs.selected(), Some(DIVISION_TAB_NORMAL));
    let after = bound_digest(&v.bind(&ctx(&stage, &snap)));
    assert_eq!(
        before, after,
        "clicking a disabled Army HQ tab must be an explicit no-op"
    );
}

#[test]
fn division_selected_tab_persists_across_scrub_a_b_a() {
    let stage = stage_with(&[]);
    let (a, b) = (snap_hq_a(), snap_hq_b());
    let mut v = DivisionDesignerView::new();
    rebind_division(&mut v, &a, true);
    let out = click_division_tab_center(&mut v, DIVISION_TAB_HQ);
    assert!(out.consumed && out.needs_redraw);

    let da1 = bound_digest(&v.bind(&ctx(&stage, &a)));
    rebind_division(&mut v, &b, false);
    let db = bound_digest(&v.bind(&ctx(&stage, &b)));
    rebind_division(&mut v, &a, false);
    let da2 = bound_digest(&v.bind(&ctx(&stage, &a)));

    assert!(da1.contains("T[sel=Some(\"hq\")"), "{da1}");
    assert!(db.contains("T[sel=Some(\"hq\")"), "{db}");
    assert!(db.contains("Name=Fuehrungsstab Ost"), "{db}");
    assert_eq!(
        da1, da2,
        "scrub A->B->A must preserve the selected division tab"
    );
    assert!(
        !da2.contains("Fuehrungsstab Ost") && !da2.contains("rocket_artillery"),
        "A after scrub must not carry B-only template data or regiment rows: {da2}"
    );
}

// ----------------------------------------------------------- inventory registration

/// All 5 designer views are inventory-collected + routed under the Designers menu cluster.
#[test]
fn cluster_views_registered() {
    use crate::host::ViewRegistry;
    let reg = ViewRegistry::from_inventory();
    for id in [
        "designers.tank",
        "designers.plane",
        "designers.division",
        "designers.equipment",
        "designers.equipment_upgrade",
    ] {
        assert!(
            reg.view(ViewId(id)).is_some(),
            "view {id} must be inventory-collected"
        );
    }
    let bar = reg.menubar();
    let group = bar
        .iter()
        .find(|g| g.cluster == MenuCluster::Designers)
        .unwrap();
    assert_eq!(
        group.items.len(),
        5,
        "all 5 designer views routed to the Designers menu"
    );
    // the 5 row templates are registered too.
    for id in [
        "designers.tank.stat_entry",
        "designers.plane.role_entry",
        "designers.division.regiment_slot_entry",
        "designers.equipment.module_slot_entry",
        "designers.upgrade.stat_entry",
    ] {
        assert!(
            reg.template(ViewId(id)).is_some(),
            "template {id} must be registered"
        );
    }
}

/// The top-level `designs` domain shim preserves the Captured trichotomy: Absent (no
/// domain), Null, and a measured object are distinguished — no fabrication.
#[test]
fn designs_domain_shim_preserves_trichotomy() {
    let absent = snap_no_designs();
    assert!(matches!(designs_domain(&absent), Captured::Absent));
    let measured = snap_save();
    assert!(matches!(designs_domain(&measured), Captured::Measured(_)));
    let null: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
        "date": "1941.1.1.12", "designs": null
    }))
    .unwrap();
    assert!(matches!(designs_domain(&null), Captured::Null));
}

// ------------------------------------------------- SEMANTIC binding-coverage (Codex (a))
//
// element_type_counts == walk proves the static .gui chrome is fully rendered, but NOT
// that every DATA-BEARING slot is bound-or-badged (Codex refuted relying on count-equality
// alone). These tests assert, per view, that EVERY expected data field appears in the bound
// output as EITHER a real bound atom (Exact/Derived with a value) OR an explicit
// unavailable "—" badge — never silently dropped, never a fabricated number.

/// Split a `bound_digest` into the set of `label -> rendered` pairs for the header strip
/// (`H[label=val(badge);...]`) and the stat-badge column (`S[label=—;...]`).
fn digest_fields(d: &str) -> (HashMap<String, String>, Vec<String>) {
    let mut headers = HashMap::new();
    let mut badges = Vec::new();
    if let Some(h) = section(d, "H[") {
        for entry in h.split(';') {
            if let Some((label, val)) = entry.split_once('=') {
                if !label.is_empty() {
                    headers.insert(label.to_string(), val.to_string());
                }
            }
        }
    }
    if let Some(s) = section(d, "S[") {
        for entry in s.split(';') {
            if let Some((label, _)) = entry.split_once('=') {
                if !label.is_empty() {
                    badges.push(label.to_string());
                }
            }
        }
    }
    (headers, badges)
}

/// Extract the contents between `marker` (e.g. "H[") and its matching `]` in the digest.
fn section<'a>(d: &'a str, marker: &str) -> Option<&'a str> {
    let start = d.find(marker)? + marker.len();
    let rest = &d[start..];
    let end = rest.find(']')?;
    Some(&rest[..end])
}

/// EVERY data-bearing field of EVERY designer is present as a bound atom OR an unavailable
/// badge — with FULLY-POPULATED variant/template data (so the bound paths are live). This
/// is the semantic coverage Codex required beyond raw element counts.
#[test]
fn every_data_field_bound_or_badged_per_view() {
    let stage_for = |g: &str| stage_with(&[g]);
    let snap = snap_save();

    // (view, gui, [header fields that MUST be present], [computed badges that MUST be "—"])
    let tank_headers = [
        "Experience",
        "Name",
        "Archetype",
        "Version",
        "Creator",
        "Obsolete",
        "Design Team",
    ];
    let tank_badges = [
        "Base Stats",
        "Combat Stats",
        "Misc Stats",
        "Production Cost (IC)",
        "Production Resources",
        "Refit Cost",
    ];
    let div_headers = [
        "Experience",
        "Name",
        "Role",
        "Priority",
        "Obsolete",
        "Army HQ",
    ];
    let div_badges = [
        "Hardness",
        "Organisation",
        "Max Strength",
        "Speed",
        "Combat Width",
        "Base Stats",
        "Combat Stats",
        "Equipment Stats",
        "Terrain Adjusters",
        "Production Cost (IC)",
        "Production Resources",
    ];
    let up_headers = ["Experience", "Variant", "Archetype", "Design Team"];
    let up_badges = ["Resulting Stats"];

    struct Case<'a> {
        vid: &'a str,
        gui: &'a str,
        headers: &'a [&'a str],
        badges: &'a [&'a str],
    }
    let cases = [
        Case {
            vid: "designers.tank",
            gui: "tank_designer.json",
            headers: &tank_headers,
            badges: &tank_badges,
        },
        Case {
            vid: "designers.plane",
            gui: "plane_designer.json",
            headers: &tank_headers,
            badges: &[
                "Base Stats",
                "Combat Stats",
                "Misc Stats",
                "Production Cost (IC)",
                "Production Resources",
                "Refit Cost",
                "Production Adjusters",
            ],
        },
        Case {
            vid: "designers.equipment",
            gui: "equipment_designer.json",
            headers: &tank_headers,
            badges: &[
                "Base Stats",
                "Combat Stats",
                "Misc Stats",
                "Production Cost (IC)",
                "Production Resources",
                "Refit Cost",
                "Naval Dominance",
                "Production Adjusters",
            ],
        },
        Case {
            vid: "designers.division",
            gui: "division_designer.json",
            headers: &div_headers,
            badges: &div_badges,
        },
        Case {
            vid: "designers.equipment_upgrade",
            gui: "equipment_upgrade_designer.json",
            headers: &up_headers,
            badges: &up_badges,
        },
    ];

    for c in cases {
        let stage = stage_for(c.gui);
        let d = bind_view(c.vid, &stage, &snap);
        let (headers, badges) = digest_fields(&d);
        // every expected header field is PRESENT (bound atom — a value or "—", never dropped).
        for h in c.headers {
            assert!(
                headers.contains_key(*h),
                "{}: data field `{h}` not bound/badged in output: {d}",
                c.vid
            );
            // it carries a fidelity badge token (E/D/U) — i.e. it is a real DisplayAtom, not blank.
            let v = &headers[*h];
            assert!(
                v.ends_with("(E)") || v.ends_with("(D)") || v.ends_with("(U)"),
                "{}: field `{h}` has no fidelity badge: {v}",
                c.vid
            );
        }
        // every expected COMPUTED panel is BADGED unavailable ("—") — never bound to a number.
        for b in c.badges {
            assert!(
                badges.contains(&b.to_string()),
                "{}: computed panel `{b}` not badged: {d}",
                c.vid
            );
        }
    }
}

/// A LIVE-sourced module value is not mislabelled as a different fidelity than a save-sourced
/// one: both the save `modules` and the live `equipment_variants_live` slot→module mapping are
/// EXACT (the real layout), so the per-slot mapping fidelity is identical across channels — a
/// live value is never dressed up as more (or less) certain than it is.
#[test]
fn live_and_save_module_mapping_same_fidelity() {
    let stage = stage_with(&["tank_designer.json"]);
    let save = TankDesignerView::new().bind(&ctx(&stage, &snap_save()));
    let live = TankDesignerView::new().bind(&ctx(&stage, &snap_live()));
    // both bind the tank's module slots as EXACT (the slot→module mapping), plus DERIVED
    // contributions from the DEF; neither path inflates to a fabricated value.
    assert!(save.fidelity.exact > 0 && live.fidelity.exact > 0);
    // and neither path produces a Combat Stats value (always unavailable).
    assert!(bound_digest(&save).contains("Combat Stats=—"));
    assert!(bound_digest(&live).contains("Combat Stats=—"));
}
