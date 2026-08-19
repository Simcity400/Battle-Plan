//! Manual verification CLI for the XMAP00 map-authority seam
//! (`map_viewer::map_authority_projection_from_world_frame`).
//!
//! For every canonical hour of a recording it runs the real projection and
//! reports each required input's derived disposition, so an admission claim can
//! be checked against what a recording actually measured rather than against a
//! hand-built fixture. It also reports, per hour, the raw `WorldFrame` view
//! statuses and the completeness signals that a stronger-than-raw admission
//! status has to consult, because a `Present` raw status is not by itself proof
//! that a census was completely measured (see `map_authority.rs`,
//! `land_terminal_detail_status`).
//!
//! This is a read-only observer: it never mutates the recording and never
//! writes anything except the report it prints.
//!
//! Usage:
//!   dump_xmap00_authority <recording-path> [--game ID] [--hours N] [--json]
//!
//! `--hours N` samples the first N canonical hours (default: all).
//! `--json` prints one JSON document instead of the text report. It carries the
//! per-input disposition roll-up and the dropped-row warning counts only — the
//! per-hour raw statuses and completeness signals are in the text report, which
//! is the one to use when investigating a single hour.
//!
//! The recording path is always supplied by the operator. Nothing here names a
//! path inside `extractor/`: this workspace does not reach across that
//! boundary, and the recording under test is passed in from outside.

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

/// One raw observation the admission derivation may consult. `status` is the
/// view status as the decoder reports it; `completeness` names, in words, any
/// signal that says the census behind that status may be short of rows.
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

/// Coverage warnings whose prefix marks a dropped row in a census this report
/// covers. A guard in `map_authority` can only be written against a prefix that
/// actually appears here.
///
/// **An empty count is not proof of a complete census.** Several decoder paths
/// drop a row with no warning at all — `build_raids` silently skips a malformed
/// `raids.countries` entry and still returns `Present`, and `build_occupation`
/// does the same for a non-numeric state key. Those losses are invisible to
/// this report by construction, which is exactly why the rows they belong to
/// are unadmitted (see `extractor/documents/technical_evidence.json` anchor
/// `technical-r2-validation-bounds-audit::Census scope and observed failure class`). Read a zero here
/// as "no *recorded* drop", never as "nothing was lost".
fn dropped_row_warnings(frame: &WorldFrame) -> BTreeMap<String, usize> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for warning in frame.coverage_warnings() {
        // The decoder is not uniform about the verb: `row skipped`,
        // `cell skipped`, `element skipped`, `row ignored` and bare `skipped`
        // all occur. Matching only one of them is how a lossy census gets
        // waved through, so match the whole vocabulary.
        if !(warning.contains("skipped") || warning.contains("ignored")) {
            continue;
        }
        // Group by the text before the first '[' or ':' so a per-index warning
        // collapses onto the census it belongs to.
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
            // The admitted XMAP00 row with a census behind it, so its refusals
            // are the ones an operator will actually have to explain. Report every input its
            // guard consults: the D24 witness, the raw member count it is
            // compared against, the decoded length, whether every leaf was
            // measured, and whether the decoded ids stay inside the declared
            // dense range.
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

    // input_id -> disposition -> hour count
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
