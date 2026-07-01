use country_economy_production_stockpile_live_lane::{
    append_compact_record, default_expected_domains, domain_name, encode_gap_badges_payload,
    gap_badge_rows, gate_compact_stream, remaining_gap_badges, DOMAIN_FRAME, DOMAIN_GAP_BADGES,
    DOMAIN_PRODUCTION, PRODUCTION_CONSTRUCTION_ROW_LEN, PRODUCTION_LINE_ROW_LEN,
    RECORD_TYPE_DOMAIN_DATA, RECORD_TYPE_DOMAIN_PERF, RECORD_TYPE_FRAME_END,
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
    let mut production_stream = None;
    let mut out = None;
    let mut badge_stream_out = None;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--production-stream" => {
                i += 1;
                production_stream =
                    Some(PathBuf::from(required(args.get(i), "--production-stream")?));
            }
            "--out" => {
                i += 1;
                out = Some(PathBuf::from(required(args.get(i), "--out")?));
            }
            "--badge-stream-out" => {
                i += 1;
                badge_stream_out =
                    Some(PathBuf::from(required(args.get(i), "--badge-stream-out")?));
            }
            "--help" | "-h" => return Err(usage()),
            other => return Err(format!("unknown argument {other}\n{}", usage())),
        }
        i += 1;
    }

    let gaps = remaining_gap_badges();
    let rows = gap_badge_rows(&gaps);
    let badge_stream = badge_compact_stream(&rows);
    let badge_gate = gate_compact_stream(&badge_stream, &[DOMAIN_GAP_BADGES]);

    if let Some(path) = badge_stream_out.as_ref() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("create badge stream dir failed: {e}"))?;
        }
        fs::write(path, &badge_stream).map_err(|e| format!("write badge stream failed: {e}"))?;
    }

    let production_gate = match production_stream.as_ref() {
        Some(path) => {
            let raw = fs::read(path).map_err(|e| format!("read production stream failed: {e}"))?;
            Some(gate_compact_stream(&raw, &[DOMAIN_PRODUCTION]))
        }
        None => None,
    };

    let json = report_json(
        &gaps,
        production_stream.as_ref(),
        production_gate.as_ref(),
        badge_stream_out.as_ref(),
        &badge_gate,
    );
    if let Some(path) = out {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("create output dir failed: {e}"))?;
        }
        fs::write(&path, json).map_err(|e| format!("write gap report failed: {e}"))?;
    } else {
        println!("{json}");
    }

    if !badge_gate.ok {
        return Err("gap badge compact gate failed; proof JSON was still written".to_string());
    }
    if let Some(gate) = production_gate {
        if !gate.ok {
            return Err("production stream gate failed; proof JSON was still written".to_string());
        }
    }
    Ok(())
}

fn badge_compact_stream(
    rows: &[country_economy_production_stockpile_live_lane::GapBadgeRow],
) -> Vec<u8> {
    let mut raw = Vec::new();
    append_compact_record(
        &mut raw,
        1,
        0,
        DOMAIN_GAP_BADGES,
        RECORD_TYPE_DOMAIN_PERF,
        &perf_payload(1, rows.len() as u32, rows.len() as u32),
    );
    append_compact_record(
        &mut raw,
        2,
        0,
        DOMAIN_GAP_BADGES,
        RECORD_TYPE_DOMAIN_DATA,
        &encode_gap_badges_payload(1, 32, rows),
    );
    append_compact_record(
        &mut raw,
        3,
        0,
        DOMAIN_FRAME,
        RECORD_TYPE_FRAME_END,
        &frame_payload(1),
    );
    raw
}

fn report_json(
    gaps: &[country_economy_production_stockpile_live_lane::RemainingGapBadge],
    production_stream: Option<&PathBuf>,
    production_gate: Option<&country_economy_production_stockpile_live_lane::LaneGate>,
    badge_stream_out: Option<&PathBuf>,
    badge_gate: &country_economy_production_stockpile_live_lane::LaneGate,
) -> String {
    let source_blocked = gaps
        .iter()
        .filter(|gap| gap.status_label == "source_blocked")
        .count();
    let schema_blocked = gaps
        .iter()
        .filter(|gap| gap.status_label == "schema_blocked")
        .count();
    let owned_elsewhere = gaps
        .iter()
        .filter(|gap| gap.status_label == "owned_elsewhere_badged")
        .count();
    let mut out = String::new();
    out.push_str("{\n");
    push_field(
        &mut out,
        1,
        "schema",
        "\"country_economy_production_stockpile_live_gap_badges_v1\"",
        true,
    );
    push_field(&mut out, 1, "all_remaining_gaps_badged", "true", true);
    push_field(
        &mut out,
        1,
        "remaining_gap_count",
        &gaps.len().to_string(),
        true,
    );
    push_field(
        &mut out,
        1,
        "source_blocked_gap_count",
        &source_blocked.to_string(),
        true,
    );
    push_field(
        &mut out,
        1,
        "schema_blocked_gap_count",
        &schema_blocked.to_string(),
        true,
    );
    push_field(
        &mut out,
        1,
        "owned_elsewhere_badged_gap_count",
        &owned_elsewhere.to_string(),
        true,
    );
    push_field(
        &mut out,
        1,
        "accepted_domains_unchanged",
        &json_array_strings(
            &default_expected_domains()
                .into_iter()
                .map(domain_name)
                .collect::<Vec<_>>(),
        ),
        true,
    );
    push_field(
        &mut out,
        1,
        "production_line_current_row_len",
        &PRODUCTION_LINE_ROW_LEN.to_string(),
        true,
    );
    push_field(
        &mut out,
        1,
        "production_construction_current_row_len",
        &PRODUCTION_CONSTRUCTION_ROW_LEN.to_string(),
        true,
    );
    if let Some(path) = production_stream {
        push_field(
            &mut out,
            1,
            "production_stream",
            &json_string(&path.display().to_string()),
            true,
        );
    }
    if let Some(path) = badge_stream_out {
        push_field(
            &mut out,
            1,
            "gap_badge_compact_stream",
            &json_string(&path.display().to_string()),
            true,
        );
    }
    push_field(
        &mut out,
        1,
        "gap_badge_compact_gate_ok",
        if badge_gate.ok { "true" } else { "false" },
        true,
    );
    push_field(
        &mut out,
        1,
        "gap_badge_stream_hours",
        &badge_gate.summary.unique_hours.len().to_string(),
        true,
    );
    push_field(
        &mut out,
        1,
        "gap_badge_skipped_hours",
        &badge_gate.summary.skipped_hours.to_string(),
        true,
    );
    if let Some(gate) = production_gate {
        push_field(
            &mut out,
            1,
            "production_stream_gate_ok",
            if gate.ok { "true" } else { "false" },
            true,
        );
        push_field(
            &mut out,
            1,
            "production_stream_hours",
            &gate.summary.unique_hours.len().to_string(),
            true,
        );
        push_field(
            &mut out,
            1,
            "production_stream_skipped_hours",
            &gate.summary.skipped_hours.to_string(),
            true,
        );
        if let Some(domain) = gate.summary.per_domain.get(&DOMAIN_PRODUCTION) {
            push_field(
                &mut out,
                1,
                "production_stream_data_records",
                &domain.data_record_count.to_string(),
                true,
            );
            out.push_str("  \"production_stream_units\": {\n");
            for (idx, (key, value)) in domain.data_units_total.iter().enumerate() {
                out.push_str(&format!(
                    "    {}: {}{}\n",
                    json_string(key),
                    value,
                    if idx + 1 == domain.data_units_total.len() {
                        ""
                    } else {
                        ","
                    }
                ));
            }
            out.push_str("  },\n");
        }
    }
    out.push_str("  \"gaps\": [\n");
    for (idx, gap) in gaps.iter().enumerate() {
        out.push_str("    {\n");
        push_field(&mut out, 3, "gap_id", &gap.gap_id.to_string(), true);
        push_field(&mut out, 3, "name", &json_string(gap.name), true);
        push_field(&mut out, 3, "status", &json_string(gap.status_label), true);
        push_field(
            &mut out,
            3,
            "current_support",
            &gap.current_support.to_string(),
            true,
        );
        push_field(
            &mut out,
            3,
            "evidence_mask",
            &gap.evidence_mask.to_string(),
            true,
        );
        push_field(
            &mut out,
            3,
            "current_compact_evidence",
            &json_string(gap.current_compact_evidence),
            true,
        );
        push_field(
            &mut out,
            3,
            "source_evidence",
            &json_string(gap.source_evidence),
            true,
        );
        push_field(
            &mut out,
            3,
            "fail_closed_badge",
            &json_string(gap.fail_closed_badge),
            true,
        );
        push_field(
            &mut out,
            3,
            "proposed_domain_id",
            &gap.proposed_domain_id.to_string(),
            true,
        );
        push_field(
            &mut out,
            3,
            "proposed_domain",
            &json_string(gap.proposed_domain_name),
            true,
        );
        push_field(
            &mut out,
            3,
            "owner_boundary",
            &json_string(gap.owner_boundary),
            false,
        );
        out.push_str(&format!(
            "    }}{}\n",
            if idx + 1 == gaps.len() { "" } else { "," }
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
    "usage: gap_report [--production-stream collector_stream.bin] [--badge-stream-out gap_badges_stream.bin] [--out gap_badges.json]".to_string()
}

fn perf_payload(read_time_us: u64, rows: u32, units: u32) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&0u64.to_le_bytes());
    payload.extend_from_slice(&read_time_us.to_le_bytes());
    payload.extend_from_slice(&rows.to_le_bytes());
    payload.extend_from_slice(&rows.to_le_bytes());
    payload.extend_from_slice(&units.to_le_bytes());
    payload.extend_from_slice(&units.to_le_bytes());
    payload.extend_from_slice(&32u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload
}

fn frame_payload(domains: u32) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&0u64.to_le_bytes());
    payload.extend_from_slice(&1u64.to_le_bytes());
    payload.extend_from_slice(&domains.to_le_bytes());
    payload.extend_from_slice(&domains.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload
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

fn json_array_strings(values: &[&str]) -> String {
    let mut out = String::from("[");
    for (idx, value) in values.iter().enumerate() {
        if idx != 0 {
            out.push_str(", ");
        }
        out.push_str(&json_string(value));
    }
    out.push(']');
    out
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
