//! D4 country/diplomacy-panel population gates (REPLAY_PRODUCTION_PLAN Phase D,
//! item D4).
//!
//! Drives the `fx_war_faction_matrix` packed fixture through the real
//! `replay_core` `ReplaySource`/`WorldTimeline` and asserts that
//! `game_ui::binding::diplomacy_panel_from_frame` renders the correct faction
//! memberships and war rows before/after the two matrix changes, and that each
//! change flips exactly the expected rows.
//!
//! Expected values are transcribed from the fixture GENERATOR SOURCE
//! (`replay_fixture_gen::fx_war_faction_matrix`), never from the code under
//! test:
//!   * tick >= 3: GER=Axis(leader), ITA=Axis, ENG=Allies(leader) — the
//!     `politics.faction` name/leader the diplomacy view reads.
//!   * tick >= 7: `GER.diplomacy.wars=[against ENG]`,
//!     `ENG.diplomacy.wars=[against GER, against ITA]` — the `wars`/`against`
//!     lists the view folds into directional `at_war` pairs. (ITA carries no
//!     `wars` list, only an `at_war` array the view does not read, so no
//!     `ITA -> ENG` war pair exists — a deliberate ground-truth asymmetry.)
//! Countries present at every tick: ENG, FRA, GER, ITA, SOV.

use game_ui::binding::{diplomacy_panel_from_frame, FactionRow, RelationRow};
use replay_core::{open_recording, WorldTimeline};
use std::path::PathBuf;

fn fixture(name: &str) -> WorldTimeline {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/positive")
        .join(format!("{name}.battleplan"));
    let source = open_recording(&path)
        .unwrap_or_else(|err| panic!("open {name}: {err} ({})", path.display()));
    WorldTimeline::open(source).unwrap_or_else(|err| panic!("timeline {name}: {err}"))
}

fn faction(name: &str, leader: &str, members: &[&str]) -> FactionRow {
    FactionRow {
        faction_name: name.to_string(),
        leader: Some(leader.to_string()),
        members: members.iter().map(|m| m.to_string()).collect(),
    }
}

/// Look up one `pov -> other` relation row.
fn rel<'a>(rows: &'a [RelationRow], other: &str) -> &'a RelationRow {
    rows.iter()
        .find(|r| r.other_tag == other)
        .unwrap_or_else(|| panic!("no relation row for {other}"))
}

/// GATE 2a — faction membership matrix appears exactly at the tick-3 change and
/// holds after; it is empty before. The change hour flips exactly the faction
/// rows (nothing before, both factions after).
#[test]
fn faction_matrix_flips_at_change_hour() {
    let tl = fixture("fx_war_faction_matrix");
    let hours = tl.hours().to_vec();
    assert_eq!(hours.len(), 12);

    // Before the flip (tick 2): no measured faction memberships.
    let before = diplomacy_panel_from_frame(&tl.frame(hours[2]).unwrap(), "GER");
    assert!(before.available);
    assert!(before.factions.is_empty(), "no factions before tick 3");

    // Change hour (tick 3): both factions materialize, sorted by name.
    let after = diplomacy_panel_from_frame(&tl.frame(hours[3]).unwrap(), "GER");
    assert_eq!(
        after.factions,
        vec![
            faction("Allies", "ENG", &["ENG"]),
            faction("Axis", "GER", &["GER", "ITA"]),
        ]
    );

    // Holds unchanged through the rest of the timeline (checked at the end).
    let last = diplomacy_panel_from_frame(&tl.frame(hours[11]).unwrap(), "SOV");
    assert_eq!(
        last.factions,
        vec![
            faction("Allies", "ENG", &["ENG"]),
            faction("Axis", "GER", &["GER", "ITA"]),
        ],
        "faction matrix is POV-independent and stable"
    );
}

/// GATE 2b — war rows appear exactly at the tick-7 change, per POV, and flip
/// exactly the expected belligerents. GER's panel gains only ENG; ENG's panel
/// gains GER and ITA.
#[test]
fn war_rows_flip_at_change_hour_per_pov() {
    let tl = fixture("fx_war_faction_matrix");
    let hours = tl.hours().to_vec();

    // Before the war flip (tick 6): no wars for any POV.
    let ger_before = diplomacy_panel_from_frame(&tl.frame(hours[6]).unwrap(), "GER");
    let eng_before = diplomacy_panel_from_frame(&tl.frame(hours[6]).unwrap(), "ENG");
    assert!(ger_before.wars.is_empty(), "GER no wars @tick6");
    assert!(eng_before.wars.is_empty(), "ENG no wars @tick6");
    assert_eq!(rel(&ger_before.relations, "ENG").at_war, None);

    // Change hour (tick 7): directional at_war pairs appear.
    let ger = diplomacy_panel_from_frame(&tl.frame(hours[7]).unwrap(), "GER");
    let eng = diplomacy_panel_from_frame(&tl.frame(hours[7]).unwrap(), "ENG");
    assert_eq!(ger.wars, vec!["ENG".to_string()], "GER belligerents @tick7");
    assert_eq!(
        eng.wars,
        vec!["GER".to_string(), "ITA".to_string()],
        "ENG belligerents @tick7"
    );

    // The at_war flip is exactly on GER->ENG, not the other GER relations.
    assert_eq!(rel(&ger.relations, "ENG").at_war, Some(true));
    assert_eq!(rel(&ger.relations, "FRA").at_war, None);
    assert_eq!(rel(&ger.relations, "ITA").at_war, None);
    assert_eq!(rel(&ger.relations, "SOV").at_war, None);
}

/// GATE 2c — relation-row detail: same_faction (folded from faction names) and
/// at_war coexist without cross-contamination, and unrecorded flags stay
/// fail-closed `None` (never a fabricated `false`).
#[test]
fn relation_row_flags_are_fail_closed() {
    let tl = fixture("fx_war_faction_matrix");
    let hours = tl.hours().to_vec();
    let ger = diplomacy_panel_from_frame(&tl.frame(hours[7]).unwrap(), "GER");

    // GER -> ITA: same faction (Axis), NOT at war.
    let ita = rel(&ger.relations, "ITA");
    assert_eq!(ita.same_faction, Some(true));
    assert_eq!(ita.at_war, None);
    // Flags with no recorded relation stay None, not Some(false).
    assert_eq!(ita.embargo, None);
    assert_eq!(ita.military_access, None);
    assert_eq!(ita.guarantee, None);

    // GER -> ENG: at war, NOT same faction.
    let eng = rel(&ger.relations, "ENG");
    assert_eq!(eng.at_war, Some(true));
    assert_eq!(eng.same_faction, None);

    // One relation row per other country (5 countries, minus self).
    assert_eq!(ger.relations.len(), 4);
    let others: Vec<&str> = ger.relations.iter().map(|r| r.other_tag.as_str()).collect();
    assert_eq!(others, vec!["ENG", "FRA", "ITA", "SOV"]);
}

/// GATE 2d — a POV absent from the recording still gets a complete, fail-closed
/// panel: the global faction matrix, one all-`None` relation row per country,
/// and no wars. Never a fabricated relationship.
#[test]
fn absent_pov_renders_fail_closed_panel() {
    let tl = fixture("fx_war_faction_matrix");
    let hours = tl.hours().to_vec();
    let usa = diplomacy_panel_from_frame(&tl.frame(hours[7]).unwrap(), "USA");

    assert!(usa.available);
    // Faction matrix is global and present regardless of POV.
    assert_eq!(usa.factions.len(), 2);
    // A relation row for every recorded country (USA is not among them).
    let others: Vec<&str> = usa.relations.iter().map(|r| r.other_tag.as_str()).collect();
    assert_eq!(others, vec!["ENG", "FRA", "GER", "ITA", "SOV"]);
    assert!(usa.relations.iter().all(|r| r.at_war.is_none()));
    assert!(usa.wars.is_empty());
}
