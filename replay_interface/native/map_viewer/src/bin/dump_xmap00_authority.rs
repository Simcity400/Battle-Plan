
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use map_viewer::{
    map_authority_projection_from_world_frame, MapAuthorityRequirementDisposition,
    MAP_AUTHORITY_REQUIRED_INPUTS,
};
use replay_core::{open_recording, open_recording_game, Leaf, ViewStatus, WorldFrame, WorldTimeline};

const USAGE: &str =
    "usage: dump_xmap00_authority <recording-path> [--game ID] [--hours N] [--json]";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("dump_xmap00_authority: {message}");
            ExitCode::FAILURE
        }
    }
}

struct Args {
    path: PathBuf,
    game: Option<String>,
    hours: Option<usize>,
    json: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut path: Option<PathBuf> = None;
    let mut game: Option<String> = None;
    let mut hours: Option<usize> = None;
    let mut json = false;
    let mut argv = std::env::args().skip(1);
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "--game" => {
                game = Some(
                    argv.next()
                        .ok_or_else(|| format!("--game needs a value\n{USAGE}"))?,
                );
            }
            "--hours" => {
                let raw = argv
                    .next()
                    .ok_or_else(|| format!("--hours needs a value\n{USAGE}"))?;
                hours = Some(
                    raw.parse()
                        .map_err(|error| format!("--hours {raw:?} is not an integer: {error}"))?,
                );
            }
            "--json" => json = true,
            "--help" | "-h" => return Err(USAGE.to_string()),
            other if other.starts_with("--") => {
                return Err(format!("unknown flag {other:?}\n{USAGE}"))
            }
            other => {
                if path.is_some() {
                    return Err(format!("unexpected extra argument {other:?}\n{USAGE}"));
                }
                path = Some(PathBuf::from(other));
            }
        }
    }
    Ok(Args {
        path: path.ok_or_else(|| format!("missing <recording-path>\n{USAGE}"))?,
        game,
        hours,
        json,
    })
}

struct Observation {
    label: &'static str,
    status: Option<ViewStatus>,
    completeness: Option<String>,
}

fn status_word(status: Option<ViewStatus>) -> &'static str {
    match status {
        Some(ViewStatus::Present) => "Present",
        Some(ViewStatus::DomainUnavailable) => "DomainUnavailable",
        Some(ViewStatus::DomainAbsent) => "DomainAbsent",
        None => "no-accessor",
    }
}

fn leaf_word<T>(leaf: &Leaf<T>) -> &'static str {
    match leaf {
        Leaf::Measured(_) => "measured",
        Leaf::Unobtained => "unobtained",
        Leaf::Absent => "absent",
    }
}

fn dropped_row_warnings(frame: &WorldFrame) -> BTreeMap<String, usize> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for warning in frame.coverage_warnings() {
        if !(warning.contains("skipped") || warning.contains("ignored")) {
            continue;
        }
        let head = warning
            .split_once('[')
            .map(|(head, _)| head)
            .unwrap_or_else(|| warning.split_once(':').map(|(head, _)| head).unwrap_or(warning));
        *counts.entry(head.trim().to_string()).or_default() += 1;
    }
    counts
}

fn observations(frame: &WorldFrame) -> Vec<Observation> {
    let raids = frame.raids();
    let scripted = frame.scripted_gui();
    let battleplans = frame.army_battleplans();
    vec![
        Observation {
            label: "buildings_status",
            status: Some(frame.buildings_status()),
            completeness: Some(format!("buildings_by_state rows={}", frame.buildings_by_state().len())),
        },
        Observation {
            label: "d48_buildings_status",
            status: Some(frame.d48_buildings_status()),
            completeness: None,
        },
        Observation {
            label: "occupation_status",
            status: Some(frame.occupation_status()),
            completeness: Some(format!("occupation_by_state rows={}", frame.occupation_by_state().len())),
        },
        Observation {
            label: "raids_status",
            status: Some(frame.raids_status()),
            completeness: Some(format!(
                "instances_complete={} instances={} country_statuses={} priorities={}",
                raids.instances_complete,
                leaf_word(&raids.instances),
                leaf_word(&raids.country_statuses),
                raids.priorities.len()
            )),
        },
        Observation {
            label: "scripted_gui_status",
            status: Some(frame.scripted_gui_status()),
            completeness: Some(format!(
                "by_country countries={} unmeasured={}",
                scripted.by_country.len(),
                scripted
                    .by_country
                    .values()
                    .filter(|leaf| !matches!(leaf, Leaf::Measured(_)))
                    .count()
            )),
        },
        Observation {
            label: "army_battleplans.status",
            status: Some(battleplans.status),
            completeness: Some(format!(
                "orders={} baseline_seen={} omitted_non_exact={:?} omitted_path_geometry={:?} omitted_identity_or_active={:?}",
                battleplans.orders.len(),
                battleplans.baseline_seen,
                battleplans.omitted_non_exact_rows_this_hour,
                battleplans.path_geometry_omitted_rows_this_hour,
                battleplans.identity_or_active_omitted_rows_this_hour
            )),
        },
        Observation {
            label: "division_units_status",
            status: Some(frame.division_units_status()),
            completeness: Some(format!("division_units rows={}", frame.division_units().len())),
        },
        Observation {
            label: "fleets_status",
            status: Some(frame.fleets_status()),
            completeness: Some(format!("fleets rows={}", frame.fleets().len())),
        },
        Observation {
            label: "wings_status",
            status: Some(frame.wings_status()),
            completeness: Some(format!("wings rows={}", frame.wings().len())),
        },
        Observation {
            label: "weather_status",
            status: Some(frame.weather_status()),
            completeness: Some(format!("weather_by_region rows={}", frame.weather_by_region().len())),
        },
        Observation {
            label: "weather_province_snow_status",
            status: Some(frame.weather_province_snow_status()),
            completeness: Some({
                let view = frame.weather_province_snow();
                let weather = frame
                    .domain("map")
                    .and_then(serde_json::Value::as_object)
                    .and_then(|map| map.get("_r2_weather_live"))
                    .and_then(serde_json::Value::as_object);
                let declared = weather
                    .and_then(|weather| weather.get("observed_province_rows"))
                    .and_then(serde_json::Value::as_u64);
                let raw = weather
                    .and_then(|weather| weather.get("provinces"))
                    .and_then(serde_json::Value::as_object)
                    .map(serde_json::Map::len);
                format!(
                    "observed_province_rows={} raw={} decoded={} all_measured={} in_declared_range={}",
                    declared
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "absent".into()),
                    raw.map(|value| value.to_string())
                        .unwrap_or_else(|| "absent".into()),
                    view.by_province.len(),
                    view.by_province
                        .values()
                        .all(|snow| matches!(snow, Leaf::Measured(_))),
                    declared.is_some_and(|declared| view
                        .by_province
                        .keys()
                        .all(|province_id| (*province_id as u64) < declared)),
                )
            }),
        },
        Observation {
            label: "hour_index/date (world.calendar_time)",
            status: None,
            completeness: Some(format!(
                "hour_index={} date={}",
                frame.hour_index(),
                frame.date().unwrap_or("null(unobtained)")
            )),
        },
    ]
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let source = match &args.game {
        Some(game) => open_recording_game(&args.path, game),
        None => open_recording(&args.path),
    }
    .map_err(|error| error.to_string())?;
    let timeline = WorldTimeline::open(source).map_err(|error| error.to_string())?;
    let manifest = timeline
        .content_manifest()
        .ok_or_else(|| "recording carries no content manifest".to_string())?
        .clone();

    let all_hours = timeline.hours().to_vec();
    let hours: Vec<i64> = match args.hours {
        Some(n) => all_hours.iter().copied().take(n).collect(),
        None => all_hours.clone(),
    };

    let mut roll: BTreeMap<&str, BTreeMap<&'static str, usize>> = BTreeMap::new();
    let mut per_hour_lines: Vec<String> = Vec::new();
    let mut warning_roll: BTreeMap<String, usize> = BTreeMap::new();

    for hour in &hours {
        let frame = timeline.frame(*hour).map_err(|error| error.to_string())?;
        let projection = map_authority_projection_from_world_frame(&frame, &manifest)?;
        for row in &projection.requirements {
            let word = match row.disposition {
                MapAuthorityRequirementDisposition::Admitted => "admitted",
                MapAuthorityRequirementDisposition::Unavailable => "unavailable",
                MapAuthorityRequirementDisposition::Absent => "absent",
            };
            let key = MAP_AUTHORITY_REQUIRED_INPUTS
                .iter()
                .copied()
                .find(|known| *known == row.input_id)
                .ok_or_else(|| format!("projection carried unknown input {}", row.input_id))?;
            *roll.entry(key).or_default().entry(word).or_default() += 1;
        }
        for (head, count) in dropped_row_warnings(&frame) {
            *warning_roll.entry(head).or_default() += count;
        }
        if !args.json {
            let mut cells: Vec<String> = Vec::new();
            for observation in observations(&frame) {
                let completeness = observation
                    .completeness
                    .map(|text| format!(" ({text})"))
                    .unwrap_or_default();
                cells.push(format!(
                    "{}={}{completeness}",
                    observation.label,
                    status_word(observation.status)
                ));
            }
            per_hour_lines.push(format!("hour {hour}: {}", cells.join("; ")));
        }
    }

    if args.json {
        let mut roll_json = serde_json::Map::new();
        for (input, counts) in &roll {
            let mut inner = serde_json::Map::new();
            for (word, count) in counts {
                inner.insert((*word).to_string(), serde_json::json!(count));
            }
            roll_json.insert((*input).to_string(), serde_json::Value::Object(inner));
        }
        let mut warnings_json = serde_json::Map::new();
        for (head, count) in &warning_roll {
            warnings_json.insert(head.clone(), serde_json::json!(count));
        }
        let document = serde_json::json!({
            "recording": args.path.to_string_lossy(),
            "game_id": timeline.game_id(),
            "hours_total": all_hours.len(),
            "hours_examined": hours.len(),
            "first_hour": timeline.first_hour(),
            "last_hour": timeline.last_hour(),
            "disposition_by_input": serde_json::Value::Object(roll_json),
            "dropped_row_warning_counts": serde_json::Value::Object(warnings_json),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&document).map_err(|error| error.to_string())?
        );
        return Ok(());
    }

    println!("recording: {}", args.path.display());
    println!("game_id: {}", timeline.game_id());
    println!(
        "hours: {} total, hour_index {}..{}, examined {}",
        all_hours.len(),
        timeline.first_hour(),
        timeline.last_hour(),
        hours.len()
    );
    println!();
    println!("== XMAP00 disposition roll-up (hours per disposition) ==");
    for input in MAP_AUTHORITY_REQUIRED_INPUTS {
        let counts = roll.get(input).cloned().unwrap_or_default();
        let rendered: Vec<String> = ["admitted", "unavailable", "absent"]
            .iter()
            .filter_map(|word| counts.get(word).map(|count| format!("{word}={count}")))
            .collect();
        println!("  {input}: {}", rendered.join(" "));
    }
    println!();
    println!("== dropped-row coverage warnings (census -> total occurrences) ==");
    if warning_roll.is_empty() {
        println!("  none");
    } else {
        for (head, count) in &warning_roll {
            println!("  {head}: {count}");
        }
    }
    println!();
    println!("== raw view statuses per hour ==");
    for line in &per_hour_lines {
        println!("{line}");
    }
    Ok(())
}
