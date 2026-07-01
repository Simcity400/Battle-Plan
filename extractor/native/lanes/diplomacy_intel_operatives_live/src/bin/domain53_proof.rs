use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::PathBuf;

use diplomacy_intel_operatives_live::decoder_gate::{
    decode_domain53_record, encode_domain53_data_record,
};
use diplomacy_intel_operatives_live::extractor::{
    build_domain53_payload, proof_fixture_rows, CURRENT_BUILD_ROOTS_870EBF,
    CURRENT_VALIDATED_SOURCE_PROOFS, EXPECTED_HOI4_SHA256_870EBF,
};
use diplomacy_intel_operatives_live::replay_verify::write_replay_artifacts;
use diplomacy_intel_operatives_live::DOMAIN_ID;

fn main() {
    let mut out_record = PathBuf::from("domain53_validated_subset.compact.bin");
    let mut out_report = PathBuf::from("domain53_validated_subset_proof.json");
    let mut out_manifest = PathBuf::from("native_session_manifest.json");
    let mut out_replay = PathBuf::from("native_session.battleplan");
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out-record" => {
                out_record = PathBuf::from(args.next().expect("--out-record requires a path"));
            }
            "--out-report" => {
                out_report = PathBuf::from(args.next().expect("--out-report requires a path"));
            }
            "--out-manifest" => {
                out_manifest = PathBuf::from(args.next().expect("--out-manifest requires a path"));
            }
            "--out-replay" => {
                out_replay = PathBuf::from(args.next().expect("--out-replay requires a path"));
            }
            _ => panic!("unknown argument {arg}"),
        }
    }

    let extraction = build_domain53_payload(proof_fixture_rows(), CURRENT_VALIDATED_SOURCE_PROOFS);
    if !extraction.ok() {
        eprintln!("domain53 proof extraction failed: {:?}", extraction.errors);
        std::process::exit(1);
    }

    let record = encode_domain53_data_record(1, 43_800_024, &extraction.payload);
    let record_summary = decode_domain53_record(0, &record);
    if !record_summary.parse_errors.is_empty() {
        eprintln!(
            "domain53 record gate failed: {:?}",
            record_summary.parse_errors
        );
        std::process::exit(1);
    }

    if let Some(parent) = out_report.parent() {
        fs::create_dir_all(parent).expect("create report output dir");
    }
    let replay_verification =
        write_replay_artifacts(&out_record, &out_manifest, &out_replay, &record)
            .expect("write and verify replay artifacts");
    fs::write(
        &out_report,
        proof_json(
            &out_record,
            &out_manifest,
            &out_replay,
            record.len(),
            extraction.payload.len(),
            extraction.gate_summary.status_flags_or,
            extraction.gate_summary.data_rows_total,
            &extraction.gate_summary.data_units_total,
            replay_verification.replay_bytes,
            replay_verification.stream_bytes,
        ),
    )
    .expect("write proof report");

    println!(
        "domain53_proof: ok record={} replay={} report={} rows={} bytes={}",
        out_record.display(),
        out_replay.display(),
        out_report.display(),
        extraction.gate_summary.data_rows_total,
        record.len()
    );
}

fn proof_json(
    out_record: &PathBuf,
    out_manifest: &PathBuf,
    out_replay: &PathBuf,
    record_bytes: usize,
    payload_bytes: usize,
    status_flags_or: u32,
    data_rows_total: u64,
    units: &BTreeMap<&'static str, u64>,
    replay_bytes: usize,
    replay_stream_bytes: usize,
) -> String {
    let roots = CURRENT_BUILD_ROOTS_870EBF
        .iter()
        .map(|root| {
            format!(
                "    {{ \"name\": {}, \"evidence\": {} }}",
                json_string(root.name),
                json_string(root.evidence)
            )
        })
        .collect::<Vec<_>>()
        .join(",\n");
    format!(
        concat!(
            "{{\n",
            "  \"lane\": \"diplomacy_intel_operatives_live\",\n",
            "  \"domain_id\": {domain_id},\n",
            "  \"build_sha256\": {sha},\n",
            "  \"source_proofs\": \"0x{source_proofs:X}\",\n",
            "  \"compact_record\": {record_path},\n",
            "  \"session_manifest\": {manifest_path},\n",
            "  \"hoi4replay\": {replay_path},\n",
            "  \"record_bytes\": {record_bytes},\n",
            "  \"payload_bytes\": {payload_bytes},\n",
            "  \"status_flags_or\": \"0x{status_flags_or:X}\",\n",
            "  \"data_rows_total\": {data_rows_total},\n",
            "  \"data_units_total\": {units_json},\n",
            "  \"hoi4replay_verified\": true,\n",
            "  \"hoi4replay_bytes\": {replay_bytes},\n",
            "  \"hoi4replay_collector_stream_bytes\": {replay_stream_bytes},\n",
            "  \"fail_closed\": {{\n",
            "    \"war_root_unvalidated\": false,\n",
            "    \"war_lifecycle_detail_unvalidated\": true,\n",
            "    \"incoming_action_queue_unvalidated\": true,\n",
            "    \"operation_detail_resisted\": true,\n",
            "    \"intel_map_resisted\": true\n",
            "  }},\n",
            "  \"current_build_roots\": [\n",
            "{roots}\n",
            "  ],\n",
            "  \"parse_errors\": []\n",
            "}}\n"
        ),
        domain_id = DOMAIN_ID,
        sha = json_string(EXPECTED_HOI4_SHA256_870EBF),
        source_proofs = CURRENT_VALIDATED_SOURCE_PROOFS,
        record_path = json_string(&out_record.display().to_string()),
        manifest_path = json_string(&out_manifest.display().to_string()),
        replay_path = json_string(&out_replay.display().to_string()),
        record_bytes = record_bytes,
        payload_bytes = payload_bytes,
        status_flags_or = status_flags_or,
        data_rows_total = data_rows_total,
        units_json = units_json(units),
        replay_bytes = replay_bytes,
        replay_stream_bytes = replay_stream_bytes,
        roots = roots,
    )
}

fn units_json(units: &BTreeMap<&'static str, u64>) -> String {
    let parts = units
        .iter()
        .map(|(key, value)| format!("{}: {}", json_string(key), value))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{{{parts}}}")
}

fn json_string(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    format!("\"{escaped}\"")
}
