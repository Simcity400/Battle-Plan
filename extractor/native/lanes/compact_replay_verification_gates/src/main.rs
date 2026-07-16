use compact_replay_verification_gates::{
    validate_report_output_path, validate_run_dir, GateOptions,
};
use std::path::PathBuf;

fn main() {
    let mut options = GateOptions::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--run-dir" => {
                options.run_dir = PathBuf::from(expect_arg(&mut args, "--run-dir"));
            }
            "--require-domain" => {
                options
                    .required_domains
                    .push(expect_arg(&mut args, "--require-domain"));
            }
            "--min-game-hours" => {
                options.min_game_hours = Some(parse_i64(
                    &expect_arg(&mut args, "--min-game-hours"),
                    "--min-game-hours",
                ));
            }
            "--max-stream-bytes" => {
                options.max_stream_bytes = parse_u64(
                    &expect_arg(&mut args, "--max-stream-bytes"),
                    "--max-stream-bytes",
                );
            }
            "--max-record-bytes" => {
                options.max_record_bytes = parse_u64(
                    &expect_arg(&mut args, "--max-record-bytes"),
                    "--max-record-bytes",
                );
            }
            "--max-bytes-per-frame" => {
                options.max_bytes_per_frame = parse_u64(
                    &expect_arg(&mut args, "--max-bytes-per-frame"),
                    "--max-bytes-per-frame",
                );
            }
            "--write-report" => {
                options.write_report = Some(PathBuf::from(expect_arg(&mut args, "--write-report")));
            }
            "--skip-replay-verify" => options.require_replay_verify = false,
            "--allow-bridge-roots" => options.allow_bridge_roots = true,
            "--allow-partial-or-source-failure" => options.allow_partial_or_source_failure = true,
            "--no-strict-semantic-units" => options.strict_semantic_units = false,
            "--help" | "-h" => {
                print_usage();
                return;
            }
            other => {
                eprintln!("unknown argument: {other}");
                print_usage();
                std::process::exit(2);
            }
        }
    }

    if options.run_dir.as_os_str().is_empty() {
        eprintln!("missing --run-dir");
        print_usage();
        std::process::exit(2);
    }

    let mut report = validate_run_dir(&options);
    if let Err(error) = validate_report_output_path(&options) {
        report.passed = false;
        report.errors.push(error);
    }
    let report_json =
        serde_json::to_string_pretty(&report.as_json()).expect("gate report serializes");
    println!("{report_json}");

    if let Some(path) = &options.write_report {
        if let Err(err) = std::fs::write(path, report_json.as_bytes()) {
            eprintln!("write report {} failed: {err}", path.display());
            std::process::exit(1);
        }
    }

    if !report.passed {
        std::process::exit(1);
    }
}

fn expect_arg(args: &mut impl Iterator<Item = String>, flag: &str) -> String {
    args.next().unwrap_or_else(|| {
        eprintln!("{flag} needs a value");
        std::process::exit(2);
    })
}

fn parse_i64(raw: &str, flag: &str) -> i64 {
    raw.parse::<i64>().unwrap_or_else(|_| {
        eprintln!("{flag} needs an integer, got {raw:?}");
        std::process::exit(2);
    })
}

fn parse_u64(raw: &str, flag: &str) -> u64 {
    raw.parse::<u64>().unwrap_or_else(|_| {
        eprintln!("{flag} needs an unsigned integer, got {raw:?}");
        std::process::exit(2);
    })
}

fn print_usage() {
    eprintln!(
        "usage: compact_replay_gate --run-dir <native run dir> [options]\n\
         options:\n\
           --require-domain <name>          require a scheduled compact domain; repeatable\n\
           --min-game-hours <n>             required explicit positive certification duration\n\
           --max-stream-bytes <n>           default 134217728\n\
           --max-record-bytes <n>           default 8388608\n\
           --max-bytes-per-frame <n>        default 2097152\n\
           --write-report <path>            write the JSON gate report\n\
           --skip-replay-verify             diagnostic-only; forces passed=false\n\
           --allow-bridge-roots             report bridge/root debt as warning instead of failure\n\
           --allow-partial-or-source-failure allow only partial-data/source-unavailable status\n\
           --no-strict-semantic-units       do not require domain semantic counter keys"
    );
}
