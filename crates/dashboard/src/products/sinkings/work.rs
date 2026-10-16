//! Deterministic work counters for the naval observation regression fixtures.
use std::cell::RefCell;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Work {
    pub(crate) defines: usize,
    pub(crate) units: usize,
    pub(crate) regions: usize,
    pub(crate) retention_spans: usize,
    pub(crate) events: usize,
    pub(crate) names: usize,
    pub(crate) results: usize,
    pub(crate) convoys: usize,
    pub(crate) fleets: usize,
    pub(crate) combats: usize,
    pub(crate) values: usize,
}
thread_local! {
    pub(crate) static WORK: RefCell<Work> = RefCell::new(Work::default());
}
