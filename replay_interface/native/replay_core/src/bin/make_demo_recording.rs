//! Build a SYNTHETIC demo `.battleplan` recording from a generated map
//! dev fixture: hour 0 is the bundle's real 1936 ownership, later hours flip

use snapshot::{pack, PackEntry, PackIdentity};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const USAGE: &str = "usage: make_demo_recording --bundle <map_bundle_dir> --out <file.battleplan> [--hours N] [--start-hour N]";
const GAME_ID: &str = "synthetic_demo";
const MONTH_DAYS: [i64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

fn main() {
    if let Err(err) = run() {
        eprintln!("make_demo_recording: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut bundle_dir: Option<PathBuf> = None;
    let mut out_path: Option<PathBuf> = None;
    let mut hours: i64 = 72;
    let mut start_hour = date_to_hour_index(1936, 1, 1, 12);
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--bundle" => {
                i += 1;
                bundle_dir = Some(PathBuf::from(args.get(i).ok_or_else(|| USAGE.to_string())?));
            }
            "--out" => {
                i += 1;
                out_path = Some(PathBuf::from(args.get(i).ok_or_else(|| USAGE.to_string())?));
            }
            "--hours" => {
                i += 1;
                hours = args
                    .get(i)
                    .ok_or_else(|| USAGE.to_string())?
                    .parse()
                    .map_err(|e| format!("--hours: {e}"))?;
            }
            "--start-hour" => {
                i += 1;
                start_hour = args
                    .get(i)
                    .ok_or_else(|| USAGE.to_string())?
                    .parse()
                    .map_err(|e| format!("--start-hour: {e}"))?;
            }
            other => return Err(format!("unknown argument {other:?}\n{USAGE}")),
        }
        i += 1;
    }
    let bundle_dir = bundle_dir.ok_or_else(|| USAGE.to_string())?;
    let out_path = out_path.ok_or_else(|| USAGE.to_string())?;
    if hours < 2 {
        return Err("--hours must be at least 2".to_string());
    }

    let owners_1936 = load_state_owners(&bundle_dir)?;
    if owners_1936.is_empty() {
        return Err("state_owner_1936.json produced no owned states".to_string());
    }
    let state_provinces = load_state_provinces(&bundle_dir)?;

    let majority_tag = majority_owner(&owners_1936);
    let mut flip_order: Vec<u32> = owners_1936
        .iter()
        .filter(|(_, tag)| **tag != majority_tag)
        .map(|(sid, _)| *sid)
        .collect();
    flip_order.sort_unstable();
    flip_order.truncate(flip_order.len() / 4);

    let mut entries: Vec<PackEntry> = Vec::new();
    let mut index_rows = Vec::new();

    let war = WarScript::new(
        &owners_1936,
        &flip_order,
        &state_provinces,
        &majority_tag,
        hours,
    );

    for step in 0..hours {
        let hour_index = start_hour + step;
        let date = hour_index_to_date(hour_index);
        let controllers = owners_at_step(&owners_1936, &flip_order, &majority_tag, step, hours);
        let snapshot = build_snapshot(&date, hour_index, &owners_1936, &controllers, step, &war);
        let file = format!("{hour_index:012}.snapshot.json");
        index_rows.push(serde_json::json!({
            "hour_index": hour_index,
            "date": date,
            "file": file,
        }));
        entries.push(PackEntry {
            path: format!("payload/{GAME_ID}/{file}"),
            bytes: serde_json::to_vec(&snapshot).map_err(|e| e.to_string())?,
            role: "snapshot",
            hour_index: Some(hour_index),
        });
    }

    let index = serde_json::json!({
        "game_id": GAME_ID,
        "schema_version": "1",
        "snapshots": index_rows,
    });
    entries.insert(
        0,
        PackEntry {
            path: format!("payload/{GAME_ID}/index.json"),
            bytes: serde_json::to_vec_pretty(&index).map_err(|e| e.to_string())?,
            role: "index",
            hour_index: None,
        },
    );

    let identity = PackIdentity {
        schema_version: Some("1".to_string()),
        ..Default::default()
    };
    let packed =
        pack(GAME_ID, &entries, &identity, true).map_err(|e| format!("pack failed: {e}"))?;
    std::fs::write(&out_path, packed).map_err(|e| format!("write {}: {e}", out_path.display()))?;
    println!(
        "wrote synthetic demo recording: {} ({} hours, {} owned states, flip set {}, {} scripted fronts)",
        out_path.display(),
        hours,
        owners_1936.len(),
        flip_order.len(),
        war.fronts.len()
    );
    Ok(())
}

fn load_state_owners(bundle_dir: &Path) -> Result<BTreeMap<u32, String>, String> {
    let path = bundle_dir.join("state_owner_1936.json");
    let text =
        std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))?;
    let object = value
        .as_object()
        .ok_or_else(|| format!("{}: expected an object", path.display()))?;
    let mut owners = BTreeMap::new();
    for (sid, tag) in object {
        let sid: u32 = sid.parse().map_err(|e| format!("state id {sid:?}: {e}"))?;
        let tag = match tag {
            serde_json::Value::String(tag) => tag.clone(),
            serde_json::Value::Object(map) => match map.get("owner").and_then(|v| v.as_str()) {
                Some(tag) => tag.to_string(),
                None => continue,
            },
            _ => return Err(format!("state {sid}: unsupported owner shape")),
        };
        owners.insert(sid, tag);
    }
    Ok(owners)
}

fn majority_owner(owners: &BTreeMap<u32, String>) -> String {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for tag in owners.values() {
        *counts.entry(tag.as_str()).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|(tag, _)| tag.to_string())
        .unwrap_or_else(|| "GER".to_string())
}

fn owners_at_step(
    base: &BTreeMap<u32, String>,
    flip_order: &[u32],
    majority_tag: &str,
    step: i64,
    total_hours: i64,
) -> BTreeMap<u32, String> {
    let mut owners = base.clone();
    let half = total_hours / 2;
    let flipped = if step <= half {
        step
    } else {
        (total_hours - step).max(0)
    } as usize;
    for sid in flip_order.iter().take(flipped.min(flip_order.len())) {
        owners.insert(*sid, majority_tag.to_string());
    }
    owners
}

fn load_state_provinces(bundle_dir: &Path) -> Result<BTreeMap<u32, Vec<u32>>, String> {
    let path = bundle_dir.join("province_to_state.json");
    let text =
        std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))?;
    let object = value
        .as_object()
        .ok_or_else(|| format!("{}: expected an object", path.display()))?;
    let mut by_state: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (province, state) in object {
        let province: u32 = match province.parse() {
            Ok(p) => p,
            Err(_) => continue,
        };
        let state = match state.as_u64() {
            Some(s) => s as u32,
            None => continue,
        };
        by_state.entry(state).or_default().push(province);
    }
    for provinces in by_state.values_mut() {
        provinces.sort_unstable();
    }
    Ok(by_state)
}

struct WarScript {
    fronts: Vec<(u32, u32, String)>,
    attacker_tag: String,
    home_province: u32,
    total_hours: i64,
}

const WAR_FRONT_COUNT: usize = 8;

impl WarScript {
    fn new(
        owners_1936: &BTreeMap<u32, String>,
        flip_order: &[u32],
        state_provinces: &BTreeMap<u32, Vec<u32>>,
        majority_tag: &str,
        total_hours: i64,
    ) -> WarScript {
        let mut fronts = Vec::new();
        for state_id in flip_order.iter().take(WAR_FRONT_COUNT) {
            let (Some(provinces), Some(owner)) =
                (state_provinces.get(state_id), owners_1936.get(state_id))
            else {
                continue;
            };
            if let Some(&front_province) = provinces.first() {
                fronts.push((*state_id, front_province, owner.clone()));
            }
        }
        let home_province = owners_1936
            .iter()
            .filter(|(_, tag)| tag.as_str() == majority_tag)
            .find_map(|(state_id, _)| state_provinces.get(state_id)?.first().copied())
            .unwrap_or(0);
        WarScript {
            fronts,
            attacker_tag: majority_tag.to_string(),
            home_province,
            total_hours,
        }
    }

    fn at_step(&self, step: i64) -> (Vec<serde_json::Value>, Vec<serde_json::Value>) {
        let half = self.total_hours / 2;
        let flipped = if step <= half {
            step
        } else {
            (self.total_hours - step).max(0)
        };
        let front = (flipped / 4) as usize;

        let mut divisions = Vec::new();
        let mut combats = Vec::new();
        for (i, (state_id, front_province, defender_tag)) in self.fronts.iter().enumerate() {
            let taken = front > i;
            let battle_here = front == i || front == i + 1;

            let peak_front = if step <= half {
                front
            } else {
                (half / 4) as usize
            };
            let destroyed = peak_front > i + 1;
            if !destroyed {
                let organisation = if i == 3 {
                    serde_json::Value::Null
                } else if battle_here {
                    serde_json::json!(0.35)
                } else {
                    serde_json::json!(0.80)
                };
                divisions.push(serde_json::json!({
                    "div_id": 1000 + i as i64,
                    "div_type": 25,
                    "owner": defender_tag,
                    "location_province": front_province,
                    "state_id": state_id,
                    "strength": 0.9,
                    "organisation": organisation,
                    "in_combat": battle_here,
                }));
            }

            let location = if taken {
                *front_province
            } else {
                self.home_province
            };
            divisions.push(serde_json::json!({
                "div_id": 2000 + i as i64,
                "div_type": 25,
                "owner": self.attacker_tag,
                "location_province": location,
                "state_id": if taken { Some(*state_id) } else { None },
                "strength": 1.0,
                "organisation": if battle_here { 0.45 } else { 0.75 },
                "in_combat": battle_here,
                "moving": !taken && front + 1 == i,
            }));

            if battle_here && !destroyed {
                combats.push(serde_json::json!({
                    "combat_id": 500 + i as i64,
                    "kind": "land",
                    "location_province": front_province,
                    "attackers": [2000 + i as i64],
                    "defenders": [1000 + i as i64],
                }));
            }
        }
        (divisions, combats)
    }
}

fn build_snapshot(
    date: &str,
    hour_index: i64,
    base_owners: &BTreeMap<u32, String>,
    controllers: &BTreeMap<u32, String>,
    step: i64,
    war: &WarScript,
) -> serde_json::Value {
    let mut states = serde_json::Map::new();
    let mut owner_by_state = serde_json::Map::new();
    for (sid, owner) in base_owners {
        let controller = controllers.get(sid).unwrap_or(owner);
        states.insert(
            sid.to_string(),
            serde_json::json!({ "owner": owner, "controller": controller }),
        );
        owner_by_state.insert(sid.to_string(), serde_json::json!(owner));
    }
    let stability = 0.70 - (step as f64) * 0.002;
    let mut politics = serde_json::Map::new();
    politics.insert("stability".to_string(), serde_json::json!(stability));
    if step % 3 == 0 {
        politics.insert("war_support".to_string(), serde_json::Value::Null);
    } else {
        politics.insert("war_support".to_string(), serde_json::json!(0.5));
    }
    let (divisions, combats) = war.at_step(step);
    serde_json::json!({
        "schema_version": "1",
        "game_id": GAME_ID,
        "date": date,
        "hour_index": hour_index,
        "save_version": null,
        "provenance": {
            // This demo is built from a map bundle, not captured from a game,
            // and the honest way to say so is a channel that says it -- not a
            // null. The fold always writes this channel as an object carrying
            // `domain_problems` (`world_fold.rs:9290-9294, :9744`), so
            // `rederive_collector_domain_problems` treats a present non-object
            // as a MALFORMED envelope and fails closed: every problem-backed
            // typed view goes down, owners and controllers among them. The
            // demo's whole point is that all 1081 owners bind.
            "collector": {
                "captured": false,
                "domains": [],
                "domain_problems": [],
            },
            "memory": null,
            "save": {
                "captured": true,
                "source_filename": "synthetic_demo_fixture",
                "save_date": date,
                "domains": ["state_ownership", "province_control", "divisions", "combats"],
            },
            "notes": [
                "SYNTHETIC DEMO RECORDING: real 1936 owners from the map bundle, scripted flips; not captured game state",
            ],
        },
        // The per-id country account, without which the viewer nulls every
        // country-composed member this generator emits and the demo arrives
        // carrying no country at all. This generator composes per tag_db_id by
        // construction: it writes exactly one country.
        "tag_db_id_index": { "country_composition_version": 2 },
        "countries": { "GER": { "_r2_tag_db_id": 1, "politics": politics } },
        "states": states,
        "provinces": {},
        "divisions": divisions,
        "combats": combats,
        "diplomacy_world": {},
        "map": { "owner_by_state": owner_by_state },
    })
}

fn date_to_hour_index(year: i64, month: usize, day: i64, hour: i64) -> i64 {
    let day_of_year: i64 = MONTH_DAYS[..month - 1].iter().sum::<i64>() + (day - 1);
    ((year * 365) + day_of_year) * 24 + hour
}

fn hour_index_to_date(hour_index: i64) -> String {
    let hour = hour_index % 24;
    let days_total = hour_index / 24;
    let year = days_total / 365;
    let mut day_of_year = days_total % 365;
    let mut month = 1;
    for (idx, len) in MONTH_DAYS.iter().enumerate() {
        if day_of_year < *len {
            month = idx + 1;
            break;
        }
        day_of_year -= len;
    }
    format!("{year}.{month}.{}.{hour}", day_of_year + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_round_trip() {
        let h = date_to_hour_index(1936, 1, 1, 12);
        assert_eq!(hour_index_to_date(h), "1936.1.1.12");
        let h2 = date_to_hour_index(1936, 3, 1, 0);
        assert_eq!(hour_index_to_date(h2), "1936.3.1.0");
        assert_eq!(h2 - date_to_hour_index(1936, 2, 28, 0), 24);
    }

    #[test]
    fn flip_script_is_symmetric() {
        let mut base = BTreeMap::new();
        base.insert(1u32, "GER".to_string());
        base.insert(2u32, "POL".to_string());
        let flips = vec![2u32];
        let at = |step| owners_at_step(&base, &flips, "GER", step, 10);
        assert_eq!(at(0)[&2], "POL");
        assert_eq!(at(5)[&2], "GER");
        assert_eq!(at(10)[&2], "POL");
    }

    #[test]
    fn war_script_moves_fights_and_destroys() {
        let mut owners = BTreeMap::new();
        owners.insert(1u32, "GER".to_string());
        owners.insert(2u32, "POL".to_string());
        owners.insert(3u32, "POL".to_string());
        let mut provinces = BTreeMap::new();
        provinces.insert(1u32, vec![100u32]);
        provinces.insert(2u32, vec![200u32]);
        provinces.insert(3u32, vec![300u32]);
        let war = WarScript::new(&owners, &[2, 3], &provinces, "GER", 48);
        assert_eq!(war.fronts.len(), 2);
        assert_eq!(war.home_province, 100);

        let (divs, combats) = war.at_step(0);
        assert_eq!(divs.len(), 4);
        let attacker0 = divs.iter().find(|d| d["div_id"] == 2000).unwrap();
        assert_eq!(attacker0["location_province"], 100);
        assert_eq!(combats.len(), 1);
        assert_eq!(combats[0]["defenders"][0], 1000);

        let (divs, combats) = war.at_step(8);
        let attacker0 = divs.iter().find(|d| d["div_id"] == 2000).unwrap();
        assert_eq!(attacker0["location_province"], 200);
        assert!(!divs.iter().any(|d| d["div_id"] == 1000));
        assert!(combats.iter().any(|c| c["combat_id"] == 501));

        let (divs, _) = war.at_step(47);
        assert!(!divs.iter().any(|d| d["div_id"] == 1000));

        assert_eq!(war.at_step(3), war.at_step(3));
    }
}
