
use replay::campaign::Replay;
use replay_columnar::envelope::Extent;
use replay_data::rows::{TABLE_COUNT, TableId};
use serde_json::{Value, json};
use std::{ffi::OsString, path::Path};

pub const USAGE: &str = "recorder sizes <campaign.battleplan> [--json]";

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Sizes {
    pub hours: u64,
    pub pages: u64,
    pub file: u64,
    pub clocks: u64,
    pub summaries: u64,
    pub index: u64,
    pub facts: u64,
    pub blocks: Vec<u64>,
    pub restarts: Vec<u64>,
}

pub fn command(args: Vec<OsString>) -> Result<(), String> {
    let mut path = None;
    let mut as_json = false;
    for arg in args {
        match arg.to_str() {
            Some("--json") => as_json = true,
            _ if path.is_none() => path = Some(arg),
            _ => return Err(USAGE.into()),
        }
    }
    let path = path.ok_or_else(|| USAGE.to_owned())?;
    let replay = Replay::open_path(Path::new(&path)).map_err(|error| error.to_string())?;
    let sizes = measure(&replay).map_err(|error| error.to_string())?;
    let report = report(&sizes);
    if as_json {
        println!("{report}");
    } else {
        print_text(&report);
    }
    Ok(())
}

pub(crate) fn measure(replay: &Replay) -> std::io::Result<Sizes> {
    let recording = replay.recording();
    let mut sizes = Sizes {
        hours: recording.hours(),
        pages: 0,
        file: recording.len(),
        clocks: 0,
        summaries: 0,
        index: 0,
        facts: recording.facts_extents().iter().map(|extent| extent.stored).sum(),
        blocks: vec![0; TABLE_COUNT],
        restarts: vec![0; TABLE_COUNT],
    };
    let stored = |extents: &[Extent]| extents.iter().map(|extent| extent.stored).sum::<u64>();
    sizes.index += stored(recording.run_directories()) + stored(recording.records_extents());
    for run in 0..recording.run_count() {
        let held = recording.run(run)?;
        let directory = held.directory();
        sizes.index += directory.spans.iter().map(|span| span.directory.stored + span.index.stored).sum::<u64>();
        sizes.index += stored(&directory.open);
        for span in 0..directory.spans.len() as u32 {
            for table in 0..recording.tables() {
                for (_, extent) in recording.span_summaries(run, span, table)? {
                    sizes.summaries += extent.stored;
                }
            }
        }
        for at in 0..held.pages() as u32 {
            sizes.pages += 1;
            let page = recording.block(run, at)?;
            sizes.clocks += u64::from(page.head.clock.stored);
            for (table, entry) in page.tables.iter().enumerate() {
                let parts: u64 = entry.unit.parts.iter().flat_map(|part| &part.frames).map(|frame| u64::from(frame.stored)).sum();
                let restores: u64 = entry.unit.first.iter().chain(&entry.unit.restore).flat_map(|restore| &restore.streams).flat_map(|(_, chunks)| chunks).map(|chunk| u64::from(chunk.frame.stored)).sum();
                sizes.blocks[table] += parts;
                sizes.restarts[table] += restores;
                sizes.summaries += entry.summary.map_or(0, |extent| extent.stored);
            }
        }
    }
    Ok(sizes)
}

fn per_hour(bytes: u64, hours: u64) -> f64 {
    if hours == 0 {
        0.0
    } else {
        bytes as f64 / hours as f64
    }
}

pub(crate) fn report(sizes: &Sizes) -> Value {
    let named: u64 = sizes.clocks
        + sizes.summaries
        + sizes.index
        + sizes.facts
        + sizes.blocks.iter().sum::<u64>()
        + sizes.restarts.iter().sum::<u64>();
    let mut tables: Vec<_> = TableId::ALL
        .iter()
        .filter(|table| sizes.blocks[table.index()] + sizes.restarts[table.index()] > 0)
        .map(|table| {
            let (blocks, restarts) = (sizes.blocks[table.index()], sizes.restarts[table.index()]);
            json!({
                "table": format!("{table:?}"),
                "blocks": blocks,
                "restarts": restarts,
                "bytes_per_hour": per_hour(blocks + restarts, sizes.hours),
            })
        })
        .collect();
    tables.sort_by_key(|table| std::cmp::Reverse(table["blocks"].as_u64().unwrap_or(0) + table["restarts"].as_u64().unwrap_or(0)));
    json!({
        "hours": sizes.hours,
        "pages": sizes.pages,
        "file_bytes": sizes.file,
        "file_bytes_per_hour": per_hour(sizes.file, sizes.hours),
        "clocks": sizes.clocks,
        "summaries": sizes.summaries,
        "index": sizes.index,
        "facts": sizes.facts,
        "unnamed": sizes.file.saturating_sub(named),
        "tables": tables,
    })
}

fn print_text(report: &Value) {
    println!(
        "hours={} pages={} file_bytes={} bytes_per_hour={:.1}",
        report["hours"], report["pages"], report["file_bytes"],
        report["file_bytes_per_hour"].as_f64().unwrap_or(0.0)
    );
    println!(
        "clocks={} summaries={} index={} facts={} unnamed={}",
        report["clocks"], report["summaries"], report["index"], report["facts"], report["unnamed"]
    );
    println!("table blocks restarts bytes_per_hour");
    for table in report["tables"].as_array().into_iter().flatten() {
        println!(
            "{} {} {} {:.1}",
            table["table"].as_str().unwrap_or_default(),
            table["blocks"],
            table["restarts"],
            table["bytes_per_hour"].as_f64().unwrap_or(0.0)
        );
    }
}
