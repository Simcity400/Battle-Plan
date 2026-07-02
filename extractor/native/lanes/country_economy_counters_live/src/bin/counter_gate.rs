use country_economy_counters_live_lane::{domain_name, gate_counter_stream};
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
    let raw = fs::read(&options.stream).map_err(|err| format!("read stream failed: {err}"))?;
    let gate = gate_counter_stream(&raw, options.min_game_hours);
    let proof = json!({
        "schema": "country_economy_counters_live.counter_gate.v1",
        "stream": options.stream.display().to_string(),
        "ok": gate.ok,
        "expected_domains": gate.expected_domains.iter().map(|id| domain_name(*id)).collect::<Vec<_>>(),
        "min_game_hours": options.min_game_hours,
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
    });
    let proof_bytes = serde_json::to_vec_pretty(&proof).map_err(|err| err.to_string())?;
    if let Some(path) = options.out {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| format!("create output dir failed: {err}"))?;
        }
        fs::write(path, &proof_bytes).map_err(|err| format!("write proof failed: {err}"))?;
    } else {
        println!("{}", String::from_utf8_lossy(&proof_bytes));
    }
    if gate.ok {
        Ok(())
    } else {
        Err("counter gate failed; proof was still written".to_string())
    }
}

struct Options {
    stream: PathBuf,
    out: Option<PathBuf>,
    min_game_hours: usize,
}

impl Options {
    fn parse() -> Result<Self, String> {
        let args = env::args().skip(1).collect::<Vec<_>>();
        let mut stream = None;
        let mut out = None;
        let mut min_game_hours = 1usize;
        let mut i = 0usize;
        while i < args.len() {
            match args[i].as_str() {
                "--stream" => {
                    i += 1;
                    stream = Some(PathBuf::from(required(args.get(i), "--stream")?));
                }
                "--out" => {
                    i += 1;
                    out = Some(PathBuf::from(required(args.get(i), "--out")?));
                }
                "--min-game-hours" => {
                    i += 1;
                    let raw = required(args.get(i), "--min-game-hours")?;
                    min_game_hours = raw
                        .parse::<usize>()
                        .map_err(|err| format!("invalid --min-game-hours {raw}: {err}"))?;
                }
                "--help" | "-h" => return Err(usage()),
                other => return Err(format!("unknown argument {other}\n{}", usage())),
            }
            i += 1;
        }
        Ok(Self {
            stream: stream.ok_or_else(usage)?,
            out,
            min_game_hours,
        })
    }
}

fn required(value: Option<&String>, flag: &str) -> Result<String, String> {
    value
        .cloned()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn usage() -> String {
    "usage: counter_gate --stream domain58_stream.bin [--out gate.json] [--min-game-hours 1]"
        .to_string()
}
