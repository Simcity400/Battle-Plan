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
    let raw = fs::read(&input).map_err(|err| format!("read {}: {err}", input.display()))?;
    let summary = gate_existing_depth_stream(&raw);
    fs::create_dir_all(&out_dir).map_err(|err| format!("create {}: {err}", out_dir.display()))?;
    let manifest = format!(
        concat!(
            "{{\n",
            "  \"lane\": \"{lane}\",\n",
            "  \"source_stream\": {source_stream},\n",
            "  \"data_records\": {data_records},\n",
            "  \"focus_event_rows\": {focus_event_rows},\n",
            "  \"focus_completed_exact_rows\": {focus_completed_exact_rows},\n",
            "  \"focus_completed_inferred_rows\": {focus_completed_inferred_rows},\n",
            "  \"focus_completed_unresolved_rows\": {focus_completed_unresolved_rows},\n",
            "  \"research_eta_rows\": {research_eta_rows},\n",
            "  \"decision_status_rows\": {decision_status_rows},\n",
            "  \"decision_event_rows\": {decision_event_rows},\n",
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
        focus_event_rows = summary.focus_event_rows,
        focus_completed_exact_rows = summary.focus_completed_exact_rows,
        focus_completed_inferred_rows = summary.focus_completed_inferred_rows,
        focus_completed_unresolved_rows = summary.focus_completed_unresolved_rows,
        research_eta_rows = summary.research_eta_rows,
        decision_status_rows = summary.decision_status_rows,
        decision_event_rows = summary.decision_event_rows,
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
        "records={} focus_events={} exact_completed={} inferred_completed={} unresolved_completed={} badges={} errors={}",
        summary.data_records,
        summary.focus_event_rows,
        summary.focus_completed_exact_rows,
        summary.focus_completed_inferred_rows,
        summary.focus_completed_unresolved_rows,
        summary.badge_rows,
        summary.parse_errors.len()
    );
    if summary.data_records == 0 || !summary.parse_errors.is_empty() {
        return Err("depth gate failed; see depth_gate_manifest.json".to_string());
    }
    Ok(())
}

fn print_usage() {
    println!("gate_depth_stream --input <collector_stream.bin> --out-dir <lane-proof-dir>");
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
