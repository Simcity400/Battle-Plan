//! Converts a version 3 or 4 columnar recording to version 5 as the recorder writes it: every
//! segment's hours made into blocks on its own worker ([`replay::columnar_blocks`]), then
//! written in order ([`replay::columnar_file::FileWriter`]). With `--check`, the written file
//! is then opened by the reader and every segment walked beside the source's walk, hour by
//! hour: each hour's record (its section lengths aside, which blocks do not keep), every
//! table's status, sample hour and truncated view, each present table's changes and every cell
//! of every row it changed, inserted or filled, and at each segment's end every table whole and
//! the dictionary. Reports each table's stored bytes in both files and what the work took.
//! `columnar-convert <source.h4col> <out.h4col> [--threads n] [--level n] [--check]`
//! `columnar-convert --journal <campaign directory> <capture> <written.h4col>` instead checks a
//! file the recorder wrote against the capture's journal ([`journal`]).

mod journal;
mod source;
mod v3;

use rayon::prelude::*;
use replay::{
    columnar_blocks::{BlockBuilder, Finished},
    columnar_file::{FileWriter, Settings},
    columnar_frames::{Recording, Walker, schema},
};
use replay_columnar::{
    envelope::{self, Identity},
    manifest::Manifest,
    section::Changes,
    state::{Cell, Table},
    walk::State,
};
use replay_data::rows::TableId;
use serde_json::json;
use source::{Segment, Snapshot, Source};
use std::{
    fs::File,
    io::{self, BufWriter},
    path::Path,
    time::Instant,
};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

#[cfg(windows)]
fn thread_ns() -> u64 {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::Threading::{GetCurrentThread, GetThreadTimes},
    };
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    // SAFETY: the pseudo-handle of the current thread and four owned FILETIMEs.
    unsafe { GetThreadTimes(GetCurrentThread(), &mut created, &mut exited, &mut kernel, &mut user) };
    let ticks = |time: FILETIME| u64::from(time.dwHighDateTime) << 32 | u64::from(time.dwLowDateTime);
    (ticks(kernel) + ticks(user)) * 100
}

#[cfg(not(windows))]
fn thread_ns() -> u64 {
    0
}

fn put_cell(hasher: &mut blake3::Hasher, cell: Option<Cell<'_>>) {
    match cell {
        None => {
            hasher.update(&[0xff]);
        }
        Some(Cell::U32List(items)) => {
            hasher.update(&(items.len() as u64).to_le_bytes());
            for item in items {
                hasher.update(&item.to_le_bytes());
            }
        }
        Some(Cell::U64List(items)) => {
            hasher.update(&(items.len() as u64).to_le_bytes());
            for item in items {
                hasher.update(&item.to_le_bytes());
            }
        }
        Some(Cell::Opaque(bytes)) => {
            hasher.update(&(bytes.len() as u64).to_le_bytes());
            hasher.update(bytes);
        }
        Some(cell) => {
            hasher.update(format!("{cell:?}").as_bytes());
        }
    }
}

fn put_list(hasher: &mut blake3::Hasher, values: &[u32]) {
    hasher.update(&(values.len() as u64).to_le_bytes());
    for value in values {
        hasher.update(&value.to_le_bytes());
    }
}

fn put_words(hasher: &mut blake3::Hasher, values: &[u64]) {
    hasher.update(&(values.len() as u64).to_le_bytes());
    for value in values {
        hasher.update(&value.to_le_bytes());
    }
}

/// What an hour did to a table: its changes and every cell of every row it changed, inserted or
/// filled.
fn digest(table: &Table, changes: &Changes) -> [u8; 16] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&[u8::from(changes.truncated)]);
    put_list(&mut hasher, &changes.read);
    put_list(&mut hasher, &changes.changed);
    put_words(&mut hasher, &changes.masks);
    put_words(&mut hasher, &changes.written);
    put_list(&mut hasher, &changes.deleted);
    for key in &changes.deleted_keys {
        hasher.update(&key.to_le_bytes());
    }
    put_list(&mut hasher, &changes.inserted);
    put_list(&mut hasher, &changes.touched);
    for column in &changes.filled {
        hasher.update(&(*column as u64).to_le_bytes());
    }
    put_list(&mut hasher, &changes.fill_slots);
    put_words(&mut hasher, &changes.fill_masks);
    let columns = table.kinds().count();
    let rows = changes.changed.iter().chain(&changes.inserted).chain(&changes.fill_slots);
    for &slot in rows {
        hasher.update(&slot.to_le_bytes());
        hasher.update(&table.key(slot).unwrap_or(u128::MAX).to_le_bytes());
        for column in 0..columns {
            put_cell(&mut hasher, table.column(column).and_then(|held| held.cell(slot)));
            hasher.update(&[u8::from(table.recorded(column, slot))]);
        }
    }
    hasher.finalize().as_bytes()[..16].try_into().expect("sixteen bytes")
}

/// An hour's record but its section lengths, which blocks do not keep.
fn same_record(a: &Manifest<'_>, b: &Manifest<'_>) -> bool {
    a.branch == b.branch
        && a.hour == b.hour
        && a.flags == b.flags
        && (a.total_ns, a.pool_bytes, a.dropped_frames) == (b.total_ns, b.pool_bytes, b.dropped_frames)
        && a.appends == b.appends
        && a.statuses == b.statuses
        && a.read == b.read
        && a.samples == b.samples
        && a.sections.len() == b.sections.len()
        && a.sections.iter().zip(&b.sections).all(|(x, y)| (x.table, &x.columns, x.ops) == (y.table, &y.columns, y.ops))
}

/// A segment's blocks, made.
struct Made {
    segment: Segment,
    blocks: Vec<Finished>,
    hours: u64,
    build_ns: u64,
    held_peak: usize,
}

fn make(source: &Source, segment: Segment, start: &Snapshot) -> io::Result<Made> {
    let begun = thread_ns();
    let run = &source.runs[segment.run];
    let mut builder = BlockBuilder::new()?;
    if segment.first == 0 {
        builder.start_run(run.branch, run.first_hour)?;
    } else {
        let next_hour = run.first_hour + segment.first as i32;
        let (state, available_from) = start.state()?;
        builder.resume(state, run.branch, next_hour, segment.first, Vec::new(), available_from)?;
    }
    let mut blocks = Vec::new();
    let mut hours = 0;
    let mut held_peak = 0;
    source.hours(&segment, |_, manifest, sections| {
        if let Some(block) = builder.hour(manifest, sections)? {
            held_peak = held_peak.max(builder.state().held_bytes());
            blocks.push(block);
        }
        hours += 1;
        Ok(true)
    })?;
    if segment.last + 1 == run.ordinals
        && let Some(block) = builder.end_run()?
    {
        blocks.push(block);
    }
    Ok(Made {
        segment,
        blocks,
        hours,
        build_ns: thread_ns() - begun,
        held_peak,
    })
}

/// Walks `segment` of the source and the written file side by side; the failures it found.
fn check(source: &Source, start: &Snapshot, written: &Recording, segment: &Segment, v4: &replay::columnar_frames::Segment) -> io::Result<(Vec<String>, u64)> {
    let mut failures = Vec::new();
    let (mut theirs, _) = start.state()?;
    let mut walker = Walker::new(written)?;
    let mut ours = State::new(schema()?)?;
    walker.begin(v4, &mut ours)?;
    let columns: Vec<usize> = schema()?.iter().map(Vec::len).collect();
    // The source's hours, read ahead of the written file's walk into a queue.
    let mut queue: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    source.hours(segment, |_, manifest, sections| {
        queue.push((manifest.to_vec(), sections.to_vec()));
        Ok(true)
    })?;
    let mut next = 0usize;
    let mut decode_ns = 0u64;
    let mut clock = thread_ns();
    walker.walk(v4, &mut ours, |ours, manifest, _| {
        decode_ns += thread_ns() - clock;
        let Some((bytes, sections)) = queue.get(next) else {
            failures.push(format!("run {} ordinal {}: the written file holds more hours", segment.run, segment.first as usize + next));
            return Ok(false);
        };
        next += 1;
        let source_manifest = Manifest::read(bytes, &columns)?;
        let mut spans = Vec::new();
        let mut at = 0usize;
        for section in &source_manifest.sections {
            spans.push(at..at + section.bytes as usize);
            at += section.bytes as usize;
        }
        theirs.apply(&source_manifest, |index| Ok(&sections[spans[index].clone()]))?;
        let ordinal = segment.first as usize + next - 1;
        if !same_record(manifest, &source_manifest) {
            failures.push(format!("run {} ordinal {ordinal}: hour records differ", segment.run));
        }
        for table in 0..columns.len() {
            if ours.status(table) != theirs.status(table)
                || ours.sampled_at(table) != theirs.sampled_at(table)
                || ours.truncated_view(table) != theirs.truncated_view(table)
            {
                failures.push(format!("run {} ordinal {ordinal}: {:?} status, sample or view differ", segment.run, TableId::ALL[table]));
            }
        }
        for section in &source_manifest.sections {
            let table = section.table;
            let mine = digest(ours.table(table).expect("table"), ours.changes(table).expect("table"));
            let held = digest(theirs.table(table).expect("table"), theirs.changes(table).expect("table"));
            if mine != held {
                failures.push(format!("run {} ordinal {ordinal}: {:?} differs", segment.run, TableId::ALL[table]));
            }
        }
        clock = thread_ns();
        Ok(failures.len() < 20)
    })?;
    if next != queue.len() {
        failures.push(format!("run {}: the written segment from {} holds {next} of {} hours", segment.run, segment.first, queue.len()));
    }
    for table in 0..columns.len() {
        if ours.table(table) != theirs.table(table) {
            failures.push(format!("run {} segment from {}: {:?} differs at its end", segment.run, segment.first, TableId::ALL[table]));
        }
    }
    for id in 0..theirs.dictionary().len().min(ours.dictionary().len()) as u32 {
        let (mut left, mut right) = (None, None);
        if theirs.dictionary().resolve(id, &mut left)? != ours.dictionary().resolve(id, &mut right)? {
            failures.push(format!("run {} segment from {}: dictionary id {id} differs", segment.run, segment.first));
            break;
        }
    }
    Ok((failures, decode_ns))
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--journal") {
        let usage = "columnar-convert --journal <campaign directory> <capture> <written.h4col>";
        let (Some(directory), Some(capture), Some(written)) = (args.get(1), args.get(2), args.get(3)) else {
            return Err(invalid(usage));
        };
        let capture = capture.parse().map_err(|_| invalid(usage))?;
        let report = journal::check(Path::new(directory), capture, Path::new(written))?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    let usage = "columnar-convert <source.h4col> <out.h4col> [--threads n] [--level n] [--check]";
    let (Some(from), Some(to)) = (args.first(), args.get(1)) else {
        return Err(invalid(usage));
    };
    let option = |name: &str| args.iter().position(|arg| arg == name).and_then(|at| args.get(at + 1)).cloned();
    let threads: usize = option("--threads").map_or(Ok(16), |value| value.parse()).map_err(|_| invalid("--threads"))?;
    let level: i32 = option("--level").map_or(Ok(Settings::HIGH.level), |value| value.parse()).map_err(|_| invalid("--level"))?;
    let check_written = args.iter().any(|arg| arg == "--check");
    rayon::ThreadPoolBuilder::new().num_threads(threads).build_global().map_err(io::Error::other)?;
    let wall = Instant::now();
    let source = Source::open(Path::new(from))?;
    let kinds = schema()?;
    let opened_s = wall.elapsed().as_secs_f64();

    // Every segment's blocks, made on its own worker from its start, which one walk of each run
    // hands on as it reaches it.
    let starts: std::sync::Mutex<Vec<(Segment, Snapshot)>> = std::sync::Mutex::new(Vec::new());
    let made: std::sync::Mutex<Vec<Made>> = std::sync::Mutex::new(Vec::new());
    let failed: std::sync::Mutex<Option<io::Error>> = std::sync::Mutex::new(None);
    rayon::scope(|scope| {
        for run in 0..source.runs.len() {
            let walked = source.starts(run, |segment, snapshot| {
                let snapshot = std::sync::Arc::new(snapshot);
                if check_written {
                    starts.lock().unwrap().push((segment, Snapshot::clone_of(&snapshot)));
                }
                let (source, made, failed) = (&source, &made, &failed);
                scope.spawn(move |_| match make(source, segment, &snapshot) {
                    Ok(done) => made.lock().unwrap().push(done),
                    Err(error) => *failed.lock().unwrap() = Some(error),
                });
                Ok(())
            });
            if let Err(error) = walked {
                *failed.lock().unwrap() = Some(error);
            }
        }
    });
    if let Some(error) = failed.into_inner().unwrap() {
        return Err(error);
    }
    let mut made = made.into_inner().unwrap();
    made.sort_by_key(|made| (made.segment.run, made.segment.first));
    let mut starts = starts.into_inner().unwrap();
    starts.sort_by_key(|(segment, _)| (segment.run, segment.first));
    let made_s = wall.elapsed().as_secs_f64() - opened_s;
    let hours: u64 = made.iter().map(|made| made.hours).sum();
    let build_ns: u64 = made.iter().map(|made| made.build_ns).sum();
    let held_peak = made.iter().map(|made| made.held_peak).max().unwrap_or(0);

    // The file, written in order: each run's blocks.
    let identity = Identity {
        schema: envelope::schema_digest(&kinds),
        campaign: source.root.identity.campaign.clone(),
        content: source.root.identity.content.clone(),
        capture_mode: source.root.identity.capture_mode,
    };
    let began = Instant::now();
    let file = BufWriter::new(File::create(to)?);
    let settings = Settings {
        level,
        parallel: true,
    };
    let mut writer = FileWriter::create(file, identity, kinds.len(), settings)?;
    let mut raw_tables = vec![0u64; kinds.len()];
    let mut raw_records = 0u64;
    let mut current = None;
    for made in &made {
        if current != Some(made.segment.run) {
            let run = &source.runs[made.segment.run];
            writer.start_run(run.branch, run.epoch, run.first_hour)?;
            current = Some(made.segment.run);
        }
        for block in &made.blocks {
            raw_records += block.records.len() as u64;
            for (table, payload) in &block.tables {
                raw_tables[*table as usize] += payload.len() as u64;
            }
            writer.block(block.block())?;
        }
    }
    let capture = source.capture()?;
    let finished = match &capture {
        Some(capture) => writer.finish_with_capture(capture, |file| io::Write::flush(file))?,
        None => writer.finish(|file| io::Write::flush(file))?,
    };
    drop(finished.file);
    let written_s = began.elapsed().as_secs_f64();

    // What each file stores, by table.
    let written = Recording::open_path(to)?;
    let mut stored = vec![0u64; kinds.len()];
    let mut restarts = vec![0u64; kinds.len()];
    let (mut records, mut index) = (0u64, 0u64);
    for run in 0..written.run_count() {
        for at in 0..written.run(run)?.pages() {
            let page = written.page(run, at)?;
            records += page.records.stored;
            for table in &page.tables {
                stored[table.stream as usize] += table.stored;
            }
            if let Some(restart) = &page.restart {
                index += restart.chunk.stored;
                for table in &restart.tables {
                    restarts[table.table as usize] += table.extent.stored;
                }
            }
        }
    }
    let (source_units, source_restarts, source_manifests) = source.stored(kinds.len());
    let source_total = std::fs::metadata(from)?.len();
    let total = std::fs::metadata(to)?.len();

    // The gate.
    let mut failures = Vec::new();
    let mut decode_ns = 0u64;
    let mut checked_s = 0.0;
    if check_written {
        let began = Instant::now();
        let pairs: Vec<(&(Segment, Snapshot), replay::columnar_frames::Segment)> = starts
            .iter()
            .zip(written.segments().iter().copied())
            .collect();
        if pairs.len() != source.segments.len() || written.segments().len() != source.segments.len() {
            failures.push(format!("the written file holds {} segments of {}", written.segments().len(), source.segments.len()));
        }
        let checked: Vec<(Vec<String>, u64)> = pairs
            .par_iter()
            .map(|((segment, start), v4)| {
                if (segment.first, segment.last) != (v4.first, v4.last) {
                    return Ok((vec![format!("segment from {} is from {} in the written file", segment.first, v4.first)], 0));
                }
                check(&source, start, &written, segment, v4)
            })
            .collect::<io::Result<_>>()?;
        for (found, ns) in checked {
            failures.extend(found);
            decode_ns += ns;
        }
        checked_s = began.elapsed().as_secs_f64();
    }

    let mut rows: Vec<_> = (0..kinds.len())
        .map(|table| {
            json!({
                "table": format!("{:?}", TableId::ALL[table]),
                "source_units": source_units[table],
                "source_restarts": source_restarts[table],
                "raw": raw_tables[table],
                "stored": stored[table],
                "restarts": restarts[table],
            })
        })
        .collect();
    rows.sort_by_key(|row| std::cmp::Reverse(row["source_units"].as_u64()));
    let per_hour = |ns: u64| ns as f64 / hours.max(1) as f64 / 1e3;
    let written_bytes = writer_written(&finished.written);
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "hours": hours,
            "source_bytes": source_total,
            "bytes": total,
            "ratio": source_total as f64 / total.max(1) as f64,
            "source_manifests": source_manifests,
            "records": records,
            "records_raw": raw_records,
            "source_restarts": source_restarts.iter().sum::<u64>(),
            "restarts": restarts.iter().sum::<u64>() + index,
            "source_units": source_units.iter().sum::<u64>(),
            "tables_stored": stored.iter().sum::<u64>(),
            "written": written_bytes,
            "open_s": opened_s,
            "make_s": made_s,
            "write_s": written_s,
            "check_s": checked_s,
            "build_us_per_hour": per_hour(build_ns),
            "walk_check_us_per_hour": per_hour(decode_ns),
            "builder_state_peak_bytes": held_peak,
            "failures": failures.len(),
            "first_failures": failures.iter().take(20).collect::<Vec<_>>(),
            "tables": rows,
        }))
        .map_err(io::Error::other)?
    );
    Ok(())
}

fn writer_written(written: &replay::columnar_file::Written) -> serde_json::Value {
    json!({
        "records": written.records,
        "tables": written.tables,
        "restarts": written.restarts,
        "index": written.index,
    })
}
