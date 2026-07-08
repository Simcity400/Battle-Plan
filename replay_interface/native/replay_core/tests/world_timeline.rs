//! Integration tests over synthetic fixtures: a 3-hour snapshot store
//! (directory and packed `.battleplan`), a compact-stream-only production
//! container (must fail closed), scrub semantics, and the Absent /
//! Unobtained / Measured trichotomy.

use replay_core::{
    open_recording, open_recording_game, Leaf, MapLayerSource, SourceError, WorldError, WorldFrame,
    WorldTimeline,
};
use serde_json::{json, Value};
use snapshot::{pack, PackEntry, PackIdentity};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const GAME: &str = "fixture_game";

fn unique_temp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "replay_core_{tag}_{}_{n}_{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

// ---------------------------------------------------------------------------
// fixture: 3 hours (100, 101, 103) exercising ownership change, division
// move + disappearance, mirror-vs-walk fallback, and the leaf trichotomy
// ---------------------------------------------------------------------------

fn snapshot_100() -> Value {
    json!({
        "schema_version": "1",
        "game_id": GAME,
        "date": "1936.1.5.4",
        "hour_index": 100,
        "save_version": null,
        "provenance": {
            "collector": {
                "captured": true,
                "domains": ["clock", "state_ownership", "province_control", "divisions", "combats"]
            },
            "memory": null,
            "save": { "captured": true, "domains": ["economy"] },
            "notes": ["synthetic fixture"]
        },
        "countries": {
            "GER": {
                // measured ZERO stability: must stay Measured(0.0), never Unobtained
                "politics": { "stability": 0.0, "war_support": 0.55,
                              "political_power": 12.5, "ruling_party": "fascism" },
                "economy": { "fuel": { "current": 1000.0, "max": 5000.0 } },
                "military": { "division_count": 2 }
            },
            "SOV": {
                "politics": { "stability": null, "war_support": null,
                              "political_power": null, "ruling_party": null },
                "economy": { "fuel": { "current": null, "max": null } },
                "military": { "division_count": 0 }
            }
        },
        "states": { "10": { "owner": "GER" }, "20": { "owner": "GER" }, "30": { "owner": null } },
        "provinces": { "1000": { "controller": "GER" }, "1001": { "controller": "GER" },
                       "2000": { "controller": "SOV" } },
        "divisions": [
            { "div_id": 5, "div_type": 1, "owner": "GER", "location_province": 1000,
              "strength": 1.0, "organisation": 30.0, "in_combat": false },
            { "div_id": 6, "owner": "GER", "location_province": 1001,
              "strength": 0.8, "organisation": null }
        ],
        "combats": [],
        "map": {
            "owner_by_state": { "10": "GER", "20": "GER", "30": null },
            "control_by_province": { "1000": "GER", "1001": "GER", "2000": "SOV" }
        }
    })
}

fn snapshot_101() -> Value {
    json!({
        "schema_version": "1",
        "game_id": GAME,
        "date": "1936.1.5.5",
        "hour_index": 101,
        "save_version": null,
        "provenance": {
            "collector": {
                "captured": true,
                "domains": ["clock", "state_ownership", "province_control", "divisions", "combats"]
            },
            "memory": null,
            "save": { "captured": true, "domains": ["economy"] },
            "notes": []
        },
        "countries": {
            "GER": {
                // same leaf UNOBTAINED here (explicit null) vs measured 0.0 at hour 100
                "politics": { "stability": null, "war_support": 0.55,
                              "political_power": 13.0, "ruling_party": "fascism" },
                "economy": { "fuel": { "current": 980.0, "max": 5000.0 } },
                "military": { "division_count": 2 }
            },
            "SOV": {
                "politics": { "stability": null, "war_support": null,
                              "political_power": null, "ruling_party": null },
                "economy": { "fuel": { "current": null, "max": null } },
                "military": { "division_count": 1 }
            }
        },
        "states": { "10": { "owner": "GER" }, "20": { "owner": "GER" }, "30": { "owner": null } },
        "provinces": { "1000": { "controller": "GER" }, "1001": { "controller": "GER" },
                       "2000": { "controller": "SOV" } },
        "divisions": [
            { "div_id": 5, "div_type": 1, "owner": "GER", "location_province": 1001,
              "strength": 0.9, "organisation": 25.0, "in_combat": true },
            { "div_id": 6, "owner": "GER", "location_province": 1001,
              "strength": 0.8, "organisation": null },
            // div_type null (unobtained, not absent): bare-id identity with a
            // coverage warning, never silent
            { "div_id": 7, "div_type": null, "owner": "SOV", "location_province": 2000,
              "strength": 1.0 }
        ],
        "combats": [
            { "combat_id": 900, "kind": "land", "location_province": 1001,
              "attackers": [5], "defenders": [7] }
        ],
        // mirrors explicitly null -> the typed views must fall back to
        // walking states.*.owner / provinces.*.controller
        "map": { "owner_by_state": null, "control_by_province": null }
    })
}

fn snapshot_103() -> Value {
    json!({
        "schema_version": "1",
        "game_id": GAME,
        "date": "1936.1.5.7",
        "hour_index": 103,
        "save_version": null,
        "provenance": {
            "collector": {
                "captured": true,
                "domains": ["clock", "state_ownership", "province_control", "divisions", "combats"]
            },
            "memory": null,
            "save": { "captured": true, "domains": ["economy"] },
            "notes": []
        },
        "countries": {
            "GER": {
                // "stability" key ABSENT entirely; war_support present but
                // wrong-typed (string) -> Unobtained + coverage warning
                "politics": { "war_support": "high",
                              "political_power": 20.0, "ruling_party": "fascism" },
                "economy": { "fuel": { "current": 900.0, "max": 5000.0 } },
                "military": { "division_count": 1 }
            },
            "SOV": {
                "politics": { "stability": null, "war_support": null,
                              "political_power": null, "ruling_party": null },
                "economy": { "fuel": { "current": null, "max": null } },
                "military": { "division_count": 0 }
            },
            // present-but-null country: leaves are unobtained, tag must not
            // vanish from the typed view
            "ITA": null
        },
        "states": { "10": { "owner": "SOV" }, "20": { "owner": "GER" }, "30": { "owner": null } },
        "provinces": { "1000": { "controller": "SOV" }, "1001": { "controller": "GER" },
                       "2000": { "controller": "SOV" } },
        "divisions": [
            { "div_id": 6, "owner": "GER", "location_province": 1001,
              "strength": 0.8, "organisation": null }
        ],
        // malformed attacker element: the WHOLE list must fail closed to
        // Unobtained (a partial list would be a fabricated participant set)
        "combats": [
            { "combat_id": null, "kind": "land", "location_province": 1001,
              "attackers": [6, "bad"], "defenders": [] }
        ],
        "map": {
            "owner_by_state": { "10": "SOV", "20": "GER", "30": null },
            "control_by_province": { "1000": "SOV", "1001": "GER", "2000": "SOV" }
        }
    })
}

fn fixture_index() -> Value {
    json!({
        "game_id": GAME,
        "schema_version": "1",
        "snapshots": [
            { "hour_index": 100, "date": "1936.1.5.4", "file": "000000000100.snapshot.json" },
            { "hour_index": 101, "date": "1936.1.5.5", "file": "000000000101.snapshot.json" },
            { "hour_index": 103, "date": "1936.1.5.7", "file": "000000000103.snapshot.json" },
            // no hour_index: must be skipped with a warning, never guessed
            { "file": "orphan.snapshot.json" }
        ]
    })
}

/// `(file name, bytes, pack role, pack hour_index)` for every fixture file.
fn fixture_files() -> Vec<(String, Vec<u8>, &'static str, Option<i64>)> {
    vec![
        (
            "index.json".to_string(),
            serde_json::to_vec(&fixture_index()).unwrap(),
            "index",
            None,
        ),
        (
            "000000000100.snapshot.json".to_string(),
            serde_json::to_vec(&snapshot_100()).unwrap(),
            "snapshot",
            Some(100),
        ),
        (
            "000000000101.snapshot.json".to_string(),
            serde_json::to_vec(&snapshot_101()).unwrap(),
            "snapshot",
            Some(101),
        ),
        (
            "000000000103.snapshot.json".to_string(),
            serde_json::to_vec(&snapshot_103()).unwrap(),
            "snapshot",
            Some(103),
        ),
    ]
}

fn write_dir_store(root: &Path) {
    let game_dir = root.join(GAME);
    fs::create_dir_all(&game_dir).unwrap();
    for (name, bytes, _, _) in fixture_files() {
        fs::write(game_dir.join(&name), &bytes).unwrap();
    }
}

fn write_container(path: &Path) {
    let entries: Vec<PackEntry> = fixture_files()
        .into_iter()
        .map(|(name, bytes, role, hour_index)| PackEntry {
            path: format!("payload/{GAME}/{name}"),
            bytes,
            role,
            hour_index,
        })
        .collect();
    let identity = PackIdentity {
        schema_version: Some("1".to_string()),
        ..Default::default()
    };
    fs::write(path, pack(GAME, &entries, &identity, false).unwrap()).unwrap();
}

fn open_timeline(path: &Path) -> WorldTimeline {
    WorldTimeline::open(open_recording(path).unwrap()).unwrap()
}

// ---------------------------------------------------------------------------
// typed views + trichotomy
// ---------------------------------------------------------------------------

#[test]
fn dir_store_typed_views_and_trichotomy() {
    let root = unique_temp_dir("dir_views");
    write_dir_store(&root);
    let timeline = open_timeline(&root);

    assert_eq!(timeline.game_id(), GAME);
    assert_eq!(timeline.hours(), &[100, 101, 103]);
    assert_eq!(timeline.first_hour(), 100);
    assert_eq!(timeline.last_hour(), 103);
    assert_eq!(timeline.warnings().len(), 1);
    assert!(timeline.warnings()[0].contains("orphan.snapshot.json"));

    let f100 = timeline.frame(100).unwrap();
    let f101 = timeline.frame(101).unwrap();
    let f103 = timeline.frame(103).unwrap();

    // map control: mirror preferred, null mirror falls back to domain walk,
    // and the source used is recorded
    assert_eq!(f100.owner_by_state_source(), MapLayerSource::MapMirror);
    assert_eq!(f101.owner_by_state_source(), MapLayerSource::DomainWalk);
    assert_eq!(f103.owner_by_state_source(), MapLayerSource::MapMirror);
    assert_eq!(
        f101.control_by_province_source(),
        MapLayerSource::DomainWalk
    );

    // state ownership change between hours
    assert_eq!(
        f100.owner_by_state()[&10],
        Leaf::Measured("GER".to_string())
    );
    assert_eq!(
        f101.owner_by_state()[&10],
        Leaf::Measured("GER".to_string())
    );
    assert_eq!(
        f103.owner_by_state()[&10],
        Leaf::Measured("SOV".to_string())
    );
    assert_eq!(f100.owner_by_state()[&30], Leaf::Unobtained);
    assert_eq!(
        f100.control_by_province()[&2000],
        Leaf::Measured("SOV".to_string())
    );
    assert_eq!(
        f103.control_by_province()[&1000],
        Leaf::Measured("SOV".to_string())
    );

    // division 5:1 moves (1000 -> 1001) then disappears; division 6 has no
    // div_type so its identity falls back to the bare id
    let div = |frame: &WorldFrame, uid: &str| {
        frame.divisions().iter().find(|d| d.div_uid == uid).cloned()
    };
    let d5_100 = div(&f100, "5:1").unwrap();
    assert_eq!(d5_100.location_province, Leaf::Measured(1000));
    assert_eq!(d5_100.in_combat, Leaf::Measured(false));
    let d5_101 = div(&f101, "5:1").unwrap();
    assert_eq!(d5_101.location_province, Leaf::Measured(1001));
    assert_eq!(d5_101.in_combat, Leaf::Measured(true));
    assert!(div(&f103, "5:1").is_none());
    let d6 = div(&f100, "6").unwrap();
    assert_eq!(d6.owner, Leaf::Measured("GER".to_string()));
    assert_eq!(d6.organisation, Leaf::Unobtained); // explicit null
    assert_eq!(d6.in_combat, Leaf::Absent); // key never captured
                                            // div_type null (not absent): bare-id identity, surfaced as a warning
    let d7 = div(&f101, "7").unwrap();
    assert_eq!(d7.owner, Leaf::Measured("SOV".to_string()));
    assert!(f101
        .coverage_warnings()
        .iter()
        .any(|w| w.contains("divisions[2].div_type is null")));

    // combats
    assert!(f100.combats().is_empty());
    assert_eq!(f101.combats().len(), 1);
    let combat = &f101.combats()[0];
    assert_eq!(combat.combat_id, Leaf::Measured(900));
    assert_eq!(combat.kind, Leaf::Measured("land".to_string()));
    assert_eq!(combat.location_province, Leaf::Measured(1001));
    assert_eq!(combat.attackers, Leaf::Measured(vec![5]));
    assert_eq!(combat.defenders, Leaf::Measured(vec![7]));
    // a malformed participant element poisons the WHOLE list (fail-closed),
    // while a measured-empty list stays Measured([])
    assert_eq!(f103.combats().len(), 1);
    let bad_combat = &f103.combats()[0];
    assert_eq!(bad_combat.combat_id, Leaf::Unobtained);
    assert_eq!(bad_combat.attackers, Leaf::Unobtained);
    assert_eq!(bad_combat.defenders, Leaf::Measured(vec![]));
    assert!(f103
        .coverage_warnings()
        .iter()
        .any(|w| w.contains("combats[0].attackers[1]")));

    // country core trichotomy: measured 0.0 vs explicit null vs absent key
    let ger_100 = f100.country("GER").unwrap();
    let ger_101 = f101.country("GER").unwrap();
    let ger_103 = f103.country("GER").unwrap();
    assert_eq!(ger_100.stability, Leaf::Measured(0.0));
    assert_eq!(ger_101.stability, Leaf::Unobtained);
    assert_eq!(ger_103.stability, Leaf::Absent);
    assert_ne!(ger_100.stability, ger_101.stability);
    assert_ne!(ger_101.stability, ger_103.stability);
    assert_ne!(ger_100.stability, ger_103.stability);

    // wrong-typed leaf: Unobtained in the view PLUS a coverage warning
    assert_eq!(ger_103.war_support, Leaf::Unobtained);
    assert!(f103
        .coverage_warnings()
        .iter()
        .any(|w| w.contains("countries.GER.politics.war_support")));
    // present-but-null country entry: all leaves unobtained, tag kept
    let ita_103 = f103.country("ITA").unwrap();
    assert_eq!(ita_103.stability, Leaf::Unobtained);
    assert_eq!(ita_103.division_count, Leaf::Unobtained);
    assert!(f103.country("USA").is_none()); // absent stays absent

    // fully well-formed frames carry no coverage warnings
    assert!(f100.coverage_warnings().is_empty());
    // hour 101's only finding is the null div_type identity degradation
    assert_eq!(f101.coverage_warnings().len(), 1);

    // remaining core values
    assert_eq!(ger_100.ruling_party, Leaf::Measured("fascism".to_string()));
    assert_eq!(ger_100.fuel_current, Leaf::Measured(1000.0));
    assert_eq!(ger_100.fuel_max, Leaf::Measured(5000.0));
    assert_eq!(ger_100.division_count, Leaf::Measured(2));
    let sov_100 = f100.country("SOV").unwrap();
    assert_eq!(sov_100.stability, Leaf::Unobtained);
    assert_eq!(sov_100.fuel_current, Leaf::Unobtained);
    assert_eq!(sov_100.division_count, Leaf::Measured(0));

    // provenance channels + notes
    let prov = f100.provenance();
    let collector = &prov.channels["collector"];
    assert_eq!(collector.captured, Leaf::Measured(true));
    assert!(collector.domains.iter().any(|d| d == "state_ownership"));
    assert_eq!(prov.channels["memory"].captured, Leaf::Unobtained);
    assert_eq!(prov.channels["save"].domains, vec!["economy".to_string()]);
    assert_eq!(prov.notes, vec!["synthetic fixture".to_string()]);

    // raw domains stay reachable, untouched
    assert!(f100.domain("states").is_some());
    assert!(f100.domain("nonexistent_domain").is_none());

    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// dir store <-> container equivalence
// ---------------------------------------------------------------------------

fn assert_frames_equal(a: &WorldFrame, b: &WorldFrame) {
    assert_eq!(a.hour_index(), b.hour_index());
    assert_eq!(a.date(), b.date());
    assert_eq!(a.save_version(), b.save_version());
    assert_eq!(a.raw(), b.raw());
    assert_eq!(a.owner_by_state(), b.owner_by_state());
    assert_eq!(a.owner_by_state_source(), b.owner_by_state_source());
    assert_eq!(a.control_by_province(), b.control_by_province());
    assert_eq!(
        a.control_by_province_source(),
        b.control_by_province_source()
    );
    assert_eq!(a.divisions(), b.divisions());
    assert_eq!(a.combats(), b.combats());
    assert_eq!(a.countries(), b.countries());
    assert_eq!(a.provenance(), b.provenance());
    assert_eq!(a.coverage_warnings(), b.coverage_warnings());
}

#[test]
fn container_and_dir_store_agree() {
    let root = unique_temp_dir("backend_agree");
    let dir_root = root.join("store");
    write_dir_store(&dir_root);
    let container_path = root.join("fixture.battleplan");
    write_container(&container_path);

    let from_dir = open_timeline(&dir_root);
    let from_container = open_timeline(&container_path);

    assert_eq!(from_dir.game_id(), from_container.game_id());
    assert_eq!(from_dir.hours(), from_container.hours());
    assert_eq!(from_dir.warnings(), from_container.warnings());
    for &hour in from_dir.hours() {
        let a = from_dir.frame(hour).unwrap();
        let b = from_container.frame(hour).unwrap();
        assert_frames_equal(&a, &b);
    }

    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// compact-stream production container fails closed
// ---------------------------------------------------------------------------

#[test]
fn compact_stream_container_fails_closed() {
    let root = unique_temp_dir("compact_closed");
    let game = "native_session_fixture";
    let index = json!({
        "game_id": game,
        "schema_version": "native_collector_session_v1",
        "snapshots": []
    });
    // Mirrors extractor/native/recorder_host's production layout.
    let timeline = json!({
        "format": "hoi4_compact_stream_timeline_v1",
        "stream_format": "compact",
        "stream": "collector_raw/compact_stream.jsonl",
        "session_manifest": "recorder/native_session_manifest.json",
        "raw_stream_role": "collector_compact_stream",
        "session_manifest_role": "recorder_manifest"
    });
    let entries = vec![
        PackEntry {
            path: format!("payload/{game}/index.json"),
            bytes: serde_json::to_vec(&index).unwrap(),
            role: "index",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game}/timeline.json"),
            bytes: serde_json::to_vec(&timeline).unwrap(),
            role: "timeline",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game}/collector_raw/compact_stream.jsonl"),
            bytes: b"synthetic-compact-stream-bytes\n".to_vec(),
            role: "collector_compact_stream",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game}/recorder/native_session_manifest.json"),
            bytes: serde_json::to_vec(&json!({ "session": "synthetic fixture" })).unwrap(),
            role: "recorder_manifest",
            hour_index: None,
        },
    ];
    let identity = PackIdentity {
        schema_version: Some("native_collector_session_v1".to_string()),
        ..Default::default()
    };
    let path = root.join("compact_only.battleplan");
    fs::write(&path, pack(game, &entries, &identity, true).unwrap()).unwrap();

    let err = open_recording(&path).err().expect("must fail closed");
    match &err {
        SourceError::Unsupported(msg) => {
            assert!(msg.contains("collector_compact_stream"), "msg: {msg}");
            assert!(msg.contains("compact"), "msg: {msg}");
            assert!(msg.contains("HOI4REPLAY_CONTAINER_SPEC"), "msg: {msg}");
            assert!(msg.contains("extraction"), "msg: {msg}");
        }
        other => panic!("expected Unsupported, got {other:?}"),
    }

    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// scrub semantics
// ---------------------------------------------------------------------------

#[test]
fn scrub_state_at_semantics() {
    let root = unique_temp_dir("scrub");
    write_dir_store(&root);
    let timeline = open_timeline(&root);

    // exact hits and the at-or-before rule for the 102 gap
    assert_eq!(timeline.state_at(100).unwrap().hour_index(), 100);
    assert_eq!(timeline.state_at(101).unwrap().hour_index(), 101);
    assert_eq!(timeline.state_at(102).unwrap().hour_index(), 101);
    assert_eq!(timeline.state_at(103).unwrap().hour_index(), 103);
    assert_eq!(timeline.state_at(5000).unwrap().hour_index(), 103);

    // before the first frame is an error, not a clamp
    match timeline.state_at(99) {
        Err(WorldError::BeforeFirst {
            hour: 99,
            first: 100,
        }) => {}
        other => panic!(
            "expected BeforeFirst, got {:?}",
            other.map(|f| f.hour_index())
        ),
    }

    // frame() is exact-hour only
    match timeline.frame(102) {
        Err(WorldError::NoSuchHour(102)) => {}
        other => panic!(
            "expected NoSuchHour, got {:?}",
            other.map(|f| f.hour_index())
        ),
    }

    // A -> B -> A scrubbing returns consistent values (no stale cache)
    let a1 = timeline.state_at(100).unwrap();
    assert_eq!(a1.owner_by_state()[&10], Leaf::Measured("GER".to_string()));
    let b = timeline.state_at(103).unwrap();
    assert_eq!(b.owner_by_state()[&10], Leaf::Measured("SOV".to_string()));
    let a2 = timeline.state_at(100).unwrap();
    assert_eq!(a2.owner_by_state()[&10], Leaf::Measured("GER".to_string()));
    assert_eq!(a1.owner_by_state(), a2.owner_by_state());
    // and the cache actually reuses the frame rather than rebuilding it
    assert!(Arc::ptr_eq(&a1, &a2));

    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// multi-game stores require explicit selection
// ---------------------------------------------------------------------------

fn write_minimal_game(root: &Path, game: &str) {
    let dir = root.join(game);
    fs::create_dir_all(&dir).unwrap();
    let snap = json!({
        "schema_version": "1",
        "game_id": game,
        "date": "1936.1.1.0",
        "hour_index": 10,
        "countries": {},
        "states": {},
        "provinces": {},
        "divisions": [],
        "combats": [],
        "map": {}
    });
    fs::write(
        dir.join("000000000010.snapshot.json"),
        serde_json::to_vec(&snap).unwrap(),
    )
    .unwrap();
    let index = json!({
        "game_id": game,
        "schema_version": "1",
        "snapshots": [
            { "hour_index": 10, "date": "1936.1.1.0", "file": "000000000010.snapshot.json" }
        ]
    });
    fs::write(dir.join("index.json"), serde_json::to_vec(&index).unwrap()).unwrap();
}

#[test]
fn multi_game_store_requires_selection() {
    let root = unique_temp_dir("multi_game");
    write_minimal_game(&root, "game_a");
    write_minimal_game(&root, "game_b");

    match open_recording(&root) {
        Err(SourceError::MultipleGames(games)) => {
            assert_eq!(games, vec!["game_a".to_string(), "game_b".to_string()]);
        }
        Err(other) => panic!("expected MultipleGames, got {other:?}"),
        Ok(_) => panic!("expected MultipleGames, got a source"),
    }

    let source = open_recording_game(&root, "game_b").unwrap();
    assert_eq!(source.game_id(), "game_b");
    assert_eq!(source.hours(), &[10]);
    let timeline = WorldTimeline::open(source).unwrap();
    assert_eq!(timeline.frame(10).unwrap().hour_index(), 10);

    match open_recording_game(&root, "game_c") {
        Err(SourceError::Open(msg)) => assert!(msg.contains("game_c"), "msg: {msg}"),
        other => panic!(
            "expected Open error, got {:?}",
            other.map(|s| s.game_id().to_string())
        ),
    }

    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// wrong-typed top-level `map` must warn and fall back, never pass silently
// ---------------------------------------------------------------------------

#[test]
fn wrong_typed_map_mirror_falls_back_with_warning() {
    let root = unique_temp_dir("bad_map");
    let game = "bad_map_game";
    let dir = root.join(game);
    fs::create_dir_all(&dir).unwrap();
    let snap = json!({
        "schema_version": "1",
        "game_id": game,
        "date": "1936.1.1.0",
        "hour_index": 10,
        "countries": {},
        "states": { "10": { "owner": "GER" } },
        "provinces": { "1000": { "controller": "GER" } },
        "divisions": [],
        "combats": [],
        // top-level map wrong-typed: must warn and fall back to the walk
        "map": 42
    });
    fs::write(
        dir.join("000000000010.snapshot.json"),
        serde_json::to_vec(&snap).unwrap(),
    )
    .unwrap();
    let index = json!({
        "game_id": game,
        "schema_version": "1",
        "snapshots": [
            { "hour_index": 10, "date": "1936.1.1.0", "file": "000000000010.snapshot.json" }
        ]
    });
    fs::write(dir.join("index.json"), serde_json::to_vec(&index).unwrap()).unwrap();

    let timeline = open_timeline(&root);
    let frame = timeline.frame(10).unwrap();
    assert_eq!(frame.owner_by_state_source(), MapLayerSource::DomainWalk);
    assert_eq!(
        frame.owner_by_state()[&10],
        Leaf::Measured("GER".to_string())
    );
    assert_eq!(
        frame.control_by_province_source(),
        MapLayerSource::DomainWalk
    );
    assert!(frame
        .coverage_warnings()
        .iter()
        .any(|w| w.contains("map: present but wrong-typed")));

    let _ = fs::remove_dir_all(&root);
}
