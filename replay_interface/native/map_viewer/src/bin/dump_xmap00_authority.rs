
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use map_viewer::{
    map_authority_projection_from_world_frame, xmap05_admission_verdict,
    MapAuthorityRequirementDisposition, MAP_AUTHORITY_REQUIRED_INPUTS,
};
use replay_core::{open_recording, open_recording_game, Leaf, ViewStatus, WorldFrame, WorldTimeline};

/// Which packet owns one authority input, asked of the product's own owner
/// table so a reader of this artifact never has to keep a second copy, and so a
/// packet nobody has written down anywhere still answers.
fn owner_of(input: &str) -> String {
    map_viewer::map_authority_owner_packet(input)
        .unwrap_or(map_viewer::MAP_AUTHORITY_OWNER_UNCLASSIFIED)
        .to_string()
}

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
        // The XMAP05 family's own inputs, at the grain the family refuses on.
        // Kept separate from the statuses above because a roll-up of
        // `unavailable=N` never says WHICH country sank the roster, and that one
        // country is the whole question for this family.
        Observation {
            label: "d18_country_roster",
            // The COMPLETENESS status, which is what the XMAP00 ledger row
            // `country.d18_complete_roster` claims. The read's own status and
            // the countries it could not name are both printed beside it, so a
            // reader can tell the two apart rather than inferring one from the
            // other.
            status: Some(frame.d18_country_roster().complete_status()),
            completeness: Some({
                let roster = frame.d18_country_roster();
                let (resolved, excluded) = match roster.partial() {
                    Ok(partial) => (partial.resolved.len(), partial.excluded.join(",")),
                    Err(_) => (0, String::new()),
                };
                format!(
                    "read={:?} resolved={} unresolved=[{}] diagnostic={}",
                    roster.read_status(),
                    resolved,
                    excluded,
                    roster.diagnostic().unwrap_or("none")
                )
            }),
        },
        Observation {
            label: "country_identities_status",
            status: Some(frame.country_identities_status()),
            completeness: Some({
                let mut measured_override = 0usize;
                let mut measured_none = 0usize;
                let mut absent = 0usize;
                let mut unobtained = 0usize;
                for (_, identity) in frame.country_identities() {
                    match &identity.cosmetic_tag {
                        Leaf::Measured(Some(_)) => measured_override += 1,
                        Leaf::Measured(None) => measured_none += 1,
                        Leaf::Absent => absent += 1,
                        Leaf::Unobtained => unobtained += 1,
                    }
                }
                format!(
                    "cosmetic override={measured_override} no-override={measured_none} \
                     absent={absent} unobtained={unobtained}"
                )
            }),
        },
        Observation {
            label: "mapc1_envelope",
            status: None,
            completeness: Some({
                match frame
                    .domain("map")
                    .and_then(serde_json::Value::as_object)
                    .and_then(|map| map.get("_r2_xmap05_control_authority"))
                    .and_then(serde_json::Value::as_object)
                {
                    None => "absent".to_string(),
                    Some(envelope) => format!(
                        "present schema={:?} schema_version={:?} country_count={:?} \
                         presentation_identity_excluded={:?}",
                        envelope.get("schema").and_then(serde_json::Value::as_str),
                        envelope
                            .get("schema_version")
                            .and_then(serde_json::Value::as_u64),
                        envelope
                            .get("country_count")
                            .and_then(serde_json::Value::as_u64),
                        envelope
                            .get("presentation_identity_excluded")
                            .and_then(serde_json::Value::as_array)
                            .map(Vec::len),
                    ),
                }
            }),
        },
        // Sizing input for the division movement-route carrier: how many rows
        // would carry a route at all, and how many already carry the one hop
        // `DivisionUnitRow` has today.
        Observation {
            label: "division movement census",
            status: None,
            completeness: Some({
                let rows = frame.division_units();
                let moving = rows
                    .iter()
                    .filter(|row| matches!(row.moving, Leaf::Measured(true)))
                    .count();
                let with_target = rows
                    .iter()
                    .filter(|row| matches!(row.move_target_province, Leaf::Measured(id) if id != 0))
                    .count();
                format!(
                    "rows={} moving={moving} with_move_target={with_target}",
                    rows.len()
                )
            }),
        },
        // When the D18 roster refuses, WHICH country keys are not TAGs, and do
        // they hold any territory? The answer decides whether an unresolved
        // country can be excluded at country grain the way an unmeasured cosmetic
        // identity now is, or whether excluding it would delete land from the map.
        Observation {
            label: "non-TAG country keys",
            status: None,
            completeness: Some({
                let non_tag: Vec<String> = frame
                    .domain("countries")
                    .and_then(serde_json::Value::as_object)
                    .map(|countries| {
                        countries
                            .keys()
                            // PURELY numeric keys only. `D01`..`D75` are legal dynamic-country
                            // TAGs (the charset admits digits), and counting those as
                            // unresolved would drown the one key that actually is.
                            .filter(|key| key.parse::<u32>().is_ok())
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();
                if non_tag.is_empty() {
                    "none".to_string()
                } else {
                    let owned = frame
                        .owner_by_state()
                        .values()
                        .chain(frame.control_by_state().values())
                        .filter_map(|leaf| match leaf {
                            Leaf::Measured(tag) => Some(tag.tag()),
                            _ => None,
                        })
                        .filter(|tag| non_tag.iter().any(|key| key == tag))
                        .count();
                    let provinces = frame
                        .control_by_province()
                        .values()
                        .filter_map(|leaf| match leaf {
                            Leaf::Measured(tag) => Some(tag.tag()),
                            _ => None,
                        })
                        .filter(|tag| non_tag.iter().any(|key| key == tag))
                        .count();
                    format!(
                        "{:?} state_rows_naming_them={owned} province_rows_naming_them={provinces}",
                        non_tag
                    )
                }
            }),
        },
        // The frame-pure half of XMAP05's admission, for checking a refusing hour.
        Observation {
            label: "xmap05 admission verdict",
            status: None,
            completeness: Some(match xmap05_admission_verdict(frame) {
                Ok(verdict) => format!(
                    "ADMITS certified={} excluded={:?} states={} in_state_provinces={}                      povs_match_roster={:?}",
                    verdict.certified_countries,
                    verdict.excluded_countries,
                    verdict.states,
                    verdict.in_state_provinces,
                    verdict.visibility_povs_match_roster
                ),
                Err(reason) => format!("REFUSES {reason}"),
            }),
        },
        Observation {
            label: "visibility POV census",
            status: Some(frame.visibility_status()),
            completeness: Some(format!(
                "povs={} roster={}",
                frame.visibility_povs().count(),
                frame
                    .d18_country_roster()
                    .partial()
                    .map(|partial| partial.resolved.len())
                    .unwrap_or(0)
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
    // The owner of each input, printed beside it. A reader who wants "what does
    // packet X own" must otherwise keep its own copy of the owner table, and a
    // copy that drifts reports a census of the wrong set.
    let mut owners: BTreeMap<&str, String> = BTreeMap::new();

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
            owners.entry(key).or_insert_with(|| owner_of(key));
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
    // The exact (input, owner) set this roll-up is a census OF, as one digest,
    // computed by the product rather than restated here.
    let census_identity = map_viewer::map_authority_set_identity();
    println!(
        "== XMAP00 disposition roll-up ({} inputs, identity {census_identity}, hours per disposition) ==",
        MAP_AUTHORITY_REQUIRED_INPUTS.len()
    );
    for input in MAP_AUTHORITY_REQUIRED_INPUTS {
        let counts = roll.get(input).cloned().unwrap_or_default();
        let rendered: Vec<String> = ["admitted", "unavailable", "absent"]
            .iter()
            .filter_map(|word| counts.get(word).map(|count| format!("{word}={count}")))
            .collect();
        let owner = owners
            .get(input)
            .map(String::as_str)
            .unwrap_or("owner-unclassified");
        println!("  {input} [{owner}]: {}", rendered.join(" "));
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
