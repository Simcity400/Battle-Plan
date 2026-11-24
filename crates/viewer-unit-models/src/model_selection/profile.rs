use std::{cell::Cell, time::Instant};
use viewer_runtime::profiling::{BuilderSpan, BuilderSpans, current_thread_cpu_time};

#[derive(Clone, Copy)]
pub(crate) enum Stage {
    Template,
    PairClassify,
    PairSorts,
    PairRank,
    Land,
    Paired,
    Graphics,
    GraphicsRules,
    GraphicsResolve,
    Fallback,
    Numbered,
    Transport,
    Camouflage,
}

const NAMES: [&str; 13] = [
    "units.selection.template",
    "units.selection.pair_classify",
    "units.selection.pair_sorts",
    "units.selection.pair_rank",
    "units.selection.land",
    "units.selection.paired",
    "units.selection.graphics",
    "units.selection.graphics.rules",
    "units.selection.graphics.resolve",
    "units.selection.fallback",
    "units.selection.numbered",
    "units.selection.transport",
    "units.selection.camouflage",
];

#[derive(Clone, Copy, Default)]
struct Total {
    first: Option<usize>,
    ms: f64,
    cpu_ms: Option<f64>,
}

pub(crate) struct Stages {
    enabled: Cell<bool>,
    next: Cell<usize>,
    totals: [Cell<Total>; NAMES.len()],
}

impl Stages {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            enabled: Cell::new(enabled),
            next: Cell::new(0),
            totals: std::array::from_fn(|_| Cell::new(Total::default())),
        }
    }

    pub(crate) fn measure<T>(&self, stage: Stage, work: impl FnOnce() -> T) -> T {
        if !self.enabled.get() {
            return work();
        }
        let started = Instant::now();
        let cpu_started = current_thread_cpu_time();
        let result = work();
        let cpu_ms = cpu_started
            .zip(current_thread_cpu_time())
            .and_then(|(before, after)| after.checked_sub(before))
            .map(|used| used.as_secs_f64() * 1000.0);
        self.add(stage, started.elapsed().as_secs_f64() * 1000.0, cpu_ms);
        result
    }

    fn add(&self, stage: Stage, ms: f64, cpu_ms: Option<f64>) {
        let slot = &self.totals[stage as usize];
        let mut total = slot.get();
        if total.first.is_none() {
            total.first = Some(self.next.get());
            self.next.set(self.next.get() + 1);
            total.cpu_ms = cpu_ms;
        } else {
            total.cpu_ms = total.cpu_ms.zip(cpu_ms).map(|(a, b)| a + b);
        }
        total.ms += ms;
        slot.set(total);
    }

    pub(crate) fn append_to(&self, spans: &mut BuilderSpans) {
        self.enabled.set(false);
        for first in 0..self.next.replace(0) {
            let (at, slot) = self
                .totals
                .iter()
                .enumerate()
                .find(|(_, slot)| slot.get().first == Some(first))
                .expect("each first completion has one stage");
            let total = slot.replace(Total::default());
            spans.record(BuilderSpan {
                name: NAMES[at],
                ms: total.ms,
                cpu_ms: total.cpu_ms,
            });
        }
    }
}

