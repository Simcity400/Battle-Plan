//! A followed log over a synthetic log made in memory (no recording holds a log yet): the hook's
//! time a step and a row, the message's bytes a step, and what follow and the playhead hold. Dev
//! tool, deleted with the playback build's last step. Prints one JSON line.
//!
//! `follow-log-bench [--rows=10] [--blocks=20] [--every=24] [--listed=1000] [--standing]`: a log
//! appending `rows` rows every `every` hours for `blocks` blocks, its oldest forgotten while more
//! than `listed` are listed, followed as a feed of its four columns (`--standing`: as the rows it
//! lists now), played flat (the head always the newest step built, so every step is `every`
//! hours but at a block's end) from its first hour to its last.

use dashboard_follow::{Follow, Message, Query, Rows, Sink, StepStats};
use playhead::{
    Playhead, Settings, Timeline,
    pages::{Pages, Run},
};
use replay_columnar::{
    manifest::Status,
    state::{Cell, Kind},
    store::Store,
    unit::{BLOCK, BlockPage, Encoder, PartSink, Settings as Writing, reader::Extents},
};
use replay_data::{Hour, rows::TableId};
use serde_json::json;
use std::{
    io,
    sync::{Arc, Mutex},
    time::Instant,
};

const KINDS: [Kind; 4] = [Kind::I64, Kind::U32, Kind::I32, Kind::U16];

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[derive(Default)]
struct Memory {
    bytes: Vec<u8>,
    pages: Vec<BlockPage>,
}

impl PartSink for Memory {
    fn frame(&mut self, stored: &[u8]) -> io::Result<u64> {
        let offset = self.bytes.len() as u64;
        self.bytes.extend_from_slice(stored);
        Ok(offset)
    }

    fn block(&mut self, page: BlockPage) -> io::Result<()> {
        self.pages.push(page);
        Ok(())
    }

    fn read_back(&mut self, offset: u64, stored: u32, out: &mut Vec<u8>) -> io::Result<()> {
        out.clear();
        out.extend_from_slice(&self.bytes[offset as usize..offset as usize + stored as usize]);
        Ok(())
    }
}

struct Log {
    bytes: Vec<u8>,
    runs: Vec<Run>,
    kinds: Vec<Vec<Kind>>,
}

impl Pages for Log {
    fn extents(&self, _run: usize) -> &dyn Extents {
        &self.bytes
    }

    fn runs(&self) -> &[Run] {
        &self.runs
    }

    fn kinds(&self, table: usize) -> Option<&[Kind]> {
        self.kinds.get(table).map(Vec::as_slice)
    }
}

struct Options {
    rows: u64,
    blocks: u32,
    every: u32,
    listed: u64,
    standing: bool,
}

/// The log in `table`'s place (every other table empty), and the rows it appended.
fn make(table: usize, options: &Options) -> io::Result<(Arc<Log>, u64)> {
    let kinds = vec![KINDS.to_vec(); table + 1];
    let mut store = Store::new(kinds.clone(), vec![false; kinds.len()]);
    let mut logs = vec![false; kinds.len()];
    logs[table] = true;
    store.declare_logs(&logs)?;
    store.attach(Encoder::new(&kinds, Writing::at(3))?);
    let mut memory = Memory::default();
    store.begin_run(0);
    let (hours, every) = (options.blocks * BLOCK, u64::from(options.every));
    let (mut count, mut oldest) = (0_u64, 0_u64);
    for at in 0..hours {
        store.begin_hour(at as Hour, &[])?;
        store.status(table, Status::Complete, true, Some(at as Hour), true)?;
        let mut changes = store.table(table)?;
        let due = (u64::from(at) + 1) * options.rows / every - u64::from(at) * options.rows / every;
        for _ in 0..due {
            let cells = [Some(Cell::I64(count as i64 * -1_000_003)), Some(Cell::U32(count as u32 * 7)), Some(Cell::I32(at as i32)), Some(Cell::U16(count as u16))];
            changes.append(u128::from(count) << 8 | 1, &cells)?;
            count += 1;
        }
        if count - oldest > options.listed {
            changes.forget_run(oldest, count - options.listed - oldest)?;
            oldest = count - options.listed;
        }
        store.end_hour(&mut memory)?;
        if at % BLOCK == BLOCK - 1 {
            store.end_block();
        }
    }
    Ok((Arc::new(Log { bytes: memory.bytes, runs: vec![Run::new(0, memory.pages)?], kinds }), count))
}

fn percentiles(values: &[u64]) -> serde_json::Value {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let at = |q: f64| sorted.get(((sorted.len() as f64 - 1.0) * q).round() as usize).copied().unwrap_or(0);
    json!({ "n": sorted.len(), "p50": at(0.5), "p99": at(0.99), "worst": sorted.last().copied().unwrap_or(0), "sum": sorted.iter().sum::<u64>() })
}

/// What the playhead holds, by part: the lanes' cells (a log's count and live set), the parts
/// under their heads and read ahead, the ring.
fn holds(report: &playhead::Report) -> serde_json::Value {
    json!({ "cells": report.held.cells, "parts": report.held.parts, "part_bytes": report.held.part_bytes, "track_bytes": report.held.track_bytes, "ring_bytes": report.ring_bytes, "total": report.total })
}

fn main() -> io::Result<()> {
    let mut options = Options { rows: 10, blocks: 20, every: 24, listed: 1_000, standing: false };
    for option in std::env::args().skip(1) {
        if option == "--standing" {
            options.standing = true;
            continue;
        }
        let (name, value) = option.split_once('=').ok_or_else(|| invalid(format!("not an option: {option}")))?;
        let number = value.parse::<u64>().map_err(|_| invalid(format!("not a number: {option}")))?;
        match name {
            "--rows" => options.rows = number,
            "--blocks" => options.blocks = number as u32,
            "--every" => options.every = number.max(1) as u32,
            "--listed" => options.listed = number,
            _ => return Err(invalid(format!("no option {name}"))),
        }
    }
    let table = TableId::ALL.iter().position(|id| *id == TableId::Countries).ok_or_else(|| invalid("no Countries"))?;
    let made = Instant::now();
    let (pages, appended) = make(table, &options)?;
    let made_s = made.elapsed().as_secs_f64();
    let last = (options.blocks * BLOCK) as Hour - 1;
    let seen: Arc<Mutex<Vec<StepStats>>> = Arc::default();
    let held = Arc::clone(&seen);
    let sink: Sink = Box::new(move |stats: &StepStats, _: Option<Message>| held.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(*stats));
    let follow = Follow::new(0);
    let settings = Settings { step_threads: 1, fetch_threads: 1, ahead: 4, ring: 3, speed: f64::from(options.every) * 60.0, rate: 60.0, ..Settings::default() };
    let playhead = Arc::new(Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages)?, settings, Some(follow.hook(sink)))?);
    let reader = playhead.declare(Vec::new(), None)?;
    let mut consumer = playhead.consumer(reader);
    let weak = Arc::downgrade(&playhead);
    follow.bind(move |tables, _joining| weak.upgrade().map_or(Ok(()), |playhead| playhead.redeclare(reader, tables)), playhead.log_reads())?;
    let rows = if options.standing { Rows::Listed } else { Rows::Appended };
    follow.follow(1, Query::Cells { table, columns: (0..KINDS.len()).collect(), rows })?;
    playhead.seek(0);
    let started = Instant::now();
    let (mut taken, mut peak_follow) = (Vec::new(), 0);
    loop {
        let head = playhead.newest().unwrap_or(0);
        playhead.advance(head);
        consumer.take(head, &mut taken);
        taken.clear();
        if let Some(error) = playhead.error() {
            return Err(invalid(error));
        }
        peak_follow = peak_follow.max(follow.held().total);
        if head >= last && seen.lock().unwrap_or_else(std::sync::PoisonError::into_inner).last().is_some_and(|stats| stats.last >= last) {
            break;
        }
        std::thread::yield_now();
    }
    let wall_s = started.elapsed().as_secs_f64();
    let seen = seen.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    let steps: Vec<&StepStats> = seen.iter().filter(|stats| !stats.reset && !stats.gap).collect();
    if steps.is_empty() {
        return Err(invalid("no step after the reset"));
    }
    let ns: Vec<u64> = steps.iter().map(|stats| stats.ns).collect();
    let bytes: Vec<u64> = steps.iter().filter(|stats| stats.sent > 0).map(|stats| u64::from(stats.bytes)).collect();
    let counters = follow.counters();
    // The rows a step appended, from the log's rule: `rows` every `every` hours.
    let rows_in = |stats: &StepStats| (i64::from(stats.last) + 1) * options.rows as i64 / i64::from(options.every) - i64::from(stats.first) * options.rows as i64 / i64::from(options.every);
    let rows: i64 = steps.iter().map(|stats| rows_in(stats)).sum();
    let hours: Vec<u64> = steps.iter().map(|stats| (stats.last - stats.first + 1) as u64).collect();
    let rows_a_step = rows as f64 / steps.len() as f64;
    println!(
        "{}",
        json!({
            "query": if options.standing { "listed" } else { "appended" }, "rows_a_step": rows_a_step, "every": options.every, "blocks": options.blocks, "appended": appended, "listed": options.listed,
            "steps": steps.len(), "step_hours": percentiles(&hours), "hook_ns": percentiles(&ns), "hook_ns_a_row_mean": ns.iter().sum::<u64>() as f64 / rows as f64,
            "message_bytes": percentiles(&bytes), "messages": counters.messages,
            "follow_held_peak": peak_follow, "follow_held_end": follow.held().total,
            "playhead_held_peak": holds(&playhead.held().1), "playhead_held_end": holds(&playhead.held().0),
            "unit_bytes": pages.bytes.len(), "made_s": made_s, "wall_s": wall_s, "last": last,
        })
    );
    drop(follow);
    if let Ok(playhead) = Arc::try_unwrap(playhead) {
        playhead.finish();
    }
    Ok(())
}
