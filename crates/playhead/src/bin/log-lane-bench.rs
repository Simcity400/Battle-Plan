
use playhead::{
    Wanted,
    changes::Changes,
    lane::{Lane, log::rows},
    pages::{Pages, Run},
};
use replay_columnar::{
    manifest::Status,
    state::{Cell, Kind},
    store::{Store, TableChanges},
    unit::{
        BLOCK, BlockPage, Encoder, PartSink, Settings,
        reader::{self, Extents, LogCursor, LogEvent},
    },
};
use serde_json::json;
use std::{io, sync::Arc, time::Instant};

const NUMERIC: [Kind; 4] = [Kind::I64, Kind::U32, Kind::I32, Kind::U16];
const TEXT: u32 = 1 << 30;
const NAMES: u32 = 64;

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

fn make(kinds: &[Kind], hours: u32, mut hour: impl FnMut(u32, &mut TableChanges<'_>) -> io::Result<()>) -> io::Result<Arc<Log>> {
    let all = vec![kinds.to_vec()];
    let mut store = Store::new(all.clone(), vec![false]);
    store.declare_logs(&[true])?;
    store.attach(Encoder::new(&all, Settings::at(3))?);
    let mut memory = Memory::default();
    store.begin_run(0);
    for id in 0..NAMES {
        store.pin_string(TEXT + id, format!("name {id}").as_bytes())?;
    }
    for at in 0..hours {
        store.begin_hour(at as i32, &[])?;
        store.status(0, Status::Complete, true, Some(at as i32), true)?;
        hour(at, &mut store.table(0)?)?;
        store.end_hour(&mut memory)?;
        if at % BLOCK == BLOCK - 1 {
            store.end_block();
        }
    }
    if !hours.is_multiple_of(BLOCK) {
        store.end_run(&mut memory)?;
    }
    Ok(Arc::new(Log { bytes: memory.bytes, runs: vec![Run::new(0, memory.pages)?], kinds: all }))
}

fn row(kinds: &[Kind], seed: u64) -> Vec<Option<Cell<'static>>> {
    kinds
        .iter()
        .map(|kind| {
            Some(match kind {
                Kind::I64 => Cell::I64(seed as i64 * -1_000_003),
                Kind::U32 => Cell::U32(seed as u32 * 7),
                Kind::I32 => Cell::I32(seed as i32 % 5_000 - 2_500),
                Kind::U16 => Cell::U16(seed as u16),
                Kind::Bytes => Cell::Bytes(TEXT + (seed % u64::from(NAMES)) as u32),
                other => unreachable!("no bench column of {other:?}"),
            })
        })
        .collect()
}

fn append(changes: &mut TableChanges<'_>, kinds: &[Kind], seed: u64) -> io::Result<u64> {
    changes.append(u128::from(seed) << 8 | 1, &row(kinds, seed))
}


fn lane(log: &Arc<Log>) -> io::Result<Lane> {
    let columns: Vec<usize> = (0..log.kinds[0].len()).collect();
    Lane::new(Arc::clone(log) as Arc<dyn Pages>, 0, 0, &columns, true)
}

#[derive(Default)]
struct Timed {
    steps: Vec<u64>,
    appended: u64,
    forgotten: u64,
    runs: u64,
    peak_held: usize,
    peak_state: usize,
}

fn step_through(lane: &mut Lane, step: i32, to: i32, changes: &mut Changes) -> io::Result<Timed> {
    let mut timed = Timed::default();
    while lane.head().ok_or_else(|| invalid("not sought"))? < to {
        let target = (lane.head().expect("sought") + step).min(to);
        lane.fetch_through(target)?;
        let started = Instant::now();
        let head = lane.step(target, changes)?;
        timed.steps.push(started.elapsed().as_nanos() as u64);
        if head != target {
            return Err(invalid(format!("the step to {target} stopped at {head}")));
        }
        let log = changes.log().ok_or_else(|| invalid("not a log"))?;
        timed.appended += log.len() as u64;
        timed.forgotten += log.forgotten().iter().map(|run| run.to - run.from).sum::<u64>();
        timed.runs += log.forgotten().len() as u64;
        let held = lane.held();
        timed.peak_held = timed.peak_held.max(held.total());
        timed.peak_state = timed.peak_state.max(lane.log().expect("a log").bytes());
        let mut retired = Vec::new();
        lane.retire_into(&mut retired);
    }
    Ok(timed)
}

fn percentiles(values: &[u64]) -> serde_json::Value {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let at = |q: f64| sorted.get(((sorted.len() as f64 - 1.0) * q).round() as usize).copied().unwrap_or(0);
    json!({ "n": sorted.len(), "p50": at(0.5), "p99": at(0.99), "worst": sorted.last().copied().unwrap_or(0), "sum": sorted.iter().sum::<u64>() })
}

fn best(repeat: usize, mut run: impl FnMut() -> io::Result<Timed>) -> io::Result<Timed> {
    let mut kept: Option<Timed> = None;
    for _ in 0..repeat {
        let timed = run()?;
        if kept.as_ref().is_none_or(|kept| timed.steps.iter().sum::<u64>() < kept.steps.iter().sum::<u64>()) {
            kept = Some(timed);
        }
    }
    kept.ok_or_else(|| invalid("no run"))
}

fn reader_alone(log: &Log) -> io::Result<(u64, u64)> {
    let (kinds, run) = (&log.kinds[0], &log.runs[0]);
    let columns: Vec<usize> = (0..kinds.len()).collect();
    let (mut ns, mut rows) = (0, 0);
    for page in run.held_pages().expect("held log pages") {
        let entry = &page.tables[0];
        let mut parts = Vec::new();
        for (index, part) in entry.parts.iter().enumerate() {
            let last = entry.parts.get(index + 1).map_or(page.hours - 1, |following| following.first - 1);
            let previous = parts.last().map(reader::Part::prefix);
            parts.push(reader::fetch(&log.bytes, part, last, &columns, previous.as_ref())?);
        }
        let Some(first) = entry.parts.first() else { continue };
        let started = Instant::now();
        let mut cursor = LogCursor::new(kinds, &columns, first.first);
        parts.into_iter().for_each(|part| cursor.push(part));
        cursor.advance(page.hours - 1, &mut |event| rows += u64::from(matches!(event, LogEvent::Appended { .. })))?;
        ns += started.elapsed().as_nanos() as u64;
    }
    Ok((ns, rows))
}

fn bench_append(repeat: usize) -> io::Result<()> {
    let text = [Kind::Bytes];
    let both = [NUMERIC.as_slice(), text.as_slice()].concat();
    for (name, kinds) in [("4 numeric", NUMERIC.as_slice()), ("1 text", text.as_slice()), ("4 numeric + 1 text", both.as_slice())] {
        let log = make(kinds, 2 * BLOCK, |hour, changes| (0..24).try_for_each(|index| append(changes, kinds, u64::from(hour) * 24 + index).map(drop)))?;
        let timed = best(repeat, || {
            let mut lane = lane(&log)?;
            lane.seek(0)?;
            step_through(&mut lane, 24, 2 * BLOCK as i32 - 1, &mut Changes::default())
        })?;
        let sum: u64 = timed.steps.iter().sum();
        let alone = (0..repeat).map(|_| reader_alone(&log)).collect::<io::Result<Vec<_>>>()?;
        let (alone_ns, alone_rows) = alone.iter().min().copied().ok_or_else(|| invalid("no run"))?;
        println!("{}", json!({ "bench": "append", "columns": name, "rows": timed.appended, "ns_a_row": sum as f64 / timed.appended as f64, "reader_alone_ns_a_row": alone_ns as f64 / alone_rows as f64, "step_ns": percentiles(&timed.steps), "unit_bytes": log.bytes.len() }));
    }
    Ok(())
}

fn bench_forget(repeat: usize) -> io::Result<()> {
    let kinds = [Kind::U32];
    let log = make(&kinds, 5 * BLOCK, |hour, changes| {
        let (block, at) = (hour / BLOCK, hour % BLOCK);
        match block {
            0 => (0..100).try_for_each(|index| append(changes, &kinds, u64::from(hour) * 100 + index).map(drop)),
            1 => (at * 50..at * 50 + 50).step_by(2).try_for_each(|ordinal| changes.forget(u64::from(ordinal))),
            2 => changes.forget_run(u64::from(36_000 + at * 50), 50),
            4 if at == 0 || at == BLOCK - 1 => append(changes, &kinds, u64::from(hour)).map(drop),
            _ => Ok(()),
        }
    })?;
    let block = BLOCK as i32;
    let phase = |first: i32, step: i32| {
        best(repeat, || {
            let mut lane = lane(&log)?;
            lane.seek(first - 1)?;
            step_through(&mut lane, step, first + block - 1, &mut Changes::default())
        })
    };
    let lone = phase(block, 24)?;
    let runs = phase(2 * block, 24)?;
    for (name, timed) in [("lone", &lone), ("runs of 50", &runs)] {
        let sum: u64 = timed.steps.iter().sum();
        println!("{}", json!({ "bench": "forget", "forgets": name, "rows": timed.forgotten, "runs": timed.runs, "ns_a_row": sum as f64 / timed.forgotten as f64, "ns_a_run": sum as f64 / timed.runs as f64, "step_ns": percentiles(&timed.steps) }));
    }
    let timer: Vec<u64> = (0..1_000).map(|_| Instant::now().elapsed().as_nanos() as u64).collect();
    for (name, (from, through), step) in [
        ("a block holding no part, 1 h", (3 * block - 1, 4 * block - 1), 1),
        ("a block holding no part, 24 h", (3 * block - 1, 4 * block - 1), 24),
        ("inside a part, 1 h", (4 * block, 5 * block - 26), 1),
        ("inside a part, 24 h", (4 * block, 5 * block - 26), 24),
    ] {
        let timed = best(repeat, || {
            let mut lane = lane(&log)?;
            lane.seek(from)?;
            step_through(&mut lane, step, through, &mut Changes::default())
        })?;
        let mut loops = Vec::new();
        for _ in 0..repeat {
            let mut lane = lane(&log)?;
            lane.seek(from)?;
            lane.fetch_through(through + 1)?;
            let mut changes = Changes::default();
            let (mut steps, mut appended) = (0_u64, 0);
            let started = Instant::now();
            while lane.head().expect("sought") < through {
                lane.step((lane.head().expect("sought") + step).min(through), &mut changes)?;
                appended += changes.log().expect("a log").len();
                steps += 1;
            }
            loops.push((started.elapsed().as_nanos() as u64 / steps, appended));
        }
        let (loop_ns, appended) = loops.iter().min().copied().expect("a loop");
        println!("{}", json!({ "bench": "no event", "where": name, "steps_with_an_event": appended, "loop_ns_a_step": loop_ns, "timed_step_ns": percentiles(&timed.steps), "timer_ns": percentiles(&timer) }));
    }
    Ok(())
}

fn listed_log(rate: u32, hours: u32, listed: u32) -> io::Result<Arc<Log>> {
    let kinds = [NUMERIC.as_slice(), &[Kind::Bytes]].concat();
    let mut oldest = 0;
    make(&kinds, hours, |hour, changes| {
        for index in 0..rate {
            append(changes, &kinds, u64::from(hour) * u64::from(rate) + u64::from(index))?;
        }
        let count = u64::from(hour + 1) * u64::from(rate);
        if listed > 0 && count - oldest > u64::from(listed) {
            changes.forget_run(oldest, count - u64::from(listed) - oldest)?;
            oldest = count - u64::from(listed);
        }
        Ok(())
    })
}

fn bench_seek(repeat: usize) -> io::Result<()> {
    for (listed, log) in [(10_000_u32, listed_log(1_389, 3 * BLOCK, 10_000)?), (2_000_000, listed_log(1_389, 3 * BLOCK, 0)?)] {
        for hour in [2 * BLOCK as i32 - 1, 2 * BLOCK as i32 + 360, 3 * BLOCK as i32 - 1] {
            let (mut ms, mut restore, mut fetch, mut replay, mut rest) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
            let mut sought = None;
            for _ in 0..repeat.max(5) {
                let mut lane = lane(&log)?;
                let started = Instant::now();
                let took = lane.seek(hour)?;
                let ns = started.elapsed().as_nanos() as u64;
                ms.push(ns);
                restore.push(took.restore_ns);
                fetch.push(took.fetch_ns);
                replay.push(took.replay_ns);
                rest.push(ns.saturating_sub(took.restore_ns + took.fetch_ns + took.replay_ns));
                sought = Some((took, lane.log().expect("a log").live().len(), lane.held().total(), lane.log().expect("a log").bytes()));
            }
            let (took, live, held, state) = sought.expect("sought");
            let replayed = hour as u64 % u64::from(BLOCK) + 1;
            println!("{}", json!({ "bench": "seek", "listed": listed, "hour": hour, "live": live, "hours_replayed": if replayed == u64::from(BLOCK) { 0 } else { replayed }, "seek_ns": percentiles(&ms), "restore_ns": percentiles(&restore), "fetch_ns": percentiles(&fetch), "replay_ns": percentiles(&replay), "live_set_and_rest_ns": percentiles(&rest), "parts": took.parts, "held_bytes": held, "state_bytes": state }));
        }
    }
    Ok(())
}

fn bench_held_and_rows(repeat: usize, held: bool, asked: bool) -> io::Result<()> {
    let mut largest = None;
    for blocks in [1, 10] {
        let started = Instant::now();
        let log = listed_log(1_389, blocks * BLOCK, 10_000)?;
        let made = started.elapsed().as_secs_f64();
        if held {
            let mut lane = lane(&log)?;
            lane.seek(0)?;
            let timed = step_through(&mut lane, 24, (blocks * BLOCK) as i32 - 1, &mut Changes::default())?;
            let state = lane.log().expect("a log");
            println!("{}", json!({ "bench": "held", "appended": state.count(), "listed": state.live().len(), "state_bytes": state.bytes(), "peak_state_bytes": timed.peak_state, "peak_lane_bytes": timed.peak_held, "end_lane_bytes": lane.held().total(), "unit_bytes": log.bytes.len(), "made_s": made, "step_ns": percentiles(&timed.steps) }));
        }
        largest = Some(log);
    }
    if asked {
        let log = largest.expect("made");
        let count = 10 * BLOCK as u64 * 1_389;
        let spread: Vec<u64> = (0..1_000).map(|index| index * (count / 1_000) + index % 997).collect();
        let listed: Vec<u64> = (0..1_000).map(|index| count - 10_000 + index * 10).collect();
        let columns: Vec<usize> = (0..5).collect();
        for (name, ordinals) in [("spread over the log", spread), ("spread over the 10,000 listed", listed)] {
            let mut ms = Vec::new();
            let mut read = None;
            for _ in 0..repeat {
                let started = Instant::now();
                let got = rows(&*log, &Wanted { run: 0, table: 0, ordinals: ordinals.clone(), columns: columns.clone() })?;
                ms.push(started.elapsed().as_nanos() as u64);
                if got.len() != ordinals.len() {
                    return Err(invalid(format!("{} rows of {} asked", got.len(), ordinals.len())));
                }
                read = Some((got.read(), got.texts().len()));
            }
            let (read, texts) = read.expect("read");
            println!("{}", json!({ "bench": "rows", "asked": ordinals.len(), "where": name, "of": count, "ns": percentiles(&ms), "blocks": read.blocks, "parts": read.parts, "walked": read.walked, "stored": read.stored, "decompressed": read.decompressed, "fetch_ns": read.fetch_ns, "rows_ns": read.rows_ns, "texts": texts }));
        }
    }
    Ok(())
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = "log-lane-bench <append|forget|seek|held|rows|all> [--repeat=3]";
    let command = args.first().ok_or_else(|| invalid(usage))?;
    let mut repeat = 3;
    for option in &args[1..] {
        repeat = option.strip_prefix("--repeat=").and_then(|value| value.parse().ok()).ok_or_else(|| invalid(usage))?;
    }
    match command.as_str() {
        "append" => bench_append(repeat),
        "forget" => bench_forget(repeat),
        "seek" => bench_seek(repeat),
        "held" => bench_held_and_rows(repeat, true, false),
        "rows" => bench_held_and_rows(repeat, false, true),
        "all" => {
            bench_append(repeat)?;
            bench_forget(repeat)?;
            bench_seek(repeat)?;
            bench_held_and_rows(repeat, true, true)
        }
        _ => Err(invalid(usage)),
    }
}
