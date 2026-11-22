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
        self.interval = Some(map_events::interval::seal(
            hour,
            collector_lifetime::world_lifetime::token(),
        ));
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

    pub fn admitted(&self, _owner: &mut crate::sampling::SpoolOwner) {
        self.0.admitted();
    }
    pub fn commit(self, _owner: &mut crate::sampling::SpoolOwner) {
        self.0.commit();
    }
}

pub fn reset_admitted(owner: &mut crate::sampling::SpoolOwner) {
    if let Some(access) = owner.capture_access() {
        map_events::lanes::reset_admitted_live(access);
    }
}

impl Ledger {
    pub fn assemble_owned(
        &mut self,
        owner: &mut crate::sampling::SpoolOwner,
        captured: &Captured,
        tables: &mut [TableInput],
    ) -> Option<Publication> {
        let (Some(interval), Some(access)) = (captured.interval, owner.capture_access()) else {
            tables[TableId::MapEvents.index()] = TableInput::Sampled(unavailable(UNSEALED));
            return None;
        };
        let mut assembled = map_events::lanes::Assembled::default();
        let row = map_events::lanes::assemble_live(*access, interval.seal, &mut assembled)
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
                map_events::interval::lost_output();
                tables[TableId::MapEvents.index()] = TableInput::Sampled(unavailable(reason));
                Some(Publication(assembled))
            }
        }
    }
}
