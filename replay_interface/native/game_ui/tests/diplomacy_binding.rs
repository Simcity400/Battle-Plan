
use game_ui::binding::{diplomacy_panel_from_frame, FactionRow, RelationRow};
use replay_core::world::FactionRosterStatus;
use replay_core::{open_recording, Leaf, SubjectAutonomyDirection, WorldTimeline};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// A TAG this frame's census admits. A test that means to name a country says
/// so here; a test about a contended key asks `named_country` and gets `None`.
fn named<'tag>(
    frame: &'tag replay_core::WorldFrame,
    tag: &'tag str,
) -> replay_core::NamedCountry<'tag> {
    frame.named_country(tag).expect("census-admitted tag")
}

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

fn rel<'a>(rows: &'a [RelationRow], other: &str) -> &'a RelationRow {
    rows.iter()
        .find(|r| r.other_tag == other)
        .unwrap_or_else(|| panic!("no relation row for {other}"))
}

#[test]
fn faction_matrix_flips_at_change_hour() {
    let tl = fixture("fx_war_faction_matrix");
    let hours = tl.hours().to_vec();
    assert_eq!(hours.len(), 12);

    let before = diplomacy_panel_from_frame(&tl.frame(hours[2]).unwrap(), named(&tl.frame(hours[2]).unwrap(), "GER"));
    assert!(before.available);
    assert!(before.factions.is_empty(), "no factions before tick 3");

    let after = diplomacy_panel_from_frame(&tl.frame(hours[3]).unwrap(), named(&tl.frame(hours[3]).unwrap(), "GER"));
    assert_eq!(
        after.factions,
        vec![
            faction("101", "GER", &["GER", "ITA"]),
            faction("202", "ENG", &["ENG"]),
        ]
    );

    let last = diplomacy_panel_from_frame(&tl.frame(hours[11]).unwrap(), named(&tl.frame(hours[11]).unwrap(), "SOV"));
    assert_eq!(
        last.factions,
        vec![
            faction("101", "GER", &["GER", "ITA"]),
            faction("202", "ENG", &["ENG"]),
        ],
        "faction matrix is POV-independent and stable"
    );
}

#[test]
fn war_rows_flip_at_change_hour_per_pov() {
    let tl = fixture("fx_war_faction_matrix");
    let hours = tl.hours().to_vec();

    let ger_before = diplomacy_panel_from_frame(&tl.frame(hours[6]).unwrap(), named(&tl.frame(hours[6]).unwrap(), "GER"));
    let eng_before = diplomacy_panel_from_frame(&tl.frame(hours[6]).unwrap(), named(&tl.frame(hours[6]).unwrap(), "ENG"));
    assert!(ger_before.wars.is_empty(), "GER no wars @tick6");
    assert!(eng_before.wars.is_empty(), "ENG no wars @tick6");
    assert_eq!(rel(&ger_before.relations, "ENG").at_war, None);

    let ger = diplomacy_panel_from_frame(&tl.frame(hours[7]).unwrap(), named(&tl.frame(hours[7]).unwrap(), "GER"));
    let eng = diplomacy_panel_from_frame(&tl.frame(hours[7]).unwrap(), named(&tl.frame(hours[7]).unwrap(), "ENG"));
    assert_eq!(ger.wars, vec!["ENG".to_string()], "GER belligerents @tick7");
    assert_eq!(
        eng.wars,
        vec!["GER".to_string(), "ITA".to_string()],
        "ENG belligerents @tick7"
    );

    assert_eq!(rel(&ger.relations, "ENG").at_war, Some(true));
    assert_eq!(rel(&ger.relations, "FRA").at_war, Some(false));
    assert_eq!(rel(&ger.relations, "ITA").at_war, Some(false));
    assert_eq!(rel(&ger.relations, "SOV").at_war, Some(false));
}

#[test]
fn relation_row_flags_are_fail_closed() {
    let tl = fixture("fx_war_faction_matrix");
    let hours = tl.hours().to_vec();
    let ger = diplomacy_panel_from_frame(&tl.frame(hours[7]).unwrap(), named(&tl.frame(hours[7]).unwrap(), "GER"));

    let ita = rel(&ger.relations, "ITA");
    assert_eq!(ita.same_faction, Some(true));
    assert_eq!(ita.at_war, Some(false));
    assert_eq!(ita.embargo, None);
    assert_eq!(ita.military_access, None);
    assert_eq!(ita.guarantee, None);

    let eng = rel(&ger.relations, "ENG");
    assert_eq!(eng.at_war, Some(true));
    assert_eq!(eng.same_faction, Some(false));

    assert_eq!(ger.relations.len(), 4);
    let others: Vec<&str> = ger.relations.iter().map(|r| r.other_tag.as_str()).collect();
    assert_eq!(others, vec!["ENG", "FRA", "ITA", "SOV"]);
}

#[test]
fn absent_pov_renders_fail_closed_panel() {
    let tl = fixture("fx_war_faction_matrix");
    let hours = tl.hours().to_vec();
    let usa = diplomacy_panel_from_frame(&tl.frame(hours[7]).unwrap(), named(&tl.frame(hours[7]).unwrap(), "USA"));

    assert!(usa.available);
    assert_eq!(usa.factions.len(), 2);
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
            r#"{{"schema_version":"1","game_id":"{game}","date":"1936.1.1.{}","hour_index":{hour},"tag_db_id_index":{{"country_composition_version":2}},"countries":{countries},"diplomacy_world":{{"domain53_extra":{extra}}}}}"#,
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
        let panel = diplomacy_panel_from_frame(&timeline.frame(hour).unwrap(), named(&timeline.frame(hour).unwrap(), "GER"));
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
    portable_test_fs_retry::remove_dir_all(root).unwrap();
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

    let nonempty = diplomacy_panel_from_frame(&frame, named(&frame, "1"));
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

/// A SUBJECT ROW NAMING ONE COUNTRY BESIDE ANOTHER'S ID REACHES NO PANEL.
///
/// The subject families withhold a contended NAME and keep the row, because the
/// two ids identify it -- and the guard that enforces "the ids are this row's"
/// returned early whenever the name was not CONTENDED. So on a frame where
/// nothing is contended at all, `GER` beside POL's id made only the NAME
/// unobtained: the row stayed measured carrying 441, and `diplomacy_panel_from_frame`
/// matches these rows to a viewer BY ID, so GER's raw subject measurement was
/// rendered in POL's panel.
#[test]
fn a_subject_row_naming_one_country_beside_anothers_id_reaches_no_panel() {
    // kills: in `subject_row_keeps_an_unclaimed_id`, return `None` instead of
    // asking `CountryIdentityRef::refusal` when the name is not contended --
    // POL's panel shows GER's row.
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "game_ui_forged_subject_{}_{}",
        std::process::id(),
        n
    ));
    let game = "forged_subject";
    let dir = root.join(game);
    fs::create_dir_all(&dir).unwrap();
    // The ONLY difference from the control below is the overlord id: 1 is
    // GER's own, 441 is POL's.
    let snapshot = |overlord_id: u32| {
        format!(
            r#"{{
          "schema_version":"1",
          "game_id":"forged_subject",
          "date":"1940.2.14.20",
          "hour_index":900,
          "tag_db_id_index":{{"country_composition_version":2}},
          "countries":{{
            "GER":{{"_r2_tag_db_id":1}},
            "POL":{{"_r2_tag_db_id":441}},
            "BUL":{{"_r2_tag_db_id":22}}
          }},
          "diplomacy_world":{{"domain53_extra":{{
            "subject_autonomy_rows":[{{
              "overlord_tag_db_id":{overlord_id},
              "subject_tag_db_id":22
            }}],
            "subject_autonomy_current_status":"measured_nonempty",
            "subject_autonomy_current_rows":[{{
              "overlord":"GER",
              "subject":"BUL",
              "overlord_tag_db_id":{overlord_id},
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
            }}]
          }}}}
        }}"#
        )
    };
    fs::write(dir.join("000000000900.snapshot.json"), snapshot(1)).unwrap();
    fs::write(
        dir.join("index.json"),
        r#"{"game_id":"forged_subject","schema_version":"1","snapshots":[{"hour_index":900,"date":"1940.2.14.20","file":"000000000900.snapshot.json"}]}"#,
    )
    .unwrap();

    // THE CONTROL: GER's own id. The row publishes and BUL is its counterpart,
    // or the refusal below proves nothing.
    let timeline = WorldTimeline::open(open_recording(&root).unwrap()).unwrap();
    let frame = timeline.frame(900).unwrap();
    let ger = diplomacy_panel_from_frame(&frame, named(&frame, "GER"));
    assert_eq!(ger.subject_autonomy.len(), 1, "the control row must publish");
    assert_eq!(ger.subject_autonomy[0].counterpart.as_deref(), Some("BUL"));
    let pol = diplomacy_panel_from_frame(&frame, named(&frame, "POL"));
    assert!(
        pol.subject_autonomy.is_empty(),
        "and POL owns no subject on the control frame"
    );
    drop(frame);
    drop(timeline);

    // ...AND THE FORGERY: the same row, the same measurement, POL's id under
    // GER's name.
    fs::write(dir.join("000000000900.snapshot.json"), snapshot(441)).unwrap();
    let timeline = WorldTimeline::open(open_recording(&root).unwrap()).unwrap();
    let frame = timeline.frame(900).unwrap();
    for tag in ["GER", "POL", "BUL"] {
        let panel = diplomacy_panel_from_frame(&frame, named(&frame, tag));
        assert!(
            panel.subject_autonomy.is_empty(),
            "a row naming GER beside POL's id belongs to neither, and reaches              no panel at all -- {tag} was shown {:?}",
            panel.subject_autonomy
        );
    }
    assert!(
        frame.coverage_warnings().iter().any(|warning| {
            warning.contains("subject_autonomy_current_rows[0]")
                && warning.contains("belong to two different countries")
        }),
        "and the fold is told which fact it hit: {:?}",
        frame.coverage_warnings()
    );
    drop(frame);
    drop(timeline);
    portable_test_fs_retry::remove_dir_all(root).unwrap();
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
          "tag_db_id_index":{"country_composition_version":2},
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
    let ger = diplomacy_panel_from_frame(&frame, named(&frame, "GER"));
    assert!(ger.subject_autonomy_available);
    assert_eq!(ger.subject_autonomy.len(), 1);
    assert!(ger.subject_autonomy[0].pov_is_overlord);
    assert_eq!(ger.subject_autonomy[0].counterpart.as_deref(), Some("BUL"));
    assert_eq!(ger.subject_autonomy[0].ratio_raw, 10_000);
    assert_eq!(
        ger.subject_autonomy[0].direction,
        Some(SubjectAutonomyDirection::TowardNext)
    );

    let bul = diplomacy_panel_from_frame(&frame, named(&frame, "BUL"));
    assert!(bul.subject_autonomy_available);
    assert_eq!(bul.subject_autonomy.len(), 1);
    assert!(!bul.subject_autonomy[0].pov_is_overlord);
    assert_eq!(bul.subject_autonomy[0].counterpart.as_deref(), Some("GER"));

    let unrelated = diplomacy_panel_from_frame(&frame, named(&frame, "ENG"));
    assert!(unrelated.subject_autonomy_available);
    assert!(unrelated.subject_autonomy.is_empty());
    drop(frame);
    drop(timeline);
    portable_test_fs_retry::remove_dir_all(root).unwrap();
}
