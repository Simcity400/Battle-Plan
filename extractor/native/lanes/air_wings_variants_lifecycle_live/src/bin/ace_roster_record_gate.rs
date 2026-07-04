//! Offline gate for the domain-46 `ACE1` air-ace-roster trailer.
//!
//! Input: a file holding either raw `ACE1` trailer bytes (starts with the
//! magic), or a full domain-46 data-record PAYLOAD (base
//! air-variant-archetypes payload with the trailer appended).
//!
//! For a full payload the tool delegates to the base `decode_payload`, which
//! decodes the self-delimiting base payload to its EXACT computed cursor and
//! then decodes the trailer there. A false-`ACE1` byte sequence inside the
//! base payload can therefore never be mistaken for the trailer offset (Codex
//! NOTE: no magic-scan false negatives). The magic prefix is only used to
//! recognise a raw-trailer file.

use air_wings_variants_lifecycle_live_lane::domain46_air_ace_roster_live as compact;
use air_wings_variants_lifecycle_live_lane::domain46_air_variant_archetypes_live as archetypes;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let mut args = env::args().skip(1);
    let record = match args.next() {
        Some(value) => PathBuf::from(value),
        None => {
            eprintln!(
                "usage: ace_roster_record_gate <ace_trailer_or_domain46_payload.bin> [out.json]"
            );
            std::process::exit(2);
        }
    };
    let out = args.next().map(PathBuf::from);
    let bytes = fs::read(&record).unwrap_or_else(|err| {
        eprintln!("read {} failed: {err}", record.display());
        std::process::exit(2);
    });

    let (parse_errors, units, status_flags_or, data_rows_total) =
        if bytes.starts_with(compact::ACE_TRAILER_MAGIC) {
            // Raw-trailer file: decode from offset 0.
            let summary = compact::decode_trailer(0, &bytes, 0);
            (
                summary.parse_errors,
                summary.data_units_total,
                summary.status_flags_or,
                summary.data_rows_total,
            )
        } else {
            // Full domain-46 payload: the base decoder locates the trailer at
            // its exact computed cursor and validates both slices.
            let summary = archetypes::decode_payload(0, &bytes);
            let data_rows_total = summary
                .data_units_total
                .get("air_ace_roster_rows")
                .copied()
                .unwrap_or(0);
            (
                summary.parse_errors,
                summary.data_units_total,
                summary.status_flags_or,
                data_rows_total,
            )
        };

    let json = summary_json(
        &record,
        bytes.len(),
        status_flags_or,
        data_rows_total,
        &units,
        &parse_errors,
    );
    if let Some(path) = out {
        fs::write(&path, json.as_bytes()).unwrap_or_else(|err| {
            eprintln!("write {} failed: {err}", path.display());
            std::process::exit(2);
        });
    }
    println!("{json}");
    if !parse_errors.is_empty() {
        std::process::exit(1);
    }
}

fn summary_json(
    record: &PathBuf,
    record_bytes: usize,
    status_flags_or: u32,
    data_rows_total: u64,
    data_units_total: &BTreeMap<&'static str, u64>,
    parse_errors: &[String],
) -> String {
    let mut units = String::new();
    for (index, (key, value)) in data_units_total.iter().enumerate() {
        if index > 0 {
            units.push_str(",\n");
        }
        units.push_str(&format!("    \"{}\": {}", json_escape(key), value));
    }
    let errors = parse_errors
        .iter()
        .map(|err| format!("\"{}\"", json_escape(err)))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        concat!(
            "{{\n",
            "  \"schema\": \"air_variant_archetypes_live.ace_roster_record_gate.v2\",\n",
            "  \"record\": \"{}\",\n",
            "  \"record_bytes\": {},\n",
            "  \"status_flags_or\": {},\n",
            "  \"data_rows_total\": {},\n",
            "  \"data_units_total\": {{\n{}\n  }},\n",
            "  \"parse_errors\": [{}]\n",
            "}}"
        ),
        json_escape(&record.display().to_string()),
        record_bytes,
        status_flags_or,
        data_rows_total,
        units,
        errors
    )
}

fn json_escape(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}
