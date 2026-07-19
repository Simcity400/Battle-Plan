//! U1 combat-detail panel gates: a LAYOUT gate (parse `landcombat.gui` via the
//! shared `game_ui` parser and resolve named child rects through the LAW-solid
//! `resolve_element_rect`) plus a BINDING gate (drive the real
//! `replay_core::ReplaySource`/`WorldTimeline` through the `fx_division_movement`
//! packed fixture and assert `combat_panel_from_frame` renders the exact
//! generator combat values, appearing/disappearing at the exact ticks).
//!
//! Expected values are transcribed from the fixture GENERATOR SOURCE
//! (`replay_fixture_gen::fx_division_movement`), never from the code under test:
//! ticks 6..=9 carry one combat `{combat_id:9001, kind:"land",
//! location_province: (1001 while tick<8 else 2000), attackers:[101],
//! defenders:[201]}`; every other tick has no active combat.

use game_ui::combat_binding::combat_panel_from_frame;
use game_ui::compose::{resolve_element_rect, ComposeOptions, Rect};
use game_ui::gui::{GuiElement, LayoutSet};
use replay_core::{open_recording, WorldTimeline};
use std::path::{Path, PathBuf};

const DEFAULT_GAME_ROOT: &str = r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV";

fn game_root() -> Option<PathBuf> {
    let root = std::env::var_os("BATTLE_PLAN_GAME_ROOT").or_else(|| std::env::var_os("HOI4_GAME_ROOT"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_ROOT));
    root.is_dir().then_some(root)
}

fn parse_layout(root: &Path, rel: &str) -> LayoutSet {
    let mut layouts = LayoutSet::default();
    layouts
        .parse_file(&root.join(rel), rel)
        .expect("parse vanilla gui");
    layouts
}

fn find<'a>(layouts: &'a LayoutSet, name: &str, kind: &str) -> &'a GuiElement {
    layouts
        .find_by_name(name)
        .into_iter()
        .find(|element| element.element_type.eq_ignore_ascii_case(kind))
        .unwrap_or_else(|| panic!("{kind} {name} missing"))
}

fn child<'a>(parent: &'a GuiElement, name: &str) -> &'a GuiElement {
    parent
        .children
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("child {name} missing"))
}

fn fixture(name: &str) -> WorldTimeline {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/positive")
        .join(format!("{name}.battleplan"));
    let source = open_recording(&path)
        .unwrap_or_else(|err| panic!("open {name}: {err} ({})", path.display()));
    WorldTimeline::open(source).unwrap_or_else(|err| panic!("timeline {name}: {err}"))
}

/// LAYOUT GATE — `landcombat.gui` parses with 0 recoveries and `landcombatview`
/// resolves to its file-declared rect; its direct combat-detail children
/// resolve to engine-law pixel positions (center anchor + explicit position).
#[test]
fn landcombatview_layout_resolves_from_files() {
    let Some(root) = game_root() else {
        eprintln!("SKIP landcombatview_layout_resolves_from_files: the game root missing");
        return;
    };
    let layouts = parse_layout(&root, "interface/landcombat.gui");
    assert_eq!(
        layouts.stats.recovery_count, 0,
        "landcombat.gui parses clean"
    );

    let view = find(&layouts, "landcombatview", "containerWindowType");
    // Declared size 567x617; orientation center; position {-512,-256}.
    let options = ComposeOptions::new(1920, 1080);
    let screen = Rect::new(0, 0, 1920, 1080);
    let view_rect = resolve_element_rect(view, screen, (0, 0), &options);
    // center anchor = (960,540); + position (-512,-256) = (448,284), size 567x617.
    assert_eq!(view_rect, Rect::new(448, 284, 567, 617));

    // Direct children resolve relative to the window rect (UpperLeft default
    // anchor, explicit positions; textboxes/icons carry no size → natural 0x0).
    let cases = [
        ("window_title", 55, 2),
        ("terrain_picture", 77, 37),
        ("combat_progress", 185, 116),
        ("land_battle_progress_frame", 183, 114),
    ];
    for (name, px, py) in cases {
        let el = child(view, name);
        let rect = resolve_element_rect(el, view_rect, (0, 0), &options);
        assert_eq!(
            (rect.x(), rect.y()),
            (view_rect.x() + px, view_rect.y() + py),
            "child {name} rect"
        );
    }
    // The two participant sub-windows are present (division/reserve lists).
    assert!(view.children.iter().any(|c| c.name == "attacker_window"));
    assert!(view.children.iter().any(|c| c.name == "defender_window"));
}

/// BINDING GATE 1 — the combat appears at tick 6, holds through tick 9, and is
/// absent before/after, with the exact generator identity + participant counts.
#[test]
fn combat_panel_flips_at_generator_ticks() {
    let tl = fixture("fx_division_movement");
    let hours = tl.hours().to_vec();
    assert_eq!(hours.len(), 12);

    for (t, &hour) in hours.iter().enumerate() {
        let b = combat_panel_from_frame(&tl.frame(hour).unwrap());
        if (6..=9).contains(&t) {
            assert!(b.available, "combat active @tick{t}");
            assert_eq!(b.combats.len(), 1, "one combat @tick{t}");
            let row = &b.combats[0];
            assert_eq!(row.combat_id, Some(9001), "combat_id @tick{t}");
            assert_eq!(row.kind.as_deref(), Some("land"), "kind @tick{t}");
            let expected_loc = if t < 8 { 1001 } else { 2000 };
            assert_eq!(
                row.location_province,
                Some(expected_loc),
                "location @tick{t}"
            );
            assert_eq!(
                row.attacker_divisions,
                Some(vec![101]),
                "attackers @tick{t}"
            );
            assert_eq!(
                row.defender_divisions,
                Some(vec![201]),
                "defenders @tick{t}"
            );
            assert_eq!(row.attacker_division_count(), Some(1));
            assert_eq!(row.defender_division_count(), Some(1));
            assert_eq!(row.terrain.as_deref(), Some("plains"));
            assert_eq!(row.progress, Some(t as f64 / 10.0));
            assert_eq!(row.combat_width, Some(80.0));
        } else {
            assert!(!b.available, "no combat @tick{t}");
            assert!(b.combats.is_empty(), "no combat rows @tick{t}");
        }
    }
}

/// BINDING GATE 2 — typed producer fields render exactly while genuinely
/// external selectors remain fail-closed.
#[test]
fn typed_display_fields_bind_and_external_selectors_fail_closed() {
    let tl = fixture("fx_division_movement");
    let hours = tl.hours().to_vec();
    let b = combat_panel_from_frame(&tl.frame(hours[7]).unwrap());
    let row = &b.combats[0];
    assert_eq!(row.progress, Some(0.7), "producer progress");
    assert_eq!(row.combat_width, Some(80.0), "producer combat width");
    assert_eq!(row.terrain.as_deref(), Some("plains"), "producer terrain");
    assert_eq!(row.bombardment_active, None, "A16 bombardment fail-closed");
    assert_eq!(row.air_superiority_state, None, "A16 air-sup fail-closed");
}
