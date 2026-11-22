use engine_land_inputs::division_land_inputs as land;
use engine_native::{
    carmy, carmy::DivisionSeed, carmy_manpower_values, memory::SliceMemory, walk::Outcome,
};

struct DeniedAllocator;

thread_local! {
    static DENY_ALLOC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

// SAFETY: delegates to std::alloc::System except when the current test thread denies allocations.
unsafe impl std::alloc::GlobalAlloc for DeniedAllocator {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        if DENY_ALLOC.try_with(std::cell::Cell::get).unwrap_or(false) {
            std::ptr::null_mut()
        } else {
            // SAFETY: layout satisfies GlobalAlloc::alloc preconditions.
            unsafe { std::alloc::System.alloc(layout) }
        }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: std::alloc::Layout, size: usize) -> *mut u8 {
        if DENY_ALLOC.try_with(std::cell::Cell::get).unwrap_or(false) {
            std::ptr::null_mut()
        } else {
            // SAFETY: pointer, layout, and size satisfy GlobalAlloc::realloc preconditions.
            unsafe { std::alloc::System.realloc(pointer, layout, size) }
        }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: std::alloc::Layout) {
        // SAFETY: pointer and layout were allocated by std::alloc::System.
        unsafe { std::alloc::System.dealloc(pointer, layout) };
    }
}

#[global_allocator]
static ALLOCATOR: DeniedAllocator = DeniedAllocator;

fn with_allocations_denied<T>(run: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            DENY_ALLOC.set(self.0);
        }
    }
    let _reset = Reset(DENY_ALLOC.replace(true));
    run()
}

fn capture(
    memory: &impl engine_native::Memory,
    base: usize,
    sources: &[DivisionSeed],
    source: Outcome,
    rows: &mut Vec<land::Row>,
    contributors: &mut Vec<land::Contributor>,
) -> land::Walk {
    let mut pointers = vec![0; rows.len()];
    let mut ordinals = vec![0; rows.len()];
    let mut first = vec![[0; 2 * engine_deployment::manpower_value_entry::STRIDE]; rows.len()];
    land::capture(
        memory,
        base,
        sources,
        source,
        rows,
        contributors,
        land::Scratch {
            pointers: &mut pointers,
            ordinals: &mut ordinals,
            first: &mut first,
        },
    )
}

fn world(countries: usize) -> (Vec<u8>, Vec<DivisionSeed>) {
    let count = countries * 10;
    let mut bytes = vec![0; count * 0x700];
    let base = bytes.as_ptr() as usize;
    let mut seeds = Vec::with_capacity(count);
    for n in 0..count {
        let b = &mut bytes[n * 0x700..(n + 1) * 0x700];
        let pointer = base + n * 0x700;
        b[carmy::MANPOWER_VALUES..carmy::MANPOWER_VALUES + 8]
            .copy_from_slice(&(carmy_manpower_values::VTABLE_RVA.0 as u64).to_le_bytes());
        b[carmy::MANPOWER_VALUES + 8..carmy::MANPOWER_VALUES + 16]
            .copy_from_slice(&((pointer + 0x680) as u64).to_le_bytes());
        b[carmy::MANPOWER_VALUES + 16..carmy::MANPOWER_VALUES + 20]
            .copy_from_slice(&2u32.to_le_bytes());
        b[carmy::MANPOWER_VALUES + 20..carmy::MANPOWER_VALUES + 24]
            .copy_from_slice(&2u32.to_le_bytes());
        b[carmy::MAXIMUM_SUPPLY_GRACE..carmy::MAXIMUM_SUPPLY_GRACE + 8]
            .copy_from_slice(&300_000i64.to_le_bytes());
        b[carmy::REMAINING_SUPPLY_GRACE..carmy::REMAINING_SUPPLY_GRACE + 8]
            .copy_from_slice(&200_000i64.to_le_bytes());
        b[..8].copy_from_slice(&(carmy::VTABLE_RVA.0 as u64).to_le_bytes());
        b[0x18..0x20].copy_from_slice(&(((n as u64 + 1) << 32 | 4713).to_le_bytes()));
        b[0x40..0x48].copy_from_slice(&17001i64.to_le_bytes());
        b[carmy::NAVAL_TRANSFER_MEMBERSHIP..carmy::NAVAL_TRANSFER_MEMBERSHIP + 8]
            .copy_from_slice(&(u64::from(n % 2 == 0)).to_le_bytes());
        b[0x680..0x684].copy_from_slice(&((n / 10 + 1) as u32).to_le_bytes());
        b[0x684..0x688].copy_from_slice(&10_000i32.to_le_bytes());
        b[0x688..0x68c].copy_from_slice(&0x8000_0001u32.to_le_bytes());
        b[0x68c..0x690].copy_from_slice(&(-7i32).to_le_bytes());
        seeds.push(DivisionSeed {
            pointer,
            division: (n as u64 + 1) << 32 | 4713,
            handle: Some((n as u64 + 1) << 32 | 4713),
            primary_vtable: Some(carmy::VTABLE_RVA.0 as u64),
            counter_supply_status: Some(17_001),
            naval_transfer_membership: Some(usize::from(n % 2 == 0)),
            ..Default::default()
        });
    }
    carmy::capture_division_seeds(
        &SliceMemory {
            base,
            bytes: &bytes,
        },
        &mut seeds,
    );
    (bytes, seeds)
}
#[test]
fn ordered_contributors_signed_amounts_and_whole_list_capacity() {
    let (bytes, mut seeds) = world(1);
    let base = bytes.as_ptr() as usize;
    let memory = SliceMemory {
        base,
        bytes: &bytes,
    };
    let mut rows = vec![land::Row::default(); 10];
    let mut entries = vec![land::Contributor::default(); 20];
    let mut capture = |source: &[DivisionSeed], entries: &mut Vec<land::Contributor>| {
        capture(
            &memory,
            0,
            source,
            Outcome::Rows { len: source.len() },
            &mut rows,
            entries,
        )
    };
    let r = capture(&seeds, &mut entries);
    assert_eq!(r.outcome, Outcome::Rows { len: 10 });
    assert_eq!(r.contributors, 20);
    assert_eq!(entries[1].amount, -7);
    assert_eq!(entries[1].country, 0x8000_0001);
    seeds[0].counter_supply_status = None;
    seeds[1].supply_grace_maximum = None;
    seeds[2].supply_grace_remaining = None;
    let r = capture(&seeds, &mut entries);
    assert_eq!(r.outcome, Outcome::Rows { len: 10 });
    seeds[0].naval_transfer_membership = None;
    assert!(matches!(
        capture(&seeds, &mut entries).outcome,
        Outcome::Unavailable { .. }
    ));
}
#[test]
fn failed_headers_entries_and_identity_are_unavailable() {
    let (mut bytes, mut seeds) = world(1);
    let base = bytes.as_ptr() as usize;
    let mut rows = vec![land::Row::default(); 10];
    let mut entries = vec![land::Contributor::default(); 20];
    let memory = SliceMemory {
        base,
        bytes: &bytes,
    };
    for missing in 0..3 {
        let mut seed = seeds[0];
        match missing {
            0 => seed.manpower_values = None,
            1 => seed.naval_transfer_membership = None,
            2 => seed.handle = None,
            _ => unreachable!(),
        }
        assert!(matches!(
            capture(
                &memory,
                0,
                &[seed],
                Outcome::Rows { len: 1 },
                &mut rows,
                &mut entries
            )
            .outcome,
            Outcome::Unavailable { .. }
        ));
    }
    let memory = SliceMemory {
        base,
        bytes: &bytes[..0x68f],
    };
    assert!(matches!(
        capture(
            &memory,
            0,
            &seeds[..1],
            Outcome::Rows { len: 1 },
            &mut rows,
            &mut entries
        )
        .outcome,
        Outcome::Unavailable {
            reason: "division manpower entries unreadable"
        }
    ));
    seeds[0].manpower_values.as_mut().unwrap()[1] ^= 1;
    bytes[carmy::MANPOWER_VALUES + 1] ^= 1;
    let m = SliceMemory {
        base,
        bytes: &bytes,
    };
    assert!(matches!(
        capture(
            &m,
            0,
            &seeds,
            Outcome::Rows { len: 10 },
            &mut rows,
            &mut entries
        )
        .outcome,
        Outcome::Unavailable { .. }
    ));
}
#[test]
fn mixed_contributor_lists_grow_contributors_for_the_whole_census() {
    let (mut bytes, mut seeds) = world(1);
    let base = bytes.as_ptr() as usize;
    for (n, count) in [2u32, 3, 1].into_iter().enumerate() {
        let at = n * 0x700 + carmy::MANPOWER_VALUES;
        bytes[at + 16..at + 20].copy_from_slice(&count.to_le_bytes());
        bytes[at + 20..at + 24].copy_from_slice(&count.to_le_bytes());
    }
    bytes[0x700 + 0x690..0x700 + 0x694].copy_from_slice(&42u32.to_le_bytes());
    bytes[0x700 + 0x694..0x700 + 0x698].copy_from_slice(&123i32.to_le_bytes());
    bytes[2 * 0x700 + 0x680..2 * 0x700 + 0x684].copy_from_slice(&99u32.to_le_bytes());
    bytes[2 * 0x700 + 0x684..2 * 0x700 + 0x688].copy_from_slice(&(-5i32).to_le_bytes());
    carmy::capture_division_seeds(
        &SliceMemory {
            base,
            bytes: &bytes,
        },
        &mut seeds,
    );
    let mut rows = vec![land::Row::default(); 3];
    let mut contributors = vec![land::Contributor::default(); 3];
    let walk = capture(
        &SliceMemory {
            base,
            bytes: &bytes,
        },
        0,
        &seeds[..3],
        Outcome::Rows { len: 3 },
        &mut rows,
        &mut contributors,
    );
    assert_eq!(walk.outcome, Outcome::Rows { len: 3 });
    assert_eq!(walk.required_contributors, 6);
    assert_eq!(walk.contributors, 6);
    assert_eq!(rows[0].division, seeds[0].division);
    assert_eq!(
        [(rows[0].start, rows[0].len), (rows[1].start, rows[1].len), (rows[2].start, rows[2].len)],
        [(0, 2), (2, 3), (5, 1)]
    );
    assert_eq!(
        contributors[4],
        land::Contributor {
            country: 42,
            amount: 123
        }
    );
    assert_eq!(
        contributors[5],
        land::Contributor {
            country: 99,
            amount: -5
        }
    );
}

#[test]
fn contributor_growth_failure_precedes_later_seed_validation_failures() {
    let (bytes, mut seeds) = world(1);
    let base = bytes.as_ptr() as usize;
    seeds[1].primary_vtable = None;
    let memory = SliceMemory {
        base,
        bytes: &bytes,
    };
    let mut rows = vec![land::Row::default(); 2];
    let mut contributors = Vec::new();
    let mut pointers = vec![0; 2];
    let mut ordinals = vec![0; 2];
    let mut first = vec![[0; 2 * engine_deployment::manpower_value_entry::STRIDE]; 2];
    let walk = with_allocations_denied(|| {
        land::capture(
            &memory,
            0,
            &seeds[..2],
            Outcome::Rows { len: 2 },
            &mut rows,
            &mut contributors,
            land::Scratch {
                pointers: &mut pointers,
                ordinals: &mut ordinals,
                first: &mut first,
            },
        )
    });
    assert_eq!(
        walk.outcome,
        Outcome::Unavailable {
            reason: "capture buffer allocation failed"
        }
    );
}

#[test]
fn division_land_gather_uses_the_whole_census_without_fixed_batches() {
    use engine_native::{Memory, memory::RecordLayout, memory::RecordSources};
    struct Counted<'a> {
        memory: SliceMemory<'a>,
        gathers: std::cell::Cell<usize>,
    }
    impl Memory for Counted<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.memory.copy(address, output)
        }
        fn copy_records(
            &self,
            sources: RecordSources<'_>,
            layout: &RecordLayout<'_>,
            output: &mut [u8],
        ) -> usize {
            self.gathers.set(self.gathers.get() + 1);
            self.memory.copy_records(sources, layout, output)
        }
    }
    let (bytes, seeds) = world(7);
    let memory = Counted {
        memory: SliceMemory {
            base: bytes.as_ptr() as usize,
            bytes: &bytes,
        },
        gathers: std::cell::Cell::new(0),
    };
    let mut rows = vec![land::Row::default(); seeds.len()];
    let mut entries = vec![land::Contributor::default(); seeds.len() * 2];
    let walk = capture(
        &memory,
        0,
        &seeds,
        Outcome::Rows { len: seeds.len() },
        &mut rows,
        &mut entries,
    );
    assert_eq!(walk.outcome, Outcome::Rows { len: seeds.len() });
    assert_eq!(memory.gathers.get(), 1);
    assert_eq!(entries.last().unwrap().amount, -7);
}

#[test]
fn division_land_long_contributor_tail_is_exact_and_a_short_read_refuses_it() {
    let (_, mut seeds) = world(1);
    let count = 257;
    let mut payload = vec![0; count * engine_deployment::manpower_value_entry::STRIDE];
    for (n, bytes) in payload.chunks_exact_mut(8).enumerate() {
        bytes[..4].copy_from_slice(&(n as u32).to_le_bytes());
        bytes[4..].copy_from_slice(&(-(n as i32) - 1).to_le_bytes());
    }
    let header = seeds[0].manpower_values.as_mut().unwrap();
    header[8..16].copy_from_slice(&0x1000u64.to_le_bytes());
    header[16..20].copy_from_slice(&(count as u32).to_le_bytes());
    header[20..24].copy_from_slice(&(count as u32).to_le_bytes());
    let mut rows = vec![land::Row::default(); 1];
    let mut entries = vec![land::Contributor::default(); count];
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &payload,
    };
    let walk = capture(
        &memory,
        0,
        &seeds[..1],
        Outcome::Rows { len: 1 },
        &mut rows,
        &mut entries,
    );
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    assert_eq!(walk.contributors, count);
    for (n, entry) in entries.iter().enumerate() {
        assert_eq!((entry.country, entry.amount), (n as u32, -(n as i32) - 1));
    }
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &payload[..payload.len() - 1],
    };
    assert_eq!(
        capture(
            &memory,
            0,
            &seeds[..1],
            Outcome::Rows { len: 1 },
            &mut rows,
            &mut entries
        )
        .outcome,
        Outcome::Unavailable {
            reason: "division manpower entries unreadable"
        }
    );
}
