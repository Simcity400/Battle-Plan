use std::env;
use std::fs;
use std::path::PathBuf;

use focus_research_decisions_depth_live::{gate_existing_depth_stream, DOMAIN_NAME};

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut input: Option<PathBuf> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut commanded_game_hour_advancements: Option<i64> = None;
    let args = env::args().skip(1).collect::<Vec<_>>();
    let mut idx = 0usize;
    while idx < args.len() {
        match args[idx].as_str() {
            "--input" => {
                idx += 1;
                input = args.get(idx).map(PathBuf::from);
            }
            "--out-dir" => {
                idx += 1;
                out_dir = args.get(idx).map(PathBuf::from);
            }
            "--game-hours" => {
                idx += 1;
                let raw = args
                    .get(idx)
                    .ok_or_else(|| "--game-hours requires a positive integer".to_string())?;
                let parsed = raw
                    .parse::<i64>()
                    .map_err(|_| format!("invalid --game-hours value: {raw}"))?;
                if commanded_game_hour_advancements.replace(parsed).is_some() {
                    return Err("--game-hours may only be specified once".to_string());
                }
            }
            "--help" | "-h" => {
                print_usage();
                return Ok(());
            }
            other => return Err(format!("unknown argument: {other}")),
        }
        idx += 1;
    }

    let input = input.ok_or_else(|| "--input is required".to_string())?;
    let out_dir = out_dir.ok_or_else(|| "--out-dir is required".to_string())?;
    let commanded_game_hour_advancements = commanded_game_hour_advancements.ok_or_else(|| {
        "--game-hours is required and means exact positive game-hour advancements; N advancements require exactly N+1 closed frames".to_string()
    })?;
    let raw = fs::read(&input).map_err(|err| format!("read {}: {err}", input.display()))?;
    let summary = gate_existing_depth_stream(&raw, commanded_game_hour_advancements);
    fs::create_dir_all(&out_dir).map_err(|err| format!("create {}: {err}", out_dir.display()))?;
    let manifest = format!(
        concat!(
            "{{\n",
            "  \"lane\": \"{lane}\",\n",
            "  \"source_stream\": {source_stream},\n",
            "  \"data_records\": {data_records},\n",
            "  \"perf_records\": {perf_records},\n",
            "  \"frame_end_records\": {frame_end_records},\n",
            "  \"certified_hours\": {certified_hours},\n",
            "  \"first_game_hour_raw\": {first_game_hour_raw},\n",
            "  \"last_game_hour_raw\": {last_game_hour_raw},\n",
            "  \"game_hours_advanced\": {game_hours_advanced},\n",
            "  \"commanded_game_hour_advancements\": {commanded_game_hour_advancements},\n",
            "  \"required_closed_frames\": {required_closed_frames},\n",
            "  \"status_flags_or\": {status_flags_or},\n",
            "  \"fatal_status_flags\": {fatal_status_flags},\n",
            "  \"focus_event_rows\": {focus_event_rows},\n",
            "  \"focus_completed_exact_rows\": {focus_completed_exact_rows},\n",
            "  \"focus_completed_inferred_rows\": {focus_completed_inferred_rows},\n",
            "  \"focus_completed_unresolved_rows\": {focus_completed_unresolved_rows},\n",
            "  \"research_eta_rows\": {research_eta_rows},\n",
            "  \"decision_status_rows\": {decision_status_rows},\n",
            "  \"decision_event_rows\": {decision_event_rows},\n",
            "  \"decision_target_rows\": {decision_target_rows},\n",
            "  \"decision_country_target_rows\": {decision_country_target_rows},\n",
            "  \"decision_numeric_target_rows\": {decision_numeric_target_rows},\n",
            "  \"badge_rows\": {badge_rows},\n",
            "  \"focus_pause_badge_rows\": {focus_pause_badge_rows},\n",
            "  \"decision_target_badge_rows\": {decision_target_badge_rows},\n",
            "  \"direct_event_badge_rows\": {direct_event_badge_rows},\n",
            "  \"parse_errors\": {parse_errors},\n",
            "  \"ok\": {ok}\n",
            "}}\n"
        ),
        lane = DOMAIN_NAME,
        source_stream = json_str(&input.display().to_string()),
        data_records = summary.data_records,
        perf_records = summary.perf_records,
        frame_end_records = summary.frame_end_records,
        certified_hours = summary.certified_hours,
        first_game_hour_raw = json_optional_i32(summary.first_game_hour_raw),
        last_game_hour_raw = json_optional_i32(summary.last_game_hour_raw),
        game_hours_advanced = summary.game_hours_advanced,
        commanded_game_hour_advancements = summary.commanded_game_hour_advancements,
        required_closed_frames = summary.required_closed_frames,
        status_flags_or = summary.status_flags_or,
        fatal_status_flags = summary.fatal_status_flags,
        focus_event_rows = summary.focus_event_rows,
        focus_completed_exact_rows = summary.focus_completed_exact_rows,
        focus_completed_inferred_rows = summary.focus_completed_inferred_rows,
        focus_completed_unresolved_rows = summary.focus_completed_unresolved_rows,
        research_eta_rows = summary.research_eta_rows,
        decision_status_rows = summary.decision_status_rows,
        decision_event_rows = summary.decision_event_rows,
        decision_target_rows = summary.decision_target_rows,
        decision_country_target_rows = summary.decision_country_target_rows,
        decision_numeric_target_rows = summary.decision_numeric_target_rows,
        badge_rows = summary.badge_rows,
        focus_pause_badge_rows = summary.focus_pause_badge_rows,
        decision_target_badge_rows = summary.decision_target_badge_rows,
        direct_event_badge_rows = summary.direct_event_badge_rows,
        parse_errors = json_string_array(&summary.parse_errors),
        ok = summary.data_records > 0 && summary.parse_errors.is_empty(),
    );
    fs::write(out_dir.join("depth_gate_manifest.json"), manifest).map_err(|err| {
        format!(
            "write {}: {err}",
            out_dir.join("depth_gate_manifest.json").display()
        )
    })?;
    println!(
        "wrote {}",
        out_dir.join("depth_gate_manifest.json").display()
    );
    println!(
        "records={} focus_events={} exact_completed={} inferred_completed={} unresolved_completed={} decision_targets={} country_targets={} numeric_targets={} badges={} errors={}",
        summary.data_records,
        summary.focus_event_rows,
        summary.focus_completed_exact_rows,
        summary.focus_completed_inferred_rows,
        summary.focus_completed_unresolved_rows,
        summary.decision_target_rows,
        summary.decision_country_target_rows,
        summary.decision_numeric_target_rows,
        summary.badge_rows,
        summary.parse_errors.len()
    );
    if summary.data_records == 0 || !summary.parse_errors.is_empty() {
        return Err("depth gate failed; see depth_gate_manifest.json".to_string());
    }
    Ok(())
}

fn print_usage() {
    println!("gate_depth_stream --input <collector_stream.bin> --out-dir <lane-proof-dir> --game-hours <exact positive advancements>");
}

fn json_optional_i32(value: Option<i32>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string())
}

fn json_string_array(values: &[String]) -> String {
    let items = values
        .iter()
        .map(|value| json_str(value))
        .collect::<Vec<_>>();
    format!("[{}]", items.join(", "))
}

fn json_str(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}
