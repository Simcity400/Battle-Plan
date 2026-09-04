
use replay_core::{CountryIdentityLeaf, CountryIdentityRosterLeaf};
use game_ui::compose::{resolve_element_rect, ComposeOptions, Rect};
use game_ui::gui::{GuiElement, LayoutSet};
use game_ui::waroverview_binding::{war_overview_from_frame, WarRelationSummary};
use replay_core::{open_recording, Leaf, WorldTimeline};

mod identity_support;
use identity_support::identity;
use std::path::{Path, PathBuf};

/// A TAG this frame's census admits. A test that means to name a country says
/// so here; a test about a contended key asks `named_country` and gets `None`.
fn named<'tag>(
    frame: &'tag replay_core::WorldFrame,
    tag: &'tag str,
) -> replay_core::NamedCountry<'tag> {
    frame.named_country(tag).expect("census-admitted tag")
}

fn game_root() -> PathBuf {
    replay_test_gates::require_vanilla_game_root()
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

#[test]
#[cfg_attr(
    not(gate_hoi4_install),
    ignore = "the game install (set BATTLE_PLAN_GAME_ROOT / BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA, or install at the default Steam path)"
)]
fn waroverview_layout_resolves_from_files() {
    let root = game_root();
    let layouts = parse_layout(&root, "interface/waroverview.gui");
    assert_eq!(
        layouts.stats.recovery_count, 0,
        "waroverview.gui parses clean"
    );

    let window = find(&layouts, "waroverview_window", "containerWindowType");
    let width = window
        .size
        .as_ref()
        .expect("window size")
        .x
        .value
        .as_deref();
    assert_eq!(width, Some("1167"), "waroverview_window declared width");
    assert!(window
        .children
        .iter()
        .any(|c| c.name == "waroverview_wars_box"));
    assert!(window.children.iter().any(|c| c.name == "attacker_window"));
    assert!(window.children.iter().any(|c| c.name == "defender_window"));

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
        first: identity(first),
        second: identity(second),
        first_casualties: Some(fc),
        second_casualties: Some(sc),
        residuals: Leaf::Absent,
    }
}

#[test]
fn war_overview_flips_at_change_hour() {
    let tl = fixture("fx_war_faction_matrix");
    let hours = tl.hours().to_vec();
    assert_eq!(hours.len(), 12);

    let before = war_overview_from_frame(&tl.frame(hours[6]).unwrap(), named(&tl.frame(hours[6]).unwrap(), "GER"));
    assert!(before.available, "diplomacy present @tick6");
    assert!(
        before.war_relations.measured().is_none(),
        "no war_relations feed @tick6 (leaf stays unmeasured, not empty)"
    );
    assert!(before.belligerents.is_empty(), "no belligerents @tick6");
    assert!(before.pov_belligerents.is_empty(), "GER no wars @tick6");

    let after = war_overview_from_frame(&tl.frame(hours[7]).unwrap(), named(&tl.frame(hours[7]).unwrap(), "GER"));
    assert!(after.available);
    assert_eq!(
        after.war_relations,
        Leaf::Measured(vec![
            summary("GER", "ENG", 1200, 900),
            summary("ITA", "ENG", 200, 300),
        ]),
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
        wars[0].attackers.tags(),
        Leaf::Measured(vec!["GER", "ITA"])
    );
    assert_eq!(wars[0].defenders.tags(), Leaf::Measured(vec!["ENG"]));
    assert_eq!(wars[0].front, Leaf::Measured("western".to_string()));
    assert_eq!(wars[0].war_name, Leaf::Absent);
    assert_eq!(wars[0].attacker_participants, Leaf::Absent);
    assert_eq!(wars[0].overall_progress_raw, Leaf::Absent);
    assert_eq!(wars[0].residuals, Leaf::Absent);
}

#[test]
fn belligerent_set_is_global_pov_belligerents_are_per_pov() {
    let tl = fixture("fx_war_faction_matrix");
    let hours = tl.hours().to_vec();
    let ger = war_overview_from_frame(&tl.frame(hours[7]).unwrap(), named(&tl.frame(hours[7]).unwrap(), "GER"));
    let eng = war_overview_from_frame(&tl.frame(hours[7]).unwrap(), named(&tl.frame(hours[7]).unwrap(), "ENG"));

    assert_eq!(ger.belligerents, eng.belligerents);
    assert_eq!(ger.war_relations, eng.war_relations);
    assert_eq!(
        eng.pov_belligerents,
        vec!["GER".to_string(), "ITA".to_string()]
    );
}

#[test]
fn absent_pov_sees_global_wars_only() {
    let tl = fixture("fx_war_faction_matrix");
    let hours = tl.hours().to_vec();
    let usa = war_overview_from_frame(&tl.frame(hours[7]).unwrap(), named(&tl.frame(hours[7]).unwrap(), "USA"));
    assert!(usa.available);
    assert_eq!(
        usa.war_relations.measured().expect("measured feed").len(),
        2,
        "global casualty rows"
    );
    assert_eq!(usa.belligerents.len(), 3, "global belligerent set");
    assert!(usa.pov_belligerents.is_empty(), "USA in no war");
}
