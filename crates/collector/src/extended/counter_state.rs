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
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self {
            settings: CaptureRows::new(0),
            selected: CaptureRows::new(0),
        }
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            settings: CaptureRows::new(1),
            selected: CaptureRows::new(capacity.selected),
        }
    }
    pub fn reset(&mut self) {
        self.settings.reset();
        self.selected.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.settings.allocated_bytes() + self.selected.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        let mut one = 1;
        self.settings.grow(&mut one);
        self.selected.grow(&mut capacity.selected);
    }
    pub fn capture(&mut self, memory: &impl Memory, base: usize, _state: usize) -> [u64; N] {
        let _settings_reads = crate::read_accounting::table(TABLES[0]);
        let begin = qpc();
        self.settings.outcome =
            grouping_globals::read_counter_settings(memory, base, &mut self.settings.rows);
        let middle = qpc();
        let _selected_reads = crate::read_accounting::table(TABLES[1]);
        self.selected.outcome =
            selection::walk_selected_units(memory, base, &mut self.selected.rows);
        let end = qpc();
        [
            middle.saturating_sub(begin).max(0) as u64,
            end.saturating_sub(middle).max(0) as u64,
        ]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.settings.outcome, self.selected.outcome]
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
            table::<tables::SelectedUnits, _>(selected, &self.selected.rows, |r| SelectedUnitRow {
                unit: r.unit,
                class: r.class,
                selected: r.selected,
                removal_pending: r.removal_pending,
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
        let mut capture = Captured::with_capacities(&Capacities { selected: 1 });
        capture.capture(&memory, 0, 0);
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
