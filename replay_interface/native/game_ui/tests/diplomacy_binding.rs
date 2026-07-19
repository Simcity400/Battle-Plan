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
use replay_core::{open_recording, Leaf, WorldTimeline};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
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

fn run_extractor_cargo(extractor_native: &Path, temp_dir: &Path, args: &[&str]) {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .current_dir(extractor_native)
        .env("TEMP", temp_dir)
        .env("TMP", temp_dir)
        .args(args)
        .output()
        .expect("launch extractor cargo command");
    assert!(
        output.status.success(),
        "extractor cargo command failed: {} stdout={} stderr={}",
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn relocate_h4cr_stream_hour(stream: &mut [u8], game_hour_raw: i32) -> usize {
    let mut offset = 0usize;
    let mut records = 0usize;
    while offset < stream.len() {
        assert!(
            stream.len() - offset >= 32,
            "truncated H4CR header at byte {offset}"
        );
        assert_eq!(&stream[offset..offset + 4], b"H4CR");
        let header_len =
            u16::from_le_bytes(stream[offset + 6..offset + 8].try_into().unwrap()) as usize;
        let payload_len =
            u32::from_le_bytes(stream[offset + 24..offset + 28].try_into().unwrap()) as usize;
        assert!(header_len >= 32, "invalid H4CR header length {header_len}");
        let next = offset
            .checked_add(header_len)
            .and_then(|value| value.checked_add(payload_len))
            .expect("H4CR record length overflow");
        assert!(
            next <= stream.len(),
            "truncated H4CR record at byte {offset}"
        );
        stream[offset + 16..offset + 20].copy_from_slice(&game_hour_raw.to_le_bytes());
        offset = next;
        records += 1;
    }
    records
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

/// Build the positive binding fixture through the production d53 encoder and
/// recorder fold. The fold deliberately projects only `leader`/`is_leader` to
/// `countries.*.politics.faction`; it does not fabricate the faction name that
/// d53 carries only as a hash.
fn production_faction_roster_binding_timeline() -> (PathBuf, WorldTimeline, i64) {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "game_ui_faction_fold_{}_{}_{}",
        std::process::id(),
        n,
        nanos
    ));
    fs::create_dir_all(&root).unwrap();

    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap();
    let extractor_native = repo_root.join("extractor/native");
    let extractor_temp = root.join("extractor_tmp");
    fs::create_dir_all(&extractor_temp).unwrap();

    let proof_record = root.join("d53.compact.bin");
    let proof_report = root.join("d53.proof.json");
    let proof_manifest = root.join("d53.manifest.json");
    let proof_replay = root.join("d53.battleplan");
    run_extractor_cargo(
        &extractor_native,
        &extractor_temp,
        &[
            "run",
            "--locked",
            "--offline",
            "--quiet",
            "-p",
            "diplomacy_intel_operatives_live",
            "--bin",
            "domain53_proof",
            "--",
            "--out-record",
            proof_record.to_str().unwrap(),
            "--out-report",
            proof_report.to_str().unwrap(),
            "--out-manifest",
            proof_manifest.to_str().unwrap(),
            "--out-replay",
            proof_replay.to_str().unwrap(),
        ],
    );

    let mut stream = fs::read(&proof_record).unwrap();
    assert_eq!(&stream[..4], b"H4CR");
    assert_eq!(u16::from_le_bytes(stream[20..22].try_into().unwrap()), 53);
    let raw_hour = 43_799_999i32 + (1936 * 365 * 24);
    // `domain53_proof` now emits a complete H4CR stream (SESSION_META, D53
    // DATA, FRAME_END), not a single D53 record. Relocate every record header
    // together; relocating only the first header leaves the data at the proof
    // CLI's historical smoke hour and makes the tested hour correctly fold an
    // unavailable roster. Payload bytes remain byte-identical.
    assert!(
        relocate_h4cr_stream_hour(&mut stream, raw_hour) >= 3,
        "the proof stream must include metadata, D53 data, and frame end"
    );
    let stream_path = root.join("collector_stream.bin");
    fs::write(&stream_path, stream).unwrap();

    let game = "faction_roster_production_fold";
    let store_root = root.join("store");
    let game_dir = store_root.join(game);
    fs::create_dir_all(&game_dir).unwrap();
    run_extractor_cargo(
        &extractor_native,
        &extractor_temp,
        &[
            "run",
            "--locked",
            "--offline",
            "--quiet",
            "-p",
            "recorder_host",
            "--bin",
            "stream_fold",
            "--",
            stream_path.to_str().unwrap(),
            game_dir.to_str().unwrap(),
            "--game-id",
            game,
            "--workers",
            "1",
        ],
    );

    let snapshots = fs::read_dir(&game_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".snapshot.json"))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        snapshots.len(),
        1,
        "the one-hour fold must emit one snapshot"
    );
    let snapshot_path = &snapshots[0];
    let file = snapshot_path.file_name().unwrap().to_str().unwrap();
    let hour = file
        .strip_suffix(".snapshot.json")
        .unwrap()
        .parse::<i64>()
        .unwrap();
    fs::write(
        game_dir.join("index.json"),
        format!(
            r#"{{"game_id":"{game}","schema_version":"1","snapshots":[{{"hour_index":{hour},"file":"{file}"}}]}}"#
        ),
    )
    .unwrap();

    let timeline = WorldTimeline::open(open_recording(&store_root).unwrap()).unwrap();
    (root, timeline, hour)
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
    }

    drop(timeline);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn measured_nonempty_real_fold_without_country_names_keeps_d53_roster() {
    let (root, timeline, hour) = production_faction_roster_binding_timeline();
    let frame = timeline.frame(hour).unwrap();
    assert!(
        !frame.diplomacy().countries.is_empty(),
        "the production fold must project the roster member countries"
    );
    assert!(
        frame
            .diplomacy()
            .countries
            .values()
            .all(|country| matches!(&country.faction_name, Leaf::Absent)),
        "the real d53 fold must not fabricate countries.*.politics.faction.name"
    );
    assert_eq!(
        frame
            .diplomacy()
            .faction_members
            .measured()
            .unwrap()
            .get("1"),
        Some(&vec!["1".to_string(), "3".to_string(), "22".to_string()]),
        "the typed view must retain the actual folded d53 member identities"
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
        "the binding must render the authoritative d53 roster even though every country faction name is absent"
    );

    drop(frame);
    drop(timeline);
    fs::remove_dir_all(root).unwrap();
}
