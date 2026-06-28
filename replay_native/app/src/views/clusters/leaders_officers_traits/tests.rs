//! Leaders / officers / traits cluster tests:
//!   * BUNDLE-MANIFEST element accounting (every declared `.gui` root classified exactly
//!     once + the summed element_type_counts == the raw walk — Codex fix #2),
//!   * render-plan digest scrub A→B→A byte-identity (immutable-LRU),
//!   * no-fabrication (FOLD-IN domains render "—", never a 0; the no-leader trait tree
//!     carries an Unavailable glyph not a fabricated Available tree; spirit tiles DERIVE
//!     from `politics.ideas_other[]`),
//!   * real-`.gui` root resolution (every routed root resolves to a populated scene),
//!   * inventory registration + menu routing.
//!
//! The save-only demo has NO `commanders`/`extra`/`order_of_battle` domain, so two
//! SYNTHETIC snapshots (a FOLDED live channel + the unfed demo) exercise the bound paths.

use super::accounting::{RootRole, ALL_MANIFESTS, ROUTED_ROOTS};
use super::aces::AcesView;
use super::leader_groups::LeaderGroupsView;
use super::navy_leader::NavyLeaderView;
use super::officer_corps::OfficerCorpsView;
use super::trait_trees::{trait_tree_viewport_from_roots, ArmyTraitTreeView, NavyTraitTreeView};
use super::unit_leader::UnitLeaderView;

use crate::host::context::{BindContext, Stage};
use crate::host::input::{PointerButton, ViewInput};
use crate::host::registry::{MenuCluster, ViewId, ViewRegistry};
use crate::host::view::{InputOutcome, RebindContext, ViewInstance};
use crate::views::digest::bound_digest;
use gui_runtime::{GuiFile, Loc, Rect};
use std::collections::HashMap;
use std::path::Path;

// ----------------------------------------------------------------------------- fixtures

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
    gui_dir().join("officer_corps.json").exists()
}

fn load_gui(name: &str) -> Option<GuiFile> {
    let p = gui_dir().join(name);
    std::fs::read_to_string(&p)
        .ok()
        .and_then(|s| GuiFile::parse(&s).ok())
}

fn stage_with(files: &[&str]) -> Stage {
    stage_with_manifest(files, HashMap::new())
}

fn stage_with_manifest(files: &[&str], sprites: HashMap<String, assets::SpriteEntry>) -> Stage {
    let mut gui_files = HashMap::new();
    for f in files {
        if let Some(g) = load_gui(f) {
            gui_files.insert(f.to_string(), g);
        }
    }
    Stage {
        resolver: assets::resolver::GameResolver::new(None, None),
        manifest: assets::SpriteManifest { sprites },
        loc: Loc::new(),
        gui_files,
        map_dir: std::path::PathBuf::from("."),
        // point assets_dir at the real gui parent so the lenient fallback works too.
        assets_dir: gui_dir().parent().unwrap().to_path_buf(),
    }
}

fn sprite_entry(name: &str, texturefile: Option<&str>, frames: u32) -> assets::SpriteEntry {
    assets::SpriteEntry {
        name: name.to_string(),
        kind: "spriteType".to_string(),
        no_of_frames: frames,
        source_gfx: None,
        texturefile: texturefile.map(str::to_string),
        category: None,
        width: None,
        height: None,
        nine_slice: None,
        size: None,
        border_size: None,
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

/// The SAVE-ONLY demo snapshot: politics folded (the spirit set IS present), but
/// `commanders`/`extra`/`order_of_battle` UNFED — every leader/ace/fleet bind must "—".
fn snap_demo() -> snapshot::Snapshot {
    serde_json::from_value(serde_json::json!({
        "date": "1941.1.1.12", "hour_index": 100,
        "countries": { "GER": {
            "politics": { "ideas_other": ["officer_corps_spirit_a", {"token": "spirit_b"}] }
        } }
    }))
    .unwrap()
}

/// A FOLDED live snapshot: `commanders` (generals/admirals with traits), `extra.aces`, and
/// `order_of_battle.fleets`/`armies` populated — exercises the bound (exact) paths.
fn snap_live() -> snapshot::Snapshot {
    serde_json::from_value(serde_json::json!({
        "date": "1942.1.1.12", "hour_index": 200,
        "countries": { "GER": {
            "politics": { "ideas_other": ["officer_corps_spirit_a"] },
            "commanders": {
                "leader": { "name": "Adolf Hitler" },
                "advisors": [ { "name": "Keitel", "slot": "high_command" } ],
                "generals": [
                    { "name": "Guderian", "role": "general", "level": 6, "attack": 5,
                      "defense": 3, "planning": 4, "logistics": 2,
                      "traits": ["panzer_leader", "trickster"] }
                ],
                "field_marshals": [
                    { "name": "Rundstedt", "role": "field_marshal", "level": 7, "attack": 4,
                      "defense": 5, "planning": 3, "logistics": 4, "traits": [] }
                ],
                "admirals": [
                    { "name": "Raeder", "role": "admiral", "level": 5, "attack": 4,
                      "defense": 3, "maneuvering": 5, "coordination": 4, "traits": ["seawolf"] }
                ]
            },
            "extra": {
                "aces": [
                    { "name": "Erich", "surname": "Hartmann", "callsign": "Bubi",
                      "modifier": "ace_bonus", "alive": true, "kill_type": "fighter" }
                ],
                "ace_count": 1
            }
        } },
        "order_of_battle": {
            "fleets": [ { "owner": "GER", "task_forces": [ {}, {} ] } ],
            "armies": [ { "leader": "x", "attached_air_wings": [ {} ] } ]
        }
    }))
    .unwrap()
}

fn sprite_manifest(entries: &[(&str, &str, u32)]) -> HashMap<String, assets::SpriteEntry> {
    entries
        .iter()
        .map(|(name, texture, frames)| {
            (
                (*name).to_string(),
                sprite_entry(name, Some(texture), *frames),
            )
        })
        .collect()
}

fn snap_with_general(general: serde_json::Value) -> snapshot::Snapshot {
    serde_json::from_value(serde_json::json!({
        "date": "1942.1.1.12", "hour_index": 200,
        "countries": { "GER": {
            "commanders": {
                "generals": [general],
                "field_marshals": [],
                "admirals": [],
                "advisors": []
            }
        } }
    }))
    .unwrap()
}

// --------------------------------------------------- element accounting (bundle manifest)

/// Count `type` tokens recursively over the RAW gui json (the authoritative element set).
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

/// The set of declared TOP-LEVEL window-root names in a `.gui` file.
fn declared_roots(name: &str) -> Vec<String> {
    let p = gui_dir().join(name);
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    match v.get("windows").and_then(|w| w.as_array()) {
        Some(ws) => ws
            .iter()
            .filter_map(|w| w.get("name").and_then(|n| n.as_str()).map(String::from))
            .collect(),
        None => Vec::new(),
    }
}

/// Every element of each `.gui` is accounted (walk == declared counts) — file-integrity.
#[test]
fn element_accounting_all_eight() {
    if !assets_present() {
        eprintln!("[leaders] skipped element_accounting: game_assets/gui not extracted");
        return;
    }
    for (file, _) in ALL_MANIFESTS {
        assert_eq!(
            raw_type_counts(file),
            declared_counts(file),
            "element accounting mismatch in {file}"
        );
    }
}

/// The real per-file element totals (a drift guard) — matches the binding map's verified set.
#[test]
fn element_totals_actual() {
    if !assets_present() {
        eprintln!("[leaders] skipped totals: game_assets/gui not extracted");
        return;
    }
    let total = |n: &str| raw_type_counts(n).values().sum::<usize>();
    assert_eq!(total("officer_corps.json"), 333);
    assert_eq!(total("unit_leader.json"), 305);
    assert_eq!(total("navy_leader.json"), 25);
    assert_eq!(total("leader_groups.json"), 82);
    assert_eq!(total("aces.json"), 19);
    assert_eq!(total("army_trait_tree.json"), 90);
    assert_eq!(total("navy_trait_tree.json"), 74);
    assert_eq!(total("trait_tree_common.json"), 56);
}

/// BUNDLE-MANIFEST coverage (Codex fix #2): for EVERY file, the cluster's manifest
/// classifies EXACTLY the declared root set — `covered_root_ids == all_declared_root_ids`
/// — so no root is orphaned (a multi-root file like officer_corps's 31 roots is fully
/// reachable) and no manifest entry names a phantom root.
#[test]
fn every_declared_root_classified() {
    if !assets_present() {
        eprintln!("[leaders] skipped manifest coverage: game_assets/gui not extracted");
        return;
    }
    for (file, manifest) in ALL_MANIFESTS {
        let declared: std::collections::BTreeSet<String> =
            declared_roots(file).into_iter().collect();
        let covered: std::collections::BTreeSet<String> =
            manifest.iter().map(|e| e.name.to_string()).collect();
        assert_eq!(
            covered, declared,
            "{file}: manifest must classify EXACTLY the declared roots (orphan/phantom check)"
        );
        // every entry has a real role (exhaustive — no unclassified root).
        for e in *manifest {
            assert!(
                matches!(
                    e.role,
                    RootRole::WindowRoot
                        | RootRole::RowTemplate
                        | RootRole::Prototype
                        | RootRole::AltSurface
                        | RootRole::Gap
                ),
                "{file}: root {} unclassified",
                e.name
            );
        }
    }
}

/// ROUTED-ROOT COVERAGE INVARIANT (Codex (a)): EVERY `RootRole::WindowRoot` across all
/// manifests has a real render owner — it is in `ROUTED_ROOTS` (one of the 7 routed views'
/// `bind`-rendered roots). So a window root can NEVER be classified covered while being
/// unrendered dead UI surface. Conversely every `ROUTED_ROOTS` entry IS a manifest
/// `WindowRoot` (no phantom render owner).
#[test]
fn window_roots_are_rendered() {
    let mut declared_window_roots: std::collections::BTreeSet<&str> =
        std::collections::BTreeSet::new();
    for (_file, manifest) in ALL_MANIFESTS {
        for e in *manifest {
            if e.role == RootRole::WindowRoot {
                assert!(
                    ROUTED_ROOTS.contains(&e.name),
                    "window-root `{}` is classified WindowRoot but has NO render owner in ROUTED_ROOTS (would be dead surface)",
                    e.name
                );
                declared_window_roots.insert(e.name);
            }
        }
    }
    // every routed root is a manifest WindowRoot (no phantom render owner).
    for r in ROUTED_ROOTS {
        assert!(
            declared_window_roots.contains(r),
            "ROUTED_ROOTS entry `{r}` is not a manifest WindowRoot"
        );
    }
    // exactly the 7 routed views.
    assert_eq!(
        declared_window_roots.len(),
        7,
        "exactly the 7 routed window roots are rendered"
    );
}

/// Every ROUTED window-root resolves to a REAL, POPULATED `.gui` scene-graph (via the
/// cluster's `load_gui` Stage-first / lenient fallback) — proving each view renders the REAL
/// game layout, not synthetic chrome. Forced through the fallback with an EMPTY stage too.
#[test]
fn every_routed_root_resolves_to_real_gui() {
    if !assets_present() {
        eprintln!("[leaders] skipped root-resolves: game_assets/gui not extracted");
        return;
    }
    let stage = stage_with(&[]); // empty → force the lenient fallback path
    let snap = snap_demo();
    let bctx = ctx(&stage, &snap);
    // (file, root) for the 7 routed roots the views render.
    let cases = [
        ("officer_corps.json", "countryofficercorpview"),
        ("unit_leader.json", "armyleaderwindow"),
        ("navy_leader.json", "navy_leader_window"),
        ("leader_groups.json", "leader_groups_portrait_window"),
        ("aces.json", "acesview"),
        ("army_trait_tree.json", "unitleader_trait_tree_window"),
        ("navy_trait_tree.json", "navyleader_trait_tree_window"),
    ];
    for (file, root) in cases {
        let g = super::load_gui(&bctx, file);
        let w = g.windows.iter().find(|w| w.name.as_deref() == Some(root));
        assert!(
            w.is_some(),
            "root {root} must resolve in {file} (real .gui)"
        );
        let node = gui_runtime::scene::Node::from_raw(w.unwrap(), &stage.manifest, &stage.loc);
        assert!(
            node.count() > 3,
            "the real {root} subtree must be populated, not an empty fallback"
        );
    }
}

// ----------------------------------------------------------- render-plan scrub A→B→A

fn bind_digest(view_id: &str, stage: &Stage, snap: &snapshot::Snapshot) -> String {
    let bv = match view_id {
        "leaders.officer_corps" => OfficerCorpsView::new().bind(&ctx(stage, snap)),
        "leaders.unit_leader" => UnitLeaderView::new().bind(&ctx(stage, snap)),
        "leaders.navy_leader" => NavyLeaderView::new().bind(&ctx(stage, snap)),
        "leaders.leader_groups" => LeaderGroupsView::new().bind(&ctx(stage, snap)),
        "leaders.aces" => AcesView::new().bind(&ctx(stage, snap)),
        "leaders.army_trait_tree" => ArmyTraitTreeView::new().bind(&ctx(stage, snap)),
        "leaders.navy_trait_tree" => NavyTraitTreeView::new().bind(&ctx(stage, snap)),
        _ => unreachable!(),
    };
    bound_digest(&bv)
}

/// Each of the 7 routed views: bind A (live) and B (demo) differ, and a scrub A→B→A rebinds
/// A byte-identically (the immutable-LRU rule — no value carried from the B visit).
fn rebind_trait_view(view: &mut dyn ViewInstance, snap: &snapshot::Snapshot) {
    view.rebind(&RebindContext {
        snapshot: snap,
        country: "GER",
        screen: (1600, 900),
        refit: true,
    });
}

fn sync_army_trait_input(view: &mut ArmyTraitTreeView, stage: &Stage, snap: &snapshot::Snapshot) {
    rebind_trait_view(view, snap);
    let bv = view.bind(&ctx(stage, snap));
    view.set_input_viewport(trait_tree_viewport_from_roots(&bv.hit.roots));
}

fn sync_navy_trait_input(view: &mut NavyTraitTreeView, stage: &Stage, snap: &snapshot::Snapshot) {
    rebind_trait_view(view, snap);
    let bv = view.bind(&ctx(stage, snap));
    view.set_input_viewport(trait_tree_viewport_from_roots(&bv.hit.roots));
}

fn click_at(view: &mut dyn ViewInstance, x: f32, y: f32) -> (InputOutcome, InputOutcome) {
    let down = view.on_input(&ViewInput::PointerDown {
        x,
        y,
        button: PointerButton::Left,
    });
    let up = view.on_input(&ViewInput::PointerUp {
        x,
        y,
        button: PointerButton::Left,
    });
    (down, up)
}

fn drag_from(view: &mut dyn ViewInstance, x: f32, y: f32) -> InputOutcome {
    let _ = view.on_input(&ViewInput::PointerDown {
        x,
        y,
        button: PointerButton::Left,
    });
    let move_out = view.on_input(&ViewInput::PointerMove {
        x: x + 8.0,
        y: y + 6.0,
        drag: true,
    });
    let _ = view.on_input(&ViewInput::PointerUp {
        x: x + 8.0,
        y: y + 6.0,
        button: PointerButton::Left,
    });
    move_out
}

fn hover_at(view: &mut dyn ViewInstance, x: f32, y: f32) -> InputOutcome {
    view.on_input(&ViewInput::PointerMove { x, y, drag: false })
}

fn rect_center(rect: Rect) -> (f32, f32) {
    (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0)
}

fn assert_add_trait_targets_match_roots(
    targets: &[(&'static str, Rect)],
    roots: &[gui_runtime::scene::Node],
) {
    let expected = ["add_trait_0", "add_trait_1", "add_trait_2", "add_trait_3"];
    assert_eq!(
        targets.iter().map(|(leaf, _)| *leaf).collect::<Vec<_>>(),
        expected,
        "only the four real add-trait leaves are covered by this disabled-mutation slice"
    );
    for (leaf, cached) in targets {
        let rendered =
            super::find_rect(roots, leaf).unwrap_or_else(|| panic!("{leaf} rendered rect"));
        assert!(
            (cached.x - rendered.x).abs() < 0.01
                && (cached.y - rendered.y).abs() < 0.01
                && (cached.w - rendered.w).abs() < 0.01
                && (cached.h - rendered.h).abs() < 0.01,
            "{leaf} disabled hit rect must match the bound real .gui rect: cached={cached:?} rendered={rendered:?}"
        );
    }
}

#[test]
fn scrub_a_b_a_byte_identical_all_seven() {
    if !assets_present() {
        eprintln!("[leaders] skipped scrub: game_assets/gui not extracted");
        return;
    }
    let cases = [
        ("leaders.officer_corps", "officer_corps.json"),
        ("leaders.unit_leader", "unit_leader.json"),
        ("leaders.navy_leader", "navy_leader.json"),
        ("leaders.leader_groups", "leader_groups.json"),
        ("leaders.aces", "aces.json"),
        ("leaders.army_trait_tree", "army_trait_tree.json"),
        ("leaders.navy_trait_tree", "navy_trait_tree.json"),
    ];
    let (a, b) = (snap_live(), snap_demo());
    for (vid, gui) in cases {
        let stage = stage_with(&[gui]);
        let da1 = bind_digest(vid, &stage, &a);
        let db = bind_digest(vid, &stage, &b);
        assert_ne!(
            da1, db,
            "{vid}: live (A) and demo (B) must differ, else the scrub proves nothing"
        );
        let da2 = bind_digest(vid, &stage, &a);
        assert_eq!(
            da1, da2,
            "{vid}: scrub A→B→A carried a value from the B visit"
        );
    }
}

// --------------------------------------------------------------------- no-fabrication

/// In the save-only demo (commanders/extra/oob unfed) every leader/ace/fleet roster section
/// badges absent + renders "—" — NEVER a fabricated empty-leader with skill 0 (Codex #3).
#[test]
fn resolved_portrait_sprite_records_exact_digest() {
    if !assets_present() {
        eprintln!("[leaders] skipped portrait-resolved: game_assets/gui not extracted");
        return;
    }
    let stage = stage_with_manifest(
        &["unit_leader.json"],
        sprite_manifest(&[(
            "GFX_portrait_GER_heinz_guderian",
            "gfx/leaders/GER/Portrait_Germany_Heinz_Guderian.dds",
            1,
        )]),
    );
    let snap = snap_with_general(serde_json::json!({
        "name": "Guderian", "role": "general", "level": 6,
        "attack": 5, "defense": 3, "planning": 4, "logistics": 2,
        "traits": ["panzer_leader"],
        "portrait_gfx": "GFX_portrait_GER_heinz_guderian"
    }));
    let bv = UnitLeaderView::new().bind(&ctx(&stage, &snap));
    let d = bound_digest(&bv);
    assert!(
        d.contains("leader_button:sprite=GFX_portrait_GER_heinz_guderian#0(E);"),
        "resolved portrait must enter the row digest as exact sprite art, got:\n{d}"
    );
}

#[test]
fn missing_portrait_sprite_records_explicit_miss() {
    if !assets_present() {
        eprintln!("[leaders] skipped portrait-missing: game_assets/gui not extracted");
        return;
    }
    let stage = stage_with(&["unit_leader.json"]);
    let snap = snap_with_general(serde_json::json!({
        "name": "No Portrait", "role": "general", "level": 1,
        "attack": 1, "defense": 1, "planning": 1, "logistics": 1,
        "traits": [],
        "portrait_gfx": "GFX_portrait_missing"
    }));
    let d = bind_digest("leaders.unit_leader", &stage, &snap);
    assert!(
        d.contains("leader_button:sprite=MISS:missing_sprite:GFX_portrait_missing(U);"),
        "missing portrait must remain an explicit miss, got:\n{d}"
    );
    assert!(
        !d.contains("leader_button:sprite=GFX_leader_unknown#0(E);"),
        "generic leader placeholder must not count as exact portrait art"
    );
}

#[test]
fn generic_unknown_portrait_is_unavailable_not_exact_default() {
    if !assets_present() {
        eprintln!("[leaders] skipped portrait-placeholder: game_assets/gui not extracted");
        return;
    }
    let stage = stage_with_manifest(
        &["unit_leader.json"],
        sprite_manifest(&[("GFX_leader_unknown", "gfx/leaders/leader_unknown.dds", 1)]),
    );
    let snap = snap_with_general(serde_json::json!({
        "name": "Placeholder", "role": "general", "level": 1,
        "attack": 1, "defense": 1, "planning": 1, "logistics": 1,
        "traits": [],
        "portrait_gfx": "GFX_leader_unknown"
    }));
    let d = bind_digest("leaders.unit_leader", &stage, &snap);
    assert!(
        d.contains("leader_button:sprite=MISS:generic_placeholder:GFX_leader_unknown(U);"),
        "generic placeholder must be unavailable, got:\n{d}"
    );
    assert!(
        !d.contains("leader_button:sprite=GFX_leader_unknown#0(E);"),
        "generic placeholder must never be recorded as exact"
    );
}

#[test]
fn typed_leader_ref_does_not_join_wrong_same_id_portrait() {
    if !assets_present() {
        eprintln!("[leaders] skipped typed-ref-portrait: game_assets/gui not extracted");
        return;
    }
    let stage = stage_with_manifest(
        &["leader_groups.json"],
        sprite_manifest(&[(
            "GFX_portrait_wrong_field_marshal",
            "gfx/leaders/wrong.dds",
            1,
        )]),
    );
    let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
        "date": "1942.1.1.12", "hour_index": 200,
        "countries": { "GER": {
            "commanders": {
                "generals": [],
                "field_marshals": [
                    {
                        "name": "Wrong Same Id",
                        "unit_leader_id": "7",
                        "portrait_gfx": "GFX_portrait_wrong_field_marshal"
                    }
                ]
            }
        } },
        "order_of_battle": {
            "army_groups": [
                {"owner": "GER", "leader": {"id": "7", "type": "corps_commander"}}
            ]
        }
    }))
    .unwrap();
    let d = bind_digest("leaders.leader_groups", &stage, &snap);
    assert!(
        d.contains("leader_icon:sprite=MISS:no_sprite_name(U);"),
        "typed ref with no matching typed commander must fail closed, got:\n{d}"
    );
    assert!(
        !d.contains("GFX_portrait_wrong_field_marshal#0"),
        "typed ref must not attach a same-id portrait from the wrong commander type"
    );
}

#[test]
fn demo_foldin_domains_badge_never_fabricate() {
    if !assets_present() {
        eprintln!("[leaders] skipped foldin: game_assets/gui not extracted");
        return;
    }
    let snap = snap_demo();
    for (vid, gui, sect) in [
        (
            "leaders.unit_leader",
            "unit_leader.json",
            "UNITLEADER sect=absent",
        ),
        (
            "leaders.navy_leader",
            "navy_leader.json",
            "NAVYLEADER sect=absent",
        ),
        ("leaders.aces", "aces.json", "ACES sect=absent"),
    ] {
        let stage = stage_with(&[gui]);
        let d = bind_digest(vid, &stage, &snap);
        assert!(
            d.contains(sect),
            "{vid}: unfed roster must badge absent, got: {d}"
        );
        // no fabricated numeric leader/ace fields snuck in.
        assert!(!d.contains("skill=0("), "{vid}: no fabricated skill 0");
        assert!(!d.contains("name=0("), "{vid}: no fabricated name 0");
    }
}

/// The officer-corps SPIRIT tiles DERIVE from `politics.ideas_other[]` (folded in the demo)
/// — present even when `commanders` is unfed (the DERIVED-NOW path, distinct from FOLD-IN).
#[test]
fn officer_corps_spirits_derived_now() {
    if !assets_present() {
        eprintln!("[leaders] skipped spirits: game_assets/gui not extracted");
        return;
    }
    let stage = stage_with(&["officer_corps.json"]);
    let d = bind_digest("leaders.officer_corps", &stage, &snap_demo());
    // the spirit section is PRESENT (derived) with the two ideas_other tokens bound DERIVED.
    assert!(
        d.contains("spirit_sect=present"),
        "spirit tiles derive from ideas_other: {d}"
    );
    assert!(
        d.contains("officer_corps_spirit_a(D)"),
        "spirit token (string) bound derived: {d}"
    );
    assert!(
        d.contains("spirit_b(D)"),
        "spirit token (object form) bound derived: {d}"
    );
    // the chief roster (commanders) is still absent (unfed) — the two paths are distinct.
    assert!(
        d.contains("chief_sect=absent"),
        "chief roster stays absent (commanders unfed): {d}"
    );
}

/// The trait tree with NO selected leader (the demo) carries a single Unavailable glyph node
/// — NOT a fabricated Available tree and NOT a silently empty tree (Codex no-fab #4).
#[test]
fn trait_tree_no_leader_is_unavailable_not_fabricated() {
    if !assets_present() {
        eprintln!("[leaders] skipped trait-no-leader: game_assets/gui not extracted");
        return;
    }
    let stage = stage_with(&["army_trait_tree.json"]);
    let d = bind_digest("leaders.army_trait_tree", &stage, &snap_demo());
    // exactly one node is drawn (the no-leader glyph), and the header fields are "—".
    assert!(
        d.contains("ARMYTRAIT branch=army nodes=1"),
        "no-leader tree has one glyph node, got: {d}"
    );
    assert!(
        d.contains('—'),
        "the no-leader header binds the not-captured glyph"
    );
    // no fabricated skill/stat number in the header.
    assert!(
        !d.contains("skill=0("),
        "no fabricated skill 0 in the no-leader header"
    );
}

/// With a FOLDED leader the army trait tree binds the selected general's OWNED traits as N3
/// `Owned` nodes (membership exact) + the header stats exact — the live path works.
#[test]
fn trait_tree_folded_leader_binds_owned_traits() {
    if !assets_present() {
        eprintln!("[leaders] skipped trait-folded: game_assets/gui not extracted");
        return;
    }
    let stage = stage_with(&["army_trait_tree.json"]);
    let bv = ArmyTraitTreeView::new().bind(&ctx(&stage, &snap_live()));
    let d = bound_digest(&bv);
    // Guderian has two owned traits → two N3 nodes drawn.
    assert!(
        d.contains("ARMYTRAIT branch=army nodes=2"),
        "two owned-trait nodes for the folded leader, got: {d}"
    );
    // the header binds the real skill (6) + stats exact.
    assert!(
        bv.fidelity.exact > 0,
        "the folded leader's header fields are exact"
    );
    assert!(
        d.contains("Guderian"),
        "the folded leader's name is bound: {d}"
    );
}

/// The aces view binds a folded ace's name (name + surname + callsign) — and never
/// fabricates a blank for an absent ace name.
#[test]
fn army_trait_node_click_selects_owned_trait_detail() {
    if !assets_present() {
        eprintln!("[leaders] skipped army-trait-click: game_assets/gui not extracted");
        return;
    }
    let stage = stage_with(&["army_trait_tree.json"]);
    let snap = snap_live();
    let mut view = ArmyTraitTreeView::new();
    sync_army_trait_input(&mut view, &stage, &snap);
    let (x, y) = view
        .debug_node_center("panzer_leader")
        .expect("panzer_leader node center");

    let (down, up) = click_at(&mut view, x, y);
    assert!(down.consumed, "owned trait pointer-down must be captured");
    assert!(up.consumed, "owned trait pointer-up must select");
    assert!(up.needs_redraw, "first selection must redraw");

    let d = bound_digest(&view.bind(&ctx(&stage, &snap)));
    assert!(
        d.contains("selected=selected"),
        "selection state visible: {d}"
    );
    assert!(
        d.contains("selected_key=panzer_leader"),
        "selection key must be the exact trait token: {d}"
    );
    assert!(
        d.contains("Trait ID=text=\"panzer_leader\",fidelity=Exact"),
        "detail must include factual trait id: {d}"
    );
    assert!(
        d.contains("State=text=\"owned\",fidelity=Exact"),
        "detail must bind the exact owned state: {d}"
    );
    assert!(
        d.contains("Leader name=text=\"Guderian\",fidelity=Exact"),
        "detail must bind the measured leader name: {d}"
    );
    assert!(
        d.contains("Catalog type=text=\"corps_commander\",fidelity=Derived"),
        "detail must bind the static DEF trait scope as derived data: {d}"
    );
    assert!(
        d.contains("Cost=text=\"700\",fidelity=Derived"),
        "detail must bind the static DEF trait cost as derived data: {d}"
    );
    assert!(
        d.contains("Modifier rows=text=\"2\",fidelity=Derived"),
        "detail must count static DEF modifier rows without fabricating effects: {d}"
    );
    assert!(
        d.contains("Trait type=text=\"—\",fidelity=Unavailable,reason=Some(Null)"),
        "null trait_type in the DEF must stay unavailable/null, not become a fake label: {d}"
    );
}

#[test]
fn army_trait_node_hover_emits_factual_tooltip_and_clears_off_node() {
    if !assets_present() {
        eprintln!("[leaders] skipped army-trait-hover: game_assets/gui not extracted");
        return;
    }
    let stage = stage_with(&["army_trait_tree.json"]);
    let snap = snap_live();
    let mut view = ArmyTraitTreeView::new();
    sync_army_trait_input(&mut view, &stage, &snap);
    let (x, y) = view
        .debug_node_center("panzer_leader")
        .expect("panzer_leader node center");

    let out = hover_at(&mut view, x, y);
    assert!(
        out.needs_redraw,
        "hovering a trait node must redraw for tooltip"
    );
    let bv = view.bind(&ctx(&stage, &snap));
    assert!(
        bv.tooltip_plan().is_some(),
        "owned trait hover emits tooltip"
    );
    let d = bound_digest(&bv);
    assert!(
        d.contains("panzer_leader"),
        "tooltip must include trait id: {d}"
    );
    assert!(d.contains("owned"), "tooltip must include owned state: {d}");
    assert!(
        d.contains("Guderian"),
        "tooltip must include leader name: {d}"
    );
    assert!(
        d.contains("\"Catalog type\"") && d.contains("\"corps_commander\""),
        "tooltip must include static DEF trait scope: {d}"
    );
    assert!(
        d.contains("\"Modifier rows\"") && d.contains("\"2\""),
        "tooltip must include factual static modifier-row count: {d}"
    );

    let out = hover_at(&mut view, 1.0, 1.0);
    assert!(out.needs_redraw, "leaving the node must clear tooltip");
    let bv = view.bind(&ctx(&stage, &snap));
    assert!(
        bv.tooltip_plan().is_none(),
        "hovering outside the trait canvas clears tooltip"
    );
}

#[test]
fn navy_trait_node_click_selects_owned_trait_detail() {
    if !assets_present() {
        eprintln!("[leaders] skipped navy-trait-click: game_assets/gui not extracted");
        return;
    }
    let stage = stage_with(&["navy_trait_tree.json"]);
    let snap = snap_live();
    let mut view = NavyTraitTreeView::new();
    sync_navy_trait_input(&mut view, &stage, &snap);
    let (x, y) = view
        .debug_node_center("seawolf")
        .expect("seawolf node center");

    let (_down, up) = click_at(&mut view, x, y);
    assert!(up.consumed);
    let d = bound_digest(&view.bind(&ctx(&stage, &snap)));
    assert!(
        d.contains("selected=selected"),
        "selection state visible: {d}"
    );
    assert!(
        d.contains("selected_key=seawolf"),
        "selection key must be the exact navy trait token: {d}"
    );
    assert!(
        d.contains("Branch=text=\"navy\",fidelity=Exact"),
        "detail must badge the navy branch exactly: {d}"
    );
    assert!(
        d.contains("Leader name=text=\"Raeder\",fidelity=Exact"),
        "detail must bind the admiral name: {d}"
    );
}

#[test]
fn navy_trait_node_hover_emits_factual_tooltip() {
    if !assets_present() {
        eprintln!("[leaders] skipped navy-trait-hover: game_assets/gui not extracted");
        return;
    }
    let stage = stage_with(&["navy_trait_tree.json"]);
    let snap = snap_live();
    let mut view = NavyTraitTreeView::new();
    sync_navy_trait_input(&mut view, &stage, &snap);
    let (x, y) = view
        .debug_node_center("seawolf")
        .expect("seawolf node center");

    let out = hover_at(&mut view, x, y);
    assert!(
        !out.consumed,
        "trait hover is passive and must not consume input"
    );
    assert!(out.needs_redraw, "hovering a navy trait must redraw");
    let bv = view.bind(&ctx(&stage, &snap));
    assert!(
        bv.tooltip_plan().is_some(),
        "navy trait hover emits tooltip"
    );
    let d = bound_digest(&bv);
    assert!(d.contains("seawolf"), "tooltip must include trait id: {d}");
    assert!(d.contains("owned"), "tooltip must include owned state: {d}");
    assert!(d.contains("navy"), "tooltip must include navy branch: {d}");
    assert!(
        d.contains("Raeder"),
        "tooltip must include admiral name: {d}"
    );
    assert!(
        d.contains("\"Catalog type\"") && d.contains("\"navy\""),
        "tooltip must include the static DEF navy trait scope: {d}"
    );
    assert!(
        d.contains("\"Cost\"") && d.contains("\"700\""),
        "tooltip must include the static DEF trait cost: {d}"
    );
}

#[test]
fn unavailable_trait_glyph_hover_is_inert_not_tooltip() {
    if !assets_present() {
        eprintln!("[leaders] skipped trait-glyph-hover: game_assets/gui not extracted");
        return;
    }
    let stage = stage_with(&["army_trait_tree.json"]);
    let snap = snap_demo();
    let mut view = ArmyTraitTreeView::new();
    sync_army_trait_input(&mut view, &stage, &snap);
    let (x, y) = view
        .debug_node_center("no_leader")
        .expect("no_leader glyph center");

    let out = hover_at(&mut view, x, y);
    assert!(
        !out.needs_redraw,
        "unavailable glyph hover should not publish tooltip state"
    );
    let bv = view.bind(&ctx(&stage, &snap));
    assert!(
        bv.tooltip_plan().is_none(),
        "unavailable glyph hover must not invent a trait tooltip"
    );
}

#[test]
fn unavailable_trait_glyphs_are_inert_not_selectable() {
    if !assets_present() {
        eprintln!("[leaders] skipped trait-glyph-inert: game_assets/gui not extracted");
        return;
    }
    let stage = stage_with(&["army_trait_tree.json"]);
    let snap = snap_demo();
    let mut view = ArmyTraitTreeView::new();
    sync_army_trait_input(&mut view, &stage, &snap);
    let (x, y) = view
        .debug_node_center("no_leader")
        .expect("no_leader glyph center");

    let (down, up) = click_at(&mut view, x, y);
    assert!(
        !down.consumed && !up.consumed,
        "unavailable glyphs must not behave like selectable traits"
    );
    let d = bound_digest(&view.bind(&ctx(&stage, &snap)));
    assert!(
        d.contains("selected=none"),
        "glyph click must not select: {d}"
    );
    assert!(
        d.contains("selected_detail=none"),
        "glyph click must not invent detail: {d}"
    );

    let snap_no_traits = snap_with_general(serde_json::json!({
        "name": "Traitless", "role": "general", "level": 1,
        "attack": 1, "defense": 1, "planning": 1, "logistics": 1,
        "traits": []
    }));
    sync_army_trait_input(&mut view, &stage, &snap_no_traits);
    let (x, y) = view
        .debug_node_center("no_traits")
        .expect("no_traits glyph center");
    let (down, up) = click_at(&mut view, x, y);
    assert!(
        !down.consumed && !up.consumed,
        "no-traits glyph must also stay inert"
    );
    let d = bound_digest(&view.bind(&ctx(&stage, &snap_no_traits)));
    assert!(
        d.contains("selected=none"),
        "no-traits click must not select: {d}"
    );
    assert!(
        d.contains("selected_detail=none"),
        "no-traits click must not invent detail: {d}"
    );
}

#[test]
fn trait_node_drag_cancels_click_selection() {
    if !assets_present() {
        eprintln!("[leaders] skipped trait-drag-cancel: game_assets/gui not extracted");
        return;
    }
    let stage = stage_with(&["army_trait_tree.json"]);
    let snap = snap_live();
    let mut view = ArmyTraitTreeView::new();
    sync_army_trait_input(&mut view, &stage, &snap);
    let (x, y) = view
        .debug_node_center("panzer_leader")
        .expect("panzer_leader node center");

    let move_out = drag_from(&mut view, x, y);
    assert!(move_out.consumed, "drag stream belongs to the trait view");
    let d = bound_digest(&view.bind(&ctx(&stage, &snap)));
    assert!(
        d.contains("selected=none"),
        "drag movement must cancel the pending click: {d}"
    );
}

#[test]
fn trait_tree_add_trait_slots_are_replay_disabled_noops() {
    if !assets_present() {
        eprintln!("[leaders] skipped trait-add-slots-noop: game_assets/gui not extracted");
        return;
    }

    let snap = snap_live();

    let army_stage = stage_with(&["army_trait_tree.json"]);
    let mut army = ArmyTraitTreeView::new();
    sync_army_trait_input(&mut army, &army_stage, &snap);
    let army_bound = army.bind(&ctx(&army_stage, &snap));
    let army_targets = army.debug_disabled_mutation_targets();
    assert_add_trait_targets_match_roots(&army_targets, &army_bound.hit.roots);

    let (x, y) = army
        .debug_node_center("panzer_leader")
        .expect("panzer_leader node center");
    let (_down, select_up) = click_at(&mut army, x, y);
    assert!(select_up.consumed && select_up.needs_redraw);
    let army_before = bound_digest(&army.bind(&ctx(&army_stage, &snap)));
    assert!(army_before.contains("selected_key=panzer_leader"));
    for (_leaf, rect) in army_targets {
        let (x, y) = rect_center(rect);
        let (down, up) = click_at(&mut army, x, y);
        assert!(
            down.consumed && !down.needs_redraw,
            "add-trait pointer-down must be a replay-disabled no-op"
        );
        assert!(down.commands.is_empty());
        assert!(
            up.consumed && !up.needs_redraw,
            "add-trait pointer-up must also be a replay-disabled no-op"
        );
        assert!(up.commands.is_empty());
    }
    let army_after = bound_digest(&army.bind(&ctx(&army_stage, &snap)));
    assert_eq!(
        army_after, army_before,
        "army add-trait no-op clicks must not alter selected trait state or digest"
    );

    let navy_stage = stage_with(&["navy_trait_tree.json"]);
    let mut navy = NavyTraitTreeView::new();
    sync_navy_trait_input(&mut navy, &navy_stage, &snap);
    let navy_bound = navy.bind(&ctx(&navy_stage, &snap));
    let navy_targets = navy.debug_disabled_mutation_targets();
    assert_add_trait_targets_match_roots(&navy_targets, &navy_bound.hit.roots);

    let (x, y) = navy
        .debug_node_center("seawolf")
        .expect("seawolf node center");
    let (_down, select_up) = click_at(&mut navy, x, y);
    assert!(select_up.consumed && select_up.needs_redraw);
    let navy_before = bound_digest(&navy.bind(&ctx(&navy_stage, &snap)));
    assert!(navy_before.contains("selected_key=seawolf"));
    for (_leaf, rect) in navy_targets {
        let (x, y) = rect_center(rect);
        let (down, up) = click_at(&mut navy, x, y);
        assert!(
            down.consumed && !down.needs_redraw,
            "navy add-trait pointer-down must be a replay-disabled no-op"
        );
        assert!(down.commands.is_empty());
        assert!(
            up.consumed && !up.needs_redraw,
            "navy add-trait pointer-up must also be a replay-disabled no-op"
        );
        assert!(up.commands.is_empty());
    }
    let navy_after = bound_digest(&navy.bind(&ctx(&navy_stage, &snap)));
    assert_eq!(
        navy_after, navy_before,
        "navy add-trait no-op clicks must not alter selected trait state or digest"
    );
}

#[test]
fn trait_tree_missing_selected_key_fails_closed_without_fallback() {
    if !assets_present() {
        eprintln!("[leaders] skipped trait-missing-selected: game_assets/gui not extracted");
        return;
    }
    let stage = stage_with(&["army_trait_tree.json"]);
    let snap = snap_live();
    let mut view = ArmyTraitTreeView::new();
    sync_army_trait_input(&mut view, &stage, &snap);
    view.debug_set_selected_key("ghost_trait");

    let d = bound_digest(&view.bind(&ctx(&stage, &snap)));
    assert!(
        d.contains("selected=missing_selected"),
        "stale key must be visible as missing, not remapped: {d}"
    );
    assert!(
        d.contains("selected_key=ghost_trait"),
        "the stale key must remain explicit for diagnostics: {d}"
    );
    assert!(
        d.contains("selected_detail=none"),
        "missing selected key must not fall back to another trait: {d}"
    );
    assert!(
        !d.contains("Trait ID=text=\"panzer_leader\""),
        "missing selection must not fall back to the first live trait"
    );
}

#[test]
fn trait_tree_selection_scrub_a_b_a_rebuilds_current_detail() {
    if !assets_present() {
        eprintln!("[leaders] skipped trait-scrub-selection: game_assets/gui not extracted");
        return;
    }
    let stage = stage_with(&["army_trait_tree.json"]);
    let (a, b) = (snap_live(), snap_demo());
    let mut view = ArmyTraitTreeView::new();
    sync_army_trait_input(&mut view, &stage, &a);
    let (x, y) = view
        .debug_node_center("trickster")
        .expect("trickster node center");
    let (_down, up) = click_at(&mut view, x, y);
    assert!(up.consumed);

    let a1 = bound_digest(&view.bind(&ctx(&stage, &a)));
    assert!(a1.contains("selected_key=trickster"));
    assert!(a1.contains("Trait ID=text=\"trickster\",fidelity=Exact"));

    sync_army_trait_input(&mut view, &stage, &b);
    let b_digest = bound_digest(&view.bind(&ctx(&stage, &b)));
    assert!(b_digest.contains("selected=missing_selected"));
    assert!(b_digest.contains("selected_key=trickster"));
    assert!(b_digest.contains("selected_detail=none"));

    sync_army_trait_input(&mut view, &stage, &a);
    let a2 = bound_digest(&view.bind(&ctx(&stage, &a)));
    assert_eq!(
        a1, a2,
        "A->B->A selection detail must rebuild byte-identically"
    );
}

#[test]
fn aces_folded_binds_joined_name() {
    if !assets_present() {
        eprintln!("[leaders] skipped aces-folded: game_assets/gui not extracted");
        return;
    }
    let stage = stage_with(&["aces.json"]);
    let d = bind_digest("leaders.aces", &stage, &snap_live());
    assert!(
        d.contains("Erich Hartmann"),
        "the ace's joined name is bound, got: {d}"
    );
    assert!(d.contains("Bubi"), "the ace's callsign is bound");
    assert!(
        d.contains("ACES sect=present"),
        "the folded aces section is present"
    );
}

// ----------------------------------------------------------- inventory registration

/// All 7 routed views are inventory-collected + routed under the Leaders menu cluster, and
/// the View-8 prototype templates are registered (the conflict-free seam — no shared edit).
#[test]
fn cluster_views_and_templates_registered() {
    let reg = ViewRegistry::from_inventory();
    for id in [
        "leaders.officer_corps",
        "leaders.unit_leader",
        "leaders.navy_leader",
        "leaders.leader_groups",
        "leaders.aces",
        "leaders.army_trait_tree",
        "leaders.navy_trait_tree",
    ] {
        assert!(
            reg.view(ViewId(id)).is_some(),
            "view {id} must be inventory-collected"
        );
    }
    let bar = reg.menubar();
    let group = bar
        .iter()
        .find(|g| g.cluster == MenuCluster::LeadersOfficersTraits)
        .unwrap();
    assert_eq!(
        group.items.len(),
        7,
        "all 7 routed views appear under the Leaders menu"
    );
    // the View-8 prototype templates + the row templates are registered.
    for id in [
        "leaders.trait_common.unit_leader_trait",
        "leaders.trait_common.picked_trait",
        "leaders.trait_common.pickable_trait",
        "leaders.trait_common.disabled_trait",
        "leaders.trait_common.active_trait",
        "leaders.aces.entry",
        "leaders.unit_leader.army_entry",
        "leaders.navy_leader.fleet_tile",
    ] {
        assert!(
            reg.template(ViewId(id)).is_some(),
            "template {id} must be registered"
        );
    }
}
