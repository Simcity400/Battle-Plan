pub use super::tables::Capacities;
use replay_data::{rows::TableId, rows::TableRows};
use replay_frame::frame::TableView;
pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::PeaceBids];
pub struct Captured(super::tables::Captured);
impl Captured {
    pub fn vacant() -> Self {
        Self(super::tables::Captured::vacant())
    }
    pub fn fail_capture(&mut self, _hour: i32, _force_full: bool, reason: &'static str) {
        self.0.fail(reason);
    }
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self(super::tables::Captured::with_capacities(capacity))
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
    pub fn outcomes(&self) -> [engine_native_memory::walk::Outcome; N] {
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
        memory: &impl engine_native_memory::Memory,
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
