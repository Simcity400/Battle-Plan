use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use map_viewer::fog_derivation_binding::{
    fog_contract_recorded_gaps, fog_contract_residuals, fog_own_reveal_oracles, FogInputOrigin,
    FOG_DERIVATION_CONTRACT,
};
use replay_core::{open_recording, open_recording_game, WorldTimeline};

const USAGE: &str =
    "usage: dump_fog_derivation_oracle <recording-path> [--game ID] [--hours N] [--json]";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("dump_fog_derivation_oracle: {message}");
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

fn origin_words(origin: FogInputOrigin) -> (&'static str, &'static str) {
    match origin {
        FogInputOrigin::Recorded(source) => ("recorded", source),
        FogInputOrigin::GameContent(source) => ("game_content", source),
        FogInputOrigin::Unrecorded(residual) => ("unrecorded", residual),
    }
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let source = match &args.game {
        Some(game) => open_recording_game(&args.path, game),
        None => open_recording(&args.path),
    }
    .map_err(|error| error.to_string())?;
    let timeline = WorldTimeline::open(source).map_err(|error| error.to_string())?;
    let all_hours = timeline.hours().to_vec();
    let hours: Vec<i64> = match args.hours {
        Some(n) => all_hours.iter().copied().take(n).collect(),
        None => all_hours.clone(),
    };

    let mut hours_agreeing = 0usize;
    let mut hours_refused: BTreeMap<String, usize> = BTreeMap::new();
    let mut recorded_gap_counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut pov_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut controlled_total = 0u64;
    let mut controlled_at_full_total = 0u64;
    let mut unexplained_total = 0u64;
    let mut unmeasured_controller_rows_total = 0u64;
    let mut counterexample_hours: Vec<serde_json::Value> = Vec::new();
    let mut province_slots: BTreeMap<usize, usize> = BTreeMap::new();

    for hour in &hours {
        let frame = timeline.frame(*hour).map_err(|error| error.to_string())?;
        for field in fog_contract_recorded_gaps(&frame, None) {
            *recorded_gap_counts.entry(field).or_default() += 1;
        }
        match fog_own_reveal_oracles(&frame) {
            Err(reason) => {
                *hours_refused.entry(reason).or_default() += 1;
            }
            Ok(rows) => {
                let mut hour_agrees = true;
                for row in rows {
                    *pov_counts.entry(row.pov.clone()).or_default() += 1;
                    *province_slots.entry(row.province_slots).or_default() += 1;
                    controlled_total += row.controlled as u64;
                    controlled_at_full_total += row.controlled_at_full as u64;
                    unexplained_total += row.unexplained_at_full as u64;
                    unmeasured_controller_rows_total += row.unmeasured_controller_rows as u64;
                    if !row.agrees() {
                        hour_agrees = false;
                        if counterexample_hours.len() < 16 {
                            counterexample_hours.push(serde_json::json!({
                                "hour_index": hour,
                                "pov": row.pov,
                                "controlled": row.controlled,
                                "controlled_at_full": row.controlled_at_full,
                                "controlled_below_full": row.controlled_below_full,
                                "controlled_out_of_range": row.controlled_out_of_range,
                                "unmeasured_controller_rows": row.unmeasured_controller_rows,
                            }));
                        }
                    }
                }
                if hour_agrees {
                    hours_agreeing += 1;
                }
            }
        }
    }

    let contract = FOG_DERIVATION_CONTRACT
        .iter()
        .map(|row| {
            let (kind, source) = origin_words(row.origin);
            serde_json::json!({
                "field": row.field,
                "origin": kind,
                "source": source,
                "basis": row.basis,
            })
        })
        .collect::<Vec<_>>();

    let document = serde_json::json!({
        "schema": "hoi4.replay.xmap05.fog-derivation-oracle.v1",
        "recording": args.path.to_string_lossy(),
        "game_id": timeline.game_id(),
        "hours_total": all_hours.len(),
        "hours_examined": hours.len(),
        "contract": contract,
        "standing_residuals": fog_contract_residuals(),
        "recorded_operand_gap_hours": recorded_gap_counts,
        "own_reveal_oracle": {
            "hours_agreeing": hours_agreeing,
            "hours_refused": hours_refused,
            "recorded_povs": pov_counts,
            "province_slot_lengths": province_slots
                .iter()
                .map(|(slots, hours)| (slots.to_string(), *hours))
                .collect::<BTreeMap<_, _>>(),
            "controlled_provinces_total": controlled_total,
            "controlled_at_full_total": controlled_at_full_total,
            "unexplained_at_full_total": unexplained_total,
            "unmeasured_controller_rows_total": unmeasured_controller_rows_total,
            "counterexamples": counterexample_hours,
        },
    });

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&document).map_err(|error| error.to_string())?
        );
        return Ok(());
    }

    println!("recording: {}", args.path.display());
    println!(
        "hours: {} total, examined {}",
        all_hours.len(),
        hours.len()
    );
    println!();
    println!("== fog derivation input contract ==");
    for row in FOG_DERIVATION_CONTRACT {
        let (kind, source) = origin_words(row.origin);
        println!("{:<28} {:<13} {source}", row.field, kind);
    }
    println!();
    println!("== own-reveal oracle vs the recorded fog map ==");
    println!("hours agreeing: {hours_agreeing} of {}", hours.len());
    for (reason, count) in &hours_refused {
        println!("refused {count} hour(s): {reason}");
    }
    println!(
        "controlled provinces {controlled_total}, at the reveal level {controlled_at_full_total}, \
         at the reveal level but not controlled {unexplained_total}, \
         unmeasured controller rows {unmeasured_controller_rows_total}"
    );
    for (field, count) in &recorded_gap_counts {
        println!("recorded operand {field} missing on {count} hour(s)");
    }
    Ok(())
}
