//! Offline size attribution for encoding work: writes a current-format
//! recording's frames again with the current block writer, in memory, and
//! attributes every compressed byte of the result to its block section (the
//! frame header, diagnostics, and each table's keyframe and later entries),
//! per recorded hour, beside the recording's stored size. Layout and encoder
//! experiments need no game run, and `--verify` checks that the rewritten
//! recording decodes to the same hours. `--columns` splits each table's bytes
//! between its section columns (control, keys, masks and each field), and
//! each column's between keyframe and later entries, and counts how often
//! each field changed. `--levels` compresses every rewritten section again at
//! other zstd levels.

use replay::{container::Replay, container::Section};
use replay_data::rows::TABLE_COUNT;
use replay_data::rows::TableId;
use replay_frame::{frame::DecodedFrame, frame::FrameDecoder, frame::TableUpdate};
use serde_json::{Value, json};
use std::{ffi::OsString, fs, io, path::Path};

pub const USAGE: &str = "recorder sizes <campaign.battleplan> [--json] [--verify] [--blocks] [--columns] [--levels <level>[,<level>...]] [--write <rewritten.battleplan>]";

/// Byte totals for one recording; `table_*` are indexed by `TableId::index`.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Sizes {
    pub refusals: Vec<Refusal>,
    pub hours: u64,
    pub blocks: u64,
    /// Blocks as the recording stores them.
    pub recorded: u64,
    /// Blocks as the current writer stores them.
    pub encoded: u64,
    /// Plain frames (spool encoding) the blocks hold.
    pub plain: u64,
    pub header_raw: u64,
    pub header: u64,
    pub diagnostics_raw: u64,
    pub diagnostics: u64,
    pub table_plain: Vec<u64>,
    pub table_keyframe: Vec<u64>,
    pub table_later: Vec<u64>,
    pub table_raw: Vec<u64>,
    /// Delta rows written whole (inserted, or every field changed), delta
    /// rows changing some fields, and deleted keys.
    pub table_whole_rows: Vec<u64>,
    pub table_changed_rows: Vec<u64>,
    pub table_deleted_rows: Vec<u64>,
    /// By field: the delta rows changing some fields that changed it.
    pub field_changes: Vec<Vec<u64>>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Refusal {
    table: String,
    branch: u64,
    reason: String,
    first_hour: i32,
    last_hour: i32,
    count: u64,
}

impl Sizes {
    /// Block directories and framing outside every section.
    pub fn framing(&self) -> u64 {
        self.encoded
            - self.header
            - self.diagnostics
            - self.table_keyframe.iter().sum::<u64>()
            - self.table_later.iter().sum::<u64>()
    }
}

pub fn command(args: Vec<OsString>) -> Result<(), String> {
    let mut path = None;
    let mut as_json = false;
    let mut verify = false;
    let mut blocks = false;
    let mut columns = false;
    let mut levels = Vec::new();
    let mut write = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--json") => as_json = true,
            Some("--verify") => verify = true,
            Some("--blocks") => blocks = true,
            Some("--columns") => columns = true,
            Some("--levels") => {
                levels = parse_levels(args.next().as_ref().and_then(|list| list.to_str()))?;
            }
            Some("--write") => write = Some(args.next().ok_or_else(|| USAGE.to_owned())?),
            _ if path.is_none() => path = Some(arg),
            _ => return Err(USAGE.into()),
        }
    }
    let path = path.ok_or_else(|| USAGE.to_owned())?;
    let source = Path::new(&path);
    let replay = Replay::open(fs::read(source).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    let (sizes, encoded) = measure(&replay).map_err(|error| error.to_string())?;
    if verify {
        let hours = verify_same_hours(&replay, &encoded).map_err(|error| error.to_string())?;
        eprintln!("verified {hours} hours decode identically");
    }
    if let Some(destination) = write {
        let destination = Path::new(&destination);
        // Hold the recorder's publish lock so an in-place rewrite targets the measured recording.
        let _lock = finalization_lock(destination)?;
        if fs::read(source).map_err(|error| error.to_string())? != replay.bytes() {
            return Err(format!(
                "{} changed while it was measured; nothing was written",
                source.display()
            ));
        }
        encoded
            .write_atomic(destination)
            .map_err(|error| error.to_string())?;
    }
    let mut report = report(&sizes);
    if blocks {
        report["block_sections"] = block_report(&encoded).map_err(|error| error.to_string())?;
    }
    if columns {
        report["columns"] = column_report(&encoded, &sizes).map_err(|error| error.to_string())?;
    }
    if !levels.is_empty() {
        report["levels"] =
            level_report(&encoded, &sizes, &levels).map_err(|error| error.to_string())?;
    }
    if as_json {
        println!("{report}");
    } else {
        print_text(&report);
    }
    Ok(())
}

fn finalization_lock(destination: &Path) -> Result<fs::File, String> {
    use std::os::windows::fs::OpenOptionsExt;
    use std::time::{Duration, Instant};
    let directory = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let path = directory.join("finalize.lock");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(&path)
        {
            Ok(file) => return Ok(file),
            Err(error) if error.raw_os_error() == Some(32) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100))
            }
            Err(error) => {
                return Err(format!(
                    "output is busy recording/finalizing or unavailable: {}: {error}",
                    path.display()
                ));
            }
        }
    }
}

/// Every rewritten block's sections, in block order.
pub(crate) fn block_report(encoded: &Replay) -> io::Result<Value> {
    let mut sections = Vec::new();
    for block in &encoded.index {
        for section in encoded
            .block_sections(block)?
            .ok_or_else(|| io::Error::other("missing grouped sections"))?
        {
            sections.push(json!({
                "branch": block.branch,
                "first_hour": block.first_hour,
                "last_hour": block.last_hour,
                "section": format!("{:?}", section.section),
                "raw": section.raw,
                "compressed": section.compressed,
            }));
        }
    }
    Ok(Value::Array(sections))
}

pub(crate) fn measure(replay: &Replay) -> io::Result<(Sizes, Replay)> {
    let mut sizes = Sizes {
        table_plain: vec![0; TABLE_COUNT],
        table_keyframe: vec![0; TABLE_COUNT],
        table_later: vec![0; TABLE_COUNT],
        table_raw: vec![0; TABLE_COUNT],
        table_whole_rows: vec![0; TABLE_COUNT],
        table_changed_rows: vec![0; TABLE_COUNT],
        table_deleted_rows: vec![0; TABLE_COUNT],
        field_changes: TableId::ALL
            .iter()
            .map(|table| vec![0; table.field_count()])
            .collect(),
        recorded: replay.index.iter().map(|block| block.length).sum(),
        ..Sizes::default()
    };
    let encoded =
        replay.reencoded(|frame, _, table_sizes| {
            sizes.hours += 1;
            for table in TableId::ALL {
                if let TableUpdate::Unavailable { reason } = &frame.tables[table.index()]
                    && replay_data::format::FormatVersion::CURRENT
                        .table_refusal(table)
                        .is_none()
                {
                    let name = format!("{table:?}");
                    if let Some(refusal) = sizes.refusals.iter_mut().find(|r| {
                        r.table == name && r.branch == frame.branch && r.reason == *reason
                    }) {
                        refusal.last_hour = frame.hour;
                        refusal.count += 1;
                    } else {
                        sizes.refusals.push(Refusal {
                            table: name,
                            branch: frame.branch,
                            reason: reason.clone(),
                            first_hour: frame.hour,
                            last_hour: frame.hour,
                            count: 1,
                        });
                    }
                }
                let TableUpdate::Delta {
                    changed, deleted, ..
                } = &frame.tables[table.index()]
                else {
                    continue;
                };
                let index = table.index();
                sizes.table_deleted_rows[index] += deleted.len() as u64;
                let whole = u64::MAX >> (64 - table.field_count());
                for row in changed {
                    if row.mask == whole {
                        sizes.table_whole_rows[index] += 1;
                        continue;
                    }
                    sizes.table_changed_rows[index] += 1;
                    for (field, changes) in sizes.field_changes[index].iter_mut().enumerate() {
                        *changes += row.mask >> field & 1;
                    }
                }
            }
            // A plain frame is its 37-byte header, a u64 per table and its updates.
            let present = TableId::ALL
                .into_iter()
                .filter(|table| {
                    replay_data::format::FormatVersion::CURRENT
                        .table_refusal(*table)
                        .is_none()
                })
                .count();
            sizes.plain += (37 + 8 * present + table_sizes.iter().sum::<usize>()) as u64;
            for (total, size) in sizes.table_plain.iter_mut().zip(table_sizes) {
                *total += size as u64;
            }
            Ok(())
        })?;
    for block in &encoded.index {
        sizes.blocks += 1;
        sizes.encoded += block.length;
        let sections = encoded
            .block_sections(block)?
            .ok_or_else(|| io::Error::other("rewritten block is not grouped"))?;
        for section in sections {
            match section.section {
                Section::Header => {
                    sizes.header_raw += section.raw;
                    sizes.header += section.compressed;
                }
                Section::Diagnostics => {
                    sizes.diagnostics_raw += section.raw;
                    sizes.diagnostics += section.compressed;
                }
                Section::Keyframe(table) => {
                    sizes.table_raw[table.index()] += section.raw;
                    sizes.table_keyframe[table.index()] += section.compressed;
                }
                Section::Later(table) => {
                    sizes.table_raw[table.index()] += section.raw;
                    sizes.table_later[table.index()] += section.compressed;
                }
            }
        }
    }
    Ok((sizes, encoded))
}

/// Decoded hours of a replay in block order, one block in memory.
struct Hours<'a> {
    replay: &'a Replay,
    blocks: std::slice::Iter<'a, replay::container::BlockIndex>,
    decoder: FrameDecoder,
    frames: std::vec::IntoIter<replay_frame::frame::Frame>,
}

impl<'a> Hours<'a> {
    fn new(replay: &'a Replay) -> Self {
        Self {
            replay,
            blocks: replay.index.iter(),
            decoder: FrameDecoder::new(),
            frames: Vec::new().into_iter(),
        }
    }
}

impl Iterator for Hours<'_> {
    type Item = io::Result<DecodedFrame>;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(frame) = self.frames.next() {
                // Table read times as blocks store them.
                return Some(self.decoder.advance(&frame).map(|decoded| DecodedFrame {
                    metrics: decoded.metrics.stored(),
                    ..decoded.clone()
                }));
            }
            let block = self.blocks.next()?;
            match self.replay.read_block(block) {
                Ok(frames) => {
                    self.frames = frames.into_iter();
                    self.decoder = FrameDecoder::new();
                }
                Err(error) => return Some(Err(error)),
            }
        }
    }
}

/// Decode both recordings and compare every hour in order.
pub(crate) fn verify_same_hours(recorded: &Replay, encoded: &Replay) -> io::Result<u64> {
    let mut hours = 0;
    let mut actual = Hours::new(encoded);
    for wanted in Hours::new(recorded) {
        let wanted = wanted?;
        match actual.next().transpose()? {
            Some(actual)
                if actual == wanted
                    && actual
                        .tables
                        .iter()
                        .zip(&wanted.tables)
                        .all(|(a, b)| a.sampled_at() == b.sampled_at()) =>
            {
                hours += 1;
            }
            _ => {
                return Err(io::Error::other(format!(
                    "branch {} hour {} decodes differently after rewriting",
                    wanted.branch, wanted.hour
                )));
            }
        }
    }
    if actual.next().is_some() {
        return Err(io::Error::other("rewritten recording holds more hours"));
    }
    Ok(hours)
}

fn per_hour(bytes: u64, hours: u64) -> f64 {
    if hours == 0 {
        0.0
    } else {
        bytes as f64 / hours as f64
    }
}

pub(crate) fn report(sizes: &Sizes) -> Value {
    let hours = sizes.hours;
    let mut tables: Vec<Value> = TableId::ALL
        .into_iter()
        .filter(|table| sizes.table_raw[table.index()] != 0)
        .map(|table| {
            let index = table.index();
            let compressed = sizes.table_keyframe[index] + sizes.table_later[index];
            json!({
                "table": format!("{table:?}"),
                "plain_per_hour": per_hour(sizes.table_plain[index], hours),
                "raw_per_hour": per_hour(sizes.table_raw[index], hours),
                "keyframe_bytes": sizes.table_keyframe[index],
                "later_per_hour": per_hour(sizes.table_later[index], hours),
                "compressed_per_hour": per_hour(compressed, hours),
                "compressed_share": if sizes.encoded == 0 { 0.0 } else { compressed as f64 / sizes.encoded as f64 },
            })
        })
        .collect();
    tables.sort_by(|a, b| {
        b["compressed_per_hour"]
            .as_f64()
            .partial_cmp(&a["compressed_per_hour"].as_f64())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let keyframes: u64 = sizes.table_keyframe.iter().sum();
    let later: u64 = sizes.table_later.iter().sum();
    json!({
        "hours": hours,
        "blocks": sizes.blocks,
        "recorded_bytes": sizes.recorded,
        "recorded_per_hour": per_hour(sizes.recorded, hours),
        "encoded_bytes": sizes.encoded,
        "encoded_per_hour": per_hour(sizes.encoded, hours),
        "plain_per_hour": per_hour(sizes.plain, hours),
        "header_raw_per_hour": per_hour(sizes.header_raw, hours),
        "header_per_hour": per_hour(sizes.header, hours),
        "diagnostics_raw_per_hour": per_hour(sizes.diagnostics_raw, hours),
        "diagnostics_per_hour": per_hour(sizes.diagnostics, hours),
        "keyframe_bytes": keyframes,
        "keyframe_bytes_per_block": if sizes.blocks == 0 { 0.0 } else { keyframes as f64 / sizes.blocks as f64 },
        "later_per_hour": per_hour(later, hours),
        "framing_per_hour": per_hour(sizes.framing(), hours),
        "tables": tables,
        "refusals": sizes.refusals.iter().map(|r| json!({
            "table": r.table, "branch": r.branch, "reason": r.reason,
            "first_hour": r.first_hour, "last_hour": r.last_hour, "count": r.count,
        })).collect::<Vec<_>>(),
    })
}

/// Each table's stored bytes split between its section columns, beside how
/// often each field changed. A section's compressed bytes are attributed to
/// its columns in proportion to their bytes compressed alone.
pub(crate) fn column_report(encoded: &Replay, sizes: &Sizes) -> io::Result<Value> {
    // By table and column: raw bytes, bytes compressed alone, attributed
    // bytes, and those of them in keyframe entries.
    let mut totals = vec![Vec::<[f64; 4]>::new(); TABLE_COUNT];
    for block in &encoded.index {
        let sections = encoded
            .block_columns(block)?
            .ok_or_else(|| io::Error::other("rewritten block is not grouped"))?;
        for section in sections {
            let (table, keyframe) = match section.section {
                Section::Keyframe(table) => (table, true),
                Section::Later(table) => (table, false),
                _ => continue,
            };
            let alone: u64 = section.columns.iter().map(|column| column.alone).sum();
            let columns = &mut totals[table.index()];
            if columns.len() < section.columns.len() {
                columns.resize(section.columns.len(), [0.0; 4]);
            }
            for (total, column) in columns.iter_mut().zip(&section.columns) {
                total[0] += column.raw as f64;
                total[1] += column.alone as f64;
                if alone != 0 {
                    let attributed = section.compressed as f64 * column.alone as f64 / alone as f64;
                    total[2] += attributed;
                    if keyframe {
                        total[3] += attributed;
                    }
                }
            }
        }
    }
    let hours = sizes.hours.max(1) as f64;
    let mut tables: Vec<(f64, Value)> = TableId::ALL
        .into_iter()
        .filter(|table| !totals[table.index()].is_empty())
        .map(|table| {
            let index = table.index();
            let attributed: f64 = totals[index].iter().map(|total| total[2]).sum();
            let keyframe: f64 = totals[index].iter().map(|total| total[3]).sum();
            let columns: Vec<Value> = totals[index]
                .iter()
                .enumerate()
                .filter(|(_, total)| total[0] != 0.0)
                .map(|(column, total)| {
                    let (name, changes) = match column {
                        0 => ("control", None),
                        1 => ("keys", None),
                        2 => ("masks", None),
                        column if column == 3 + table.field_count() => ("aliases", None),
                        column => (
                            table.field_name(column - 3).unwrap_or("unknown"),
                            sizes.field_changes[index].get(column - 3),
                        ),
                    };
                    json!({
                        "column": name,
                        "attributed_per_hour": total[2] / hours,
                        "keyframe_per_hour": total[3] / hours,
                        "share": if attributed == 0.0 { 0.0 } else { total[2] / attributed },
                        "raw_per_hour": total[0] / hours,
                        "alone_per_hour": total[1] / hours,
                        "changes_per_hour": changes.map(|changes| *changes as f64 / hours),
                    })
                })
                .collect();
            (
                attributed,
                json!({
                    "table": format!("{table:?}"),
                    "attributed_per_hour": attributed / hours,
                    "keyframe_per_hour": keyframe / hours,
                    "whole_rows_per_hour": sizes.table_whole_rows[index] as f64 / hours,
                    "changed_rows_per_hour": sizes.table_changed_rows[index] as f64 / hours,
                    "deleted_rows_per_hour": sizes.table_deleted_rows[index] as f64 / hours,
                    "columns": columns,
                }),
            )
        })
        .collect();
    tables.sort_by(|a, b| b.0.total_cmp(&a.0));
    Ok(Value::Array(
        tables.into_iter().map(|(_, table)| table).collect(),
    ))
}

/// Comma-separated zstd levels.
fn parse_levels(list: Option<&str>) -> Result<Vec<i32>, String> {
    let list = list.ok_or(USAGE)?;
    list.split(',')
        .map(|level| match level.trim().parse::<i32>() {
            Ok(level) if (1..=22).contains(&level) => Ok(level),
            _ => Err(format!("zstd level {level:?} is not 1 to 22")),
        })
        .collect()
}

/// Every rewritten section compressed again at each of `levels`: the bytes
/// per hour of all sections and of each table's, beside the writer's.
pub(crate) fn level_report(encoded: &Replay, sizes: &Sizes, levels: &[i32]) -> io::Result<Value> {
    // By table (the last entry: header and diagnostics): written bytes, then
    // each level's.
    let mut totals = vec![vec![0_u64; levels.len() + 1]; TABLE_COUNT + 1];
    let mut elapsed = vec![std::time::Duration::ZERO; levels.len()];
    for block in &encoded.index {
        for section in encoded
            .block_levels(block, levels)?
            .ok_or_else(|| io::Error::other("rewritten block is not grouped"))?
        {
            let index = match section.section {
                Section::Keyframe(table) | Section::Later(table) => table.index(),
                _ => TABLE_COUNT,
            };
            totals[index][0] += section.compressed;
            for (total, bytes) in totals[index][1..].iter_mut().zip(&section.levels) {
                *total += bytes;
            }
            for (total, time) in elapsed.iter_mut().zip(&section.elapsed) {
                *total += *time;
            }
        }
    }
    let hours = sizes.hours.max(1) as f64;
    let per_hour = |bytes: u64| bytes as f64 / hours;
    let sum = |level: usize| totals.iter().map(|table| table[level]).sum::<u64>();
    let mut ranked: Vec<(usize, &Vec<u64>)> = totals[..TABLE_COUNT]
        .iter()
        .enumerate()
        .filter(|(_, table)| table[0] != 0)
        .collect();
    ranked.sort_by(|a, b| b.1[0].cmp(&a.1[0]));
    Ok(json!({
        "written_per_hour": per_hour(sum(0)),
        "levels": levels.iter().enumerate().map(|(at, level)| json!({
            "level": level,
            "sections_per_hour": per_hour(sum(at + 1)),
            "seconds": elapsed[at].as_secs_f64(),
        })).collect::<Vec<_>>(),
        "tables": ranked.iter().map(|(index, table)| json!({
            "table": format!("{:?}", TableId::ALL[*index]),
            "written_per_hour": per_hour(table[0]),
            "levels_per_hour": table[1..].iter().map(|bytes| per_hour(*bytes)).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    }))
}

fn print_text(report: &Value) {
    for section in report["block_sections"].as_array().into_iter().flatten() {
        println!(
            "block branch={} hours={}..={} section={} raw={} compressed={}",
            section["branch"],
            section["first_hour"],
            section["last_hour"],
            section["section"].as_str().unwrap_or_default(),
            section["raw"],
            section["compressed"]
        );
    }
    for refusal in report["refusals"].as_array().into_iter().flatten() {
        println!(
            "refused {} branch={} hours={}..={} count={}: {}",
            refusal["table"].as_str().unwrap(),
            refusal["branch"],
            refusal["first_hour"],
            refusal["last_hour"],
            refusal["count"],
            refusal["reason"].as_str().unwrap()
        );
    }
    let number = |key: &str| report[key].as_f64().unwrap_or(0.0);
    println!("hours={} blocks={}", report["hours"], report["blocks"]);
    println!(
        "recorded_per_hour={:.1} encoded_per_hour={:.1} plain_per_hour={:.1}",
        number("recorded_per_hour"),
        number("encoded_per_hour"),
        number("plain_per_hour")
    );
    println!(
        "header_per_hour={:.1} (raw {:.1}) diagnostics_per_hour={:.1} (raw {:.1}) framing_per_hour={:.1}",
        number("header_per_hour"),
        number("header_raw_per_hour"),
        number("diagnostics_per_hour"),
        number("diagnostics_raw_per_hour"),
        number("framing_per_hour")
    );
    println!(
        "keyframe_bytes={} keyframe_bytes_per_block={:.0} later_per_hour={:.1}",
        report["keyframe_bytes"],
        number("keyframe_bytes_per_block"),
        number("later_per_hour")
    );
    println!(
        "table plain_per_hour raw_per_hour keyframe_bytes later_per_hour compressed_per_hour compressed_share"
    );
    for table in report["tables"].as_array().into_iter().flatten() {
        println!(
            "{} {:.1} {:.1} {} {:.1} {:.1} {:.4}",
            table["table"].as_str().unwrap_or_default(),
            table["plain_per_hour"].as_f64().unwrap_or(0.0),
            table["raw_per_hour"].as_f64().unwrap_or(0.0),
            table["keyframe_bytes"],
            table["later_per_hour"].as_f64().unwrap_or(0.0),
            table["compressed_per_hour"].as_f64().unwrap_or(0.0),
            table["compressed_share"].as_f64().unwrap_or(0.0)
        );
    }
    if let Some(levels) = report["levels"]["levels"].as_array() {
        println!(
            "sections written_per_hour={:.1}",
            report["levels"]["written_per_hour"].as_f64().unwrap_or(0.0)
        );
        for level in levels {
            println!(
                "sections level={} per_hour={:.1} seconds={:.1}",
                level["level"],
                level["sections_per_hour"].as_f64().unwrap_or(0.0),
                level["seconds"].as_f64().unwrap_or(0.0)
            );
        }
        for table in report["levels"]["tables"]
            .as_array()
            .into_iter()
            .flatten()
            .take(30)
        {
            println!(
                "level table {} written={:.1} levels={}",
                table["table"].as_str().unwrap_or_default(),
                table["written_per_hour"].as_f64().unwrap_or(0.0),
                table["levels_per_hour"]
            );
        }
    }
    let Some(columns) = report["columns"].as_array() else {
        return;
    };
    println!(
        "table column attributed_per_hour share raw_per_hour alone_per_hour changes_per_hour keyframe_per_hour"
    );
    for table in columns {
        let name = table["table"].as_str().unwrap_or_default();
        println!(
            "{name} rows attributed_per_hour={:.1} keyframe_per_hour={:.1} whole_rows_per_hour={:.2} changed_rows_per_hour={:.2} deleted_rows_per_hour={:.2}",
            table["attributed_per_hour"].as_f64().unwrap_or(0.0),
            table["keyframe_per_hour"].as_f64().unwrap_or(0.0),
            table["whole_rows_per_hour"].as_f64().unwrap_or(0.0),
            table["changed_rows_per_hour"].as_f64().unwrap_or(0.0),
            table["deleted_rows_per_hour"].as_f64().unwrap_or(0.0)
        );
        for column in table["columns"].as_array().into_iter().flatten() {
            println!(
                "{name} {} {:.1} {:.4} {:.1} {:.1} {} {:.1}",
                column["column"].as_str().unwrap_or_default(),
                column["attributed_per_hour"].as_f64().unwrap_or(0.0),
                column["share"].as_f64().unwrap_or(0.0),
                column["raw_per_hour"].as_f64().unwrap_or(0.0),
                column["alone_per_hour"].as_f64().unwrap_or(0.0),
                column["changes_per_hour"]
                    .as_f64()
                    .map_or("-".to_owned(), |changes| format!("{changes:.2}")),
                column["keyframe_per_hour"].as_f64().unwrap_or(0.0)
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::spool::Record;
    use replay_frame::frame::Frame;

    fn replay() -> Replay {
        open(crate::test_support::allocation_records())
    }

    fn open(records: Vec<Record>) -> Replay {
        let encoded = Replay::from_records(
            crate::test_support::ALLOCATION_CAMPAIGN.to_vec(),
            &[records],
        )
        .unwrap();
        Replay::open(encoded.bytes().to_vec()).unwrap()
    }

    /// The allocation campaign as a writer encodes it: its first hour a
    /// keyframe, the later ones deltas of the rows whose hour moved.
    fn delta_replay() -> Replay {
        let mut decoder = replay_frame::frame::FrameDecoder::new();
        let mut encoder = replay_frame::frame::FrameEncoder::new();
        let mut first = true;
        open(
            crate::test_support::allocation_records()
                .into_iter()
                .map(|record| match record {
                    Record::Frame(frame) => {
                        let decoded = decoder.apply(&frame).unwrap();
                        let frame = encoder.encode(decoded, first).unwrap();
                        first = false;
                        Record::Frame(frame)
                    }
                    record => record,
                })
                .collect(),
        )
    }

    #[test]
    fn validation_refusals_survive_sizes_and_verification() {
        let mut records = crate::test_support::allocation_records();
        let reason = "spool validation failed: States: row 64 column continent: missing or not recorded row field";
        let mut count = 0;
        for record in &mut records {
            if let Record::Frame(frame) = record {
                frame.tables[TableId::States.index()] = TableUpdate::Unavailable {
                    reason: reason.into(),
                };
                count += 1;
            }
        }
        let replay = open(records);
        let (sizes, encoded) = measure(&replay).unwrap();
        let refusal = report(&sizes)["refusals"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["table"] == "States")
            .unwrap()
            .clone();
        assert_eq!(refusal["reason"], reason);
        assert_eq!(refusal["count"], count);
        assert_eq!(verify_same_hours(&replay, &encoded).unwrap(), count);
    }

    #[test]
    fn sizes_attribute_every_rewritten_byte_to_a_section_or_framing() {
        let replay = replay();
        let (sizes, encoded) = measure(&replay).unwrap();
        let frames: Vec<Frame> = crate::test_support::allocation_records()
            .into_iter()
            .filter_map(|record| match record {
                Record::Frame(frame) => Some(frame),
                _ => None,
            })
            .collect();
        assert_eq!(sizes.hours, frames.len() as u64);
        let mut expected = vec![0u64; TABLE_COUNT];
        for frame in &frames {
            for (total, size) in expected
                .iter_mut()
                .zip(frame.table_encoded_sizes().unwrap())
            {
                *total += size as u64;
            }
        }
        assert_eq!(sizes.table_plain, expected);
        // The current writer reproduces a current recording byte for byte.
        assert_eq!(encoded.bytes(), replay.bytes());
        assert_eq!(sizes.encoded, sizes.recorded);
        assert_eq!(sizes.blocks, replay.index.len() as u64);
        assert!(sizes.header + sizes.diagnostics > 0 && sizes.framing() > 0);
        assert!(sizes.table_keyframe.iter().sum::<u64>() > 0);
        assert_eq!(verify_same_hours(&replay, &encoded).unwrap(), sizes.hours);
        let report = report(&sizes);
        assert_eq!(report["hours"], frames.len());
        let shares: f64 = report["tables"]
            .as_array()
            .unwrap()
            .iter()
            .map(|table| table["compressed_share"].as_f64().unwrap())
            .sum();
        assert!(shares > 0.0 && shares <= 1.0);
    }

    #[test]
    fn block_sections_travel_inside_the_json_report() {
        let replay = replay();
        let (sizes, encoded) = measure(&replay).unwrap();
        let mut report = report(&sizes);
        report["block_sections"] = block_report(&encoded).unwrap();
        let parsed: Value = serde_json::from_str(&report.to_string()).unwrap();
        let sections = parsed["block_sections"].as_array().unwrap();
        assert!(
            sections
                .iter()
                .all(|section| section["section"].is_string())
        );
        let compressed: u64 = sections
            .iter()
            .map(|section| section["compressed"].as_u64().unwrap())
            .sum();
        assert_eq!(
            compressed,
            sizes.header
                + sizes.diagnostics
                + sizes.table_keyframe.iter().sum::<u64>()
                + sizes.table_later.iter().sum::<u64>()
        );
    }

    #[test]
    fn columns_attribute_every_table_byte_to_a_named_column() {
        let replay = delta_replay();
        let (sizes, encoded) = measure(&replay).unwrap();
        let report = column_report(&encoded, &sizes).unwrap();
        let tables = report.as_array().unwrap();
        assert!(!tables.is_empty());
        let attributed = tables
            .iter()
            .map(|table| table["attributed_per_hour"].as_f64().unwrap())
            .sum::<f64>()
            * sizes.hours as f64;
        let stored = (sizes.table_keyframe.iter().sum::<u64>()
            + sizes.table_later.iter().sum::<u64>()) as f64;
        assert!((attributed - stored).abs() <= stored * 1e-9 + 1e-6);
        for table in tables {
            for column in table["columns"].as_array().unwrap() {
                let name = column["column"].as_str().unwrap();
                assert_ne!(name, "unknown", "{table}");
                let counted = !matches!(name, "control" | "keys" | "masks" | "aliases");
                assert_eq!(column["changes_per_hour"].is_f64(), counted, "{table}");
            }
        }
        // A delta row changing some fields changes at least one.
        for table in TableId::ALL {
            let index = table.index();
            assert!(
                sizes.field_changes[index].iter().sum::<u64>() >= sizes.table_changed_rows[index]
            );
        }
        assert!(
            sizes.table_whole_rows.iter().sum::<u64>()
                + sizes.table_changed_rows.iter().sum::<u64>()
                > 0
        );
        assert_eq!(TableId::Divisions.field_name(6), Some("organisation"));
        assert_eq!(TableId::Divisions.field_name(9), None);
        // Keyframe bytes are part of each column's and table's.
        for table in tables {
            let keyframe = table["keyframe_per_hour"].as_f64().unwrap();
            assert!(keyframe <= table["attributed_per_hour"].as_f64().unwrap() + 1e-9);
            let columns = table["columns"].as_array().unwrap();
            let sum: f64 = columns
                .iter()
                .map(|column| column["keyframe_per_hour"].as_f64().unwrap())
                .sum();
            assert!((sum - keyframe).abs() <= 1e-6, "{table}");
        }
    }

    #[test]
    fn levels_compress_every_section_again_and_the_writers_level_matches() {
        let replay = replay();
        let (sizes, encoded) = measure(&replay).unwrap();
        let stored = replay::container::STORED_LEVEL;
        let report = level_report(&encoded, &sizes, &[9, stored, 22]).unwrap();
        let written = report["written_per_hour"].as_f64().unwrap();
        let at = |index: usize| {
            report["levels"][index]["sections_per_hour"]
                .as_f64()
                .unwrap()
        };
        // The default writer stores each section at the stored level.
        assert!(written <= at(1), "{report}");
        assert!(at(2) > 0.0);
        let tables: f64 = report["tables"]
            .as_array()
            .unwrap()
            .iter()
            .map(|table| table["written_per_hour"].as_f64().unwrap())
            .sum();
        assert!(tables <= written);
        assert_eq!(parse_levels(Some("9, 19")).unwrap(), [9, 19]);
        for bad in ["", "0", "23", "x", "9,"] {
            assert!(parse_levels(Some(bad)).is_err(), "{bad}");
        }
    }
}
