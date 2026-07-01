#![allow(dead_code)]

mod domain24_weather_sandstorm_v3;

use domain24_weather_sandstorm_v3::decode_payload;
use std::env;
use std::fs;
use std::process;

fn main() {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: weather_sandstorm_v3_gate <domain24_v3_payload.bin>");
        process::exit(2);
    };
    let payload = match fs::read(&path) {
        Ok(payload) => payload,
        Err(err) => {
            eprintln!("failed to read {path}: {err}");
            process::exit(2);
        }
    };
    let summary = decode_payload(0, &payload);
    println!("{{");
    println!("  \"payload\": \"{}\",", json_escape(&path));
    println!("  \"payload_bytes\": {},", payload.len());
    println!("  \"status_flags_or\": {},", summary.status_flags_or);
    println!("  \"data_rows_total\": {},", summary.data_rows_total);
    println!(
        "  \"data_changed_rows_total\": {},",
        summary.data_changed_rows_total
    );
    println!("  \"data_units_total\": {{");
    for (idx, (key, value)) in summary.data_units_total.iter().enumerate() {
        let comma = if idx + 1 == summary.data_units_total.len() {
            ""
        } else {
            ","
        };
        println!("    \"{}\": {}{}", json_escape(key), value, comma);
    }
    println!("  }},");
    println!("  \"parse_errors\": [");
    for (idx, error) in summary.parse_errors.iter().enumerate() {
        let comma = if idx + 1 == summary.parse_errors.len() {
            ""
        } else {
            ","
        };
        println!("    \"{}\"{}", json_escape(error), comma);
    }
    println!("  ]");
    println!("}}");
    if !summary.parse_errors.is_empty() {
        process::exit(1);
    }
    if summary
        .data_units_total
        .get("weather_region_sandstorm_rows")
        .copied()
        .unwrap_or(0)
        == 0
    {
        eprintln!("weather_region_sandstorm_rows was zero");
        process::exit(1);
    }
}

fn json_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out
}
