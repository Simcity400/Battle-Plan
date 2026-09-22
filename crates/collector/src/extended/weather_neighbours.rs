//! Static SWeatherPerProvince neighbour topology, converted to weather slot ids.
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
    pub(crate) fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.rows.fail(super::failed_capture(TABLES[0], hour, force_full, reason));
    }

    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities { rows: 0, ids: 0 })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            ids: Arena::new(capacity.ids),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.ids.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes() + self.ids.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.rows.grow(&mut capacity.rows);
        grow_capacity(&mut capacity.ids, self.ids.required);
        self.ids.grow(&mut capacity.ids);
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
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        let _reads = crate::read_accounting::table(TABLES[0]);
        self.reset();
        if !replay::cadence::should_walk(TABLES[0], hour, force_full) {
            self.rows.outcome = Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON,
            };
            return [0];
        }
        let started = qpc();
        let first = Self::read(
            memory,
            base,
            state,
            &mut self.rows.rows,
            &mut self.ids.first,
        );
        self.rows.outcome = first.outcome;
        self.ids.len = first.len;
        self.ids.required = first.required;
        [qpc().saturating_sub(started).max(0) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }
    pub fn tables(&self) -> [TableView; N] {
        let outcome = self.outcomes()[0];
        let Some(ids) = self.ids.first.get(..self.ids.len) else {
            return [unavailable("weather neighbour arena length invalid")];
        };
        let rows = &self.rows.rows[..outcome.len().min(self.rows.rows.len())];
        if rows.iter().any(|r| {
            r.start
                .checked_add(r.count as usize)
                .is_none_or(|end| end > ids.len())
        }) {
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
    use engine::memory::{CountedMemory, SliceMemory};
    #[test]
    fn neighbours_are_read_on_full_hours_only_and_keep_native_order() {
        let memory = CountedMemory::new(SliceMemory {
            base: 0,
            bytes: &[],
        });
        let mut capture = Captured::with_capacities(&Capacities { rows: 1, ids: 2 });
        for hour in -24..48 {
            let reads = crate::read_accounting::Capture::new();
            let ticks = capture.capture(&memory, 0, 0x1000, hour, false);
            let counts = reads.finish();
            let [Outcome::Unavailable { reason }] = capture.outcomes() else {
                panic!("unreadable neighbours must be unavailable at hour {hour}");
            };
            assert_eq!(ticks, [0]);
            assert_eq!(counts, crate::read_accounting::ZERO);
            assert_eq!(reason, replay::cadence::SKIPPED_REASON);
        }
        let reads = crate::read_accounting::Capture::new();
        capture.capture(&memory, 0, 0x1000, 0, true);
        assert!(reads.finish()[TABLES[0].index()].spans > 0);
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
