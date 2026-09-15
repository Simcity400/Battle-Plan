//! Four-hour slot-3 SWeatherPerProvince neighbour topology, converted to weather slot ids.
use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows, grow_capacity},
};
use engine::{Memory, cweather_manager, sweather_per_province, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{TableId, WeatherProvinceNeighbourRow, tables},
};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::WeatherProvinceNeighbours];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    ids: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: 20_000,
            ids: 131_072,
        }
    }
}

pub struct Captured {
    rows: CaptureRows<sweather_per_province::NeighbourRow>,
    ids: Arena<u32>,
}
impl Captured {
    pub fn with_capacities(capacity: &Capacities, successful: &[u16]) -> Self {
        assert_eq!(successful.len(), N);
        Self {
            rows: CaptureRows::new(capacity.rows, successful[0]),
            ids: Arena::new(capacity.ids, successful[0]),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.ids.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes() + self.ids.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities, successful: &[u16]) {
        assert_eq!(successful.len(), N);
        self.rows.grow(&mut capacity.rows, successful[0]);
        grow_capacity(&mut capacity.ids, self.ids.required);
        self.ids.grow(&mut capacity.ids, successful[0]);
    }
    fn read(
        memory: &impl Memory,
        base: usize,
        state: usize,
        rows: &mut [sweather_per_province::NeighbourRow],
        ids: &mut [u32],
    ) -> sweather_per_province::NeighbourWalk {
        match cweather_manager::Reader::new(memory, base, state) {
            Ok(reader) => reader.province_neighbours(rows, ids),
            Err(reason) => sweather_per_province::NeighbourWalk {
                outcome: Outcome::Unavailable { reason },
                len: 0,
                required: 0,
            },
        }
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        needed: &[bool],
        hour: i32,
        force_full: bool,
    ) -> ([bool; N], [u64; N]) {
        assert_eq!(needed.len(), N);
        self.reset();
        if !replay::cadence::should_walk(TABLES[0], hour, force_full) {
            self.rows.outcome = Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON,
            };
            return ([false], [0]);
        }
        let started = qpc();
        let first = Self::read(memory, base, state, &mut self.rows.rows, &mut self.ids.first);
        self.rows.outcome = first.outcome;
        self.ids.len = first.len;
        self.ids.required = first.required;
        let mut matched = false;
        if needed[0] {
            self.rows.checked = true;
            let second = Self::read(
                memory,
                base,
                state,
                &mut self.rows.second,
                &mut self.ids.second,
            );
            let rows = self.rows.compare(second.outcome);
            let ids = self.ids.compare(second.len, second.required);
            if !ids {
                self.rows.mismatches += 1;
            }
            matched = rows && ids;
        }
        ([matched], [qpc().saturating_sub(started).max(0) as u64])
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [if self.rows.checked && self.rows.mismatches != 0 {
            Outcome::Unavailable {
                reason: "weather neighbours changed during double read",
            }
        } else {
            self.rows.outcome
        }]
    }
    pub fn checked_mismatches(&self) -> [usize; N] {
        [self.rows.mismatches]
    }
    pub fn tables(&self) -> [TableView; N] {
        let outcome = self.outcomes()[0];
        let Some(ids) = self.ids.first.get(..self.ids.len) else {
            return [unavailable("weather neighbour arena length invalid")];
        };
        let rows = &self.rows.rows[..outcome.len().min(self.rows.rows.len())];
        if rows
            .iter()
            .any(|r| r.start.checked_add(r.count as usize).is_none_or(|end| end > ids.len()))
        {
            return [unavailable("weather neighbours lie outside captured arena")];
        }
        [table::<tables::WeatherProvinceNeighbours, _>(
            outcome,
            &self.rows.rows,
            |r| WeatherProvinceNeighbourRow {
                province: r.id,
                neighbours: ids[r.start..r.start + r.count as usize].to_vec(),
            },
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::memory::SliceMemory;
    #[test]
    fn off_hours_do_not_read_and_neighbour_ids_convert_in_native_order() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut capture = Captured::with_capacities(&Capacities { rows: 1, ids: 2 }, &[0]);
        assert_eq!(
            capture.capture(&memory, 0, 0, &[true], 0, false),
            ([false], [0])
        );
        assert!(!capture.capture(&memory, 0, 0, &[true], 0, true).0[0]);
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        capture.reset();
        capture.rows.rows[0] = sweather_per_province::NeighbourRow {
            id: 4,
            data: 0x1000,
            count: 2,
            start: 0,
        };
        capture.ids.first[..2].copy_from_slice(&[9, 2]);
        capture.ids.len = 2;
        capture.rows.outcome = Outcome::Rows { len: 1 };
        let [TableView::Complete(replay::rows::TableRows::WeatherProvinceNeighbours(rows))] =
            capture.tables()
        else {
            panic!("complete");
        };
        assert_eq!(rows[0].neighbours, [9, 2]);
    }
}
