use super::{Knobs, Measured, Outcome, Reading};
use replay::container::{BlockIndex, Keep, Replay};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    io,
    marker::PhantomData,
    panic::{AssertUnwindSafe, catch_unwind},
    rc::Rc,
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::Duration,
};

thread_local! {
    static ACTIVE: RefCell<Option<Arc<State>>> = const { RefCell::new(None) };
}
#[derive(Default)]
pub(crate) struct State {
    counts: Mutex<Counts>,
}
#[derive(Default)]
struct Counts {
    tasks: u64,
    families: BTreeMap<&'static str, CountsForFamily>,
}
#[derive(Default)]
struct CountsForFamily {
    blocks: u64,
    failures: u64,
    declined: u64,
    first_decline: Option<String>,
    peak_fact_bytes: Option<u64>,
}
pub(crate) struct Session {
    state: Arc<State>,
    _thread: PhantomData<Rc<()>>,
}
impl Session {
    pub(crate) fn start() -> io::Result<Self> {
        ACTIVE.with(|active| {
            let mut active = active.borrow_mut();
            if active.is_some() {
                return Err(io::Error::other(
                    "A borrowed extraction runtime is already active",
                ));
            }
            let state = Arc::new(State::default());
            *active = Some(state.clone());
            Ok(Self {
                state,
                _thread: PhantomData,
            })
        })
    }
    pub(crate) fn report(&self) -> io::Result<Value> {
        let counts = self
            .state
            .counts
            .lock()
            .map_err(|_| io::Error::other("Borrowed runtime telemetry panicked"))?;
        let names: Vec<_> = counts.families.keys().copied().collect();
        Ok(
            json!({"partial":true,"S3":false,"reader_tasks":counts.tasks,
            "delivered_family_count":names.len(),"delivered_families":names,
            "families":counts.families.iter().map(|(name, n)| json!({"family":name,
                "path":"main field-watch/quiet/deadline Extract delivery; the pre-pass workers' tasks; main Branch::fold and writer",
                "blocks":n.blocks,"borrowed_hours":null,"failures":n.failures,"declined_blocks":n.declined,
                "first_named_decline":n.first_decline,"declined_path":"main ordinary observation; no fabricated facts",
                "peak_block_fact_bytes":n.peak_fact_bytes})).collect::<Vec<_>>(),
            "spool":null,"W_bound":null,"private_memory_bound":null,
            "prepass_worker_concurrency":super::workers(crate::prepared::pipeline::parallelism()),
            "concurrency_policy":"Up to two live prepass tasks; main decoder helpers, independent folds, ordinary readers and side jobs retain their existing parallelism",
            "scope":"One main watched read per scheduled prepass task with its existing family-selected borrowed projections, not one global decode per physical block; old observer retention, folds and historical reads remain",
            "observation":"Main dispatcher unchanged: read_by_changes, observe_reported, quiet/deadline and block-boundary carry; hour delivery count is not measured",
            "publication":"Unmodified main block publication, Batches/64MiB cadence, coverage and commit driver"}),
        )
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        ACTIVE.with(|active| {
            active.borrow_mut().take();
        });
    }
}
pub(super) fn current() -> Option<Arc<State>> {
    ACTIVE.with(|active| active.borrow().clone())
}

#[derive(Debug)]
struct Failure {
    family: &'static str,
    ordinal: usize,
    branch: u64,
    hour: i32,
    operation: &'static str,
    source: Arc<io::Error>,
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Runtime family {}, physical block {}, branch {}, hour {}, {}: {}",
            self.family, self.ordinal, self.branch, self.hour, self.operation, self.source
        )
    }
}
impl std::error::Error for Failure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&*self.source)
    }
}
fn named(
    family: &'static str,
    ordinal: usize,
    index: &BlockIndex,
    hour: i32,
    operation: &'static str,
    source: Arc<io::Error>,
) -> io::Error {
    io::Error::new(
        source.kind(),
        Failure {
            family,
            ordinal,
            branch: index.branch,
            hour,
            operation,
            source,
        },
    )
}
#[derive(Debug)]
struct ObserverPanic;
impl std::fmt::Display for ObserverPanic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Borrowed observer panicked")
    }
}
impl std::error::Error for ObserverPanic {}
pub(super) fn fatal(error: &io::Error) -> bool {
    panicking(error) || error.kind() == io::ErrorKind::Interrupted
}
pub(super) fn panicking(error: &io::Error) -> bool {
    error.get_ref().is_some_and(|inner| {
        inner.is::<ObserverPanic>()
            || inner
                .downcast_ref::<Failure>()
                .is_some_and(|named| panicking(&named.source))
    })
}
pub(super) fn checked<T>(run: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    catch_unwind(AssertUnwindSafe(run)).unwrap_or_else(|_| Err(io::Error::other(ObserverPanic)))
}
pub(super) fn extract(
    replay: &Replay,
    index: &BlockIndex,
    families: &[Reading<'_>],
    stop: &AtomicBool,
    knobs: &Knobs,
    state: &State,
    keep: Option<Keep<'_>>,
) -> Measured {
    let measured = super::extract_main(replay, index, families, stop, knobs, keep);
    let mut counts = match state.counts.lock() {
        Ok(counts) => counts,
        Err(_) => {
            return failed(
                families,
                Arc::new(io::Error::other("Runtime telemetry panicked")),
            );
        }
    };
    counts.tasks += measured.reads;
    for (reading, (outcome, _, bytes)) in families.iter().zip(&measured.outcomes) {
        let entry = counts.families.entry(reading.family.name).or_default();
        entry.blocks += 1;
        match outcome {
            Outcome::Facts(_) => {
                entry.peak_fact_bytes = Some(
                    entry
                        .peak_fact_bytes
                        .map_or(*bytes, |prior| prior.max(*bytes)),
                );
            }
            Outcome::Declined(error) => {
                entry.declined += 1;
                if entry.first_decline.is_none() {
                    let reason = error.to_string();
                    eprintln!("preparation-cost: {reason}; using main ordinary observation");
                    entry.first_decline = Some(reason);
                }
            }
            Outcome::Sequential => entry.declined += 1,
            Outcome::Failed(_) => entry.failures += 1,
        }
    }
    measured
}
pub(super) fn declined(
    replay: &Replay,
    index: &BlockIndex,
    family: &'static str,
    source: io::Error,
) -> Outcome {
    let fatal = fatal(&source);
    let ordinal = replay
        .block_position(index)
        .ok_or_else(|| io::Error::other("Runtime block is absent"))
        .and_then(|position| replay.block_ordinal(position));
    let error = match ordinal {
        Ok(ordinal) => named(
            family,
            ordinal,
            index,
            index.last_hour,
            "extract",
            Arc::new(source),
        ),
        Err(error) => error,
    };
    if fatal {
        Outcome::Failed(error)
    } else {
        Outcome::Declined(error)
    }
}
pub(super) fn panicked(replay: &Replay, index: &BlockIndex, families: &[Reading<'_>]) -> Measured {
    let source = Arc::new(io::Error::other("Borrowed runtime task panicked"));
    let ordinal = replay
        .block_position(index)
        .ok_or_else(|| io::Error::other("Runtime block is absent"))
        .and_then(|position| replay.block_ordinal(position));
    match ordinal {
        Ok(ordinal) => Measured {
            outcomes: families
                .iter()
                .map(|reading| {
                    (
                        Outcome::Failed(named(
                            reading.family.name,
                            ordinal,
                            index,
                            index.last_hour,
                            "task",
                            source.clone(),
                        )),
                        Duration::ZERO,
                        0,
                    )
                })
                .collect(),
            read: Duration::ZERO,
            reads: 0,
        },
        Err(error) => failed(families, Arc::new(error)),
    }
}

#[derive(Debug)]
struct SetupFailure {
    family: &'static str,
    source: Arc<io::Error>,
}
impl std::fmt::Display for SetupFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Runtime family {} setup: {}", self.family, self.source)
    }
}
impl std::error::Error for SetupFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&*self.source)
    }
}
fn failed(families: &[Reading<'_>], source: Arc<io::Error>) -> Measured {
    Measured {
        outcomes: families
            .iter()
            .map(|family| {
                (
                    Outcome::Failed(io::Error::new(
                        source.kind(),
                        SetupFailure {
                            family: family.family.name,
                            source: source.clone(),
                        },
                    )),
                    Duration::ZERO,
                    0,
                )
            })
            .collect(),
        read: Duration::ZERO,
        reads: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Extract, Family};
    use super::*;
    struct Declining;
    impl Extract for Declining {
        fn observe(
            &mut self,
            _: &Replay,
            _: &replay_frame::frame::DecodedFrame,
            _: &crate::prepared::frame_changes::FrameChanges,
        ) -> io::Result<()> {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "synthetic fast-path decline",
            ))
        }
        fn finish(self: Box<Self>) -> io::Result<Box<dyn std::any::Any + Send>> {
            panic!("a declined extract must never finish")
        }
    }
    static DECLINING: Family = Family {
        name: "synthetic-decline",
        extract: "synthetic.extract",
        sequential: "synthetic.sequential",
        fallback: "synthetic.fallback",
        tables: &[],
        fields: None,
        start: |_| Box::new(Declining),
    };
    #[test]
    fn named_decline_uses_main_fallback_and_has_no_fact_measurement() {
        use crate::test_fixtures::{CAMPAIGN, records, source};
        let replay = Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![source(1, 100, Some(1))])],
        )
        .unwrap();
        let session = Session::start().unwrap();
        let measured = extract(
            &replay,
            &replay.index[0],
            &[Reading::of(&DECLINING)],
            &AtomicBool::new(false),
            &Knobs {
                runtime: Some(session.state.clone()),
                ..Knobs::default()
            },
            &session.state,
            None,
        );
        let (outcome, _, _) = measured.outcomes.into_iter().next().unwrap();
        let Outcome::Declined(error) = outcome else {
            panic!("decline must use ordinary main observation")
        };
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(
            error
                .to_string()
                .contains("synthetic-decline, physical block")
        );
        let report = session.report().unwrap();
        let row = &report["families"][0];
        assert_eq!(row["declined_blocks"], 1);
        assert!(row["peak_block_fact_bytes"].is_null());
        assert!(
            row["first_named_decline"]
                .as_str()
                .unwrap()
                .contains("synthetic fast-path decline")
        );
    }
    #[test]
    fn session_is_scoped_and_missing_measurements_are_null() {
        assert!(current().is_none());
        let session = Session::start().unwrap();
        assert!(Session::start().is_err());
        assert!(current().is_some());
        let report = session.report().unwrap();
        assert!(report["private_memory_bound"].is_null());
        assert!(report["spool"].is_null());
        assert!(report["delivered_families"].as_array().unwrap().is_empty());
        drop(session);
        assert!(current().is_none());
    }
}
