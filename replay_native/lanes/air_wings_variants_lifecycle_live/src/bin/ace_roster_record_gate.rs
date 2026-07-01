use air_wings_variants_lifecycle_live_lane::domain46_air_ace_roster_live as compact;
use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let mut args = env::args().skip(1);
    let record = match args.next() {
        Some(value) => PathBuf::from(value),
        None => {
            eprintln!("usage: ace_roster_record_gate <air_ace_roster_record.bin> [out.json]");
            std::process::exit(2);
        }
    };
    let out = args.next().map(PathBuf::from);
    let bytes = fs::read(&record).unwrap_or_else(|err| {
        eprintln!("read {} failed: {err}", record.display());
        std::process::exit(2);
    });
    let summary = compact::decode_compact_data_record(0, &bytes);
    let json = summary_json(&record, bytes.len(), &summary);
    if let Some(path) = out {
        fs::write(&path, json.as_bytes()).unwrap_or_else(|err| {
            eprintln!("write {} failed: {err}", path.display());
            std::process::exit(2);
        });
    }
    println!("{json}");
    if !summary.parse_errors.is_empty() {
        std::process::exit(1);
    }
}

fn summary_json(
    record: &PathBuf,
    record_bytes: usize,
    summary: &compact::AirAceRosterRecordGateSummary,
) -> String {
    let mut units = String::new();
    for (index, (key, value)) in summary.data_units_total.iter().enumerate() {
        if index > 0 {
            units.push_str(",\n");
        }
        units.push_str(&format!("    \"{}\": {}", json_escape(key), value));
    }
    let errors = summary
        .parse_errors
        .iter()
        .map(|err| format!("\"{}\"", json_escape(err)))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        concat!(
            "{{\n",
            "  \"schema\": \"air_variant_archetypes_live.ace_roster_record_gate.v1\",\n",
            "  \"domain_id\": {},\n",
            "  \"record\": \"{}\",\n",
            "  \"record_bytes\": {},\n",
            "  \"record_count\": {},\n",
            "  \"payload_bytes_total\": {},\n",
            "  \"status_flags_or\": {},\n",
            "  \"data_rows_total\": {},\n",
            "  \"data_changed_rows_total\": {},\n",
            "  \"data_units_total\": {{\n{}\n  }},\n",
            "  \"parse_errors\": [{}]\n",
            "}}"
        ),
        compact::DOMAIN_ID,
        json_escape(&record.display().to_string()),
        record_bytes,
        summary.record_count,
        summary.payload_bytes_total,
        summary.status_flags_or,
        summary.data_rows_total,
        summary.data_changed_rows_total,
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
