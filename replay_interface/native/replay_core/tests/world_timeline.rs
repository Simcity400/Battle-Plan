#![recursion_limit = "512"]


use replay_core::world::FactionRosterStatus;
use replay_core::{
    open_recording, open_recording_game, open_recording_with_decoder,
    CertifiedPrefoldDecoder, CertifiedPrefoldRequest, Leaf, MapLayerSource, PrefoldDecodeError,
    PrefoldedHourEntry, PrefoldedReplay, SourceError, ViewStatus, WorldError, WorldFrame,
    WorldTimeline,
};
use serde_json::{json, Value};
use snapshot::{pack, PackEntry, PackIdentity, Store};
use std::fs;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;
#[cfg(windows)]
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::{mem::MaybeUninit, ptr::null_mut};
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

// fixture: 3 hours (100, 101, 103) exercising ownership change, division

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
                    "naval_base": { "level": 2.0, "healthy_level": 2.0, "damaged": false }
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
                { "id": 1, "owner": "GER", "name": "Baltic Fleet",
                  "task_forces": [
                    { "id": 2, "owner": "GER", "location_province": 321,
                      "dominant_hull_class": "destroyer",
                      "ships": [
                        { "id": "ship-1", "definition": "destroyer", "organisation": 0.9,
                          "strength": 0.8, "experience": 0.2, "name": "Z1",
                          "equipment": [[77, 88, 1.0]] }
                      ] }
                  ] }
            ],
            "air_wings": [
                { "id": 3, "owner": "GER", "name": "JG 1", "count": 120,
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
            "ITA": null
        },
        "states": { "10": { "owner": "SOV" }, "20": { "owner": "GER" }, "30": { "owner": null } },
        "provinces": { "1000": { "controller": "SOV" }, "1001": { "controller": "GER" },
                       "2000": { "controller": "SOV" } },
        "divisions": [
            { "div_id": 6, "owner": "GER", "location_province": 1001,
              "strength": 0.8, "organisation": null }
        ],
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
            { "file": "orphan.snapshot.json" }
        ]
    })
}

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
    // Windows could not even be acquired.
    portable_test_fs_retry::with_tree_read_guard(path, || {
        WorldTimeline::open(open_recording(path).unwrap()).unwrap()
    })
    .expect("hold the store open across the scanner's window")
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

fn write_unit_census_problem_store(root: &Path) {
    let game = "unit_census_problems";
    let dir = root.join(game);
    fs::create_dir_all(&dir).unwrap();
    let units = |extra: Value| {
        let mut base = json!({
            "divisions": [
                {"div_id": 5, "div_type": 29, "owner": "GER", "location_province": 42}
            ],
            "order_of_battle": {
                "fleets": [{
                    "id": 3,
                    "owner": "ENG",
                    "task_forces": [{"id": 71, "location_province": 118}]
                }],
                "air_wings": [
                    {"id": 12, "owner": "SOV", "air_base_province": 907, "state_id": 219}
                ]
            }
        });
        let object = base.as_object_mut().unwrap();
        for (key, value) in extra.as_object().unwrap() {
            object.insert(key.clone(), value.clone());
        }
        base
    };

    // Hour 30: a fleet with zero task forces, which the fold spells by simply
    // not inserting the key.
    let mut absent_task_forces = units(json!({}));
    absent_task_forces["order_of_battle"]["fleets"] = json!([{"id": 3, "owner": "ENG"}]);
    let mut hour_30 = json!({
        "schema_version": "1",
        "game_id": game,
        "date": "1936.1.2.6",
        "hour_index": 30
    });
    for (key, value) in absent_task_forces.as_object().unwrap() {
        hour_30[key] = value.clone();
    }

    // Hour 31: complete unit arrays, but the collector declared a problem on
    // the two unit CENSUS domains (20 divisions_live, 30 air_wings_live) AND on
    // 29 ships_live, which owns ship composition rather than the task-force
    // enumeration and must therefore leave the fleet view alone.
    let mut hour_31 = json!({
        "schema_version": "1",
        "game_id": game,
        "date": "1936.1.2.7",
        "hour_index": 31,
        "provenance": {
            "collector": {
                "domain_problems": [
                    {"domain_id": 20, "problem": "sweep_incomplete"},
                    {"domain_id": 29, "problem": "sweep_incomplete"},
                    {"domain_id": 30, "problem": "sweep_incomplete"}
                ]
            }
        }
    });
    for (key, value) in units(json!({})).as_object().unwrap() {
        hour_31[key] = value.clone();
    }

    // Hour 32: the same complete arrays with NO declared problem, so the
    // downgrades above are attributable to the problems and not to the rows.
    let mut hour_32 = json!({
        "schema_version": "1",
        "game_id": game,
        "date": "1936.1.2.8",
        "hour_index": 32
    });
    for (key, value) in units(json!({})).as_object().unwrap() {
        hour_32[key] = value.clone();
    }

    for (hour, snapshot) in [(30, &hour_30), (31, &hour_31), (32, &hour_32)] {
        fs::write(
            dir.join(format!("{hour:012}.snapshot.json")),
            serde_json::to_vec(snapshot).unwrap(),
        )
        .unwrap();
    }
    let index = json!({
        "game_id": game,
        "schema_version": "1",
        "snapshots": [
            { "hour_index": 30, "date": "1936.1.2.6", "file": "000000000030.snapshot.json" },
            { "hour_index": 31, "date": "1936.1.2.7", "file": "000000000031.snapshot.json" },
            { "hour_index": 32, "date": "1936.1.2.8", "file": "000000000032.snapshot.json" }
        ]
    });
    fs::write(dir.join("index.json"), serde_json::to_vec(&index).unwrap()).unwrap();
}

/// kills, in both directions:
///
/// * dropping (20, DivisionUnits) or (30, Wings) from `DOMAIN_PROBLEM_VIEW_MAP`
///   — a collector problem on a unit census domain must downgrade its typed
///   view rather than leave retained rows presenting as measured. mu_t83
///   declares problems for 20 and 30 at two of its three sampled hours.
/// * ADDING (29, Fleets) — D29 is one row per live ship, not the task-force
///   enumeration D28 owns, so a ships problem must leave the fleet view Present.
/// * refusing an absent `task_forces` key — the fold spells a childless fleet by
///   omitting it, so absence is a complete census of zero, not an unknown one.
#[test]
fn a_declared_problem_on_a_unit_census_domain_downgrades_its_typed_view() {
    let root = unique_temp_dir("unit_census_problems");
    write_unit_census_problem_store(&root);
    let timeline = open_timeline(&root);

    let clean = timeline.frame(32).unwrap();
    assert_eq!(clean.division_units_status(), ViewStatus::Present);
    assert_eq!(clean.fleets_status(), ViewStatus::Present);
    assert_eq!(clean.wings_status(), ViewStatus::Present);
    assert_eq!(clean.division_units().len(), 1);
    assert_eq!(clean.fleets().len(), 1);
    assert_eq!(clean.wings().len(), 1);

    let problems = timeline.frame(31).unwrap();
    assert_eq!(
        problems.division_units_status(),
        ViewStatus::DomainUnavailable,
        "domain 20 declared a problem"
    );
    assert_eq!(
        problems.wings_status(),
        ViewStatus::DomainUnavailable,
        "domain 30 declared a problem"
    );
    // kills: mapping 29 ships_live to the whole Fleets view. D28 owns the
    // fleet/task-force enumeration; a ships problem leaves task-force identity,
    // owner and location authoritative, and poisoning the aggregate would
    // disable consumers whose rows are intact.
    assert_eq!(
        problems.fleets_status(),
        ViewStatus::Present,
        "a ships-domain problem must not invalidate the task-force enumeration"
    );
    assert!(
        !problems.coverage_warnings().iter().any(|warning| {
            warning.contains("no typed view is mapped to it")
                && (warning.contains("domain 20") || warning.contains("domain 30"))
        }),
        "the unit census domains must no longer report as unmapped: {:?}",
        problems.coverage_warnings()
    );

    // kills: treating an absent task_forces key as an unknown census. The fold
    // creates a fleet object for every OOB_KIND_FLEET row and only inserts the
    // key for fleets that have children, so absent means zero task forces.
    // Refusing it made 25 fleets on a healthy mu_t83 hour report unavailable.
    let empty_fleet = timeline.frame(30).unwrap();
    assert_eq!(
        empty_fleet.fleets_status(),
        ViewStatus::Present,
        "a fleet with no task-force children is complete, not unknown"
    );
    assert!(empty_fleet.fleets().is_empty());
    assert!(
        !empty_fleet
            .coverage_warnings()
            .iter()
            .any(|warning| warning.contains("task_forces")),
        "no task-force warning is owed for a childless fleet: {:?}",
        empty_fleet.coverage_warnings()
    );
}

fn write_faction_roster_status_store(root: &Path) {
    let game = "faction_roster_status";
    let dir = root.join(game);
    fs::create_dir_all(&dir).unwrap();
    let states = [
        (20, "unavailable", json!(null)),
        (21, "measured_empty", json!([])),
        (
            22,
            "measured_nonempty",
            json!([{
                "faction_id": 7,
                "faction_type": 88,
                "name_hash": 0x1234_5678u32,
                "leader": "GER",
                "leader_tag_db_id": 1,
                "member_tag_db_ids": [1, 16],
                "members": ["GER", "ITA"],
                "member_count": 2
            }]),
        ),
    ];
    let mut snapshots = Vec::new();
    for (hour, status, rows) in states {
        let mut extra = json!({ "faction_roster_status": status });
        if !rows.is_null() {
            extra["faction_rows"] = rows;
        }
        let mut snapshot = json!({
            "schema_version": "1",
            "game_id": game,
            "date": format!("1936.1.1.{}", hour - 20),
            "hour_index": hour,
            "countries": {},
            "diplomacy_world": { "domain53_extra": extra }
        });
        if hour == 20 {
            snapshot["diplomacy_live"] =
                json!({"faction_members": {"axis": ["GER", "ITA"]}});
        }
        let file = format!("{hour:012}.snapshot.json");
        fs::write(dir.join(&file), serde_json::to_vec(&snapshot).unwrap()).unwrap();
        snapshots.push(json!({
            "hour_index": hour,
            "date": format!("1936.1.1.{}", hour - 20),
            "file": file
        }));
    }
    let index = json!({
        "game_id": game,
        "schema_version": "1",
        "snapshots": snapshots
    });
    fs::write(dir.join("index.json"), serde_json::to_vec(&index).unwrap()).unwrap();
}

fn write_legacy_faction_roster_store(root: &Path) {
    let game = "legacy_faction_roster";
    let dir = root.join(game);
    fs::create_dir_all(&dir).unwrap();
    let cases = [
        (30, None),
        (31, Some(json!({"axis": ["GER", "ITA"]}))),
        (32, Some(json!({"axis": 7}))),
        (33, Some(json!({"axis": ["GER", "ITA"], "allies": 7}))),
        (34, Some(json!({}))),
    ];
    let mut snapshots = Vec::new();
    for (hour, legacy) in cases {
        let mut snapshot = json!({
            "schema_version": "1",
            "game_id": game,
            "date": format!("1936.1.1.{}", hour - 30),
            "hour_index": hour,
            "countries": {}
        });
        if let Some(legacy) = legacy {
            snapshot["diplomacy_live"] = json!({"faction_members": legacy});
        }
        let file = format!("{hour:012}.snapshot.json");
        fs::write(dir.join(&file), serde_json::to_vec(&snapshot).unwrap()).unwrap();
        snapshots.push(json!({
            "hour_index": hour,
            "date": format!("1936.1.1.{}", hour - 30),
            "file": file
        }));
    }
    let index = json!({
        "game_id": game,
        "schema_version": "1",
        "snapshots": snapshots
    });
    fs::write(dir.join("index.json"), serde_json::to_vec(&index).unwrap()).unwrap();
}


#[test]
fn domain53_faction_roster_status_survives_open_into_typed_view() {
    let root = unique_temp_dir("faction_roster_status");
    write_faction_roster_status_store(&root);
    let timeline = open_timeline(&root);

    let unavailable = timeline.frame(20).unwrap();
    assert_eq!(
        unavailable.diplomacy().faction_roster_status(),
        FactionRosterStatus::Unavailable
    );
    assert_eq!(unavailable.diplomacy().faction_members, Leaf::Unobtained);

    let empty = timeline.frame(21).unwrap();
    assert_eq!(
        empty.diplomacy().faction_roster_status(),
        FactionRosterStatus::MeasuredEmpty
    );
    assert_eq!(
        empty.diplomacy().faction_members,
        Leaf::Measured(Default::default())
    );

    let nonempty = timeline.frame(22).unwrap();
    assert_eq!(
        nonempty.diplomacy().faction_roster_status(),
        FactionRosterStatus::MeasuredNonEmpty
    );
    assert_eq!(
        nonempty
            .diplomacy()
            .faction_members
            .measured()
            .unwrap()
            .get("7"),
        Some(&vec!["GER".to_string(), "ITA".to_string()])
    );
}

#[test]
fn modern_unavailable_faction_status_beats_stale_legacy_value() {
    let root = unique_temp_dir("faction_roster_modern_precedence");
    write_faction_roster_status_store(&root);
    let serialized: Value = serde_json::from_slice(
        &portable_test_fs_retry::read(
            root.join("faction_roster_status")
                .join("000000000020.snapshot.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        serialized["diplomacy_live"]["faction_members"]["axis"],
        json!(["GER", "ITA"]),
        "fixture must carry the stale legacy value challenged by this test"
    );
    let timeline = open_timeline(&root);

    let frame = timeline.frame(20).unwrap();
    assert_eq!(
        frame.diplomacy().faction_roster_status(),
        FactionRosterStatus::Unavailable
    );
    assert_eq!(frame.diplomacy().faction_members, Leaf::Unobtained);
}

#[test]
fn legacy_faction_roster_is_measured_only_when_valid_and_nonempty() {
    let root = unique_temp_dir("legacy_faction_roster");
    write_legacy_faction_roster_store(&root);
    let timeline = open_timeline(&root);

    for hour in [30, 32, 33, 34] {
        let frame = timeline.frame(hour).unwrap();
        let diplomacy = frame.diplomacy();
        assert_eq!(
            diplomacy.faction_roster_status(),
            FactionRosterStatus::Unavailable,
            "legacy case at hour {hour} must fail closed"
        );
        assert_ne!(
            diplomacy.faction_roster_status(),
            FactionRosterStatus::MeasuredEmpty
        );
    }

    let valid = timeline.frame(31).unwrap();
    assert_eq!(
        valid.diplomacy().faction_roster_status(),
        FactionRosterStatus::MeasuredNonEmpty
    );
    assert_eq!(
        valid
            .diplomacy()
            .faction_members
            .measured()
            .unwrap()
            .get("axis"),
        Some(&vec!["GER".to_string(), "ITA".to_string()])
    );
}

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
    assert_eq!(d6.organisation, Leaf::Unobtained);
    assert_eq!(d6.in_combat, Leaf::Absent);
    let d7 = div(&f101, "7").unwrap();
    assert_eq!(d7.owner, Leaf::Measured("SOV".to_string()));
    assert!(f101
        .coverage_warnings()
        .iter()
        .any(|w| w.contains("divisions[2].div_type is null")));

    assert!(f100.combats().is_empty());
    assert_eq!(f101.combats().len(), 1);
    let combat = &f101.combats()[0];
    assert_eq!(combat.combat_id, Leaf::Measured(900));
    assert_eq!(combat.kind, Leaf::Measured("land".to_string()));
    assert_eq!(combat.location_province, Leaf::Measured(1001));
    assert_eq!(combat.attackers, Leaf::Measured(vec![5]));
    assert_eq!(combat.defenders, Leaf::Measured(vec![7]));
    assert_eq!(f103.combats().len(), 1);
    let bad_combat = &f103.combats()[0];
    assert_eq!(bad_combat.combat_id, Leaf::Unobtained);
    assert_eq!(bad_combat.attackers, Leaf::Unobtained);
    assert_eq!(bad_combat.defenders, Leaf::Measured(vec![]));
    assert!(f103
        .coverage_warnings()
        .iter()
        .any(|w| w.contains("combats[0].attackers[1]")));

    let ger_100 = f100.country("GER").unwrap();
    let ger_101 = f101.country("GER").unwrap();
    let ger_103 = f103.country("GER").unwrap();
    assert_eq!(ger_100.stability, Leaf::Measured(0.0));
    assert_eq!(ger_101.stability, Leaf::Unobtained);
    assert_eq!(ger_103.stability, Leaf::Absent);
    assert_ne!(ger_100.stability, ger_101.stability);
    assert_ne!(ger_101.stability, ger_103.stability);
    assert_ne!(ger_100.stability, ger_103.stability);

    assert_eq!(ger_103.war_support, Leaf::Unobtained);
    assert!(f103
        .coverage_warnings()
        .iter()
        .any(|w| w.contains("countries.GER.politics.war_support")));
    let ita_103 = f103.country("ITA").unwrap();
    assert_eq!(ita_103.stability, Leaf::Unobtained);
    assert_eq!(ita_103.division_count, Leaf::Unobtained);
    assert!(f103.country("USA").is_none());

    assert!(
        f100.coverage_warnings().is_empty(),
        "unexpected hour-100 coverage warnings: {:?}",
        f100.coverage_warnings()
    );
    assert_eq!(f101.coverage_warnings().len(), 1);

    assert_eq!(ger_100.ruling_party, Leaf::Measured("fascism".to_string()));
    assert_eq!(ger_100.fuel_current, Leaf::Measured(1000.0));
    assert_eq!(ger_100.fuel_max, Leaf::Measured(5000.0));
    assert_eq!(ger_100.division_count, Leaf::Measured(2));
    let sov_100 = f100.country("SOV").unwrap();
    assert_eq!(sov_100.stability, Leaf::Unobtained);
    assert_eq!(sov_100.fuel_current, Leaf::Unobtained);
    assert_eq!(sov_100.division_count, Leaf::Measured(0));

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
    assert_eq!(f100.fleets()[0].task_force_uid, "2");
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
    assert_eq!(f100.wings()[0].wing_uid, "3");
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
        session.speed_events.measured().unwrap()[0].displayed_speed(),
        Leaf::Measured(4)
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
        Leaf::Measured(Some("GER_reich".to_string()))
    );
    assert_eq!(
        ger_identity.ruling_ideology,
        Leaf::Measured("fascism".to_string())
    );
    assert_eq!(
        f101.country_identity("GER").unwrap().cosmetic_tag,
        Leaf::Measured(None)
    );
    assert_eq!(
        f101.topbar_scalars("GER").unwrap().nukes_total,
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
        state_10_buildings.province_building_totals["naval_base"].healthy_level,
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

    let prov = f100.provenance();
    let collector = &prov.channels["collector"];
    assert_eq!(collector.captured, Leaf::Measured(true));
    assert!(collector.domains.iter().any(|d| d == "state_ownership"));
    assert_eq!(prov.channels["memory"].captured, Leaf::Unobtained);
    assert_eq!(prov.channels["save"].domains, vec!["economy".to_string()]);
    assert_eq!(prov.notes, vec!["synthetic fixture".to_string()]);

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

    assert_eq!(absent.diplomacy().status, ViewStatus::DomainUnavailable);
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

#[test]
fn relocated_portable_replay_file_ingests_without_extractor_tree() {
    let source_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("positive")
        .join("fx_topbar_scalars_sweep.battleplan");
    let unrelated_root = unique_temp_dir("portable_relocated").join("standalone-install-data");
    fs::create_dir_all(&unrelated_root).unwrap();
    let relocated = unrelated_root.join("copied-session.battleplan");
    fs::copy(&source_path, &relocated).unwrap();

    let timeline = WorldTimeline::open(open_recording(&relocated).unwrap()).unwrap();
    let frame = timeline.state_at(timeline.first_hour()).unwrap();
    assert_eq!(
        frame.session_metadata().localisation_language,
        Leaf::Measured("l_english".to_string())
    );
    let display = match &frame.session_metadata().display_variant {
        Leaf::Measured(value) => value,
        other => panic!("expected measured XUI25 display variant, got {other:?}"),
    };
    assert_eq!(
        (display.physical_width, display.physical_height),
        (1920, 1080)
    );
    assert_eq!((display.min_gui_width, display.min_gui_height), (1280, 720));
    assert_eq!(display.gui_scale_f32_bits, 1.0f32.to_bits());
    assert_eq!(display.display_mode, "borderless_fullscreen");

    drop(timeline);
    portable_test_fs_retry::remove_dir_all(unrelated_root.parent().unwrap()).unwrap();
}

#[test]
fn incompatible_replay_index_schema_version_fails_clearly() {
    let root = unique_temp_dir("unsupported_schema");
    let game = "unsupported_schema_fixture";
    let snapshot = json!({
        "game_id": game,
        "hour_index": 1,
        "date": "1936.1.1.1",
        "schema_version": "999"
    });
    let entries = vec![
        PackEntry {
            path: format!("payload/{game}/index.json"),
            bytes: serde_json::to_vec(&json!({
                "game_id": game,
                "schema_version": "999",
                "snapshots": [{"hour_index": 1, "file": "000000000001.snapshot.json"}]
            }))
            .unwrap(),
            role: "index",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game}/000000000001.snapshot.json"),
            bytes: serde_json::to_vec(&snapshot).unwrap(),
            role: "snapshot",
            hour_index: Some(1),
        },
    ];
    let path = root.join("unsupported-schema.battleplan");
    fs::write(
        &path,
        pack(game, &entries, &PackIdentity::default(), false).unwrap(),
    )
    .unwrap();

    let error = open_recording(&path)
        .err()
        .expect("must reject unknown schema");
    match error {
        SourceError::Unsupported(message) => {
            assert!(message.contains("incompatible replay index schema_version"));
            assert!(message.contains("999"));
            assert!(message.contains("supports schema_version \"1\""));
        }
        other => panic!("expected Unsupported, got {other:?}"),
    }
    portable_test_fs_retry::remove_dir_all(root).unwrap();
}


#[test]
fn compact_stream_container_fails_closed() {
    let root = unique_temp_dir("compact_closed");
    let game = "native_session_fixture";
    let index = json!({
        "game_id": game,
        "schema_version": "1",
        "snapshots": []
    });
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
            bytes: serde_json::to_vec(&json!({
                "modset": {
                    "resolved": true,
                    "errors": [],
                    "dlc": { "enabled": [] },
                    "dlc_display_names": {},
                    "settings_binding": {
                        "authority": "pre_launch_bracketed",
                        "launch_proven": true,
                        "errors": [],
                        "language": "l_english",
                        "display_variant": {
                            "physical_width": 2752, "physical_height": 1152,
                            "min_gui_width": 1280, "min_gui_height": 720,
                            "gui_scale_f32_bits": 1.0f32.to_bits(),
                            "refresh_rate_hz": 60, "max_refresh_rate_hz": 75,
                            "fullscreen": false, "borderless": true,
                            "display_mode": "borderless_fullscreen",
                        },
                        "attach_settings_sha256": "aa".repeat(32),
                        "seal_settings_sha256": null,
                        "endpoint_semantics_equal": null
                    },
                }
            }))
            .unwrap(),
            role: "recorder_manifest",
            hour_index: None,
        },
    ];
    let identity = PackIdentity {
        schema_version: Some("1".to_string()),
        ..Default::default()
    };
    let path = root.join("compact_only.battleplan");
    fs::write(&path, pack(game, &entries, &identity, true).unwrap()).unwrap();

    let err = portable_test_fs_retry::with_tree_read_guard(&path, || open_recording(&path))
        .expect("hold the container open across the scanner's window")
        .err()
        .expect("must fail closed");
    match &err {
        SourceError::UnsupportedCompactStreamOnly(msg) => {
            assert!(msg.contains("collector_compact_stream"), "msg: {msg}");
            assert!(msg.contains("compact"), "msg: {msg}");
            assert!(msg.contains("hour-index/checkpoint"), "msg: {msg}");
            assert!(msg.contains("portable pre-folder"), "msg: {msg}");
            assert!(msg.contains("ordinary product open"), "msg: {msg}");
        }
        other => panic!("expected UnsupportedCompactStreamOnly, got {other:?}"),
    }

    let _ = fs::remove_dir_all(&root);
}

#[derive(Debug)]
struct FixtureCertifiedPrefolder;

impl CertifiedPrefoldDecoder for FixtureCertifiedPrefolder {
    fn decoder_id(&self) -> &'static str {
        "test.fixture.certified-prefolder"
    }

    fn decode_certified(
        &self,
        request: CertifiedPrefoldRequest,
    ) -> Result<PrefoldedReplay, PrefoldDecodeError> {
        let (game_id, bytes) = request.into_parts();
        if bytes != b"H4CRfixture-stream" {
            return Err(PrefoldDecodeError::new("unexpected fixture stream bytes"));
        }
        let hour = 7;
        let snapshot = serde_json::to_vec(&json!({
            "game_id": game_id,
            "schema_version": "1",
            "hour_index": hour,
            "date": "1936.1.1.7",
            "countries": {"FRA": {"stability": 0.5}}
        }))
        .unwrap();
        let entry = PrefoldedHourEntry::from_json(hour, &snapshot)
            .map_err(|error| PrefoldDecodeError::new(error.to_string()))?;
        PrefoldedReplay::try_new(
            entry.snapshot().game_id.clone().unwrap(),
            Value::String("1".into()),
            vec![entry],
        )
        .map_err(|error| PrefoldDecodeError::new(error.to_string()))
    }
}

#[derive(Debug)]
struct ConflictingProvenancePrefolder;

impl CertifiedPrefoldDecoder for ConflictingProvenancePrefolder {
    fn decoder_id(&self) -> &'static str {
        "test.conflicting-provenance-prefolder"
    }

    fn decode_certified(
        &self,
        request: CertifiedPrefoldRequest,
    ) -> Result<PrefoldedReplay, PrefoldDecodeError> {
        let (game_id, _) = request.into_parts();
        let snapshot = serde_json::to_vec(&json!({
            "game_id": game_id,
            "schema_version": "1",
            "hour_index": 7,
            "provenance": {"mod": {"modset_id": "snapshot-modset"}}
        }))
        .unwrap();
        let entry = PrefoldedHourEntry::from_json(7, &snapshot)
            .map_err(|error| PrefoldDecodeError::new(error.to_string()))?;
        PrefoldedReplay::try_new(
            entry.snapshot().game_id.clone().unwrap(),
            Value::String("1".into()),
            vec![entry],
        )
        .map_err(|error| PrefoldDecodeError::new(error.to_string()))
    }
}

#[test]
fn explicit_portable_prefolder_wires_a_stream_only_container() {
    let root = unique_temp_dir("compact_prefolded");
    let game = "compact-prefolded-game";
    let entries = vec![
        PackEntry {
            path: format!("payload/{game}/index.json"),
            bytes: serde_json::to_vec(&json!({
                "game_id": game,
                "schema_version": "1",
                "snapshots": []
            }))
            .unwrap(),
            role: "index",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game}/collector_raw/collector_stream.bin"),
            bytes: b"H4CRfixture-stream".to_vec(),
            role: "collector_compact_stream",
            hour_index: None,
        },
    ];
    let identity = PackIdentity {
        schema_version: Some("1".to_string()),
        ..Default::default()
    };
    let path = root.join("compact_prefolded.battleplan");
    fs::write(&path, pack(game, &entries, &identity, true).unwrap()).unwrap();

    let source = portable_test_fs_retry::with_tree_read_guard(&path, || {
        assert!(matches!(
            open_recording(&path),
            Err(SourceError::UnsupportedCompactStreamOnly(_))
        ));
        open_recording_with_decoder(&path, &FixtureCertifiedPrefolder).unwrap()
    })
    .expect("hold the container open across the scanner's window");
    assert_eq!(source.game_id(), game);
    assert_eq!(source.hours(), &[7]);
    let receipt = source.prefold_receipt().expect("compact source receipt");
    assert_eq!(receipt.decoder_id(), "test.fixture.certified-prefolder");
    assert_eq!(receipt.source_len(), 18);
    assert_eq!(receipt.source_sha256().len(), 64);
    let frame = source.frame(7).unwrap();
    assert_eq!(frame.date.as_deref(), Some("1936.1.1.7"));
    assert_eq!(frame.rest["countries"]["FRA"]["stability"], json!(0.5));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn compact_prefold_rejects_cross_game_role_path() {
    let root = unique_temp_dir("compact_cross_namespace");
    let game = "compact-prefolded-game";
    let entries = vec![
        PackEntry {
            path: format!("payload/{game}/index.json"),
            bytes: serde_json::to_vec(&json!({
                "game_id": game,
                "schema_version": "1",
                "snapshots": []
            }))
            .unwrap(),
            role: "index",
            hour_index: None,
        },
        PackEntry {
            path: "payload/other/collector_raw/collector_stream.bin".to_string(),
            bytes: b"H4CRfixture-stream".to_vec(),
            role: "collector_compact_stream",
            hour_index: None,
        },
    ];
    let path = root.join("cross_namespace.battleplan");
    fs::write(
        &path,
        pack(game, &entries, &PackIdentity::default(), true).unwrap(),
    )
    .unwrap();

    let error = match open_recording_with_decoder(&path, &FixtureCertifiedPrefolder) {
        Err(error) => error,
        Ok(_) => panic!("cross-game stream role path must be rejected"),
    };
    assert!(matches!(error, SourceError::Open(_)));
    assert!(error.to_string().contains("outside manifest game namespace"));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn compact_prefold_reconciles_first_generated_snapshot_provenance() {
    let root = unique_temp_dir("compact_prefold_provenance");
    let game = "compact-prefolded-game";
    let entries = vec![
        PackEntry {
            path: format!("payload/{game}/index.json"),
            bytes: serde_json::to_vec(&json!({
                "game_id": game,
                "schema_version": "1",
                "snapshots": []
            }))
            .unwrap(),
            role: "index",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game}/collector_raw/collector_stream.bin"),
            bytes: b"H4CRfixture-stream".to_vec(),
            role: "collector_compact_stream",
            hour_index: None,
        },
    ];
    let identity = PackIdentity {
        mod_identity: Some(json!({"modset_id": "manifest-modset"})),
        ..Default::default()
    };
    let path = root.join("provenance_mismatch.battleplan");
    fs::write(&path, pack(game, &entries, &identity, true).unwrap()).unwrap();

    let outcome = portable_test_fs_retry::with_tree_read_guard(&path, || {
        open_recording_with_decoder(&path, &ConflictingProvenancePrefolder)
    })
    .expect("hold the container open across the scanner's window");
    let error = match outcome {
        Err(error) => error,
        Ok(_) => panic!("conflicting prefold provenance must be rejected"),
    };
    assert!(matches!(error, SourceError::ModsetMismatch(_)));
    assert!(error.to_string().contains("manifest-modset"));
    assert!(error.to_string().contains("snapshot-modset"));
    let _ = fs::remove_dir_all(&root);
}

fn resolved_recorder_manifest() -> Value {
    json!({
        "modset": {
            "resolved": true,
            "errors": [],
            "dlc": {
                "installed": ["dlc/a.dlc", "dlc/b.dlc"],
                "enabled": ["dlc/a.dlc"],
                "disabled": ["dlc/b.dlc"]
            },
            "dlc_display_names": {
                "dlc/a.dlc": "DLC A",
                "dlc/b.dlc": "DLC B"
            },
            "settings_binding": {
                "authority": "pre_launch_bracketed",
                "launch_proven": true,
                "errors": [],
                "language": "l_english",
                "display_variant": {
                    "physical_width": 2752, "physical_height": 1152,
                    "min_gui_width": 1280, "min_gui_height": 720,
                    "gui_scale_f32_bits": 1.0f32.to_bits(),
                    "refresh_rate_hz": 60, "max_refresh_rate_hz": 75,
                    "fullscreen": false, "borderless": true,
                    "display_mode": "borderless_fullscreen",
                },
                "attach_settings_sha256": "aa".repeat(32),
                "seal_settings_sha256": null,
                "endpoint_semantics_equal": null
            },
        }
    })
}

fn metadata_snapshot(game: &str, rest: Value) -> Value {
    let mut snapshot = json!({
        "game_id": game,
        "hour_index": 1,
        "date": "1936.1.1.1",
        "schema_version": "1"
    });
    let object = snapshot.as_object_mut().unwrap();
    for (key, value) in rest.as_object().unwrap() {
        object.insert(key.clone(), value.clone());
    }
    snapshot
}

fn recorder_portable_timeline() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "format": "hoi4_compact_stream_timeline_v1",
        "stream_format": "compact",
        "stream": "collector_raw/collector_stream.bin",
        "session_manifest": "recorder/native_session_manifest.json",
        "raw_stream_role": "collector_compact_stream",
        "session_manifest_role": "recorder_manifest"
    }))
    .unwrap()
}

fn portable_content_manifest_fixture() -> replay_core::ContentManifest {
    let mut manifest = replay_core::ContentManifest {
        schema: replay_core::PORTABLE_CONTENT_MANIFEST_SCHEMA.into(),
        product_modset_sha256: "a".repeat(64),
        source_game: replay_core::SourceGameIdentity {
            version: "1.0".into(),
            build: "metadata-fixture".into(),
            executable_sha256: "b".repeat(64),
            build_profile: "metadata-fixture".into(),
        },
        source_profile: replay_core::SourceProfileIdentity {
            profile_id: "metadata-fixture".into(),
            playset_id: "metadata-fixture".into(),
            selection_sha256: "0".repeat(64),
        },
        effective_vfs_manifest_sha256: "c".repeat(64),
        ordered_layers: Vec::new(),
        dlc_selection: Vec::new(),
        localisation_language: "l_english".into(),
        required_settings: std::collections::BTreeMap::new(),
        recorder_local_settings: std::collections::BTreeMap::from([
            ("physical_width".into(), "2752".into()),
            ("physical_height".into(), "1152".into()),
            ("min_gui_width".into(), "1280".into()),
            ("min_gui_height".into(), "720".into()),
            ("gui_scale_f32_bits".into(), 1.0f32.to_bits().to_string()),
            ("refresh_rate_hz".into(), "60".into()),
            ("max_refresh_rate_hz".into(), "75".into()),
            ("fullscreen".into(), "false".into()),
            ("borderless".into(), "true".into()),
            ("display_mode".into(), "borderless_fullscreen".into()),
            ("settings_file_sha256".into(), "aa".repeat(32)),
            ("settings_authority".into(), "pre_launch_bracketed".into()),
        ]),
        required_content: Vec::new(),
        manifest_digest_sha256: None,
    };
    manifest.source_profile.selection_sha256 =
        manifest.computed_selection_sha256().unwrap();
    manifest.with_computed_digest().unwrap()
}

fn absent_static_artifact_roster() -> Vec<Value> {
    [
        (
            "templates_equipment_catalog",
            "static/templates_equipment_catalog/domain59_static_catalog.json",
        ),
        (
            "localisation_join",
            "static/localisation_join/localisation_catalog.json",
        ),
        (
            "decisions_static_definitions",
            "static/decisions_static_definitions/decision_definitions_catalog.json",
        ),
        (
            "idea_tech_loc_xref_static",
            "static/idea_tech_loc_xref_static/idea_tech_loc_xref_catalog.json",
        ),
        (
            "ship_definitions_static",
            "static/ship_definitions_static/ship_definitions_catalog.json",
        ),
        (
            "gfx_icon_asset_manifest_static",
            "static/gfx_icon_asset_manifest_static/gfx_icon_manifest_catalog.json",
        ),
        (
            "technology_sharing_static",
            "static/technology_sharing_static/technology_sharing_static.json",
        ),
        (
            "focus_defs_static",
            "static/focus_defs_static/focus_defs_static.json",
        ),
    ]
    .into_iter()
    .map(|(name, path)| json!({"name": name, "path": path, "present": false}))
    .collect()
}

#[must_use = "the scanner guard must be held until the container is opened"]
fn write_portable_metadata_container(
    root: &Path,
    tag: &str,
    snapshot: Value,
) -> (PathBuf, std::fs::File) {
    let game = format!("metadata_{tag}");
    let content = portable_content_manifest_fixture();
    let mut recorder_manifest = resolved_recorder_manifest();
    recorder_manifest["replay_game_id"] = json!(game);
    recorder_manifest["replay_packaging"] = json!({"static_artifacts": []});
    recorder_manifest["portable_content_manifest"] =
        serde_json::to_value(&content).unwrap();
    recorder_manifest["modset"]["game"] =
        json!({"exe_sha256": content.source_game.executable_sha256.clone()});
    let entries = vec![
        PackEntry {
            path: format!("payload/{game}/index.json"),
            bytes: serde_json::to_vec(&json!({
                "game_id": game,
                "schema_version": "1",
                "host_fold_skipped": false,
                "static_artifacts": absent_static_artifact_roster(),
                "snapshots": [{
                    "hour_index": 1,
                    "date": "1936.1.1.1",
                    "file": "000000000001.snapshot.json"
                }]
            }))
            .unwrap(),
            role: "index",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game}/timeline.json"),
            bytes: recorder_portable_timeline(),
            role: "timeline",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game}/000000000001.snapshot.json"),
            bytes: serde_json::to_vec(&snapshot).unwrap(),
            role: "snapshot",
            hour_index: Some(1),
        },
        PackEntry {
            path: format!("payload/{game}/recorder/native_session_manifest.json"),
            bytes: serde_json::to_vec(&recorder_manifest).unwrap(),
            role: "recorder_manifest",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game}/collector_raw/collector_stream.bin"),
            bytes: b"H4CR portable metadata fixture".to_vec(),
            role: "collector_compact_stream",
            hour_index: None,
        },
    ];
    let path = root.join(format!("{tag}.battleplan"));
    fs::write(
        &path,
        pack(
            &game,
            &entries,
            &PackIdentity {
                schema_version: Some("1".into()),
                build_exe_sha256: Some(content.source_game.executable_sha256.clone()),
                build_pinned: true,
                content_manifest: Some(content),
                ..Default::default()
            },
            false,
        )
        .unwrap(),
    )
    .unwrap();
    let guard = portable_test_fs_retry::open(&path)
        .expect("hold the container open across the scanner's window");
    (path, guard)
}

fn write_metadata_container(
    root: &Path,
    tag: &str,
    timeline_bytes: Vec<u8>,
    snapshot: Value,
    extra_entries: Vec<(String, Vec<u8>, &'static str)>,
) -> PathBuf {
    write_metadata_container_with_timeline_role(
        root,
        tag,
        timeline_bytes,
        snapshot,
        "timeline",
        extra_entries,
    )
}

fn write_metadata_container_with_timeline_role(
    root: &Path,
    tag: &str,
    timeline_bytes: Vec<u8>,
    snapshot: Value,
    timeline_role: &'static str,
    extra_entries: Vec<(String, Vec<u8>, &'static str)>,
) -> PathBuf {
    let game = format!("metadata_{tag}");
    let index = json!({
        "game_id": game,
        "schema_version": "1",
        "snapshots": [{
            "hour_index": 1,
            "date": "1936.1.1.1",
            "file": "000000000001.snapshot.json"
        }]
    });
    let mut entries = vec![
        PackEntry {
            path: format!("payload/{game}/index.json"),
            bytes: serde_json::to_vec(&index).unwrap(),
            role: "index",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game}/timeline.json"),
            bytes: timeline_bytes,
            role: timeline_role,
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game}/000000000001.snapshot.json"),
            bytes: serde_json::to_vec(&snapshot).unwrap(),
            role: "snapshot",
            hour_index: Some(1),
        },
    ];
    entries.extend(
        extra_entries
            .into_iter()
            .map(|(path, bytes, role)| PackEntry {
                path: format!("payload/{game}/{path}"),
                bytes,
                role,
                hour_index: None,
            }),
    );
    let path = root.join(format!("{tag}.battleplan"));
    fs::write(
        &path,
        pack(
            &game,
            &entries,
            &PackIdentity {
                schema_version: Some("1".into()),
                ..Default::default()
            },
            false,
        )
        .unwrap(),
    )
    .unwrap();
    path
}
fn write_metadata_container_without_timeline(
    root: &Path,
    tag: &str,
    extra_entries: Vec<(String, Vec<u8>, &'static str)>,
) -> PathBuf {
    let game = format!("metadata_{tag}");
    let index = json!({
        "game_id": game,
        "schema_version": "1",
        "snapshots": [{"hour_index": 1, "file": "000000000001.snapshot.json"}]
    });
    let snapshot = metadata_snapshot(&game, json!({}));
    let mut entries = vec![
        PackEntry {
            path: format!("payload/{game}/index.json"),
            bytes: serde_json::to_vec(&index).unwrap(),
            role: "index",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game}/000000000001.snapshot.json"),
            bytes: serde_json::to_vec(&snapshot).unwrap(),
            role: "snapshot",
            hour_index: Some(1),
        },
    ];
    entries.extend(
        extra_entries
            .into_iter()
            .map(|(path, bytes, role)| PackEntry {
                path: format!("payload/{game}/{path}"),
                bytes,
                role,
                hour_index: None,
            }),
    );
    let path = root.join(format!("{tag}.battleplan"));
    fs::write(
        &path,
        pack(&game, &entries, &PackIdentity::default(), false).unwrap(),
    )
    .unwrap();
    path
}

fn canonical_json(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(canonical_json).collect()),
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            let mut ordered = serde_json::Map::new();
            for key in keys {
                ordered.insert(key.clone(), canonical_json(&object[key]));
            }
            Value::Object(ordered)
        }
        other => other.clone(),
    }
}

fn stamped_static_artifact_bytes(mut semantic: Value) -> Vec<u8> {
    let mut covered = semantic.clone();
    covered
        .as_object_mut()
        .expect("artifact root is an object")
        .remove("artifact_sha256");
    let embedded = snapshot::sha256::Sha256::hex(
        &serde_json::to_vec(&canonical_json(&covered)).expect("canonical artifact serialises"),
    );
    semantic["artifact_sha256"] = json!(embedded);
    serde_json::to_vec(&semantic).expect("artifact serialises")
}

fn research_catalog_artifact_bytes() -> [(&'static str, &'static str, Vec<u8>); 4] {
    const START_YEARS: [(&str, i64); 2] = [("construction1", 1936), ("infantry_weapons", 1918)];
    const IDS: [&str; 2] = ["construction1", "infantry_weapons"];
    let mut keys = serde_json::Map::new();
    let mut technologies = serde_json::Map::new();
    for id in IDS {
        keys.insert(
            id.to_string(),
            json!({
                "name": format!("{id} display name"),
                "raw": format!("{id} display name"),
                "has_interpolation": false,
                "source_file": "localisation/english/technologies_l_english.yml",
                "source_layer": "vanilla",
            }),
        );
        technologies.insert(
            id.to_string(),
            json!({
                "id": id,
                "kind": "tech",
                "categories": [],
                "folders": [],
                "start_year": START_YEARS
                    .iter()
                    .find(|(name, _)| *name == id)
                    .expect("every fixture technology has an authored year")
                    .1,
                "localisation_status": "resolved",
                "resolved_loc_key": id,
                "localised_name": format!("{id} display name"),
                "resolution_rule": "id",
                "loc_key": id,
                "source_file": "common/technologies/infantry.txt",
                "source_layer": "vanilla",
            }),
        );
    }
    [
        (
            "localisation_join",
            "static/localisation_join/localisation_catalog.json",
            stamped_static_artifact_bytes(json!({
                "schema": "static_localisation_join_r3.static_artifact.v1",
                "artifact_kind": "deterministic_static_replay_enrichment",
                "artifact_sha256": "",
                "request": "R3",
                "keys": Value::Object(keys),
                "meta": {"key_count": IDS.len()},
            })),
        ),
        (
            "idea_tech_loc_xref_static",
            "static/idea_tech_loc_xref_static/idea_tech_loc_xref_catalog.json",
            stamped_static_artifact_bytes(json!({
                "schema": "idea_tech_loc_xref_static.static_artifact.v2",
                "artifact_kind": "deterministic_static_replay_enrichment",
                "artifact_sha256": "",
                "tokens": {
                    "ideas": serde_json::Map::new(),
                    "technologies": Value::Object(technologies),
                },
                "omitted": {"ideas": [], "technologies": []},
                "meta": {
                    "idea_count": 0,
                    "idea_mapped": 0,
                    "idea_omitted_no_loc_key": 0,
                    "tech_count": IDS.len(),
                    "tech_mapped": IDS.len(),
                    "tech_omitted_no_loc_key": 0,
                    "tech_start_year_mapped": IDS.len(),
                    "tech_start_year_absent": 0,
                    "tech_start_year_unresolved_count": 0,
                },
            })),
        ),
        (
            "decisions_static_definitions",
            "static/decisions_static_definitions/decision_definitions_catalog.json",
            stamped_static_artifact_bytes(json!({
                "schema": "decisions_static_definitions.static_artifact.v1",
                "artifact_kind": "deterministic_static_replay_enrichment",
                "artifact_sha256": "",
                "categories": serde_json::Map::new(),
                "decisions": serde_json::Map::new(),
                "meta": {"category_count": 0, "decision_count": 0},
            })),
        ),
        (
            "technology_sharing_static",
            "static/technology_sharing_static/technology_sharing_static.json",
            stamped_static_artifact_bytes(json!({
                "schema": "technology_sharing_static.static_artifact.v1",
                "artifact_kind": "deterministic_static_replay_enrichment",
                "artifact_sha256": "",
                "definitions": [],
                "definitions_by_id": serde_json::Map::new(),
                "join_keys": {"compatibility_status": "no_runtime_join_proven"},
                "meta": {
                    "definition_count": 0,
                    "runtime_token_proof": "engine_runtime_intern_ordinal_not_string_hash",
                    "runtime_hash_proof": "fnv1a_only_indexes_intern_table_not_runtime_token",
                },
            })),
        ),
    ]
}

fn attach_time_content_proof(joins: Vec<Value>, refusals: Vec<Value>) -> Value {
    let layers = vec![json!({
        "ordinal": 0,
        "kind": "base",
        "layer_id": "base",
        "descriptor_sha256": Value::Null,
        "descriptor_file_sha256": Value::Null,
        "source_container_sha256": Value::Null,
    })];
    json!({
        "schema": "hoi4.attach-time-content-proof.v1",
        "kind": "attach_time_effective_vfs_join_v1",
        "game_process_start_unix": 1_786_770_247i64,
        "observed_at_unix": 1_786_770_380i64,
        "completed_at_unix": 1_786_770_403i64,
        "elapsed_ms": 23_042u64,
        "hashed_bytes": 1_799_816_841u64,
        "hashed_files": 35_902u64,
        "window_attempts": 1,
        "window_settled": true,
        "executable_sha256":
            "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
        "executable_size_bytes": 55_680_632u64,
        "dlc_load_sha256":
            "f88eff887671ebcb3a33e1dacd2378f8057f9d2ead35ef9fd94da06d4b8c6197",
        "layer_stack": layers.clone(),
        "layer_stack_sha256":
            portable_content_manifest::attach_content_epoch::layer_stack_digest(&layers).unwrap(),
        "artifact_joins": joins,
        "refused_artifacts": refusals,
        "required_content": [{
            "purpose": "D81 province catalog",
            "logical_path": "map/definition.csv",
            "sha256": "86846be71198d6772c651638aa22e3656133198de9b7c49c6234ed48cf33d87b",
            "bytes": 465_289u64,
            "winning_layer_id": "base",
        }],
        "refused_required_content": [],
    })
}

#[test]
fn the_attach_time_content_epoch_admits_packaged_research_catalogs_end_to_end() {
    let root = unique_temp_dir("attach_time_content_epoch");
    let artifacts = research_catalog_artifact_bytes();

    let build = |tag: &str, mutate: &dyn Fn(&mut Value, &mut Vec<Value>)| -> PathBuf {
        let game = format!("attach_epoch_{tag}");
        let mut joins = artifacts
            .iter()
            .map(|(name, _, bytes)| {
                json!({
                    "role": name,
                    "artifact_sha256": snapshot::sha256::Sha256::hex(bytes),
                    "claimed_paths": 1,
                    "joined_bytes": bytes.len(),
                    "proven_absent_paths": 0,
                    "localisation_source_sha256": Value::Null,
                })
            })
            .collect::<Vec<_>>();
        let mut proof = attach_time_content_proof(Vec::new(), Vec::new());
        mutate(&mut proof, &mut joins);
        proof["artifact_joins"] = json!(joins);
        let epoch =
            portable_content_manifest::attach_content_epoch::epoch_from_proof_json(&proof)
                .map(|epoch| epoch.digest().to_string())
                .unwrap_or_else(|_| "0".repeat(64));

        let mut recorder_manifest = resolved_recorder_manifest();
        recorder_manifest["replay_game_id"] = json!(game);
        recorder_manifest["content_proof_kind"] = json!("attach_time_effective_vfs_join_v1");
        recorder_manifest["attach_time_content_proof"] = proof;
        recorder_manifest["replay_packaging"] = json!({
            "static_artifacts": artifacts
                .iter()
                .map(|(name, path, bytes)| json!({
                    "name": name,
                    "path": path,
                    "sha256": snapshot::sha256::Sha256::hex(bytes),
                    "bytes": bytes.len(),
                    "content_manifest_digest_sha256": epoch,
                }))
                .collect::<Vec<_>>(),
        });

        let packaged_names = artifacts
            .iter()
            .map(|(name, _, _)| *name)
            .collect::<Vec<_>>();
        let roster = absent_static_artifact_roster()
            .into_iter()
            .map(|mut row| {
                let name = row["name"].as_str().unwrap_or_default().to_string();
                if let Some((_, path, bytes)) = artifacts
                    .iter()
                    .find(|(candidate, _, _)| *candidate == name)
                {
                    row = json!({
                        "name": name,
                        "path": path,
                        "present": true,
                        "role": "static_artifact",
                        "sha256": snapshot::sha256::Sha256::hex(bytes),
                        "bytes": bytes.len(),
                    });
                }
                row
            })
            .collect::<Vec<_>>();
        assert_eq!(packaged_names.len(), 4);

        let mut entries = vec![
            PackEntry {
                path: format!("payload/{game}/index.json"),
                bytes: serde_json::to_vec(&json!({
                    "game_id": game,
                    "schema_version": "1",
                    "host_fold_skipped": false,
                    "static_artifacts": roster,
                    "snapshots": [{
                        "hour_index": 1,
                        "date": "1936.1.1.1",
                        "file": "000000000001.snapshot.json"
                    }]
                }))
                .unwrap(),
                role: "index",
                hour_index: None,
            },
            PackEntry {
                path: format!("payload/{game}/timeline.json"),
                bytes: recorder_portable_timeline(),
                role: "timeline",
                hour_index: None,
            },
            PackEntry {
                path: format!("payload/{game}/000000000001.snapshot.json"),
                bytes: serde_json::to_vec(&metadata_snapshot(
                    &game,
                    json!({"world": {"scenario": {}}}),
                ))
                .unwrap(),
                role: "snapshot",
                hour_index: None,
            },
            PackEntry {
                path: format!("payload/{game}/recorder/native_session_manifest.json"),
                bytes: serde_json::to_vec(&recorder_manifest).unwrap(),
                role: "recorder_manifest",
                hour_index: None,
            },
            PackEntry {
                path: format!("payload/{game}/collector_raw/collector_stream.bin"),
                bytes: b"H4CR portable metadata fixture".to_vec(),
                role: "collector_compact_stream",
                hour_index: None,
            },
        ];
        entries[2].hour_index = Some(1);
        for (_, path, bytes) in artifacts.iter() {
            entries.push(PackEntry {
                path: format!("payload/{game}/{path}"),
                bytes: bytes.clone(),
                role: "static_artifact",
                hour_index: None,
            });
        }
        let out = root.join(format!("{tag}.battleplan"));
        fs::write(
            &out,
            pack(
                &game,
                &entries,
                &PackIdentity {
                    schema_version: Some("1".into()),
                    ..Default::default()
                },
                false,
            )
            .unwrap(),
        )
        .unwrap();
        out
    };

    let admitted = build("admitted", &|_, _| {});
    let source = portable_test_fs_retry::with_tree_read_guard(&admitted, || {
        open_recording(&admitted)
    })
    .expect("hold the recording open across the scanner's window")
    .expect(
        "a KIND C recording opens — including its A34 decision catalog, whose own loader used to \
         demand a portable content manifest no shipped recorder can produce",
    );
    assert!(
        source.decision_catalog().is_some(),
        "the attach-time epoch must admit the packaged A34 decision catalog"
    );
    let admission = source
        .product_admission()
        .expect("a recorder-manifest recording carries a product admission");
    assert!(
        admission.static_artifacts_not_content_bound.is_empty(),
        "{:?}",
        admission.static_artifacts_not_content_bound
    );
    let localisation = source
        .static_artifact("localisation_join")
        .expect("the attach-time epoch binds localisation_join");
    let idea_technology = source
        .static_artifact("idea_tech_loc_xref_static")
        .expect("the attach-time epoch binds idea_tech_loc_xref_static");
    let technology_sharing = source
        .static_artifact("technology_sharing_static")
        .expect("the attach-time epoch binds technology_sharing_static");
    let catalogs = replay_core::admit_static_research_catalogs(
        localisation,
        idea_technology,
        technology_sharing,
    )
    .expect("the packaged research catalogs are admitted");
    assert_eq!(catalogs.idea_technology().technology_count(), 2);
    assert_eq!(
        catalogs.technology_sharing().definitions().len(),
        0,
        "the fixture packages an empty sharing roster"
    );
    assert_eq!(
        catalogs.technology_display_name("construction1"),
        Some("construction1 display name")
    );
    assert_eq!(
        catalogs
            .idea_technology()
            .technology("construction1")
            .expect("the admitted catalog carries the technology")
            .start_year,
        Some(1936.0)
    );
    assert_eq!(
        catalogs
            .idea_technology()
            .technology("infantry_weapons")
            .expect("the admitted catalog carries the technology")
            .start_year,
        Some(1918.0)
    );
    drop(source);

    let refused = build("refused", &|proof, joins| {
        joins.retain(|join| join["role"] != json!("technology_sharing_static"));
        proof["refused_artifacts"] = json!([{
            "role": "technology_sharing_static",
            "reason": "a claimed path disagreed on re-read",
        }]);
    });
    let source = open_recording(&refused).expect("a partially refused proof still opens");
    assert!(source.static_artifact("localisation_join").is_some());
    assert!(source.static_artifact("technology_sharing_static").is_none());
    assert_eq!(
        source
            .product_admission()
            .expect("recorder-manifest recording")
            .static_artifacts_not_content_bound,
        vec!["technology_sharing_static".to_string()]
    );
    drop(source);

    let restamped = build("restamped", &|_, joins| {
        for join in joins.iter_mut() {
            if join["role"] == json!("idea_tech_loc_xref_static") {
                join["artifact_sha256"] = json!("11".repeat(32));
            }
        }
    });
    let source = open_recording(&restamped).expect("a mismatched join still opens the recording");
    assert!(source.static_artifact("idea_tech_loc_xref_static").is_none());
    assert!(source.static_artifact("localisation_join").is_some());
    drop(source);

    let tampered = build("tampered", &|proof, _| {
        proof["layer_stack_sha256"] = json!("22".repeat(32));
    });
    let Err(error) = open_recording(&tampered) else {
        panic!("a tampered proof refuses the recording");
    };
    let message = format!("{error}");
    assert!(
        message.contains("claims a content epoch its own attach-time proof does not yield"),
        "{message}"
    );

    let unsettled = build("unsettled", &|proof, _| {
        proof["window_settled"] = json!(false);
    });
    let Err(error) = open_recording(&unsettled) else {
        panic!("an unsettled window binds nothing");
    };
    assert!(format!("{error}").contains("never settled"), "{error}");
}

#[test]
fn a_recording_with_no_content_epoch_opens_and_refuses_its_unbound_static_artifacts() {
    use replay_core::{ProductionProfileAuthority, ProductionProfileGuarantee};

    const REMOVE_EPOCH_FIELD: &str = "<remove the epoch field>";
    const NAME: &str = "ship_definitions_static";
    const RELATIVE: &str = "static/ship_definitions_static/ship_definitions_catalog.json";

    let root = unique_temp_dir("late_attach_static_roster");
    let payload = br#"{"ships":[]}"#.to_vec();
    let payload_sha =
        format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(&payload));

    let build = |tag: &str,
                 row_epoch: Value,
                 production_profile: Option<Value>,
                 extra: Option<Value>,
                 legacy: bool|
     -> PathBuf {
        let game = format!("metadata_{tag}");
        let mut recorder_manifest = resolved_recorder_manifest();
        recorder_manifest["replay_game_id"] = json!(game);
        if let Some(profile) = production_profile {
            recorder_manifest["production_profile"] = profile;
            recorder_manifest["scheduled_domains"] = json!(["fuel_live"]);
        }
        let mut row = json!({
            "name": NAME,
            "path": RELATIVE,
            "sha256": payload_sha,
            "bytes": payload.len(),
            "content_manifest_digest_sha256": row_epoch,
        });
        if row["content_manifest_digest_sha256"] == json!(REMOVE_EPOCH_FIELD) {
            row.as_object_mut()
                .unwrap()
                .remove("content_manifest_digest_sha256");
        }
        let mut rows = vec![row];
        if let Some(extra) = extra.clone() {
            rows.push(extra);
        }
        recorder_manifest["replay_packaging"] = json!({ "static_artifacts": rows });
        let roster = absent_static_artifact_roster()
            .into_iter()
            .filter(|row| !legacy || row["name"] != json!("focus_defs_static"))
            .map(|mut row| {
                if row["name"] == json!(NAME) {
                    row = json!({
                        "name": NAME,
                        "path": RELATIVE,
                        "present": true,
                        "role": "static_artifact",
                        "sha256": payload_sha,
                        "bytes": payload.len(),
                    });
                }
                row
            })
            .collect::<Vec<_>>();
        let entries = vec![
            PackEntry {
                path: format!("payload/{game}/index.json"),
                bytes: serde_json::to_vec(&json!({
                    "game_id": game,
                    "schema_version": "1",
                    "host_fold_skipped": false,
                    "static_artifacts": roster,
                    "snapshots": [{
                        "hour_index": 1,
                        "date": "1936.1.1.1",
                        "file": "000000000001.snapshot.json"
                    }]
                }))
                .unwrap(),
                role: "index",
                hour_index: None,
            },
            PackEntry {
                path: format!("payload/{game}/timeline.json"),
                bytes: recorder_portable_timeline(),
                role: "timeline",
                hour_index: None,
            },
            PackEntry {
                path: format!("payload/{game}/000000000001.snapshot.json"),
                bytes: serde_json::to_vec(&metadata_snapshot(
                    &game,
                    json!({"world": {"scenario": {}}}),
                ))
                .unwrap(),
                role: "snapshot",
                hour_index: Some(1),
            },
            PackEntry {
                path: format!("payload/{game}/{RELATIVE}"),
                bytes: payload.clone(),
                role: "static_artifact",
                hour_index: None,
            },
            PackEntry {
                path: format!("payload/{game}/recorder/native_session_manifest.json"),
                bytes: serde_json::to_vec(&recorder_manifest).unwrap(),
                role: "recorder_manifest",
                hour_index: None,
            },
            PackEntry {
                path: format!("payload/{game}/collector_raw/collector_stream.bin"),
                bytes: b"H4CR portable metadata fixture".to_vec(),
                role: "collector_compact_stream",
                hour_index: None,
            },
        ];
        let path = root.join(format!("{tag}.battleplan"));
        fs::write(
            &path,
            pack(
                &game,
                &entries,
                &PackIdentity {
                    schema_version: Some("1".into()),
                    ..Default::default()
                },
                false,
            )
            .unwrap(),
        )
        .unwrap();
        path
    };

    let write = |tag: &str, row_epoch: Value, production_profile: Option<Value>| -> PathBuf {
        build(tag, row_epoch, production_profile, None, false)
    };
    let write_with_extra_row =
        |tag: &str, row_epoch: Value, production_profile: Option<Value>, extra: Option<Value>| {
            build(tag, row_epoch, production_profile, extra, false)
        };

    let absent_names = || -> Vec<String> {
        absent_static_artifact_roster()
            .into_iter()
            .filter_map(|row| row["name"].as_str().map(str::to_string))
            .filter(|name| name != NAME)
            .collect()
    };
    let disabled_profile = || {
        json!({
            "enabled": false,
            "profile_domains": ["fuel_live"],
            "required_static_artifacts": absent_static_artifact_roster()
                .into_iter()
                .filter_map(|row| row["name"].as_str().map(str::to_string))
                .collect::<Vec<_>>(),
            "mandatory_domains_not_production_complete": [],
            "profile_domains_not_scheduled": [],
            "missing_required_static_artifacts": absent_names(),
        })
    };

    let consistent = write("consistent", Value::Null, Some(disabled_profile()));
    let source = open_recording(&consistent).expect(
        "a recording whose container finalized no content manifest must still open; the \
         artifacts it cannot bind are declined, not the recording",
    );
    assert!(source.static_artifact(NAME).is_none());
    let admission = source
        .product_admission()
        .expect("a recorder-manifest recording carries a product admission");
    assert_eq!(admission.authority, ProductionProfileAuthority::Disabled);
    assert!(admission
        .guarantees_not_held
        .contains(&ProductionProfileGuarantee::ContentBoundStaticArtifacts));
    assert_eq!(
        admission.static_artifacts_not_content_bound,
        vec![NAME.to_string()]
    );
    drop(source);

    let unstated = write("unstated", Value::Null, None);
    let source = open_recording(&unstated).expect("an unstated authority is still admissible");
    assert_eq!(
        source
            .product_admission()
            .expect("recorder-manifest recording")
            .authority,
        ProductionProfileAuthority::Unstated
    );
    drop(source);

    let mut lying = disabled_profile();
    lying["missing_required_static_artifacts"] = json!([]);
    let path = write("declares_nothing_missing", Value::Null, Some(lying));
    let Err(error) = open_recording(&path) else {
        panic!("a declared missing set that disagrees with the package must refuse");
    };
    assert!(
        error.to_string().contains("disagrees with the artifacts the verified package does not carry"),
        "{error}"
    );

    let mut repeated = disabled_profile();
    let mut names = absent_names();
    names.push(names[0].clone());
    repeated["missing_required_static_artifacts"] = json!(names);
    let path = write("declares_a_repeat", Value::Null, Some(repeated));
    let Err(error) = open_recording(&path) else {
        panic!("a repeated declared name must refuse");
    };
    assert!(error.to_string().contains("repeats a name"), "{error}");

    let mut unknown = disabled_profile();
    unknown["missing_required_static_artifacts"] = json!(["not_a_static_artifact"]);
    let path = write("declares_an_unknown", Value::Null, Some(unknown));
    let Err(error) = open_recording(&path) else {
        panic!("an unknown declared name must refuse");
    };
    assert!(
        error
            .to_string()
            .contains("is not a required static artifact"),
        "{error}"
    );

    let path = write_with_extra_row(
        "packages_an_unknown_row",
        Value::Null,
        Some(disabled_profile()),
        Some(json!({
            "name": "unexpected_static",
            "path": "static/unexpected_static/unexpected.json",
            "sha256": "dd".repeat(32),
            "bytes": 1,
            "content_manifest_digest_sha256": Value::Null,
        })),
    );
    let Err(error) = open_recording(&path) else {
        panic!("a recorder row naming a non-required artifact must refuse");
    };
    assert!(
        error
            .to_string()
            .contains("which is not a required static artifact"),
        "{error}"
    );

    let path = write_with_extra_row(
        "packages_a_repeated_row",
        Value::Null,
        Some(disabled_profile()),
        Some(json!({
            "name": NAME,
            "path": RELATIVE,
            "sha256": payload_sha,
            "bytes": payload.len(),
            "content_manifest_digest_sha256": Value::Null,
        })),
    );
    let Err(error) = open_recording(&path) else {
        panic!("a repeated recorder row must refuse");
    };
    assert!(error.to_string().contains("more than once"), "{error}");

    let legacy_declared = absent_names()
        .into_iter()
        .filter(|name| name != "focus_defs_static")
        .collect::<Vec<_>>();
    let legacy_required = absent_static_artifact_roster()
        .into_iter()
        .filter_map(|row| row["name"].as_str().map(str::to_string))
        .filter(|name| name != "focus_defs_static")
        .collect::<Vec<_>>();
    let mut legacy_profile = disabled_profile();
    legacy_profile["missing_required_static_artifacts"] = json!(legacy_declared);
    legacy_profile["required_static_artifacts"] = json!(legacy_required);
    let legacy = build("legacy_seven_row", Value::Null, Some(legacy_profile), None, true);
    let source = open_recording(&legacy)
        .expect("a pre-focus-carrier recording remains readable, as this join promises");
    assert!(source.static_artifact("focus_defs_static").is_none());
    assert!(source
        .product_admission()
        .expect("recorder-manifest recording")
        .guarantees_not_held
        .contains(&ProductionProfileGuarantee::RequiredStaticArtifacts));
    drop(source);

    let contradictory = write("contradictory", json!("cc".repeat(32)), None);
    let Err(error) = open_recording(&contradictory) else {
        panic!("a static-artifact row may not name an epoch the container never finalized");
    };
    assert!(
        error.to_string().contains(NAME),
        "the refusal must name the artifact: {error}"
    );

    for (tag, malformed) in [
        ("epoch_number", json!(7)),
        ("epoch_object", json!({})),
        ("epoch_array", json!([])),
        ("epoch_bool", json!(false)),
        ("epoch_absent", Value::String(REMOVE_EPOCH_FIELD.into())),
    ] {
        let path = write(tag, malformed.clone(), None);
        let Err(error) = open_recording(&path) else {
            panic!("{malformed} is a malformed content epoch, not the null disposition");
        };
        assert!(
            error.to_string().contains("neither a digest string nor null"),
            "{malformed} was not refused as malformed: {error}"
        );
    }

    let mut claiming_profile = disabled_profile();
    claiming_profile["enabled"] = json!(true);
    claiming_profile["missing_required_static_artifacts"] = json!([]);
    let claiming = write("claiming_the_profile", Value::Null, Some(claiming_profile));
    let Err(error) = open_recording(&claiming) else {
        panic!("a capture claiming the production profile must carry a content epoch");
    };
    assert!(
        error
            .to_string()
            .contains("while recorder production_profile.enabled is true"),
        "the refusal must name why the carve-out does not apply: {error}"
    );

    let _ = fs::remove_dir_all(&root);
}

fn write_epoch_bound_package(
    root: &Path,
    tag: &str,
    carried: &[(String, String)],
    omitted_from_the_roster: &[&str],
    production_profile: Option<Value>,
) -> PathBuf {
    let game = format!("metadata_{tag}");
    let content = portable_content_manifest_fixture();
    let epoch = content
        .manifest_digest_sha256
        .clone()
        .expect("the fixture content manifest is digested");
    let carried_names = carried
        .iter()
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();

    let mut entries = Vec::new();
    let mut index_rows = Vec::new();
    let mut recorder_rows = Vec::new();
    for row in absent_static_artifact_roster() {
        let name = row["name"].as_str().expect("roster name").to_string();
        let relative = row["path"].as_str().expect("roster path").to_string();
        if omitted_from_the_roster.contains(&name.as_str()) {
            continue;
        }
        if !carried_names.contains(&name) {
            index_rows.push(row);
            continue;
        }
        let payload = format!("{{\"artifact\":\"{name}\"}}").into_bytes();
        let sha = format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(&payload));
        index_rows.push(json!({
            "name": name,
            "path": relative,
            "present": true,
            "role": "static_artifact",
            "sha256": sha,
            "bytes": payload.len(),
        }));
        recorder_rows.push(json!({
            "name": name,
            "path": relative,
            "sha256": sha,
            "bytes": payload.len(),
            "content_manifest_digest_sha256": epoch,
        }));
        entries.push(PackEntry {
            path: format!("payload/{game}/{relative}"),
            bytes: payload,
            role: "static_artifact",
            hour_index: None,
        });
    }

    let mut recorder_manifest = resolved_recorder_manifest();
    recorder_manifest["replay_game_id"] = json!(game);
    recorder_manifest["portable_content_manifest"] = serde_json::to_value(&content).unwrap();
    recorder_manifest["modset"]["game"] =
        json!({"exe_sha256": content.source_game.executable_sha256.clone()});
    if let Some(profile) = production_profile {
        recorder_manifest["production_profile"] = profile;
        recorder_manifest["scheduled_domains"] = json!(["fuel_live"]);
    }
    recorder_manifest["replay_packaging"] = json!({ "static_artifacts": recorder_rows });

    entries.push(PackEntry {
        path: format!("payload/{game}/index.json"),
        bytes: serde_json::to_vec(&json!({
            "game_id": game,
            "schema_version": "1",
            "host_fold_skipped": false,
            "static_artifacts": index_rows,
            "snapshots": [{
                "hour_index": 1,
                "date": "1936.1.1.1",
                "file": "000000000001.snapshot.json"
            }]
        }))
        .unwrap(),
        role: "index",
        hour_index: None,
    });
    entries.push(PackEntry {
        path: format!("payload/{game}/timeline.json"),
        bytes: recorder_portable_timeline(),
        role: "timeline",
        hour_index: None,
    });
    entries.push(PackEntry {
        path: format!("payload/{game}/000000000001.snapshot.json"),
        bytes: serde_json::to_vec(&metadata_snapshot(&game, json!({"world": {"scenario": {}}})))
            .unwrap(),
        role: "snapshot",
        hour_index: Some(1),
    });
    entries.push(PackEntry {
        path: format!("payload/{game}/recorder/native_session_manifest.json"),
        bytes: serde_json::to_vec(&recorder_manifest).unwrap(),
        role: "recorder_manifest",
        hour_index: None,
    });
    entries.push(PackEntry {
        path: format!("payload/{game}/collector_raw/collector_stream.bin"),
        bytes: b"H4CR portable metadata fixture".to_vec(),
        role: "collector_compact_stream",
        hour_index: None,
    });

    let path = root.join(format!("{tag}.battleplan"));
    fs::write(
        &path,
        pack(
            &game,
            &entries,
            &PackIdentity {
                schema_version: Some("1".into()),
                build_exe_sha256: Some(content.source_game.executable_sha256.clone()),
                build_pinned: true,
                content_manifest: Some(content),
                ..Default::default()
            },
            false,
        )
        .unwrap(),
    )
    .unwrap();
    path
}

fn required_static_artifact_names() -> Vec<(String, String)> {
    absent_static_artifact_roster()
        .into_iter()
        .filter_map(|row| {
            Some((
                row["name"].as_str()?.to_string(),
                row["path"].as_str()?.to_string(),
            ))
        })
        .collect()
}

#[test]
fn an_unstated_recording_carrying_every_artifact_holds_the_static_artifact_guarantee() {
    use replay_core::{ProductionProfileAuthority, ProductionProfileGuarantee};

    let root = unique_temp_dir("unstated_full_roster");
    let carried = required_static_artifact_names();
    assert_eq!(carried.len(), 8);
    let path = write_epoch_bound_package(&root, "unstated_full", &carried, &[], None);

    let source = open_recording(&path).expect("an unstated recording opens");
    let admission = source
        .product_admission()
        .expect("recorder-manifest recording");
    assert_eq!(admission.authority, ProductionProfileAuthority::Unstated);
    assert!(admission.missing_required_static_artifacts.is_empty());
    assert_eq!(
        admission.guarantees_not_held,
        vec![
            ProductionProfileGuarantee::ProductionProfileEnabled,
            ProductionProfileGuarantee::FullProfileDomainSchedule,
        ],
        "the package carries all eight, so that guarantee IS held; the two nothing in the \
         package can answer stay unevidenced"
    );
    for (name, _) in &carried {
        assert!(source.static_artifact(name).is_some());
    }
    drop(source);

    let advertised_names = carried
        .iter()
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    let profile_with = |advertised: Value| {
        json!({
            "enabled": true,
            "profile_domains": ["fuel_live"],
            "required_static_artifacts": advertised,
            "mandatory_domains_not_production_complete": [],
            "profile_domains_not_scheduled": [],
            "missing_required_static_artifacts": [],
        })
    };
    let mut short = advertised_names.clone();
    short.pop();
    let mut repeated = advertised_names.clone();
    repeated.push(advertised_names[0].clone());
    for (tag, advertised, expected) in [
        ("advertises_nothing", json!([]), "is not the canonical required set"),
        ("advertises_short", json!(short), "is not the canonical required set"),
        (
            "advertises_unknown",
            json!(["not_a_static_artifact"]),
            "is not the canonical required set",
        ),
        ("advertises_a_repeat", json!(repeated), "repeats a name"),
        ("advertises_a_number", json!([7]), "is not a string"),
        ("advertises_null", Value::Null, "is absent or is not an array"),
    ] {
        let path = write_epoch_bound_package(&root, tag, &carried, &[], Some(profile_with(advertised.clone())));
        let Err(error) = open_recording(&path) else {
            panic!("{advertised} is not a valid advertised required set");
        };
        assert!(
            error.to_string().contains(expected),
            "{advertised} was refused for the wrong reason: {error}"
        );
    }

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_legacy_production_package_carrying_every_artifact_its_build_required_opens() {
    use replay_core::ProductionProfileGuarantee;

    const SKIPPED: &str = "focus_defs_static";
    let root = unique_temp_dir("legacy_production_roster");

    let carried = required_static_artifact_names()
        .into_iter()
        .filter(|(name, _)| name != SKIPPED)
        .collect::<Vec<_>>();
    assert_eq!(carried.len(), 7);

    let required = carried
        .iter()
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    let path = write_epoch_bound_package(
        &root,
        "legacy_production",
        &carried,
        &[SKIPPED],
        Some(json!({
            "enabled": true,
            "profile_domains": ["fuel_live"],
            "required_static_artifacts": required,
            "mandatory_domains_not_production_complete": [],
            "profile_domains_not_scheduled": [],
            "missing_required_static_artifacts": [],
        })),
    );

    let source = open_recording(&path)
        .expect("a pre-focus production package carrying all seven of its artifacts must open");
    for (name, _) in &carried {
        assert!(
            source.static_artifact(name).is_some(),
            "{name} is epoch-bound and must be admitted"
        );
    }
    assert!(source.static_artifact(SKIPPED).is_none());
    let admission = source
        .product_admission()
        .expect("recorder-manifest recording");
    assert!(admission
        .guarantees_not_held
        .contains(&ProductionProfileGuarantee::RequiredStaticArtifacts));
    assert_eq!(
        admission.missing_required_static_artifacts,
        vec![SKIPPED.to_string()]
    );
    drop(source);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn recorder_metadata_binds_session_metadata_end_to_end() {
    let root = unique_temp_dir("recorder_metadata_e2e");
    let (path, _scanner_guard) = write_portable_metadata_container(
        &root,
        "canonical",
        metadata_snapshot("metadata_canonical", json!({"world": {"scenario": {}}})),
    );
    let timeline = WorldTimeline::open(open_recording(&path).unwrap()).unwrap();
    let frame = timeline.frame(1).unwrap();
    assert_eq!(
        frame.session_metadata().active_dlcs,
        Leaf::Measured(vec!["DLC A".to_string()])
    );
    assert_eq!(
        frame.session_metadata().localisation_language,
        Leaf::Measured("l_english".to_string())
    );
    assert!(matches!(
        &frame.session_metadata().display_variant,
        Leaf::Measured(display)
            if display.physical_width == 2752
                && display.gui_scale_f32_bits == 1.0f32.to_bits()
                && display.display_mode == "borderless_fullscreen"
    ));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn genuinely_unreferenced_snapshot_container_remains_legacy() {
    let root = unique_temp_dir("metadata_legacy");
    let path = write_metadata_container(
        &root,
        "legacy",
        br#"{"format":"snapshot_timeline_v1"}"#.to_vec(),
        metadata_snapshot("metadata_legacy", json!({})),
        vec![],
    );
    let timeline = WorldTimeline::open(open_recording(&path).unwrap()).unwrap();
    let frame = timeline.frame(1).unwrap();
    assert_eq!(frame.session_metadata().active_dlcs, Leaf::Absent);
    assert_eq!(frame.session_metadata().localisation_language, Leaf::Absent);
    assert_eq!(frame.session_metadata().display_variant, Leaf::Absent);
    let _ = fs::remove_dir_all(root);
}
#[test]
fn wholly_absent_canonical_timeline_obeys_the_same_legacy_gate() {
    let root = unique_temp_dir("metadata_no_timeline_matrix");
    let legacy = write_metadata_container_without_timeline(&root, "no_timeline_legacy", vec![]);
    let timeline = WorldTimeline::open(open_recording(&legacy).unwrap()).unwrap();
    let frame = timeline.frame(1).unwrap();
    assert_eq!(frame.session_metadata().active_dlcs, Leaf::Absent);

    let orphan = write_metadata_container_without_timeline(
        &root,
        "no_timeline_orphan_role",
        vec![(
            "recorder/native_session_manifest.json".into(),
            serde_json::to_vec(&resolved_recorder_manifest()).unwrap(),
            "recorder_manifest",
        )],
    );
    let message = open_recording(&orphan).err().unwrap().to_string();
    assert!(message.contains("recorder_manifest"), "{message}");
    assert!(message.contains("canonical timeline"), "{message}");
    assert!(message.contains("absent"), "{message}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn container_timeline_role_reconciliation_matrix_fails_closed() {
    let root = unique_temp_dir("metadata_timeline_roles");

    let zero = write_metadata_container_with_timeline_role(
        &root,
        "zero_timeline_role",
        br#"{"legacy":true}"#.to_vec(),
        metadata_snapshot("metadata_zero_timeline_role", json!({})),
        "metadata",
        vec![],
    );
    let timeline = WorldTimeline::open(open_recording(&zero).unwrap()).unwrap();
    assert_eq!(
        timeline.frame(1).unwrap().session_metadata().active_dlcs,
        Leaf::Absent
    );

    let mismatch = write_metadata_container_with_timeline_role(
        &root,
        "mismatched_timeline_role",
        br#"{"legacy":true}"#.to_vec(),
        metadata_snapshot("metadata_mismatched_timeline_role", json!({})),
        "metadata",
        vec![("other_timeline.json".into(), b"{}".to_vec(), "timeline")],
    );
    let message = open_recording(&mismatch).err().unwrap().to_string();
    assert!(message.contains("timeline role"), "{message}");
    assert!(message.contains("other_timeline.json"), "{message}");

    let duplicate = write_metadata_container(
        &root,
        "duplicate_timeline_roles",
        br#"{"legacy":true}"#.to_vec(),
        metadata_snapshot("metadata_duplicate_timeline_roles", json!({})),
        vec![("other_timeline.json".into(), b"{}".to_vec(), "timeline")],
    );
    let message = open_recording(&duplicate).err().unwrap().to_string();
    assert!(message.contains("timeline role"), "{message}");
    assert!(message.contains("multiple"), "{message}");
    assert!(message.contains("other_timeline.json"), "{message}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn recorder_role_without_timeline_reference_is_inconsistent() {
    let root = unique_temp_dir("metadata_orphan_recorder_role");
    let path = write_metadata_container(
        &root,
        "orphan_recorder_role",
        br#"{"legacy":true}"#.to_vec(),
        metadata_snapshot("metadata_orphan_recorder_role", json!({})),
        vec![(
            "recorder/native_session_manifest.json".into(),
            serde_json::to_vec(&resolved_recorder_manifest()).unwrap(),
            "recorder_manifest",
        )],
    );
    let message = open_recording(&path).err().unwrap().to_string();
    assert!(message.contains("recorder_manifest"), "{message}");
    assert!(message.contains("session_manifest"), "{message}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn referenced_but_missing_recorder_role_is_never_legacy() {
    let root = unique_temp_dir("metadata_missing_role");
    let reference = "recorder/native_session_manifest.json";
    let path = write_metadata_container(
        &root,
        "missing_role",
        format!(r#"{{"session_manifest":"{reference}"}}"#).into_bytes(),
        metadata_snapshot("metadata_missing_role", json!({})),
        vec![],
    );
    let error = open_recording(&path).err().expect("must reject");
    let message = error.to_string();
    assert!(
        message.contains("timeline.json.session_manifest"),
        "{message}"
    );
    assert!(message.contains(reference), "{message}");
    assert!(message.contains("recorder_manifest"), "{message}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn explicit_invalid_session_manifest_references_fail_closed() {
    let root = unique_temp_dir("metadata_invalid_refs");
    let cases = vec![
        ("null", json!(null)),
        ("blank", json!("  ")),
        ("non_string", json!(7)),
        ("absolute", json!("/recorder/manifest.json")),
        (
            "backslash",
            Value::String(format!("recorder{}manifest.json", char::from(92))),
        ),
        ("dot", json!("recorder/./manifest.json")),
        ("dotdot", json!("recorder/../manifest.json")),
        ("empty_segment", json!("recorder//manifest.json")),
    ];
    for (tag, reference) in cases {
        let path = write_metadata_container(
            &root,
            tag,
            serde_json::to_vec(&json!({"session_manifest": reference})).unwrap(),
            metadata_snapshot(&format!("metadata_{tag}"), json!({})),
            vec![],
        );
        let error = open_recording(&path).err().expect("must reject");
        let message = error.to_string();
        assert!(
            message.contains("timeline.json.session_manifest"),
            "{tag}: {message}"
        );
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn duplicate_and_mismatched_recorder_roles_fail_closed() {
    let root = unique_temp_dir("metadata_bad_roles");
    let timeline = br#"{"session_manifest":"recorder/native_session_manifest.json"}"#.to_vec();
    let manifest = serde_json::to_vec(&resolved_recorder_manifest()).unwrap();

    let mismatch = write_metadata_container(
        &root,
        "role_mismatch",
        timeline.clone(),
        metadata_snapshot("metadata_role_mismatch", json!({})),
        vec![(
            "recorder/other.json".into(),
            manifest.clone(),
            "recorder_manifest",
        )],
    );
    let message = open_recording(&mismatch).err().unwrap().to_string();
    assert!(message.contains("session_manifest"), "{message}");
    assert!(message.contains("recorder_manifest path"), "{message}");

    let duplicate = write_metadata_container(
        &root,
        "role_duplicate",
        timeline,
        metadata_snapshot("metadata_role_duplicate", json!({})),
        vec![
            (
                "recorder/native_session_manifest.json".into(),
                manifest.clone(),
                "recorder_manifest",
            ),
            ("recorder/backup.json".into(), manifest, "recorder_manifest"),
        ],
    );
    let message = open_recording(&duplicate).err().unwrap().to_string();
    assert!(
        message.contains("exactly one recorder_manifest"),
        "{message}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn unresolved_or_incomplete_recorder_binding_fails_closed() {
    let root = unique_temp_dir("metadata_bad_binding");
    let timeline = br#"{"session_manifest":"recorder/native_session_manifest.json"}"#.to_vec();
    let unresolved = json!({
        "modset": {
            "resolved": false,
            "errors": ["settings.txt language was unreadable"]
        }
    });
    let unresolved_path = write_metadata_container(
        &root,
        "unresolved",
        timeline.clone(),
        metadata_snapshot("metadata_unresolved", json!({})),
        vec![(
            "recorder/native_session_manifest.json".into(),
            serde_json::to_vec(&unresolved).unwrap(),
            "recorder_manifest",
        )],
    );
    let message = open_recording(&unresolved_path).err().unwrap().to_string();
    assert!(message.contains("unresolved"), "{message}");
    assert!(
        message.contains("settings.txt language was unreadable"),
        "{message}"
    );

    let mut missing_name = resolved_recorder_manifest();
    missing_name["modset"]["dlc_display_names"]
        .as_object_mut()
        .unwrap()
        .remove("dlc/a.dlc");
    let missing_name_path = write_metadata_container(
        &root,
        "missing_name",
        timeline,
        metadata_snapshot("metadata_missing_name", json!({})),
        vec![(
            "recorder/native_session_manifest.json".into(),
            serde_json::to_vec(&missing_name).unwrap(),
            "recorder_manifest",
        )],
    );
    let message = open_recording(&missing_name_path)
        .err()
        .unwrap()
        .to_string();
    assert!(message.contains("metadata binding check"), "{message}");
    assert!(message.contains("dlc/a.dlc"), "{message}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn snapshot_session_or_scenario_conflicts_are_rejected_before_injection() {
    let root = unique_temp_dir("metadata_conflicts");
    let cases = [
        (
            "session_conflict",
            json!({"session": {"active_dlcs": ["Wrong DLC"]}}),
            "session.active_dlcs",
        ),
        (
            "scenario_conflict",
            json!({"world": {"scenario": {"localisation_language": "l_french"}}}),
            "world.scenario.localisation_language",
        ),
        (
            "malformed_world",
            json!({"world": []}),
            "snapshot world is not an object",
        ),
    ];
    for (tag, rest, expected_context) in cases {
        let (path, _scanner_guard) = write_portable_metadata_container(
            &root,
            tag,
            metadata_snapshot(&format!("metadata_{tag}"), rest),
        );
        let timeline = WorldTimeline::open(open_recording(&path).unwrap()).unwrap();
        let message = timeline.frame(1).err().expect("must reject").to_string();
        assert!(message.contains(expected_context), "{tag}: {message}");
    }
    let _ = fs::remove_dir_all(root);
}

fn write_metadata_directory(root: &Path, game: &str, timeline_bytes: &[u8], manifest_bytes: &[u8]) {
    let game_dir = root.join(game);
    fs::create_dir_all(game_dir.join("recorder")).unwrap();
    fs::write(
        game_dir.join("index.json"),
        serde_json::to_vec(&json!({
            "game_id": game,
            "schema_version": "1",
            "snapshots": [{
                "hour_index": 1,
                "file": "000000000001.snapshot.json"
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        game_dir.join("000000000001.snapshot.json"),
        serde_json::to_vec(&metadata_snapshot(game, json!({}))).unwrap(),
    )
    .unwrap();
    fs::write(game_dir.join("timeline.json"), timeline_bytes).unwrap();
    fs::write(
        game_dir
            .join("recorder")
            .join("native_session_manifest.json"),
        manifest_bytes,
    )
    .unwrap();
}

#[test]
fn directory_reference_gate_rejects_missing_and_explicit_invalid_manifest() {
    let root = unique_temp_dir("metadata_directory_reference_gate");
    let missing = root.join("missing");
    let reference = "recorder/native_session_manifest.json";
    write_metadata_directory(
        &missing,
        "missing_manifest",
        br#"{"session_manifest":"recorder/native_session_manifest.json"}"#,
        &serde_json::to_vec(&resolved_recorder_manifest()).unwrap(),
    );
    portable_test_fs_retry::remove_file(
        missing
            .join("missing_manifest")
            .join("recorder")
            .join("native_session_manifest.json"),
    )
    .unwrap();
    let message = open_recording(&missing).err().unwrap().to_string();
    assert!(message.contains(reference), "{message}");
    assert!(message.contains("missing"), "{message}");

    let invalid = root.join("invalid");
    write_metadata_directory(
        &invalid,
        "invalid_reference",
        br#"{"session_manifest":null}"#,
        &serde_json::to_vec(&resolved_recorder_manifest()).unwrap(),
    );
    let message = open_recording(&invalid).err().unwrap().to_string();
    assert!(
        message.contains("timeline.json.session_manifest"),
        "{message}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn duplicate_timeline_members_are_rejected_in_container_and_directory() {
    let root = unique_temp_dir("metadata_duplicate_timeline");
    let duplicate =
        br#"{"session_manifest":"recorder/native_session_manifest.json","session_manifest":"recorder/native_session_manifest.json"}"#;
    let manifest = serde_json::to_vec(&resolved_recorder_manifest()).unwrap();
    let container = write_metadata_container(
        &root,
        "duplicate_timeline_container",
        duplicate.to_vec(),
        metadata_snapshot("metadata_duplicate_timeline_container", json!({})),
        vec![(
            "recorder/native_session_manifest.json".into(),
            manifest.clone(),
            "recorder_manifest",
        )],
    );
    let message = open_recording(&container).err().unwrap().to_string();
    assert!(message.contains("duplicate JSON member"), "{message}");
    assert!(message.contains("session_manifest"), "{message}");

    let directory = root.join("directory");
    write_metadata_directory(
        &directory,
        "duplicate_timeline_directory",
        duplicate,
        &manifest,
    );
    let message = open_recording(&directory).err().unwrap().to_string();
    assert!(message.contains("duplicate JSON member"), "{message}");
    assert!(message.contains("session_manifest"), "{message}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn duplicate_nested_manifest_members_are_rejected_in_container_and_directory() {
    let root = unique_temp_dir("metadata_duplicate_nested");
    let timeline = br#"{"session_manifest":"recorder/native_session_manifest.json"}"#;
    let duplicate_manifest = br#"{
        "modset":{
            "resolved":true,
            "errors":[],
            "dlc":{"enabled":[],"enabled":["dlc/a.dlc"]},
            "dlc_display_names":{"dlc/a.dlc":"DLC A"},
            "localisation_language":"l_english"
        }
    }"#;
    let container = write_metadata_container(
        &root,
        "duplicate_nested_container",
        timeline.to_vec(),
        metadata_snapshot("metadata_duplicate_nested_container", json!({})),
        vec![(
            "recorder/native_session_manifest.json".into(),
            duplicate_manifest.to_vec(),
            "recorder_manifest",
        )],
    );
    let message = open_recording(&container).err().unwrap().to_string();
    assert!(message.contains("duplicate JSON member"), "{message}");
    assert!(message.contains("enabled"), "{message}");

    let directory = root.join("directory");
    write_metadata_directory(
        &directory,
        "duplicate_nested_directory",
        timeline,
        duplicate_manifest,
    );
    let message = open_recording(&directory).err().unwrap().to_string();
    assert!(message.contains("duplicate JSON member"), "{message}");
    assert!(message.contains("enabled"), "{message}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn directory_intermediate_lookalike_is_attributed_and_never_legacy() {
    let root = unique_temp_dir("metadata_intermediate_lookalike");
    let game = "lookalike_game";
    let game_dir = root.join(game);
    fs::create_dir_all(game_dir.join("Recorder")).unwrap();
    fs::write(
        game_dir.join("index.json"),
        serde_json::to_vec(&json!({
            "game_id": game,
            "snapshots": [{"hour_index": 1, "file": "000000000001.snapshot.json"}]
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        game_dir.join("000000000001.snapshot.json"),
        serde_json::to_vec(&metadata_snapshot(game, json!({}))).unwrap(),
    )
    .unwrap();
    fs::write(
        game_dir.join("timeline.json"),
        br#"{"session_manifest":"recorder/native_session_manifest.json"}"#,
    )
    .unwrap();
    fs::write(
        game_dir
            .join("Recorder")
            .join("native_session_manifest.json"),
        serde_json::to_vec(&resolved_recorder_manifest()).unwrap(),
    )
    .unwrap();
    let message = open_recording(&root).err().unwrap().to_string();
    assert!(message.contains("ASCII-lookalike policy"), "{message}");
    assert!(message.contains("recorder"), "{message}");
    assert!(message.contains("Recorder"), "{message}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn same_parent_metadata_hard_link_alias_is_ambiguous() {
    let root = unique_temp_dir("metadata_hardlink_alias");
    let game = "hardlink_game";
    write_metadata_directory(
        &root,
        game,
        br#"{"session_manifest":"recorder/native_session_manifest.json"}"#,
        &serde_json::to_vec(&resolved_recorder_manifest()).unwrap(),
    );
    let recorder = root.join(game).join("recorder");
    fs::hard_link(
        recorder.join("native_session_manifest.json"),
        recorder.join("backup.json"),
    )
    .unwrap();
    let message = open_recording(&root).err().unwrap().to_string();
    assert!(message.contains("same-parent identity alias"), "{message}");
    assert!(
        message.contains("native_session_manifest.json"),
        "{message}"
    );
    assert!(message.contains("backup.json"), "{message}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn canonical_portable_opens_recorder_directory_rejects_and_legacy_directory_opens() {
    let root = unique_temp_dir("metadata_directory_positives");
    let recorder_directory_root = root.join("recorder_directory_rejected");
    write_metadata_directory(
        &recorder_directory_root,
        "recorder_directory",
        &recorder_portable_timeline(),
        &serde_json::to_vec(&resolved_recorder_manifest()).unwrap(),
    );
    let message = open_recording(&recorder_directory_root)
        .err()
        .expect("recorder-owned metadata must not bypass portable container authority")
        .to_string();
    assert!(
        message.contains("requires a portable container authority"),
        "{message}"
    );

    let (canonical_path, _scanner_guard) = write_portable_metadata_container(
        &root,
        "canonical_portable",
        metadata_snapshot("metadata_canonical_portable", json!({})),
    );
    let timeline = WorldTimeline::open(open_recording(&canonical_path).unwrap()).unwrap();
    let frame = timeline.frame(1).unwrap();
    assert_eq!(
        frame.session_metadata().active_dlcs,
        Leaf::Measured(vec!["DLC A".to_string()])
    );

    let legacy_root = root.join("legacy");
    let legacy_game = "legacy_directory";
    let game_dir = legacy_root.join(legacy_game);
    fs::create_dir_all(&game_dir).unwrap();
    fs::write(
        game_dir.join("index.json"),
        serde_json::to_vec(&json!({
            "game_id": legacy_game,
            "snapshots": [{"hour_index": 1, "file": "000000000001.snapshot.json"}]
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        game_dir.join("000000000001.snapshot.json"),
        serde_json::to_vec(&metadata_snapshot(legacy_game, json!({}))).unwrap(),
    )
    .unwrap();
    fs::write(game_dir.join("timeline.json"), br#"{"legacy":true}"#).unwrap();
    let timeline = WorldTimeline::open(open_recording(&legacy_root).unwrap()).unwrap();
    assert_eq!(
        timeline.frame(1).unwrap().session_metadata().active_dlcs,
        Leaf::Absent
    );
    let _ = fs::remove_dir_all(root);
}

#[cfg(windows)]
fn windows_dehydrated_replay_snapshot_candidates(
    root: &Path,
) -> Result<(usize, Vec<(String, String)>), String> {
    const OFFLINE: u32 = 0x0000_1000;
    const REPARSE_POINT: u32 = 0x0000_0400;
    const RECALL_ON_OPEN: u32 = 0x0004_0000;
    const RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;
    const DEHYDRATED: u32 = OFFLINE | RECALL_ON_OPEN | RECALL_ON_DATA_ACCESS;

    let mut pending = vec![root.to_path_buf()];
    let mut scanned_files = 0usize;
    let mut candidates = Vec::new();
    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory).map_err(|error| {
            format!("enumerate metadata under {}: {error}", directory.display())
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                format!("read metadata entry under {}: {error}", directory.display())
            })?;
            let metadata = fs::symlink_metadata(entry.path()).map_err(|error| {
                format!(
                    "query link-safe metadata for {}: {error}",
                    entry.path().display()
                )
            })?;
            let attributes = metadata.file_attributes();
            let file_type = metadata.file_type();
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                if attributes & REPARSE_POINT == 0 {
                    pending.push(entry.path());
                }
                continue;
            }
            if file_type.is_file() {
                scanned_files += 1;
                let path = entry.path();
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if attributes & DEHYDRATED != 0
                    && attributes & REPARSE_POINT != 0
                    && name.ends_with(".snapshot.json")
                {
                    let relative = path.strip_prefix(root).map_err(|error| {
                        format!("derive replay-relative placeholder path: {error}")
                    })?;
                    let components = relative
                        .components()
                        .map(|component| component.as_os_str().to_string_lossy().into_owned())
                        .collect::<Vec<_>>();
                    if let [game, file] = components.as_slice() {
                        candidates.push((game.clone(), file.clone()));
                    }
                }
            }
        }
    }
    candidates.sort();
    Ok((scanned_files, candidates))
}

#[cfg(windows)]
#[derive(Debug, Eq, PartialEq)]
struct WindowsHandleIdentity {
    reparse_tag: u32,
    volume_serial_number: String,
    file_id_128: String,
}

#[cfg(windows)]
fn windows_handle_identity(
    path: &Path,
    flags: u32,
) -> Result<WindowsHandleIdentity, String> {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FileAttributeTagInfo, FileIdInfo, GetFileInformationByHandleEx,
        FILE_ATTRIBUTE_TAG_INFO, FILE_ID_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, OPEN_EXISTING,
    };

    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let raw = unsafe {
        CreateFileW(
            wide.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            flags,
            null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(format!(
            "metadata-only CreateFileW failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut info = MaybeUninit::<FILE_ATTRIBUTE_TAG_INFO>::zeroed();
    let ok = unsafe {
        GetFileInformationByHandleEx(
            handle.as_raw_handle(),
            FileAttributeTagInfo,
            info.as_mut_ptr().cast(),
            std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    };
    if ok == 0 {
        return Err(format!(
            "FileAttributeTagInfo failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    let tag = unsafe { info.assume_init() }.ReparseTag;
    let mut file_id = MaybeUninit::<FILE_ID_INFO>::zeroed();
    let ok = unsafe {
        GetFileInformationByHandleEx(
            handle.as_raw_handle(),
            FileIdInfo,
            file_id.as_mut_ptr().cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if ok == 0 {
        return Err(format!(
            "FileIdInfo failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    let file_id = unsafe { file_id.assume_init() };
    if file_id.FileId.Identifier.iter().all(|byte| *byte == 0)
        || file_id.FileId.Identifier.iter().all(|byte| *byte == u8::MAX)
    {
        return Err("FileIdInfo returned an unavailable 128-bit identifier sentinel".into());
    }
    let file_id_128 = file_id
        .FileId
        .Identifier
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(WindowsHandleIdentity {
        reparse_tag: tag,
        volume_serial_number: format!("{:016x}", file_id.VolumeSerialNumber),
        file_id_128,
    })
}

#[cfg(windows)]
fn windows_cloud_files_identity(path: &Path) -> Result<WindowsHandleIdentity, String> {
    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
    let identity = windows_handle_identity(path, FILE_FLAG_OPEN_REPARSE_POINT)?;
    if identity.reparse_tag & 0xffff_0fff != 0x9000_001a {
        return Err(format!(
            "selected reparse tag {:#010x} is not Cloud Files",
            identity.reparse_tag
        ));
    }
    Ok(identity)
}

#[cfg(windows)]
fn windows_require_expected_identity(
    identity: &WindowsHandleIdentity,
    volume_variable: &str,
    file_id_variable: &str,
    context: &str,
) {
    let expected_volume = std::env::var(volume_variable)
        .unwrap_or_else(|_| panic!("F3_ENV_GATE={volume_variable}_NOT_SET"));
    let expected_file_id = std::env::var(file_id_variable)
        .unwrap_or_else(|_| panic!("F3_ENV_GATE={file_id_variable}_NOT_SET"));
    assert_eq!(
        identity.volume_serial_number, expected_volume,
        "F3_ENV_GATE={context}_VOLUME_IDENTITY_CHANGED"
    );
    assert_eq!(
        identity.file_id_128, expected_file_id,
        "F3_ENV_GATE={context}_FILE_IDENTITY_CHANGED"
    );
}

#[cfg(windows)]
#[test]
#[ignore = "requires a real dehydrated OneDrive recorder-metadata store on the dev rig"]
fn windows_onedrive_dehydrated_recorder_store_binds_end_to_end_before_ship() {
    let provider_state = std::env::var("BATTLE_PLAN_ONEDRIVE_SYNC_PROVIDER_STATE").or_else(|_| std::env::var("HOI4_ONEDRIVE_SYNC_PROVIDER_STATE"))
        .expect("F3_ENV_GATE=ONEDRIVE_PROVIDER_STATE_NOT_SET: set enabled or disabled through the deployment acceptance runner");
    assert!(
        matches!(provider_state.as_str(), "enabled" | "disabled"),
        "F3_ENV_GATE=ONEDRIVE_PROVIDER_STATE_INVALID: expected enabled or disabled"
    );
    let path = std::env::var_os("BATTLE_PLAN_ONEDRIVE_DEHYDRATED_REPLAY_STORE").or_else(|| std::env::var_os("HOI4_ONEDRIVE_DEHYDRATED_REPLAY_STORE"))
        .map(PathBuf::from)
        .expect("F3_ENV_GATE=ONEDRIVE_STORE_ENV_NOT_SET: set BATTLE_PLAN_ONEDRIVE_DEHYDRATED_REPLAY_STORE to a genuinely dehydrated directory store");
    let (scanned_files, candidates) = windows_dehydrated_replay_snapshot_candidates(&path)
        .unwrap_or_else(|error| panic!("F3_ENV_GATE=ONEDRIVE_METADATA_PROBE_FAILED: {error}"));
    assert!(
        !candidates.is_empty(),
        "F3_ENV_GATE=ONEDRIVE_REPLAY_SNAPSHOT_NOT_DEHYDRATED: metadata-only scan found no dehydrated <game>/*.snapshot.json payload among {scanned_files} files; refusing an unrelated-placeholder or hydrated-store pass"
    );
    let selected_path = std::env::var("BATTLE_PLAN_ONEDRIVE_SELECTED_REPLAY_SNAPSHOT").or_else(|_| std::env::var("HOI4_ONEDRIVE_SELECTED_REPLAY_SNAPSHOT"))
        .expect("F3_ENV_GATE=ONEDRIVE_SELECTED_SNAPSHOT_NOT_SET: deployment runner must select one exact game/snapshot.json placeholder");
    assert!(
        !selected_path.contains('\\') && selected_path.split('/').count() == 2,
        "F3_ENV_GATE=ONEDRIVE_SELECTED_SNAPSHOT_INVALID: expected game/file.snapshot.json"
    );
    let (game, file) = candidates
        .iter()
        .find(|(game, file)| selected_path == format!("{game}/{file}"))
        .expect("F3_ENV_GATE=ONEDRIVE_SELECTED_SNAPSHOT_NOT_DEHYDRATED: selected exact snapshot is absent from the fresh placeholder census");
    let expected_tag_text = std::env::var("BATTLE_PLAN_ONEDRIVE_SELECTED_REPARSE_TAG").or_else(|_| std::env::var("HOI4_ONEDRIVE_SELECTED_REPARSE_TAG"))
        .expect("F3_ENV_GATE=ONEDRIVE_SELECTED_REPARSE_TAG_NOT_SET");
    let expected_tag = u32::from_str_radix(
        expected_tag_text
            .strip_prefix("0x")
            .expect("selected reparse tag must use 0xhhhhhhhh form"),
        16,
    )
    .expect("selected reparse tag must be hexadecimal");
    let root_identity = windows_handle_identity(
        &path,
        windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS,
    )
    .unwrap_or_else(|error| panic!("F3_ENV_GATE=ONEDRIVE_STORE_IDENTITY_REJECTED: {error}"));
    windows_require_expected_identity(
        &root_identity,
        "BATTLE_PLAN_ONEDRIVE_STORE_VOLUME_SERIAL_NUMBER",
        "BATTLE_PLAN_ONEDRIVE_STORE_FILE_ID_128",
        "ONEDRIVE_STORE",
    );
    let selected_identity = windows_cloud_files_identity(&path.join(game).join(file))
        .unwrap_or_else(|error| panic!("F3_ENV_GATE=ONEDRIVE_CLOUD_TAG_REJECTED: {error}"));
    assert_eq!(
        selected_identity.reparse_tag, expected_tag,
        "F3_ENV_GATE=ONEDRIVE_SELECTED_REPARSE_TAG_CHANGED"
    );
    windows_require_expected_identity(
        &selected_identity,
        "BATTLE_PLAN_ONEDRIVE_SELECTED_VOLUME_SERIAL_NUMBER",
        "BATTLE_PLAN_ONEDRIVE_SELECTED_FILE_ID_128",
        "ONEDRIVE_SELECTED",
    );
    let store = Store::open_directory(&path).unwrap();
    let index = store.index(game).unwrap();
    let selected = index
        .snapshots
        .iter()
        .find(|entry| entry.file.as_str() == file.as_str())
        .expect("dehydrated replay snapshot must be listed by its game index");
    let selected_hour = selected
        .hour_index
        .expect("selected replay snapshot must carry an index hour");
    drop(store);
    let timeline = WorldTimeline::open(open_recording_game(&path, game).unwrap()).unwrap();
    if provider_state == "disabled" {
        let error = match timeline.frame(selected_hour) {
            Ok(_) => panic!("disabled provider unexpectedly supplied replay-frame bytes"),
            Err(error) => error,
        };
        assert!(
            matches!(error, WorldError::Source(SourceError::Open(_))),
            "disabled provider must surface through the replay source adapter: {error}"
        );
    } else {
        let frame = timeline
            .frame(selected_hour)
            .expect("enabled sync provider failed to hydrate exact replay frame");
        let metadata = frame.session_metadata();
        assert!(
            matches!(&metadata.active_dlcs, Leaf::Measured(_)),
            "recorder DLC binding did not reach session metadata: {:?}",
            metadata.active_dlcs
        );
        assert!(
            matches!(&metadata.localisation_language, Leaf::Measured(language) if !language.trim().is_empty()),
            "recorder language binding did not reach session metadata: {:?}",
            metadata.localisation_language
        );
    }
}

#[test]
fn directory_timeline_filename_lookalike_is_not_legacy() {
    let root = unique_temp_dir("metadata_timeline_lookalike");
    let game = "timeline_lookalike";
    let game_dir = root.join(game);
    fs::create_dir_all(&game_dir).unwrap();
    fs::write(
        game_dir.join("index.json"),
        serde_json::to_vec(&json!({
            "game_id": game,
            "snapshots": [{"hour_index": 1, "file": "000000000001.snapshot.json"}]
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        game_dir.join("000000000001.snapshot.json"),
        serde_json::to_vec(&metadata_snapshot(game, json!({}))).unwrap(),
    )
    .unwrap();
    fs::write(game_dir.join("Timeline.json"), br#"{"legacy":true}"#).unwrap();
    let message = open_recording(&root).err().unwrap().to_string();
    assert!(message.contains("ASCII-lookalike policy"), "{message}");
    assert!(message.contains("Timeline.json"), "{message}");
    let _ = fs::remove_dir_all(root);
}


#[test]
fn scrub_state_at_semantics() {
    let root = unique_temp_dir("scrub");
    write_dir_store(&root);
    let timeline = open_timeline(&root);

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

    match timeline.frame(102) {
        Err(WorldError::NoSuchHour(102)) => {}
        other => panic!(
            "expected NoSuchHour, got {:?}",
            other.map(|f| f.hour_index())
        ),
    }

    let a1 = timeline.state_at(100).unwrap();
    assert_eq!(a1.owner_by_state()[&10], Leaf::Measured("GER".to_string()));
    let b = timeline.state_at(103).unwrap();
    assert_eq!(b.owner_by_state()[&10], Leaf::Measured("SOV".to_string()));
    let a2 = timeline.state_at(100).unwrap();
    assert_eq!(a2.owner_by_state()[&10], Leaf::Measured("GER".to_string()));
    assert_eq!(a1.owner_by_state(), a2.owner_by_state());
    assert!(Arc::ptr_eq(&a1, &a2));

    let _ = fs::remove_dir_all(&root);
}


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


#[test]
fn frame_cache_budget_is_content_neutral() {
    use replay_core::TimelineConfig;

    let root = unique_temp_dir("cache_budget");
    write_dir_store(&root);

    assert_eq!(
        TimelineConfig::default().frame_cache_entries,
        16,
        "default budget is the soak-proven 16-entry cap"
    );
    let reference = open_timeline(&root);
    assert_eq!(reference.frame_cache_entries(), 16);

    for cap in [4usize, 1, 0] {
        let timeline = WorldTimeline::open_with_config(
            open_recording(&root).unwrap(),
            TimelineConfig {
                frame_cache_entries: cap,
            },
        )
        .unwrap();
        assert_eq!(timeline.frame_cache_entries(), cap);
        for _ in 0..2 {
            for &hour in reference.hours() {
                let got = timeline.seek(hour).unwrap();
                let want = reference.seek(hour).unwrap();
                assert_eq!(got.hour_index(), want.hour_index(), "cap {cap} @ {hour}");
                assert_eq!(got.raw(), want.raw(), "cap {cap} raw content @ {hour}");
                assert_eq!(
                    got.owner_by_state(),
                    want.owner_by_state(),
                    "cap {cap} owner view @ {hour}"
                );
                let played = timeline.play_to(hour).unwrap();
                assert_eq!(played.raw(), got.raw(), "cap {cap} seek==play_to @ {hour}");
            }
        }
    }

    let _ = fs::remove_dir_all(&root);
}


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

