use crate::{
    clock::qpc,
    table_capture::{Arena, CaptureRows},
    table_conversion::{table, unavailable},
    world_input_changes::{self as changes, Changes, Domain, Held, Plan},
};
use engine_allocation::{copy_vec, owned_vec};
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::cprovince;
use replay_data::{rows::ProvinceNameRow, rows::TableId, rows::tables};
use replay_frame::frame::TableView;
use std::sync::Mutex;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::ProvinceNames];

type Names = (Vec<cprovince::NameRow>, Vec<u8>);

static HELD: Mutex<Held<Names>> = Mutex::new(Held::new((Vec::new(), Vec::new())));

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    bytes: usize,
}

collector_primitives::capacity_max!(Capacities { rows, bytes });

impl Capacities {
    pub fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let demand = cprovince::count_names(memory, base, state);
        let rows = collector_extended_support::demand::rows(demand.outcome)?;
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
    pub fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.names
            .fail(super::failed_capture(TABLES[0], hour, force_full, reason));
    }

    pub fn vacant() -> Self {
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
        self.capture_held(
            memory,
            base,
            game_state,
            hour,
            force_full,
            &changes::Observer,
            &HELD,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn capture_held(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        changes: &impl Changes,
        held: &Mutex<Held<Names>>,
    ) -> [u64; N] {
        let _reads = crate::read_accounting::table(TABLES[0]);
        self.reset();
        if !replay_data::cadence::should_walk(TABLES[0], hour, force_full) {
            self.names.outcome = Outcome::Unavailable {
                reason: replay_data::cadence::SKIPPED_REASON,
            };
            return [0];
        }
        let started = qpc();
        let mut held = held.try_lock().ok();
        let plan = held.as_ref().map_or(Plan::READ, |held| {
            held.plan(Domain::ProvinceNames, changes, force_full)
        });
        if plan.carry
            && let Some(held) = &held
        {
            let (rows, bytes) = held.carried(Domain::ProvinceNames);
            let ensured = (
                engine_native_memory::walk::Buffer::Growing(&mut self.names.rows).ensure(rows.len()),
                engine_native_memory::walk::Buffer::Growing(&mut self.bytes.first).ensure(bytes.len()),
            );
            self.names.outcome = match ensured {
                (Ok(_), Ok(_)) => {
                    self.names.rows[..rows.len()].copy_from_slice(rows);
                    self.bytes.first[..bytes.len()].copy_from_slice(bytes);
                    self.bytes.len = bytes.len();
                    self.bytes.required = bytes.len();
                    Outcome::Rows { len: rows.len() }
                }
                (Err(reason), _) | (_, Err(reason)) => Outcome::Unavailable { reason },
            };
            return [qpc().saturating_sub(started).max(0) as u64];
        }
        let first = cprovince::walk_names(
            memory,
            base,
            game_state,
            &mut self.names.rows,
            &mut self.bytes.first,
        );
        self.names.outcome = first.outcome;
        self.bytes.len = first.bytes_len;
        self.bytes.required = first.required_bytes;
        if let Some(held) = &mut held {
            let read = match first.outcome {
                Outcome::Rows { len } => Some((
                    &self.names.rows[..len],
                    &self.bytes.first[..first.bytes_len],
                )),
                _ => None,
            };
            held.settle(
                Domain::ProvinceNames,
                plan.read(Domain::ProvinceNames, changes),
                read.is_some(),
                |(rows, bytes)| {
                    read.is_some_and(|(fresh, text)| {
                        changes::assign(rows, fresh) && changes::assign(bytes, text)
                    })
                },
            );
        }
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
    use engine_native_memory::memory::{CountedMemory, SliceMemory};

    #[test]
    fn names_are_read_every_hour_and_preserve_native_failure() {
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
            assert!(reads.finish()[TABLES[0].index()].spans > 0, "hour {hour}");
            assert_ne!(reason, replay_data::cadence::SKIPPED_REASON);
            assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        }
    }

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

    fn census() -> Vec<u8> {
        let mut bytes = vec![0; 0x2000];
        let root = BASE + engine_native::cgame_state::PROVINCES;
        put(&mut bytes, root, &(POINTERS as u64).to_le_bytes());
        put(&mut bytes, root + 8, &2_u32.to_le_bytes());
        put(&mut bytes, root + 12, &2_u32.to_le_bytes());
        put(&mut bytes, POINTERS + 8, &(PROVINCE as u64).to_le_bytes());
        put(
            &mut bytes,
            PROVINCE,
            &(cprovince::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut bytes, PROVINCE + cprovince::ID, &1_u32.to_le_bytes());
        bytes
    }

    fn rows(expected: &[u8]) -> TableView {
        TableView::Complete(replay_data::rows::TableRows::ProvinceNames(
            if expected.is_empty() {
                vec![]
            } else {
                vec![ProvinceNameRow {
                    province: 1,
                    name: expected.to_vec(),
                }]
            },
        ))
    }

    #[test]
    fn live_override_changes_and_removal_are_captured_the_next_hour() {
        let mut bytes = census();
        let mut capture = Captured::with_capacities(&Capacities::default());
        for (hour, value) in [
            (1, &b"Original"[..]),
            (2, &b"Renamed"[..]),
            (3, &b"Renamed"[..]),
            (4, &b""[..]),
        ] {
            name(&mut bytes, value);
            let memory = SliceMemory {
                base: BASE,
                bytes: &bytes,
            };
            capture.capture(&memory, 0, BASE, hour, false);
            assert_eq!(capture.tables(), [rows(value)], "hour {hour}");
        }
    }

    #[test]
    fn quiet_hours_publish_the_held_read_without_reading() {
        let changes = changes::Isolated::new();
        let held = Mutex::new(Held::new((Vec::new(), Vec::new())));
        let mut bytes = census();
        let mut capture = Captured::with_capacities(&Capacities::default());
        let mut at = |bytes: &[u8], hour: i32, force_full: bool| {
            let memory = CountedMemory::new(SliceMemory { base: BASE, bytes });
            let reads = crate::read_accounting::Capture::new();
            capture.capture_held(&memory, 0, BASE, hour, force_full, &changes, &held);
            let spans = reads.finish()[TABLES[0].index()].spans;
            (capture.tables(), spans)
        };
        name(&mut bytes, b"Original");
        let (table, spans) = at(&bytes, 1, false);
        assert_eq!(table, [rows(b"Original")]);
        assert!(spans > 0);
        name(&mut bytes, b"Unseen");
        assert_eq!(at(&bytes, 2, false), ([rows(b"Original")], 0));
        let (table, spans) = at(&bytes, 3, true);
        assert_eq!(table, [rows(b"Unseen")]);
        assert!(spans > 0);
        name(&mut bytes, b"Renamed");
        changes.write(Domain::ProvinceNames);
        assert_eq!(at(&bytes, 4, false).0, [rows(b"Renamed")]);
        assert_eq!(at(&bytes, 5, false), ([rows(b"Renamed")], 0));
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
            replay_data::rows::TableRows::ProvinceNames(vec![ProvinceNameRow {
                province: 85,
                name: vec![b'A', 0, 255],
            }])
        );
        capture.names.rows[0].start = usize::MAX;
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
    }
}
