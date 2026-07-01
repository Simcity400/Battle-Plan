use std::env;
use std::fs;
use std::path::PathBuf;

use focus_research_decisions_depth_live::derive_depth_stream;

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
    let proof = derive_depth_stream(&raw)?;

    fs::create_dir_all(&out_dir).map_err(|err| format!("create {}: {err}", out_dir.display()))?;
    fs::write(out_dir.join("depth_stream.bin"), &proof.depth_stream).map_err(|err| {
        format!(
            "write {}: {err}",
            out_dir.join("depth_stream.bin").display()
        )
    })?;
    fs::write(
        out_dir.join("proof_manifest.json"),
        proof.manifest_json(&input.display().to_string()),
    )
    .map_err(|err| {
        format!(
            "write {}: {err}",
            out_dir.join("proof_manifest.json").display()
        )
    })?;
    fs::write(
        out_dir.join("sample_events.jsonl"),
        proof.sample_events_jsonl(),
    )
    .map_err(|err| {
        format!(
            "write {}: {err}",
            out_dir.join("sample_events.jsonl").display()
        )
    })?;

    println!("wrote {}", out_dir.join("depth_stream.bin").display());
    println!("wrote {}", out_dir.join("proof_manifest.json").display());
    println!("wrote {}", out_dir.join("sample_events.jsonl").display());
    println!(
        "frames={} skipped_hours={} focus_events={} research_eta_rows={} decision_events={} decision_status_rows={} decision_targets={} country_targets={} numeric_targets={} badge_rows={}",
        proof.source_frames,
        proof.skipped_game_hours,
        proof.focus_event_rows,
        proof.research_eta_rows,
        proof.decision_event_rows,
        proof.decision_status_rows,
        proof.decision_target_rows,
        proof.decision_country_target_rows,
        proof.decision_numeric_target_rows,
        proof.badge_rows
    );
    if !proof.parse_errors.is_empty() || !proof.depth_parse_errors.is_empty() {
        return Err("proof completed with parse errors; see proof_manifest.json".to_string());
    }
    Ok(())
}

fn print_usage() {
    println!("proof_from_stream --input <collector_stream.bin> --out-dir <lane-proof-dir>");
}
