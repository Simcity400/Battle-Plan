#![recursion_limit = "512"]

use portable_content_manifest::{
    normalize_game_relative_path, ContentLayerKind, ContentManifest, LayerStorageKind,
    RecordedContentLayer, RequiredContent, RequiredContentKind, SourceGameIdentity,
    SourceProfileIdentity, UsedContentKey, SCHEMA,
};
use serde_json::{json, Map, Value};
use snapshot::{pack, pack_with, PackEntry, PackIdentity, PackOptions};
use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_SEED: u64 = 424_242;
const SNAPSHOT_COUNT: usize = 12;
const BUILD_SHA: &str = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120";
const MODSET: &str = "fixture-modset-a1a2";
const FIXTURE_BASE_DEFINITION: &[u8] =
    b"fixture_country = { graphical_culture = fixture_graphical_culture }\n";
const FIXTURE_MOD_TEXTURE: &[u8] = b"DDS fixture replay icon payload v1\n";

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn main() -> Result<()> {
    let args = Args::parse()?;
    generate(&args.out_dir, args.seed)?;
    Ok(())
}

struct Args {
    out_dir: PathBuf,
    seed: u64,
}

impl Args {
    fn parse() -> Result<Args> {
        let mut out_dir = default_out_dir();
        let mut seed = DEFAULT_SEED;
        let mut it = std::env::args().skip(1);
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--out" => {
                    let value = it.next().ok_or("--out requires a path")?;
                    out_dir = PathBuf::from(value);
                }
                "--seed" => {
                    let value = it.next().ok_or("--seed requires a u64")?;
                    seed = value.parse::<u64>()?;
                }
                "--help" | "-h" => {
                    println!("usage: replay_fixture_gen [--out <dir>] [--seed <u64>]");
                    std::process::exit(0);
                }
                other => return Err(format!("unknown argument {other:?}").into()),
            }
        }
        Ok(Args { out_dir, seed })
    }
}

fn default_out_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn generate(out_dir: &Path, seed: u64) -> Result<()> {
    fs::create_dir_all(out_dir)?;
    let positive_dir = out_dir.join("positive");
    let negative_dir = out_dir.join("negative");
    let anomaly_dir = out_dir.join("anomaly");
    reset_dir(&positive_dir)?;
    reset_dir(&negative_dir)?;
    reset_dir(&anomaly_dir)?;

    let mut positive_manifest = Vec::new();
    for scenario in positive_scenarios(seed) {
        let file = positive_dir.join(format!("{}.battleplan", scenario.name));
        let bytes = pack_scenario(&scenario, seed)?;
        fs::write(&file, &bytes)?;
        positive_manifest.push(scenario_manifest(
            &scenario,
            &file,
            out_dir,
            bytes.len() as u64,
        ));
    }

    let mut anomaly_manifest = Vec::new();
    for scenario in anomaly_scenarios(seed) {
        let file = anomaly_dir.join(format!("{}.battleplan", scenario.name));
        let bytes = pack_scenario(&scenario, seed)?;
        fs::write(&file, &bytes)?;
        anomaly_manifest.push(scenario_manifest(
            &scenario,
            &file,
            out_dir,
            bytes.len() as u64,
        ));
    }

    let mut negative_manifest = Vec::new();
    for neg in negative_fixtures(seed)? {
        let file = negative_dir.join(format!("{}.battleplan", neg.name));
        fs::write(&file, &neg.bytes)?;
        negative_manifest.push(json!({
            "name": neg.name,
            "path": rel_slash(&file, out_dir),
            "description": neg.description,
            "expected_error_contains": neg.expected_error_contains,
            "bytes": neg.bytes.len() as u64,
        }));
    }

    let manifest = json!({
        "format": "hoi4replay_fixture_manifest",
        "version": 1,
        "seed": seed,
        "snapshot_count_per_positive_fixture": SNAPSHOT_COUNT,
        "generator": {
            "crate": "replay_interface/native/replay_fixture_gen",
            "packing_api": "portable_replay/native/snapshot::pack",
            "determinism": "all timestamps, ordering, identity, and scenario data derive from the explicit seed"
        },
        "positive": positive_manifest,
        "anomaly": anomaly_manifest,
        "negative": negative_manifest,
    });
    fs::write(
        out_dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;

    println!(
        "wrote {} positive, {} anomaly, and {} negative fixtures to {}",
        manifest["positive"].as_array().map_or(0, Vec::len),
        manifest["anomaly"].as_array().map_or(0, Vec::len),
        manifest["negative"].as_array().map_or(0, Vec::len),
        out_dir.display()
    );
    Ok(())
}

fn reset_dir(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_dir_all(path)?;
    }
    fs::create_dir_all(path)?;
    Ok(())
}

#[derive(Clone)]
struct Probe {
    hour: i64,
    path: Vec<String>,
    equals: Value,
    note: String,
}

impl Probe {
    fn new(hour: i64, path: &[&str], equals: Value, note: &str) -> Probe {
        Probe {
            hour,
            path: path.iter().map(|p| p.to_string()).collect(),
            equals,
            note: note.to_string(),
        }
    }
}

struct Scenario {
    name: String,
    description: String,
    contracts: Vec<String>,
    snapshots: Vec<Value>,
    probes: Vec<Probe>,
    events: Vec<Value>,
    anomaly: Option<Value>,
}

struct NegativeFixture {
    name: &'static str,
    description: &'static str,
    expected_error_contains: &'static str,
    bytes: Vec<u8>,
}

fn positive_scenarios(seed: u64) -> Vec<Scenario> {
    vec![
        fx_owner_controller_flip(seed),
        fx_cosmetic_ideology(seed),
        fx_war_faction_matrix(seed),
        fx_capital_moves(seed),
        fx_weather_sweep(seed),
        fx_snow_mud_accumulator(seed),
        fx_topbar_scalars_sweep(seed),
        fx_division_movement(seed),
        fx_naval_air_activity(seed),
        fx_building_damage(seed),
        fx_pov_visibility(seed),
        fx_dlc_language_variation(seed),
        fx_research_focus(seed),
        fx_country_management(seed),
        fx_designs_catalog(seed),
        fx_occupation_sweep(seed),
        fx_leader_roster(seed),
        fx_power_balance_combat(seed),
        fx_scripted_gui_vars(seed),
    ]
}

fn scenario<F>(
    seed: u64,
    name: &str,
    description: &str,
    contracts: &[&str],
    domains: &[&str],
    dlcs: &[&str],
    language: &str,
    mut mutate: F,
) -> Scenario
where
    F: FnMut(usize, &mut Value),
{
    let mut rng = StableRng::new(seed, name);
    let mut snapshots = Vec::with_capacity(SNAPSHOT_COUNT);
    let base_offset = (rng.next_u64() % 6) as u32;
    for tick in 0..SNAPSHOT_COUNT {
        let (date, hour) = fixture_time(base_offset + tick as u32);
        let mut snap = base_snapshot(
            name,
            &date,
            hour,
            domains,
            dlcs,
            language,
            seed,
            rng.next_u64(),
        );
        mutate(tick, &mut snap);
        refresh_division_counts(&mut snap);
        refresh_map_layers(&mut snap);
        snapshots.push(snap);
    }
    Scenario {
        name: name.to_string(),
        description: description.to_string(),
        contracts: contracts.iter().map(|c| c.to_string()).collect(),
        snapshots,
        probes: Vec::new(),
        events: Vec::new(),
        anomaly: None,
    }
}

fn with_probe(mut scenario: Scenario, probe: Probe) -> Scenario {
    scenario.probes.push(probe);
    scenario
}

fn with_event(mut scenario: Scenario, event: Value) -> Scenario {
    scenario.events.push(event);
    scenario
}

fn with_anomaly(mut scenario: Scenario, anomaly: Value) -> Scenario {
    scenario.anomaly = Some(anomaly);
    scenario
}

fn drop_snapshot(mut scenario: Scenario, tick: usize) -> Scenario {
    scenario.snapshots.remove(tick);
    scenario
}


fn anomaly_scenarios(seed: u64) -> Vec<Scenario> {
    vec![
        fx_anomaly_missing_domain(seed),
        fx_anomaly_type_flip(seed),
        fx_anomaly_out_of_range(seed),
        fx_anomaly_hour_gap(seed),
        fx_anomaly_identity_mutation(seed),
    ]
}

fn fx_anomaly_missing_domain(seed: u64) -> Scenario {
    let s = scenario(
        seed,
        "fx_anomaly_missing_domain",
        "Weather domain is present for the first six hours then vanishes entirely mid-recording (Absent-after-Present).",
        &["plan:F4:missing_domain"],
        &["clock", "country_identity", "weather"],
        &["No Step Back"],
        "english",
        |tick, snap| {
            if tick < 6 {
                snap["weather"]["regions"] = json!({
                    "201": {"region_id": 201, "phenom": 3, "phenom_name": "snow", "temp_c": -5.0, "active": true, "intensity": 0.8}
                });
                snap["weather"]["source"] = json!("M");
            } else if let Some(obj) = snap.as_object_mut() {
                obj.remove("weather");
            }
        },
    );
    let crossing = hour_at(&s, 6);
    let last_good = hour_at(&s, 5);
    with_anomaly(
        s,
        json!({
            "class": "missing_domain",
            "monitor": "weather",
            "crossing_hour": crossing,
            "last_good_hour": last_good,
            "detail": "weather view is Present hours 0-5 then the whole weather domain is Absent; policy holds the last-good weather marked HELD, never invents rows"
        }),
    )
}

fn fx_anomaly_type_flip(seed: u64) -> Scenario {
    let s = scenario(
        seed,
        "fx_anomaly_type_flip",
        "GER political_power is a number for the first six hours then flips to a string mid-recording.",
        &["plan:F4:type_flip"],
        &["clock", "country_identity", "politics_live"],
        &["La Resistance"],
        "english",
        |tick, snap| {
            if tick < 6 {
                snap["countries"]["GER"]["politics"]["political_power"] = json!(120.0 + tick as f64);
            } else {
                snap["countries"]["GER"]["politics"]["political_power"] = json!("surging");
            }
        },
    );
    let crossing = hour_at(&s, 6);
    let last_good = hour_at(&s, 5);
    with_anomaly(
        s,
        json!({
            "class": "type_flip",
            "monitor": "countries.GER.political_power",
            "crossing_hour": crossing,
            "last_good_hour": last_good,
            "detail": "GER political_power number->string at the crossing hour; CountryCore.political_power becomes Unobtained + a wrong-typed coverage warning; policy holds the last-good number marked HELD"
        }),
    )
}

fn fx_anomaly_out_of_range(seed: u64) -> Scenario {
    let s = scenario(
        seed,
        "fx_anomaly_out_of_range",
        "GER manpower goes negative and a province id absent from the map appears mid-recording.",
        &["plan:F4:out_of_range"],
        &[
            "clock",
            "country_identity",
            "country_scalars",
            "state_ownership",
        ],
        &["Arms Against Tyranny"],
        "english",
        |tick, snap| {
            if tick >= 6 {
                snap["countries"]["GER"]["military"]["manpower"]["available"] = json!(-5000);
                snap["provinces"]["987654"] = province("GER", 10);
            }
        },
    );
    let crossing = hour_at(&s, 6);
    let last_good = hour_at(&s, 5);
    with_anomaly(
        s,
        json!({
            "class": "out_of_range",
            "monitor": "countries.GER.manpower_available",
            "province_monitor": "map.control_by_province",
            "out_of_map_province": 987654,
            "known_province_ids": [1000, 1001, 2000, 3000],
            "crossing_hour": crossing,
            "last_good_hour": last_good,
            "detail": "GER manpower_available goes negative and province 987654 (absent from the map) appears from the crossing hour; policy holds the last-good manpower marked HELD and flags the stray province, never clamps or invents"
        }),
    )
}

fn fx_anomaly_hour_gap(seed: u64) -> Scenario {
    let s = scenario(
        seed,
        "fx_anomaly_hour_gap",
        "A mid-recording hourly snapshot is missing, leaving a hole in the otherwise-contiguous hour series.",
        &["plan:F4:hour_gap"],
        &["clock", "country_identity", "state_ownership"],
        &["By Blood Alone"],
        "english",
        |_tick, _snap| {},
    );
    let missing = hour_at(&s, 6);
    let before = hour_at(&s, 5);
    let after = hour_at(&s, 7);
    let s = drop_snapshot(s, 6);
    with_anomaly(
        s,
        json!({
            "class": "hour_gap",
            "monitor": "timeline",
            "missing_hours": [missing],
            "before_gap_hour": before,
            "after_gap_hour": after,
            "detail": "the snapshot for the missing hour is absent between the before/after hours; the gap scrubs as unavailable, never interpolated"
        }),
    )
}

fn fx_anomaly_identity_mutation(seed: u64) -> Scenario {
    let s = scenario(
        seed,
        "fx_anomaly_identity_mutation",
        "Recording identity (modset id + save version) mutates mid-recording; the first snapshot keeps the manifest identity so the container still opens.",
        &["plan:F4:identity_mutation"],
        &["clock", "country_identity", "world_meta"],
        &["Together for Victory"],
        "english",
        |tick, snap| {
            if tick >= 6 {
                snap["save_version"] = json!("fixture-build-2-HIJACK");
                snap["world"]["scenario"]["save_version"] = json!("fixture-build-2-HIJACK");
                snap["provenance"]["mod"]["modset_id"] = json!("fixture-modset-MUTATED");
            }
        },
    );
    let crossing = hour_at(&s, 6);
    let last_good = hour_at(&s, 5);
    with_anomaly(
        s,
        json!({
            "class": "identity_mutation",
            "monitor": "recording_identity",
            "crossing_hour": crossing,
            "last_good_hour": last_good,
            "baseline_modset": MODSET,
            "mutated_modset": "fixture-modset-MUTATED",
            "detail": "recording modset_id + save_version change at the crossing hour (recording identity must be invariant across one recording); policy holds the baseline identity marked HELD and logs the mutation once"
        }),
    )
}

fn fx_owner_controller_flip(seed: u64) -> Scenario {
    let mut s = scenario(
        seed,
        "fx_owner_controller_flip",
        "State owner, state controller, and province controller flip over an hourly timeline.",
        &[
            "requirements:1",
            "requirements:2",
            "plan:A2:owner_controller_flips",
        ],
        &[
            "clock",
            "country_identity",
            "state_ownership",
            "state_control",
            "province_control",
        ],
        &["Together for Victory", "Man the Guns"],
        "english",
        |tick, snap| {
            if tick >= 5 {
                snap["states"]["10"]["owner"] = json!("SOV");
                snap["states"]["10"]["controller"] = json!("SOV");
                snap["provinces"]["1000"]["controller"] = json!("SOV");
            }
            if tick >= 9 {
                snap["states"]["20"]["controller"] = json!("GER");
                snap["provinces"]["2000"]["controller"] = json!("GER");
            }
        },
    );
    let h5 = hour_at(&s, 5);
    s = with_probe(
        s,
        Probe::new(
            h5,
            &["states", "10", "owner"],
            json!("SOV"),
            "state 10 owner flip",
        ),
    );
    let h9 = hour_at(&s, 9);
    with_event(
        with_probe(
            s,
            Probe::new(
                h9,
                &["provinces", "2000", "controller"],
                json!("GER"),
                "province 2000 controller flip",
            ),
        ),
        json!({
            "type": "state_owner_change",
            "hour_index": h5,
            "date": date_at_hour(h5),
            "subject": "SOV",
            "object": "GER",
            "key": "10",
            "from": "GER",
            "to": "SOV",
            "source": "C",
            "confidence": "exact",
            "detail": {"fixture": "owner_controller_flip"}
        }),
    )
}

fn fx_cosmetic_ideology(seed: u64) -> Scenario {
    let mut s = scenario(
        seed,
        "fx_cosmetic_ideology",
        "Country cosmetic tag and ruling-party ideology change mid-recording.",
        &[
            "requirements:7",
            "requirements:7b",
            "plan:A2:cosmetic_ideology",
        ],
        &["clock", "country_identity", "country_misc", "politics_live"],
        &["Together for Victory", "La Resistance"],
        "english",
        |tick, snap| {
            if tick < 4 {
                snap["countries"]["GER"]["misc"]["cosmetic_tag"] = Value::Null;
                snap["countries"]["GER"]["extra"]["cosmetic_tag"] = Value::Null;
                snap["countries"]["GER"]["politics"]["ruling_party"] = json!("fascism");
                snap["countries"]["GER"]["politics"]["party_popularities"] = json!({"fascism": 0.70, "democratic": 0.10, "communism": 0.15, "neutrality": 0.05});
            } else if tick < 8 {
                snap["countries"]["GER"]["misc"]["cosmetic_tag"] =
                    json!("GER_civil_war_democratic");
                snap["countries"]["GER"]["extra"]["cosmetic_tag"] =
                    json!("GER_civil_war_democratic");
                snap["countries"]["GER"]["politics"]["ruling_party"] = json!("democratic");
                snap["countries"]["GER"]["politics"]["party_popularities"] = json!({"fascism": 0.20, "democratic": 0.60, "communism": 0.15, "neutrality": 0.05});
            } else {
                snap["countries"]["GER"]["misc"]["cosmetic_tag"] = json!("GER_reichskommissariat");
                snap["countries"]["GER"]["extra"]["cosmetic_tag"] = json!("GER_reichskommissariat");
                snap["countries"]["GER"]["politics"]["ruling_party"] = json!("neutrality");
            }
        },
    );
    let h4 = hour_at(&s, 4);
    s = with_probe(
        s,
        Probe::new(
            h4,
            &["countries", "GER", "misc", "cosmetic_tag"],
            json!("GER_civil_war_democratic"),
            "cosmetic tag update",
        ),
    );
    with_probe(
        s,
        Probe::new(
            h4,
            &["countries", "GER", "politics", "ruling_party"],
            json!("democratic"),
            "ideology update",
        ),
    )
}

fn fx_war_faction_matrix(seed: u64) -> Scenario {
    let mut s = scenario(
        seed,
        "fx_war_faction_matrix",
        "War relation, faction membership, and live diplomacy matrix evolve.",
        &["requirements:8b", "plan:A2:war_faction_matrix"],
        &[
            "clock",
            "country_identity",
            "diplomacy",
            "diplomacy_live",
            "diplo_relations",
        ],
        &["Man the Guns"],
        "english",
        |tick, snap| {
            if tick >= 3 {
                snap["countries"]["GER"]["politics"]["faction"] =
                    json!({"name": "Axis", "is_leader": true});
                snap["countries"]["ITA"]["politics"]["faction"] =
                    json!({"name": "Axis", "is_leader": false});
                snap["countries"]["ENG"]["politics"]["faction"] =
                    json!({"name": "Allies", "is_leader": true});
                snap["diplomacy_world"]["factions"] = json!({
                    "axis": {"id": "axis", "template": "Axis", "leader": "GER", "members": ["GER", "ITA"]},
                    "allies": {"id": "allies", "template": "Allies", "leader": "ENG", "members": ["ENG"]}
                });
                snap["diplomacy_live"]["faction_members"] =
                    json!({"101": ["GER", "ITA"], "202": ["ENG"]});
            }
            if tick >= 7 {
                snap["countries"]["GER"]["diplomacy"]["at_war"] = json!(["ENG"]);
                snap["countries"]["ENG"]["diplomacy"]["at_war"] = json!(["GER", "ITA"]);
                snap["countries"]["ITA"]["diplomacy"]["at_war"] = json!(["ENG"]);
                snap["countries"]["GER"]["diplomacy"]["wars"] = json!([{"against": "ENG"}]);
                snap["countries"]["ENG"]["diplomacy"]["wars"] =
                    json!([{"against": "GER"}, {"against": "ITA"}]);
                snap["diplomacy_world"]["wars"] = json!({
                    "axis_allies_1936": {
                        "attackers": ["GER", "ITA"],
                        "defenders": ["ENG"],
                        "start_hour": snap["hour_index"].clone(),
                        "front": "western"
                    }
                });
                snap["diplomacy_live"]["war_relations"] = json!([
                    {"first": "GER", "second": "ENG", "first_casualties": 1200, "second_casualties": 900},
                    {"first": "ITA", "second": "ENG", "first_casualties": 200, "second_casualties": 300}
                ]);
                snap["countries"]["GER"]["diplomacy"]["relation_flags"] = json!({"ITA": ["same_faction", "military_access"], "ENG": ["at_war", "embargo"]});
            }
        },
    );
    let h7 = hour_at(&s, 7);
    s = with_probe(
        s,
        Probe::new(
            h7,
            &["countries", "GER", "diplomacy", "at_war"],
            json!(["ENG"]),
            "GER war list",
        ),
    );
    with_probe(
        s,
        Probe::new(
            h7,
            &["diplomacy_live", "war_relations", "0", "second"],
            json!("ENG"),
            "live diplomacy war relation",
        ),
    )
}

fn fx_capital_moves(seed: u64) -> Scenario {
    let s = scenario(
        seed,
        "fx_capital_moves",
        "Country capital state moves while ownership stays stable.",
        &["requirements:8b", "plan:A2:capital_moves"],
        &[
            "clock",
            "country_identity",
            "country_scalars",
            "state_ownership",
        ],
        &["La Resistance"],
        "english",
        |tick, snap| {
            snap["countries"]["FRA"]["politics"]["capital_state"] = json!(20);
            if tick >= 6 {
                snap["countries"]["FRA"]["politics"]["capital_state"] = json!(30);
                snap["countries"]["FRA"]["politics"]["ideas"] = json!(["government_relocated"]);
                snap["states"]["30"]["owner"] = json!("FRA");
                snap["states"]["30"]["controller"] = json!("FRA");
            }
            snap["map"]["vp_by_state"] = json!({"20": 20, "30": 15});
        },
    );
    let h6 = hour_at(&s, 6);
    with_probe(
        s,
        Probe::new(
            h6,
            &["countries", "FRA", "politics", "capital_state"],
            json!(30),
            "capital relocation",
        ),
    )
}

fn fx_weather_sweep(seed: u64) -> Scenario {
    let kinds = [
        (0, "clear", 0.0, 18.0),
        (1, "rain", 0.35, 14.0),
        (2, "storm", 0.9, 9.0),
        (3, "snow", 0.5, -2.0),
        (4, "blizzard", 1.0, -12.0),
        (5, "sandstorm", 0.7, 31.0),
    ];
    let s = scenario(
        seed,
        "fx_weather_sweep",
        "Strategic-region weather phenomenon and per-province displayed weather sweep.",
        &["requirements:6", "plan:A2:weather_sweep"],
        &["clock", "country_identity", "weather"],
        &["By Blood Alone"],
        "english",
        |tick, snap| {
            let (phenom, name, intensity, temp) = kinds[tick % kinds.len()];
            snap["weather"]["regions"] = json!({
                "101": {"region_id": 101, "phenom": phenom, "phenom_name": name, "temp_c": temp, "active": true, "intensity": intensity}
            });
            snap["weather"]["provinces"] = json!({
                "1000": {"province_id": 1000, "temp_c": temp, "temp_change_c": -0.25, "precip": intensity, "snow": if temp < 0.0 { intensity } else { 0.0 }},
                "2000": {"province_id": 2000, "temp_c": temp - 3.0, "temp_change_c": -0.5, "precip": intensity / 2.0, "snow": if temp < 3.0 { intensity / 2.0 } else { 0.0 }}
            });
            snap["weather"]["global_rollup"] =
                json!({"dominant": name, "sampled_regions": 1, "max_intensity": intensity});
            snap["weather"]["source"] = json!("M");
            snap["map"]["strategic_regions"] =
                json!({"101": {"name": "fixture_west", "provinces": [1000, 2000]}});
        },
    );
    let h4 = hour_at(&s, 4);
    with_probe(
        s,
        Probe::new(
            h4,
            &["weather", "regions", "101", "phenom_name"],
            json!("blizzard"),
            "blizzard region row",
        ),
    )
}

fn fx_snow_mud_accumulator(seed: u64) -> Scenario {
    let s = scenario(
        seed,
        "fx_snow_mud_accumulator",
        "Per-province engine snow accumulators and mud booleans evolve independently from region weather kind.",
        &["requirements:6", "plan:A2:snow_mud_accumulator"],
        &["clock", "country_identity", "weather"],
        &["No Step Back"],
        "english",
        |tick, snap| {
            let snow_raw = (tick as i64 * 700_000).min(6_000_000);
            let muddy = (5..9).contains(&tick);
            snap["weather"]["regions"] = json!({
                "201": {"region_id": 201, "phenom": if tick < 6 { 3 } else { 0 }, "phenom_name": if tick < 6 { "snow" } else { "clear" }, "temp_c": if tick < 6 { -5.0 } else { 4.0 }, "active": true}
            });
            let mut regions = Map::new();
            for region_id in 0..305u32 {
                regions.insert(
                    region_id.to_string(),
                    json!({
                        "region_id": region_id,
                        "temperature": 0.0,
                        "temperature_raw": 0,
                        "phenomenon": 0,
                        "weather_kind": "no_phenomenon",
                        "next_weather_change_raw": 0,
                        "active_modifier_count": 0,
                        "intensity": 0,
                        "rain_light": false,
                        "rain_heavy": false,
                        "snow": false,
                        "blizzard": false,
                        "arctic_water": false,
                        "active_signal": false,
                        "phenomenon_active": false,
                        "sandstorm": false
                    }),
                );
            }
            snap["map"]["_r2_weather_live"] = json!({
                "source": "weather_live",
                "scale_raw": 100000,
                "observed_region_rows": 305,
                "active_region_count": 0,
                "regions": Value::Object(regions),
                "observed_province_rows": 3,
                "provinces": {
                    "0": {
                        "snow_raw": snow_raw,
                        "mud_present": true,
                        "mud_active": muddy
                    },
                    "1": {
                        "snow_raw": (snow_raw + 1_800_000).min(9_000_000),
                        "mud_present": true,
                        "mud_active": false
                    },
                    "2": {
                        "snow_raw": 0,
                        "mud_present": true,
                        "mud_active": !muddy
                    }
                }
            });
            snap["weather"]["source"] = json!("M");
        },
    );
    let h8 = hour_at(&s, 8);
    with_probe(
        s,
        Probe::new(
            h8,
            &["map", "_r2_weather_live", "provinces", "0", "snow_raw"],
            json!(5_600_000),
            "province snow accumulator at hour 8",
        ),
    )
}

fn fx_topbar_scalars_sweep(seed: u64) -> Scenario {
    let mut s = scenario(
        seed,
        "fx_topbar_scalars_sweep",
        "Per-POV topbar scalar values sweep across measured zero, ratios, stockpiles, fuel, and alerts.",
        &["requirements:4", "requirements:9b", "plan:A2:topbar_scalars_sweep"],
        &[
            "clock",
            "country_identity",
            "country_scalars",
            "economy",
            "fuel_live",
            "country_misc",
            "decisions_live",
            "supply_inputs",
        ],
        &["Arms Against Tyranny"],
        "english",
        |tick, snap| {
            let t = tick as i64;
            for tag in ["GER", "ENG", "SOV"] {
                let base = match tag {
                    "GER" => 0,
                    "ENG" => 20,
                    _ => 40,
                };
                snap["countries"][tag]["politics"]["political_power"] = json!((base + t * 3) as f64);
                snap["countries"][tag]["politics"]["stability"] = json!(0.40 + (t as f64 * 0.02));
                snap["countries"][tag]["politics"]["war_support"] = json!(0.30 + (t as f64 * 0.015));
                snap["countries"][tag]["politics"]["command_power"] = json!((base + 10 + t) as f64);
                snap["countries"][tag]["military"]["manpower"]["available"] = json!(100_000 + base * 1000 + t * 100);
                snap["countries"][tag]["military"]["xp"] =
                    json!({"army": t as f64, "navy": (t * 2) as f64, "air": (t * 3) as f64});
                snap["countries"][tag]["military"]["convoys_count"] = json!(base + 50 + t);
                snap["countries"][tag]["economy"]["factories"] =
                    json!({"civ": base + 10 + t, "mil": base + 5 + t, "dockyard": base + 2, "under_construction": t % 3});
                snap["countries"][tag]["economy"]["fuel"] =
                    json!({"current": (500 + base * 10 + t * 25) as f64, "max": (2000 + base * 20) as f64, "gain": 12.5, "cost": -3.0});
                snap["countries"][tag]["extra"] =
                    json!({"nukes_total": t / 4, "nukes_ready_total": t / 6, "cosmetic_tag": null});
                snap["countries"][tag]["scalars"] = json!({
                    "industrial_ratio": 0.5 + tick as f64 * 0.01,
                    "convoy_ratio": 0.9 - tick as f64 * 0.01,
                    "fuel_ratio": 0.25 + tick as f64 * 0.02,
                    "xp_ratio": 0.1 + tick as f64 * 0.03,
                    "supply_value": 12.0 + tick as f64,
                    "supply_ratio": 0.8 - tick as f64 * 0.01,
                    "alerts": {"initiative": tick % 3, "decisions_available": tick % 4, "intel": tick % 2, "market_contracts": tick % 5}
                });
                snap["countries"][tag]["decisions"]["status_present"] =
                    json!({"political_action": tick as i64, "market_action": (tick % 3) as i64});
            }
            snap["world"]["world_tension_inputs"] = json!({
                "average_major_ic": 42.0,
                "tension_scaling_base_country": "GER",
                "fixture_topbar_world_tension": 0.05 + tick as f64 * 0.01
            });
        },
    );
    attach_proven_xui25_display_variant(&mut s);
    let h11 = hour_at(&s, 11);
    s = with_probe(
        s,
        Probe::new(
            h11,
            &["countries", "GER", "military", "convoys_count"],
            json!(61),
            "convoy count sweep endpoint",
        ),
    );
    with_probe(
        s,
        Probe::new(
            h11,
            &["countries", "GER", "scalars", "alerts", "market_contracts"],
            json!(1),
            "alert count sweep",
        ),
    )
}

fn attach_proven_xui25_display_variant(scenario: &mut Scenario) {
    for snapshot in &mut scenario.snapshots {
        snapshot["session"] = json!({
            "localisation_language": "l_english",
            "display_variant": {
                "physical_width": 1920,
                "physical_height": 1080,
                "min_gui_width": 1280,
                "min_gui_height": 720,
                "gui_scale_f32_bits": 1.0f32.to_bits(),
                "refresh_rate_hz": 60,
                "max_refresh_rate_hz": 60,
                "fullscreen": false,
                "borderless": true,
                "display_mode": "borderless_fullscreen",
                "settings_file_sha256": "81e7be36cf183e982cf8ac99c174a8f1ac64fce4f358f480840067286beb4ef2"
            }
        });
    }
}

fn fx_division_movement(seed: u64) -> Scenario {
    let s = scenario(
        seed,
        "fx_division_movement",
        "Division position, movement target, progress, combat flag, and counter-badge fields change.",
        &["requirements:3", "plan:A2:division_movement"],
        &["clock", "country_identity", "divisions", "land_combats", "province_control"],
        &["No Step Back"],
        "english",
        |tick, snap| {
            let loc = if tick < 4 { 1000 } else if tick < 8 { 1001 } else { 2000 };
            snap["divisions"] = json!([
                division(101, 1, "GER", loc, 10, tick < 8, if tick < 8 { Some(2000) } else { None }, tick as f64 / 11.0),
                division(201, 1, "SOV", 2000, 20, false, None, 0.0)
            ]);
            if tick >= 6 && tick <= 9 {
                snap["divisions"][0]["in_combat"] = json!(true);
                snap["combats"] = json!([{
                    "combat_id": 9001,
                    "kind": "land",
                    "location_province": loc,
                    "attackers": [101],
                    "defenders": [201],
                    "combat_width": 80.0,
                    "terrain": "plains",
                    "progress": tick as f64 / 10.0
                }]);
            }
        },
    );
    let h8 = hour_at(&s, 8);
    with_probe(
        s,
        Probe::new(
            h8,
            &["divisions", "0", "location_province"],
            json!(2000),
            "division reaches target province",
        ),
    )
}

fn fx_naval_air_activity(seed: u64) -> Scenario {
    let s = scenario(
        seed,
        "fx_naval_air_activity",
        "Region-level naval control, air activity, fleets, task forces, ships, and air wings evolve.",
        &["requirements:5", "requirements:8", "plan:A2:naval_air_activity"],
        &["clock", "country_identity", "naval_air", "order_of_battle", "ships", "air_wings"],
        &["Man the Guns", "By Blood Alone"],
        "english",
        |tick, snap| {
            let controller = if tick < 5 { "ENG" } else { "GER" };
            snap["naval_air"] = json!({
                "region_control": {
                    "301": [
                        {"controller": controller, "tri_state": if tick < 5 { "friendly" } else { "enemy" }, "supremacy": 0.55 + tick as f64 * 0.02}
                    ]
                },
                "air_superiority": {
                    "301": {"controller": if tick < 7 { "ENG" } else { "GER" }, "mission_mask": 5, "wing_count": 2 + tick as i64, "plane_count": 120 + tick as i64 * 10}
                },
                "source": "M",
                "confidence": "region_proxy"
            });
            snap["order_of_battle"]["fleets"] = json!([
                {"id": 500, "owner": "ENG", "name": "Home Fleet", "leader": 44,
                 "ui_eligible": true, "ui_ordinal": 0,
                 "_r2_oob_command_capture": {
                     "kind": 3, "member_status": 1, "owner_status": 1,
                     "logical_member_count": 1, "member_count_engine": 1
                 },
                 "task_forces": [
                    {"id": 501, "id_space": 61, "name": "Channel Patrol", "location_province": 3000, "owner": "ENG", "mission": 1, "ship_count": 8 + tick as i64, "dominant_hull_class": "capital",
                     "_r2_oob_command_capture": {"kind": 4, "parent_status": 1, "owner_status": 1}}
                ]}
            ]);
            snap["order_of_battle"]["extra"] = json!({
                "naval_missions_status_live": {
                    "source": "C",
                    "confidence": "validated_slice",
                    "projection": "R2 order_of_battle.extra annotation",
                    "task_force_existence_owner_domain": "oob_command_live",
                    "ship_existence_owner_domain": "ships",
                    "task_force_count": 1,
                    "fixture_required_rows": 0,
                    "oob_command_join_checked": true,
                    "oob_command_orphan_rows": 0,
                    "task_forces": [{
                        "task_force_id": 501,
                        "fleet_id": 500,
                        "owner": "ENG",
                        "owner_tag": "ENG",
                        "owner_tag_db_id": 3,
                        "location_province_id": 3000,
                        "location_province": 3000,
                        "strategic_region_id": 301,
                        "ship_count": 8 + tick as i64,
                        "dominant_hull_class": "capital",
                        "mission_enum": 1,
                        "mission_name": "PATROL",
                        "mission_state": 0,
                        "detached_activity": "none",
                        "activity_key": null,
                        "active_combat_epochs": null,
                        "combat_membership_authority": null,
                        "home_base_province_id": 3000,
                        "repair_base_province_id": null,
                        "sunk_convoys": 0,
                        "oob_command_task_force_seen": true
                    }]
                }
            });
            snap["order_of_battle"]["ships"] = json!([
                {"instance_id": 7000, "definition": "battleship_1936", "category": "capital", "strength_hp": 100.0 - tick as f64, "organisation": 0.7}
            ]);
            snap["order_of_battle"]["air_wings"] = json!([
                {"id": 800, "owner": "ENG", "name": "Fighter Wing", "count": 100 + tick as i64, "mission": "air_superiority", "strategic_region": 301, "air_base": "3000:1", "state_id": 30, "effective_planes": 90 + tick as i64}
            ]);
        },
    );
    let h7 = hour_at(&s, 7);
    with_probe(
        s,
        Probe::new(
            h7,
            &["naval_air", "air_superiority", "301", "controller"],
            json!("GER"),
            "air controller changes",
        ),
    )
}

fn fx_building_damage(seed: u64) -> Scenario {
    let s = scenario(
        seed,
        "fx_building_damage",
        "State and province building levels, damage, repair, and construction queue evolve.",
        &["requirements:10", "plan:A2:building_damage"],
        &[
            "clock",
            "country_identity",
            "state_detail",
            "buildings_live",
            "production_live",
        ],
        &["Arms Against Tyranny"],
        "english",
        |tick, snap| {
            let damage = (tick as f64 * 0.07).min(0.75);
            snap["states"]["10"]["buildings"] = json!({"infrastructure": 5, "arms_factory": 4, "industrial_complex": 3, "anti_air_building": 1});
            snap["states"]["10"]["building_damage"] = json!([
                {"scope": "state", "scope_id": 10, "building_type": "arms_factory", "level": 4, "healthy_level": if tick < 6 { 4 } else { 2 }, "damaged": tick >= 6, "damage_fraction": damage, "repair_progress": (tick as f64 / 11.0), "under_construction": false},
                {"scope": "province", "scope_id": 1000, "building_type": "naval_base", "level": 2, "healthy_level": 1, "damaged": true, "damage_fraction": 0.25, "repair_progress": 0.5, "under_construction": false}
            ]);
            snap["countries"]["GER"]["economy"]["construction_queue"] = json!([
                {"template": "arms_factory", "location_state": 10, "active_factories": 5, "produced": tick as f64 * 120.0, "cost": 7200.0, "priority": 1, "created_date": "1936.1.1.0"}
            ]);
            snap["countries"]["GER"]["production"]["construction_queue"] = json!([
                {"id": 900, "template_id": 42, "active_factories": 5, "cost": 7200.0, "produced": tick as f64 * 120.0, "priority": 1}
            ]);
        },
    );
    let h6 = hour_at(&s, 6);
    with_probe(
        s,
        Probe::new(
            h6,
            &["states", "10", "building_damage", "0", "healthy_level"],
            json!(2),
            "factory damage state",
        ),
    )
}

fn fx_pov_visibility(seed: u64) -> Scenario {
    let s = scenario(
        seed,
        "fx_pov_visibility",
        "Per-player-country province visibility and spotted unit sets flip independently.",
        &["requirements:13", "plan:A2:pov_visibility"],
        &[
            "clock",
            "country_identity",
            "intel",
            "divisions",
            "naval_air",
            "air_wings",
        ],
        &["La Resistance", "No Step Back"],
        "english",
        |tick, snap| {
            let ger_1000 = if tick < 5 { "visible" } else { "fogged" };
            let ger_spots = if tick < 5 {
                json!({"divisions": ["201:1"], "fleets": ["501"], "wings": []})
            } else {
                json!({"divisions": [], "fleets": [], "wings": []})
            };
            let eng_2000 = if tick < 7 { "fogged" } else { "visible" };
            snap["visibility"] = json!({
                "cadence": "hourly",
                "pov_tags": ["GER", "ENG", "SOV"],
                "povs": {
                    "GER": {
                        "provinces": {"1000": ger_1000, "1001": "visible", "2000": "fogged", "3000": "fogged"},
                        "visible_foreign_units": ger_spots,
                        "events": [
                            {"hour_index": snap["hour_index"].clone(), "type": if tick < 5 { "unit_spotted" } else { "unit_lost" }, "unit": "201:1", "unit_kind": "division"}
                        ]
                    },
                    "ENG": {
                        "provinces": {"1000": "fogged", "1001": "fogged", "2000": eng_2000, "3000": "visible"},
                        "visible_foreign_units": {"divisions": if tick >= 7 { json!(["101:1"]) } else { json!([]) }, "fleets": [], "wings": ["800"]},
                        "events": []
                    },
                    "SOV": {
                        "provinces": {"1000": "fogged", "1001": "fogged", "2000": "visible", "3000": "fogged"},
                        "visible_foreign_units": {"divisions": ["101:1"], "fleets": [], "wings": []},
                        "events": []
                    }
                }
            });
        },
    );
    let h5 = hour_at(&s, 5);
    with_probe(
        s,
        Probe::new(
            h5,
            &["visibility", "povs", "GER", "provinces", "1000"],
            json!("fogged"),
            "GER loses active province visibility",
        ),
    )
}

fn fx_dlc_language_variation(seed: u64) -> Scenario {
    let s = scenario(
        seed,
        "fx_dlc_language_variation",
        "Recording-level DLC set and localisation language select identity/localisation variants.",
        &[
            "requirements:4",
            "requirements:9",
            "plan:A2:dlc_language_variation",
        ],
        &["clock", "country_identity", "world_meta"],
        &["Together for Victory"],
        "german",
        |tick, snap| {
            let dlcs = if tick < 6 {
                json!(["Together for Victory"])
            } else {
                json!(["Together for Victory", "La Resistance", "Man the Guns"])
            };
            snap["world"]["scenario"]["dlcs"] = dlcs;
            snap["world"]["scenario"]["localisation_language"] = if tick < 6 {
                json!("german")
            } else {
                json!("english")
            };
            snap["world"]["scenario"]["localisation_join"] = json!({
                "language": snap["world"]["scenario"]["localisation_language"].clone(),
                "modset_id": MODSET,
                "sample_keys": {
                    "CAN_UK": if tick < 6 { "KANADA" } else { "Canada" },
                    "GER_fascism": if tick < 6 { "DEUTSCHES REICH" } else { "German Reich" }
                }
            });
            snap["countries"]["CAN"] = country("CAN", false, "democratic", 40, 14.0);
            let cosmetic = if tick < 6 {
                json!("CAN_UK")
            } else {
                json!("CAN_ALY")
            };
            snap["countries"]["CAN"]["misc"]["cosmetic_tag"] = cosmetic.clone();
            snap["countries"]["CAN"]["extra"]["cosmetic_tag"] = cosmetic;
        },
    );
    let h6 = hour_at(&s, 6);
    with_probe(
        s,
        Probe::new(
            h6,
            &["world", "scenario", "localisation_language"],
            json!("english"),
            "language selection changes",
        ),
    )
}


fn fx_research_focus(seed: u64) -> Scenario {
    let s = scenario(
        seed,
        "fx_research_focus",
        "Per-country research (completed count / in-progress / slots) and national-focus (current / completed / progress) evolve.",
        &["requirements:14", "plan:FXSTREAMS:research_focus"],
        &["clock", "country_identity", "tech_live", "focus_live"],
        &["No Step Back"],
        "english",
        |tick, snap| {
            let t = tick as i64;
            snap["countries"]["GER"]["research"]["tech_count"] = json!(4 + t);
            snap["countries"]["GER"]["research"]["research_slots"] =
                json!(if tick < 6 { 3 } else { 4 });
            snap["countries"]["GER"]["research"]["technologies"] = json!([
                {"name": "infantry_weapons", "level": 1, "date": "1936.1.1.0", "research_points": 10.0},
                {"name": "fighter1", "level": 1, "date": "1936.1.2.0", "research_points": 12.0}
            ]);
            snap["countries"]["GER"]["research"]["in_progress"] = json!([
                {"name": if tick < 6 { "infantry_weapons2" } else { "artillery2" }, "points_factor": 1.0, "progress_pct": tick as f64 / 11.0}
            ]);
            snap["countries"]["GER"]["focus"]["current"] =
                if tick < 8 { json!("GER_rhineland") } else { Value::Null };
            snap["countries"]["GER"]["focus"]["completed"] = if tick < 4 {
                json!(["GER_army_effort"])
            } else {
                json!(["GER_army_effort", "GER_rhineland"])
            };
            snap["countries"]["GER"]["focus"]["completed_count"] =
                json!(if tick < 4 { 1 } else { 2 });
            snap["countries"]["GER"]["focus"]["progress"] = json!(tick as f64 * 3.0);
        },
    );
    let h8 = hour_at(&s, 8);
    with_probe(
        s,
        Probe::new(
            h8,
            &["countries", "GER", "research", "tech_count"],
            json!(12),
            "research tech_count sweep",
        ),
    )
}

fn fx_country_management(seed: u64) -> Scenario {
    let s = scenario(
        seed,
        "fx_country_management",
        "Per-country politics detail, successor production queue+stockpile, intel/operatives, trade, and command abilities evolve.",
        &["requirements:14", "plan:FXSTREAMS:country_management", "rendering-laws:XUI12"],
        &[
            "clock",
            "country_identity",
            "politics_live",
            "production_live",
            "intel",
            "economy",
            "abilities_live",
        ],
        &["La Resistance"],
        "english",
        |tick, snap| {
            let t = tick as i64;
            let ger = &mut snap["countries"]["GER"];
            ger["politics"]["party_popularities"] = json!({
                "fascism": 0.60 - tick as f64 * 0.01,
                "democratic": 0.20,
                "communism": 0.15,
                "neutrality": 0.05 + tick as f64 * 0.01
            });
            ger["politics"]["ideas"] =
                json!(["german_reich_spirit", if tick < 6 { "tolerant_society" } else { "hardened_society" }]);
            ger["politics"]["national_spirits"] = json!(["prussian_army"]);
            ger["politics"]["laws"] = json!({
                "economy": if tick < 6 { "civilian_economy" } else { "partial_mobilisation" },
                "trade": "export_focus",
                "conscription": "limited_conscription"
            });
            ger["politics"]["appointed_advisors"] =
                json!(["general_staff", if tick < 6 { "captain_of_industry" } else { "war_industrialist" }]);
            ger["abilities"] = json!([
                {"ability": "battle_cry", "active": tick % 2 == 0, "cooldown": t},
                {"ability": "force_attack", "active": false, "cooldown": 0}
            ]);
            ger["production"]["stockpile_total"] = json!(1000.0 + tick as f64 * 50.0);
            ger["production"]["stockpile"] = json!({"1:1": 900 + t * 10, "2:3": 120 - t});
            ger["production"]["stockpile_complete"] = json!(true);
            ger["production"]["military_lines_complete"] = json!(true);
            ger["production"]["naval_lines_complete"] = json!(true);
            ger["production"]["military_lines"] = json!([{
                "id": 41,
                "priority": 0,
                "line_subtype": "military",
                "equipment_variant_id": "1:1",
                "active_factories": 7,
                "requested_factories": 10,
                "efficiency": 0.72 + tick as f64 * 0.001,
                "efficiency_cap": 0.85,
                "efficiency_growth": 0.003,
                "cost": 4.5,
                "speed": 18.0,
                "produced": 2.25 + tick as f64,
                "amount": -1,
                "progress_ratio": 0.5,
                "daily_output": 4.0,
                "days_to_next_unit": 0.125,
                "mio_id": "german_tank_organization"
            }]);
            ger["production"]["naval_lines"] = json!([{
                "id": 72,
                "priority": 1,
                "line_subtype": "ship_refit",
                "equipment_variant_id": "2:3",
                "active_factories": 3,
                "requested_factories": 4,
                "efficiency": 0.61,
                "efficiency_cap": 0.8,
                "efficiency_growth": 0.002,
                "cost": 9.0,
                "speed": 7.0,
                "produced": 1.0,
                "amount": 2,
                "progress_ratio": 0.25,
                "daily_output": 1.5,
                "days_to_next_unit": 0.5,
                "mio_id": "german_naval_organization"
            }]);
            ger["production"]["construction_queue"] = json!([
                {"id": 900, "template_id": 42, "active_factories": 5, "cost": 7200.0, "produced": tick as f64 * 120.0, "priority": 1}
            ]);
            ger["military"]["equipment_stockpile"] = json!([
                {"archetype": "infantry_equipment", "equip_id": 1, "equip_type": 1, "amount": 900.0 + tick as f64 * 10.0},
                {"archetype": "artillery_equipment", "equip_id": 2, "equip_type": 3, "amount": 60.0}
            ]);
            ger["intel"]["agency"] = json!({
                "name": "Abwehr",
                "is_created": true,
                "upgrade_progress": tick as f64 / 11.0,
                "capacity": {"max_operatives": 3 + t, "usable_slots": 2, "defense": 5}
            });
            ger["intel"]["operatives"] = json!([
                {"op_id": 7001, "name": "Agent Alpha", "skill": 2 + t, "state": if tick < 6 { "on_mission" } else { "on_operation" }, "captured": false}
            ]);
            ger["intel"]["operations"] = json!({
                "priority": 1,
                "running": if tick >= 6 {
                    json!([{"op_id": 5001, "type": "coup", "target": "FRA", "duration": 30}])
                } else {
                    json!([])
                },
                "finished": []
            });
            ger["economy"]["resources"] = json!({
                "complete": true,
                "produced": {"oil": 5.0, "aluminium": 0.0, "rubber": 0.0, "tungsten": 2.0, "steel": 40.0 + tick as f64, "chromium": 0.0, "coal": 0.0},
                "imported": {"oil": 0.0, "aluminium": 0.0, "rubber": 0.0, "tungsten": 0.0, "steel": 0.0, "chromium": 0.0, "coal": 0.0},
                "used": {"oil": 3.0, "aluminium": 0.0, "rubber": 0.0, "tungsten": 1.0, "steel": 12.0, "chromium": 0.0, "coal": 0.0},
                "exported": {"oil": 0.0, "aluminium": 0.0, "rubber": 0.0, "tungsten": 0.0, "steel": 5.0, "chromium": 0.0, "coal": 0.0},
                "to_export": {"oil": 0.0, "aluminium": 0.0, "rubber": 0.0, "tungsten": 0.0, "steel": 3.0, "chromium": 0.0, "coal": 0.0},
                "base_export": {"oil": 0.0, "aluminium": 0.0, "rubber": 0.0, "tungsten": 0.0, "steel": 8.0, "chromium": 0.0, "coal": 0.0},
                "available_derived": {"oil": 2.0, "aluminium": 0.0, "rubber": 0.0, "tungsten": 1.0, "steel": 23.0 + tick as f64, "chromium": 0.0, "coal": 0.0}
            });
            ger["diplomacy"]["lend_lease"] = json!([
                {"contract_id": "10:2", "direction": "incoming", "counterpart": "SOV", "seller": "SOV", "buyer": "GER", "convoys": {"assigned": 4 + t, "total": 10}, "completed": false}
            ]);
        },
    );
    let h7 = hour_at(&s, 7);
    let s = with_probe(
        s,
        Probe::new(
            h7,
            &["countries", "GER", "production", "stockpile_total"],
            json!(1350.0),
            "production stockpile_total sweep",
        ),
    );
    let s = with_probe(
        s,
        Probe::new(
            h7,
            &[
                "countries",
                "GER",
                "production",
                "military_lines",
                "0",
                "line_subtype",
            ],
            json!("military"),
            "XUI12 successor production line subtype",
        ),
    );
    with_probe(
        s,
        Probe::new(
            h7,
            &[
                "countries",
                "GER",
                "intel",
                "operations",
                "running",
                "0",
                "type",
            ],
            json!("coup"),
            "intel operation running",
        ),
    )
}

fn fx_designs_catalog(seed: u64) -> Scenario {
    let s = scenario(
        seed,
        "fx_designs_catalog",
        "World design catalog: division templates and equipment variants evolve (regiment count, naming, obsolescence).",
        &["requirements:14", "plan:FXSTREAMS:designs_catalog"],
        &["clock", "country_identity", "designs"],
        &["No Step Back", "By Blood Alone"],
        "english",
        |tick, snap| {
            snap["designs"]["division_templates"] = json!([
                {"id": "1:1", "type": 1, "country": "GER", "name": "Infanterie", "role": "infantry", "n_regiments": 6 + tick as i64, "n_support": 2},
                {"id": "2:1", "type": 1, "country": "SOV", "name": "Strelkovaya", "role": "infantry", "n_regiments": 9, "n_support": 1}
            ]);
            snap["designs"]["equipment_variants"] = json!({
                "100:3": {"id": 100, "type": 3, "archetype": "artillery_equipment", "family": "land_other", "name": if tick < 6 { Value::Null } else { json!("le.FH 18") }, "version": tick as i64, "obsolete": false},
                "200:12": {"id": 200, "type": 12, "archetype": "fighter_equipment", "family": "plane", "name": "Bf 109", "version": 1, "obsolete": tick >= 8}
            });
        },
    );
    let h8 = hour_at(&s, 8);
    with_probe(
        s,
        Probe::new(
            h8,
            &["designs", "division_templates", "0", "n_regiments"],
            json!(14),
            "division template regiment growth",
        ),
    )
}

fn fx_occupation_sweep(seed: u64) -> Scenario {
    let s = scenario(
        seed,
        "fx_occupation_sweep",
        "State occupation resistance/compliance (and speeds, law, garrison status) evolve on an occupied state.",
        &["requirements:14", "plan:FXSTREAMS:occupation_sweep"],
        &["clock", "country_identity", "state_ownership", "state_control", "state_detail"],
        &["La Resistance"],
        "english",
        |tick, snap| {
            snap["states"]["20"]["controller"] = json!("GER");
            snap["provinces"]["2000"]["controller"] = json!("GER");
            snap["states"]["20"]["occupation"] = json!({
                "resistance": (tick as f64 * 4.0).min(40.0),
                "resistance_speed": 0.5 - tick as f64 * 0.03,
                "compliance": (tick as f64 * 2.0).min(25.0),
                "compliance_speed": 0.2,
                "occupied_country_tag": "SOV",
                "operational_status": if tick < 6 { "occupied" } else { "martial_law" }
            });
            snap["states"]["20"]["occupation_law"] =
                json!(if tick < 6 { "civilian_oversight" } else { "harsh_quotas" });
        },
    );
    let h6 = hour_at(&s, 6);
    with_probe(
        s,
        Probe::new(
            h6,
            &["states", "20", "occupation", "occupied_country_tag"],
            json!("SOV"),
            "occupation occupied-country tag",
        ),
    )
}

fn fx_leader_roster(seed: u64) -> Scenario {
    let s = scenario(
        seed,
        "fx_leader_roster",
        "Country leader plus field-marshal/general/admiral roster with traits and assignments evolve.",
        &["requirements:14", "plan:FXSTREAMS:leader_roster"],
        &["clock", "country_identity", "characters", "politics_live"],
        &["No Step Back"],
        "english",
        |tick, snap| {
            let t = tick as i64;
            let ger = &mut snap["countries"]["GER"];
            ger["commanders"]["leader"] = json!({
                "char_id": 1000, "name": "Country Leader", "token": "GER_leader", "ideology_group": "fascism", "source": "C"
            });
            ger["commanders"]["field_marshals"] = json!([
                {"name": "von Rundstedt", "token": "GER_rundstedt", "level": 4 + t, "experience": tick as f64, "traits": ["old_guard", if tick < 6 { "infantry_leader" } else { "panzer_leader" }], "assignment": if tick < 6 { Value::Null } else { json!({"kind": "army_group", "name": "Heeresgruppe A"}) }}
            ]);
            ger["commanders"]["generals"] = json!([
                {"name": "Guderian", "token": "GER_guderian", "level": 5, "experience": 10.0 + tick as f64, "traits": ["panzer_leader", "guerilla_fighter"], "assignment": {"kind": "army", "name": "2. Armee"}}
            ]);
            ger["commanders"]["admirals"] = json!([
                {"name": "Doenitz", "token": "GER_donitz", "level": 4, "traits": ["seawolf"], "assignment": Value::Null}
            ]);
            ger["commanders"]["counts"] =
                json!({"generals": 1, "field_marshals": 1, "admirals": 1, "advisors": 2, "roster_size": 4 + t});
        },
    );
    let h9 = hour_at(&s, 9);
    with_probe(
        s,
        Probe::new(
            h9,
            &[
                "countries",
                "GER",
                "commanders",
                "field_marshals",
                "0",
                "level",
            ],
            json!(13),
            "field marshal skill growth",
        ),
    )
}

fn fx_power_balance_combat(seed: u64) -> Scenario {
    let s = scenario(
        seed,
        "fx_power_balance_combat",
        "BBA power-balance slider, naval/air combat detail, convoy-loss aggregates, and raid priorities evolve.",
        &["requirements:14", "plan:FXSTREAMS:power_balance_combat"],
        &["clock", "country_identity", "world_meta", "naval_air", "order_of_battle", "events", "raids"],
        &["By Blood Alone", "Man the Guns"],
        "english",
        |tick, snap| {
            let t = tick as i64;
            snap["world"]["power_balances"] = json!([
                {"template": "GER_alignment", "value": -0.5 + tick as f64 * 0.08, "left_side": "hardliners", "right_side": "moderates", "trending_side": if tick < 6 { "left" } else { "right" }, "countries": ["GER"]}
            ]);
            snap["naval_combats"] = json!([
                {"naval_combat_id": 8001, "location_province": 3000, "attacker": "ENG", "defender": "GER", "attacker_task_force": "501:1", "defender_task_force": "600:1", "result_type": if tick < 6 { "naval_battle" } else { "port_strike" }, "progress": tick as f64 / 11.0, "attacker_ships": 8, "defender_ships": 4}
            ]);
            snap["air_combats"] = json!([
                {"air_combat_id": 8500, "strategic_region": 301, "attacker": "ENG", "defender": "GER", "attacker_planes": 100 - t, "defender_planes": 60}
            ]);
            snap["world_events"]["convoy_losses_by_owner"] = json!({"ENG": 10 + t});
            snap["raids"]["countries"] = json!([{"tag": "ENG", "priority": 2}]);
            snap["raids"]["counters"] =
                json!({"next_state": 1, "existing_target_num": t, "detectable_target_num": t, "next_target": 5});
        },
    );
    let h8 = hour_at(&s, 8);
    with_probe(
        s,
        Probe::new(
            h8,
            &["world", "power_balances", "0", "trending_side"],
            json!("right"),
            "power balance trending side flip",
        ),
    )
}

fn fx_scripted_gui_vars(seed: u64) -> Scenario {
    let s = scenario(
        seed,
        "fx_scripted_gui_vars",
        "Per-country scripted-GUI variable state and world variables/flags evolve (national-mechanic panel inputs).",
        &["requirements:14", "plan:FXSTREAMS:scripted_gui_vars"],
        &["clock", "country_identity", "world_meta", "scripted_gui"],
        &["Together for Victory"],
        "english",
        |tick, snap| {
            let t = tick as f64;
            snap["countries"]["GER"]["extra"]["_r2_script_variables"] =
                json!({"inner_circle_loyalty": 10.0 + t, "rk_influence": 0.5});
            snap["countries"]["GER"]["extra"]["script_flags"] = json!({
                "inner_circle_active": {"value_raw": 1, "date_hour_raw": 12}
            });
            snap["countries"]["USA"] = country("USA", false, "democratic", 361, 25.0);
            snap["countries"]["USA"]["extra"]["_r2_script_variables"] =
                json!({"congress_support": 50.0 - t * 2.0, "war_readiness^0": 1.0});
            snap["world"]["variables"] = json!({"global_tension_mod": t, "campaign_phase^1": 3.0});
            snap["world"]["flags"] = json!({
                "war_escalation_started": {"value": tick >= 6, "date": "1936.1.1.0"}
            });
        },
    );
    let h8 = hour_at(&s, 8);
    with_probe(
        s,
        Probe::new(
            h8,
            &[
                "countries",
                "GER",
                "extra",
                "_r2_script_variables",
                "inner_circle_loyalty",
            ],
            json!(18.0),
            "scripted-GUI country variable sweep",
        ),
    )
}

fn pack_scenario(scenario: &Scenario, seed: u64) -> Result<Vec<u8>> {
    let game_id = &scenario.name;
    let mut entries = Vec::new();
    let mut index_entries = Vec::new();

    for snap in &scenario.snapshots {
        let hour = snap["hour_index"]
            .as_i64()
            .ok_or("snapshot has no integer hour_index")?;
        let date = snap["date"].as_str().ok_or("snapshot has no date")?;
        let file = format!("{hour:012}.snapshot.json");
        index_entries.push(json!({
            "hour_index": hour,
            "date": date,
            "file": file,
            "save_version": snap["save_version"].clone(),
            "counts": {
                "countries": snap["countries"].as_object().map_or(0, Map::len),
                "states": snap["states"].as_object().map_or(0, Map::len),
                "provinces": snap["provinces"].as_object().map_or(0, Map::len),
                "divisions": snap["divisions"].as_array().map_or(0, Vec::len)
            }
        }));
        entries.push(PackEntry {
            path: format!("payload/{game_id}/{file}"),
            bytes: serde_json::to_vec(snap)?,
            role: "snapshot",
            hour_index: Some(hour),
        });
    }

    let index = json!({
        "game_id": game_id,
        "schema_version": "1",
        "snapshots": index_entries,
    });
    entries.push(PackEntry {
        path: format!("payload/{game_id}/index.json"),
        bytes: serde_json::to_vec(&index)?,
        role: "index",
        hour_index: None,
    });

    let timeline = json!({
        "game_id": game_id,
        "schema_version": "1",
        "format": "hoi4replay_fixture_timeline_v1",
        "event_count": scenario.events.len(),
        "events": scenario.events,
    });
    entries.push(PackEntry {
        path: format!("payload/{game_id}/timeline.json"),
        bytes: serde_json::to_vec(&timeline)?,
        role: "timeline",
        hour_index: None,
    });

    let mut raw = format!(
        "{{\"type\":\"collector_started\",\"scenario\":\"{}\",\"seed\":{}}}\n",
        scenario.name, seed
    );
    for (i, snap) in scenario.snapshots.iter().enumerate() {
        let delta = json!({
            "type": "domain_delta",
            "sequence": i + 1,
            "hour_index": snap["hour_index"],
            "date": snap["date"],
            "scenario": scenario.name,
            "contracts": scenario.contracts.clone(),
            "snapshot_hash_authority": "container manifest entry sha256"
        });
        raw.push_str(&serde_json::to_string(&delta)?);
        raw.push('\n');
    }
    raw.push_str(&format!(
        "{{\"type\":\"fixture_summary\",\"snapshots\":{},\"contracts\":{}}}\n",
        scenario.snapshots.len(),
        serde_json::to_string(&scenario.contracts)?
    ));
    entries.push(PackEntry {
        path: format!("payload/{game_id}/collector_raw/collector_frames.jsonl"),
        bytes: raw.into_bytes(),
        role: "collector_raw_stream",
        hour_index: None,
    });

    Ok(pack(
        game_id,
        &entries,
        &pack_identity(seed, MODSET)?,
        true,
    )?)
}

fn scenario_manifest(scenario: &Scenario, file: &Path, root: &Path, bytes: u64) -> Value {
    let hours = scenario
        .snapshots
        .iter()
        .filter_map(|s| s["hour_index"].as_i64())
        .collect::<Vec<_>>();
    let probes = scenario
        .probes
        .iter()
        .map(|p| {
            json!({
                "hour": p.hour,
                "path": p.path,
                "equals": p.equals,
                "note": p.note,
            })
        })
        .collect::<Vec<_>>();
    let mut entry = json!({
        "name": scenario.name,
        "path": rel_slash(file, root),
        "description": scenario.description,
        "contracts": scenario.contracts,
        "snapshot_count": scenario.snapshots.len(),
        "hours": hours,
        "probes": probes,
        "bytes": bytes,
    });
    if let Some(anomaly) = &scenario.anomaly {
        entry["anomaly"] = anomaly.clone();
    }
    entry
}

fn negative_fixtures(seed: u64) -> Result<Vec<NegativeFixture>> {
    let mut out = Vec::new();

    let malformed_opts = PackOptions {
        required_features_raw: Some(json!("future_delta_encoding_v2")),
        ..Default::default()
    };
    out.push(NegativeFixture {
        name: "neg_malformed_manifest",
        description: "Manifest required_features has the wrong JSON type.",
        expected_error_contains: "required_features",
        bytes: pack_with(
            "neg_malformed_manifest",
            &minimal_entries("neg_malformed_manifest", MODSET, "clean")?,
            &pack_identity(seed, MODSET)?,
            false,
            &malformed_opts,
        )?,
    });

    let mut bad_hash = pack(
        "neg_bad_snapshot_hash",
        &minimal_entries("neg_bad_snapshot_hash", MODSET, "clean")?,
        &pack_identity(seed, MODSET)?,
        false,
    )?;
    let needle = b"\"negative_hash_marker\":\"clean\"";
    let pos = bad_hash
        .windows(needle.len())
        .position(|w| w == needle)
        .ok_or("negative hash marker not found in STORE zip")?;
    bad_hash[pos + needle.len() - 3] = b'X';
    out.push(NegativeFixture {
        name: "neg_bad_snapshot_hash",
        description: "First snapshot payload is changed after manifest hashing. The stitching fix6 whole-archive payload CRC-32 validation at open now catches the byte change (defense-in-depth) before the manifest sha256 check.",
        expected_error_contains: "crc32 mismatch",
        bytes: bad_hash,
    });

    out.push(NegativeFixture {
        name: "neg_modset_mismatch",
        description: "Manifest modset id disagrees with the first snapshot provenance.mod.",
        expected_error_contains: "disagrees with snapshot provenance.mod",
        bytes: pack(
            "neg_modset_mismatch",
            &minimal_entries("neg_modset_mismatch", "snapshot-side-modset", "clean")?,
            &pack_identity(seed, "manifest-side-modset")?,
            false,
        )?,
    });

    let game = "neg_empty_snapshot_index";
    let compact_index = json!({
        "game_id": game,
        "schema_version": "native_collector_session_v1",
        "snapshots": []
    });
    let compact_timeline = json!({
        "format": "hoi4_compact_stream_timeline_v1",
        "raw_stream_role": "collector_compact_stream",
        "stream": "collector_raw/compact_stream.jsonl"
    });
    let compact_entries = vec![
        PackEntry {
            path: format!("payload/{game}/index.json"),
            bytes: serde_json::to_vec(&compact_index)?,
            role: "index",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game}/timeline.json"),
            bytes: serde_json::to_vec(&compact_timeline)?,
            role: "timeline",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game}/collector_raw/compact_stream.jsonl"),
            bytes: b"compact-stream-fixture\n".to_vec(),
            role: "collector_compact_stream",
            hour_index: None,
        },
    ];
    let mut compact_identity = pack_identity(seed, MODSET)?;
    compact_identity.schema_version = Some("native_collector_session_v1".to_string());
    out.push(NegativeFixture {
        name: "neg_empty_snapshot_index",
        description: "Compact-stream flavor signature with an obsolete non-v1 index schema. Compact-only packages are now product-supported only through the certified worker and must still declare canonical replay index schema_version 1.",
        expected_error_contains: "incompatible replay index schema_version",
        bytes: pack(game, &compact_entries, &compact_identity, true)?,
    });

    Ok(out)
}

fn minimal_entries(game: &str, snapshot_modset: &str, marker: &str) -> Result<Vec<PackEntry>> {
    let (date, hour) = fixture_time(0);
    let mut snap = base_snapshot(
        game,
        &date,
        hour,
        &["clock", "country_identity", "state_ownership"],
        &["Together for Victory"],
        "english",
        DEFAULT_SEED,
        0,
    );
    snap["provenance"]["mod"] = mod_identity(snapshot_modset);
    snap["negative_hash_marker"] = json!(marker);
    refresh_division_counts(&mut snap);
    refresh_map_layers(&mut snap);

    let file = format!("{hour:012}.snapshot.json");
    let index = json!({
        "game_id": game,
        "schema_version": "1",
        "snapshots": [{"hour_index": hour, "date": date, "file": file}]
    });
    Ok(vec![
        PackEntry {
            path: format!("payload/{game}/index.json"),
            bytes: serde_json::to_vec(&index)?,
            role: "index",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game}/{file}"),
            bytes: serde_json::to_vec(&snap)?,
            role: "snapshot",
            hour_index: Some(hour),
        },
    ])
}

fn base_snapshot(
    game_id: &str,
    date: &str,
    hour_index: i64,
    domains: &[&str],
    dlcs: &[&str],
    language: &str,
    seed: u64,
    salt: u64,
) -> Value {
    let mut countries = Map::new();
    countries.insert("GER".to_string(), country("GER", true, "fascism", 10, 40.0));
    countries.insert(
        "SOV".to_string(),
        country("SOV", true, "communism", 20, 30.0),
    );
    countries.insert(
        "ENG".to_string(),
        country("ENG", true, "democratic", 30, 50.0),
    );
    countries.insert(
        "FRA".to_string(),
        country("FRA", false, "democratic", 20, 20.0),
    );
    countries.insert(
        "ITA".to_string(),
        country("ITA", false, "fascism", 10, 15.0),
    );

    let dlc_values = dlcs.iter().map(|d| json!(d)).collect::<Vec<_>>();
    let domain_values = domains.iter().map(|d| json!(d)).collect::<Vec<_>>();
    let mut snap = json!({
        "schema_version": "1",
        "game_id": game_id,
        "date": date,
        "hour_index": hour_index,
        "save_version": "fixture-build-1",
        "provenance": {
            "collector": {
                "captured": true,
                "backend": "synthetic_fixture_generator",
                "collector_version": "fixture-gen-v1",
                "exe_sha256": BUILD_SHA,
                "mod_checksum": MODSET,
                "quality_status": "complete",
                "capture_boundary": "scheduled_hour_snapshot",
                "transport": "synthetic",
                "transport_crc32": "00000000",
                "transport_bytes": 0,
                "sequence": hour_index,
                "domains": domain_values
            },
            "memory": null,
            "save": null,
            "mod": mod_identity(MODSET),
            "notes": ["generated fixture; no wall clock or random source"]
        },
        "countries": countries,
        "states": {
            "10": state("GER", "GER", "plains"),
            "20": state("SOV", "SOV", "forest"),
            "30": state("ENG", "ENG", "urban")
        },
        "provinces": {
            "1000": province("GER", 10),
            "1001": province("GER", 10),
            "2000": province("SOV", 20),
            "3000": province("ENG", 30)
        },
        "divisions": [
            division(101, 1, "GER", 1000, 10, false, null_i64(), 0.0),
            division(201, 1, "SOV", 2000, 20, false, null_i64(), 0.0)
        ],
        "combats": [],
        "diplomacy_world": {"wars": {}, "factions": {}, "subjects": []},
        "map": {
            "owner_by_state": null,
            "control_by_province": null,
            "vp_by_state": {"10": 5, "20": 3, "30": 20},
            "controlled_industry_share": null,
            "front_lines": [],
            "strategic_regions": {}
        },
        "designs": {
            "division_templates": [
                {"id": "1:1", "country": "GER", "name": "Fixture Infantry", "role": "infantry"}
            ],
            "equipment_variants": {},
            "equipment_variants_live": null,
            "equipment_variants_live_source": null,
            "template_stats": null,
            "template_stats_source": null,
            "equipment_variant_stats": null,
            "equipment_variant_stats_source": null
        },
        "order_of_battle": {
            "army_groups": [],
            "armies": [],
            "fleets": [],
            "navy_theaters": [],
            "air_wings": [],
            "theatres": [],
            "by_country": {},
            "ships": [],
            "battle_plan_orders": null
        },
        "mio_live": {"by_tag": null, "by_id": null, "count": null, "source": null},
        "market_live": {"contracts": null, "count": null, "source": null},
        "diplomacy_live": {"war_relations": null, "faction_members": null, "subtype_warning": null, "source": null},
        "special_projects": {"facilities": null, "projects": null, "scientists": null, "count": null, "source": null},
        "naval_air": {"region_control": null, "air_superiority": null, "source": null, "confidence": null},
        "supply": {"by_province": null, "source": null, "confidence": null},
        "weather": {"regions": null, "provinces": null, "global_rollup": null, "source": null},
        "models": {"fixture_seed": seed, "fixture_salt": salt},
        "intel_ledger": {},
        "strategic_air": {"air_bases": [], "indices": null, "per_country_region_blocks": null},
        "strategic_operatives": {"pools": null},
        "raids": {"counters": null, "targets": [], "countries": []},
        "world_events": {
            "fired_events": [],
            "pending_events": [],
            "threat": [],
            "convoy_losses_by_killer": null,
            "convoy_losses_by_owner": null
        },
        "world": {
            "scenario": {
                "start_date": "1936.1.1.0",
                "current_date_live": date,
                "difficulty": "normal",
                "historical": true,
                "tutorial": false,
                "dlcs": dlc_values,
                "mods": [],
                "session": {"id": format!("fixture-{game_id}-{seed}"), "seed": seed},
                "speed": 5,
                "paused": false,
                "save_version": "fixture-build-1",
                "ideology": "mixed",
                "localisation_language": language,
                "players": [
                    {"tag": "GER", "slot": 1, "label": "Fixture GER"},
                    {"tag": "SOV", "slot": 2, "label": "Fixture SOV"},
                    {"tag": "ENG", "slot": 3, "label": "Fixture ENG"}
                ],
                "gameplaysettings": {},
                "game_rules_empty": true,
                "difficulty_settings_empty": true
            },
            "variables": {},
            "flags": {},
            "tech_sharing_groups": [],
            "power_balances": [],
            "faction_influence": [],
            "world_tension_inputs": {"average_major_ic": 42.0, "tension_scaling_base_country": "GER"},
            "saved_event_targets": []
        },
        "events": []
    });
    refresh_division_counts(&mut snap);
    refresh_map_layers(&mut snap);
    snap
}

fn country(tag: &str, human: bool, party: &str, capital: i64, pp: f64) -> Value {
    json!({
        "economy": {
            "factories": {"civ": 10, "mil": 5, "dockyard": 2, "under_construction": 0},
            "construction_queue": [],
            "resources": {},
            "fuel": {"current": 500.0, "max": 2000.0, "gain": 0.0, "cost": 0.0},
            "gross_ic_by_equipment": null,
            "logistics": null,
            "market_price_levels": null,
            "market_subsidies": null
        },
        "military": {
            "num_ships": 0,
            "division_count": 0,
            "manpower": {"available": 100000, "deployed": 24000, "army": 20000, "navy": 2000, "air": 2000},
            "xp": {"army": 0.0, "navy": 0.0, "air": 0.0},
            "equipment_stockpile": [],
            "convoys": [{"equip_id": 1, "equip_type": 70, "archetype": "convoy", "amount": 50.0}],
            "convoys_count": 50,
            "ships_by_hull": null,
            "casualties": {"killed": 0, "captured": 0},
            "command_power": 10.0,
            "deployment_queue": null,
            "reinforcement_priority": null
        },
        "politics": {
            "stability": 0.5,
            "war_support": 0.4,
            "political_power": pp,
            "command_power": 10.0,
            "ruling_party": party,
            "party_popularities": {"fascism": 0.25, "democratic": 0.25, "communism": 0.25, "neutrality": 0.25},
            "laws": {},
            "ideas_all": [],
            "ideas_other": [],
            "national_spirits": [],
            "idea_groups": null,
            "advisor_ideas": null,
            "ideas": [],
            "idea_count": 0,
            "election_frequency": null,
            "party_count": 4,
            "low_confidence": false,
            "last_election": null,
            "elections_allowed": null,
            "appointed_advisors": null,
            "appointed_advisor_details": null,
            "appointed_advisor_slots": null,
            "appointed_advisor_ledgers": null,
            "cabinet": null,
            "is_human": human,
            "is_major": matches!(tag, "GER" | "SOV" | "ENG" | "FRA" | "ITA"),
            "capital_state": capital,
            "autonomy": null,
            "faction": null
        },
        "research": {
            "tech_count": 0,
            "technologies": [],
            "in_progress": [],
            "in_progress_live": null,
            "research_slots": 4,
            "original_research_slots": 4,
            "research_bonuses": []
        },
        "focus": {
            "focus_tree": format!("{tag}_focus"),
            "continuous_focus_palette": null,
            "completed": [],
            "completed_count": 0,
            "current": null,
            "current_continuous": null,
            "progress": null,
            "paused": null
        },
        "production": {
            "stockpile": null,
            "stockpile_total": null,
            "military_lines": null,
            "naval_lines": null,
            "military_factories": null,
            "naval_factories": null,
            "construction_queue": null
        },
        "decisions": {"active": [], "timed": [], "missions": [], "timed_live": null, "status_present": null},
        "diplomacy": {
            "wars": [],
            "guarantees": [],
            "non_aggression": [],
            "embargoes": [],
            "opinion": null,
            "lend_lease": [],
            "volunteers": [],
            "attaches": [],
            "expeditionary": [],
            "at_war": [],
            "casualties": 0,
            "faction_group": null,
            "relation_flags": null,
            "overlord": null,
            "autonomy_state": null,
            "autonomy_ratio": null
        },
        "intel": {
            "agency": null,
            "operatives": [],
            "operations": [],
            "decryption": null,
            "peace_conference": null,
            "operatives_live": null,
            "cryptology_live": null,
            "operative_skill_xp": null
        },
        "commanders": {"leader": null, "army_leaders": null, "navy_leaders": null},
        "scalars": {},
        "score": null,
        "mios": [],
        "scheduled_variants": [],
        "mio_policies": null,
        "order_of_battle": null,
        "extra": {"nukes_total": 0, "nukes_ready_total": 0, "cosmetic_tag": null},
        "human_name": if human { json!(format!("{tag} Player")) } else { Value::Null },
        "misc": {"convoys_destroyed": 0, "accidents_score": 0, "ace_count": 0, "cosmetic_tag": null},
        "identity": {"original_capital": capital, "original_tag": tag, "reserved_dynamic_country": false, "templates_locked": false},
        "script_variables": {},
        "country_reports": null,
        "abilities": null,
        "projects": null,
        "scientists": null
    })
}

fn state(owner: &str, controller: &str, category: &str) -> Value {
    json!({
        "owner": owner,
        "controller": controller,
        "is_core_of": [owner],
        "is_claimed_by": [],
        "buildings": {"infrastructure": 3, "arms_factory": 1, "industrial_complex": 1},
        "province_buildings": {},
        "victory_points_total": 1,
        "victory_points_by_province": {},
        "resources": {"steel": 1.0},
        "dynamic_modifier": [],
        "extra_shared_slots": 0,
        "manpower_pool": {"available": 1000, "locked": 0, "total": 1000},
        "category": category,
        "occupation": null,
        "occupation_law": null,
        "demilitarized": false,
        "supply": 1.0
    })
}

fn province(controller: &str, state_id: i64) -> Value {
    json!({
        "controller": controller,
        "state_id": state_id,
        "supply": 1.0,
        "buildings": {},
        "rail_usage": null,
        "route_usage": null,
        "naval_base_usage": null,
        "port_usage": null
    })
}

fn division(
    div_id: i64,
    div_type: i64,
    owner: &str,
    location_province: i64,
    state_id: i64,
    moving: bool,
    move_target_province: Option<i64>,
    move_progress: f64,
) -> Value {
    json!({
        "div_id": div_id,
        "div_type": div_type,
        "owner": owner,
        "location_province": location_province,
        "state_id": state_id,
        "template_id": 1,
        "template_type": 1,
        "template_name": "Fixture Infantry",
        "template_role": "infantry",
        "organisation": 42.0,
        "strength": 0.95,
        "equipment": [],
        "equipment_live": {"infantry_equipment_0": 900},
        "experience": 0.2,
        "moving": moving,
        "move_target_province": move_target_province,
        "move_progress": move_progress,
        "in_combat": false,
        "encircled": false,
        "supply_status": 0.9,
        "script_id": null,
        "division_name": {"type": "fixture", "name_order": div_id},
        "max_supply": 1.0,
        "army_current_supply_ratio": 0.9,
        "supply_gain": 0.0,
        "motorization_level": 0,
        "move_priority": "normal",
        "last_combat_date": null,
        "dig_in": 1.5,
        "dig_in_cap": 5.0,
        "acclimatization": {"hot_climate": 0.0, "cold_climate": 0.1, "max": 1.0},
        "army_manpower": {"value": 10000, "need": 0},
        "killed": 0,
        "killer_definition": null,
        "equipment_archetypes": 1,
        "has_officer": true
    })
}

fn null_i64() -> Option<i64> {
    None
}

fn refresh_division_counts(snap: &mut Value) {
    let mut counts = std::collections::BTreeMap::<String, i64>::new();
    if let Some(divisions) = snap["divisions"].as_array() {
        for div in divisions {
            if let Some(owner) = div["owner"].as_str() {
                *counts.entry(owner.to_string()).or_default() += 1;
            }
        }
    }
    if let Some(countries) = snap["countries"].as_object_mut() {
        for (tag, country) in countries {
            let count = counts.get(tag).copied().unwrap_or(0);
            country["military"]["division_count"] = json!(count);
        }
    }
}

fn refresh_map_layers(snap: &mut Value) {
    let mut owners = Map::new();
    if let Some(states) = snap["states"].as_object() {
        for (sid, state) in states {
            owners.insert(sid.clone(), state["owner"].clone());
        }
    }
    let mut controllers = Map::new();
    if let Some(provinces) = snap["provinces"].as_object() {
        for (pid, province) in provinces {
            controllers.insert(pid.clone(), province["controller"].clone());
        }
    }
    snap["map"]["owner_by_state"] = Value::Object(owners);
    snap["map"]["control_by_province"] = Value::Object(controllers);
}

fn mod_identity(modset_id: &str) -> Value {
    json!({
        "modset_id": modset_id,
        "source": "dlc_load.json",
        "load_order": ["fixture.mod"],
        "mods": [
            {"name": "Fixture Replay Mod", "workshop_id": null, "version": "1.0", "supported_version": "1.19.*", "descriptor": "fixture.mod"}
        ]
    })
}

fn pack_identity(seed: u64, modset_id: &str) -> Result<PackIdentity> {
    Ok(PackIdentity {
        schema_version: Some("1".to_string()),
        build_exe_sha256: Some(BUILD_SHA.to_string()),
        build_pinned: true,
        mod_identity: Some(mod_identity(modset_id)),
        created_utc: Some(created_utc(seed)),
        content_manifest: Some(fixture_content_manifest(seed, modset_id)?),
    })
}

/// Generated recordings therefore use the production VFS admission contract.
fn fixture_content_manifest(seed: u64, modset_id: &str) -> Result<ContentManifest> {
    let base_path = "common/replay_fixture/scenario.txt";
    let mod_path = "gfx/interface/replay_fixture/icon.dds";
    let base_sha = sha256(FIXTURE_BASE_DEFINITION);
    let mod_sha = sha256(FIXTURE_MOD_TEXTURE);
    let descriptor_bytes = format!(
        "name=Fixture Replay Mod\nid={modset_id}\nreplace_path=gfx/interface/replay_fixture\n"
    );
    let descriptor_sha = sha256(descriptor_bytes.as_bytes());
    let base_layer_id = "base:fixture-runtime".to_string();
    let mod_layer_id = format!("mod:{modset_id}");

    let effective_vfs_manifest_sha256 = sha256(&serde_json::to_vec(&json!({
        "schema": "hoi4.fixture-effective-vfs.v1",
        "layers": [
            {
                "ordinal": 0,
                "kind": "base",
                "id": base_layer_id,
                "storage": "directory",
                "replace_paths": [],
            },
            {
                "ordinal": 1,
                "kind": "mod",
                "id": mod_layer_id,
                "descriptor_sha256": descriptor_sha,
                "storage": "directory",
                "replace_paths": ["gfx/interface/replay_fixture"],
            }
        ],
        "files": [
            {"path": base_path, "winner": 0, "source_sha256": base_sha},
            {"path": mod_path, "winner": 1, "source_sha256": mod_sha},
        ]
    }))?);
    let product_modset_sha256 = sha256(
        format!("{BUILD_SHA}:{modset_id}:{effective_vfs_manifest_sha256}").as_bytes(),
    );

    let mut manifest = ContentManifest {
        schema: SCHEMA.into(),
        product_modset_sha256,
        source_game: SourceGameIdentity {
            version: "fixture-1.19.2".into(),
            build: "fixture-build-1".into(),
            executable_sha256: BUILD_SHA.into(),
            build_profile: "replay_fixture_gen_v1".into(),
        },
        source_profile: SourceProfileIdentity {
            profile_id: format!("fixture-profile-{seed}"),
            playset_id: modset_id.into(),
            selection_sha256: "0".repeat(64),
        },
        effective_vfs_manifest_sha256: effective_vfs_manifest_sha256.clone(),
        ordered_layers: vec![
            RecordedContentLayer {
                ordinal: 0,
                layer_kind: ContentLayerKind::Base,
                layer_id: base_layer_id.clone(),
                display_name: "Fixture Base Content".into(),
                descriptor_sha256: None,
                source_container_sha256: None,
                storage_kind: LayerStorageKind::Directory,
                replace_paths: vec![],
                required_for_playback: true,
            },
            RecordedContentLayer {
                ordinal: 1,
                layer_kind: ContentLayerKind::Mod,
                layer_id: mod_layer_id.clone(),
                display_name: "Fixture Replay Mod".into(),
                descriptor_sha256: Some(descriptor_sha),
                source_container_sha256: None,
                storage_kind: LayerStorageKind::Directory,
                replace_paths: vec!["gfx/interface/replay_fixture".into()],
                required_for_playback: true,
            },
        ],
        dlc_selection: vec![],
        localisation_language: "l_english".into(),
        required_settings: BTreeMap::new(),
        recorder_local_settings: BTreeMap::new(),
        required_content: vec![
            fixture_required_content(
                "fixture-base-country-definition",
                RequiredContentKind::Definition,
                base_path,
                0,
                ContentLayerKind::Base,
                &base_layer_id,
                &effective_vfs_manifest_sha256,
                &base_sha,
                FIXTURE_BASE_DEFINITION.len(),
                "replay_fixture_gen.snapshot_semantics",
            )?,
            fixture_required_content(
                "fixture-mod-interface-texture",
                RequiredContentKind::Texture,
                mod_path,
                1,
                ContentLayerKind::Mod,
                &mod_layer_id,
                &effective_vfs_manifest_sha256,
                &mod_sha,
                FIXTURE_MOD_TEXTURE.len(),
                "replay_fixture_gen.mod_override",
            )?,
        ],
        manifest_digest_sha256: None,
    };
    manifest.source_profile.selection_sha256 = manifest.computed_selection_sha256()?;
    Ok(manifest.with_computed_digest()?)
}

#[allow(clippy::too_many_arguments)]
fn fixture_required_content(
    content_key: &str,
    kind: RequiredContentKind,
    path: &str,
    ordinal: u32,
    layer_kind: ContentLayerKind,
    layer_id: &str,
    vfs_sha: &str,
    payload_sha: &str,
    payload_bytes: usize,
    consumer: &str,
) -> Result<RequiredContent> {
    Ok(RequiredContent {
        content_key: content_key.into(),
        kind,
        content: UsedContentKey {
            effective_vfs_manifest_sha256: vfs_sha.into(),
            normalized_game_relative_path: normalize_game_relative_path(path)?,
            winning_layer_ordinal: ordinal,
            layer_kind,
            layer_id: layer_id.into(),
            source_sha256: payload_sha.into(),
            payload_sha256: payload_sha.into(),
            payload_bytes: payload_bytes as u64,
        },
        observation_count: 1,
        consumers: vec![consumer.into()],
    })
}

fn sha256(bytes: &[u8]) -> String {
    snapshot::sha256::Sha256::hex(bytes)
}

fn created_utc(seed: u64) -> String {
    format!("2026-07-09T00:{:02}:00Z", seed % 60)
}

fn fixture_time(offset_hours: u32) -> (String, i64) {
    let month_days = [31u32, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let year = 1936i64;
    let mut day_index = offset_hours / 24;
    let hour = offset_hours % 24;
    let mut month = 1u32;
    for days in month_days {
        if day_index < days {
            break;
        }
        day_index -= days;
        month += 1;
    }
    let day = day_index + 1;
    let before_month: u32 = month_days.iter().take((month - 1) as usize).sum();
    let hour_index = (year * 365 + i64::from(before_month + day - 1)) * 24 + i64::from(hour);
    (format!("{year}.{month}.{day}.{hour}"), hour_index)
}

fn hour_at(scenario: &Scenario, tick: usize) -> i64 {
    scenario.snapshots[tick]["hour_index"]
        .as_i64()
        .expect("fixture hour")
}

fn date_at_hour(hour: i64) -> String {
    format!("hour:{hour}")
}

fn rel_slash(path: &Path, root: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_content_manifest::{
        PlaybackContentInventory, PlaybackContentLayer, ResolvedPlaybackContent,
    };

    #[test]
    fn portable_identity_records_exact_base_and_mod_vfs_winners() {
        let identity = pack_identity(DEFAULT_SEED, MODSET).expect("fixture identity");
        let manifest = identity
            .content_manifest
            .expect("portable content manifest");
        manifest.validate().expect("valid finalized manifest");
        assert_eq!(
            manifest.manifest_digest_sha256.as_deref(),
            Some(manifest.computed_digest().unwrap().as_str())
        );
        assert_eq!(manifest.ordered_layers.len(), 2);
        assert_eq!(
            manifest.ordered_layers[0].layer_kind,
            ContentLayerKind::Base
        );
        let mod_layer = &manifest.ordered_layers[1];
        assert_eq!(mod_layer.layer_kind, ContentLayerKind::Mod);
        assert_eq!(mod_layer.replace_paths, ["gfx/interface/replay_fixture"]);

        let base = &manifest.required_content[0];
        assert_eq!(base.content.winning_layer_ordinal, 0);
        assert_eq!(base.content.payload_sha256, sha256(FIXTURE_BASE_DEFINITION));
        assert_eq!(
            base.content.payload_bytes,
            FIXTURE_BASE_DEFINITION.len() as u64
        );
        let mod_override = &manifest.required_content[1];
        assert_eq!(mod_override.content.winning_layer_ordinal, 1);
        assert_eq!(mod_override.content.layer_id, mod_layer.layer_id);
        assert_eq!(
            mod_override.content.payload_sha256,
            sha256(FIXTURE_MOD_TEXTURE)
        );
        assert_eq!(
            mod_override.content.payload_bytes,
            FIXTURE_MOD_TEXTURE.len() as u64
        );

        let playback = PlaybackContentInventory {
            source_game: manifest.source_game.clone(),
            source_profile: manifest.source_profile.clone(),
            layers: manifest
                .ordered_layers
                .iter()
                .map(|layer| PlaybackContentLayer {
                    recorded_ordinal: layer.ordinal,
                    layer_kind: layer.layer_kind,
                    layer_id: layer.layer_id.clone(),
                    descriptor_sha256: layer.descriptor_sha256.clone(),
                    source_container_sha256: layer.source_container_sha256.clone(),
                    storage_kind: layer.storage_kind,
                    replace_paths: layer.replace_paths.clone(),
                })
                .collect(),
            dlc_selection: manifest.dlc_selection.clone(),
            localisation_language: manifest.localisation_language.clone(),
            settings: manifest.required_settings.clone(),
            resolved_content: manifest
                .required_content
                .iter()
                .map(|required| ResolvedPlaybackContent {
                    normalized_game_relative_path: required
                        .content
                        .normalized_game_relative_path
                        .clone(),
                    winning_layer_ordinal: required.content.winning_layer_ordinal,
                    layer_kind: required.content.layer_kind,
                    layer_id: required.content.layer_id.clone(),
                    source_sha256: required.content.source_sha256.clone(),
                    payload_sha256: required.content.payload_sha256.clone(),
                    payload_bytes: required.content.payload_bytes,
                })
                .collect(),
        };
        let report = manifest.validate_playback(&playback);
        assert!(report.compatible, "{:#?}", report.issues);
        assert_eq!(report.verified_required_content, 2);
    }

    #[test]
    fn generated_fixture_set_is_byte_identical_after_relocation() {
        let root = std::env::temp_dir().join(format!(
            "battle-plan-fixture-relocation-{}",
            std::process::id()
        ));
        let first = root.join("first/location");
        let second = root.join("relocated/recordings");
        if root.exists() {
            portable_test_fs_retry::remove_dir_all(&root).expect("remove stale fixture test root");
        }

        let result = (|| -> Result<()> {
            generate(&first, DEFAULT_SEED)?;
            generate(&second, DEFAULT_SEED)?;
            let generated = read_tree(&first)?;
            assert_fixture_trees_equal("relocated generation", &generated, &read_tree(&second)?);
            assert_fixture_trees_equal(
                "committed replay_interface/fixtures corpus",
                &generated,
                &read_tree(&default_out_dir())?,
            );
            Ok(())
        })();
        let cleanup = fs::remove_dir_all(&root);
        result.expect("generate relocated fixtures");
        cleanup.expect("clean fixture relocation test root");
    }

    fn assert_fixture_trees_equal(
        label: &str,
        expected: &BTreeMap<String, Vec<u8>>,
        actual: &BTreeMap<String, Vec<u8>>,
    ) {
        const EXTERNAL_FIXTURES: &[&str] = &[
            "positive/fx_naval_invasion_paradrop_orders.battleplan",
            "xmap17_faction_joint_certification.json",
            "xmap19_weather_visual_lifecycle_joint_certification.json",
            "xui09_research_joint_certification.json",
        ];
        let mut differences = Vec::new();
        for path in expected.keys().chain(actual.keys()) {
            if label == "committed replay_interface/fixtures corpus"
                && EXTERNAL_FIXTURES.contains(&path.as_str())
            {
                continue;
            }
            if differences.iter().any(|difference: &String| {
                difference.starts_with(&format!("{path}:"))
            }) {
                continue;
            }
            match (expected.get(path), actual.get(path)) {
                (Some(expected), Some(actual)) if expected != actual => differences.push(format!(
                    "{path}: bytes differ (generated {} bytes sha256 {}, actual {} bytes sha256 {})",
                    expected.len(),
                    sha256(expected),
                    actual.len(),
                    sha256(actual)
                )),
                (Some(_), None) => differences.push(format!("{path}: missing")),
                (None, Some(_)) => differences.push(format!("{path}: unexpected")),
                _ => {}
            }
        }
        differences.sort();
        differences.dedup();
        assert!(
            differences.is_empty(),
            "{label} is stale or non-deterministic:\n{}\nregenerate with replay_fixture_gen --seed {DEFAULT_SEED}",
            differences.join("\n")
        );
    }

    fn read_tree(root: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
        fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) -> Result<()> {
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    visit(root, &path, out)?;
                } else {
                    out.insert(rel_slash(&path, root), fs::read(path)?);
                }
            }
            Ok(())
        }

        let mut out = BTreeMap::new();
        visit(root, root, &mut out)?;
        Ok(out)
    }
}

#[derive(Clone)]
struct StableRng {
    state: u64,
}

impl StableRng {
    fn new(seed: u64, salt: &str) -> StableRng {
        StableRng {
            state: seed ^ fnv1a64(salt.as_bytes()),
        }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    hash
}
