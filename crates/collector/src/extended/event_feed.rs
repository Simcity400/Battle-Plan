//! Hourly pointer-free Event feed handoff; the spool encodes and acknowledges it.
use engine::{Memory, walk::Outcome};
use replay::{frame::TableView, rows::TableId};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::EventFeedHistory];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    pub events: usize,
}
const ASSEMBLED: &str = "Event feed assembled by spool";

pub struct Captured {
    snapshot: crate::event_feed_history::Snapshot,
    sampled: bool,
}

impl Captured {
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities { events: 0 })
    }
    pub(crate) fn fail(&mut self, hour: i32, reason: &'static str) {
        crate::event_feed_observer::lost_drain();
        self.snapshot
            .refuse(crate::world_lifetime::generation(), hour, reason);
        self.sampled = true;
    }
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            snapshot: crate::event_feed_history::Snapshot::new(capacity.events),
            sampled: false,
        }
    }
    pub fn reset(&mut self) {
        self.sampled = false;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.snapshot.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        crate::table_capture::grow_capacity(&mut capacity.events, self.snapshot.required);
        let _ = self.snapshot.grow(capacity.events);
    }
    pub(crate) fn drain_growing(&mut self, hour: i32, partial: bool) {
        crate::event_feed_observer::capture_growing(&mut self.snapshot, hour, partial);
        self.sampled = true;
    }
    pub(crate) fn discard(&mut self) {
        if self.sampled {
            crate::event_feed_observer::discarded(self.snapshot.publication());
            self.sampled = false;
        }
    }
    pub(crate) fn prepare_pack(
        &self,
    ) -> std::io::Result<Option<crate::event_feed_history::Prepared>> {
        self.sampled
            .then(|| self.snapshot.prepare_pack())
            .transpose()
    }
    pub fn capture(&mut self, _: &impl Memory, _: usize, _: usize) -> [u64; N] {
        let start = crate::sampling::qpc();
        [crate::sampling::qpc().saturating_sub(start).max(1) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [Outcome::Unavailable { reason: ASSEMBLED }]
    }
    pub fn tables(&self) -> [TableView; N] {
        [crate::spooling::unavailable(ASSEMBLED)]
    }
}
