//! Deterministic work counters for the naval observation regression fixtures.
use std::cell::RefCell;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Work {
    pub defines: usize,
    pub units: usize,
    pub regions: usize,
    pub retention_spans: usize,
    pub events: usize,
    pub names: usize,
    pub results: usize,
    pub convoys: usize,
    pub fleets: usize,
    pub combats: usize,
    pub values: usize,
}
thread_local! {
    pub static WORK: RefCell<Work> = RefCell::new(Work::default());
}
