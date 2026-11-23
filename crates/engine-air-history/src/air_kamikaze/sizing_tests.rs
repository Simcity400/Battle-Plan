use super::*;
use engine_native_memory::memory::SliceMemory;
use std::cell::RefCell;

const BASE: usize = 0x1_0000_0000;

struct Fixture {
    bytes: Vec<u8>,
    histories: Vec<usize>,
    windows: Vec<usize>,
    capacities: Vec<usize>,
    records: Vec<usize>,
    values: Vec<usize>,
}

impl Fixture {
    fn allocate(&mut self, len: usize) -> usize {
        let at = self.bytes.len().next_multiple_of(8);
        self.bytes.resize(at + len, 0);
        at
    }

    fn qword(&mut self, at: usize, value: u64) {
        self.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn dword(&mut self, at: usize, value: u32) {
        self.bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn new(capacities: Vec<usize>) -> Self {
        let mut fixture = Self {
            bytes: vec![0; 8],
            histories: Vec::new(),
            windows: Vec::new(),
            capacities,
            records: Vec::new(),
            values: Vec::new(),
        };
        for record in 0..*fixture.capacities.iter().max().unwrap_or(&0) {
            let object = fixture.allocate(8);
            let values = fixture.allocate(cloop_history::air_stats::FIELD_COUNT as usize * 8);
            fixture.qword(object, values as u64);
            for (channel, &field) in CHANNELS.iter().enumerate() {
                fixture.qword(values + field * 8, value(record, channel) as u64);
            }
            fixture.records.push(object);
            fixture.values.push(values);
        }
        for index in 0..fixture.capacities.len() {
            let capacity = fixture.capacities[index];
            let history = fixture.allocate(0x40);
            let window = fixture.allocate(0x40);
            let records = fixture.allocate(capacity * 8);
            fixture.qword(history, (BASE + cloop_history::VTABLE_RVA.0) as u64);
            fixture.dword(
                history + cloop_history::FIELD_COUNT,
                cloop_history::air_stats::FIELD_COUNT,
            );
            fixture.qword(history + FIRST_WINDOW, window as u64);
            fixture.qword(window, (BASE + loop_history_window::VTABLE_RVA.0) as u64);
            fixture.qword(window + loop_history_window::RECORDS, records as u64);
            fixture.dword(window + loop_history_window::RECORDS + 8, capacity as u32);
            fixture.dword(window + loop_history_window::RECORDS + 12, capacity as u32);
            fixture.dword(window + loop_history_window::CAPACITY, capacity as u32);
            fixture.dword(
                window + loop_history_window::CURRENT_SLOT,
                capacity as u32 - 1,
            );
            fixture.bytes[window + loop_history_window::WRAPPED] = 1;
            for record in 0..capacity {
                fixture.qword(records + record * 8, fixture.records[record] as u64);
            }
            fixture.histories.push(history);
            fixture.windows.push(window);
        }
        fixture
    }

    fn batch_records(&self) -> usize {
        self.capacities
            .chunks(WINDOW_BATCH)
            .map(|chunk| chunk.iter().sum())
            .max()
            .unwrap_or(0)
    }
}

fn value(record: usize, channel: usize) -> i64 {
    record as i64 * 100 + channel as i64 - 3
}

struct Trace<'a> {
    bytes: &'a [u8],
    calls: RefCell<Vec<(usize, usize)>>,
    fault: Option<usize>,
    headers_only: bool,
}

impl<'a> Trace<'a> {
    fn new(bytes: &'a [u8], fault: Option<usize>) -> Self {
        Self {
            bytes,
            calls: RefCell::default(),
            fault,
            headers_only: false,
        }
    }
}

impl Memory for Trace<'_> {
    fn copy(&self, at: usize, output: &mut [u8]) -> bool {
        if !self.headers_only || output.len() == WINDOW_FIELDS[2].len {
            self.calls.borrow_mut().push((at, output.len()));
        }
        if self
            .fault
            .is_some_and(|fault| at <= fault && fault < at.saturating_add(output.len()))
        {
            if let Some(first) = output.first_mut() {
                *first = 0xee;
            }
            return false;
        }
        SliceMemory {
            base: 0,
            bytes: self.bytes,
        }
        .copy(at, output)
    }
}

#[test]
fn cold_and_presized_windows_keep_fixed_read_order_and_each_first_fault() {
    let fixture = Fixture::new(vec![65, 129, 97]);
    let faults = [
        (None, None),
        (Some(fixture.histories[0]), Some("air history unreadable")),
        (
            Some(fixture.windows[0] + loop_history_window::CAPACITY),
            Some("air history window unreadable"),
        ),
        (
            Some(fixture.windows[1] + loop_history_window::RECORDS),
            Some("air history window unreadable"),
        ),
        (
            Some(fixture.records[64]),
            Some("air history record unreadable"),
        ),
        (Some(fixture.values[96] + CHANNELS[8] * 8), None),
    ];
    for (fault, failure) in faults {
        let mut prepared = WindowScratch::new();
        prepared.size(fixture.batch_records()).unwrap();
        let mut cold = WindowScratch::new();
        let first = Trace::new(&fixture.bytes, fault);
        let second = Trace::new(&fixture.bytes, fault);
        let mut current = Vec::new();
        let result = history_windows(
            &first,
            BASE,
            &fixture.histories,
            &mut cold,
            |at, state, rows, failures| {
                current.push((at, state, rows.to_vec(), failures.to_vec()));
                Ok(())
            },
        );
        let mut expected = Vec::new();
        let baseline = history_windows(
            &second,
            BASE,
            &fixture.histories,
            &mut prepared,
            |at, state, rows, failures| {
                expected.push((at, state, rows.to_vec(), failures.to_vec()));
                Ok(())
            },
        );
        assert_eq!(result, baseline);
        assert_eq!(result.err(), failure);
        assert_eq!(current, expected);
        assert_eq!(*first.calls.borrow(), *second.calls.borrow());
        if failure.is_some() {
            assert!(result.is_err());
            assert!(
                current.is_empty(),
                "the failing original batch published a prefix"
            );
        } else {
            assert_eq!(result, Ok(()));
            assert_eq!(current.len(), 3);
            for (_, _, rows, failures) in &current {
                for (record, (channels, refusal)) in rows.iter().zip(failures).enumerate() {
                    if fault.is_some() && record == 96 {
                        assert_eq!(*refusal, Some(SCENARIO_REFUSALS[6]));
                        assert_eq!(&channels[..2], &[value(record, 0), value(record, 1)]);
                        assert_eq!(&channels[2..], &[0; 7]);
                    } else {
                        assert_eq!(*refusal, None);
                        assert_eq!(
                            *channels,
                            std::array::from_fn(|channel| value(record, channel))
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn signed_window_bounds_still_fail_before_record_reads() {
    for (capacity, current) in [
        (0u32, 0u32),
        (u32::MAX, 0),
        (66, 0),
        (65, 65),
        (65, u32::MAX),
    ] {
        let mut fixture = Fixture::new(vec![65]);
        fixture.dword(fixture.windows[0] + loop_history_window::CAPACITY, capacity);
        fixture.dword(
            fixture.windows[0] + loop_history_window::CURRENT_SLOT,
            current,
        );
        let memory = Trace::new(&fixture.bytes, None);
        let mut scratch = WindowScratch::new();
        assert_eq!(
            history_windows(
                &memory,
                BASE,
                &fixture.histories,
                &mut scratch,
                |_, _, _, _| { panic!("invalid original window reached publication") }
            ),
            Err("air history window invalid")
        );
        assert!(scratch.records.is_empty() && scratch.channels.is_empty());
        assert!(
            !memory
                .calls
                .borrow()
                .iter()
                .any(|&(at, _)| fixture.records.contains(&at))
        );
    }
}
