//! Passive, temporary Milestone 1.1 launcher and clock baseline measurement.
use crate::windows::{self, Event, Process};
use engine::{PINNED_BUILD, build::hash_executable, cgame_state};
use serde_json::json;
use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler};

static STOP: AtomicBool = AtomicBool::new(false);
unsafe extern "system" fn control(kind: u32) -> i32 {
    if kind == CTRL_C_EVENT || kind == CTRL_BREAK_EVENT {
        STOP.store(true, Ordering::Relaxed);
        1
    } else {
        0
    }
}

pub fn command(args: Vec<OsString>) -> Result<(), String> {
    let Some(command) = args.first().and_then(|s| s.to_str()) else {
        return Err(usage());
    };
    if command == "probe-inspect" && args.len() == 2 {
        return inspect(Path::new(&args[1]));
    }
    if command != "probe" && command != "probe-baseline" {
        return Err(usage());
    }
    let mut output = None;
    let mut site = "tick".to_string();
    let mut seconds = 120_u64;
    for pair in args[1..].chunks(2) {
        if pair.len() != 2 {
            return Err(usage());
        }
        match pair[0].to_str() {
            Some("--out") => output = Some(PathBuf::from(&pair[1])),
            Some("--site") if command == "probe" => {
                site = pair[1].to_str().ok_or("site must be text")?.to_string()
            }
            Some("--seconds") => {
                seconds = pair[1]
                    .to_str()
                    .and_then(|v| v.parse().ok())
                    .filter(|&v| v > 0)
                    .ok_or("seconds must be positive")?
            }
            _ => return Err(usage()),
        }
    }
    if !["tick", "daily", "country_events"].contains(&site.as_str()) {
        return Err("site must be tick, daily, or country_events".into());
    }
    let output = output.ok_or_else(usage)?;
    fs::create_dir_all(&output).map_err(|e| e.to_string())?;
    let output = output.canonicalize().map_err(|e| e.to_string())?;
    unsafe {
        SetConsoleCtrlHandler(Some(control), 1);
    }
    println!("Waiting for one hoi4.exe process. Probe duration: {seconds}s.");
    let pid = loop {
        match windows::games()?.as_slice() {
            [pid] => break *pid,
            [] => {
                if STOP.load(Ordering::Relaxed) {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_secs(2));
            }
            _ => {
                return Err(
                    "two hoi4.exe processes are unsupported; no injection attempted".into(),
                );
            }
        }
    };
    let process = Process::open(pid, command == "probe")?;
    let identity = hash_executable(File::open(&process.path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if !PINNED_BUILD.matches(&identity) {
        return Err(format!(
            "unsupported build: wanted {} / {} bytes / {:02x?}; got {} bytes / {:02x?}",
            PINNED_BUILD.version,
            PINNED_BUILD.exe_size,
            PINNED_BUILD.exe_sha256,
            identity.size,
            identity.sha256
        ));
    }
    if windows::modules(pid)?.iter().any(|m| {
        m.path
            .file_name()
            .is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case("collector.dll"))
    }) {
        return Err("collector.dll is already loaded; detach that recorder first".into());
    }
    if command == "probe-baseline" {
        return baseline(&process, &output, seconds);
    }
    let session = format!(
        "{}-{}",
        pid,
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos()
    );
    let directory = output.join(&session);
    let bin = directory.join("bin");
    fs::create_dir_all(&bin).map_err(|e| e.to_string())?;
    let source = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .with_file_name("collector.dll");
    let dll = bin.join("collector.dll");
    fs::copy(&source, &dll).map_err(|e| format!("copy {}: {e}", source.display()))?;
    let dll_hash =
        hash_executable(File::open(&dll).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    fs::write(
        bin.join("collector.json"),
        serde_json::to_vec_pretty(
            &json!({"session_id":session, "output_dir":directory, "site":site}),
        )
        .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::write(directory.join("launch.json"), serde_json::to_vec_pretty(&json!({"pid":pid, "process_start_time":process.start_time, "site":site, "collector_sha256":format!("{:02x?}", dll_hash.sha256), "recorder_version":env!("CARGO_PKG_VERSION"), "requested_seconds":seconds})).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let prefix = format!("Local\\HOI4Replay.Probe.{session}");
    let ready = Event::new(&format!("{prefix}.ready"))?;
    let detach = Event::new(&format!("{prefix}.detach"))?;
    let done = Event::new(&format!("{prefix}.done"))?;
    struct DetachOnDrop<'a>(&'a Event, PathBuf);
    impl Drop for DetachOnDrop<'_> {
        fn drop(&mut self) {
            let _ = fs::write(&self.1, b"cancelled\n");
            let _ = self.0.set();
        }
    }
    let _detach_on_error = DetachOnDrop(&detach, bin.join("collector.cancel"));
    process.inject(&dll)?;
    let error_file = bin.join("collector.error.txt");
    let deadline = Instant::now() + Duration::from_secs(30);
    while !ready.handle.signaled()? {
        if error_file.exists() {
            return Err(fs::read_to_string(&error_file).unwrap_or_else(|e| e.to_string()));
        }
        if process.handle.signaled()? {
            return Err("game exited before probe became ready".into());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "probe readiness timed out; detach requested; inspect {}",
                directory.display()
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    println!("Probe attached at {site}; logging {}", directory.display());
    let started = Instant::now();
    let mut status = Instant::now();
    while started.elapsed() < Duration::from_secs(seconds) && !STOP.load(Ordering::Relaxed) {
        if process.handle.signaled()? {
            println!("Game exited; probe prefix retained.");
            return inspect(&directory.join("probe.jsonl"));
        }
        if windows::games()?.len() > 1 {
            return Err("second hoi4.exe process detected; detaching the probe".into());
        }
        if error_file.exists() {
            return Err(fs::read_to_string(&error_file).unwrap_or_else(|e| e.to_string()));
        }
        if status.elapsed() >= Duration::from_secs(60) {
            inspect(&directory.join("probe.jsonl"))?;
            status = Instant::now();
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    detach.set()?;
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done.handle.signaled()? {
        if process.handle.signaled()? {
            return inspect(&directory.join("probe.jsonl"));
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "detach has not completed; collector remains loaded; inspect {}",
                error_file.display()
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    while !process.handle.signaled()? && process.has_module(&dll)? {
        if Instant::now() >= deadline {
            return Err("detach event arrived but collector.dll is still mapped".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if error_file.exists() {
        return Err(fs::read_to_string(error_file).unwrap_or_else(|e| e.to_string()));
    }
    println!("Original bytes restored; collector.dll confirmed absent.");
    inspect(&directory.join("probe.jsonl"))
}

fn usage() -> String {
    "Milestone 1.1 only:\n  recorder probe --out <directory> [--site tick|daily|country_events] [--seconds 120]\n  recorder probe-baseline --out <directory> [--seconds 120]\n  recorder probe-inspect <probe.jsonl>".into()
}

fn baseline(process: &Process, output: &Path, seconds: u64) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output.join(format!(
                "baseline-{}.jsonl",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|e| e.to_string())?
                    .as_nanos()
            )))
        .map_err(|e| e.to_string())?;
    let start = Instant::now();
    println!("Free-run clock observation started; no DLL is loaded.");
    while start.elapsed() < Duration::from_secs(seconds) && !STOP.load(Ordering::Relaxed) {
        if process.handle.signaled()? {
            break;
        }
        if windows::games()?.len() > 1 {
            return Err("second hoi4.exe process detected".into());
        }
        // External observations measure free-run speed only. They do not certify tables.
        let first = cgame_state::read_clock(process, process.base);
        let second = cgame_state::read_clock(process, process.base);
        let value = if first == second {
            first.map(|c| json!({"hour":c.hour,"game_state":c.game_state,"play_time":c.play_time,"play_session_started_at":c.play_session_started_at,"campaign_id":c.campaign_id.as_str(),"speed":c.speed}))
        } else {
            None
        };
        writeln!(file, "{}", json!({"elapsed_s":start.elapsed().as_secs_f64(),"clock":value,"pid":process.pid,"process_start_time":process.start_time})).map_err(|e| e.to_string())?;
        std::thread::sleep(Duration::from_millis(100));
    }
    file.sync_all().map_err(|e| e.to_string())?;
    println!(
        "Free-run clock observations written to {}",
        output.display()
    );
    Ok(())
}

fn inspect(path: &Path) -> Result<(), String> {
    let input = BufReader::new(File::open(path).map_err(|e| e.to_string())?);
    let report = summarize_probe(input)?;
    let hours = report
        .hour_range
        .map_or_else(|| "n/a".into(), |(a, b)| format!("{a}..{b}"));
    println!(
        "calls={} hours={hours} threads={:?}; dropped_calls={} invalid_lines={} incomplete_tail={}",
        report.calls, report.threads, report.dropped, report.invalid_lines, report.incomplete_tail
    );
    println!(
        "continuous_pairs={} elapsed_s={:.3} hours_per_s={}; discontinuities={} identity_changes={} invalid_qpc_pairs={}",
        report.continuous_pairs,
        report.continuous_seconds,
        report
            .hours_per_second()
            .map_or_else(|| "n/a".into(), |v| format!("{v:.3}")),
        report.discontinuities,
        report.identity_changes,
        report.invalid_qpc_pairs
    );
    println!(
        "Throughput includes pauses between consecutive hours; discontinuities are observations, not acceptance failures."
    );
    println!(
        "tick_step_checks={} violations={} unavailable={}; walk_clock_changes={} before_unavailable={} after_unavailable={}",
        report.tick_step_checks,
        report.tick_step_violations,
        report.tick_step_unavailable,
        report.walk_clock_changes,
        report.before_unavailable,
        report.after_unavailable
    );
    print_cost("total", &report.costs[0], report.calls);
    for (index, label) in ["clock.entry", "clock.before_walk", "clock.after_walk"]
        .iter()
        .enumerate()
    {
        print_cost(label, &report.costs[index + 5], report.calls);
    }
    for (index, label) in [
        "provinces.first",
        "countries.first",
        "provinces.second",
        "countries.second",
    ]
    .iter()
    .enumerate()
    {
        let table = &report.tables[index];
        print_cost(label, &report.costs[index + 1], report.calls);
        let rows = table
            .row_range
            .map_or_else(|| "n/a".into(), |(a, b)| format!("{a}..{b}"));
        println!(
            "  {label}: rows={rows} complete={} truncated={} unavailable={} missing_outcome={} max_unread_rows={}",
            table.complete,
            table.truncated,
            table.unavailable,
            table.missing,
            table.max_unread_rows
        );
    }
    println!(
        "double_read_mismatches: provinces={} countries={}",
        report.mismatches[0], report.mismatches[1]
    );
    println!("{}", path.display());
    Ok(())
}

fn print_cost(label: &str, costs: &[f64], calls: u64) {
    let values = cost_percentiles(costs).map_or_else(
        || "n/a".into(),
        |[p50, p95, p99, max]| format!("p50={p50:.3} p95={p95:.3} p99={p99:.3} max={max:.3}"),
    );
    println!(
        "{label}_ms {values}; samples={} missing={}",
        costs.len(),
        calls.saturating_sub(costs.len() as u64)
    );
}

fn cost_percentiles(sorted: &[f64]) -> Option<[f64; 4]> {
    if sorted.is_empty() {
        return None;
    }
    Some([50, 95, 99, 100].map(|p| sorted[(sorted.len() * p).div_ceil(100) - 1]))
}

#[derive(Default)]
struct TableSummary {
    row_range: Option<(u64, u64)>,
    complete: u64,
    truncated: u64,
    unavailable: u64,
    missing: u64,
    max_unread_rows: u64,
}

#[derive(Default)]
struct ProbeReport {
    calls: u64,
    hour_range: Option<(i64, i64)>,
    threads: std::collections::BTreeSet<u64>,
    costs: [Vec<f64>; 8],
    tables: [TableSummary; 4],
    mismatches: [u64; 2],
    dropped: u64,
    invalid_lines: u64,
    incomplete_tail: u64,
    continuous_pairs: u64,
    continuous_seconds: f64,
    discontinuities: u64,
    identity_changes: u64,
    invalid_qpc_pairs: u64,
    tick_step_checks: u64,
    tick_step_violations: u64,
    tick_step_unavailable: u64,
    before_unavailable: u64,
    after_unavailable: u64,
    walk_clock_changes: u64,
}

impl ProbeReport {
    fn hours_per_second(&self) -> Option<f64> {
        (self.continuous_seconds > 0.0)
            .then(|| self.continuous_pairs as f64 / self.continuous_seconds)
    }
}

fn extend_range<T: Ord + Copy>(range: &mut Option<(T, T)>, value: T) {
    *range = Some(range.map_or((value, value), |(a, b)| (a.min(value), b.max(value))));
}

fn clock_hour(clock: &serde_json::Value) -> Option<i64> {
    clock["hour"]
        .as_i64()
        .filter(|&hour| i32::try_from(hour).is_ok())
}

fn same_identity(first: &serde_json::Value, second: &serde_json::Value) -> bool {
    [
        "game_state",
        "play_session_started_at",
        "seed",
        "campaign_id",
        "campaign_bytes",
    ]
    .iter()
    .all(|&field| first[field] == second[field])
}

/// Summarize observations only. Missing records break continuity; no absent value
/// is replaced by a zero, and no discontinuity is classified as a proven reload.
fn summarize_probe(mut input: impl BufRead) -> Result<ProbeReport, String> {
    let mut report = ProbeReport::default();
    let mut bytes = Vec::new();
    let mut frequency = None;
    let mut tick_site = false;
    let mut previous: Option<(serde_json::Value, Option<i64>, u64)> = None;
    loop {
        bytes.clear();
        if input
            .read_until(b'\n', &mut bytes)
            .map_err(|e| e.to_string())?
            == 0
        {
            break;
        }
        let value: serde_json::Value = match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(error) => {
                if !bytes.ends_with(b"\n") && error.is_eof() {
                    report.incomplete_tail += 1;
                } else {
                    report.invalid_lines += 1;
                }
                previous = None;
                continue;
            }
        };
        if value["record"] == "start" {
            frequency = value["qpc_frequency"].as_i64().filter(|&v| v > 0);
            tick_site = value["site_rva"].as_u64() == Some(cgame_state::TICK_CANDIDATE_RVA as u64);
            previous = None;
            continue;
        }
        report.dropped = report
            .dropped
            .max(value["dropped_calls"].as_u64().unwrap_or(0));
        if value["record"] != "call" {
            // Restoration can be logged while the last callback is still active;
            // its completed frame follows this control record during final drain.
            if value["record"] != "restored" && value["record"] != "roster" {
                previous = None;
            }
            continue;
        }
        report.calls += 1;
        if let Some(thread) = value["thread_id"].as_u64() {
            report.threads.insert(thread);
        }
        for index in 0..8 {
            let cost = match index {
                0 => &value["total_ms"],
                1..=4 => &value["table_ms"][index - 1],
                _ => &value["clock_ms"][index - 5],
            };
            if let Some(cost) = cost.as_f64().filter(|v| v.is_finite() && *v >= 0.0) {
                report.costs[index].push(cost);
            }
        }
        for (index, table) in report.tables.iter_mut().enumerate() {
            let name = if index % 2 == 0 {
                "provinces"
            } else {
                "countries"
            };
            let outcome = &value[name][index / 2];
            match outcome["kind"].as_str() {
                Some("rows") => table.complete += 1,
                Some("truncated") => table.truncated += 1,
                Some("unavailable") => table.unavailable += 1,
                _ => table.missing += 1,
            }
            if let Some(rows) = outcome["rows"].as_u64() {
                extend_range(&mut table.row_range, rows);
            }
            table.max_unread_rows = table
                .max_unread_rows
                .max(outcome["unread_rows"].as_u64().unwrap_or(0));
        }
        report.mismatches[0] += value["province_mismatches"].as_u64().unwrap_or(0);
        report.mismatches[1] += value["country_mismatches"].as_u64().unwrap_or(0);
        let before = &value["before_walk"];
        let hour = clock_hour(before);
        let after_hour = clock_hour(&value["after_walk"]);
        report.before_unavailable += u64::from(hour.is_none());
        report.after_unavailable += u64::from(after_hour.is_none());
        if hour.is_some() && after_hour.is_some() && before != &value["after_walk"] {
            report.walk_clock_changes += 1;
        }
        if tick_site {
            if let (Some(entry), Some(hour)) = (clock_hour(&value["entry"]), hour) {
                report.tick_step_checks += 1;
                report.tick_step_violations += u64::from(entry.checked_add(1) != Some(hour));
            } else {
                report.tick_step_unavailable += 1;
            }
        }
        let Some(hour) = hour else {
            previous = None;
            continue;
        };
        extend_range(&mut report.hour_range, hour);
        let qpc = value["qpc"].as_i64();
        let dropped = value["dropped_calls"].as_u64().unwrap_or(0);
        if let Some((prior, prior_qpc, prior_dropped)) = &previous {
            let adjacent = clock_hour(prior).and_then(|v| v.checked_add(1)) == Some(hour);
            let same_world = same_identity(prior, before);
            report.discontinuities += u64::from(!adjacent);
            report.identity_changes += u64::from(!same_world);
            if adjacent && same_world && *prior_dropped == dropped {
                match (frequency, *prior_qpc, qpc) {
                    (Some(frequency), Some(first), Some(last)) if last > first => {
                        report.continuous_pairs += 1;
                        report.continuous_seconds +=
                            (i128::from(last) - i128::from(first)) as f64 / frequency as f64;
                    }
                    _ => report.invalid_qpc_pairs += 1,
                }
            }
        }
        previous = Some((before.clone(), qpc, dropped));
    }
    for costs in &mut report.costs {
        costs.sort_by(f64::total_cmp);
    }
    Ok(report)
}

#[cfg(test)]
mod report_tests {
    use super::*;

    fn start() -> serde_json::Value {
        json!({"record":"start", "site_rva":cgame_state::TICK_CANDIDATE_RVA, "qpc_frequency":1000})
    }

    fn call(hour: i32, qpc: i64) -> serde_json::Value {
        let clock = |hour| json!({"hour":hour,"game_state":123,"play_time":0,"play_session_started_at":1,"seed":9,"campaign_id":"campaign"});
        json!({"record":"call", "entry":clock(hour-1), "before_walk":clock(hour), "after_walk":clock(hour), "qpc":qpc, "thread_id":42, "dropped_calls":0,
            "total_ms":3.5, "table_ms":[0.1,0.2,0.3,0.4],
            "provinces":[{"kind":"rows","rows":10},{"kind":"rows","rows":10}],
            "countries":[{"kind":"rows","rows":2},{"kind":"rows","rows":2}]})
    }

    fn summary(records: &[serde_json::Value], tail: &str) -> ProbeReport {
        let mut bytes = records.iter().map(|v| format!("{v}\n")).collect::<String>();
        bytes.push_str(tail);
        summarize_probe(bytes.as_bytes()).unwrap()
    }

    #[test]
    fn reloads_and_dropped_hours_do_not_inflate_continuous_throughput() {
        let mut loaded_next = call(13, 9000);
        loaded_next["before_walk"]["play_session_started_at"] = json!(2);
        loaded_next["after_walk"]["play_session_started_at"] = json!(2);
        let mut dropped = call(15, 9100);
        dropped["dropped_calls"] = json!(1);
        let records = [
            start(),
            call(10, 0),
            call(11, 100),
            call(11, 200),
            call(5, 400),
            call(12, 500),
            loaded_next,
            dropped,
        ];
        let report = summary(&records, "");
        assert_eq!(report.calls, 7);
        assert_eq!(report.hour_range, Some((5, 15)));
        assert_eq!(report.continuous_pairs, 1);
        assert_eq!(report.hours_per_second(), Some(10.0));
        assert_eq!(report.discontinuities, 4);
        assert_eq!(report.identity_changes, 2);
        assert_eq!(report.tick_step_violations, 0);
        assert_eq!(report.dropped, 1);
    }

    #[test]
    fn absent_clocks_and_malformed_lines_break_continuity_and_remain_visible() {
        let mut missing = call(11, 100);
        missing["before_walk"] = serde_json::Value::Null;
        missing["after_walk"] = serde_json::Value::Null;
        missing["total_ms"] = serde_json::Value::Null;
        let mut changed = call(13, 300);
        changed["after_walk"]["hour"] = json!(14);
        changed["entry"]["hour"] = json!(11);
        changed["countries"][1] = json!({"kind":"unavailable","reason":"bad pointer"});
        changed["provinces"][0] = json!({"kind":"truncated","rows":7,"unread_rows":3});
        let report = summary(
            &[start(), call(10, 0), missing, call(12, 200), changed],
            "bad json\n{\"record\":",
        );
        assert_eq!(report.calls, 4);
        assert_eq!(report.continuous_pairs, 1);
        assert_eq!(report.discontinuities, 0);
        assert_eq!(report.before_unavailable, 1);
        assert_eq!(report.after_unavailable, 1);
        assert_eq!(report.walk_clock_changes, 1);
        assert_eq!(report.tick_step_unavailable, 1);
        assert_eq!(report.tick_step_violations, 1);
        assert_eq!(report.invalid_lines, 1);
        assert_eq!(report.incomplete_tail, 1);
        assert_eq!(report.costs[0].len(), 3);
        assert_eq!(report.tables[0].row_range, Some((7, 10)));
        assert_eq!(report.tables[0].max_unread_rows, 3);
        assert_eq!(report.tables[3].unavailable, 1);
        let tail = format!("bad json\n{}\n", call(11, 100));
        let broken = summary(&[start(), call(10, 0)], &tail);
        assert_eq!(broken.calls, 2);
        assert_eq!(broken.continuous_pairs, 0);
        assert_eq!(broken.invalid_lines, 1);
    }

    #[test]
    fn clock_and_cost_statistics_do_not_invent_zero_samples() {
        let mut absent_qpc = call(2, 100);
        absent_qpc["qpc"] = serde_json::Value::Null;
        let report = summary(
            &[start(), call(1, 100), absent_qpc, call(3, 100), call(4, 90)],
            "",
        );
        assert_eq!(report.continuous_pairs, 0);
        assert_eq!(report.invalid_qpc_pairs, 3);
        assert_eq!(report.hours_per_second(), None);
        assert_eq!(cost_percentiles(&[]), None);
        let samples = (1..=100).map(f64::from).collect::<Vec<_>>();
        assert_eq!(cost_percentiles(&samples), Some([50.0, 95.0, 99.0, 100.0]));
        assert_eq!(report.threads.into_iter().collect::<Vec<_>>(), vec![42]);
    }

    #[test]
    fn clock_timings_preserve_missing_and_partial_observations() {
        let untimed = call(1, 0);
        let mut timed = call(2, 100);
        timed["clock_ms"] = json!([0.01, 0.02, 0.03]);
        let mut partial = call(3, 200);
        partial["clock_ms"] = json!([0.04, null, 0.0]);
        let report = summary(&[start(), untimed, timed, partial], "");
        assert_eq!(report.calls, 3);
        assert_eq!(report.costs[5], vec![0.01, 0.04]);
        assert_eq!(report.costs[6], vec![0.02]);
        assert_eq!(report.costs[7], vec![0.0, 0.03]);
        let missing = summary(&[start(), call(1, 0)], "");
        for costs in &missing.costs[5..] {
            assert_eq!(cost_percentiles(costs), None);
        }
    }

    #[test]
    fn restoration_before_final_drain_preserves_the_last_hour_pair() {
        let mut last = call(11, 100);
        last["before_walk"]["play_time"] = json!(u64::MAX);
        last["after_walk"]["play_time"] = json!(u64::MAX);
        let report = summary(
            &[
                start(),
                call(10, 0),
                json!({"record":"restored","active_callbacks":1}),
                last,
                json!({"record":"detach","dropped_calls":0}),
            ],
            "",
        );
        assert_eq!(report.continuous_pairs, 1);
        assert_eq!(report.hours_per_second(), Some(10.0));
        assert_eq!(report.identity_changes, 0);
        assert_eq!(report.discontinuities, 0);
    }
}
