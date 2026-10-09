//! Resident history bids sampled before finalization for the fresh-save oracle.
pub use super::peace_settlements::Capacities;
use replay::{
    frame::TableView,
    rows::{TableId, TableRows},
};
pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::PeaceBids];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];
pub struct Captured(super::peace_settlements::Captured);
impl Captured {
    pub(crate) fn vacant() -> Self {
        Self(super::peace_settlements::Captured::vacant())
    }
    pub(crate) fn fail_capture(&mut self, _hour: i32, _force_full: bool, reason: &'static str) {
        self.0.fail(reason);
    }
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self(super::peace_settlements::Captured::with_capacities(
            capacity,
        ))
    }
    pub fn reset(&mut self) {
        self.0.reset();
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.0.grow(capacity);
    }
    pub fn allocated_bytes(&self) -> usize {
        self.0.allocated_bytes()
    }
    pub fn outcomes(&self) -> [engine::walk::Outcome; N] {
        self.0.outcomes()
    }
    pub fn tables(&self) -> [TableView; N] {
        self.0.tables().map(|view| match view {
            TableView::Complete(TableRows::PeaceSettlements(rows)) => {
                TableView::Complete(TableRows::PeaceBids(rows))
            }
            TableView::Truncated {
                rows: TableRows::PeaceSettlements(rows),
                missing,
            } => TableView::Truncated {
                rows: TableRows::PeaceBids(rows),
                missing,
            },
            view => view,
        })
    }
    pub fn capture(
        &mut self,
        memory: &impl engine::Memory,
        base: usize,
        state: usize,
        hour: i32,
        _force_full: bool,
    ) -> [u64; N] {
        let started = super::qpc();
        let _reads = crate::read_accounting::table(TableId::PeaceBids);
        self.0.capture_bids(memory, base, state, hour);
        [super::elapsed(started)]
    }
}
