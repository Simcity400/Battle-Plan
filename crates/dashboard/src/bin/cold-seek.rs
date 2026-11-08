//! How fast a cold seek into the last page of a segment is, as JSON: the recording opened afresh
//! selecting the map's tables, the segment's restart restored, then its last page's units
//! loaded alone, each step timed, for every segment from a restart over two pages or more in
//! turn, and the p50 and p95 of each step and of both. Restores and loads decode apart on
//! rayon's threads, as the viewer's do.
//!
//! `cold-seek <recording> [--samples N]`

use dashboard_base::map_tables::selected_tables;
use replay::columnar_frames::{Recording, Walker, on_rayon};
use serde_json::json;
use std::{io, path::PathBuf, time::Instant};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, what.into())
}

/// The `share` quantile of `values`, in milliseconds.
fn quantile(values: &mut [f64], share: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    let at = ((values.len() as f64 - 1.0) * share).round() as usize;
    values.get(at).copied().unwrap_or(f64::NAN)
}

fn main() -> io::Result<()> {
    let usage = || invalid("usage: cold-seek <recording> [--samples N]");
    let mut args = std::env::args_os().skip(1);
    let path = PathBuf::from(args.next().ok_or_else(usage)?);
    let mut samples = 60_usize;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--samples") => {
                samples = args
                    .next()
                    .and_then(|value| value.to_str()?.parse().ok())
                    .filter(|&count| count > 0)
                    .ok_or_else(usage)?;
            }
            _ => return Err(usage()),
        }
    }
    let tables = selected_tables();
    // Each segment from a restart over two pages or more, and its last page.
    let mut seeks = Vec::new();
    {
        let recording = Recording::open_path(&path)?;
        for segment in recording.segments() {
            if segment.restart.is_none() {
                continue;
            }
            let pages = recording.run(segment.run)?.page_extents();
            let first = pages.partition_point(|page| page.first < segment.first);
            let last = pages.partition_point(|page| page.first <= segment.last) - 1;
            if last > first {
                seeks.push((*segment, last));
            }
        }
    }
    if seeks.is_empty() {
        return Err(invalid("The recording holds no segment from a restart over two pages"));
    }
    let (mut restores, mut loads, mut totals, mut bytes) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for at in 0..samples {
        let (segment, last) = seeks[at % seeks.len()];
        // Afresh: no restart or page another seek decoded is held.
        let mut recording = Recording::open_path(&path)?;
        recording.select(tables);
        let mut walker = Walker::new(&recording)?;
        walker.decode_apart(on_rayon);
        let mut state = recording.state()?;
        let began = Instant::now();
        walker.begin(&segment, &mut state)?;
        let restored = began.elapsed().as_secs_f64() * 1e3;
        let began = Instant::now();
        let decoded = walker.load_page(segment.run, last)?;
        let loaded = began.elapsed().as_secs_f64() * 1e3;
        restores.push(restored);
        loads.push(loaded);
        totals.push(restored + loaded);
        bytes.push(decoded as f64);
    }
    let summary = |values: &mut Vec<f64>| {
        json!({
            "p50": quantile(values, 0.5),
            "p95": quantile(values, 0.95),
        })
    };
    println!(
        "{}",
        json!({
            "file": path.display().to_string(),
            "tables": tables.len(),
            "segments": seeks.len(),
            "samples": samples,
            "threads": std::thread::available_parallelism().map_or(1, usize::from),
            "restore_ms": summary(&mut restores),
            "load_last_page_ms": summary(&mut loads),
            "restore_and_load_ms": summary(&mut totals),
            "page_bytes": summary(&mut bytes),
        })
    );
    Ok(())
}
