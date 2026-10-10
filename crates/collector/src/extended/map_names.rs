//! Province override bytes. Native row/text storage grows only off-thread.
use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows},
};
use engine::allocation::{copy_vec, owned_vec};
use engine::{Memory, cprovince, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{ProvinceNameRow, TableId, tables},
};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::ProvinceNames];
pub const READ_GROUPS: &[(&str, usize, &[usize])] =
    &[("province name overrides and province census", 0, &[0])];

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    bytes: usize,
}

impl Capacities {
    pub(crate) fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        // Serialize@140E6CA90 retains each override's observed string length.
        let demand = cprovince::walk_names(memory, base, state, &mut [], &mut []);
        let rows = crate::country_scale::table_demand::rows(demand.outcome)?;
        crate::country_scale::raise(&mut self.rows, rows);
        crate::country_scale::raise(&mut self.bytes, demand.required_bytes);
        Ok(())
    }
}

pub struct Captured {
    names: CaptureRows<cprovince::NameRow>,
    bytes: Arena<u8>,
}
impl Captured {
    pub(crate) fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.names
            .fail(super::failed_capture(TABLES[0], hour, force_full, reason));
    }

    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities { rows: 0, bytes: 0 })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            names: CaptureRows::new(capacity.rows),
            bytes: Arena::new(capacity.bytes),
        }
    }
    pub fn reset(&mut self) {
        self.names.reset();
        self.bytes.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.names.allocated_bytes() + self.bytes.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.names.grow(&mut capacity.rows);
        self.bytes.grow(&mut capacity.bytes);
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        let _reads = crate::read_accounting::table(TABLES[0]);
        self.reset();
        if !replay::cadence::should_walk(TABLES[0], hour, force_full) {
            self.names.outcome = Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON,
            };
            return [0];
        }
        let started = qpc();
        let first = cprovince::walk_names_growing(
            memory,
            base,
            game_state,
            &mut self.names.rows,
            &mut self.bytes.first,
        );
        self.names.outcome = first.outcome;
        self.bytes.len = first.bytes_len;
        self.bytes.required = first.required_bytes;
        [qpc().saturating_sub(started).max(0) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.names.outcome]
    }
    pub fn tables(&self) -> [TableView; N] {
        let outcome = self.outcomes()[0];
        let Some(bytes) = self.bytes.first.get(..self.bytes.len) else {
            return [unavailable("province name arena length invalid")];
        };
        let Some(rows) = self.names.rows.get(..outcome.len()) else {
            return [unavailable("province name row count invalid")];
        };
        let text = |r: &cprovince::NameRow| {
            r.start
                .checked_add(r.len)
                .and_then(|end| bytes.get(r.start..end))
        };
        if rows.iter().any(|r| text(r).is_none()) {
            return [unavailable("province name lies outside captured arena")];
        }
        [table::<tables::ProvinceNames, _>(
            outcome,
            &self.names.rows,
            |r| ProvinceNameRow {
                province: r.province,
                name: owned_vec(copy_vec(text(r).unwrap())),
            },
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_job_hides_partial_names_without_losing_row_or_arena_growth() {
        for (outcome, required) in [
            (Outcome::Rows { len: 6 }, 6),
            (
                Outcome::Truncated {
                    len: 2,
                    unread_rows: 19,
                },
                19,
            ),
        ] {
            let mut actual_capacity = Capacities { rows: 8, bytes: 8 };
            let mut expected_capacity = actual_capacity;
            let mut actual = Captured::with_capacities(&actual_capacity);
            let mut expected = Captured::with_capacities(&expected_capacity);
            for captured in [&mut actual, &mut expected] {
                captured.names.outcome = outcome;
                captured.bytes.len = 6;
                captured.bytes.required = required;
                captured.bytes.first[0] = 77;
            }
            actual.fail_capture(1, true, "accepted job failed");
            actual.fail_capture(1, true, "accepted job failed");
            assert_eq!((actual.bytes.len, actual.bytes.required), (6, required));
            assert_eq!(actual.tables(), [unavailable("accepted job failed")]);
            actual.grow(&mut actual_capacity);
            expected.grow(&mut expected_capacity);
            assert!(actual_capacity == expected_capacity);
            assert_eq!(actual_capacity.bytes, if required == 6 { 16 } else { 32 });
            assert_eq!(actual.bytes.first[0], 77);
            actual.reset();
            let grown = actual_capacity;
            actual.grow(&mut actual_capacity);
            assert!(actual_capacity == grown);
        }
    }
    use engine::memory::{CountedMemory, SliceMemory};

    #[test]
    fn names_are_read_on_refresh_and_full_hours_and_preserve_native_failure() {
        let memory = CountedMemory::new(SliceMemory {
            base: 0,
            bytes: &[],
        });
        let mut capture = Captured::with_capacities(&Capacities::default());
        for (hour, force) in (-24..24).map(|hour| (hour, false)).chain([(0, true)]) {
            let reads = crate::read_accounting::Capture::new();
            capture.capture(&memory, 0, 0x1000, hour, force);
            let [Outcome::Unavailable { reason }] = capture.outcomes() else {
                panic!("unreadable names must fail at hour {hour}");
            };
            if !force && hour.rem_euclid(24) != 1 {
                assert_eq!(reads.finish(), crate::read_accounting::ZERO);
                assert_eq!(reason, replay::cadence::SKIPPED_REASON);
                continue;
            }
            assert!(reads.finish()[TABLES[0].index()].spans > 0);
            assert_ne!(reason, replay::cadence::SKIPPED_REASON);
            assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        }
    }

    #[test]
    fn live_override_changes_and_removal_are_captured_after_off_slot_hours() {
        const BASE: usize = 0x1000;
        const PROVINCE: usize = 0x2000;
        const POINTERS: usize = 0x1400;
        fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
            let start = address - BASE;
            bytes[start..start + value.len()].copy_from_slice(value);
        }
        fn name(bytes: &mut [u8], value: &[u8]) {
            assert!(value.len() < 16);
            let address = PROVINCE + cprovince::NAME_OVERRIDE;
            put(bytes, address, &[0; 16]);
            put(bytes, address, value);
            put(bytes, address + 16, &(value.len() as u64).to_le_bytes());
            put(bytes, address + 24, &15_u64.to_le_bytes());
        }
        let mut bytes = vec![0; 0x2000];
        let root = BASE + engine::cgame_state::PROVINCES;
        put(&mut bytes, root, &(POINTERS as u64).to_le_bytes());
        put(&mut bytes, root + 8, &2_u32.to_le_bytes());
        put(&mut bytes, root + 12, &2_u32.to_le_bytes());
        put(&mut bytes, POINTERS + 8, &(PROVINCE as u64).to_le_bytes());
        put(
            &mut bytes,
            PROVINCE,
            &(cprovince::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(&mut bytes, PROVINCE + cprovince::ID, &1_u32.to_le_bytes());
        let mut capture = Captured::with_capacities(&Capacities::default());
        for (hour, value, expected) in [
            (1, &b"Original"[..], Some(&b"Original"[..])),
            (2, &b"Renamed"[..], None),
            (25, &b"Renamed"[..], Some(&b"Renamed"[..])),
            (26, &b""[..], None),
            (49, &b""[..], Some(&b""[..])),
        ] {
            name(&mut bytes, value);
            capture.capture(
                &SliceMemory {
                    base: BASE,
                    bytes: &bytes,
                },
                0,
                BASE,
                hour,
                false,
            );
            let [table] = capture.tables();
            if let Some(expected) = expected {
                let expected_rows = if expected.is_empty() {
                    vec![]
                } else {
                    vec![ProvinceNameRow {
                        province: 1,
                        name: expected.to_vec(),
                    }]
                };
                assert_eq!(
                    table,
                    TableView::Complete(replay::rows::TableRows::ProvinceNames(expected_rows))
                );
            } else {
                assert_eq!(
                    table,
                    TableView::Unavailable {
                        reason: replay::cadence::SKIPPED_REASON.into()
                    }
                );
            }
        }
    }

    #[test]
    fn conversion_preserves_bytes_and_rejects_invalid_arena_ranges() {
        let mut capture = Captured::with_capacities(&Capacities { rows: 1, bytes: 3 });
        capture.names.rows[0] = cprovince::NameRow {
            province: 85,
            start: 0,
            len: 3,
        };
        capture.names.outcome = Outcome::Rows { len: 1 };
        capture.bytes.len = 3;
        capture.bytes.first[..3].copy_from_slice(&[b'A', 0, 255]);
        let [TableView::Complete(rows)] = capture.tables() else {
            panic!("complete");
        };
        assert_eq!(
            rows,
            replay::rows::TableRows::ProvinceNames(vec![ProvinceNameRow {
                province: 85,
                name: vec![b'A', 0, 255],
            }])
        );
        capture.names.rows[0].start = usize::MAX;
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
    }
}
