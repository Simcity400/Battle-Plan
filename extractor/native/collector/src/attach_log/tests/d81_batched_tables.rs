use super::*;
use std::ffi::c_void;
use std::time::Instant;

#[link(name = "kernel32")]
extern "system" {
    fn VirtualAlloc(
        address: *mut c_void,
        size: usize,
        alloc_type: u32,
        protect: u32,
    ) -> *mut c_void;
    fn VirtualFree(address: *mut c_void, size: usize, free_type: u32) -> i32;
    fn VirtualProtect(
        address: *mut c_void,
        size: usize,
        new_protect: u32,
        old_protect: *mut u32,
    ) -> i32;
}

const FZ_MEM_RESERVE: u32 = 0x2000;
const FZ_MEM_COMMIT: u32 = 0x1000;
const FZ_MEM_RELEASE: u32 = 0x8000;
const FZ_PAGE_READWRITE: u32 = 0x04;
const FZ_PAGE_NOACCESS: u32 = 0x01;

/// One `VirtualAlloc` arena, released on drop.
struct Arena {
    base: usize,
    size: usize,
}

impl Arena {
    fn new(size: usize) -> Arena {
        let base = unsafe {
            VirtualAlloc(
                std::ptr::null_mut(),
                size,
                FZ_MEM_RESERVE | FZ_MEM_COMMIT,
                FZ_PAGE_READWRITE,
            )
        };
        assert!(!base.is_null(), "fixture arena VirtualAlloc failed");
        let arena = Arena {
            base: base as usize,
            size,
        };
        page_cache_new_frame();
        arena
    }

    fn fill_pattern(&self, seed: u64) {
        let mut state = seed | 1;
        for offset in (0..self.size).step_by(8) {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            self.put_u64(self.base + offset, z ^ (z >> 31));
        }
    }

    fn put_u64(&self, address: usize, value: u64) {
        assert!(address >= self.base && address + 8 <= self.base + self.size);
        unsafe { std::ptr::write_unaligned(address as *mut u64, value) };
    }

    fn put_u32(&self, address: usize, value: u32) {
        assert!(address >= self.base && address + 4 <= self.base + self.size);
        unsafe { std::ptr::write_unaligned(address as *mut u32, value) };
    }

    fn put_u8(&self, address: usize, value: u8) {
        assert!(address >= self.base && address < self.base + self.size);
        unsafe { std::ptr::write_unaligned(address as *mut u8, value) };
    }

    fn deny_page(&self, page: usize) {
        let mut previous = 0u32;
        let ok = unsafe {
            VirtualProtect(
                (self.base + page * RPM_PAGE_SIZE) as *mut c_void,
                RPM_PAGE_SIZE,
                FZ_PAGE_NOACCESS,
                &mut previous,
            )
        };
        assert_ne!(ok, 0, "VirtualProtect must split the fixture arena");
        page_cache_new_frame();
    }
}

impl Drop for Arena {
    fn drop(&mut self) {
        unsafe { VirtualFree(self.base as *mut c_void, 0, FZ_MEM_RELEASE) };
        page_cache_new_frame();
    }
}

const FIXTURE_SELECTOR_MASK: u32 = 4_095;
const FIXTURE_SELECTOR_MAX_DISTANCE: u8 = 8;

const SELECTOR_CONSUMED: [(usize, usize); 3] = D81_SELECTOR_CONSUMED_FIELDS;

#[test]
fn the_batched_rail_slots_answer_what_the_live_stable_reads_answer() {
    const PAGES: usize = 16;
    let arena = Arena::new(PAGES * RPM_PAGE_SIZE);
    arena.fill_pattern(0xD81F_2000);

    let rail_table = arena.base + 0x40;
    let slots = (arena.size - 0x40) / 8;
    let ids: Vec<u32> = (0..slots as u32)
        .filter(|id| id % 7 == 3 || id % 13 == 1)
        .collect();
    assert!(ids.len() > 100, "the fixture must exercise a real id set");

    let (base, len) = d81_rail_slot_extent(rail_table, ids.iter().copied())
        .expect("the extent is formable for an in-arena table");
    let pair = D81ExtentPair::copy(base, len).expect("a fully readable extent copies whole");

    for id in &ids {
        let slot = rail_table + (*id as usize) * 8;
        let live = read_u64_stable(slot);
        assert!(live.is_some(), "the fixture slot must be live-readable");
        assert_eq!(
            pair.u64_at(slot),
            live,
            "the batched image must answer the live stable read at id {id}"
        );
    }
}

#[test]
fn the_batched_selector_fields_answer_what_the_live_stable_reads_answer() {
    let slots = FIXTURE_SELECTOR_MASK as usize + FIXTURE_SELECTOR_MAX_DISTANCE as usize + 2;
    let arena = Arena::new(
        ((slots * D81_SELECTOR_ENTRY_STRIDE) + 2 * RPM_PAGE_SIZE) / RPM_PAGE_SIZE * RPM_PAGE_SIZE,
    );
    arena.fill_pattern(0xD81F_3000);

    let table = arena.base + D81_SELECTOR_ENTRY_STRIDE;
    let (base, len) =
        d81_selector_extent(table, FIXTURE_SELECTOR_MASK, FIXTURE_SELECTOR_MAX_DISTANCE)
            .expect("the extent is formable");
    assert_eq!(
        base, table,
        "the selector extent starts at the table itself"
    );
    assert_eq!(
        len,
        slots * D81_SELECTOR_ENTRY_STRIDE,
        "the extent is mask + max_distance + 2 buckets"
    );
    let pair = D81ExtentPair::copy(base, len).expect("a fully readable extent copies whole");

    for slot in 0..slots {
        let entry = table + slot * D81_SELECTOR_ENTRY_STRIDE;
        assert_eq!(
            pair.u8_at(entry + 4),
            read_u8_stable(entry + 4),
            "marker at bucket {slot}"
        );
        assert_eq!(
            pair.u32_at(entry + 8),
            read_u32_stable(entry + 8),
            "key at bucket {slot}"
        );
        assert_eq!(
            pair.u64_at(entry + 0x10),
            read_u64_stable(entry + 0x10),
            "value at bucket {slot}"
        );
        assert!(
            pair.u8_at(entry + 4).is_some(),
            "a readable bucket must be SERVED from the image, not merely agree \
             with a live read that also ran"
        );
    }
}

#[test]
fn an_unreadable_page_inside_the_extent_costs_only_the_fields_on_that_page() {
    const PAGES: usize = 8;
    const DENIED: usize = 3;
    let arena = Arena::new(PAGES * RPM_PAGE_SIZE);
    arena.fill_pattern(0xD81F_4000);

    let rail_table = arena.base;
    let ids: Vec<u32> = (0..(arena.size / 8) as u32).collect();
    let (base, len) =
        d81_rail_slot_extent(rail_table, ids.iter().copied()).expect("the extent is formable");
    assert_eq!(base, rail_table);
    assert_eq!(len, arena.size);

    arena.deny_page(DENIED);

    let pair = D81ExtentPair::copy(base, len)
        .expect("the page retry must still produce a pair when only one page is denied");

    let mut served = 0usize;
    let mut declined = 0usize;
    for id in &ids {
        let slot = rail_table + (*id as usize) * 8;
        let on_denied_page = (slot - arena.base) / RPM_PAGE_SIZE == DENIED;
        match pair.u64_at(slot) {
            Some(value) => {
                served += 1;
                assert!(
                    !on_denied_page,
                    "no field on the denied page can be served from the image"
                );
                assert_eq!(
                    Some(value),
                    read_u64_stable(slot),
                    "a served field is still the live read's value"
                );
            }
            None => {
                declined += 1;
                assert!(
                    on_denied_page,
                    "slot {id} is on a readable page and must still be served"
                );
                assert_eq!(
                    read_u64_stable(slot),
                    None,
                    "the live fallback refuses this slot exactly as it does today"
                );
            }
        }
    }
    assert_eq!(
        declined,
        RPM_PAGE_SIZE / 8,
        "exactly the denied page's slots lose the fast path"
    );
    assert_eq!(
        served,
        ids.len() - RPM_PAGE_SIZE / 8,
        "every other slot in the extent still publishes from the image"
    );
}

#[test]
fn an_unreadable_page_in_the_over_read_costs_nothing() {
    const PAGES: usize = 8;
    const DENIED: usize = 3;
    let arena = Arena::new(PAGES * RPM_PAGE_SIZE);
    arena.fill_pattern(0xD81F_4400);

    let rail_table = arena.base;
    let slots_per_page = RPM_PAGE_SIZE / 8;
    let ids: Vec<u32> = (0..(arena.size / 8) as u32)
        .filter(|id| (*id as usize) / slots_per_page != DENIED)
        .collect();
    let (base, len) =
        d81_rail_slot_extent(rail_table, ids.iter().copied()).expect("the extent is formable");
    assert!(
        base <= rail_table + DENIED * RPM_PAGE_SIZE
            && base + len > rail_table + DENIED * RPM_PAGE_SIZE,
        "the fixture must put the denied page inside the extent, or it proves \
         nothing about over-reading"
    );

    arena.deny_page(DENIED);
    let pair = D81ExtentPair::copy(base, len).expect("the page retry produces a pair");

    for id in &ids {
        let slot = rail_table + (*id as usize) * 8;
        assert_eq!(
            pair.u64_at(slot),
            read_u64_stable(slot),
            "id {id} is not on the denied page and must still be served"
        );
        assert!(
            pair.u64_at(slot).is_some(),
            "and it must be served from the IMAGE, not merely agree with a live \
             read: an unreadable page nothing reads costs zero fields"
        );
    }
}

#[test]
fn an_extent_that_is_readable_nowhere_produces_no_pair() {
    const PAGES: usize = 4;
    let arena = Arena::new(PAGES * RPM_PAGE_SIZE);
    for page in 0..PAGES {
        arena.deny_page(page);
    }
    assert!(
        D81ExtentPair::copy(arena.base, arena.size).is_none(),
        "no page copied means no pair"
    );
}

struct TornWorld {
    base: usize,
    bytes: Vec<u8>,
    mutations: Vec<(usize, u8)>,
    denied_pages: Vec<usize>,
    pass: std::cell::Cell<usize>,
}

impl TornWorld {
    fn new(base: usize, len: usize, seed: u64) -> TornWorld {
        let mut bytes = vec![0u8; len];
        let mut state = seed | 1;
        for chunk in bytes.chunks_mut(8) {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            let word = (z ^ (z >> 31)).to_le_bytes();
            chunk.copy_from_slice(&word[..chunk.len()]);
        }
        TornWorld {
            base,
            bytes,
            mutations: Vec::new(),
            denied_pages: Vec::new(),
            pass: std::cell::Cell::new(0),
        }
    }

    fn primitive(&self) -> impl FnMut(usize, &mut [u8]) -> bool + '_ {
        move |address: usize, buf: &mut [u8]| {
            let pass = self.pass.get();
            self.pass.set(pass + 1);
            let offset = address - self.base;
            if self.denied_pages.iter().any(|page| {
                let lo = page * RPM_PAGE_SIZE;
                offset < lo + RPM_PAGE_SIZE && offset + buf.len() > lo
            }) {
                return false;
            }
            buf.copy_from_slice(&self.bytes[offset..offset + buf.len()]);
            if pass % 2 == 1 {
                for (mutation_offset, value) in &self.mutations {
                    if *mutation_offset >= offset && *mutation_offset < offset + buf.len() {
                        buf[*mutation_offset - offset] = *value;
                    }
                }
            }
            true
        }
    }
}

#[test]
fn a_write_between_the_two_copies_refuses_only_the_field_it_touched() {
    const BASE: usize = 0x0002_0000_0000;
    let slots = 64usize;
    let len = slots * D81_SELECTOR_ENTRY_STRIDE;
    let mut world = TornWorld::new(BASE, len, 0xD81F_5000);
    let torn_bucket = 17usize;
    let key_offset = torn_bucket * D81_SELECTOR_ENTRY_STRIDE + 8;
    world
        .mutations
        .push((key_offset, world.bytes[key_offset] ^ 0xFF));

    let pair = D81ExtentPair::copy_with(BASE, len, world.primitive())
        .expect("both passes succeed, so the pair exists");

    for slot in 0..slots {
        let entry = BASE + slot * D81_SELECTOR_ENTRY_STRIDE;
        let marker = pair.u8_at(entry + 4);
        let key = pair.u32_at(entry + 8);
        let value = pair.u64_at(entry + 0x10);
        if slot == torn_bucket {
            assert_eq!(key, None, "the rewritten key cannot be served");
            assert!(
                marker.is_some() && value.is_some(),
                "the other two fields of the SAME bucket are untouched and must \
                 still be served -- refusal is per field, not per bucket"
            );
        } else {
            assert!(
                marker.is_some() && key.is_some() && value.is_some(),
                "bucket {slot} shares no byte with the rewritten key"
            );
        }
    }
}

/// the owner ruled against. Observed in `--release`:
#[test]
fn engine_junk_in_a_selector_bucket_s_dead_bytes_cannot_manufacture_a_torn_field() {
    const BASE: usize = 0x0003_0000_0000;
    let slots = 32usize;
    let len = slots * D81_SELECTOR_ENTRY_STRIDE;
    let mut world = TornWorld::new(BASE, len, 0xD81F_6000);
    let bucket = 9usize;
    let start = bucket * D81_SELECTOR_ENTRY_STRIDE;
    let dead: Vec<usize> = (0..D81_SELECTOR_ENTRY_STRIDE)
        .filter(|offset| {
            !SELECTOR_CONSUMED
                .iter()
                .any(|(field, width)| *offset >= *field && *offset < field + width)
        })
        .collect();
    assert_eq!(
        dead.len(),
        D81_SELECTOR_ENTRY_STRIDE - 13,
        "a bucket is 24 bytes of which d81 consumes 13"
    );
    for offset in &dead {
        world
            .mutations
            .push((start + offset, world.bytes[start + offset] ^ 0xFF));
    }

    let pair = D81ExtentPair::copy_with(BASE, len, world.primitive())
        .expect("both passes succeed, so the pair exists");

    let entry = BASE + start;
    assert!(
        pair.u8_at(entry + 4).is_some(),
        "the marker is outside every rewritten byte"
    );
    assert!(
        pair.u32_at(entry + 8).is_some(),
        "the key is outside every rewritten byte"
    );
    assert!(
        pair.u64_at(entry + 0x10).is_some(),
        "the value is outside every rewritten byte"
    );
}

#[test]
fn a_refused_whole_extent_falls_back_to_a_page_granular_pair() {
    const BASE: usize = 0x0004_0000_0000;
    const PAGES: usize = 6;
    const DENIED: usize = 2;
    let len = PAGES * RPM_PAGE_SIZE;
    let mut world = TornWorld::new(BASE, len, 0xD81F_7000);
    world.denied_pages.push(DENIED);

    let pair = D81ExtentPair::copy_with(BASE, len, world.primitive())
        .expect("five of six pages are readable");
    for page in 0..PAGES {
        let probe = BASE + page * RPM_PAGE_SIZE + 0x40;
        if page == DENIED {
            assert_eq!(pair.u64_at(probe), None, "the denied page answers nothing");
        } else {
            assert!(
                pair.u64_at(probe).is_some(),
                "page {page} is readable and must be recovered by the retry"
            );
        }
    }
    let straddle = BASE + DENIED * RPM_PAGE_SIZE - 4;
    assert_eq!(
        pair.u64_at(straddle),
        None,
        "a field spanning into the denied page cannot be served"
    );
    let clean_straddle = BASE + RPM_PAGE_SIZE - 4;
    assert!(
        pair.u64_at(clean_straddle - 4).is_some() && pair.u64_at(clean_straddle + 4).is_some(),
        "the fixture must straddle two pages that BOTH copied, or it proves \
         nothing beyond the denied-page case"
    );
    assert_eq!(
        pair.u64_at(clean_straddle),
        None,
        "a retried pair refuses any range that crosses a page, even between two \
         pages it holds"
    );

    let world = TornWorld::new(BASE, len, 0xD81F_7100);
    let whole = D81ExtentPair::copy_with(BASE, len, world.primitive())
        .expect("no page is denied, so the whole extent copies");
    assert!(
        whole.u64_at(clean_straddle).is_some(),
        "a whole-extent pair sampled every byte in one adjacent pair and must \
         not lose the straddling range"
    );
}

/// `checked_add(1)`. Observed in `--release`:
#[test]
fn the_selector_extent_covers_the_last_marker_the_probe_can_read() {
    let mask = FIXTURE_SELECTOR_MASK;
    let max_distance = FIXTURE_SELECTOR_MAX_DISTANCE;
    let slots = mask as usize + max_distance as usize + 2;
    let arena = Arena::new(
        (slots * D81_SELECTOR_ENTRY_STRIDE + 2 * RPM_PAGE_SIZE) / RPM_PAGE_SIZE * RPM_PAGE_SIZE,
    );
    arena.fill_pattern(0xD81F_8000);
    let table = arena.base;
    let (base, len) = d81_selector_extent(table, mask, max_distance).expect("formable");
    let pair = D81ExtentPair::copy(base, len).expect("readable");

    let mut entry = table + (mask as usize) * D81_SELECTOR_ENTRY_STRIDE;
    let mut distance = 1u16;
    let mut markers = 0usize;
    loop {
        assert!(
            pair.u8_at(entry + 4).is_some(),
            "the terminating marker one slot past the last admitted probe step \
             must be inside the image"
        );
        markers += 1;
        if distance > u16::from(max_distance).saturating_add(1) {
            break;
        }
        assert!(
            pair.u32_at(entry + 8).is_some(),
            "the key of an admitted probe step must be inside the image"
        );
        assert!(
            pair.u64_at(entry + 0x10).is_some(),
            "the value of an admitted probe step must be inside the image"
        );
        distance += 1;
        entry += D81_SELECTOR_ENTRY_STRIDE;
    }
    assert_eq!(
        markers,
        max_distance as usize + 2,
        "the probe reads max_distance + 2 markers from the top of the table"
    );
    assert_eq!(len, slots * D81_SELECTOR_ENTRY_STRIDE);
}

#[test]
fn the_rail_extent_spans_every_id_the_walk_reads_even_when_ids_are_sparse() {
    const TABLE: usize = 0x0005_0000_0000;
    for ids in [
        vec![0u32],
        vec![7, 100, 9_000, 13_412],
        vec![13_412, 7, 9_000, 100],
        (0..13_413u32).collect::<Vec<_>>(),
        (0..13_413u32).filter(|id| id % 5 == 2).collect::<Vec<_>>(),
    ] {
        let (base, len) =
            d81_rail_slot_extent(TABLE, ids.iter().copied()).expect("formable for these ids");
        let lo = *ids.iter().min().expect("non-empty") as usize;
        let hi = *ids.iter().max().expect("non-empty") as usize;
        assert_eq!(base, TABLE + lo * 8, "the extent starts at the lowest id");
        assert_eq!(
            base + len,
            TABLE + hi * 8 + 8,
            "the extent ends past the highest id's whole slot"
        );
        for id in &ids {
            let slot = TABLE + (*id as usize) * 8;
            assert!(
                slot >= base && slot + 8 <= base + len,
                "id {id} is read at an address inside the extent"
            );
        }
    }
    let sparse = [7u32, 100, 9_000, 13_412];
    let (_, len) = d81_rail_slot_extent(TABLE, sparse.iter().copied()).expect("formable");
    assert!(
        len > sparse.len() * 8,
        "an extent sized by the COUNT of visited ids would be short of the ids \
         themselves"
    );
    assert_eq!(d81_rail_slot_extent(TABLE, std::iter::empty()), None);
}

/// Observed in `--release`:
#[test]
fn no_consumed_field_can_straddle_a_page_boundary() {
    let crosses = |offset: usize, width: usize| {
        offset / RPM_PAGE_SIZE != (offset + width - 1) / RPM_PAGE_SIZE
    };
    for base in (0..RPM_PAGE_SIZE).step_by(8) {
        assert!(
            !crosses(base, 8),
            "a rail slot at page offset {base} straddles a page"
        );
        for (field, width) in SELECTOR_CONSUMED {
            assert!(
                !crosses(base + field, width),
                "the selector field at +{field:#x} ({width} bytes) straddles a \
                 page when its bucket starts at page offset {base}"
            );
        }
    }
    assert_eq!(
        D81_SELECTOR_ENTRY_STRIDE % 8,
        0,
        "a stride that is not a multiple of 8 puts buckets on offsets the sweep \
         above never tries"
    );
}

#[test]
fn an_extent_past_the_transient_cap_is_declined_rather_than_copied() {
    const BASE: usize = 0x0006_0000_0000;
    let mut attempts = 0usize;
    let pair = D81ExtentPair::copy_with(BASE, D81_BATCH_MAX_EXTENT_BYTES + 1, |_, _| {
        attempts += 1;
        true
    });
    assert!(pair.is_none(), "an over-cap extent produces no pair");
    assert_eq!(
        attempts, 0,
        "and it is declined before any copy is attempted"
    );
    let (_, len) = d81_selector_extent(BASE, u32::MAX - 2, 0).expect("the arithmetic still holds");
    assert!(len > D81_BATCH_MAX_EXTENT_BYTES);
    assert!(D81ExtentPair::copy(BASE, len).is_none());
}

fn counted(body: impl FnOnce()) -> (u64, u64) {
    const UNUSED_DOMAIN_ID: u16 = 219;
    let _rc_counters = super::super::lock_rc_counters();
    RC_ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = rc_domain_take_and_reset();
    {
        let _scope = RcDomainScope::new(UNUSED_DOMAIN_ID);
        body();
    }
    rc_flush_current_thread();
    RC_ENABLED.store(false, std::sync::atomic::Ordering::Relaxed);
    rc_domain_take_and_reset()
        .iter()
        .find(|(id, _)| *id == UNUSED_DOMAIN_ID)
        .map(|(_, counters)| (counters[RC_PD_READS], counters[RC_PD_BULK_BYTES]))
        .unwrap_or((0, 0))
}

/// cargo test -p collector --release d81_batched_table_price -- --ignored --nocapture
#[test]
#[ignore]
fn d81_batched_table_price() {
    const PROVINCES: u32 = 13_413;
    const RAILED: usize = 3_510;
    const PROBE_STEPS: usize = 2;
    const SELECTOR_MASK: u32 = 16_383;
    const SELECTOR_MAX_DISTANCE: u8 = 8;
    const TRIALS: usize = 7;

    let rail_bytes = (PROVINCES as usize) * 8;
    let selector_slots = SELECTOR_MASK as usize + SELECTOR_MAX_DISTANCE as usize + 2;
    let selector_bytes = selector_slots * D81_SELECTOR_ENTRY_STRIDE;
    let arena = Arena::new(
        (rail_bytes + selector_bytes + 4 * RPM_PAGE_SIZE) / RPM_PAGE_SIZE * RPM_PAGE_SIZE,
    );
    arena.fill_pattern(0xD81F_9000);
    let rail_table = arena.base;
    let selector_table =
        arena.base + (rail_bytes + RPM_PAGE_SIZE - 1) / RPM_PAGE_SIZE * RPM_PAGE_SIZE;
    for slot in 0..selector_slots {
        arena.put_u8(
            selector_table + slot * D81_SELECTOR_ENTRY_STRIDE + 4,
            u8::try_from(PROBE_STEPS + 1).expect("small"),
        );
        arena.put_u32(selector_table + slot * D81_SELECTOR_ENTRY_STRIDE + 8, 0);
    }

    let probe_start = |lookup: usize| -> usize {
        selector_table + ((lookup * 7_919) & SELECTOR_MASK as usize) * D81_SELECTOR_ENTRY_STRIDE
    };

    let live = || {
        let mut acc = 0u64;
        for id in 0..PROVINCES as usize {
            acc = acc.wrapping_add(read_u64_stable(rail_table + id * 8).unwrap_or(0));
        }
        for lookup in 0..RAILED {
            let mut entry = probe_start(lookup);
            for _ in 0..PROBE_STEPS {
                acc = acc.wrapping_add(u64::from(read_u8_stable(entry + 4).unwrap_or(0)));
                acc = acc.wrapping_add(u64::from(read_u32_stable(entry + 8).unwrap_or(0)));
                entry += D81_SELECTOR_ENTRY_STRIDE;
            }
            acc = acc.wrapping_add(read_u64_stable(entry + 0x10).unwrap_or(0));
        }
        acc
    };
    let batched = || {
        let mut acc = 0u64;
        let (rail_base, rail_len) =
            d81_rail_slot_extent(rail_table, 0..PROVINCES).expect("formable");
        let rail = D81ExtentPair::copy(rail_base, rail_len).expect("readable");
        let (selector_base, selector_len) =
            d81_selector_extent(selector_table, SELECTOR_MASK, SELECTOR_MAX_DISTANCE)
                .expect("formable");
        let selector = D81ExtentPair::copy(selector_base, selector_len).expect("readable");
        for id in 0..PROVINCES as usize {
            acc = acc.wrapping_add(
                rail.u64_at(rail_table + id * 8)
                    .or_else(|| read_u64_stable(rail_table + id * 8))
                    .unwrap_or(0),
            );
        }
        for lookup in 0..RAILED {
            let mut entry = probe_start(lookup);
            for _ in 0..PROBE_STEPS {
                acc = acc.wrapping_add(u64::from(
                    selector
                        .u8_at(entry + 4)
                        .or_else(|| read_u8_stable(entry + 4))
                        .unwrap_or(0),
                ));
                acc = acc.wrapping_add(u64::from(
                    selector
                        .u32_at(entry + 8)
                        .or_else(|| read_u32_stable(entry + 8))
                        .unwrap_or(0),
                ));
                entry += D81_SELECTOR_ENTRY_STRIDE;
            }
            acc = acc.wrapping_add(
                selector
                    .u64_at(entry + 0x10)
                    .or_else(|| read_u64_stable(entry + 0x10))
                    .unwrap_or(0),
            );
        }
        acc.wrapping_add((rail.image_bytes() + selector.image_bytes()) as u64)
    };

    let (live_calls, live_bulk) = counted(|| {
        std::hint::black_box(live());
    });
    let (batched_calls, batched_bulk) = counted(|| {
        std::hint::black_box(batched());
    });

    let live_scalar_bytes =
        2 * (PROVINCES as usize) * 8 + RAILED * (PROBE_STEPS * 2 * 1 + PROBE_STEPS * 2 * 4 + 2 * 8);
    let batched_scalar_bytes = 0usize;

    println!(
        "\n# d81 batched-table price (staged arena, counted through the \
         collector's own rc_d219_reads)\n"
    );
    println!("| arm | guarded calls | bulk bytes | scalar bytes | total bytes |");
    println!("|---|---:|---:|---:|---:|");
    println!(
        "| LIVE (per-slot / per-probe-step) | {live_calls} | {live_bulk} | \
         {live_scalar_bytes} | {} |",
        live_bulk as usize + live_scalar_bytes
    );
    println!(
        "| BATCHED (two extent pairs) | {batched_calls} | {batched_bulk} | \
         {batched_scalar_bytes} | {} |",
        batched_bulk as usize + batched_scalar_bytes
    );
    println!(
        "\ncall reduction {:.2}x, bytes {:.2}x",
        live_calls as f64 / batched_calls.max(1) as f64,
        (batched_bulk as usize + batched_scalar_bytes) as f64
            / (live_bulk as usize + live_scalar_bytes).max(1) as f64
    );

    let mut live_ms: Vec<f64> = Vec::with_capacity(TRIALS);
    let mut batched_ms: Vec<f64> = Vec::with_capacity(TRIALS);
    for trial in 0..TRIALS {
        let first_live = trial % 2 == 0;
        let mut run = |is_live: bool| {
            let started = Instant::now();
            let acc = if is_live { live() } else { batched() };
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            std::hint::black_box(acc);
            ms
        };
        if first_live {
            live_ms.push(run(true));
            batched_ms.push(run(false));
        } else {
            batched_ms.push(run(false));
            live_ms.push(run(true));
        }
    }
    let band = |values: &[f64]| {
        let lo = values.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        (lo, hi)
    };
    let (live_lo, live_hi) = band(&live_ms);
    let (batched_lo, batched_hi) = band(&batched_ms);
    println!(
        "\nwall over {TRIALS} interleaved pairs, ms: LIVE {live_lo:.2}-{live_hi:.2}, \
         BATCHED {batched_lo:.2}-{batched_hi:.2}, ratio {:.2}-{:.2}x",
        live_lo / batched_hi.max(f64::MIN_POSITIVE),
        live_hi / batched_lo.max(f64::MIN_POSITIVE)
    );
    let mut build_ms: Vec<f64> = Vec::with_capacity(TRIALS);
    for _ in 0..TRIALS {
        let started = Instant::now();
        let (rail_base, rail_len) =
            d81_rail_slot_extent(rail_table, 0..PROVINCES).expect("formable");
        let rail = D81ExtentPair::copy(rail_base, rail_len).expect("readable");
        let (selector_base, selector_len) =
            d81_selector_extent(selector_table, SELECTOR_MASK, SELECTOR_MAX_DISTANCE)
                .expect("formable");
        let selector = D81ExtentPair::copy(selector_base, selector_len).expect("readable");
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        std::hint::black_box(rail.image_bytes() + selector.image_bytes());
        build_ms.push(ms);
    }
    let (build_lo, build_hi) = band(&build_ms);
    println!(
        "of which building the two pairs (allocate + 4 guarded copies, \
         {} bytes held): {build_lo:.2}-{build_hi:.2} ms",
        2 * (rail_bytes + selector_bytes)
    );
    println!(
        "\nWALL DOES NOT TRANSFER TO THE LIVE FRAME. One arena means one \
         VirtualQuery region and an MRU hit on nearly every guarded call, so \
         this understates the fixed per-call component the batching removes. \
         The call count is the measurement; only a capture prices the frame."
    );
}

#[test]
fn the_two_clusters_are_a_call_reduction() {
    const PROVINCES: u32 = 2_048;
    const RAILED: usize = 512;
    const PROBE_STEPS: usize = 2;
    const SELECTOR_MASK: u32 = 2_047;
    const SELECTOR_MAX_DISTANCE: u8 = 8;

    let rail_bytes = (PROVINCES as usize) * 8;
    let selector_slots = SELECTOR_MASK as usize + SELECTOR_MAX_DISTANCE as usize + 2;
    let selector_bytes = selector_slots * D81_SELECTOR_ENTRY_STRIDE;
    let arena = Arena::new(
        (rail_bytes + selector_bytes + 4 * RPM_PAGE_SIZE) / RPM_PAGE_SIZE * RPM_PAGE_SIZE,
    );
    arena.fill_pattern(0xD81F_A000);
    let rail_table = arena.base;
    let selector_table =
        arena.base + (rail_bytes + RPM_PAGE_SIZE - 1) / RPM_PAGE_SIZE * RPM_PAGE_SIZE;
    for slot in 0..selector_slots {
        arena.put_u8(
            selector_table + slot * D81_SELECTOR_ENTRY_STRIDE + 4,
            u8::try_from(PROBE_STEPS + 1).expect("small"),
        );
        arena.put_u32(selector_table + slot * D81_SELECTOR_ENTRY_STRIDE + 8, 0);
    }
    let probe_start = |lookup: usize| -> usize {
        selector_table + ((lookup * 7_919) & SELECTOR_MASK as usize) * D81_SELECTOR_ENTRY_STRIDE
    };

    let (live_calls, _) = counted(|| {
        let mut acc = 0u64;
        for id in 0..PROVINCES as usize {
            acc = acc.wrapping_add(read_u64_stable(rail_table + id * 8).unwrap_or(0));
        }
        for lookup in 0..RAILED {
            let mut entry = probe_start(lookup);
            for _ in 0..PROBE_STEPS {
                acc = acc.wrapping_add(u64::from(read_u8_stable(entry + 4).unwrap_or(0)));
                acc = acc.wrapping_add(u64::from(read_u32_stable(entry + 8).unwrap_or(0)));
                entry += D81_SELECTOR_ENTRY_STRIDE;
            }
            acc = acc.wrapping_add(read_u64_stable(entry + 0x10).unwrap_or(0));
        }
        std::hint::black_box(acc);
    });
    let (batched_calls, batched_bulk) = counted(|| {
        let mut acc = 0u64;
        let (rail_base, rail_len) =
            d81_rail_slot_extent(rail_table, 0..PROVINCES).expect("formable");
        let rail = D81ExtentPair::copy(rail_base, rail_len).expect("readable");
        let (selector_base, selector_len) =
            d81_selector_extent(selector_table, SELECTOR_MASK, SELECTOR_MAX_DISTANCE)
                .expect("formable");
        let selector = D81ExtentPair::copy(selector_base, selector_len).expect("readable");
        for id in 0..PROVINCES as usize {
            acc = acc.wrapping_add(
                rail.u64_at(rail_table + id * 8)
                    .or_else(|| read_u64_stable(rail_table + id * 8))
                    .unwrap_or(0),
            );
        }
        for lookup in 0..RAILED {
            let mut entry = probe_start(lookup);
            for _ in 0..PROBE_STEPS {
                acc = acc.wrapping_add(u64::from(
                    selector
                        .u8_at(entry + 4)
                        .or_else(|| read_u8_stable(entry + 4))
                        .unwrap_or(0),
                ));
                acc = acc.wrapping_add(u64::from(
                    selector
                        .u32_at(entry + 8)
                        .or_else(|| read_u32_stable(entry + 8))
                        .unwrap_or(0),
                ));
                entry += D81_SELECTOR_ENTRY_STRIDE;
            }
            acc = acc.wrapping_add(
                selector
                    .u64_at(entry + 0x10)
                    .or_else(|| read_u64_stable(entry + 0x10))
                    .unwrap_or(0),
            );
        }
        std::hint::black_box(acc);
    });

    assert_eq!(
        live_calls,
        2 * u64::from(PROVINCES) + (RAILED as u64) * (4 * PROBE_STEPS as u64 + 2),
        "the live arm must issue exactly the census's call count, or the \
         comparison is against something other than what d81 used to do"
    );
    assert_eq!(
        batched_calls, 4,
        "the batched arm is two adjacent copies per extent and nothing else"
    );
    assert_eq!(
        batched_bulk as usize,
        2 * (rail_bytes + selector_bytes),
        "and it moves each extent twice, which is the byte cost of the saving"
    );
    assert!(
        batched_calls * 100 < live_calls,
        "the two clusters must be a call reduction of at least two orders of \
         magnitude: {live_calls} -> {batched_calls}"
    );
}

#[test]
fn the_batched_reads_are_a_fast_path_and_never_a_refusal() {
    let source = include_str!("../../attach_log.rs");
    let walk = source
        .split("fn d81_walk_provinces(")
        .nth(1)
        .expect("the d81 province walk");

    for (site, fallback) in [
        ("batch.u64_at(slot)", "source.u64_stable(slot)"),
        (
            "batch.u8_at(marker_address)",
            "source.u8_stable(marker_address)",
        ),
        (
            "batch.u32_at(entry_key_address)",
            "source.u32_stable(entry_key_address)",
        ),
        (
            "batch.u64_at(value_address)",
            "source.u64_stable(value_address)",
        ),
    ] {
        let at = walk
            .find(site)
            .unwrap_or_else(|| panic!("the batched read {site:?} must exist"));
        let tail = &walk[at..at + 200.min(walk.len() - at)];
        assert!(
            tail.contains(&format!(".or_else(|| {fallback})")),
            "the batched read {site:?} must fall back to the exact live read \
             {fallback:?}, or a batch miss becomes a refusal d81 never had"
        );
    }
    let reader = source
        .split("fn read_compact_rail_replay_state(")
        .nth(1)
        .expect("the observe half of d81");
    assert!(
        reader.contains("D81ExtentPair::copy(base, len)")
            && reader.matches("D81ExtentPair::copy(").count() == 2,
        "exactly the two funded clusters are batched; a third would need its \
         own equivalence proof"
    );
}
