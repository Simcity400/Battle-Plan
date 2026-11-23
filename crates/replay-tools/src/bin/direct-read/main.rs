mod meter;
mod read_hours;
mod replies;
mod rows_digest;
mod walk;

use meter::{Meter, Spent, round, spread};
use replay_recording::{cells::TableReader, columnar_frames::Recording, summaries};
use replay_lanes::lanes;
use replay_data::scope::CaptureMode;
use replay_frame::frame::{TableView, cells_view};
use replay_columnar::summary::{self, Shape};
use replay_data::rows::TableId;
use serde_json::{Value, json};
use std::io;

engine_allocation::counting_system_allocator!();

pub fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

const USAGE: &str = "direct-read <recording> info|seek|chart-time|replies|walk|read-hours|rows-digest|purge|readall [Table] [--options]";

pub struct Options(Vec<(String, String)>);

impl Options {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.iter().find(|(held, _)| held == name).map(|(_, value)| value.as_str())
    }

    pub fn number<T: std::str::FromStr>(&self, name: &str, default: T) -> io::Result<T> {
        match self.get(name) {
            Some(value) => value.parse().map_err(|_| invalid(format!("--{name} is not a number"))),
            None => Ok(default),
        }
    }
}

pub fn table_named(name: &str) -> io::Result<TableId> {
    TableId::ALL.into_iter().find(|id| format!("{id:?}") == name).ok_or_else(|| invalid(format!("No table {name}")))
}

pub fn fields(table: TableId, names: Option<&str>) -> io::Result<Option<Vec<usize>>> {
    let Some(names) = names else { return Ok(None) };
    names
        .split(',')
        .map(|name| (0..table.field_count()).find(|&field| table.field_name(field) == Some(name)).ok_or_else(|| invalid(format!("{table:?} has no field {name}"))))
        .collect::<io::Result<Vec<_>>>()
        .map(Some)
}

pub fn open(path: &str) -> io::Result<Recording> {
    let recording = Recording::open_path(path)?;
    if let Some(damage) = recording.damage().first() {
        return Err(invalid(damage.to_string()));
    }
    Ok(recording)
}

pub struct Random(pub u64);

impl Random {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
}

pub fn sample_hours(recording: &Recording, samples: usize, seed: u64) -> io::Result<Vec<(u32, u32)>> {
    let mut random = Random(seed);
    let total = recording.hours();
    let mut out = Vec::with_capacity(samples);
    for _ in 0..samples {
        let mut at = random.next() % total;
        let mut found = None;
        for run in 0..recording.run_count() {
            let hours = u64::from(recording.run(run)?.ordinals);
            if at < hours {
                found = Some((run, at as u32));
                break;
            }
            at -= hours;
        }
        out.push(found.ok_or_else(|| invalid("A sampled hour lies in no run"))?);
    }
    Ok(out)
}

fn info(path: &str) -> io::Result<Value> {
    let opening = Meter::start();
    let recording = open(path)?;
    let opened = opening.stop();
    let runs: Vec<Value> = (0..recording.run_count())
        .filter_map(|run| recording.run(run).ok())
        .map(|run| json!({ "branch": run.branch, "epoch": run.epoch, "first_hour": run.first_hour, "hours": run.ordinals, "blocks": run.pages() }))
        .collect();
    let schema: String = recording.root_identity().schema.iter().map(|byte| format!("{byte:02x}")).collect();
    let ours: String = replay_columnar::envelope::schema_digest(&replay_recording::schema::schema()?).iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(json!({ "file_bytes": recording.len(), "hours": recording.hours(), "runs": runs, "damage": recording.damage().len(), "schema": schema, "build_schema": ours, "open": opened.json() }))
}

fn phase(name: &'static str) {
    engine_allocation::labels::phase(name);
}

fn labelled() -> Value {
    if !engine_allocation::labels::ENABLED {
        return Value::Null;
    }
    let phases: Vec<Value> = engine_allocation::labels::phases()
        .iter()
        .map(|phase| {
            let mut labels: Vec<_> = phase.labels.iter().filter(|charge| charge.at_peak.abs() > 1 << 16).collect();
            labels.sort_by_key(|charge| std::cmp::Reverse(charge.at_peak));
            json!({ "phase": phase.name, "peak": phase.tracked_peak,
                "labels": labels.iter().map(|charge| json!({ "owner": charge.owner, "kind": charge.kind.name(), "at_peak": charge.at_peak })).collect::<Vec<_>>() })
        })
        .collect();
    json!(phases)
}

fn seek(path: &str, table: TableId, options: &Options) -> io::Result<Value> {
    let samples: usize = options.number("samples", 60)?;
    let seed = options.number("seed", 0x9e37_79b9_7f4a_7c15_u64)?;
    let columns = fields(table, options.get("columns"))?;
    let hours = sample_hours(&open(path)?, samples, seed)?;
    let format = replay_data::format::FormatVersion::CURRENT.version();
    let one = |recording: &Recording, run: u32, ordinal: u32| -> io::Result<(Spent, Value)> {
        let meter = Meter::start();
        let mut reader = TableReader::new(recording, run, table, columns.as_deref())?;
        reader.seek(recording, ordinal)?;
        let advanced = meter.stop();
        let view = cells_view(table, &reader.view(), reader.status(), reader.read_rows(), CaptureMode::Full, format)?;
        let spent = meter.stop();
        let rows = match &view {
            TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows.len(),
            _ => 0,
        };
        std::hint::black_box(&view);
        Ok((spent, json!({ "run": run, "ordinal": ordinal, "rows": rows, "cells_bytes": reader.held_bytes(), "advanced_ms": round(advanced.wall_ms) })))
    };
    phase("seek: each opened");
    let purge_requested = options.get("purge").is_some();
    let mut cache = replay_tools::cold_cache::probe_file(
        std::path::Path::new(path),
        replay_tools::cold_cache::ResetScope::FreshRecordingPerSample,
    )?;
    let mut cold = Vec::new();
    let mut details = Vec::new();
    for &(run, ordinal) in &hours {
        if purge_requested {
            cache = replay_tools::cold_cache::probe_file(
                std::path::Path::new(path),
                replay_tools::cold_cache::ResetScope::FreshRecordingPerSample,
            )?;
        }
        let recording = open(path)?;
        let (spent, mut detail) = one(&recording, run, ordinal)?;
        detail["wall_ms"] = json!(round(spent.wall_ms));
        detail["read_bytes"] = json!(spent.read_bytes);
        details.push(detail);
        cold.push(spent);
    }
    phase("seek: one recording");
    let kept = open(path)?;
    let mut warm = Vec::new();
    for &(run, ordinal) in &hours {
        warm.push(one(&kept, run, ordinal)?.0);
    }
    let summarize = |spent: &[Spent]| {
        json!({
            "wall_ms": spread(&mut spent.iter().map(|spent| spent.wall_ms).collect::<Vec<_>>()),
            "cpu_ms": spread(&mut spent.iter().map(|spent| spent.cpu_ms).collect::<Vec<_>>()),
            "read_bytes": spread(&mut spent.iter().map(|spent| spent.read_bytes as f64).collect::<Vec<_>>()),
        })
    };
    Ok(json!({
        "command": "seek",
        "table": format!("{table:?}"),
        "samples": samples,
        "purge_requested": purge_requested,
        "system_cache_evicted": cache.system_cache_evicted,
        "cache": cache,
        "each_from_a_recording_opened_for_it": summarize(&cold),
        "again_from_one_recording": summarize(&warm),
        "cells_bytes": spread(&mut details.iter().map(|detail| detail["cells_bytes"].as_f64().unwrap_or(0.0)).collect::<Vec<_>>()),
        "details": details,
        "memory": labelled(),
    }))
}

pub fn grid(per_pixel: i64) -> (u8, i64) {
    if per_pixel < summary::bucket_hours(0) {
        return (lanes::WALKED, per_pixel.max(1));
    }
    let mut level = 0u8;
    while summary::bucket_hours(level + 1) <= per_pixel {
        level += 1;
    }
    (level, (per_pixel / summary::bucket_hours(level)) * summary::bucket_hours(level))
}

pub fn views(first_hour: i64, ordinals: u32) -> [(&'static str, i32, i32); 3] {
    let end = first_hour + i64::from(ordinals);
    let half = first_hour + i64::from(ordinals) / 2;
    [("run", first_hour, end), ("year", half, half + 8760), ("three years", first_hour, first_hour + 3 * 8760)].map(|(name, from, to)| (name, from as i32, (to.min(end) - 1) as i32))
}

pub fn eight_keys(recording: &Recording, table: TableId) -> io::Result<Vec<u128>> {
    let mut lives = lanes::lifetimes(recording, 0, table)?;
    lives.sort_by_key(|(key, first, last)| (std::cmp::Reverse(last - first), *key));
    Ok(lives.iter().take(8).map(|(key, _, _)| *key).collect())
}

pub fn chart_column(table: TableId, options: &Options) -> io::Result<u32> {
    let registry = summaries::registry()?;
    let Some((_, Shape::Values(columns))) = registry.iter().find(|(held, _)| *held == table.index()) else {
        return Err(invalid("The table has no summarized columns"));
    };
    match options.get("column") {
        Some(id) => id.parse().map_err(|_| invalid("--column=<source id>")),
        None => Ok(columns[0].source.id()),
    }
}

fn chart_time(path: &str, table: TableId, options: &Options) -> io::Result<Value> {
    let source = chart_column(table, options)?;
    let width: i64 = options.number("pixels", 1000)?;
    let recording = open(path)?;
    let held = recording.run(0)?;
    let (start, ordinals) = (i64::from(held.first_hour), held.ordinals);
    let eight = eight_keys(&recording, table)?;
    drop(recording);
    let cache = replay_tools::cold_cache::probe_file(
        std::path::Path::new(path),
        replay_tools::cold_cache::ResetScope::FreshRecordingPerSample,
    )?;
    let time = |keys: Option<&[u128]>, from: i32, to: i32, level: u8, bin_hours: i64| -> io::Result<Value> {
        let probe = replay_tools::cold_cache::probe_file(
            std::path::Path::new(path),
            replay_tools::cold_cache::ResetScope::FreshRecordingPerSample,
        )?;
        let cold = open(path)?;
        let meter = Meter::start();
        let chart = lanes::chart_at(&cold, 0, table, source, keys, from, to, level, bin_hours)?;
        let first = meter.stop();
        let mut warm = Vec::new();
        for _ in 0..5 {
            let meter = Meter::start();
            lanes::chart_at(&cold, 0, table, source, keys, from, to, level, bin_hours)?;
            warm.push(meter.stop().wall_ms);
        }
        Ok(json!({ "keys": chart.keys.len(), "pieces": chart.pieces, "walked": chart.walked, "bins": chart.bins, "cache_state": probe.label, "fresh_open": first.json(), "warm_ms": spread(&mut warm) }))
    };
    let top = |from: i32, to: i32| -> io::Result<Value> {
        let probe = replay_tools::cold_cache::probe_file(
            std::path::Path::new(path),
            replay_tools::cold_cache::ResetScope::FreshRecordingPerSample,
        )?;
        let cold = open(path)?;
        let meter = Meter::start();
        let got = lanes::top(&cold, 0, table, source, from, to, width as u32, 10, lanes::By::Last)?;
        let first = meter.stop();
        let mut warm = Vec::new();
        for _ in 0..5 {
            let meter = Meter::start();
            lanes::top(&cold, 0, table, source, from, to, width as u32, 10, lanes::By::Last)?;
            warm.push(meter.stop().wall_ms);
        }
        Ok(json!({ "bins": got.len(), "cache_state": probe.label, "fresh_open": first.json(), "warm_ms": spread(&mut warm) }))
    };
    let mut shown = Vec::new();
    for (name, from, to) in views(start, ordinals) {
        let (level, bin_hours) = grid((i64::from(to) + 1 - i64::from(from)) / width);
        phase("chart");
        shown.push(json!({
            "view": name,
            "level": level,
            "bin_hours": bin_hours,
            "eight": time(Some(&eight), from, to, level, bin_hours)?,
            "all": time(None, from, to, level, bin_hours)?,
            "top10": top(from, to)?,
        }));
    }
    let end = start + i64::from(ordinals);
    let level = if end - start >= summary::bucket_hours(1) { 1 } else { 0 };
    let size = summary::bucket_hours(level);
    let whole = (end - start + 2 * size).div_euclid(size) * size;
    phase("rank");
    let rank = time(None, start as i32, (end - 1) as i32, level, whole)?;
    Ok(json!({ "command": "chart-time", "table": format!("{table:?}"), "column": source, "cache": cache, "views": shown, "rank_all_keys_whole_run": rank, "memory": labelled() }))
}

pub fn purge(path: &str) -> io::Result<Value> {
    replay_tools::cold_cache::probe_file(
        std::path::Path::new(path),
        replay_tools::cold_cache::ResetScope::FreshRecordingPerSample,
    )?;
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "direct-read purge cannot evict or verify the Windows system file cache: opening a handle with FILE_FLAG_NO_BUFFERING only selects unbuffered I/O for that handle and does not evict cached pages for later opens",
    ))
}

fn readall(path: &str) -> io::Result<Value> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut buffer = vec![0_u8; 4 << 20];
    let meter = Meter::start();
    let mut total = 0_u64;
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total += count as u64;
    }
    let spent = meter.stop();
    Ok(json!({ "command": "readall", "bytes": total, "spent": spent.json(), "megabytes_per_second": round(total as f64 / 1e6 / (spent.wall_ms / 1e3)) }))
}

fn main() -> io::Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or_else(|| invalid(USAGE))?;
    let command = args.next().ok_or_else(|| invalid(USAGE))?;
    let mut table = None;
    let mut options = Vec::new();
    for arg in args {
        match arg.strip_prefix("--") {
            Some(option) => {
                let (name, value) = option.split_once('=').unwrap_or((option, ""));
                options.push((name.to_owned(), value.to_owned()));
            }
            None if table.is_none() => table = Some(table_named(&arg)?),
            None => return Err(invalid(USAGE)),
        }
    }
    let options = Options(options);
    let table = || table.ok_or_else(|| invalid(USAGE));
    eprintln!("{}", json!({ "build": { "memory_labels": engine_allocation::labels::ENABLED } }));
    let report = match command.as_str() {
        "info" => info(&path)?,
        "seek" => seek(&path, table()?, &options)?,
        "chart-time" => chart_time(&path, table()?, &options)?,
        "replies" => replies::all(&path, &options)?,
        "walk" => walk::walk(&path, &options)?,
        "read-hours" => read_hours::read_hours(&path, &options)?,
        "rows-digest" => rows_digest::rows_digest(&path, &options)?,
        "purge" => purge(&path)?,
        "readall" => readall(&path)?,
        _ => return Err(invalid(USAGE)),
    };
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}
