use std::{
    cell::Cell,
    sync::{
        Mutex, TryLockError,
        atomic::{AtomicI64, AtomicU8, AtomicU64, AtomicUsize, Ordering},
    },
};

const CAPACITY: usize = 2 * crate::roles::MAX_JOBS;
const CALLBACK: u8 = u8::MAX;
const OFFSET_BITS: u32 = 48;
const OFFSET_MASK: u64 = (1 << OFFSET_BITS) - 1;

thread_local! {
    static WORKER: Cell<u8> = const { Cell::new(CALLBACK) };
}

pub(crate) fn enter_worker(index: usize) {
    WORKER.set(u8::try_from(index).unwrap_or(CALLBACK - 1));
}

struct Log {
    begin: AtomicI64,
    len: AtomicUsize,
    template_fallback: AtomicU8,
    spans: [[AtomicU64; 2]; CAPACITY],
}

static LOG: Log = Log::new();

impl Log {
    const fn new() -> Self {
        Self {
            begin: AtomicI64::new(0),
            len: AtomicUsize::new(0),
            template_fallback: AtomicU8::new(0),
            spans: [const { [const { AtomicU64::new(0) }; 2] }; CAPACITY],
        }
    }

    fn begin(&self, now: i64) {
        self.begin.store(now, Ordering::Relaxed);
        self.template_fallback.store(0, Ordering::Relaxed);
        self.len.store(0, Ordering::Release);
    }

    fn record(&self, role: usize, started: i64, ended: i64) {
        let at = self.len.fetch_add(1, Ordering::Relaxed);
        let Some(span) = self.spans.get(at) else {
            return;
        };
        let offset = started
            .saturating_sub(self.begin.load(Ordering::Relaxed))
            .max(0) as u64;
        let worker = WORKER.try_with(Cell::get).unwrap_or(CALLBACK);
        span[0].store(
            ((role as u64) << 56) | (u64::from(worker) << OFFSET_BITS) | offset.min(OFFSET_MASK),
            Ordering::Relaxed,
        );
        span[1].store(
            ended.saturating_sub(started).max(0) as u64,
            Ordering::Relaxed,
        );
    }
}

const TOTAL_STAGES: usize = crate::roles::ROLE_COUNT + PARTS.len();
static STAGE_CALLS: [AtomicU64; TOTAL_STAGES] = [const { AtomicU64::new(0) }; TOTAL_STAGES];
static STAGE_TICKS: [AtomicU64; TOTAL_STAGES] = [const { AtomicU64::new(0) }; TOTAL_STAGES];

fn accumulate(index: usize, started: i64, ended: i64) {
    if let (Some(calls), Some(ticks)) = (STAGE_CALLS.get(index), STAGE_TICKS.get(index)) {
        calls.fetch_add(1, Ordering::Relaxed);
        ticks.fetch_add(ended.saturating_sub(started).max(0) as u64, Ordering::Relaxed);
    }
}

pub fn begin(now: i64) {
    LOG.begin(now);
}

pub fn record(role: usize, started: i64, ended: i64) {
    LOG.record(role, started, ended);
    accumulate(role, started, ended);
}

#[derive(Clone, Copy)]
pub enum Part {
    SettleSources,
    SettleCertify,
    SettleDaily,
    SettleMaxima,
    SettleRanges,
    TemplateCensus,
    TemplateRanges,
    TemplateApply,
    TemplateSerial,
    TemplateSpeedKeys,
    TemplateCertify,
    TemplateDictionary,
    GovernanceBuildingsWait,
    ProjectsRestWait,
    VariantsLanes,
    VariantsJoin,
    VariantsSnapshot,
    VariantsContext,
    VariantsSetup,
    ScoresRailsWait,
    RaidStateWait,
    IntelResidualWait,
    OrdinaryNaviesWait,
    PresentationSpeedTermsWait,
}

const PARTS: [&str; 24] = [
    "settle_sources",
    "settle_certify",
    "settle_daily",
    "settle_maxima",
    "settle_ranges",
    "template_census",
    "template_ranges",
    "template_apply",
    "template_serial",
    "template_speed_keys",
    "template_certify",
    "template_dictionary",
    "governance_buildings_wait",
    "projects_rest_wait",
    "variants_lanes",
    "variants_join",
    "variants_snapshot",
    "variants_context",
    "variants_setup",
    "scores_rails_wait",
    "raid_state_wait",
    "intel_residual_wait",
    "ordinary_navies_wait",
    "presentation_speed_terms_wait",
];

pub fn record_part(part: Part, started: i64, ended: i64) {
    let index = crate::roles::ROLE_COUNT + part as usize;
    LOG.record(index, started, ended);
    accumulate(index, started, ended);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateFallback {
    OneLane,
    Census,
    Lent,
    Room,
    Dispatch,
    Incomplete,
    Redecided,
}

const FALLBACKS: [&str; 7] = [
    "one lane",
    "roster census refused or under the split size",
    "a range still held the stores",
    "a refused allocation",
    "range dispatch refused",
    "a range incomplete",
    "a failed range's last batch held a repeated template",
];

pub fn note_template_fallback(fallback: TemplateFallback) {
    LOG.template_fallback
        .store(fallback as u8 + 1, Ordering::Relaxed);
}

#[derive(serde::Serialize)]
#[serde(untagged)]
enum Stage {
    Job(crate::roles::Role),
    Part(&'static str),
}

fn stage(index: usize) -> Option<Stage> {
    match index.checked_sub(crate::roles::ROLE_COUNT) {
        None => crate::roles::ROLES.get(index).copied().map(Stage::Job),
        Some(part) => PARTS.get(part).copied().map(Stage::Part),
    }
}

#[derive(Clone, Copy)]
struct Timeline {
    hour: Option<i32>,
    held_ns: u64,
    len: usize,
    dropped: usize,
    template_fallback: u8,
    spans: [[u64; 2]; CAPACITY],
}

impl Timeline {
    const EMPTY: Self = Self {
        hour: None,
        held_ns: 0,
        len: 0,
        dropped: 0,
        template_fallback: 0,
        spans: [[0; 2]; CAPACITY],
    };

    fn fill(&mut self, log: &Log, hour: Option<i32>, held_ns: u64, ns: &dyn Fn(i64) -> u64) {
        let logged = log.len.load(Ordering::Acquire);
        self.hour = hour;
        self.held_ns = held_ns;
        self.len = logged.min(CAPACITY);
        self.dropped = logged - self.len;
        self.template_fallback = log.template_fallback.load(Ordering::Relaxed);
        for (kept, span) in self.spans.iter_mut().zip(&log.spans[..self.len]) {
            let word = span[0].load(Ordering::Relaxed);
            let begin = ns((word & OFFSET_MASK) as i64).min(OFFSET_MASK);
            *kept = [
                (word & !OFFSET_MASK) | begin,
                ns(span[1].load(Ordering::Relaxed) as i64),
            ];
        }
    }

    fn status(mut self) -> Option<impl serde::Serialize + use<>> {
        let len = self.len;
        self.spans[..len].sort_unstable_by_key(|span| span[0] & OFFSET_MASK);
        self.hour.map(|hour| {
            collector_status::status::status!({
                "hour": hour,
                "held_ns": self.held_ns,
                "dropped": self.dropped,
                "template_fallback": self
                    .template_fallback
                    .checked_sub(1)
                    .and_then(|fallback| FALLBACKS.get(usize::from(fallback)).copied()),
                "stages": collector_status::status::sequence(self.spans.into_iter().take(len).map(|[word, wall_ns]| {
                    let worker = (word >> OFFSET_BITS) as u8;
                    collector_status::status::status!({
                        "stage": stage((word >> 56) as usize),
                        "worker": (worker != CALLBACK).then_some(worker),
                        "begin_ns": word & OFFSET_MASK,
                        "wall_ns": wall_ns,
                    })
                })),
            })
        })
    }
}

struct Kept {
    forced: Timeline,
    slowest: Timeline,
}

static KEPT: Mutex<Kept> = Mutex::new(Kept {
    forced: Timeline::EMPTY,
    slowest: Timeline::EMPTY,
});
static SLOWEST_NS: AtomicU64 = AtomicU64::new(0);
static MISSED: AtomicU64 = AtomicU64::new(0);

pub fn keep(hour: Option<i32>, held_ns: u64, forced: bool, ns: &dyn Fn(i64) -> u64) {
    if !forced && held_ns <= SLOWEST_NS.load(Ordering::Relaxed) {
        return;
    }
    let mut kept = match KEPT.try_lock() {
        Ok(kept) => kept,
        Err(TryLockError::Poisoned(error)) => error.into_inner(),
        Err(TryLockError::WouldBlock) => {
            MISSED.fetch_add(1, Ordering::Relaxed);
            return;
        }
    };
    if forced {
        kept.forced.fill(&LOG, hour, held_ns, ns);
    } else {
        SLOWEST_NS.store(held_ns, Ordering::Relaxed);
        kept.slowest.fill(&LOG, hour, held_ns, ns);
    }
}

pub fn status(ns: &dyn Fn(i64) -> u64) -> impl serde::Serialize + use<> {
    let (forced, slowest) = {
        let kept = KEPT
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (kept.forced, kept.slowest)
    };
    let totals: [(u64, u64); TOTAL_STAGES] = std::array::from_fn(|index| {
        let calls = STAGE_CALLS[index].load(Ordering::Relaxed);
        let ticks = STAGE_TICKS[index].load(Ordering::Relaxed);
        (calls, ns(ticks.min(i64::MAX as u64) as i64))
    });
    collector_status::status::status!({
        "scope": "every acquisition job of the latest forced hour and of the slowest other hour since attach, by begin: its stage, the pool worker that ran it (null: the game thread), its begin after the hold began and its wall, both ns; totals: cumulative calls and wall_ns per stage since attach; settle_*, template_* and variants_* stages are serial parts inside the presentation_settle, templates and variants jobs, governance_buildings_wait and projects_rest_wait the governance and gated_projects parents' waits for the work they gave a worker beside their own; template_fallback: why the hour's template walk ran serially (null: split or never offered ranges); dropped: jobs past the log's capacity; missed: hours not kept while status read them",
        "forced": forced.status(),
        "slowest": slowest.status(),
        "totals": collector_status::status::sequence(
            totals
                .into_iter()
                .enumerate()
                .filter(|&(_, (calls, _))| calls != 0)
                .map(|(index, (calls, wall_ns))| {
                    collector_status::status::status!({
                        "stage": stage(index),
                        "calls": calls,
                        "wall_ns": wall_ns,
                    })
                }),
        ),
        "missed": MISSED.load(Ordering::Relaxed),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kept_hour_names_each_jobs_stage_worker_begin_and_wall() {
        let log = Box::new(Log::new());
        log.begin(1_000);
        enter_worker(3);
        log.record(crate::roles::Role::Templates as usize, 1_010, 1_050);
        log.record(
            crate::roles::ROLE_COUNT + Part::TemplateApply as usize,
            1_030,
            1_040,
        );
        WORKER.set(CALLBACK);
        log.record(crate::roles::Role::Presentation as usize, 1_002, 1_004);
        let mut timeline = Box::new(Timeline::EMPTY);
        timeline.fill(&log, Some(7), 99, &|ticks| ticks as u64 * 100);
        collector_test_support::with_allocations_denied(|| {
            serde_json::to_writer(std::io::sink(), &timeline.status()).unwrap();
        });
        let status = serde_json::to_value(timeline.status()).unwrap();
        assert_eq!(status["hour"], 7);
        assert_eq!(status["dropped"], 0);
        let stages = status["stages"].as_array().unwrap();
        assert_eq!(stages.len(), 3);
        assert_eq!(stages[2]["stage"], "template_apply");
        assert_eq!(stages[2]["worker"], 3);
        assert_eq!(stages[2]["begin_ns"], 3_000);
        assert_eq!(stages[2]["wall_ns"], 1_000);
        assert_eq!(PARTS.len(), Part::PresentationSpeedTermsWait as usize + 1);
        assert_eq!(status["template_fallback"], serde_json::Value::Null);
        assert_eq!(FALLBACKS.len(), TemplateFallback::Redecided as usize + 1);
        log.template_fallback
            .store(TemplateFallback::Redecided as u8 + 1, Ordering::Relaxed);
        timeline.fill(&log, Some(7), 99, &|ticks| ticks as u64 * 100);
        let status = serde_json::to_value(timeline.status()).unwrap();
        assert_eq!(
            status["template_fallback"],
            "a failed range's last batch held a repeated template"
        );
        log.begin(2_000);
        timeline.fill(&log, Some(8), 99, &|ticks| ticks as u64);
        assert_eq!(timeline.template_fallback, 0);
        assert!(crate::roles::ROLE_COUNT + PARTS.len() <= usize::from(u8::MAX));
        assert_eq!(stages[0]["stage"], "presentation");
        assert_eq!(stages[0]["worker"], serde_json::Value::Null);
        assert_eq!(stages[0]["begin_ns"], 200);
        assert_eq!(stages[0]["wall_ns"], 200);
        assert_eq!(stages[1]["stage"], "templates");
        assert_eq!(stages[1]["worker"], 3);
        assert_eq!(stages[1]["begin_ns"], 1_000);
        assert_eq!(stages[1]["wall_ns"], 4_000);
    }

    #[test]
    fn spans_past_the_capacity_are_counted_not_kept() {
        let log = Box::new(Log::new());
        log.begin(0);
        for _ in 0..CAPACITY + 2 {
            log.record(0, 1, 2);
        }
        let mut timeline = Box::new(Timeline::EMPTY);
        timeline.fill(&log, Some(1), 0, &|ticks| ticks as u64);
        assert_eq!(timeline.len, CAPACITY);
        assert_eq!(timeline.dropped, 2);
    }
}
