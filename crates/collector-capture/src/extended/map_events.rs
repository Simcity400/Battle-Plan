use collector_map_events as map_events;
use collector_projection::table_conversion::unavailable;
use engine_native::walk::Outcome;
use replay_data::{rows::TableId, rows::TableRows};
use replay_frame::{frame::TableInput, frame::TableView};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::MapEvents];
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
    pub(crate) fn seal(&mut self, hour: i32) -> u64 {
        let started = collector_primitives::clock::qpc();
        // SAFETY: Runs on the admitted hourly capture boundary as the sole map-events consumer
        // while map-event lanes remain live.
        self.interval = Some(unsafe {
            map_events::interval::seal(hour, collector_lifetime::world_lifetime::token())
        });
        collector_primitives::clock::qpc()
            .saturating_sub(started)
            .max(0) as u64
    }
}

#[derive(Default)]
pub struct Ledger;

pub struct Publication(map_events::lanes::Assembled);
impl Publication {
    pub fn allocated_bytes(&self) -> usize {
        self.0.allocated_bytes()
    }

    /// # Safety
    /// Acknowledge only the originating session's actual queue admission, while its observer
    /// storage is live and before commit or reset.
    pub unsafe fn admitted(&self) {
        // SAFETY: This is the spool's actual queue admission while its originating observer
        // remains live.
        unsafe { self.0.admitted() };
    }
    /// # Safety
    /// Acknowledge only after the originating session's writer confirms this publication, once per
    /// assembled publication in assembly order, before detaching or releasing its event lanes.
    pub unsafe fn commit(self) {
        // SAFETY: The originating spool has the writer receipt and still owns the observers.
        unsafe { self.0.commit() };
    }
}

/// # Safety
/// Only the originating spool may reset its admitted frontier after discarding uncommitted output,
/// while the event lanes are still live.
pub unsafe fn reset_admitted() {
    // SAFETY: The sole spool has invalidated pending writes and still owns its observer frontier.
    unsafe { map_events::lanes::reset_admitted() };
}

impl Ledger {
    /// # Safety
    /// Only the spool owning this frame's event interval may assemble it. Retain the session's
    /// event lanes until every publication is admitted and committed or discarded.
    pub unsafe fn assemble_owned(
        &mut self,
        captured: &Captured,
        tables: &mut [TableInput],
    ) -> Option<Publication> {
        let Some(interval) = captured.interval else {
            tables[TableId::MapEvents.index()] = TableInput::Sampled(unavailable(UNSEALED));
            return None;
        };
        let mut assembled = map_events::lanes::Assembled::default();
        // SAFETY: The originating spool is the sole consumer assembling this frame's sealed interval
        // while the session's map-event lanes remain live.
        let row = unsafe { map_events::lanes::assemble(interval.seal, &mut assembled) }
            .and_then(|()| map_events::interval::row(&interval, &assembled));
        match row {
            Ok(row) => {
                let mut rows = Vec::new();
                if engine_allocation::reserve_exact(&mut rows, 1).is_err() {
                    // SAFETY: Originating spool owns map-event assembly failure reporting.
                    unsafe { map_events::interval::lost_output() };
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
                // SAFETY: Originating spool owns map-event assembly failure reporting.
                unsafe { map_events::interval::lost_output() };
                tables[TableId::MapEvents.index()] = TableInput::Sampled(unavailable(reason));
                Some(Publication(assembled))
            }
        }
    }
}
