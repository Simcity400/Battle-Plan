//! Hourly class-13 simulation operands; map presentation remains viewer-owned.
//!
//! One walk of every country's railway-gun roster. Guns move, fight and lose
//! strength at any hour, and a vanilla campaign holds tens of them, so the
//! table is hourly; rows and route cells grow from the demand a short hour
//! reports, never from a fixed bound.
use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows, grow_capacity},
};
use engine::allocation::{collect_vec, copy_vec, owned_vec};
use engine::{Memory, cunit::FirstCombatMemo, railway_gun_counter_inputs, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{RailwayGunCounterInputRow, TableId, tables},
};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::RailwayGunCounterInputs];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    paths: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: 64,
            paths: 1024,
        }
    }
}

pub struct Captured {
    rows: CaptureRows<railway_gun_counter_inputs::Row>,
    paths: Arena<u32>,
    /// First-combat answers for this capture's guns. The gun job runs apart from the
    /// division movement packet, so it keeps its own memo, sized off-thread.
    combats: FirstCombatMemo,
}
impl Captured {
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities { rows: 0, paths: 0 })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            paths: Arena::new(capacity.paths),
            combats: FirstCombatMemo::try_new(capacity.rows)
                .map_err(crate::allocation::io)
                .unwrap_or_default(),
        }
    }

    pub fn reset(&mut self) {
        self.rows.reset();
        self.paths.reset();
        self.combats.clear();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes() + self.paths.allocated_bytes() + self.combats.allocated_bytes()
    }

    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.rows.grow(&mut capacity.rows);
        grow_capacity(&mut capacity.paths, self.paths.required);
        self.paths.grow(&mut capacity.paths);
        let _ = self
            .combats
            .reserve(capacity.rows)
            .map_err(crate::allocation::io);
    }

    pub fn capture(&mut self, memory: &impl Memory, base: usize, game_state: usize) -> [u64; N] {
        let _reads = crate::read_accounting::table(TABLES[0]);
        let begin = qpc();
        // Answers are valid for one capture only; clearing keeps the reserved room.
        self.combats.clear();
        let walk = railway_gun_counter_inputs::walk(
            memory,
            base,
            game_state,
            &mut self.rows.rows,
            &mut self.paths.first,
            &mut self.combats,
        );
        self.rows.outcome = walk.outcome;
        self.paths.len = walk.paths_len;
        self.paths.required = walk.required_paths;
        [qpc().saturating_sub(begin).max(0) as u64]
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }

    pub fn tables(&self) -> [TableView; N] {
        let Some(paths) = self.paths.first.get(..self.paths.len) else {
            return [unavailable("railway-gun path arena length invalid")];
        };
        let rows = &self.rows.rows[..self.rows.outcome.len().min(self.rows.rows.len())];
        if rows.iter().any(|row| {
            [
                row.ordinary_start.checked_add(row.ordinary_len),
                row.alternate_start.checked_add(row.alternate_len),
            ]
            .iter()
            .any(|end| end.is_none_or(|end| end > paths.len()))
        }) {
            return [unavailable(
                "railway-gun route lies outside the captured arena",
            )];
        }
        // Each gun sits in exactly one country roster; a repeated handle is a
        // torn census. Checked here, off the game thread, in n log n.
        let mut handles: Vec<u64> = match collect_vec(rows.iter().map(|row| row.gun)) {
            Ok(handles) => handles,
            Err(_) => return [unavailable("collector allocation failed")],
        };
        handles.sort_unstable();
        if handles.windows(2).any(|pair| pair[0] == pair[1]) {
            return [unavailable("railway gun appears in more than one roster")];
        }
        let route = |start: usize, len: usize| owned_vec(copy_vec(&paths[start..start + len]));
        [table::<tables::RailwayGunCounterInputs, _>(
            self.rows.outcome,
            &self.rows.rows,
            |r| RailwayGunCounterInputRow {
                gun: r.gun,
                roster_owner: r.roster_owner,
                expeditionary_owner: r.expeditionary_owner,
                logical_owner: r.logical_owner,
                province: r.province,
                naval_transfer: u8::from(r.naval_transfer),
                transfer_action_status: u8::from(r.transfer_action_status),
                exiled: u8::from(r.exiled),
                retreat: u8::from(r.retreat),
                strategic_redeployment: u8::from(r.strategic_redeployment),
                reference_entry_count: r.reference_entry_count,
                assignee_group: r.assignee_group,
                subunit_token: r.subunit_token,
                current_strength: r.current_strength,
                maximum_strength: r.maximum_strength,
                received_supply_ratio: r.received_supply_ratio,
                railway_building_level: r.railway_building_level,
                target: r.target,
                combat_role: r.combat_role,
                combat_province: r.combat_province,
                combat_count: r.combat_count,
                attacking: u8::from(r.attacking),
                ordinary_path: route(r.ordinary_start, r.ordinary_len),
                alternate_path: route(r.alternate_start, r.alternate_len),
            },
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::memory::SliceMemory;
    use replay::rows::TableRows;

    fn gun(handle: u64) -> railway_gun_counter_inputs::Row {
        railway_gun_counter_inputs::Row {
            gun: handle,
            roster_owner: 88,
            expeditionary_owner: 9,
            logical_owner: 10,
            province: 12,
            retreat: true,
            reference_entry_count: 2,
            assignee_group: (3_u64 << 32) | 70,
            subunit_token: 42,
            current_strength: -123_456,
            maximum_strength: 234_567,
            received_supply_ratio: -500,
            railway_building_level: Some(-3),
            target: 13,
            combat_role: 2,
            combat_province: 13,
            combat_count: 3,
            attacking: true,
            ordinary_start: 0,
            ordinary_len: 2,
            alternate_start: 2,
            alternate_len: 1,
            ..Default::default()
        }
    }

    #[test]
    fn projects_native_operands_and_refuses_bad_arenas_and_repeated_guns() {
        let mut capture = Captured::with_capacities(&Capacities { rows: 2, paths: 3 });
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        capture.capture(&memory, 0, 0);
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        capture.reset();
        capture.rows.outcome = Outcome::Rows { len: 0 };
        assert!(matches!(
            capture.tables()[0],
            TableView::Complete(TableRows::RailwayGunCounterInputs(ref rows)) if rows.is_empty()
        ));
        capture.rows.rows[0] = gun((7_u64 << 32) | 4713);
        capture.paths.first.copy_from_slice(&[13, 14, 15]);
        capture.paths.len = 3;
        capture.rows.outcome = Outcome::Rows { len: 1 };
        let views = capture.tables();
        let TableView::Complete(TableRows::RailwayGunCounterInputs(rows)) = &views[0] else {
            panic!("complete railway-gun table");
        };
        let row = &rows[0];
        assert_eq!(
            (row.ordinary_path.as_slice(), row.alternate_path.as_slice()),
            (&[13, 14][..], &[15][..])
        );
        assert_eq!(row.railway_building_level, Some(-3));
        assert_eq!(
            (row.retreat, row.reference_entry_count, row.exiled),
            (1, 2, 0)
        );
        assert_eq!(row.received_supply_ratio, -500);

        capture.rows.rows[0].alternate_len = 2;
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        capture.rows.rows[0].alternate_len = 1;
        capture.rows.rows[1] = gun((7_u64 << 32) | 4713);
        capture.rows.outcome = Outcome::Rows { len: 2 };
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        capture.rows.outcome = Outcome::Truncated {
            len: 1,
            unread_rows: 2,
        };
        assert!(matches!(capture.tables()[0], TableView::Truncated { .. }));
    }

    #[test]
    fn truncated_route_demand_grows_the_arena() {
        let mut capacity = Capacities { rows: 1, paths: 2 };
        let mut capture = Captured::with_capacities(&capacity);
        capture.rows.outcome = Outcome::Truncated {
            len: 0,
            unread_rows: 1,
        };
        capture.paths.required = 9;
        capture.grow(&mut capacity);
        assert!(capacity.paths >= 9 && capture.paths.first.len() >= 9);
        assert!(capacity.rows >= 1);
    }
}
