//! The hourly map event interval. The game thread seals its coverage in
//! constant time; the spool thread copies, decodes and writes its events and
//! frees their lane space only after the frame is written.
use crate::{map_events, spooling::unavailable};
use engine_native::walk::Outcome;
use replay_data::{rows::TableId, rows::TableRows};
use replay_frame::{frame::TableInput, frame::TableView};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::MapEvents];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[("map event interval", 0, &[0])];
const ASSEMBLED: &str = "map event interval assembled by spool";
const UNSEALED: &str = "map event interval not sealed this hour";

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities;

collector_primitives::capacity_max!(Capacities;);

pub struct Captured {
    interval: Option<map_events::interval::Interval>,
}
impl Captured {
    pub fn with_capacities(_capacity: &Capacities) -> Self {
        Self { interval: None }
    }
    pub fn reset(&mut self) {
        self.interval = None;
    }
    pub fn allocated_bytes(&self) -> usize {
        0
    }
    pub fn grow(&mut self, _capacity: &mut Capacities) {}
    pub fn outcomes(&self) -> [Outcome; N] {
        [Outcome::Unavailable { reason: ASSEMBLED }]
    }
    pub fn tables(&self) -> [TableView; N] {
        [unavailable(ASSEMBLED)]
    }
    /// Game thread, at the hourly cut.
    pub(crate) fn seal(&mut self, hour: i32) -> u64 {
        let started = crate::sampling::qpc();
        self.interval = Some(map_events::interval::seal(
            hour,
            crate::world_lifetime::token(),
        ));
        crate::sampling::qpc().saturating_sub(started).max(0) as u64
    }
}

#[derive(Default)]
pub struct Ledger;

/// What committing a written interval frees.
pub(crate) struct Publication(map_events::lanes::Assembled);
impl Publication {
    pub(crate) fn commit(self) {
        self.0.commit();
    }
}

impl Ledger {
    /// Spool worker only. An interval that cannot be copied or decoded is
    /// unavailable, and the next one reports its events as lost.
    pub(crate) fn assemble_owned(
        &mut self,
        captured: &Captured,
        tables: &mut [TableInput],
    ) -> Option<Publication> {
        let Some(interval) = captured.interval else {
            tables[TableId::MapEvents.index()] = TableInput::Sampled(unavailable(UNSEALED));
            return None;
        };
        let mut assembled = map_events::lanes::Assembled::default();
        let row = map_events::lanes::assemble(interval.seal, &mut assembled)
            .and_then(|()| map_events::interval::row(&interval, &assembled));
        match row {
            Ok(row) => {
                let mut rows = Vec::new();
                if engine_allocation::reserve_exact(&mut rows, 1).is_err() {
                    map_events::interval::lost_output();
                    tables[TableId::MapEvents.index()] =
                        TableInput::Sampled(unavailable("map event interval allocation failed"));
                    return None;
                }
                rows.push(row);
                tables[TableId::MapEvents.index()] =
                    TableInput::Sampled(TableView::Complete(TableRows::MapEvents(rows)));
                Some(Publication(assembled))
            }
            Err(reason) => {
                // Free what was copied once the frame is written, so a frame
                // that cannot decode is reported lost rather than retried
                // every hour.
                map_events::interval::lost_output();
                tables[TableId::MapEvents.index()] = TableInput::Sampled(unavailable(reason));
                Some(Publication(assembled))
            }
        }
    }
}
