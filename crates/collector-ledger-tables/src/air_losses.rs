use crate::{
    clock::qpc,
    table_capture::{CaptureRows, grow_vec},
    table_conversion::unavailable,
};
use collector_observers::air_losses as producer;
use engine_allocation as allocation;
use engine_native::{Memory, walk::Outcome};
use replay_data::{rows::AirLossEventRow, rows::TableId, rows::TableRows};
use replay_frame::frame::TableView;

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [TableId::AirLossEvents, TableId::AirDailyRoster];
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    pub events: usize,
    pub roster: usize,
}

collector_primitives::capacity_max!(Capacities { events, roster });
impl Capacities {
    pub fn raise_for_countries(&mut self, slots: usize) {
        self.roster = self.roster.max(slots);
    }
}
pub struct Captured {
    rows: CaptureRows<producer::Entry>,
    roster: Vec<engine_air_history::air_loss_sites::RosterRow>,
    roster_outcome: Outcome,
}
impl Captured {
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities::default())
    }
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.events),
            roster: allocation::owned_vec(allocation::vec(capacity.roster)),
            roster_outcome: Outcome::Rows { len: 0 },
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.roster.clear();
        self.roster_outcome = Outcome::Rows { len: 0 };
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.roster.capacity()
                * std::mem::size_of::<engine_air_history::air_loss_sites::RosterRow>()
    }
    /// # Safety
    /// Run only during attach or the active session's serial sizing pass (or an isolated test),
    /// while air-loss observer storage remains live and teardown is excluded.
    pub unsafe fn grow(&mut self, capacity: &mut Capacities) {
        // SAFETY: Caller guarantees attach or serial sizing pass with live air-loss storage.
        unsafe { producer::presize(0) };
        capacity.events = capacity.events.max(producer::capacity());
        grow_vec(&mut self.rows.rows, capacity.events);
        grow_vec(&mut self.roster, capacity.roster);
    }
    pub fn fail(&mut self, reason: &'static str) {
        let outcome = Outcome::Unavailable { reason };
        self.rows.fail(outcome);
        self.roster_outcome = outcome;
    }
    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        self.reset();
        let start = qpc();
        self.rows.outcome = match producer::drain(&mut self.rows.rows) {
            Ok(len) => Outcome::Rows { len },
            Err(reason) => Outcome::Unavailable { reason },
        };
        let drained = qpc().saturating_sub(start).max(0) as u64;
        let start = qpc();
        self.roster_outcome =
            match engine_air_history::air_loss_sites::roster(memory, base, state, &mut self.roster)
            {
                Ok(()) => Outcome::Rows {
                    len: self.roster.len(),
                },
                Err(reason) => Outcome::Unavailable { reason },
            };
        [drained, qpc().saturating_sub(start).max(0) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome, self.roster_outcome]
    }
    pub fn tables(&self) -> [TableView; N] {
        let losses = match self.rows.outcome {
            Outcome::Rows { len } => match project(&self.rows.rows[..len]) {
                Ok(rows) => TableView::Complete(TableRows::AirLossEvents(rows)),
                Err(reason) => {
                    producer::lost_output(len);
                    unavailable(reason)
                }
            },
            Outcome::Unavailable { reason } => unavailable(reason),
            _ => unavailable("air loss incomplete drain"),
        };
        let roster = crate::table_conversion::table::<replay_data::rows::tables::AirDailyRoster, _>(
            self.roster_outcome,
            &self.roster,
            |row| replay_data::rows::AirDailyRosterRow {
                country: row.country,
                eligible: row.eligible,
            },
        );
        [losses, roster]
    }
}

fn project(events: &[producer::Entry]) -> Result<Vec<AirLossEventRow>, &'static str> {
    allocation::collect_vec(events.iter().map(|entry| AirLossEventRow {
        id: entry.id,
        game_hour: entry.game_hour,
        country: entry.country,
        region: entry.region,
        native_cause: entry.native_cause,
        family_base: entry.family_base,
        credited_enemy: entry.credited_enemy,
        amount_raw: entry.amount_raw,
        opponent: entry.opponent,
        daily_slot: entry.daily_slot,
    }))
    .map_err(|_| "collector allocation failed")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn projection_preserves_compact_native_context() {
        let entry = producer::Entry {
            id: 99,
            game_hour: 48,
            country: 4999,
            region: 701,
            family_base: 71,
            native_cause: 6,
            credited_enemy: true,
            amount_raw: -100000,
            opponent: Some(5001),
            daily_slot: 29,
        };
        let rows = project(&[entry]).unwrap();
        assert_eq!(rows[0].amount_raw, entry.amount_raw);
        assert_eq!(rows[0].opponent, entry.opponent);
        assert_eq!(rows[0].daily_slot, entry.daily_slot);
        assert_eq!(
            (rows[0].game_hour, rows[0].family_base, rows[0].native_cause),
            (48, 71, 6)
        );
        assert!(project(&[]).unwrap().is_empty());
    }
    #[test]
    fn roster_storage_is_presized_and_grows_off_thread_for_live_slots() {
        let mut capacity = Capacities::default();
        capacity.raise_for_countries(5000);
        let mut captured = Captured::with_capacities(&capacity);
        assert!(captured.roster.capacity() >= 5000);
        let pointer = captured.roster.as_ptr();
        captured.roster.extend((0..5000).map(|country| {
            engine_air_history::air_loss_sites::RosterRow {
                country,
                eligible: true,
            }
        }));
        assert_eq!(captured.roster.as_ptr(), pointer);
        capacity.raise_for_countries(7000);
        // SAFETY: Isolated unit test sizing local buffers with no live collector attached.
        unsafe { captured.grow(&mut capacity) };
        assert!(captured.roster.capacity() >= 7000);
    }
}
