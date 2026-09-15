//! Hourly client counter grouping level, counter colour mode and unit selection list.
use crate::{sampling::qpc, spooling::table, table_capture::CaptureRows};
use engine::{Memory, grouping_globals, selection_manager_layout as selection, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{CounterSettingsRow, SelectedUnitRow, TableId, tables},
};

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [TableId::CounterSettings, TableId::SelectedUnits];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    selected: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self { selected: 256 }
    }
}

pub struct Captured {
    settings: CaptureRows<grouping_globals::CounterSettingsRow>,
    selected: CaptureRows<selection::SelectedRow>,
}
impl Captured {
    pub fn with_capacities(capacity: &Capacities, successful: &[u16]) -> Self {
        assert_eq!(successful.len(), N);
        Self {
            settings: CaptureRows::new(1, successful[0]),
            selected: CaptureRows::new(capacity.selected, successful[1]),
        }
    }
    pub fn reset(&mut self) {
        self.settings.reset();
        self.selected.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.settings.allocated_bytes() + self.selected.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities, successful: &[u16]) {
        assert_eq!(successful.len(), N);
        let mut one = 1;
        self.settings.grow(&mut one, successful[0]);
        self.selected.grow(&mut capacity.selected, successful[1]);
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        _state: usize,
        needed: &[bool],
    ) -> ([bool; N], [u64; N]) {
        assert_eq!(needed.len(), N);
        let begin = qpc();
        self.settings.outcome =
            grouping_globals::read_counter_settings(memory, base, &mut self.settings.rows);
        self.settings.checked = needed[0];
        let settings = needed[0] && {
            let second =
                grouping_globals::read_counter_settings(memory, base, &mut self.settings.second);
            self.settings.compare(second)
        };
        let middle = qpc();
        self.selected.outcome =
            selection::walk_selected_units(memory, base, &mut self.selected.rows);
        self.selected.checked = needed[1];
        let selected = needed[1] && {
            let second = selection::walk_selected_units(memory, base, &mut self.selected.second);
            self.selected.compare(second)
        };
        let end = qpc();
        (
            [settings, selected],
            [
                middle.saturating_sub(begin).max(0) as u64,
                end.saturating_sub(middle).max(0) as u64,
            ],
        )
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [
            if self.settings.checked && self.settings.mismatches != 0 {
                Outcome::Unavailable {
                    reason: "counter settings changed during double read",
                }
            } else {
                self.settings.outcome
            },
            if self.selected.checked && self.selected.mismatches != 0 {
                Outcome::Unavailable {
                    reason: "unit selection changed during double read",
                }
            } else {
                self.selected.outcome
            },
        ]
    }
    pub fn checked_mismatches(&self) -> [usize; N] {
        [self.settings.mismatches, self.selected.mismatches]
    }
    pub fn tables(&self) -> [TableView; N] {
        let [settings, selected] = self.outcomes();
        [
            table::<tables::CounterSettings, _>(settings, &self.settings.rows, |r| {
                CounterSettingsRow {
                    grouping_level: r.grouping_level,
                    previous_group_count: r.previous_group_count,
                    counter_color_mode: r.counter_color_mode,
                }
            }),
            table::<tables::SelectedUnits, _>(selected, &self.selected.rows, |r| {
                SelectedUnitRow {
                    unit: r.unit,
                    class: r.class,
                    selected: r.selected,
                    removal_pending: r.removal_pending,
                }
            }),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::memory::SliceMemory;
    #[test]
    fn unreadable_roots_fail_each_table_independently_and_convert_raw_fields() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut capture = Captured::with_capacities(&Capacities { selected: 1 }, &[0, 0]);
        assert_eq!(capture.capture(&memory, 0, 0, &[true, true]).0, [false, false]);
        let [settings, selected] = capture.tables();
        assert!(matches!(settings, TableView::Unavailable { .. }));
        assert!(matches!(selected, TableView::Unavailable { .. }));
        capture.reset();
        capture.settings.rows[0] = grouping_globals::CounterSettingsRow {
            grouping_level: 3,
            previous_group_count: -1,
            counter_color_mode: 1,
        };
        capture.settings.outcome = Outcome::Rows { len: 1 };
        capture.selected.rows[0] = selection::SelectedRow {
            unit: (5 << 32) | 51,
            class: 0,
            selected: 1,
            removal_pending: 0,
        };
        capture.selected.outcome = Outcome::Rows { len: 1 };
        let [
            TableView::Complete(replay::rows::TableRows::CounterSettings(settings)),
            TableView::Complete(replay::rows::TableRows::SelectedUnits(selected)),
        ] = capture.tables()
        else {
            panic!("complete");
        };
        assert_eq!(settings[0].previous_group_count, -1);
        assert_eq!(selected[0].unit, (5 << 32) | 51);
    }
}
