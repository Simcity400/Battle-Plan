//! What the rows workers did, counted while frames are measured (`replay_frame::cpu`): taken once a
//! presented frame by the paced report ([`take`]), and summed over the run ([`totals`]). Every
//! worker of the process adds to the same tally.

use super::{Make, Whole};
use replay_data::{
    Hour,
    rows::{TABLE_COUNT, TableId},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::Mutex,
    time::{Duration, Instant},
};

/// What one phase of a wake spent: wall time, and the worker thread's CPU time (none where the
/// platform clock gives none) and cycles, so the time it was off the CPU shows; and what it
/// allocated, counted where it allocates: the bytes its mirrors grew by, the views it made, the
/// copies of rows a frame held that it took to patch, and the rows it made.
#[derive(Clone, Copy, Default)]
pub struct Spent {
    pub ns: u64,
    pub cpu_ns: Option<u64>,
    pub cycles: u64,
    pub grew: i64,
    pub views: u64,
    pub copies: u64,
    pub rows: u64,
}

impl Spent {
    fn add(&mut self, other: Self) {
        self.ns += other.ns;
        if let Some(cpu) = other.cpu_ns {
            *self.cpu_ns.get_or_insert(0) += cpu;
        }
        self.cycles += other.cycles;
        self.grew += other.grew;
        self.views += other.views;
        self.copies += other.copies;
        self.rows += other.rows;
    }

    fn json(self) -> Value {
        json!({"ms": self.ns as f64 / 1e6, "cpu_ms": self.cpu_ns.map(|ns| ns as f64 / 1e6), "cycles": self.cycles, "grew_bytes": self.grew, "views": self.views, "copies": self.copies, "rows": self.rows})
    }
}

/// A phase being measured on the worker's thread.
pub struct Phase {
    wall: Instant,
    cpu: Option<Duration>,
    cycles: u64,
}

impl Phase {
    pub fn begin() -> Self {
        Self { wall: Instant::now(), cpu: viewer_foundation::profiling::current_thread_cpu_time(), cycles: replay_frame::cpu::thread_cycles() }
    }

    pub fn end(self) -> Spent {
        Spent {
            ns: self.wall.elapsed().as_nanos() as u64,
            cpu_ns: self.cpu.zip(viewer_foundation::profiling::current_thread_cpu_time()).map(|(before, after)| after.saturating_sub(before).as_nanos() as u64),
            cycles: replay_frame::cpu::thread_cycles().saturating_sub(self.cycles),
            ..Spent::default()
        }
    }
}

/// One skip to the head: the steps left unapplied, the bytes they carried for the tables held, and
/// how long until every table held had arrived again.
#[derive(Clone, Copy)]
pub struct Skip {
    pub steps: u64,
    pub bytes: u64,
    pub ns: u64,
}

impl Skip {
    fn json(self) -> Value {
        json!({"steps": self.steps, "bytes": self.bytes, "ms": self.ns as f64 / 1e6})
    }
}

/// One wake of a worker, as it counted it.
#[derive(Default)]
pub struct Wake {
    pub taken: u64,
    pub applied: u64,
    pub apply_ns: u64,
    /// The worker thread's CPU time over the applies; none where the platform gives none.
    pub apply_cpu_ns: Option<u64>,
    /// Per table change set applied: (table, nanoseconds, bytes it carried).
    pub tables: Vec<(usize, u64, u64)>,
    /// Per view made: (table, nanoseconds, rows made, how).
    pub made: Vec<(usize, u64, usize, Make)>,
    /// Frames published, by the reader's place in the code.
    pub frames: Vec<String>,
    /// A skip begun: the steps passed over and the bytes they carried.
    pub skipped: Option<(u64, u64)>,
    pub skip_done: Option<Skip>,
    /// After the wake: the steps built and not taken that end at or before the head, and the last
    /// hour it applied.
    pub behind: Option<(u64, Option<Hour>)>,
    /// Its phases: taking steps from the playhead, applying them to its mirrors, making frames,
    /// publishing them to the readers; the time it slept waiting for a wake, and the time it was
    /// blocked taking the readers' lock.
    pub take: Spent,
    pub apply: Spent,
    pub make: Spent,
    pub publish: Spent,
    pub wait_ns: u64,
    pub lock_ns: u64,
}

#[derive(Clone, Copy, Default)]
struct Table {
    applies: u64,
    apply_ns: u64,
    bytes: u64,
    makes: u64,
    make_ns: u64,
    rows: u64,
    in_place: u64,
    copied: u64,
    keyed: u64,
    /// Made whole, by [`Whole::ALL`].
    whole: [u64; Whole::ALL.len()],
}

#[derive(Default)]
struct Frame {
    taken: u64,
    applied: u64,
    apply_ns: u64,
    apply_cpu_ns: Option<u64>,
    made: u64,
    made_ns: u64,
    frames: BTreeMap<String, u64>,
    asked: BTreeMap<String, u64>,
    skipped: Vec<(u64, u64)>,
    skips: Vec<Skip>,
    wakes: u64,
    phases: [Spent; 4],
    wait_ns: u64,
    lock_ns: u64,
}

struct Tally {
    frame: Frame,
    behind: Option<(u64, Option<Hour>)>,
    tables: Vec<Table>,
    /// Per reader: frames it asked for ([`super::RowReader::ready`]) and frames made for it.
    readers: BTreeMap<String, (u64, u64)>,
    skips: Vec<Skip>,
    begun: u64,
}

static TALLY: Mutex<Option<Tally>> = Mutex::new(None);

pub fn enabled() -> bool {
    replay_frame::cpu::enabled()
}

fn tally(held: &mut Option<Tally>) -> &mut Tally {
    held.get_or_insert_with(|| Tally { frame: Frame::default(), behind: None, tables: vec![Table::default(); TABLE_COUNT], readers: BTreeMap::new(), skips: Vec::new(), begun: 0 })
}

/// A reader that waits until it is ready said it is: a frame asked for, while frames are measured.
pub fn asked(reader: String) {
    if !enabled() {
        return;
    }
    let mut held = TALLY.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let tally = tally(&mut held);
    tally.readers.entry(reader.clone()).or_default().0 += 1;
    *tally.frame.asked.entry(reader).or_default() += 1;
}

/// Adds a wake's counts, while frames are measured.
pub fn record(wake: Wake) {
    if !enabled() {
        return;
    }
    let mut held = TALLY.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let tally = tally(&mut held);
    let frame = &mut tally.frame;
    frame.taken += wake.taken;
    frame.wakes += 1;
    for (into, spent) in frame.phases.iter_mut().zip([wake.take, wake.apply, wake.make, wake.publish]) {
        into.add(spent);
    }
    frame.wait_ns += wake.wait_ns;
    frame.lock_ns += wake.lock_ns;
    frame.applied += wake.applied;
    frame.apply_ns += wake.apply_ns;
    if let Some(cpu) = wake.apply_cpu_ns {
        *frame.apply_cpu_ns.get_or_insert(0) += cpu;
    }
    for (table, ns, bytes) in wake.tables {
        let held = &mut tally.tables[table];
        (held.applies, held.apply_ns, held.bytes) = (held.applies + 1, held.apply_ns + ns, held.bytes + bytes);
    }
    for (table, ns, rows, how) in wake.made {
        frame.made += 1;
        frame.made_ns += ns;
        let held = &mut tally.tables[table];
        (held.makes, held.make_ns, held.rows) = (held.makes + 1, held.make_ns + ns, held.rows + rows as u64);
        match how {
            Make::InPlace => held.in_place += 1,
            Make::Copied => held.copied += 1,
            Make::Keyed => held.keyed += 1,
            Make::Whole(why) => held.whole[Whole::ALL.iter().position(|&each| each == why).unwrap_or_default()] += 1,
        }
    }
    for reader in wake.frames {
        tally.readers.entry(reader.clone()).or_default().1 += 1;
        *frame.frames.entry(reader).or_default() += 1;
    }
    if let Some(skipped) = wake.skipped {
        frame.skipped.push(skipped);
        tally.begun += 1;
    }
    if let Some(skip) = wake.skip_done {
        frame.skips.push(skip);
        tally.skips.push(skip);
    }
    if wake.behind.is_some() {
        tally.behind = wake.behind;
    }
}

/// This frame's counts, with the map's head `head`; none before a worker counted anything.
pub fn take(head: Option<Hour>) -> Value {
    let mut held = TALLY.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(tally) = held.as_mut() else { return Value::Null };
    let frame = std::mem::take(&mut tally.frame);
    let (steps, hour) = tally.behind.unwrap_or((0, None));
    json!({
        "behind_steps": tally.behind.map(|_| steps),
        "behind_hours": head.zip(hour).map(|(head, hour)| head - hour),
        "taken": frame.taken,
        "applied": frame.applied,
        "apply_ms": frame.apply_ns as f64 / 1e6,
        "apply_cpu_ms": frame.apply_cpu_ns.map(|ns| ns as f64 / 1e6),
        "made": frame.made,
        "made_ms": frame.made_ns as f64 / 1e6,
        "frames": frame.frames,
        "frames_asked": frame.asked,
        "skips_begun": frame.skipped.iter().map(|&(steps, bytes)| json!({"steps": steps, "bytes": bytes})).collect::<Vec<_>>(),
        "skips_done": frame.skips.into_iter().map(Skip::json).collect::<Vec<_>>(),
        "wakes": frame.wakes,
        "phases": json!({"take": frame.phases[0].json(), "apply": frame.phases[1].json(), "make": frame.phases[2].json(), "publish": frame.phases[3].json()}),
        "wait_ms": frame.wait_ns as f64 / 1e6,
        "lock_ms": frame.lock_ns as f64 / 1e6,
    })
}

/// The run's counts: per table its applies and views made, per reader its frames, and every skip.
pub fn totals() -> Value {
    let held = TALLY.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(tally) = held.as_ref() else { return Value::Null };
    let mut tables: Vec<(usize, Table)> = tally.tables.iter().copied().enumerate().filter(|(_, table)| table.applies + table.makes > 0).collect();
    tables.sort_by_key(|(_, table)| std::cmp::Reverse(table.apply_ns + table.make_ns));
    json!({
        "tables": tables.into_iter().map(|(index, table)| json!({
            "table": format!("{:?}", TableId::ALL[index]),
            "applies": table.applies, "apply_ms": table.apply_ns as f64 / 1e6, "bytes": table.bytes,
            "makes": table.makes, "make_ms": table.make_ns as f64 / 1e6, "rows_made": table.rows,
            "patched_in_place": table.in_place, "patched_on_a_copy": table.copied, "made_of_a_readers_keys": table.keyed,
            "whole": Whole::ALL.iter().zip(table.whole).filter(|(_, count)| *count > 0).map(|(why, count)| (why.name(), count)).collect::<BTreeMap<_, _>>(),
        })).collect::<Vec<_>>(),
        "frames_by_reader": tally.readers.iter().map(|(reader, &(asked, made))| (reader.clone(), json!({"asked": asked, "made": made}))).collect::<serde_json::Map<_, _>>(),
        "skips_begun": tally.begun,
        "skips_done": tally.skips.iter().copied().map(Skip::json).collect::<Vec<_>>(),
        "scope": "The rows workers over the run while frames were measured: per table the change sets applied to their mirrors (time and bytes) and the views made (time and rows; how many patched in place, on a copy of rows a frame held, and made whole by why), largest first; per reader, named by where the code opens it, the frames it asked for (a reader made rows only when ready says so each time it is: a maker when its product is answered, a frame-thread reader each frame it looks; others ask nothing) and the frames made for it; and each skip to the head: the steps it left unapplied, the bytes they carried, and the time until every table it holds arrived again. Per frame in frames.jsonl, rows_worker.",
    })
}
