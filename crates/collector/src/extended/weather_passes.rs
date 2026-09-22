//! Hourly province pass operands plus daily scheduling state and region re-roll inputs.
use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows, grow_capacity},
};
use engine::{Memory, cweather_manager, sweather_per_province, sweather_per_region, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{TableId, WeatherPassStateRow, WeatherProvincePassRow, WeatherRegionPassRow, tables},
};

pub const N: usize = 3;
pub const TABLES: [TableId; N] = [
    TableId::WeatherPassState,
    TableId::WeatherProvincePasses,
    TableId::WeatherRegionPasses,
];
/// The hourly province table owns shared manager authentication.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[(
    "weather manager pass state and pass operands",
    1,
    &[0, 1, 2],
)];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    provinces: usize,
    regions: usize,
    flags: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            provinces: 20_000,
            regions: 512,
            flags: 4096,
        }
    }
}

pub struct Captured {
    state: CaptureRows<cweather_manager::PassStateRow>,
    provinces: CaptureRows<sweather_per_province::PassRow>,
    regions: CaptureRows<sweather_per_region::PassRow>,
    flags: Arena<u32>,
}
impl Captured {
    pub(crate) fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.state.fail(super::failed_capture(TABLES[0], hour, force_full, reason));
        self.provinces.fail(super::failed_capture(TABLES[1], hour, force_full, reason));
        self.regions.fail(super::failed_capture(TABLES[2], hour, force_full, reason));
    }

    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self {
            state: CaptureRows::new(0),
            provinces: CaptureRows::new(0),
            regions: CaptureRows::new(0),
            flags: Arena::new(0),
        }
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            state: CaptureRows::new(1),
            provinces: CaptureRows::new(capacity.provinces),
            regions: CaptureRows::new(capacity.regions),
            flags: Arena::new(capacity.flags),
        }
    }
    pub fn reset(&mut self) {
        self.state.reset();
        self.provinces.reset();
        self.regions.reset();
        self.flags.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.state.allocated_bytes()
            + self.provinces.allocated_bytes()
            + self.regions.allocated_bytes()
            + self.flags.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        let mut one = 1;
        self.state.grow(&mut one);
        self.provinces.grow(&mut capacity.provinces);
        self.regions.grow(&mut capacity.regions);
        grow_capacity(&mut capacity.flags, self.flags.required);
        self.flags.grow(&mut capacity.flags);
    }
    fn read(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        active: [bool; N],
        ticks: &mut [u64; N],
    ) -> ([Outcome; N], usize, usize) {
        const SKIPPED: Outcome = Outcome::Unavailable {
            reason: replay::cadence::SKIPPED_REASON,
        };
        let mut outcomes = [SKIPPED; N];
        let mut reads = crate::read_accounting::table(TABLES[1]);
        let begin = qpc();
        let reader = match cweather_manager::Reader::new(memory, base, state) {
            Ok(reader) => reader,
            Err(reason) => {
                ticks[1] += qpc().saturating_sub(begin).max(0) as u64;
                return (
                    active.map(|due| {
                        if due {
                            Outcome::Unavailable { reason }
                        } else {
                            SKIPPED
                        }
                    }),
                    0,
                    0,
                );
            }
        };
        ticks[1] += qpc().saturating_sub(begin).max(0) as u64;
        if active[0] {
            reads.select(TABLES[0]);
            let begin = qpc();
            outcomes[0] = reader.pass_state(&mut self.state.rows);
            ticks[0] += qpc().saturating_sub(begin).max(0) as u64;
        }
        reads.select(TABLES[1]);
        let begin = qpc();
        outcomes[1] = reader.province_passes(&mut self.provinces.rows);
        ticks[1] += qpc().saturating_sub(begin).max(0) as u64;
        if active[2] {
            reads.select(TABLES[2]);
            let begin = qpc();
            let regions = reader.region_passes(&mut self.regions.rows, &mut self.flags.first);
            ticks[2] += qpc().saturating_sub(begin).max(0) as u64;
            outcomes[2] = regions.outcome;
            (outcomes, regions.len, regions.required)
        } else {
            (outcomes, 0, 0)
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
        let mut ticks = [0; N];
        let active = TABLES.map(|table| replay::cadence::should_walk(table, hour, force_full));
        let ([pass_state, provinces, regions], flags_len, flags_required) =
            self.read(memory, base, state, active, &mut ticks);
        self.flags.len = flags_len;
        self.flags.required = flags_required;
        self.state.outcome = pass_state;
        self.provinces.outcome = provinces;
        self.regions.outcome = regions;
        ticks
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.state.outcome,
            self.provinces.outcome,
            self.regions.outcome,
        ]
    }
    pub fn tables(&self) -> [TableView; N] {
        let [pass_state, provinces, regions] = self.outcomes();
        let state = table::<tables::WeatherPassState, _>(pass_state, &self.state.rows, |r| {
            WeatherPassStateRow {
                seed: r.seed,
                province_cursor: r.province_cursor,
                region_cursor: r.region_cursor,
                provinces_per_pass: r.provinces_per_pass,
                regions_per_pass: r.regions_per_pass,
                initial_passes: r.initial_passes,
                initial_pending: r.initial_pending,
                disabled: r.disabled,
                suspended: r.suspended,
            }
        });
        let provinces =
            table::<tables::WeatherProvincePasses, _>(provinces, &self.provinces.rows, |r| {
                WeatherProvincePassRow {
                    province: r.id,
                    eligible: r.eligible,
                    offset_key: r.offset_key,
                    permanent_snow: r.permanent_snow,
                    staged_temperature: r.staged_temperature,
                    staged_valid: r.staged_valid,
                    staged_change: r.staged_change,
                }
            });
        let Some(flags) = self.flags.first.get(..self.flags.len) else {
            return [
                state,
                provinces,
                unavailable("weather region flag arena length invalid"),
            ];
        };
        let rows = &self.regions.rows[..regions.len().min(self.regions.rows.len())];
        if rows.iter().any(|r| {
            r.start
                .checked_add(r.count as usize)
                .is_none_or(|end| end > flags.len())
        }) {
            return [
                state,
                provinces,
                unavailable("weather region flags lie outside captured arena"),
            ];
        }
        [
            state,
            provinces,
            table::<tables::WeatherRegionPasses, _>(regions, &self.regions.rows, |r| {
                WeatherRegionPassRow {
                    region: r.id,
                    eligible: r.eligible,
                    condition_flags: flags[r.start..r.start + r.count as usize].to_vec(),
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
    fn missing_manager_fails_every_table_and_region_flags_convert_exactly() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut capture = Captured::with_capacities(&Capacities {
            provinces: 1,
            regions: 1,
            flags: 2,
        });
        capture.capture(&memory, 0, 0, 0, true);
        assert!(
            capture
                .tables()
                .iter()
                .all(|t| matches!(t, TableView::Unavailable { .. }))
        );
        capture.reset();
        capture.state.rows[0].seed = u32::MAX;
        capture.state.outcome = Outcome::Rows { len: 1 };
        capture.provinces.outcome = Outcome::Rows { len: 0 };
        capture.regions.rows[0] = sweather_per_region::PassRow {
            id: 0,
            eligible: 1,
            data: 0x1000,
            count: 2,
            start: 0,
        };
        capture.flags.first[..2].copy_from_slice(&[1, 0xffff]);
        capture.flags.len = 2;
        capture.regions.outcome = Outcome::Rows { len: 1 };
        let [
            TableView::Complete(replay::rows::TableRows::WeatherPassState(state)),
            _,
            TableView::Complete(replay::rows::TableRows::WeatherRegionPasses(regions)),
        ] = capture.tables()
        else {
            panic!("complete");
        };
        assert_eq!(state[0].seed, u32::MAX);
        assert_eq!(regions[0].condition_flags, [1, 0xffff]);
        capture.regions.rows[0].count = 3;
        assert!(matches!(capture.tables()[2], TableView::Unavailable { .. }));
    }

    #[test]
    fn daily_private_weather_reads_do_not_change_hourly_manager_and_province_sources() {
        use engine::memory::{CountedMemory, ReadCounts};

        let mut bytes = vec![0; 0x10000];
        let state = 0x1000;
        let manager = 0x2000;
        bytes[state + engine::cgame_state::WEATHER_MANAGER
            ..state + engine::cgame_state::WEATHER_MANAGER + 8]
            .copy_from_slice(&(manager as u64).to_le_bytes());
        bytes[manager..manager + 8]
            .copy_from_slice(&(cweather_manager::VTABLE_RVA as u64).to_le_bytes());
        let mut capture = Captured::with_capacities(&Capacities {
            provinces: 0,
            regions: 0,
            flags: 0,
        });
        let allocated = capture.allocated_bytes();
        let mut daily_totals = crate::read_accounting::ZERO;
        for hour in -1..48 {
            let memory = CountedMemory::new(SliceMemory {
                base: 0,
                bytes: &bytes,
            });
            let scope = crate::read_accounting::Capture::new();
            let ticks = capture.capture(&memory, 0, state, hour, false);
            let counts = scope.finish();
            if (0..24).contains(&hour) {
                for (total, count) in daily_totals.iter_mut().zip(counts) {
                    *total += count;
                }
            }
            assert_eq!(
                counts[TABLES[1].index()],
                ReadCounts {
                    copy_calls: 3,
                    spans: 3,
                    bytes: 32,
                    ..ReadCounts::ZERO
                }
            );
            assert_eq!(capture.provinces.outcome, Outcome::Rows { len: 0 });
            for index in [0, 2] {
                let due = replay::cadence::should_walk(TABLES[index], hour, false);
                if due {
                    assert_eq!(
                        capture.outcomes()[index],
                        Outcome::Rows {
                            len: usize::from(index == 0)
                        }
                    );
                    assert_eq!(
                        counts[TABLES[index].index()],
                        if index == 0 {
                            ReadCounts {
                                scattered_calls: 1,
                                spans: 4,
                                bytes: 27,
                                ..ReadCounts::ZERO
                            }
                        } else {
                            ReadCounts {
                                copy_calls: 1,
                                spans: 1,
                                bytes: 16,
                                ..ReadCounts::ZERO
                            }
                        }
                    );
                } else {
                    assert_eq!(
                        capture.outcomes()[index],
                        Outcome::Unavailable {
                            reason: replay::cadence::SKIPPED_REASON
                        }
                    );
                    assert_eq!(counts[TABLES[index].index()], ReadCounts::ZERO);
                    assert_eq!(ticks[index], 0);
                }
            }
            assert_eq!(capture.allocated_bytes(), allocated);
        }
        let memory = CountedMemory::new(SliceMemory {
            base: 0,
            bytes: &bytes,
        });
        let vector = |counts: ReadCounts| {
            [
                counts.copy_calls,
                counts.scattered_calls,
                counts.record_calls,
                counts.plan_calls,
                counts.spans,
                counts.bytes,
            ]
        };
        assert_eq!(
            TABLES.map(|table| vector(daily_totals[table.index()])),
            [
                [0, 1, 0, 0, 4, 27],
                [72, 0, 0, 0, 72, 768],
                [1, 0, 0, 0, 1, 16]
            ]
        );
        let sum = |counts: crate::read_accounting::TableCounts| {
            let mut total = ReadCounts::ZERO;
            for count in counts {
                total += count;
            }
            vector(total)
        };
        let after = sum(daily_totals);
        assert_eq!(after, [73, 1, 0, 0, 77, 811]);

        let scope = crate::read_accounting::Capture::new();
        capture.capture(&memory, 0, state, 1, true);
        let counts = scope.finish();
        assert_eq!(counts[TABLES[0].index()].bytes, 27);
        assert_eq!(counts[TABLES[2].index()].bytes, 16);
        bytes[state + engine::cgame_state::WEATHER_MANAGER
            ..state + engine::cgame_state::WEATHER_MANAGER + 8]
            .fill(0);
        capture.capture(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            state,
            1,
            false,
        );
        assert_eq!(
            capture.state.outcome,
            Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON
            }
        );
        assert_eq!(
            capture.regions.outcome,
            Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON
            }
        );
        assert!(
            matches!(capture.provinces.outcome, Outcome::Unavailable { reason } if reason != replay::cadence::SKIPPED_REASON)
        );
    }
}
