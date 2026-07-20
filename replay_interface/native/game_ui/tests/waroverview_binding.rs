//! U1 war-overview panel gates: a LAYOUT gate (parse `waroverview.gui` via the
//! shared `game_ui` parser and resolve the participant-row template's named
//! textboxes through `resolve_element_rect`) plus a BINDING gate (drive the real
//! `replay_core` `fx_war_faction_matrix` packed fixture and assert
//! `war_overview_from_frame` renders the exact generator war-relation casualties
//! and belligerent set, appearing at the exact tick).
//!
//! Expected values are transcribed from the fixture GENERATOR SOURCE
//! (`replay_fixture_gen::fx_war_faction_matrix`), never from the code under
//! test. At tick>=7:
//!   * `diplomacy_live.war_relations = [{GER,ENG,1200,900},{ITA,ENG,200,300}]`
//!   * `at_war`: GER->ENG, ENG->{GER,ITA}, ITA->ENG.
//! Before tick 7 there is no war (no war_relations, no at_war pairs).

use game_ui::compose::{resolve_element_rect, ComposeOptions, Rect};
use game_ui::gui::{GuiElement, LayoutSet};
use game_ui::waroverview_binding::{war_overview_from_frame, WarRelationSummary};
use replay_core::{open_recording, Leaf, WorldTimeline};
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

/// LAYOUT GATE — `waroverview.gui` parses clean; the main window and the wars
/// gridbox are present, and the `war_ally_entry` participant-row template's
/// named textboxes resolve to their file-declared pixel offsets.
#[test]
fn waroverview_layout_resolves_from_files() {
    let Some(root) = game_root() else {
        eprintln!("SKIP waroverview_layout_resolves_from_files: the game root missing");
        return;
    };
    let layouts = parse_layout(&root, "interface/waroverview.gui");
    assert_eq!(
        layouts.stats.recovery_count, 0,
        "waroverview.gui parses clean"
    );

    // Main window: declared width 1167 (height is 85% → runtime, not gated).
    let window = find(&layouts, "waroverview_window", "containerWindowType");
    let width = window
        .size
        .as_ref()
        .expect("window size")
        .x
        .value
        .as_deref();
    assert_eq!(width, Some("1167"), "waroverview_window declared width");
    // The wars list gridbox and both participant columns exist.
    assert!(window
        .children
        .iter()
        .any(|c| c.name == "waroverview_wars_box"));
    assert!(window.children.iter().any(|c| c.name == "attacker_window"));
    assert!(window.children.iter().any(|c| c.name == "defender_window"));

    // Participant row template: fixed size 530x41; textbox children carry no
    // size (natural 0x0) and UpperLeft-anchor to their explicit positions.
    let entry = find(&layouts, "war_ally_entry", "containerWindowType");
    let options = ComposeOptions::new(1920, 1080);
    let entry_rect = Rect::new(0, 0, 530, 41);
    let cases = [
        ("country_name", 56, 4),
        ("warscore", 269, 11),
        ("divisions", 317, 11),
        ("ic", 393, 11),
        ("casualties", 466, 11),
    ];
    for (name, px, py) in cases {
        let el = child(entry, name);
        let rect = resolve_element_rect(el, entry_rect, (0, 0), &options);
        assert_eq!(
            (rect.x(), rect.y()),
            (px, py),
            "war_ally_entry child {name}"
        );
    }
}

fn summary(first: &str, second: &str, fc: i64, sc: i64) -> WarRelationSummary {
    WarRelationSummary {
        first: first.to_string(),
        second: second.to_string(),
        first_casualties: Some(fc),
        second_casualties: Some(sc),
        residuals: Leaf::Absent,
    }
}

/// BINDING GATE 1 — no war before tick 7; at tick 7 the two live belligerent-
/// pair casualty rows + the belligerent set materialize exactly as the
/// generator wrote them.
#[test]
fn war_overview_flips_at_change_hour() {
    let tl = fixture("fx_war_faction_matrix");
    let hours = tl.hours().to_vec();
    assert_eq!(hours.len(), 12);

    // Before the war flip (tick 6): diplomacy present, but no war rows.
    let before = war_overview_from_frame(&tl.frame(hours[6]).unwrap(), "GER");
    assert!(before.available, "diplomacy present @tick6");
    assert!(before.war_relations.is_empty(), "no war_relations @tick6");
    assert!(before.belligerents.is_empty(), "no belligerents @tick6");
    assert!(before.pov_belligerents.is_empty(), "GER no wars @tick6");

    // Change hour (tick 7): the two casualty rows + belligerent set appear.
    let after = war_overview_from_frame(&tl.frame(hours[7]).unwrap(), "GER");
    assert!(after.available);
    assert_eq!(
        after.war_relations,
        vec![
            summary("GER", "ENG", 1200, 900),
            summary("ITA", "ENG", 200, 300),
        ],
        "war_relations casualties @tick7"
    );
    assert_eq!(
        after.belligerents,
        vec!["ENG".to_string(), "GER".to_string(), "ITA".to_string()],
        "belligerent set @tick7"
    );
    assert_eq!(
        after.pov_belligerents,
        vec!["ENG".to_string()],
        "GER wars @tick7"
    );
    let wars = after.war_objects.measured().expect("war objects @tick7");
    assert_eq!(wars.len(), 1);
    assert_eq!(wars[0].source_key, "axis_allies_1936");
    assert_eq!(
        wars[0].attackers,
        Leaf::Measured(vec!["GER".to_string(), "ITA".to_string()])
    );
    assert_eq!(wars[0].defenders, Leaf::Measured(vec!["ENG".to_string()]));
    assert_eq!(wars[0].front, Leaf::Measured("western".to_string()));
    assert_eq!(wars[0].war_name, Leaf::Absent);
    assert_eq!(wars[0].attacker_participants, Leaf::Absent);
    assert_eq!(wars[0].overall_progress_raw, Leaf::Absent);
    assert_eq!(wars[0].residuals, Leaf::Absent);
}

/// BINDING GATE 2 — the belligerent set is POV-independent (global), while
/// `pov_belligerents` is per-POV. ENG's POV sees its two enemies; the global
/// set is identical to GER's.
#[test]
fn belligerent_set_is_global_pov_belligerents_are_per_pov() {
    let tl = fixture("fx_war_faction_matrix");
    let hours = tl.hours().to_vec();
    let ger = war_overview_from_frame(&tl.frame(hours[7]).unwrap(), "GER");
    let eng = war_overview_from_frame(&tl.frame(hours[7]).unwrap(), "ENG");

    // The global belligerent set + casualty rows do not depend on POV.
    assert_eq!(ger.belligerents, eng.belligerents);
    assert_eq!(ger.war_relations, eng.war_relations);
    // POV belligerents differ: ENG is at war with GER and ITA.
    assert_eq!(
        eng.pov_belligerents,
        vec!["GER".to_string(), "ITA".to_string()]
    );
}

/// BINDING GATE 3 — a POV absent from the recording still gets the global war
/// picture (fail-closed panel), with an empty POV-belligerent list.
#[test]
fn absent_pov_sees_global_wars_only() {
    let tl = fixture("fx_war_faction_matrix");
    let hours = tl.hours().to_vec();
    let usa = war_overview_from_frame(&tl.frame(hours[7]).unwrap(), "USA");
    assert!(usa.available);
    assert_eq!(usa.war_relations.len(), 2, "global casualty rows");
    assert_eq!(usa.belligerents.len(), 3, "global belligerent set");
    assert!(usa.pov_belligerents.is_empty(), "USA in no war");
}
