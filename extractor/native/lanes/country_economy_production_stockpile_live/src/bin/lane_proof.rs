use country_economy_production_stockpile_live_lane::{
    default_expected_domains, domain_id, domain_name, gate_compact_stream, LaneGate,
};
use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let mut stream = None;
    let mut manifest = None;
    let mut replay = None;
    let mut out = None;
    let mut expected = Vec::new();
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--stream" => {
                i += 1;
                stream = Some(PathBuf::from(required(args.get(i), "--stream")?));
            }
            "--manifest" => {
                i += 1;
                manifest = Some(PathBuf::from(required(args.get(i), "--manifest")?));
            }
            "--replay" => {
                i += 1;
                replay = Some(PathBuf::from(required(args.get(i), "--replay")?));
            }
            "--out" => {
                i += 1;
                out = Some(PathBuf::from(required(args.get(i), "--out")?));
            }
            "--expected" => {
                i += 1;
                let name = required(args.get(i), "--expected")?;
                let id =
                    domain_id(&name).ok_or_else(|| format!("unknown expected domain {name}"))?;
                if !expected.contains(&id) {
                    expected.push(id);
                }
            }
            "--help" | "-h" => return Err(usage()),
            other => return Err(format!("unknown argument {other}\n{}", usage())),
        }
        i += 1;
    }
    let stream = stream.ok_or_else(usage)?;
    let raw = fs::read(&stream).map_err(|e| format!("read stream failed: {e}"))?;
    if expected.is_empty() {
        expected = default_expected_domains();
    }
    let gate = gate_compact_stream(&raw, &expected);
    let manifest_path = manifest.clone();
    let manifest_text = match manifest {
        Some(path) => {
            Some(fs::read_to_string(path).map_err(|e| format!("read manifest failed: {e}"))?)
        }
        None => None,
    };
    let replay_exists = replay.as_ref().map(|path| path.exists());
    let json = proof_json(
        &gate,
        &stream,
        manifest_path.as_ref(),
        manifest_text.as_deref(),
        replay.as_ref(),
        replay_exists,
    );
    if let Some(path) = out {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("create output dir failed: {e}"))?;
        }
        fs::write(&path, json).map_err(|e| format!("write proof failed: {e}"))?;
    } else {
        println!("{json}");
    }
    if gate.ok {
        Ok(())
    } else {
        Err("lane gate failed; proof JSON was still written".to_string())
    }
}

fn proof_json(
    gate: &LaneGate,
    stream: &PathBuf,
    manifest: Option<&PathBuf>,
    manifest_text: Option<&str>,
    replay: Option<&PathBuf>,
    replay_exists: Option<bool>,
) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    push_json_field(
        &mut out,
        1,
        "schema",
        "\"country_economy_production_stockpile_live_lane_proof_v1\"",
        true,
    );
    push_json_field(
        &mut out,
        1,
        "stream",
        &json_string(&stream.display().to_string()),
        true,
    );
    if let Some(manifest) = manifest {
        push_json_field(
            &mut out,
            1,
            "manifest",
            &json_string(&manifest.display().to_string()),
            true,
        );
    }
    if let Some(replay) = replay {
        push_json_field(
            &mut out,
            1,
            "replay",
            &json_string(&replay.display().to_string()),
            true,
        );
    }
    if let Some(exists) = replay_exists {
        push_json_field(
            &mut out,
            1,
            "replay_file_exists",
            if exists { "true" } else { "false" },
            true,
        );
    }
    if let Some(text) = manifest_text {
        push_json_field(
            &mut out,
            1,
            "manifest_mentions_replay_file",
            if text.contains("\"replay_file\"") {
                "true"
            } else {
                "false"
            },
            true,
        );
        push_json_field(
            &mut out,
            1,
            "manifest_hourly_requirement_met",
            if text.contains("\"met\": true") {
                "true"
            } else {
                "false"
            },
            true,
        );
        push_json_field(
            &mut out,
            1,
            "manifest_zero_skipped_hours",
            if text.contains("\"skipped_game_hours\": 0") {
                "true"
            } else {
                "false"
            },
            true,
        );
    }
    push_json_field(
        &mut out,
        1,
        "ok",
        if gate.ok { "true" } else { "false" },
        true,
    );
    push_json_array_numbers(
        &mut out,
        1,
        "expected_domain_ids",
        &gate.expected_domains,
        true,
    );
    push_json_array_strings(
        &mut out,
        1,
        "expected_domains",
        &gate
            .expected_domains
            .iter()
            .map(|id| domain_name(*id).to_string())
            .collect::<Vec<_>>(),
        true,
    );
    let summary = &gate.summary;
    push_json_field(
        &mut out,
        1,
        "total_bytes",
        &summary.total_bytes.to_string(),
        true,
    );
    push_json_field(
        &mut out,
        1,
        "total_payload_bytes",
        &summary.total_payload_bytes.to_string(),
        true,
    );
    push_json_field(
        &mut out,
        1,
        "record_count",
        &summary.record_count.to_string(),
        true,
    );
    push_json_field(
        &mut out,
        1,
        "frame_end_record_count",
        &summary.frame_end_record_count.to_string(),
        true,
    );
    push_json_field(
        &mut out,
        1,
        "unique_game_hour_count",
        &summary.unique_hours.len().to_string(),
        true,
    );
    push_json_field(
        &mut out,
        1,
        "first_game_hour_raw",
        &summary
            .first_hour
            .map_or("null".to_string(), |v| v.to_string()),
        true,
    );
    push_json_field(
        &mut out,
        1,
        "last_game_hour_raw",
        &summary
            .last_hour
            .map_or("null".to_string(), |v| v.to_string()),
        true,
    );
    push_json_field(
        &mut out,
        1,
        "skipped_game_hours",
        &summary.skipped_hours.to_string(),
        true,
    );
    push_json_field(
        &mut out,
        1,
        "ordered",
        if summary.ordered { "true" } else { "false" },
        true,
    );
    push_json_field(
        &mut out,
        1,
        "status_flags_or",
        &summary.status_flags_or.to_string(),
        true,
    );
    push_json_array_strings(&mut out, 1, "parse_errors", &summary.parse_errors, true);
    push_json_array_strings(
        &mut out,
        1,
        "ordering_errors",
        &summary.ordering_errors,
        true,
    );
    push_json_array_strings(
        &mut out,
        1,
        "coverage_errors",
        &summary.coverage_errors,
        true,
    );
    out.push_str("  \"per_domain\": {\n");
    for (idx, (_id, domain)) in summary.per_domain.iter().enumerate() {
        out.push_str(&format!("    {:?}: {{\n", domain.name));
        push_json_field(
            &mut out,
            3,
            "domain_id",
            &domain.domain_id.to_string(),
            true,
        );
        push_json_field(
            &mut out,
            3,
            "record_count",
            &domain.record_count.to_string(),
            true,
        );
        push_json_field(
            &mut out,
            3,
            "perf_record_count",
            &domain.perf_record_count.to_string(),
            true,
        );
        push_json_field(
            &mut out,
            3,
            "data_record_count",
            &domain.data_record_count.to_string(),
            true,
        );
        push_json_field(
            &mut out,
            3,
            "row_count_total",
            &domain.row_count_total.to_string(),
            true,
        );
        push_json_field(
            &mut out,
            3,
            "changed_rows_total",
            &domain.changed_rows_total.to_string(),
            true,
        );
        push_json_field(
            &mut out,
            3,
            "unit_count_total",
            &domain.unit_count_total.to_string(),
            true,
        );
        push_json_field(
            &mut out,
            3,
            "changed_unit_count_total",
            &domain.changed_unit_count_total.to_string(),
            true,
        );
        push_json_field(
            &mut out,
            3,
            "data_rows_total",
            &domain.data_rows_total.to_string(),
            true,
        );
        push_json_field(
            &mut out,
            3,
            "data_changed_rows_total",
            &domain.data_changed_rows_total.to_string(),
            true,
        );
        push_json_field(
            &mut out,
            3,
            "status_flags_or",
            &domain.status_flags_or.to_string(),
            true,
        );
        out.push_str("      \"data_units_total\": {\n");
        for (unit_idx, (key, value)) in domain.data_units_total.iter().enumerate() {
            out.push_str(&format!(
                "        {}: {}{}\n",
                json_string(key),
                value,
                if unit_idx + 1 == domain.data_units_total.len() {
                    ""
                } else {
                    ","
                }
            ));
        }
        out.push_str("      }\n");
        out.push_str(&format!(
            "    }}{}\n",
            if idx + 1 == summary.per_domain.len() {
                ""
            } else {
                ","
            }
        ));
    }
    out.push_str("  }\n");
    out.push_str("}\n");
    out
}

fn required(value: Option<&String>, flag: &str) -> Result<String, String> {
    value
        .cloned()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn usage() -> String {
    "usage: lane_proof --stream <collector_stream.bin> [--manifest native_session_manifest.json] [--replay native_session.battleplan] [--out proof.json] [--expected fuel_live|production_live|stockpile_variant_defs|stockpile_variant_upgrades]".to_string()
}

fn push_json_field(out: &mut String, indent: usize, key: &str, value: &str, comma: bool) {
    out.push_str(&"  ".repeat(indent));
    out.push_str(&json_string(key));
    out.push_str(": ");
    out.push_str(value);
    if comma {
        out.push(',');
    }
    out.push('\n');
}

fn push_json_array_numbers(
    out: &mut String,
    indent: usize,
    key: &str,
    values: &[u16],
    comma: bool,
) {
    out.push_str(&"  ".repeat(indent));
    out.push_str(&json_string(key));
    out.push_str(": [");
    for (idx, value) in values.iter().enumerate() {
        if idx != 0 {
            out.push_str(", ");
        }
        out.push_str(&value.to_string());
    }
    out.push(']');
    if comma {
        out.push(',');
    }
    out.push('\n');
}

fn push_json_array_strings(
    out: &mut String,
    indent: usize,
    key: &str,
    values: &[String],
    comma: bool,
) {
    out.push_str(&"  ".repeat(indent));
    out.push_str(&json_string(key));
    out.push_str(": [");
    for (idx, value) in values.iter().enumerate() {
        if idx != 0 {
            out.push_str(", ");
        }
        out.push_str(&json_string(value));
    }
    out.push(']');
    if comma {
        out.push(',');
    }
    out.push('\n');
}

fn json_string(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
