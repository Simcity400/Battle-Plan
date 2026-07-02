use country_economy_counters_live_lane::{
    append_compact_record, counter_badge_rows, domain_name, economy_counter_badges,
    encode_counter_payload, gate_counter_stream, summarize_source_stream, DOMAIN_FRAME,
    DOMAIN_FUEL_LIVE, DOMAIN_ID, DOMAIN_PRODUCTION_LIVE, RECORD_TYPE_DOMAIN_DATA,
    RECORD_TYPE_DOMAIN_PERF, RECORD_TYPE_FRAME_END, STATUS_LANE_LOCAL_PROOF,
};
use serde_json::json;
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
    let options = Options::parse()?;
    let badges = economy_counter_badges();
    let rows = counter_badge_rows(&badges);
    let stream = counter_stream(&rows);
    let gate = gate_counter_stream(&stream, 1);
    let source_gate = match options.source_stream.as_ref() {
        Some(path) => {
            let raw = fs::read(path).map_err(|err| format!("read source stream failed: {err}"))?;
            Some(summarize_source_stream(
                &raw,
                &[DOMAIN_FUEL_LIVE, DOMAIN_PRODUCTION_LIVE],
            ))
        }
        None => None,
    };

    if let Some(path) = options.stream_out.as_ref() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| format!("create stream dir failed: {err}"))?;
        }
        fs::write(path, &stream).map_err(|err| format!("write counter stream failed: {err}"))?;
    }

    let report = json!({
        "schema": "country_economy_counters_live.counter_report.v1",
        "domain": domain_name(DOMAIN_ID),
        "domain_id": DOMAIN_ID,
        "master_rows_narrowed": [
            "Fuel: capacity/gain/use audited against domain 10",
            "Military production: residual efficiency/resource shortage/conversion/line history separated from domain 11 captured fields",
            "Factories/industry capacity: UI-visible totals/damaged/free/consumer-goods badged as source-blocked",
            "Resources: non-market country resource flows/shortages separated from market_trade domain 43 and state deposit owners"
        ],
        "backlog_reduced": true,
        "status_counts": {
            "validated_slice": badges.iter().filter(|b| b.status_label == "validated_slice").count(),
            "schema_blocked": badges.iter().filter(|b| b.status_label == "schema_blocked").count(),
            "source_blocked": badges.iter().filter(|b| b.status_label == "source_blocked").count(),
            "owned_elsewhere": badges.iter().filter(|b| b.status_label == "owned_elsewhere").count(),
        },
        "counter_stream": options.stream_out.as_ref().map(|p| p.display().to_string()),
        "counter_gate": gate_json(&gate),
        "source_stream": options.source_stream.as_ref().map(|p| p.display().to_string()),
        "source_gate": source_gate.as_ref().map(gate_json),
        "badges": badges.iter().map(|badge| json!({
            "counter_id": badge.counter_id,
            "name": badge.name,
            "status": badge.status_label,
            "current_support": badge.current_support,
            "current_domain_id": badge.current_domain_id,
            "current_domain": if badge.current_domain_id == 0 { "none" } else { domain_name(badge.current_domain_id as u16) },
            "proposed_record_kind": badge.proposed_record_kind,
            "proposed_record": badge.proposed_record,
            "current_row_len": badge.current_row_len,
            "root_status": badge.root_status,
            "proof_status": badge.proof_status,
            "owner_domain_id": badge.owner_domain_id,
            "owner_domain": if badge.owner_domain_id == 0 { "none" } else { domain_name(badge.owner_domain_id as u16) },
            "exclusion_mask": badge.exclusion_mask,
            "evidence_mask": badge.evidence_mask,
            "current_compact_evidence": badge.current_compact_evidence,
            "source_evidence": badge.source_evidence,
            "fail_closed_badge": badge.fail_closed_badge,
            "owner_boundary": badge.owner_boundary,
        })).collect::<Vec<_>>(),
    });
    let report_bytes = serde_json::to_vec_pretty(&report).map_err(|err| err.to_string())?;
    if let Some(path) = options.out.as_ref() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| format!("create report dir failed: {err}"))?;
        }
        fs::write(path, &report_bytes).map_err(|err| format!("write report failed: {err}"))?;
    } else {
        println!("{}", String::from_utf8_lossy(&report_bytes));
    }

    if !gate.ok {
        return Err("counter stream gate failed; report was still written".to_string());
    }
    if let Some(source_gate) = source_gate {
        if !source_gate.ok {
            return Err("source stream gate failed; report was still written".to_string());
        }
    }
    Ok(())
}

fn counter_stream(rows: &[country_economy_counters_live_lane::CounterBadgeRow]) -> Vec<u8> {
    let mut raw = Vec::new();
    append_compact_record(
        &mut raw,
        1,
        0,
        DOMAIN_ID,
        RECORD_TYPE_DOMAIN_PERF,
        &perf_payload(rows.len() as u32, rows.len() as u32),
    );
    append_compact_record(
        &mut raw,
        2,
        0,
        DOMAIN_ID,
        RECORD_TYPE_DOMAIN_DATA,
        &encode_counter_payload(1, STATUS_LANE_LOCAL_PROOF, rows),
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

fn perf_payload(rows: u32, units: u32) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&0u64.to_le_bytes());
    payload.extend_from_slice(&1u64.to_le_bytes());
    payload.extend_from_slice(&rows.to_le_bytes());
    payload.extend_from_slice(&rows.to_le_bytes());
    payload.extend_from_slice(&units.to_le_bytes());
    payload.extend_from_slice(&units.to_le_bytes());
    payload.extend_from_slice(&STATUS_LANE_LOCAL_PROOF.to_le_bytes());
    payload
}

fn frame_payload(domains: u32) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&0u64.to_le_bytes());
    payload.extend_from_slice(&1u64.to_le_bytes());
    payload.extend_from_slice(&domains.to_le_bytes());
    payload.extend_from_slice(&domains.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&STATUS_LANE_LOCAL_PROOF.to_le_bytes());
    payload
}

fn gate_json(gate: &country_economy_counters_live_lane::CounterGate) -> serde_json::Value {
    json!({
        "ok": gate.ok,
        "expected_domains": gate.expected_domains.iter().map(|id| domain_name(*id)).collect::<Vec<_>>(),
        "total_bytes": gate.summary.total_bytes,
        "record_count": gate.summary.record_count,
        "domain_data_records": gate.summary.domain_data_records,
        "frame_end_records": gate.summary.frame_end_records,
        "unique_game_hours": gate.summary.unique_game_hours,
        "first_game_hour_raw": gate.summary.first_game_hour_raw,
        "last_game_hour_raw": gate.summary.last_game_hour_raw,
        "skipped_game_hours": gate.summary.skipped_game_hours,
        "status_flags_or": gate.summary.status_flags_or,
        "parse_errors": gate.summary.parse_errors,
        "ordering_errors": gate.summary.ordering_errors,
        "coverage_errors": gate.summary.coverage_errors,
        "per_domain": gate.summary.per_domain.iter().map(|(id, domain)| {
            (domain_name(*id).to_string(), json!({
                "domain_id": domain.domain_id,
                "perf_records": domain.perf_records,
                "data_records": domain.data_records,
                "row_count_total": domain.row_count_total,
                "changed_rows_total": domain.changed_rows_total,
                "unit_count_total": domain.unit_count_total,
                "status_flags_or": domain.status_flags_or,
            }))
        }).collect::<serde_json::Map<_, _>>(),
    })
}

struct Options {
    source_stream: Option<PathBuf>,
    stream_out: Option<PathBuf>,
    out: Option<PathBuf>,
}

impl Options {
    fn parse() -> Result<Self, String> {
        let args = env::args().skip(1).collect::<Vec<_>>();
        let mut source_stream = None;
        let mut stream_out = None;
        let mut out = None;
        let mut i = 0usize;
        while i < args.len() {
            match args[i].as_str() {
                "--source-stream" => {
                    i += 1;
                    source_stream = Some(PathBuf::from(required(args.get(i), "--source-stream")?));
                }
                "--stream-out" => {
                    i += 1;
                    stream_out = Some(PathBuf::from(required(args.get(i), "--stream-out")?));
                }
                "--out" => {
                    i += 1;
                    out = Some(PathBuf::from(required(args.get(i), "--out")?));
                }
                "--help" | "-h" => return Err(usage()),
                other => return Err(format!("unknown argument {other}\n{}", usage())),
            }
            i += 1;
        }
        Ok(Self {
            source_stream,
            stream_out,
            out,
        })
    }
}

fn required(value: Option<&String>, flag: &str) -> Result<String, String> {
    value
        .cloned()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn usage() -> String {
    "usage: counter_report [--source-stream collector_stream.bin] [--stream-out domain58_stream.bin] [--out report.json]".to_string()
}
