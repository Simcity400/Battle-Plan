//! Size attribution of a campaign's recording: every stored byte of the file by what it holds,
//! each table's blocks and restarts, the hour records, the dictionaries, the index and the game
//! facts, per recorded hour, beside the file's length.

use replay::campaign::Replay;
use replay_columnar::envelope::Extent;
use replay_data::rows::{TABLE_COUNT, TableId};
use serde_json::{Value, json};
use std::{ffi::OsString, path::Path};

pub const USAGE: &str = "recorder sizes <campaign.battleplan> [--json]";

/// Stored bytes of one recording by what they hold; `blocks` and `restarts` by table.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Sizes {
    pub hours: u64,
    pub pages: u64,
    pub file: u64,
    pub records: u64,
    pub dictionaries: u64,
    /// Pages, run directories, campaign records and roots.
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

/// Every stored byte `replay`'s file names, by what it holds.
pub(crate) fn measure(replay: &Replay) -> std::io::Result<Sizes> {
    let recording = replay.recording();
    let mut sizes = Sizes {
        hours: recording.hours(),
        pages: 0,
        file: recording.len(),
        records: 0,
        dictionaries: 0,
        index: 0,
        facts: recording.facts_extents().iter().map(|extent| extent.stored).sum(),
        blocks: vec![0; TABLE_COUNT],
        restarts: vec![0; TABLE_COUNT],
    };
    let stored = |extents: &[Extent]| extents.iter().map(|extent| extent.stored).sum::<u64>();
    sizes.index += stored(recording.run_directories()) + stored(recording.records_extents());
    for run in 0..recording.run_count() {
        let held = recording.run(run)?;
        for (at, page) in held.page_extents().iter().enumerate() {
            sizes.pages += 1;
            sizes.index += page.stored;
            let page = recording.page(run, at)?;
            sizes.records += page.records.stored;
            for block in &page.tables {
                if let Some(total) = sizes.blocks.get_mut(block.stream as usize) {
                    *total += block.stored;
                }
            }
            if let Some(restart) = &page.restart {
                sizes.dictionaries += restart.chunk.stored;
                for table in &restart.tables {
                    if let Some(total) = sizes.restarts.get_mut(table.table as usize) {
                        *total += table.extent.stored;
                    }
                }
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
    let named: u64 = sizes.records
        + sizes.dictionaries
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
        "records": sizes.records,
        "dictionaries": sizes.dictionaries,
        "index": sizes.index,
        "facts": sizes.facts,
        // The header, footers and revisions' superseded roots and directories.
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
        "records={} dictionaries={} index={} facts={} unnamed={}",
        report["records"], report["dictionaries"], report["index"], report["facts"], report["unnamed"]
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
