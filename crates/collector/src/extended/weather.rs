//! Native weather acquisition. Only growth and replay conversion allocate.

use crate::{sampling::qpc, spooling::table, table_capture::CaptureRows};
use engine::{Memory, cweather_manager, sweather_per_province, sweather_per_region, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{self, TableId, tables},
};

pub const N: usize = 3;
pub const TABLES: [TableId; N] = [
    TableId::WeatherLimits,
    TableId::WeatherRegions,
    TableId::WeatherProvinces,
];

/// Manager authentication is charged once to limits for each acquisition pass.
/// The two dense arrays retain their own read/comparison times and outcomes.
pub const READ_GROUPS: &[(&str, usize, &[usize])] =
    &[("weather manager and dense tables", 0, &[0, 1, 2])];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    pub rows: [usize; N],
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            // Keep the pinned map below CaptureRows' 75% growth threshold.
            rows: [2, 512, 20_000],
        }
    }
}

pub struct Captured {
    limits: CaptureRows<cweather_manager::Row>,
    regions: CaptureRows<sweather_per_region::Row>,
    provinces: CaptureRows<sweather_per_province::Row>,
}

impl Captured {
    pub fn with_capacities(capacity: &Capacities, successful: &[u16]) -> Self {
        Self {
            limits: CaptureRows::new(capacity.rows[0], successful[0]),
            regions: CaptureRows::new(capacity.rows[1], successful[1]),
            provinces: CaptureRows::new(capacity.rows[2], successful[2]),
        }
    }

    pub fn reset(&mut self) {
        self.limits.reset();
        self.regions.reset();
        self.provinces.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.limits.allocated_bytes()
            + self.regions.allocated_bytes()
            + self.provinces.allocated_bytes()
    }

    pub fn grow(&mut self, capacity: &mut Capacities, successful: &[u16]) {
        self.limits.grow(&mut capacity.rows[0], successful[0]);
        self.regions.grow(&mut capacity.rows[1], successful[1]);
        self.provinces.grow(&mut capacity.rows[2], successful[2]);
    }

    /// The manager is resolved afresh for the second pass. Every primary walk
    /// still runs after its own consistency window completes. No allocation,
    /// encoding or I/O occurs here; returned times are raw QPC ticks.
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        needed: &[bool],
    ) -> ([bool; N], [u64; N]) {
        self.reset();
        let mut matched = [false; N];
        let mut ticks = [0; N];
        let begin = qpc();
        let first = cweather_manager::Reader::new(memory, base, game_state);
        ticks[0] = elapsed(begin);
        let first = match first {
            Ok(reader) => reader,
            Err(reason) => {
                self.limits.outcome = Outcome::Unavailable { reason };
                self.regions.outcome = Outcome::Unavailable { reason };
                self.provinces.outcome = Outcome::Unavailable { reason };
                return (matched, ticks);
            }
        };
        macro_rules! primary {
            ($index:literal, $field:ident) => {{
                let begin = qpc();
                self.$field.outcome =
                    bounded(first.$field(&mut self.$field.rows), self.$field.rows.len());
                ticks[$index] = ticks[$index].saturating_add(elapsed(begin));
            }};
        }
        primary!(0, limits);
        primary!(1, regions);
        primary!(2, provinces);

        if needed[..N].iter().any(|needed| *needed) {
            let begin = qpc();
            let second = cweather_manager::Reader::new(memory, base, game_state);
            ticks[0] = ticks[0].saturating_add(elapsed(begin));
            macro_rules! check {
                ($index:literal, $field:ident) => {
                    if needed[$index] {
                        let begin = qpc();
                        self.$field.checked = true;
                        let outcome = match &second {
                            Ok(reader) => bounded(
                                reader.$field(&mut self.$field.second),
                                self.$field.second.len(),
                            ),
                            Err(reason) => Outcome::Unavailable { reason },
                        };
                        matched[$index] = self.$field.compare(outcome);
                        ticks[$index] = ticks[$index].saturating_add(elapsed(begin));
                    }
                };
            }
            check!(0, limits);
            check!(1, regions);
            check!(2, provinces);
        }
        (matched, ticks)
    }

    pub fn checked_mismatches(&self) -> [usize; N] {
        [
            self.limits.mismatches,
            self.regions.mismatches,
            self.provinces.mismatches,
        ]
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [
            effective(&self.limits),
            effective(&self.regions),
            effective(&self.provinces),
        ]
    }

    /// Spool-thread conversion; raw values and incomplete-table status survive.
    pub fn tables(&self) -> [TableView; N] {
        let outcomes = self.outcomes();
        [
            table::<tables::WeatherLimits, _>(outcomes[0], &self.limits.rows, |r| {
                rows::WeatherLimitsRow {
                    max_water: r.max_water,
                    max_snow: r.max_snow,
                }
            }),
            table::<tables::WeatherRegions, _>(outcomes[1], &self.regions.rows, |r| {
                rows::WeatherRegionRow {
                    id: r.id,
                    temperature: r.temperature,
                    next_change_hour: r.next_change_hour,
                    light_rain: r.light_rain,
                    heavy_rain: r.heavy_rain,
                    snow: r.snow,
                    blizzard: r.blizzard,
                    sandstorm: r.sandstorm,
                    arctic_water: r.arctic_water,
                }
            }),
            table::<tables::WeatherProvinces, _>(outcomes[2], &self.provinces.rows, |r| {
                rows::WeatherProvinceRow {
                    id: r.id,
                    water: r.water,
                    snow: r.snow,
                    temperature: r.temperature,
                    temperature_change: r.temperature_change,
                    mud: r.mud,
                    custom_mud: r.custom_mud,
                    custom_modifier_count: r.custom_modifier_count,
                }
            }),
        ]
    }
}

fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

fn bounded(outcome: Outcome, capacity: usize) -> Outcome {
    if outcome.len() > capacity {
        Outcome::Unavailable {
            reason: "walker row count exceeds allocated buffer",
        }
    } else if matches!(outcome, Outcome::Truncated { unread_rows: 0, .. }) {
        Outcome::Unavailable {
            reason: "walker marked truncation without missing rows",
        }
    } else {
        outcome
    }
}

fn effective<R>(captured: &CaptureRows<R>) -> Outcome {
    if captured.checked && captured.mismatches != 0 {
        Outcome::Unavailable {
            reason: "table changed during double read",
        }
    } else {
        captured.outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sampling::CONSISTENCY_TARGET;
    use engine::{cgame_state, memory::SliceMemory};
    use replay::rows::TableSchema;
    use std::cell::Cell;

    fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
        bytes[address - 0x1000..address - 0x1000 + value.len()].copy_from_slice(value);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x4000];
        put(
            &mut bytes,
            0x1000 + cgame_state::WEATHER_MANAGER,
            &0x2000_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x2000,
            &(cweather_manager::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x2000 + cweather_manager::MAX_WATER,
            &i64::MAX.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x2000 + cweather_manager::MAX_SNOW,
            &(-1_i64).to_le_bytes(),
        );
        for (offset, address, vtable) in [
            (
                cweather_manager::REGIONS,
                0x3000_u64,
                sweather_per_region::VTABLE_RVA,
            ),
            (
                cweather_manager::PROVINCES,
                0x4000_u64,
                sweather_per_province::VTABLE_RVA,
            ),
        ] {
            put(&mut bytes, 0x2000 + offset, &address.to_le_bytes());
            put(&mut bytes, 0x2000 + offset + 8, &3_u32.to_le_bytes());
            put(&mut bytes, 0x2000 + offset + 12, &1_u32.to_le_bytes());
            put(&mut bytes, address as usize, &(vtable as u64).to_le_bytes());
        }
        put(
            &mut bytes,
            0x3000 + sweather_per_region::TEMPERATURE,
            &i64::MIN.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x3000 + sweather_per_region::NEXT_CHANGE_HOUR,
            &i32::MIN.to_le_bytes(),
        );
        put(&mut bytes, 0x3000 + sweather_per_region::SNOW, &[255]);
        put(
            &mut bytes,
            0x4000 + sweather_per_province::WATER,
            &((1_i64 << 40) + 3).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x4000 + sweather_per_province::TEMPERATURE_CHANGE,
            &(-(1_i64 << 41) - 7).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x4000 + sweather_per_province::CUSTOM_MUD,
            &[254],
        );
        bytes
    }

    fn small() -> (Capacities, Captured) {
        let capacities = Capacities { rows: [2; N] };
        let captured = Captured::with_capacities(&capacities, &[0; N]);
        (capacities, captured)
    }

    fn complete<S: TableSchema>(view: &TableView) -> &[S::Row] {
        let TableView::Complete(rows) = view else {
            panic!("expected complete table")
        };
        rows.rows::<S>().unwrap()
    }

    #[test]
    fn acquisition_and_conversion_keep_dense_zero_native_words_and_raw_flags() {
        let (_, mut captured) = small();
        let bytes = fixture();
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let (matched, _) = captured.capture(&memory, 0, 0x1000, &[true; N]);
        assert_eq!(matched, [true; N]);
        assert_eq!(captured.checked_mismatches(), [0; N]);
        let tables = captured.tables();
        assert_eq!(
            complete::<tables::WeatherLimits>(&tables[0]),
            &[rows::WeatherLimitsRow {
                max_water: i64::MAX,
                max_snow: -1
            }]
        );
        let region = &complete::<tables::WeatherRegions>(&tables[1])[0];
        assert_eq!(
            (
                region.id,
                region.temperature,
                region.next_change_hour,
                region.snow
            ),
            (0, i64::MIN, i32::MIN, 255)
        );
        let province = &complete::<tables::WeatherProvinces>(&tables[2])[0];
        assert_eq!(
            (
                province.id,
                province.temperature,
                province.water,
                province.temperature_change,
                province.custom_mud
            ),
            (0, 0, (1_i64 << 40) + 3, -(1_i64 << 41) - 7, 254)
        );
    }

    #[test]
    fn table_faults_and_truncation_remain_independent_and_do_not_earn_checks() {
        let (mut capacities, _) = small();
        capacities.rows[2] = 0;
        let mut captured = Captured::with_capacities(&capacities, &[0; N]);
        let mut bytes = fixture();
        put(
            &mut bytes,
            0x2000 + cweather_manager::REGIONS_COUNT,
            &(-1_i32).to_le_bytes(),
        );
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let (matched, _) = captured.capture(&memory, 0, 0x1000, &[true; N]);
        assert_eq!(matched, [true, false, false]);
        assert!(matches!(
            captured.outcomes()[1],
            Outcome::Unavailable { .. }
        ));
        assert_eq!(
            captured.outcomes()[2],
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
        let tables = captured.tables();
        assert!(matches!(tables[1], TableView::Unavailable { .. }));
        assert!(matches!(tables[2], TableView::Truncated { missing: 1, .. }));
        captured.grow(&mut capacities, &[0; N]);
        assert!(!captured.provinces.rows.is_empty());
        put(
            &mut bytes,
            0x1000 + cgame_state::WEATHER_MANAGER,
            &0_u64.to_le_bytes(),
        );
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(
            captured.capture(&memory, 0, 0x1000, &[true; N]).0,
            [false; N]
        );
        assert!(
            captured
                .outcomes()
                .into_iter()
                .all(|outcome| matches!(outcome, Outcome::Unavailable { .. }))
        );
    }

    struct ChangingWater<'a> {
        bytes: &'a [u8],
        copies: Cell<usize>,
    }
    impl Memory for ChangingWater<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if !(SliceMemory {
                base: 0x1000,
                bytes: self.bytes,
            })
            .copy(address, output)
            {
                return false;
            }
            if address == 0x4000 + sweather_per_province::WATER {
                let copies = self.copies.get();
                self.copies.set(copies + 1);
                if copies != 0 {
                    output[0] ^= 1;
                }
            }
            true
        }
    }

    #[test]
    fn changed_table_is_unavailable_and_finished_windows_release_independently() {
        let (mut capacities, mut captured) = small();
        let bytes = fixture();
        let memory = ChangingWater {
            bytes: &bytes,
            copies: Cell::new(0),
        };
        assert_eq!(
            captured.capture(&memory, 0, 0x1000, &[true; N]).0,
            [true, true, false]
        );
        assert_eq!(captured.checked_mismatches(), [0, 0, 1]);
        assert!(matches!(
            captured.tables()[2],
            TableView::Unavailable { .. }
        ));
        let before = captured.allocated_bytes();
        captured.grow(
            &mut capacities,
            &[CONSISTENCY_TARGET, 0, CONSISTENCY_TARGET],
        );
        assert!(captured.limits.second.is_empty() && captured.provinces.second.is_empty());
        assert!(!captured.regions.second.is_empty());
        assert!(captured.allocated_bytes() < before);
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(
            captured
                .capture(&memory, 0, 0x1000, &[false, true, false])
                .0,
            [false, true, false]
        );
        assert_eq!(captured.checked_mismatches(), [0; N]);
        assert_eq!(captured.outcomes(), [Outcome::Rows { len: 1 }; N]);
    }
}
