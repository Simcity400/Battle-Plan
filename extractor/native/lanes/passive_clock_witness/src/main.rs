//! Verdict tool for the passive capture harness.
//!
//! `Invoke-HeadlessRawCapture.ps1` appends every observation it takes to the
//! witness JSONL -- including the one that ends the run, which the previous
//! harness threw away by throwing before it wrote -- and then asks this tool
//! whether the run may continue. Exit code 0 means continue, 3 means stop, and
//! the reason is a single named failure on stderr.
//!
//! Keeping the decision here rather than in the harness is what makes it
//! testable: the workspace has no PowerShell test runner, and every guard in
//! this lane has to be provable by making a production change and watching a
//! named test go red.

use std::path::PathBuf;
use std::process::ExitCode;

use passive_clock_witness::{ClockSample, StallPolicy, WitnessRun};

const EXIT_CONTINUE: u8 = 0;
const EXIT_USAGE: u8 = 2;
const EXIT_FATAL: u8 = 3;

struct Args {
    witness: PathBuf,
    intervals_out: Option<PathBuf>,
    policy: StallPolicy,
}

fn parse_args() -> Result<Args, String> {
    let mut witness = None;
    let mut intervals_out = None;
    let mut policy = StallPolicy::default();
    let mut argv = std::env::args().skip(1);
    while let Some(flag) = argv.next() {
        let mut value = || {
            argv.next()
                .ok_or_else(|| format!("{flag} requires a value"))
        };
        match flag.as_str() {
            "--witness" => witness = Some(PathBuf::from(value()?)),
            "--intervals-out" => intervals_out = Some(PathBuf::from(value()?)),
            "--stall-samples" => {
                policy.min_consecutive_samples = value()?
                    .parse()
                    .map_err(|error| format!("--stall-samples: {error}"))?;
            }
            "--stall-span-ms" => {
                policy.min_span_ms = value()?
                    .parse()
                    .map_err(|error| format!("--stall-span-ms: {error}"))?;
            }
            other => return Err(format!("unrecognised argument '{other}'")),
        }
    }
    if policy.min_consecutive_samples == 0 || policy.min_span_ms <= 0 {
        return Err(
            "the stall escalation policy must require at least one sample over a positive span"
                .into(),
        );
    }
    Ok(Args {
        witness: witness.ok_or("--witness is required")?,
        intervals_out,
        policy,
    })
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(error) => {
            eprintln!("passive_clock_witness: {error}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    let text = match std::fs::read_to_string(&args.witness) {
        Ok(text) => text,
        Err(error) => {
            eprintln!(
                "passive_clock_witness: cannot read {}: {error}",
                args.witness.display()
            );
            return ExitCode::from(EXIT_USAGE);
        }
    };

    let mut run = WitnessRun::new(args.policy);
    let mut fatal = None;
    for (index, line) in text.lines().enumerate() {
        // PowerShell writes UTF-8 with a BOM; strip it rather than fail on the
        // first line of every run.
        let line = line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() {
            continue;
        }
        let sample: ClockSample = match serde_json::from_str(line) {
            Ok(sample) => sample,
            Err(error) => {
                eprintln!(
                    "passive_clock_witness: {}:{} is not an admissible sample: {error}",
                    args.witness.display(),
                    index + 1
                );
                return ExitCode::from(EXIT_USAGE);
            }
        };
        match run.admit(sample) {
            Ok(_) => {}
            Err(failure) => {
                fatal = Some(failure);
                break;
            }
        }
    }

    // The interval report is written whatever the verdict: a run that ended on
    // a failure is exactly the run whose paused/active split someone will want
    // to read.
    if let Some(path) = args.intervals_out.as_ref() {
        let report = run.interval_report();
        match serde_json::to_string_pretty(&report) {
            Ok(json) => {
                if let Err(error) = std::fs::write(path, json) {
                    eprintln!(
                        "passive_clock_witness: cannot write {}: {error}",
                        path.display()
                    );
                    return ExitCode::from(EXIT_USAGE);
                }
            }
            Err(error) => {
                eprintln!("passive_clock_witness: cannot serialise the interval report: {error}");
                return ExitCode::from(EXIT_USAGE);
            }
        }
    }

    match fatal {
        Some(failure) => {
            eprintln!("passive_clock_witness: {failure}");
            ExitCode::from(EXIT_FATAL)
        }
        None => {
            let report = run.interval_report();
            println!(
                "phase=passive_witness_verdict samples={} measured_advancing_ms={} \
                 measured_advancing_with_hold_ms={} measured_paused_by_engine_ms={} measured_paused_unforced_ms={} \
                 measured_tick_inhibited_ms={} measured_stalled_ms={} \
                 unobserved_ms={} wall_clock_ms={} observed_game_hours={}",
                report.samples,
                report.advancing_ms,
                report.advancing_with_hold_ms,
                report.paused_by_engine_ms,
                report.paused_unforced_ms,
                report.tick_inhibited_ms,
                report.stalled_ms,
                report.unobserved_ms,
                report.wall_clock_ms,
                report.observed_game_hours
            );
            ExitCode::from(EXIT_CONTINUE)
        }
    }
}
