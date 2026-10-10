use super::*;
use engine::{cgame_state, cnational_focus, memory::SliceMemory};
use std::cell::RefCell;

const BASE: usize = 0x140000000;
const GS: usize = 0x10000;
const COUNTRY_TABLE: usize = 0x11000;

struct World {
    bytes: Vec<u8>,
    countries: usize,
    roots: usize,
    completed: usize,
    country_count: usize,
    completed_count: usize,
    token_len: usize,
}

impl World {
    fn new(country_count: usize, first_completed: usize, token_len: usize) -> Self {
        let country_stride = ccountry::FOCUS_PROGRESS + 8;
        let root_stride = cnational_focus_progress::PAUSED + 8;
        let links = COUNTRY_TABLE + (country_count + 1) * 8;
        let countries = (links + (country_count + 1) * 4 + 7) & !7;
        let roots = countries + country_count * country_stride;
        let completed = roots + country_count * root_stride;
        let completed_count = country_count - 1 + first_completed;
        let definition = completed + completed_count * 8;
        let text = definition + cnational_focus::TOKEN + 32;
        let mut world = Self {
            bytes: vec![0; text - GS + token_len + 1],
            countries,
            roots,
            completed,
            country_count,
            completed_count,
            token_len,
        };
        world.array(
            GS + cgame_state::COUNTRIES,
            COUNTRY_TABLE,
            country_count + 1,
        );
        world.array(GS + cgame_state::COUNTRY_LINKS, links, country_count + 1);
        let mut ordinal = 0;
        for index in 0..country_count {
            let pointer = countries + index * country_stride;
            let root = roots + index * root_stride;
            let count = if index == 0 { first_completed } else { 1 };
            world.u64(COUNTRY_TABLE + (index + 1) * 8, pointer as u64);
            world.u32(links + (index + 1) * 4, (index + 1) as u32);
            world.u64(pointer, (BASE + ccountry::VTABLE_RVA) as u64);
            world.u32(pointer + ccountry::ID, (index + 1) as u32);
            world.u64(pointer + ccountry::FOCUS_PROGRESS, root as u64);
            world.u64(root, (BASE + cnational_focus_progress::VTABLE_RVA) as u64);
            world.u64(root + cnational_focus_progress::PROGRESS, index as u64);
            world.array(
                root + cnational_focus_progress::COMPLETED_DATA,
                completed + ordinal * 8,
                count,
            );
            for _ in 0..count {
                world.u64(completed + ordinal * 8, definition as u64);
                ordinal += 1;
            }
        }
        assert_eq!(ordinal, completed_count);
        world.u64(definition + cnational_focus::TOKEN, text as u64);
        world.u64(definition + cnational_focus::TOKEN + 16, token_len as u64);
        world.u64(definition + cnational_focus::TOKEN + 24, token_len as u64);
        world.bytes[text - GS..text - GS + token_len].fill(b'x');
        world
    }

    fn memory(&self) -> SliceMemory<'_> {
        SliceMemory {
            base: GS,
            bytes: &self.bytes,
        }
    }

    fn u64(&mut self, address: usize, value: u64) {
        self.bytes[address - GS..address - GS + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn u32(&mut self, address: usize, value: u32) {
        self.bytes[address - GS..address - GS + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn array(&mut self, address: usize, data: usize, count: usize) {
        self.u64(address, data as u64);
        self.u32(address + 8, count as u32);
        self.u32(address + 12, count as u32);
    }

    fn warm_job(&self) -> FocusJob {
        FocusJob {
            rows: vec![Default::default(); self.country_count],
            completed: vec![Default::default(); self.completed_count],
            bytes: vec![0; self.completed_count * self.token_len],
            provisioned: true,
            ..Default::default()
        }
    }
}

struct Reads<'a> {
    memory: SliceMemory<'a>,
    fail: Option<(usize, usize)>,
    spans: RefCell<Vec<(usize, usize)>>,
}

impl Memory for Reads<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.spans.borrow_mut().push((address, output.len()));
        self.fail != Some((address, output.len())) && self.memory.copy(address, output)
    }
}

fn context() -> FocusCopyContext {
    FocusCopyContext {
        base: BASE,
        game_state: GS,
        active: true,
        rows: [0; 2],
        bytes: 0,
    }
}

fn assert_same_payload(cold: &FocusJob, warm: &FocusJob) {
    assert_eq!(cold.outcomes, warm.outcomes);
    assert_eq!(cold.bytes_len, warm.bytes_len);
    assert_eq!(cold.required_bytes, warm.required_bytes);
    assert_eq!(
        &cold.rows[..cold.outcome(0).len()],
        &warm.rows[..warm.outcome(0).len()]
    );
    assert_eq!(
        &cold.completed[..cold.outcome(1).len()],
        &warm.completed[..warm.outcome(1).len()]
    );
    assert_eq!(&cold.bytes[..cold.bytes_len], &warm.bytes[..warm.bytes_len]);
}

#[test]
fn default_pool_miss_focus_job_grows_5000_countries_and_later_original_headers() {
    let mut job = FocusJob::default();
    let mut captured = Captured::vacant();
    assert!(!job.is_provisioned());
    assert_eq!(job.capacities(), ([0; 2], 0));
    assert_eq!(captured.allocated_bytes(), 0);
    for (first_completed, token_len) in [(1, 29), (5_001, 37), (5_001, 513)] {
        let world = World::new(5_000, first_completed, token_len);
        let source = Reads {
            memory: world.memory(),
            fail: None,
            spans: RefCell::new(Vec::new()),
        };
        let reference = Reads {
            memory: world.memory(),
            fail: None,
            spans: RefCell::new(Vec::new()),
        };
        let previous_rows = job.completed.len();
        let previous_bytes = job.bytes.len();
        let report = copy_focus_refresh(&source, context(), &mut job);
        assert!(report.raw_ok);
        assert!(job.is_provisioned());
        assert_eq!(job.outcome(0), Outcome::Rows { len: 5_000 });
        assert_eq!(
            job.outcome(1),
            Outcome::Rows {
                len: world.completed_count
            }
        );
        assert_eq!(job.bytes_len, world.completed_count * token_len);
        if token_len == 37 {
            assert!(job.completed.len() > previous_rows);
        }
        if token_len == 513 {
            assert!(job.bytes.len() > previous_bytes);
        }
        let mut warm = world.warm_job();
        assert!(copy_focus_refresh(&reference, context(), &mut warm).raw_ok);
        assert_same_payload(&job, &warm);
        assert_eq!(*source.spans.borrow(), *reference.spans.borrow());
        for header in [
            GS + cgame_state::COUNTRIES,
            GS + cgame_state::COUNTRY_LINKS,
            world.roots + cnational_focus_progress::COMPLETED_DATA,
        ] {
            assert_eq!(
                source
                    .spans
                    .borrow()
                    .iter()
                    .filter(|&&span| span == (header, 16))
                    .count(),
                1
            );
        }
        let native_reads = source.spans.borrow().len();
        captured.decode_focus_payload(BASE, &job, &mut [0; 2]);
        assert_eq!(source.spans.borrow().len(), native_reads);
        assert_eq!(captured.focus.outcome, job.outcome(0));
        assert_eq!(captured.completed.outcome, job.outcome(1));
        assert_eq!(captured.focus_text.len, job.bytes_len);
        assert_eq!(
            &captured.focus_text.first[..job.bytes_len],
            &job.bytes[..job.bytes_len]
        );
        for (index, row) in captured.focus.rows[..5_000].iter().enumerate() {
            assert_eq!(row.country, index as u32 + 1);
            assert_eq!(row.progress, index as i64);
            assert_eq!(
                row.completed_count,
                if index == 0 {
                    first_completed as i32
                } else {
                    1
                }
            );
        }
        for row in &captured.completed.rows[..world.completed_count] {
            assert_eq!(
                &captured.focus_text.first[row.token.start..row.token.start + row.token.len],
                vec![b'x'; token_len]
            );
        }
    }
}

#[test]
fn default_focus_job_keeps_the_same_original_first_fault_as_warm_storage() {
    let world = World::new(5_000, 1, 29);
    for failure in [
        (GS + cgame_state::COUNTRIES, 16),
        (COUNTRY_TABLE + 8, 8),
        (world.countries, 8),
        (world.roots, 8),
        (world.roots + cnational_focus_progress::CURRENT, 16),
        (world.roots + cnational_focus_progress::COMPLETED_DATA, 16),
        (world.completed, 8),
    ] {
        let source = Reads {
            memory: world.memory(),
            fail: Some(failure),
            spans: RefCell::new(Vec::new()),
        };
        let reference = Reads {
            memory: world.memory(),
            fail: Some(failure),
            spans: RefCell::new(Vec::new()),
        };
        let mut cold = FocusJob::default();
        let mut warm = world.warm_job();
        assert!(copy_focus_refresh(&source, context(), &mut cold).raw_ok);
        assert!(copy_focus_refresh(&reference, context(), &mut warm).raw_ok);
        assert_same_payload(&cold, &warm);
        assert_eq!(
            *source.spans.borrow(),
            *reference.spans.borrow(),
            "failure={failure:?}"
        );
        assert_eq!(
            source
                .spans
                .borrow()
                .iter()
                .filter(|&&span| span == failure)
                .count(),
            1
        );
        let native_reads = source.spans.borrow().len();
        let mut captured = Captured::vacant();
        captured.decode_focus_payload(BASE, &cold, &mut [0; 2]);
        assert_eq!(source.spans.borrow().len(), native_reads);
        assert_eq!(captured.focus.outcome, cold.outcome(0));
        assert_eq!(captured.completed.outcome, cold.outcome(1));
        if failure == (world.roots + cnational_focus_progress::CURRENT, 16) {
            assert!(matches!(
                captured.focus.outcome,
                Outcome::Unavailable { .. }
            ));
            assert_eq!(captured.completed.outcome, Outcome::Rows { len: 5_000 });
        }
        if failure == (world.roots + cnational_focus_progress::COMPLETED_DATA, 16) {
            assert_eq!(captured.focus.outcome, Outcome::Rows { len: 5_000 });
            assert!(matches!(
                captured.completed.outcome,
                Outcome::Unavailable { .. }
            ));
        }
    }
}
