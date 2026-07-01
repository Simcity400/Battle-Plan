use market_trade_live_lane::gate_compact_stream;
use std::env;
use std::fs;
use std::process;

fn main() {
    let mut args = env::args().skip(1);
    let mut stream_path = None;
    let mut min_game_hours = 1usize;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--stream" => stream_path = args.next(),
            "--min-game-hours" => {
                let Some(value) = args.next() else {
                    eprintln!("--min-game-hours requires a value");
                    process::exit(2);
                };
                min_game_hours = value.parse().unwrap_or_else(|err| {
                    eprintln!("invalid --min-game-hours {value:?}: {err}");
                    process::exit(2);
                });
            }
            "-h" | "--help" => {
                print_usage();
                return;
            }
            other => {
                eprintln!("unknown argument {other:?}");
                print_usage();
                process::exit(2);
            }
        }
    }
    let Some(stream_path) = stream_path else {
        print_usage();
        process::exit(2);
    };
    let raw = fs::read(&stream_path).unwrap_or_else(|err| {
        eprintln!("cannot read {stream_path}: {err}");
        process::exit(2);
    });
    let gate = gate_compact_stream(&raw, min_game_hours);
    println!("ok={}", gate.ok);
    println!("problems={:?}", gate.problems);
    println!("domain_data_records={}", gate.summary.domain_data_records);
    println!("unique_game_hours={}", gate.summary.unique_game_hours);
    println!(
        "skipped_game_hours={}",
        gate.summary.skipped_game_hours.len()
    );
    println!("row_count_total={}", gate.summary.row_count_total);
    println!(
        "observed_contract_rows_total={}",
        gate.summary.observed_contract_rows_total
    );
    println!(
        "omitted_candidate_rows_total={}",
        gate.summary.omitted_candidate_rows_total
    );
    println!(
        "partial_field_rows_total={}",
        gate.summary.partial_field_rows_total
    );
    println!("parse_errors={:?}", gate.summary.parse_errors);
    process::exit(if gate.ok { 0 } else { 1 });
}

fn print_usage() {
    eprintln!("usage: market_trade_gate --stream <collector_stream.bin> [--min-game-hours <n>]");
}
