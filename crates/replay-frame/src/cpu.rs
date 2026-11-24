
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex, OnceLock, PoisonError,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};

use crate::{frame::TABLE_COUNT, rows::TableId};

static ENABLED: AtomicBool = AtomicBool::new(false);
static BLOCKS: AtomicBool = AtomicBool::new(false);
type BlockLabel = (Option<usize>, &'static str, &'static str, &'static str);
static BLOCK_TOTALS: Mutex<Option<HashMap<BlockLabel, [u64; 2]>>> = Mutex::new(None);
static TSC_HZ: OnceLock<f64> = OnceLock::new();
static REGISTRY: Mutex<Vec<Arc<Mutex<ThreadAcc>>>> = Mutex::new(Vec::new());
static EXITED: Mutex<Option<Exited>> = Mutex::new(None);
static INTERNED: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
static DECODES: Mutex<Option<Decodes>> = Mutex::new(None);
static HOURS: OnceLock<HourBits> = OnceLock::new();
static NOTES: Mutex<Option<HashMap<Label, [u64; 2]>>> = Mutex::new(None);
static COPIES: Mutex<Option<Vec<[u64; 3]>>> = Mutex::new(None);
static REUSES: Mutex<Option<Vec<[u64; 3]>>> = Mutex::new(None);
type LogKey = (&'static str, &'static str);
static LOGGED: Mutex<Option<HashMap<LogKey, [u64; 2]>>> = Mutex::new(None);
static CYCLE_READ_NS: OnceLock<f64> = OnceLock::new();

thread_local! {
    static CYCLE_READS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static DELTA_COPIES: RefCell<[[u64; 2]; TABLE_COUNT]> =
        const { RefCell::new([[0; 2]; TABLE_COUNT]) };
    static DELTA_REUSES: RefCell<[[u64; 2]; TABLE_COUNT]> =
        const { RefCell::new([[0; 2]; TABLE_COUNT]) };
    static FRAME_LOG: RefCell<HashMap<&'static str, [u64; 2]>> = RefCell::new(HashMap::new());
}

#[cfg(any(test, feature = "test-support"))]
pub fn frame_log_counts() -> HashMap<&'static str, [u64; 2]> {
    FRAME_LOG.with(|counts| counts.borrow().clone())
}

pub(crate) fn logged(kind: &'static str, rows: usize) {
    #[cfg(any(test, feature = "test-support"))]
    FRAME_LOG.with(|counts| {
        let mut counts = counts.borrow_mut();
        let value = counts.entry(kind).or_default();
        value[0] += 1;
        value[1] += rows as u64;
    });
    if !enabled() {
        return;
    }
    let family = with_local(|acc| owner(&acc.stack)).unwrap_or("-");
    let mut logged = LOGGED.lock().unwrap_or_else(PoisonError::into_inner);
    let value = logged
        .get_or_insert_with(HashMap::new)
        .entry((kind, family))
        .or_default();
    value[0] += 1;
    value[1] += rows as u64;
}

#[cfg(any(test, feature = "test-support"))]
pub fn delta_copies() -> [[u64; 2]; TABLE_COUNT] {
    DELTA_COPIES.with(|counts| *counts.borrow())
}

pub(crate) fn copied(table: TableId, rows: usize, started: u64) {
    #[cfg(any(test, feature = "test-support"))]
    DELTA_COPIES.with(|counts| {
        let mut values = counts.borrow_mut();
        values[table.index()][0] += 1;
        values[table.index()][1] += rows as u64;
    });
    charge(&COPIES, table, rows, started);
}

#[cfg(any(test, feature = "test-support"))]
pub fn delta_reuses() -> [[u64; 2]; TABLE_COUNT] {
    DELTA_REUSES.with(|counts| *counts.borrow())
}

pub(crate) fn reused(table: TableId, rows: usize, started: u64) {
    #[cfg(any(test, feature = "test-support"))]
    DELTA_REUSES.with(|counts| {
        let mut values = counts.borrow_mut();
        values[table.index()][0] += 1;
        values[table.index()][1] += rows as u64;
    });
    charge(&REUSES, table, rows, started);
}

fn charge(totals: &Mutex<Option<Vec<[u64; 3]>>>, table: TableId, rows: usize, started: u64) {
    if !enabled() {
        return;
    }
    let cycles = stamp().wrapping_sub(started);
    let mut totals = totals.lock().unwrap_or_else(PoisonError::into_inner);
    let counts = &mut totals.get_or_insert_with(|| vec![[0; 3]; TABLE_COUNT])[table.index()];
    counts[0] += 1;
    counts[1] += rows as u64;
    counts[2] += cycles;
}

type Label = (&'static str, &'static str);

#[inline]
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn enable() {
    TSC_HZ.get_or_init(calibrate);
    CYCLE_READ_NS.get_or_init(|| {
        const READS: u32 = 4096;
        let started = Instant::now();
        for _ in 0..READS {
            std::hint::black_box(current_cycles());
        }
        started.elapsed().as_nanos() as f64 / f64::from(READS)
    });
    replay_data::measurement::enable();
    ENABLED.store(true, Ordering::Release);
    job_priority::accounting::install(|| {
        enabled().then(|| {
            let context = Context::capture();
            Box::new(move |category| Box::new(context.enter(category)) as Box<dyn std::any::Any>)
                as job_priority::accounting::Enter
        })
    });
}

pub fn enable_blocks() {
    enable();
    BLOCKS.store(true, Ordering::Release);
}

#[must_use]
pub struct BlockScope(Option<Option<usize>>);

impl Drop for BlockScope {
    fn drop(&mut self) {
        if let Some(previous) = self.0 {
            with_local(|acc| {
                acc.charge(current_cycles());
                acc.block = previous;
            });
        }
    }
}

pub fn block(ordinal: Option<usize>) -> BlockScope {
    if !BLOCKS.load(Ordering::Relaxed) {
        return BlockScope(None);
    }
    BlockScope(with_local(|acc| {
        acc.charge(current_cycles());
        std::mem::replace(&mut acc.block, ordinal)
    }))
}

pub fn current_block() -> Option<usize> {
    if !BLOCKS.load(Ordering::Relaxed) {
        return None;
    }
    with_local(|acc| acc.block).flatten()
}

pub fn process_cpu_ns() -> Option<u64> {
    process_ns()
}

fn owner(stack: &[Label]) -> &'static str {
    stack
        .iter()
        .rev()
        .find(|label| !label.1.contains(':'))
        .map_or("-", |label| label.1)
}

fn phase(stack: &[Label]) -> &'static str {
    stack
        .iter()
        .rev()
        .find_map(|label| match label.0 {
            "inflate" => Some("inflate"),
            "decode" | "decode_wire" | "decode_keyframe" | "decode_shadow" => Some("column decode"),
            "reconstruct" => Some("apply"),
            "anchors" => Some("anchors"),
            "extract" => Some("extract"),
            "fold observe" | "observe" => Some("fold observe"),
            "fold" | "fold driver" | "fold facts" => Some("fold driver"),
            "commit" | "fold block flush" | "fold publish" | "fold flush" | "fold save"
            | "fold continuation" => Some("publication"),
            _ => None,
        })
        .unwrap_or("shared")
}

fn block_label(block: Option<usize>, stack: &[Label]) -> Option<BlockLabel> {
    let category = stack.last()?.0;
    if stack.iter().any(|label| label.0 == "shared commit") {
        Some((None, category, "shared commit", "shared commit"))
    } else {
        Some((block, category, owner(stack), phase(stack)))
    }
}

pub fn block_snapshot() -> Value {
    let mut totals = BLOCK_TOTALS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .unwrap_or_default();
    let registry = REGISTRY
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    for acc in registry {
        let acc = acc.lock().unwrap_or_else(PoisonError::into_inner);
        for (label, value) in &acc.blocks {
            let into = totals.entry(*label).or_default();
            into[0] += value[0];
            into[1] += value[1];
        }
        if let Some(label) = block_label(acc.block, &acc.stack) {
            totals.entry(label).or_default()[0] += acc
                .handle
                .cycles()
                .unwrap_or(acc.last)
                .saturating_sub(acc.last);
        }
    }
    let hz = *TSC_HZ.get_or_init(calibrate);
    let mut rows: Vec<_> = totals.into_iter().collect();
    rows.sort_unstable_by_key(|(key, _)| *key);
    json!(
        rows.into_iter()
            .map(|((block, category, family, phase), value)| json!({
                "block":block, "category":category, "family":family, "phase":phase,
                "ns": (value[0] as f64 / hz * 1e9).round() as u64, "calls":value[1]
            }))
            .collect::<Vec<_>>()
    )
}

pub fn set_hours(first: i32, last: i32) {
    if last < first {
        return;
    }
    let count = (i64::from(last) - i64::from(first) + 1) as usize;
    let words = count.div_ceil(64);
    let _ = HOURS.set(HourBits {
        first,
        count,
        words,
        bits: (0..words * TABLE_COUNT)
            .map(|_| AtomicU64::new(0))
            .collect(),
    });
}

#[must_use]
pub struct Scope(bool);

impl Drop for Scope {
    #[inline]
    fn drop(&mut self) {
        if self.0 {
            with_local(|acc| acc.pop());
        }
    }
}

#[inline]
pub fn scope(category: &'static str, family: Option<&'static str>) -> Scope {
    if !enabled() {
        return Scope(false);
    }
    Scope(with_local(|acc| acc.push(category, family)).is_some())
}

#[inline]
pub fn scope_phase(family: &'static str) -> Scope {
    if !enabled() {
        return Scope(false);
    }
    Scope(
        with_local(|acc| {
            let category = acc.stack.last().map_or("thread", |label| label.0);
            acc.push(category, Some(family));
        })
        .is_some(),
    )
}

pub struct ReaderScope {
    active: bool,
    _table: replay_data::measurement::TableScope,
}

impl Drop for ReaderScope {
    fn drop(&mut self) {
        if self.active {
            with_local(ThreadAcc::pop_reader);
        }
    }
}

#[inline]
pub fn reader(phase: &'static str, table: TableId) -> ReaderScope {
    #[cfg(any(test, feature = "test-support"))]
    READER_CALLS.with(|calls| {
        if let Some(calls) = &mut *calls.borrow_mut() {
            *calls.entry((phase, table)).or_default() += 1;
        }
    });
    let active = if enabled() {
        static NAMES: OnceLock<Vec<&'static str>> = OnceLock::new();
        let names = NAMES.get_or_init(|| {
            TableId::ALL
                .iter()
                .map(|table| intern(&format!("{table:?}")))
                .collect()
        });
        with_local(|acc| acc.push_reader(phase, names[table.index()])).is_some()
    } else {
        false
    };
    ReaderScope {
        active,
        _table: replay_data::measurement::table(table),
    }
}

pub fn scope_named(category: &'static str, family: &str) -> Scope {
    if !enabled() {
        return Scope(false);
    }
    let family = intern(family);
    Scope(with_local(|acc| acc.push(category, Some(family))).is_some())
}

#[derive(Clone, Default)]
pub struct Context(Option<(Vec<Label>, Option<usize>)>);

impl Context {
    pub fn capture() -> Self {
        Self(if enabled() {
            with_local(|acc| (acc.stack.clone(), acc.block))
        } else {
            None
        })
    }

    pub fn enter(self, category: &'static str) -> ContextScope {
        ContextScope(self.0.and_then(|(mut stack, block)| {
            with_local(|acc| {
                acc.charge(current_cycles());
                let family = stack.last().map_or("-", |label| label.1);
                stack.push((category, family));
                let previous = (std::mem::replace(&mut acc.stack, stack), acc.block);
                acc.block = block;
                acc.totals.entry((category, family)).or_default()[1] += 1;
                if BLOCKS.load(Ordering::Relaxed) {
                    acc.blocks
                        .entry(block_label(acc.block, &acc.stack).unwrap())
                        .or_default()[1] += 1;
                }
                previous
            })
        }))
    }
}

#[must_use]
pub struct ContextScope(Option<(Vec<Label>, Option<usize>)>);

impl Drop for ContextScope {
    fn drop(&mut self) {
        if let Some((stack, block)) = self.0.take() {
            with_local(|acc| {
                acc.charge(current_cycles());
                acc.stack = stack;
                acc.block = block;
            });
        }
    }
}

pub fn family() -> Option<&'static str> {
    if !enabled() {
        return None;
    }
    with_local(|acc| acc.stack.last().map(|label| label.1)).flatten()
}

pub fn cycle_reads() -> u64 {
    CYCLE_READS.with(std::cell::Cell::get)
}

pub fn cycle_read_ns() -> Option<f64> {
    CYCLE_READ_NS.get().copied()
}

#[inline]
pub fn thread_cycles() -> u64 {
    if enabled() { current_cycles() } else { 0 }
}

#[inline]
pub fn stamp() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        unsafe { core::arch::x86_64::_rdtsc() }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        0
    }
}

pub fn note(category: &'static str, family: &'static str, started: u64) {
    if !enabled() {
        return;
    }
    let spent = current_cycles().saturating_sub(started);
    let mut notes = NOTES.lock().unwrap_or_else(PoisonError::into_inner);
    let entry = notes
        .get_or_insert_with(HashMap::new)
        .entry((category, family))
        .or_default();
    entry[0] += spent;
    entry[1] += 1;
}

pub fn note_named(category: &'static str, family: &str, started: u64) {
    if enabled() {
        note(category, intern(family), started);
    }
}

pub fn decoded(branch: u64, block_first_hour: i32, table: TableId, cycles: u64) {
    if !enabled() {
        return;
    }
    let family = family().unwrap_or("-");
    let mut decodes = DECODES.lock().unwrap_or_else(PoisonError::into_inner);
    let decodes = decodes.get_or_insert_with(Decodes::default);
    let seen = decodes
        .seen
        .entry((branch, block_first_hour, table.index() as u16))
        .or_default();
    let stats = decodes.tables.entry(table.index()).or_insert([0; 6]);
    if seen.is_empty() {
        stats[0] += cycles;
        stats[1] += 1;
    } else if seen.contains(&family) {
        stats[4] += cycles;
        stats[5] += 1;
    } else {
        stats[2] += cycles;
        stats[3] += 1;
    }
    if !seen.contains(&family) {
        seen.push(family);
    }
    let by = decodes.by_family.entry(family).or_insert([0; 2]);
    by[0] += cycles;
    by[1] += 1;
    let by = decodes
        .by_family_table
        .entry((family, table.index()))
        .or_insert([0; 2]);
    by[0] += cycles;
    by[1] += 1;
}

pub fn reconstructed(hour: i32, tables: &[(usize, u64)]) {
    if !enabled() || tables.is_empty() {
        return;
    }
    let bits = HOURS.get();
    let offset = bits.and_then(|bits| {
        let offset = i64::from(hour) - i64::from(bits.first);
        (offset >= 0 && (offset as usize) < bits.count).then_some(offset as usize)
    });
    with_local(|acc| {
        for &(table, cycles) in tables {
            let slot = &mut acc.tables[table];
            let kind = match (bits, offset) {
                (Some(bits), Some(offset)) => {
                    let word = &bits.bits[table * bits.words + offset / 64];
                    let mask = 1u64 << (offset % 64);
                    if word.fetch_or(mask, Ordering::Relaxed) & mask == 0 {
                        0
                    } else {
                        1
                    }
                }
                _ => 2,
            };
            slot[kind * 2] += cycles;
            slot[kind * 2 + 1] += 1;
        }
    });
}

pub struct OwnLabel {
    pub category: &'static str,
    pub family: &'static str,
    pub ns: u64,
    pub wall_ns: Option<u64>,
    pub calls: u64,
}

pub fn own_labels() -> Vec<OwnLabel> {
    if !enabled() {
        return Vec::new();
    }
    let hz = *TSC_HZ.get_or_init(calibrate);
    let ns = |cycles: u64| (cycles as f64 / hz * 1e9).round() as u64;
    with_local(|acc| {
        let pending = current_cycles().saturating_sub(acc.last);
        let open_wall = Instant::now().saturating_duration_since(acc.last_wall).as_nanos() as u64;
        let top = acc.stack.last().copied();
        acc.totals
            .iter()
            .map(|(&label, &[cycles, calls])| {
                let open = top == Some(label);
                let wall = acc.walls.get(&label).copied().map_or(open.then_some(open_wall), |wall| Some(wall + if open { open_wall } else { 0 }));
                OwnLabel { category: label.0, family: label.1, ns: ns(cycles + if open { pending } else { 0 }), wall_ns: wall, calls }
            })
            .collect()
    })
    .unwrap_or_default()
}

pub fn own_totals() -> HashMap<String, [Option<u64>; 3]> {
    own_labels()
        .into_iter()
        .map(|label| {
            (
                format!("{}/{}", label.family, label.category),
                [Some(label.ns), label.wall_ns, Some(label.calls)],
            )
        })
        .chain(notes().into_iter().map(|(category, family, ns, calls)| {
            (format!("{family}/{category}"), [Some(ns), None, Some(calls)])
        }))
        .collect()
}

pub fn notes() -> Vec<(&'static str, &'static str, u64, u64)> {
    let hz = *TSC_HZ.get_or_init(calibrate);
    NOTES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .map(|notes| notes.iter().map(|(&(category, family), &[cycles, calls])| (category, family, (cycles as f64 / hz * 1e9).round() as u64, calls)).collect())
        .unwrap_or_default()
}

pub fn snapshot() -> Value {
    let hz = *TSC_HZ.get_or_init(calibrate);
    let ns = |cycles: u64| (cycles as f64 / hz * 1e9).round() as u64;
    let mut labels: HashMap<(String, &'static str, &'static str), [u64; 2]> = HashMap::new();
    let mut walls: HashMap<(String, &'static str, &'static str), u64> = HashMap::new();
    let mut tables = vec![[0u64; 6]; TABLE_COUNT];
    let mut reader_phases: HashMap<Label, [u64; 2]> = HashMap::new();
    let mut threads = Vec::new();
    let registry: Vec<_> = REGISTRY
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    for acc in registry {
        let acc = acc.lock().unwrap_or_else(PoisonError::into_inner);
        let now = acc.handle.cycles().unwrap_or(acc.last);
        let pending = now.saturating_sub(acc.last);
        for (label, value) in &acc.totals {
            let entry = labels
                .entry((acc.role.to_owned(), label.0, label.1))
                .or_default();
            entry[0] += value[0];
            entry[1] += value[1];
        }
        if let Some(top) = acc.stack.last() {
            labels
                .entry((acc.role.to_owned(), top.0, top.1))
                .or_default()[0] += pending;
            *walls.entry((acc.role.to_owned(), top.0, top.1)).or_default() +=
                Instant::now().saturating_duration_since(acc.last_wall).as_nanos() as u64;
        }
        for (label, wall) in &acc.walls {
            *walls.entry((acc.role.to_owned(), label.0, label.1)).or_default() += wall;
        }
        for (label, value) in &acc.reader_phases {
            let entry = reader_phases.entry(*label).or_default();
            entry[0] += value[0];
            entry[1] += value[1];
        }
        if let Some(label) = acc.reader_stack.last() {
            reader_phases.entry(*label).or_default()[0] += pending;
        }
        for (into, from) in tables.iter_mut().zip(&acc.tables) {
            for (into, from) in into.iter_mut().zip(from) {
                *into += from;
            }
        }
        threads.push(json!({
            "name": acc.name, "role": acc.role, "cycleNs": ns(now),
            "timesNs": acc.handle.times_ns(),
        }));
    }
    let mut exited_threads = 0;
    let mut exited_ns: HashMap<&'static str, u64> = HashMap::new();
    if let Some(exited) = EXITED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
    {
        exited_threads = exited.threads;
        for ((role, category, family), value) in &exited.labels {
            let entry = labels
                .entry(((*role).to_owned(), category, family))
                .or_default();
            entry[0] += value[0];
            entry[1] += value[1];
            *exited_ns.entry(role).or_default() += value[0];
        }
        for (label, value) in &exited.reader_phases {
            let entry = reader_phases.entry(*label).or_default();
            entry[0] += value[0];
            entry[1] += value[1];
        }
        for (into, from) in tables.iter_mut().zip(&exited.tables) {
            for (into, from) in into.iter_mut().zip(from) {
                *into += from;
            }
        }
    }
    let mut labels: Vec<_> = labels.into_iter().collect();
    labels.sort_by(|a, b| b.1[0].cmp(&a.1[0]));
    let labels: Vec<Value> = labels
        .into_iter()
        .map(|((role, category, family), value)| {
            let wall = walls.get(&(role.clone(), category, family)).copied();
            json!({"role": role, "category": category, "family": family,
                   "ns": ns(value[0]), "calls": value[1], "wallNs": wall})
        })
        .collect();
    let reconstruct: Vec<Value> = tables
        .iter()
        .enumerate()
        .filter(|(_, slot)| slot.iter().any(|value| *value != 0))
        .map(|(table, slot)| {
            json!({"table": format!("{:?}", TableId::ALL[table]),
                   "firstNs": ns(slot[0]), "first": slot[1],
                   "repeatNs": ns(slot[2]), "repeats": slot[3],
                   "untrackedNs": ns(slot[4]), "untracked": slot[5]})
        })
        .collect();
    let (decode_tables, decode_families, decode_family_tables, decode_keys) = match DECODES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
    {
        Some(decodes) => {
            let mut by_table: Vec<_> = decodes.tables.iter().collect();
            by_table.sort_by_key(|(table, _)| **table);
            let tables: Vec<Value> = by_table
                .into_iter()
                .map(|(table, stats)| {
                    json!({"table": format!("{:?}", TableId::ALL[*table]),
                           "firstNs": ns(stats[0]), "first": stats[1],
                           "otherFamilyNs": ns(stats[2]), "otherFamily": stats[3],
                           "sameFamilyNs": ns(stats[4]), "sameFamily": stats[5]})
                })
                .collect();
            let families: Vec<Value> = decodes
                    .by_family
                    .iter()
                    .map(|(family, value)| {
                        json!({"family": family, "ns": ns(value[0]), "decodes": value[1]})
                    })
                    .collect();
            let mut by_family_table: Vec<_> = decodes.by_family_table.iter().collect();
            by_family_table.sort_unstable_by_key(|((family, table), _)| (*family, *table));
            let family_tables: Vec<Value> = by_family_table
                .into_iter()
                .map(|((family, table), value)| {
                    json!({"family": family, "table": format!("{:?}", TableId::ALL[*table]),
                    "ns": ns(value[0]), "decodes": value[1]})
                })
                .collect();
            (tables, families, family_tables, decodes.seen.len())
        }
        None => (Vec::new(), Vec::new(), Vec::new(), 0),
    };
    let notes: Vec<Value> = NOTES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .map(|notes| {
            notes
                .iter()
                .map(|((category, family), value)| {
                    json!({"category": category, "family": family, "ns": ns(value[0]), "calls": value[1]})
                })
                .collect()
        })
        .unwrap_or_default();
    let charged = |totals: &Mutex<Option<Vec<[u64; 3]>>>, count: &str| -> Vec<Value> {
        totals
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|totals| {
                totals
                    .iter()
                    .enumerate()
                    .filter(|(_, counts)| counts[0] != 0)
                    .map(|(table, counts)| {
                        json!({
                            "table": format!("{:?}", TableId::ALL[table]),
                            (count): counts[0], "baselineRows": counts[1], "ns": ns(counts[2]),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let copies = charged(&COPIES, "copies");
    let reuses = charged(&REUSES, "reuses");
    let frame_log: Vec<Value> = LOGGED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .map(|logged| {
            logged
                .iter()
                .map(|((kind, family), value)| {
                    json!({"kind": kind, "family": family, "count": value[0], "rows": value[1]})
                })
                .collect()
        })
        .unwrap_or_default();
    json!({
        "readerPhases": ({
            let mut phases: Vec<_> = reader_phases.into_iter().collect();
            phases.sort_unstable_by_key(|(label, _)| *label);
            phases.into_iter().map(|((phase, table), value)| json!({"phase": phase, "table": table, "ns": ns(value[0]), "calls": value[1]})).collect::<Vec<_>>()
        }),
        "readerOperations": replay_data::measurement::snapshot().into_iter().map(|(table, values)| {
            let mut row = serde_json::Map::new();
            row.insert("table".into(), json!(table.map_or_else(|| "unattributed".to_owned(), |table| format!("{table:?}"))));
            for (name, value) in values { row.insert(name.into(), json!(value)); }
            Value::Object(row)
        }).collect::<Vec<_>>(),
        "relevantSourceMasks": replay_data::measurement::source_masks().into_iter().map(|(target, source, mask)| {
            json!({"table": format!("{target:?}"), "source": format!("{source:?}"), "mask": mask})
        }).collect::<Vec<_>>(),
        "notes": notes,
        "deltaCopies": copies,
        "deltaReuses": reuses,
        "frameLog": frame_log,
        "scope": "Thread cycle time (QueryThreadCycleTime, which a blocked thread does not accrue) charged to the (category, family) label on top of each thread's stack, converted at the calibrated counter rate. A thread's time before its first label is its base label from its thread name. Live threads include their current label's open span. readerPhases are exclusive among reader phases by table (nested generic inflate/wire/shadow scopes remain within the active reader phase); alias_group_inclusive notes overlap phases and source decoding and must not be added to them. readerOperations are operation counts, copy bytes are source-level element traffic, not allocator or hardware measurements. relevantSourceMasks OR the propagated source-field masks. decodeTables: each non-group table section decode of a block, first, by another family after one had, or by the same family again. reconstructTables: each reconstructed table-hour, first or repeated (by any decoder) by hour, ignoring branch.",
        "tscHz": hz,
        "cycleReadNs": cycle_read_ns(),
        "processCpuNs": process_ns(),
        "labels": labels,
        "threads": threads,
        "exitedThreads": exited_threads,
        "exitedByRole": exited_ns.into_iter().map(|(role, cycles)| json!({"role": role, "ns": ns(cycles)})).collect::<Vec<_>>(),
        "decodeTables": decode_tables,
        "decodeFamilies": decode_families,
        "decodeFamilyTables": decode_family_tables,
        "decodeKeys": decode_keys,
        "reconstructTables": reconstruct,
    })
}

#[derive(Default)]
struct Decodes {
    seen: HashMap<(u64, i32, u16), Vec<&'static str>>,
    tables: HashMap<usize, [u64; 6]>,
    by_family: HashMap<&'static str, [u64; 2]>,
    by_family_table: HashMap<(&'static str, usize), [u64; 2]>,
}

struct HourBits {
    first: i32,
    count: usize,
    words: usize,
    bits: Vec<AtomicU64>,
}

#[derive(Default)]
struct Exited {
    threads: u64,
    labels: HashMap<(&'static str, &'static str, &'static str), [u64; 2]>,
    tables: Vec<[u64; 6]>,
    reader_phases: HashMap<Label, [u64; 2]>,
}

struct ThreadAcc {
    handle: Handle,
    name: String,
    role: &'static str,
    stack: Vec<Label>,
    reader_stack: Vec<Label>,
    last: u64,
    totals: HashMap<Label, [u64; 2]>,
    block: Option<usize>,
    blocks: HashMap<BlockLabel, [u64; 2]>,
    tables: Vec<[u64; 6]>,
    reader_phases: HashMap<Label, [u64; 2]>,
    walls: HashMap<Label, u64>,
    last_wall: Instant,
}

impl ThreadAcc {
    fn charge(&mut self, now: u64) {
        let spent = now.saturating_sub(self.last);
        self.last = now;
        let wall = Instant::now();
        let waited = wall.saturating_duration_since(self.last_wall).as_nanos() as u64;
        self.last_wall = wall;
        if let Some(top) = self.stack.last() {
            *self.walls.entry(*top).or_default() += waited;
            self.totals.entry(*top).or_default()[0] += spent;
            if BLOCKS.load(Ordering::Relaxed) {
                self.blocks
                    .entry(block_label(self.block, &self.stack).unwrap())
                    .or_default()[0] += spent;
            }
        }
        if let Some(label) = self.reader_stack.last() {
            self.reader_phases.entry(*label).or_default()[0] += spent;
        }
    }

    fn push(&mut self, category: &'static str, family: Option<&'static str>) {
        let now = current_cycles();
        self.charge(now);
        let family = family
            .or_else(|| self.stack.last().map(|label| label.1))
            .unwrap_or("-");
        let label = (category, family);
        self.totals.entry(label).or_default()[1] += 1;
        self.stack.push(label);
        if BLOCKS.load(Ordering::Relaxed) {
            self.blocks
                .entry(block_label(self.block, &self.stack).unwrap())
                .or_default()[1] += 1;
        }
    }

    fn push_reader(&mut self, phase: &'static str, table: &'static str) {
        self.push(phase, None);
        let label = (phase, table);
        self.reader_phases.entry(label).or_default()[1] += 1;
        self.reader_stack.push(label);
    }

    fn pop_reader(&mut self) {
        self.pop();
        self.reader_stack.pop();
    }

    fn pop(&mut self) {
        let now = current_cycles();
        self.charge(now);
        if self.stack.len() > 1 {
            self.stack.pop();
        }
    }
}

struct Local(Arc<Mutex<ThreadAcc>>);

impl Drop for Local {
    fn drop(&mut self) {
        let mut acc = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let now = current_cycles();
        acc.charge(now);
        if !acc.blocks.is_empty() {
            let mut blocks = BLOCK_TOTALS.lock().unwrap_or_else(PoisonError::into_inner);
            let blocks = blocks.get_or_insert_with(HashMap::new);
            for (label, value) in acc.blocks.drain() {
                let into = blocks.entry(label).or_default();
                into[0] += value[0];
                into[1] += value[1];
            }
        }
        let mut exited = EXITED.lock().unwrap_or_else(PoisonError::into_inner);
        let exited = exited.get_or_insert_with(|| Exited {
            tables: vec![[0; 6]; TABLE_COUNT],
            ..Exited::default()
        });
        exited.threads += 1;
        let role = acc.role;
        for (label, value) in acc.totals.drain() {
            let entry = exited.labels.entry((role, label.0, label.1)).or_default();
            entry[0] += value[0];
            entry[1] += value[1];
        }
        for (label, value) in acc.reader_phases.drain() {
            let entry = exited.reader_phases.entry(label).or_default();
            entry[0] += value[0];
            entry[1] += value[1];
        }
        for (into, from) in exited.tables.iter_mut().zip(&acc.tables) {
            for (into, from) in into.iter_mut().zip(from) {
                *into += from;
            }
        }
        drop(acc);
        REGISTRY
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|other| !Arc::ptr_eq(other, &self.0));
    }
}

thread_local! {
    static LOCAL: RefCell<Option<Local>> = const { RefCell::new(None) };
}

fn with_local<T>(run: impl FnOnce(&mut ThreadAcc) -> T) -> Option<T> {
    LOCAL
        .try_with(|local| {
            let mut local = local.try_borrow_mut().ok()?;
            let local = local.get_or_insert_with(register);
            let mut acc = local.0.lock().unwrap_or_else(PoisonError::into_inner);
            Some(run(&mut acc))
        })
        .ok()
        .flatten()
}

fn register() -> Local {
    let name = std::thread::current()
        .name()
        .unwrap_or("unnamed")
        .to_owned();
    let (role, base) = match name.strip_prefix("preparation of ") {
        Some(family) => (intern("family thread"), ("fold", intern(family))),
        None => {
            let role = intern(name.trim_end_matches(|c: char| c.is_ascii_digit() || c == ' '));
            (role, ("thread", role))
        }
    };
    let mut totals = HashMap::new();
    totals.insert(base, [0, 1]);
    let acc = Arc::new(Mutex::new(ThreadAcc {
        handle: Handle::current(),
        name,
        role,
        stack: vec![base],
        reader_stack: Vec::new(),
        last: 0,
        totals,
        block: None,
        blocks: HashMap::new(),
        tables: vec![[0; 6]; TABLE_COUNT],
        reader_phases: HashMap::new(),
        walls: HashMap::new(),
        last_wall: Instant::now(),
    }));
    REGISTRY
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(Arc::clone(&acc));
    Local(acc)
}

fn intern(text: &str) -> &'static str {
    let mut interned = INTERNED.lock().unwrap_or_else(PoisonError::into_inner);
    let interned = interned.get_or_insert_with(HashSet::new);
    if let Some(found) = interned.get(text) {
        return found;
    }
    let leaked: &'static str = Box::leak(text.to_owned().into_boxed_str());
    interned.insert(leaked);
    leaked
}

fn calibrate() -> f64 {
    let started = Instant::now();
    let first = stamp();
    while started.elapsed().as_millis() < 100 {
        std::hint::spin_loop();
    }
    let cycles = stamp().wrapping_sub(first);
    let seconds = started.elapsed().as_secs_f64();
    if cycles == 0 || seconds <= 0.0 {
        1e9
    } else {
        cycles as f64 / seconds
    }
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn QueryThreadCycleTime(
        thread: windows_sys::Win32::Foundation::HANDLE,
        cycles: *mut u64,
    ) -> i32;
}

#[cfg(windows)]
struct Handle(windows_sys::Win32::Foundation::HANDLE);

// SAFETY: a thread handle is a kernel object reference usable from any thread.
#[cfg(windows)]
unsafe impl Send for Handle {}

#[cfg(windows)]
impl Handle {
    fn current() -> Self {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThreadId, OpenThread, THREAD_QUERY_LIMITED_INFORMATION,
        };
        Self(unsafe { OpenThread(THREAD_QUERY_LIMITED_INFORMATION, 0, GetCurrentThreadId()) })
    }

    fn cycles(&self) -> Option<u64> {
        if self.0.is_null() {
            return None;
        }
        let mut cycles = 0u64;
        (unsafe { QueryThreadCycleTime(self.0, &raw mut cycles) } != 0).then_some(cycles)
    }

    fn times_ns(&self) -> Option<u64> {
        use windows_sys::Win32::{Foundation::FILETIME, System::Threading::GetThreadTimes};
        if self.0.is_null() {
            return None;
        }
        let zero = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let [mut created, mut exited, mut kernel, mut user] = [zero; 4];
        let read =
            unsafe { GetThreadTimes(self.0, &raw mut created, &raw mut exited, &raw mut kernel, &raw mut user) };
        (read != 0).then(|| (filetime(kernel) + filetime(user)) * 100)
    }
}

#[cfg(windows)]
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
        }
    }
}

#[cfg(windows)]
fn filetime(time: windows_sys::Win32::Foundation::FILETIME) -> u64 {
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
}

#[cfg(windows)]
fn current_cycles() -> u64 {
    use windows_sys::Win32::System::Threading::GetCurrentThread;
    CYCLE_READS.with(|reads| reads.set(reads.get() + 1));
    let mut cycles = 0u64;
    unsafe { QueryThreadCycleTime(GetCurrentThread(), &raw mut cycles) };
    cycles
}

#[cfg(windows)]
fn process_ns() -> Option<u64> {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::Threading::{GetCurrentProcess, GetProcessTimes},
    };
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let [mut created, mut exited, mut kernel, mut user] = [zero; 4];
    let read = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &raw mut created,
            &raw mut exited,
            &raw mut kernel,
            &raw mut user,
        )
    };
    (read != 0).then(|| (filetime(kernel) + filetime(user)) * 100)
}

#[cfg(not(windows))]
struct Handle;

#[cfg(not(windows))]
impl Handle {
    fn current() -> Self {
        Self
    }
    fn cycles(&self) -> Option<u64> {
        None
    }
    fn times_ns(&self) -> Option<u64> {
        None
    }
}

#[cfg(not(windows))]
fn current_cycles() -> u64 {
    0
}

#[cfg(not(windows))]
fn process_ns() -> Option<u64> {
    None
}

#[cfg(test)]
mod reader_tests {
    use super::*;

    #[test]
    fn reader_phases_are_exclusive_across_nested_tables_but_keep_nested_grammar_categories() {
        let family = ("thread", "observer");
        let grammar = ("reader_raw_grammar", "DivisionStatus");
        let apply = ("reader_raw_apply", "DivisionPanelStats");
        let mut acc = ThreadAcc {
            handle: Handle::current(),
            name: "synthetic".into(),
            role: "synthetic",
            stack: vec![family],
            reader_stack: Vec::new(),
            last: 0,
            totals: HashMap::new(),
            block: None,
            blocks: HashMap::new(),
            tables: Vec::new(),
            reader_phases: HashMap::new(),
            walls: HashMap::new(),
            last_wall: Instant::now(),
        };
        acc.charge(10);
        assert!(acc.walls.contains_key(&family), "wall time is charged to the label on top, beside its cycles");
        acc.reader_stack.push(grammar);
        acc.stack.push(("reader_raw_grammar", "observer"));
        acc.charge(30);
        acc.stack.push(("decode_wire", "observer"));
        acc.charge(40);
        acc.reader_stack.push(apply);
        acc.stack.push(("reader_raw_apply", "observer"));
        acc.charge(70);
        acc.reader_stack.pop();
        acc.stack.pop();
        acc.charge(90);
        assert_eq!(acc.reader_phases[&grammar][0], 50);
        assert_eq!(acc.reader_phases[&apply][0], 30);
        assert_eq!(acc.totals.values().map(|value| value[0]).sum::<u64>(), 90);
        assert_eq!(acc.stack.last().unwrap().1, "observer");
    }
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static READER_CALLS: RefCell<Option<HashMap<(&'static str, TableId), u64>>> =
        const { RefCell::new(None) };
}

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub fn capture_reader_calls<T>(
    work: impl FnOnce() -> T,
) -> (T, HashMap<(&'static str, TableId), u64>) {
    struct Restore(Option<HashMap<(&'static str, TableId), u64>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            READER_CALLS.set(self.0.take());
        }
    }
    let _restore = Restore(READER_CALLS.replace(Some(HashMap::new())));
    let result = work();
    (result, READER_CALLS.take().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_context_keeps_parent_ownership_and_restores_the_worker_on_unwind() {
        std::thread::spawn(|| {
            let original = with_local(|acc| (acc.stack.clone(), acc.block)).unwrap();
            for (stack, block, owner, phase) in [
                (vec![("harness", "shared")], None, "shared", "shared"),
                (
                    vec![("fold", "family"), ("extract", "family:part")],
                    Some(3),
                    "family",
                    "extract",
                ),
                (
                    vec![("fold", "family"), ("commit", "family")],
                    Some(8),
                    "family",
                    "publication",
                ),
                (
                    vec![("commit", "family"), ("shared commit", "shared commit")],
                    Some(8),
                    "shared commit",
                    "shared commit",
                ),
            ] {
                let context = Context(Some((stack, block)));
                let result = std::panic::catch_unwind(|| {
                    let _helper = context.enter("helper");
                    with_local(|acc| {
                        assert_eq!(acc.block, block);
                        assert_eq!(
                            block_label(acc.block, &acc.stack),
                            Some((
                                if owner == "shared commit" {
                                    None
                                } else {
                                    block
                                },
                                "helper",
                                owner,
                                phase,
                            ))
                        );
                        assert_eq!(acc.totals[acc.stack.last().unwrap()][1], 1);
                    });
                    panic!("synthetic helper failure");
                });
                assert!(result.is_err());
                assert_eq!(
                    with_local(|acc| (acc.stack.clone(), acc.block)).unwrap(),
                    original
                );
            }
        })
        .join()
        .unwrap();
    }

    #[test]
    fn group_publication_attribution_is_independent_of_leader_and_block() {
        for (leader, block) in [("family-a", Some(3)), ("family-b", Some(8))] {
            let mut stack = vec![("fold", leader), ("commit", leader)];
            let mut totals = HashMap::<BlockLabel, u64>::new();
            *totals
                .entry(block_label(block, &stack).unwrap())
                .or_default() += 11;
            stack.push(("fold save", leader));
            assert_eq!(
                block_label(block, &stack),
                Some((block, "fold save", leader, "publication"))
            );
            *totals
                .entry(block_label(block, &stack).unwrap())
                .or_default() += 17;
            stack.pop();
            stack.push(("shared commit", "shared commit"));
            assert_eq!(
                block_label(block, &stack),
                Some((None, "shared commit", "shared commit", "shared commit"))
            );
            *totals
                .entry(block_label(block, &stack).unwrap())
                .or_default() += 29;
            stack.push(("commit", leader));
            assert_eq!(
                block_label(block, &stack),
                Some((None, "commit", "shared commit", "shared commit"))
            );
            *totals
                .entry(block_label(block, &stack).unwrap())
                .or_default() += 31;
            stack.pop();
            stack.pop();
            assert_eq!(
                block_label(block, &stack),
                Some((block, "commit", leader, "publication"))
            );
            *totals
                .entry(block_label(block, &stack).unwrap())
                .or_default() += 13;
            assert_eq!(totals.values().sum::<u64>(), 101);
            assert_eq!(
                totals
                    .iter()
                    .filter(|(label, _)| label.2 == "shared commit")
                    .map(|(_, cpu)| *cpu)
                    .sum::<u64>(),
                60
            );
            assert_eq!(
                totals
                    .iter()
                    .filter(|(label, _)| label.2 == leader)
                    .map(|(_, cpu)| *cpu)
                    .sum::<u64>(),
                41
            );
        }
    }
}
