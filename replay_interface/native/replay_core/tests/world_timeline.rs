#![recursion_limit = "512"]

//! Integration tests over synthetic fixtures: a 3-hour snapshot store
//! (directory and packed `.battleplan`), a compact-stream-only production
//! container (must fail closed), scrub semantics, and the Absent /
//! Unobtained / Measured trichotomy.

use replay_core::{
    open_recording, open_recording_game, Leaf, MapLayerSource, SourceError, ViewStatus, WorldError,
    WorldFrame, WorldTimeline,
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
                              "political_power": 12.5, "command_power": 44.0,
                              "ruling_party": "fascism", "is_human": true,
                              "faction": { "name": "Axis", "is_leader": true },
                              "capital_state": 64 },
                "diplomacy": {
                    "wars": [ { "against": ["SOV"], "war_score": 0.1 } ],
                    "non_aggression": ["ITA"],
                    "guarantees": [],
                    "embargoes": ["SOV"],
                    "military_access": [ { "tag": "ITA" } ]
                },
                "economy": {
                    "factories": { "civ": 32, "mil": 41, "dockyard": 5 },
                    "fuel": { "current": 1000.0, "max": 5000.0 },
                    "logistics": { "supply_value": 12.0, "supply_ratio": 0.8 }
                },
                "military": {
                    "division_count": 2,
                    "manpower": { "available": 123456 },
                    "xp": { "army": 10.5, "navy": 3.25, "air": 7.0 },
                    "convoys": [ { "archetype": "convoy", "amount": 91.0 } ]
                },
                "extra": { "cosmetic_tag": "GER_reich", "nukes_total": 2, "nukes_ready_total": 1 }
            },
            "SOV": {
                "politics": { "stability": null, "war_support": null,
                              "political_power": null, "command_power": null,
                              "ruling_party": null, "is_human": false,
                              "faction": { "name": null, "is_leader": null },
                              "capital_state": null },
                "diplomacy": { "wars": [ { "against": ["GER"] } ] },
                "economy": {
                    "factories": { "civ": null, "mil": null, "dockyard": null },
                    "fuel": { "current": null, "max": null }
                },
                "military": {
                    "division_count": 0,
                    "manpower": { "available": null },
                    "xp": { "army": null, "navy": null, "air": null },
                    "convoys": []
                },
                "extra": { "cosmetic_tag": null, "nukes_total": null, "nukes_ready_total": null }
            }
        },
        "states": {
            "10": {
                "owner": "GER",
                "buildings": {
                    "arms_factory": { "level": 3.0, "healthy_level": 2.0, "damaged": true,
                                      "damage_fraction": 0.33, "repair_progress": 0.2,
                                      "under_construction": false },
                    "infrastructure": 5
                },
                "province_buildings": {
                    "1000": {
                        "naval_base": { "level": 2.0, "healthy_level": 2.0, "damaged": false }
                    }
                }
            },
            "20": { "owner": "GER" },
            "30": { "owner": null }
        },
        "provinces": { "1000": { "controller": "GER" }, "1001": { "controller": "GER" },
                       "2000": { "controller": "SOV" } },
        "divisions": [
            { "div_id": 5, "div_type": 1, "owner": "GER", "location_province": 1000,
              "state_id": 10, "template_id": 100, "template_type": 7,
              "template_name": "1. Infanterie", "template_role": "infantry",
              "strength": 1.0, "organisation": 30.0, "moving": true,
              "move_target_province": 1001, "move_progress": 0.4, "in_combat": false,
              "equipment": [ { "archetype": "infantry_equipment", "equip_id": 11,
                               "equip_type": 22, "amount": 900.0 } ] },
            { "div_id": 6, "owner": "GER", "location_province": 1001,
              "strength": 0.8, "organisation": null }
        ],
        "combats": [],
        "session": {
            "player_countries": [ { "tag": "GER", "player": "host" }, { "tag": "ITA", "player": null } ],
            "speed_events": [ { "hour_index": 100, "speed": 3, "paused": false } ],
            "active_dlcs": ["Arms Against Tyranny"],
            "localisation_language": "english"
        },
        "naval_supremacy": {
            "42": { "controller": "GER", "state": "friendly", "supremacy": 0.72 }
        },
        "weather": {
            "by_region": {
                "42": {
                    "weather_kind": "snow", "intensity": 0.8,
                    "snow_level_0_255": 180, "mud_level_0_255": 12,
                    "snow_mud_fade": 0.5, "snow_mud_fade_from": 0.25,
                    "snow_mud_fade_to": 0.75
                }
            }
        },
        "diplomacy_pairs": {
            "GER|SOV": { "at_war": true, "same_faction": false, "nap": false,
                         "access": false, "guarantee": false, "embargo": true,
                         "subject": false },
            "GER|ITA": { "at_war": false, "same_faction": true, "nap": true,
                         "access": true, "guarantee": false, "embargo": false,
                         "subject": false }
        },
        "order_of_battle": {
            "fleets": [
                { "id": "fleet-1", "owner": "GER", "name": "Baltic Fleet",
                  "task_forces": [
                    { "id": "tf-1", "owner": "GER", "location_province": 321,
                      "mission": "patrol", "dominant_hull_class": "destroyer",
                      "ships": [
                        { "id": "ship-1", "definition": "destroyer", "organisation": 0.9,
                          "strength": 0.8, "experience": 0.2, "name": "Z1",
                          "equipment": [[77, 88, 1.0]] }
                      ] }
                  ] }
            ],
            "air_wings": [
                { "id": "wing-1", "owner": "GER", "name": "JG 1", "count": 120,
                  "experience": 0.3, "manpower": 240, "mission": "air_superiority",
                  "strategic_region": 42, "air_base": "1000:1", "state_id": 10,
                  "plane_definition": "fighter_equipment_0", "effective_planes": 118,
                  "equipment": [ { "equip_id": 501, "equip_type": 2, "archetype": "fighter",
                                   "amount": 120.0 } ] }
            ]
        },
        "visibility": {
            "GER": {
                "provinces": { "1000": true, "2000": "fogged", "2001": null },
                "spotted_units": { "divisions": ["7"], "fleets": ["tf-9"], "wings": [] }
            }
        },
        "world": { "world_tension": 0.42 },
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
                              "political_power": 13.0, "command_power": null,
                              "ruling_party": "fascism", "faction": { "name": null, "is_leader": null },
                              "capital_state": null },
                "economy": { "factories": { "civ": null, "mil": null, "dockyard": null },
                             "fuel": { "current": 980.0, "max": 5000.0 } },
                "military": { "division_count": 2, "manpower": { "available": null },
                              "xp": { "army": null, "navy": null, "air": null },
                              "convoys": null },
                "extra": { "cosmetic_tag": null, "nukes_total": null, "nukes_ready_total": null }
            },
            "SOV": {
                "politics": { "stability": null, "war_support": null,
                              "political_power": null, "ruling_party": null },
                "economy": { "fuel": { "current": null, "max": null } },
                "military": { "division_count": 1 }
            }
        },
        "states": { "10": { "owner": "GER", "buildings": null },
                    "20": { "owner": "GER" }, "30": { "owner": null } },
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
              "strength": 1.0, "equipment": null }
        ],
        "combats": [
            { "combat_id": 900, "kind": "land", "location_province": 1001,
              "attackers": [5], "defenders": [7] }
        ],
        "session": null,
        "naval_supremacy": null,
        "weather": null,
        "diplomacy_pairs": {
            "GER|SOV": { "at_war": null, "same_faction": null, "nap": null,
                         "access": null, "guarantee": null, "embargo": null,
                         "subject": null }
        },
        "order_of_battle": null,
        "visibility": null,
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

fn write_absence_unavailable_store(root: &Path) {
    let game = "absence_unavailable";
    let dir = root.join(game);
    fs::create_dir_all(&dir).unwrap();
    let absent = json!({
        "schema_version": "1",
        "game_id": game,
        "date": "1936.1.1.0",
        "hour_index": 10
    });
    let unavailable = json!({
        "schema_version": "1",
        "game_id": game,
        "date": "1936.1.1.1",
        "hour_index": 11,
        "countries": null,
        "states": null,
        "provinces": null,
        "divisions": null,
        "combats": null,
        "map": null,
        "session": null,
        "naval_supremacy": null,
        "weather": null,
        "diplomacy_pairs": null,
        "order_of_battle": null,
        "visibility": null
    });
    fs::write(
        dir.join("000000000010.snapshot.json"),
        serde_json::to_vec(&absent).unwrap(),
    )
    .unwrap();
    fs::write(
        dir.join("000000000011.snapshot.json"),
        serde_json::to_vec(&unavailable).unwrap(),
    )
    .unwrap();
    let index = json!({
        "game_id": game,
        "schema_version": "1",
        "snapshots": [
            { "hour_index": 10, "date": "1936.1.1.0", "file": "000000000010.snapshot.json" },
            { "hour_index": 11, "date": "1936.1.1.1", "file": "000000000011.snapshot.json" }
        ]
    });
    fs::write(dir.join("index.json"), serde_json::to_vec(&index).unwrap()).unwrap();
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

    // New A3 typed views: unit positions/composition, metadata, weather,
    // diplomacy, topbar, buildings, and country-POV visibility.
    assert_eq!(f100.division_units_status(), ViewStatus::Present);
    let d5_unit = f100
        .division_units()
        .iter()
        .find(|d| d.div_uid == "5:1")
        .unwrap();
    assert_eq!(
        d5_unit.template_role,
        Leaf::Measured("infantry".to_string())
    );
    assert_eq!(d5_unit.moving, Leaf::Measured(true));
    assert_eq!(d5_unit.move_target_province, Leaf::Measured(1001));
    assert_eq!(
        d5_unit.equipment.measured().unwrap()[0].amount,
        Leaf::Measured(900.0)
    );
    assert_eq!(f101.division_units_status(), ViewStatus::Present);
    let d7_unit = f101
        .division_units()
        .iter()
        .find(|d| d.div_uid == "7")
        .unwrap();
    assert_eq!(d7_unit.equipment, Leaf::Unobtained);

    assert_eq!(f100.fleets_status(), ViewStatus::Present);
    assert_eq!(f100.fleets()[0].task_force_uid, "tf-1");
    assert_eq!(f100.fleets()[0].ship_count, Leaf::Measured(1));
    assert_eq!(
        f100.fleets()[0].dominant_hull_class,
        Leaf::Measured("destroyer".to_string())
    );
    assert_eq!(
        f100.fleets()[0].ships.measured().unwrap()[0].definition,
        Leaf::Measured("destroyer".to_string())
    );
    assert_eq!(f100.wings_status(), ViewStatus::Present);
    assert_eq!(f100.wings()[0].wing_uid, "wing-1");
    assert_eq!(
        f100.wings()[0].mission_kind,
        Leaf::Measured("air_superiority".to_string())
    );
    assert_eq!(f100.wings()[0].strategic_region_id, Leaf::Measured(42));
    assert_eq!(f101.fleets_status(), ViewStatus::DomainUnavailable);
    assert_eq!(f101.wings_status(), ViewStatus::DomainUnavailable);

    let session = f100.session_metadata();
    assert_eq!(session.status, ViewStatus::Present);
    assert_eq!(
        session.player_countries.measured().unwrap()[0].tag,
        Leaf::Measured("GER".to_string())
    );
    assert_eq!(
        session.speed_events.measured().unwrap()[0].speed,
        Leaf::Measured(3)
    );
    assert_eq!(
        session.active_dlcs,
        Leaf::Measured(vec!["Arms Against Tyranny".to_string()])
    );
    assert_eq!(
        session.localisation_language,
        Leaf::Measured("english".to_string())
    );
    assert_eq!(
        f101.session_metadata().status,
        ViewStatus::DomainUnavailable
    );
    assert_eq!(f101.session_metadata().player_countries, Leaf::Unobtained);

    assert_eq!(f100.naval_supremacy_status(), ViewStatus::Present);
    assert_eq!(
        f100.naval_supremacy_by_region()[&42].controller,
        Leaf::Measured("GER".to_string())
    );
    assert_eq!(
        f100.naval_supremacy_by_region()[&42].supremacy,
        Leaf::Measured(0.72)
    );
    assert_eq!(f101.naval_supremacy_status(), ViewStatus::DomainUnavailable);

    assert_eq!(f100.weather_status(), ViewStatus::Present);
    let weather = &f100.weather_by_region()[&42];
    assert_eq!(weather.weather_kind, Leaf::Measured("snow".to_string()));
    assert_eq!(weather.snow_level_0_255, Leaf::Measured(180));
    assert_eq!(weather.mud_level_0_255, Leaf::Measured(12));
    assert_eq!(weather.snow_mud_fade, Leaf::Measured(0.5));
    assert_eq!(f101.weather_status(), ViewStatus::DomainUnavailable);

    let ger_identity = f100.country_identity("GER").unwrap();
    assert_eq!(
        ger_identity.cosmetic_tag,
        Leaf::Measured("GER_reich".to_string())
    );
    assert_eq!(
        ger_identity.ruling_ideology,
        Leaf::Measured("fascism".to_string())
    );
    assert_eq!(
        f101.country_identity("GER").unwrap().cosmetic_tag,
        Leaf::Unobtained
    );

    let dip = f100.diplomacy();
    assert_eq!(dip.status, ViewStatus::Present);
    assert_eq!(
        dip.countries["GER"].faction_name,
        Leaf::Measured("Axis".to_string())
    );
    assert_eq!(dip.countries["GER"].capital_state, Leaf::Measured(64));
    let ger_sov = f100.diplomatic_pair_state("GER", "SOV").unwrap();
    assert_eq!(ger_sov.at_war, Leaf::Measured(true));
    assert_eq!(ger_sov.same_faction, Leaf::Measured(false));
    assert_eq!(ger_sov.embargo, Leaf::Measured(true));
    let ger_ita = f100.diplomatic_pair_state("GER", "ITA").unwrap();
    assert_eq!(ger_ita.same_faction, Leaf::Measured(true));
    assert_eq!(ger_ita.non_aggression_pact, Leaf::Measured(true));
    assert_eq!(
        f101.diplomatic_pair_state("GER", "SOV").unwrap().at_war,
        Leaf::Unobtained
    );

    let topbar = f100.topbar_scalars("GER").unwrap();
    assert_eq!(topbar.political_power, Leaf::Measured(12.5));
    assert_eq!(topbar.command_power, Leaf::Measured(44.0));
    assert_eq!(topbar.manpower_available, Leaf::Measured(123456));
    assert_eq!(topbar.xp_army, Leaf::Measured(10.5));
    assert_eq!(topbar.factories_civ, Leaf::Measured(32));
    assert_eq!(topbar.factories_mil, Leaf::Measured(41));
    assert_eq!(topbar.factories_dockyard, Leaf::Measured(5));
    assert_eq!(topbar.convoys, Leaf::Measured(91.0));
    assert_eq!(topbar.nukes_total, Leaf::Measured(2));
    assert_eq!(topbar.nukes_ready, Leaf::Measured(1));
    assert_eq!(topbar.world_tension, Leaf::Measured(0.42));
    assert_eq!(topbar.supply_ratio, Leaf::Measured(0.8));
    assert_eq!(
        f101.topbar_scalars("GER").unwrap().convoys,
        Leaf::Unobtained
    );

    assert_eq!(f100.buildings_status(), ViewStatus::Present);
    let state_10_buildings = &f100.buildings_by_state()[&10];
    assert_eq!(state_10_buildings.status, ViewStatus::Present);
    assert_eq!(
        state_10_buildings.state_buildings["arms_factory"].level,
        Leaf::Measured(3.0)
    );
    assert_eq!(
        state_10_buildings.state_buildings["arms_factory"].damaged,
        Leaf::Measured(true)
    );
    assert_eq!(
        state_10_buildings.province_buildings[&1000]["naval_base"].healthy_level,
        Leaf::Measured(2.0)
    );
    assert_eq!(
        f101.buildings_by_state()[&10].status,
        ViewStatus::DomainUnavailable
    );

    assert_eq!(f100.visibility_status(), ViewStatus::Present);
    let ger_visibility = f100.visibility_for("GER");
    assert_eq!(ger_visibility.status, ViewStatus::Present);
    assert_eq!(
        ger_visibility.provinces[&1000],
        Leaf::Measured(replay_core::ProvinceVisibility {
            level: "visible".to_string()
        })
    );
    assert_eq!(
        ger_visibility.provinces[&2000],
        Leaf::Measured(replay_core::ProvinceVisibility {
            level: "fogged".to_string()
        })
    );
    assert_eq!(ger_visibility.provinces[&2001], Leaf::Unobtained);
    assert_eq!(
        f100.spotted_units_for("GER").divisions,
        Leaf::Measured(vec!["7".to_string()])
    );
    assert_eq!(f100.spotted_units_for("GER").wings, Leaf::Measured(vec![]));
    assert_eq!(f100.visibility_for("SOV").status, ViewStatus::DomainAbsent);
    assert_eq!(f101.visibility_status(), ViewStatus::DomainUnavailable);

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

#[test]
fn typed_view_status_distinguishes_absent_from_unavailable_domains() {
    let root = unique_temp_dir("view_status");
    write_absence_unavailable_store(&root);
    let timeline = open_timeline(&root);
    let absent = timeline.frame(10).unwrap();
    let unavailable = timeline.frame(11).unwrap();

    assert_eq!(absent.owner_by_state_status(), ViewStatus::DomainAbsent);
    assert_eq!(
        unavailable.owner_by_state_status(),
        ViewStatus::DomainUnavailable
    );
    assert_eq!(absent.control_by_state_status(), ViewStatus::DomainAbsent);
    assert_eq!(
        unavailable.control_by_state_status(),
        ViewStatus::DomainUnavailable
    );
    assert_eq!(
        absent.control_by_province_status(),
        ViewStatus::DomainAbsent
    );
    assert_eq!(
        unavailable.control_by_province_status(),
        ViewStatus::DomainUnavailable
    );

    assert_eq!(absent.division_units_status(), ViewStatus::DomainAbsent);
    assert_eq!(
        unavailable.division_units_status(),
        ViewStatus::DomainUnavailable
    );
    assert_eq!(absent.fleets_status(), ViewStatus::DomainAbsent);
    assert_eq!(unavailable.fleets_status(), ViewStatus::DomainUnavailable);
    assert_eq!(absent.wings_status(), ViewStatus::DomainAbsent);
    assert_eq!(unavailable.wings_status(), ViewStatus::DomainUnavailable);

    assert_eq!(absent.countries_status(), ViewStatus::DomainAbsent);
    assert_eq!(
        unavailable.countries_status(),
        ViewStatus::DomainUnavailable
    );
    assert_eq!(absent.country_identities_status(), ViewStatus::DomainAbsent);
    assert_eq!(
        unavailable.country_identities_status(),
        ViewStatus::DomainUnavailable
    );
    assert_eq!(absent.topbar_status(), ViewStatus::DomainAbsent);
    assert_eq!(unavailable.topbar_status(), ViewStatus::DomainUnavailable);

    assert_eq!(absent.session_metadata().status, ViewStatus::DomainAbsent);
    assert_eq!(
        unavailable.session_metadata().status,
        ViewStatus::DomainUnavailable
    );
    assert_eq!(absent.naval_supremacy_status(), ViewStatus::DomainAbsent);
    assert_eq!(
        unavailable.naval_supremacy_status(),
        ViewStatus::DomainUnavailable
    );
    assert_eq!(absent.weather_status(), ViewStatus::DomainAbsent);
    assert_eq!(unavailable.weather_status(), ViewStatus::DomainUnavailable);

    assert_eq!(absent.diplomacy().status, ViewStatus::DomainAbsent);
    assert_eq!(
        unavailable.diplomacy().status,
        ViewStatus::DomainUnavailable
    );
    assert_eq!(absent.buildings_status(), ViewStatus::DomainAbsent);
    assert_eq!(
        unavailable.buildings_status(),
        ViewStatus::DomainUnavailable
    );
    assert_eq!(absent.visibility_status(), ViewStatus::DomainAbsent);
    assert_eq!(
        unavailable.visibility_status(),
        ViewStatus::DomainUnavailable
    );
    assert_eq!(
        absent.visibility_for("GER").status,
        ViewStatus::DomainAbsent
    );
    assert_eq!(
        unavailable.visibility_for("GER").status,
        ViewStatus::DomainUnavailable
    );

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
    assert_eq!(a.owner_by_state_status(), b.owner_by_state_status());
    assert_eq!(a.control_by_state(), b.control_by_state());
    assert_eq!(a.control_by_state_source(), b.control_by_state_source());
    assert_eq!(a.control_by_state_status(), b.control_by_state_status());
    assert_eq!(a.control_by_province(), b.control_by_province());
    assert_eq!(
        a.control_by_province_source(),
        b.control_by_province_source()
    );
    assert_eq!(
        a.control_by_province_status(),
        b.control_by_province_status()
    );
    assert_eq!(a.divisions(), b.divisions());
    assert_eq!(a.division_units(), b.division_units());
    assert_eq!(a.division_units_status(), b.division_units_status());
    assert_eq!(a.fleets(), b.fleets());
    assert_eq!(a.fleets_status(), b.fleets_status());
    assert_eq!(a.wings(), b.wings());
    assert_eq!(a.wings_status(), b.wings_status());
    assert_eq!(a.combats(), b.combats());
    assert_eq!(a.countries(), b.countries());
    assert_eq!(a.countries_status(), b.countries_status());
    assert_eq!(a.session_metadata(), b.session_metadata());
    assert_eq!(a.naval_supremacy_by_region(), b.naval_supremacy_by_region());
    assert_eq!(a.naval_supremacy_status(), b.naval_supremacy_status());
    assert_eq!(a.weather_by_region(), b.weather_by_region());
    assert_eq!(a.weather_status(), b.weather_status());
    assert_eq!(a.country_identities(), b.country_identities());
    assert_eq!(a.country_identities_status(), b.country_identities_status());
    assert_eq!(a.diplomacy(), b.diplomacy());
    assert_eq!(a.topbar_scalars_by_country(), b.topbar_scalars_by_country());
    assert_eq!(a.topbar_status(), b.topbar_status());
    assert_eq!(a.buildings_by_state(), b.buildings_by_state());
    assert_eq!(a.buildings_status(), b.buildings_status());
    assert_eq!(a.visibility_status(), b.visibility_status());
    assert_eq!(a.visibility_for("GER"), b.visibility_for("GER"));
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
    let lookup = timeline.lookup_hour(102).unwrap();
    assert_eq!(lookup.requested_hour, 102);
    assert_eq!(lookup.frame_hour, 101);
    assert!(!lookup.exact_match);
    assert_eq!(lookup.next_hour, Some(103));
    assert_frames_equal(
        &timeline.seek(102).unwrap(),
        &timeline.play_to(102).unwrap(),
    );

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
