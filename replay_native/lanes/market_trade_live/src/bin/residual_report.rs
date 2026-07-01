use market_trade_live_lane::{
    append_compact_record, encode_payload_with_residual_badges, gate_compact_stream,
    market_trade_residual_badges, residual_badge_rows, MarketTradeGate, MarketTradeResidualBadge,
    DOMAIN_ID, RECORD_TYPE_DOMAIN_DATA, RECORD_TYPE_FRAME_END,
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
    let mut out = None;
    let mut stream_out = None;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                i += 1;
                out = Some(PathBuf::from(required(args.get(i), "--out")?));
            }
            "--stream-out" => {
                i += 1;
                stream_out = Some(PathBuf::from(required(args.get(i), "--stream-out")?));
            }
            "--help" | "-h" => return Err(usage()),
            other => return Err(format!("unknown argument {other}\n{}", usage())),
        }
        i += 1;
    }

    let badges = market_trade_residual_badges();
    let rows = residual_badge_rows(&badges);
    let payload = encode_payload_with_residual_badges(0, 0, 0, 0, &[], &rows);
    let mut stream = Vec::new();
    append_compact_record(
        &mut stream,
        1,
        0,
        DOMAIN_ID,
        RECORD_TYPE_DOMAIN_DATA,
        &payload,
    );
    append_compact_record(&mut stream, 2, 0, 0, RECORD_TYPE_FRAME_END, &[]);
    let gate = gate_compact_stream(&stream, 1);

    if let Some(path) = stream_out.as_ref() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("create residual stream dir failed: {err}"))?;
        }
        fs::write(path, &stream).map_err(|err| format!("write residual stream failed: {err}"))?;
    }

    let json = report_json(&badges, stream_out.as_ref(), stream.len(), &gate);
    if let Some(path) = out.as_ref() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("create residual report dir failed: {err}"))?;
        }
        fs::write(path, json).map_err(|err| format!("write residual report failed: {err}"))?;
    } else {
        println!("{json}");
    }

    if gate.ok {
        Ok(())
    } else {
        Err("market trade residual badge gate failed; proof JSON was still written".to_string())
    }
}

fn report_json(
    badges: &[MarketTradeResidualBadge],
    stream_out: Option<&PathBuf>,
    stream_bytes: usize,
    gate: &MarketTradeGate,
) -> String {
    let source_blocked = badges
        .iter()
        .filter(|badge| badge.status_label == "source_blocked")
        .count();
    let schema_blocked = badges
        .iter()
        .filter(|badge| badge.status_label == "schema_blocked")
        .count();
    let owned_elsewhere = badges
        .iter()
        .filter(|badge| badge.status_label == "owned_elsewhere_context")
        .count();
    let shared_patch_blocked = badges
        .iter()
        .filter(|badge| badge.status_label == "shared_patch_blocked")
        .count();
    let validated_slice = badges
        .iter()
        .filter(|badge| badge.status_label == "validated_slice")
        .count();

    let mut out = String::new();
    out.push_str("{\n");
    push_field(
        &mut out,
        1,
        "schema",
        "\"market_trade_live_residual_badges_v1\"",
        true,
    );
    push_field(&mut out, 1, "domain_id", &DOMAIN_ID.to_string(), true);
    push_field(&mut out, 1, "domain_name", "\"market_trade_live\"", true);
    push_field(
        &mut out,
        1,
        "stream_format",
        "\"H4CR domain 43 with optional MTRB residual trailer\"",
        true,
    );
    if let Some(path) = stream_out {
        push_field(
            &mut out,
            1,
            "residual_compact_stream",
            &json_string(&path.display().to_string()),
            true,
        );
    }
    push_field(
        &mut out,
        1,
        "residual_compact_stream_bytes",
        &stream_bytes.to_string(),
        true,
    );
    push_field(&mut out, 1, "all_residuals_classified", "true", true);
    push_field(
        &mut out,
        1,
        "residual_gap_count",
        &badges.len().to_string(),
        true,
    );
    push_field(
        &mut out,
        1,
        "validated_slice_count",
        &validated_slice.to_string(),
        true,
    );
    push_field(
        &mut out,
        1,
        "source_blocked_count",
        &source_blocked.to_string(),
        true,
    );
    push_field(
        &mut out,
        1,
        "schema_blocked_count",
        &schema_blocked.to_string(),
        true,
    );
    push_field(
        &mut out,
        1,
        "owned_elsewhere_context_count",
        &owned_elsewhere.to_string(),
        true,
    );
    push_field(
        &mut out,
        1,
        "shared_patch_blocked_count",
        &shared_patch_blocked.to_string(),
        true,
    );
    push_field(&mut out, 1, "gate_ok", bool_json(gate.ok), true);
    let gate_problems = gate
        .problems
        .iter()
        .map(|problem| (*problem).to_string())
        .collect::<Vec<_>>();
    push_array_strings(&mut out, 1, "gate_problems", &gate_problems, true);
    push_field(
        &mut out,
        1,
        "domain_data_records",
        &gate.summary.domain_data_records.to_string(),
        true,
    );
    push_field(
        &mut out,
        1,
        "unique_game_hours",
        &gate.summary.unique_game_hours.to_string(),
        true,
    );
    push_field(
        &mut out,
        1,
        "skipped_game_hours",
        &gate.summary.skipped_game_hours.len().to_string(),
        true,
    );
    push_field(
        &mut out,
        1,
        "residual_badge_records",
        &gate.summary.residual_badge_records.to_string(),
        true,
    );
    push_field(
        &mut out,
        1,
        "residual_badge_rows_total",
        &gate.summary.residual_badge_rows_total.to_string(),
        true,
    );
    push_field(
        &mut out,
        1,
        "residual_source_blocked_rows_total",
        &gate.summary.residual_source_blocked_rows_total.to_string(),
        true,
    );
    push_field(
        &mut out,
        1,
        "residual_schema_blocked_rows_total",
        &gate.summary.residual_schema_blocked_rows_total.to_string(),
        true,
    );
    push_field(
        &mut out,
        1,
        "residual_owned_elsewhere_rows_total",
        &gate.summary.residual_owned_elsewhere_rows_total.to_string(),
        true,
    );
    push_field(
        &mut out,
        1,
        "residual_shared_patch_blocked_rows_total",
        &gate
            .summary
            .residual_shared_patch_blocked_rows_total
            .to_string(),
        true,
    );
    push_array_strings(
        &mut out,
        1,
        "parse_errors",
        &gate.summary.parse_errors,
        true,
    );
    out.push_str("  \"residuals\": [\n");
    for (idx, badge) in badges.iter().enumerate() {
        out.push_str("    {\n");
        push_field(&mut out, 3, "gap_id", &badge.gap_id.to_string(), true);
        push_field(&mut out, 3, "name", &json_string(badge.name), true);
        push_field(
            &mut out,
            3,
            "status",
            &json_string(badge.status_label),
            true,
        );
        push_field(
            &mut out,
            3,
            "current_support",
            &badge.current_support.to_string(),
            true,
        );
        push_field(
            &mut out,
            3,
            "evidence_mask",
            &badge.evidence_mask.to_string(),
            true,
        );
        push_field(
            &mut out,
            3,
            "current_compact_evidence",
            &json_string(badge.current_compact_evidence),
            true,
        );
        push_field(
            &mut out,
            3,
            "source_evidence",
            &json_string(badge.source_evidence),
            true,
        );
        push_field(
            &mut out,
            3,
            "fail_closed_badge",
            &json_string(badge.fail_closed_badge),
            true,
        );
        push_field(
            &mut out,
            3,
            "proposed_record_kind",
            &badge.proposed_record_kind.to_string(),
            true,
        );
        push_field(
            &mut out,
            3,
            "proposed_record",
            &json_string(badge.proposed_record),
            true,
        );
        push_field(&mut out, 3, "owner", &json_string(badge.owner_label), false);
        out.push_str(&format!(
            "    }}{}\n",
            if idx + 1 == badges.len() { "" } else { "," }
        ));
    }
    out.push_str("  ]\n");
    out.push_str("}\n");
    out
}

fn required(value: Option<&String>, flag: &str) -> Result<String, String> {
    value
        .cloned()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn usage() -> String {
    "usage: residual_report [--stream-out residual_badges_stream.bin] [--out residual_badges.json]"
        .to_string()
}

fn bool_json(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

fn push_field(out: &mut String, indent: usize, key: &str, value: &str, comma: bool) {
    out.push_str(&"  ".repeat(indent));
    out.push_str(&json_string(key));
    out.push_str(": ");
    out.push_str(value);
    if comma {
        out.push(',');
    }
    out.push('\n');
}

fn push_array_strings(out: &mut String, indent: usize, key: &str, values: &[String], comma: bool) {
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
