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
//!   * tick >= 3: roster 101=[GER(leader), ITA], roster 202=[ENG(leader)] —
//!     the authoritative diplomacy_live.faction_members identities/members.
//!   * tick >= 7: `GER.diplomacy.wars=[against ENG]`,
//!     `ENG.diplomacy.wars=[against GER, against ITA]` — the `wars`/`against`
//!     lists the view folds into directional `at_war` pairs. (ITA carries no
//!     `wars` list, only an `at_war` array the view does not read, so no
//!     `ITA -> ENG` war pair exists — a deliberate ground-truth asymmetry.)
//! Countries present at every tick: ENG, FRA, GER, ITA, SOV.

use game_ui::binding::{diplomacy_panel_from_frame, FactionRow, RelationRow};
use replay_core::world::FactionRosterStatus;
use replay_core::{open_recording, Leaf, SubjectAutonomyDirection, WorldTimeline};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

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

    // Change hour (tick 3): both roster identities materialize, sorted by id.
    let after = diplomacy_panel_from_frame(&tl.frame(hours[3]).unwrap(), "GER");
    assert_eq!(
        after.factions,
        vec![
            faction("101", "GER", &["GER", "ITA"]),
            faction("202", "ENG", &["ENG"]),
        ]
    );

    // Holds unchanged through the rest of the timeline (checked at the end).
    let last = diplomacy_panel_from_frame(&tl.frame(hours[11]).unwrap(), "SOV");
    assert_eq!(
        last.factions,
        vec![
            faction("101", "GER", &["GER", "ITA"]),
            faction("202", "ENG", &["ENG"]),
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

fn faction_roster_binding_timeline() -> (PathBuf, WorldTimeline) {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "game_ui_faction_status_{}_{}_{}",
        std::process::id(),
        n,
        nanos
    ));
    let game = "faction_status";
    let dir = root.join(game);
    fs::create_dir_all(&dir).unwrap();
    let states = [
        (30, Some("unavailable"), None),
        (31, Some("measured_empty"), Some("[]")),
        (33, None, None),
    ];
    let countries = r#"{"GER":{"politics":{"faction":{"name":"Axis","is_leader":true}}},"ITA":{"politics":{"faction":{"name":"Axis","is_leader":false}}}}"#;
    let mut index_rows = Vec::new();
    for (hour, status, rows) in states {
        let extra = match (status, rows) {
            (Some(status), Some(rows)) => {
                format!(r#"{{"faction_roster_status":"{status}","faction_rows":{rows}}}"#)
            }
            (Some(status), None) => format!(r#"{{"faction_roster_status":"{status}"}}"#),
            (None, None) => "{}".to_string(),
            (None, Some(_)) => unreachable!("faction rows require a roster status"),
        };
        let snapshot = format!(
            r#"{{"schema_version":"1","game_id":"{game}","date":"1936.1.1.{}","hour_index":{hour},"countries":{countries},"diplomacy_world":{{"domain53_extra":{extra}}}}}"#,
            hour - 30
        );
        let file = format!("{hour:012}.snapshot.json");
        fs::write(dir.join(&file), snapshot).unwrap();
        index_rows.push(format!(
            r#"{{"hour_index":{hour},"date":"1936.1.1.{}","file":"{file}"}}"#,
            hour - 30
        ));
    }
    let index = format!(
        r#"{{"game_id":"{game}","schema_version":"1","snapshots":[{}]}}"#,
        index_rows.join(",")
    );
    fs::write(dir.join("index.json"), index).unwrap();
    let timeline = WorldTimeline::open(open_recording(&root).unwrap()).unwrap();
    (root, timeline)
}

/// Open the reviewed replay-owned projection of the D53 proof fixture through
/// the recorder fold. The committed boundary retains only the fields consumed
/// by this binding law; its adjacent provenance binds the extractor producers.
fn committed_faction_roster_binding_timeline() -> (WorldTimeline, i64) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/diplomacy_binding_production_fold");
    let timeline = WorldTimeline::open(
        open_recording(&root)
            .unwrap_or_else(|error| panic!("open committed D53 fold fixture: {error}")),
    )
    .unwrap_or_else(|error| panic!("committed D53 fold timeline: {error}"));
    let hour = {
        let hours = timeline.hours();
        assert_eq!(hours, &[16_959_360], "committed D53 fold fixture census");
        hours[0]
    };
    (timeline, hour)
}

#[test]
fn non_nonempty_roster_status_suppresses_country_faction_rows() {
    let (root, timeline) = faction_roster_binding_timeline();
    let cases = [
        (30, FactionRosterStatus::Unavailable, false, "unavailable"),
        (
            31,
            FactionRosterStatus::MeasuredEmpty,
            true,
            "measured-empty",
        ),
        (33, FactionRosterStatus::Unavailable, false, "absent status"),
    ];

    for (hour, expected_status, expected_available, label) in cases {
        let panel = diplomacy_panel_from_frame(&timeline.frame(hour).unwrap(), "GER");
        assert!(panel.available, "other diplomacy leaves remain available");
        assert_eq!(panel.faction_roster_status, expected_status, "{label}");
        assert_eq!(panel.factions_available, expected_available, "{label}");
        assert!(
            panel.factions.is_empty(),
            "{label} roster leaked country politics faction rows: {:?}",
            panel.factions
        );
        assert!(!panel.subject_autonomy_available, "{label}");
        assert!(panel.subject_autonomy.is_empty(), "{label}");
    }

    drop(timeline);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn measured_nonempty_committed_fold_without_country_names_keeps_d53_roster() {
    let (timeline, hour) = committed_faction_roster_binding_timeline();
    let frame = timeline.frame(hour).unwrap();
    assert!(
        !frame.diplomacy().countries.is_empty(),
        "the reviewed fold projection must retain the roster member countries"
    );
    assert!(
        frame
            .diplomacy()
            .countries
            .values()
            .all(|country| matches!(&country.faction_name, Leaf::Absent)),
        "the reviewed D53 fold projection must not fabricate countries.*.politics.faction.name"
    );
    assert_eq!(
        frame
            .diplomacy()
            .faction_members
            .measured()
            .unwrap()
            .get("1"),
        Some(&vec!["1".to_string(), "3".to_string(), "22".to_string()]),
        "the typed view must retain the reviewed folded D53 member identities"
    );

    let nonempty = diplomacy_panel_from_frame(&frame, "1");
    assert!(nonempty.factions_available);
    assert_eq!(
        nonempty.faction_roster_status,
        FactionRosterStatus::MeasuredNonEmpty
    );
    assert_eq!(
        nonempty.factions,
        vec![faction("1", "1", &["1", "22", "3"])],
        "the binding must render the source-derived D53 roster even though every country faction name is absent"
    );
}

#[test]
fn a16_current_autonomy_is_available_to_both_country_panel_sides() {
    static A16_COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = A16_COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "game_ui_a16_binding_{}_{}",
        std::process::id(),
        n
    ));
    let game = "a16_binding";
    let dir = root.join(game);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("000000000900.snapshot.json"),
        r#"{
          "schema_version":"1",
          "game_id":"a16_binding",
          "date":"1940.2.14.20",
          "hour_index":900,
          "countries":{
            "GER":{"_r2_tag_db_id":1},
            "BUL":{"_r2_tag_db_id":22}
          },
          "diplomacy_world":{"domain53_extra":{
            "subject_autonomy_rows":[{
              "overlord_tag_db_id":1,
              "subject_tag_db_id":22
            }],
            "subject_autonomy_current_status":"measured_nonempty",
            "subject_autonomy_current_rows":[{
              "overlord":"GER",
              "subject":"BUL",
              "overlord_tag_db_id":1,
              "subject_tag_db_id":22,
              "current_state_hash":40977,
              "previous_state_hash":36881,
              "next_state_hash":49169,
              "ratio_raw":10000,
              "current_score_raw":110000,
              "lower_score_raw":100000,
              "upper_score_raw":200000,
              "last_modified_date":"1940.2.14.20",
              "last_modified_date_raw":60795475,
              "direction":"toward_next"
            }]
          }}
        }"#,
    )
    .unwrap();
    fs::write(
        dir.join("index.json"),
        r#"{"game_id":"a16_binding","schema_version":"1","snapshots":[{"hour_index":900,"date":"1940.2.14.20","file":"000000000900.snapshot.json"}]}"#,
    )
    .unwrap();

    let timeline = WorldTimeline::open(open_recording(&root).unwrap()).unwrap();
    let frame = timeline.frame(900).unwrap();
    let ger = diplomacy_panel_from_frame(&frame, "GER");
    assert!(ger.subject_autonomy_available);
    assert_eq!(ger.subject_autonomy.len(), 1);
    assert!(ger.subject_autonomy[0].pov_is_overlord);
    assert_eq!(ger.subject_autonomy[0].counterpart.as_deref(), Some("BUL"));
    assert_eq!(ger.subject_autonomy[0].ratio_raw, 10_000);
    assert_eq!(
        ger.subject_autonomy[0].direction,
        Some(SubjectAutonomyDirection::TowardNext)
    );

    let bul = diplomacy_panel_from_frame(&frame, "BUL");
    assert!(bul.subject_autonomy_available);
    assert_eq!(bul.subject_autonomy.len(), 1);
    assert!(!bul.subject_autonomy[0].pov_is_overlord);
    assert_eq!(bul.subject_autonomy[0].counterpart.as_deref(), Some("GER"));

    let unrelated = diplomacy_panel_from_frame(&frame, "ENG");
    assert!(unrelated.subject_autonomy_available);
    assert!(unrelated.subject_autonomy.is_empty());
    drop(frame);
    drop(timeline);
    fs::remove_dir_all(root).unwrap();
}
