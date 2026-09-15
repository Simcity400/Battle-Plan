//! Hourly weather pass scheduling state, province staged pass operands and region re-roll inputs.
use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows, grow_capacity},
};
use engine::{Memory, cweather_manager, sweather_per_province, sweather_per_region, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{
        TableId, WeatherPassStateRow, WeatherProvincePassRow, WeatherRegionPassRow, tables,
    },
};

pub const N: usize = 3;
pub const TABLES: [TableId; N] = [
    TableId::WeatherPassState,
    TableId::WeatherProvincePasses,
    TableId::WeatherRegionPasses,
];
/// Manager authentication is charged once to the singleton for each acquisition pass.
pub const READ_GROUPS: &[(&str, usize, &[usize])] =
    &[("weather manager pass state and pass operands", 0, &[0, 1, 2])];

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
    pub fn with_capacities(capacity: &Capacities, successful: &[u16]) -> Self {
        assert_eq!(successful.len(), N);
        Self {
            state: CaptureRows::new(1, successful[0]),
            provinces: CaptureRows::new(capacity.provinces, successful[1]),
            regions: CaptureRows::new(capacity.regions, successful[2]),
            flags: Arena::new(capacity.flags, successful[2]),
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
    pub fn grow(&mut self, capacity: &mut Capacities, successful: &[u16]) {
        assert_eq!(successful.len(), N);
        let mut one = 1;
        self.state.grow(&mut one, successful[0]);
        self.provinces.grow(&mut capacity.provinces, successful[1]);
        self.regions.grow(&mut capacity.regions, successful[2]);
        grow_capacity(&mut capacity.flags, self.flags.required);
        self.flags.grow(&mut capacity.flags, successful[2]);
    }
    fn read(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        second: bool,
        ticks: &mut [u64; N],
    ) -> ([Outcome; N], usize, usize) {
        let begin = qpc();
        let reader = match cweather_manager::Reader::new(memory, base, state) {
            Ok(reader) => reader,
            Err(reason) => {
                ticks[0] += qpc().saturating_sub(begin).max(0) as u64;
                return ([Outcome::Unavailable { reason }; N], 0, 0);
            }
        };
        let state_rows = if second {
            &mut self.state.second
        } else {
            &mut self.state.rows
        };
        let pass_state = reader.pass_state(state_rows);
        let after_state = qpc();
        let province_rows = if second {
            &mut self.provinces.second
        } else {
            &mut self.provinces.rows
        };
        let provinces = reader.province_passes(province_rows);
        let after_provinces = qpc();
        let (region_rows, arena) = if second {
            (&mut self.regions.second, &mut self.flags.second)
        } else {
            (&mut self.regions.rows, &mut self.flags.first)
        };
        let regions = reader.region_passes(region_rows, arena);
        let end = qpc();
        ticks[0] += after_state.saturating_sub(begin).max(0) as u64;
        ticks[1] += after_provinces.saturating_sub(after_state).max(0) as u64;
        ticks[2] += end.saturating_sub(after_provinces).max(0) as u64;
        (
            [pass_state, provinces, regions.outcome],
            regions.len,
            regions.required,
        )
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        needed: &[bool],
    ) -> ([bool; N], [u64; N]) {
        assert_eq!(needed.len(), N);
        let mut ticks = [0; N];
        let ([pass_state, provinces, regions], flags_len, flags_required) =
            self.read(memory, base, state, false, &mut ticks);
        self.flags.len = flags_len;
        self.flags.required = flags_required;
        self.state.outcome = pass_state;
        self.provinces.outcome = provinces;
        self.regions.outcome = regions;
        self.state.checked = needed[0];
        self.provinces.checked = needed[1];
        self.regions.checked = needed[2];
        let mut matched = [false; N];
        if needed.iter().any(|v| *v) {
            let ([pass_state, provinces, regions], second_len, second_required) =
                self.read(memory, base, state, true, &mut ticks);
            if needed[0] {
                matched[0] = self.state.compare(pass_state);
            }
            if needed[1] {
                matched[1] = self.provinces.compare(provinces);
            }
            if needed[2] {
                let rows = self.regions.compare(regions);
                let arena = self.flags.compare(second_len, second_required);
                if !arena {
                    self.regions.mismatches += 1;
                }
                matched[2] = rows && arena;
            }
        }
        (matched, ticks)
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [
            checked(&self.state, "weather pass state changed during double read"),
            checked(
                &self.provinces,
                "weather province passes changed during double read",
            ),
            checked(
                &self.regions,
                "weather region passes changed during double read",
            ),
        ]
    }
    pub fn checked_mismatches(&self) -> [usize; N] {
        [
            self.state.mismatches,
            self.provinces.mismatches,
            self.regions.mismatches,
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
        if rows
            .iter()
            .any(|r| r.start.checked_add(r.count as usize).is_none_or(|end| end > flags.len()))
        {
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

fn checked<R>(rows: &CaptureRows<R>, reason: &'static str) -> Outcome {
    if rows.checked && rows.mismatches != 0 {
        Outcome::Unavailable { reason }
    } else {
        rows.outcome
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
        let mut capture = Captured::with_capacities(
            &Capacities {
                provinces: 1,
                regions: 1,
                flags: 2,
            },
            &[0; N],
        );
        assert_eq!(capture.capture(&memory, 0, 0, &[true; N]).0, [false; N]);
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
}
