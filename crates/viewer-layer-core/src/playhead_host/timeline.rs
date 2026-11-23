
use std::{
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

pub const MARKS: [&str; 8] = ["asked", "rows", "build_began", "build_ended", "published", "join_began", "waited", "joined"];

const STEP: [&str; 5] = ["apply_ended", "settles_began", "ended", "views_settled", "joins_began"];

const MAKERS: usize = 16;

const SLOTS: usize = MAKERS * MARKS.len() + STEP.len();

const NONE: u64 = u64::MAX;

#[derive(Clone, Copy)]
pub struct Maker(usize);

#[derive(Clone, Copy)]
pub enum Mark {
    Asked = 0,
    Rows = 1,
    BuildBegan = 2,
    BuildEnded = 3,
    Published = 4,
    JoinBegan = 5,
    Waited = 6,
    Joined = 7,
}

static NAMES: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
static ORIGIN: AtomicU64 = AtomicU64::new(NONE);
static AT: [AtomicU64; SLOTS] = [const { AtomicU64::new(NONE) }; SLOTS];
static STEPS: Mutex<Vec<[u32; SLOTS]>> = Mutex::new(Vec::new());

fn epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

fn now() -> u64 {
    epoch().elapsed().as_nanos() as u64
}

fn on() -> bool {
    viewer_runtime::view_record::on()
}

pub fn maker(name: &'static str) -> Maker {
    let mut names = NAMES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let at = names.iter().position(|held| *held == name).unwrap_or_else(|| {
        names.push(name);
        names.len() - 1
    });
    Maker(at)
}

pub fn mark(maker: Maker, mark: Mark) {
    if !on() || maker.0 >= MAKERS {
        return;
    }
    let slot = &AT[maker.0 * MARKS.len() + mark as usize];
    match mark {
        Mark::Asked | Mark::Rows | Mark::BuildBegan | Mark::JoinBegan => {
            let _ = slot.compare_exchange(NONE, now(), Ordering::Relaxed, Ordering::Relaxed);
        }
        Mark::BuildEnded | Mark::Published | Mark::Waited | Mark::Joined => slot.store(now(), Ordering::Relaxed),
    }
}

pub fn step() {
    if !on() {
        return;
    }
    for slot in &AT {
        slot.store(NONE, Ordering::Relaxed);
    }
    ORIGIN.store(now(), Ordering::Relaxed);
}

pub fn applied() {
    step_mark(0);
}

pub fn settling() {
    step_mark(1);
}

pub fn views_settled() {
    step_mark(3);
}

pub fn joining() {
    step_mark(4);
}

fn step_mark(at: usize) {
    if !on() {
        return;
    }
    AT[MAKERS * MARKS.len() + at].store(now(), Ordering::Relaxed);
}

pub fn close(composing: bool) {
    if !on() {
        return;
    }
    step_mark(2);
    let origin = ORIGIN.swap(NONE, Ordering::Relaxed);
    if composing || origin == NONE {
        return;
    }
    let mut row = [u32::MAX; SLOTS];
    for (held, slot) in row.iter_mut().zip(&AT) {
        let at = slot.load(Ordering::Relaxed);
        if at != NONE && at >= origin {
            *held = u32::try_from(at - origin).unwrap_or(u32::MAX - 1);
        }
    }
    STEPS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(row);
}

fn percentile(values: &mut Vec<u32>, share: f64) -> Option<f64> {
    values.sort_unstable();
    let at = ((values.len() as f64 * share) as usize).min(values.len().saturating_sub(1));
    values.get(at).map(|&ns| f64::from(ns) / 1e6)
}

pub fn report() -> serde_json::Value {
    use serde_json::json;
    let steps = std::mem::take(&mut *STEPS.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
    let names = NAMES.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    let column = |at: usize| {
        let mut values: Vec<u32> = steps.iter().map(|row| row[at]).filter(|&ns| ns != u32::MAX).collect();
        let (p50, p90) = (percentile(&mut values, 0.5), percentile(&mut values, 0.9));
        json!({"p50_ms": p50, "p90_ms": p90, "steps": values.len()})
    };
    let makers: Vec<serde_json::Value> = names
        .iter()
        .enumerate()
        .take(MAKERS)
        .map(|(maker, name)| {
            let marks: serde_json::Map<String, serde_json::Value> = MARKS.iter().enumerate().map(|(at, mark)| ((*mark).to_owned(), column(maker * MARKS.len() + at))).collect();
            json!({"name": name, "marks": marks})
        })
        .collect();
    let step: serde_json::Map<String, serde_json::Value> = STEP.iter().enumerate().map(|(at, name)| ((*name).to_owned(), column(MAKERS * MARKS.len() + at))).collect();
    json!({
        "steps": steps.len(),
        "step": step,
        "makers": makers,
        "scope": "Milliseconds from the start of the map side's step. views_settled is when the views the step before posted were all made, which the side waits for before it applies; settles_began and joins_began bracket the layers that join no maker. Per maker: asked, when the step told it to work; rows, when its worker took the rows it builds from; build_began and build_ended, its build; published, when its worker last handed an answer over; join_began and waited, the layer's join and the end of its wait for the product; joined, when the layer's settle took its product. A mark left out of a step (the maker did nothing in it) is not counted; steps says how many steps marked it.",
    })
}
