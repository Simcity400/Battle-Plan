//! d81's two hand-batched read clusters: that they answer what the live reads
//! answered, that they lose nothing when part of the extent is unreadable or
//! torn, and what they cost.
//!
//! # What is under test
//!
//! Lane FX priced d81's in-bracket read census and found two clusters that are
//! 37% of its guarded calls and are BOTH indexed into one contiguous engine
//! table apiece:
//!
//!   * the rail table, `read_u64_stable(rail_table + id*8)`, once per province
//!     the topology names -- 26,826 guarded calls at t31's 13,413 provinces,
//!     the largest single row of the census;
//!   * the anchor-selector hash probe, a marker/key/value sequence per railed
//!     province -- 35,100 calls at t26's 3,510 railed provinces and two probe
//!     steps.
//!
//! Lane FZ replaced both with [`D81ExtentPair`]: two adjacent bulk copies of
//! the whole extent, decoded afterwards, with the individual field reads served
//! out of the copied images. That is the shape `d81_hash_table_rows` +
//! `d81_read_stable_bytes` already run in production for the hub's hash tables.
//!
//! # The claim these tests hold, stated so it can be falsified
//!
//! THE PAIR CAN ONLY EVER PRODUCE VALUES. Every accessor is an `Option` whose
//! `None` means "this pair cannot answer", never "the read failed", and both
//! call sites are `batch.…().or_else(|| <the exact live read>)`. So the
//! batching is a fast path in lane BG's sense and every refusal d81 publishes
//! -- stage, reason, index, subject, related id, raw value, anchor provenance
//! and the whole `read_witness` quintuple -- is still produced by the code that
//! produced it before. What the tests below have to establish is therefore:
//!
//!   1. where the pair DOES answer, its answer is the live read's answer
//!      ([`the_batched_rail_slots_answer_what_the_live_stable_reads_answer`],
//!      [`the_batched_selector_fields_answer_what_the_live_stable_reads_answer`]);
//!   2. where it cannot, the loss is confined to the bytes actually affected
//!      -- per PAGE for unreadability
//!      ([`an_unreadable_page_inside_the_extent_costs_only_the_fields_on_that_page`])
//!      and per FIELD for a write between the two copies
//!      ([`a_write_between_the_two_copies_refuses_only_the_field_it_touched`]);
//!   3. dead bucket bytes are outside every comparison
//!      ([`engine_junk_in_a_selector_bucket_s_dead_bytes_cannot_manufacture_a_torn_field`]);
//!   4. the extents are wide enough for every address the readers can form
//!      ([`the_selector_extent_covers_the_last_marker_the_probe_can_read`],
//!      [`the_rail_extent_spans_every_id_the_walk_reads_even_when_ids_are_sparse`]).
//!
//! # What they do NOT establish
//!
//! That the pair is taken at the same INSTANT the live reads were taken. It is
//! not, and no test can make it so: the pair is taken once before the walk
//! where the reads it replaces happened during it. [`D81ExtentPair`]'s own doc
//! states both consequences (a value rewritten after the pair is published at
//! its pair-time value; a page that stops being readable mid-walk is published
//! rather than refused). That is the sampling-window widening lane FX called
//! inherent to copy-only, and it is a property of the design, not a defect
//! these tests could catch.
//!
//! And they are offline. `d81` has no end-to-end fixture (see the head of
//! `d81_frame_cost.rs`), so the arms below drive the batching primitives and
//! the live primitives over the SAME staged pages rather than driving
//! `read_compact_rail_replay_state` twice. Wall time measured here does not
//! transfer to the live frame -- lane FX proved that directly, and
//! [`d81_batched_table_price`] says so at its own numbers.

use super::*;
use std::ffi::c_void;
use std::time::Instant;

// ---------------------------------------------------------------------------
// A staged world: real committed pages, real guarded reads
// ---------------------------------------------------------------------------

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
///
/// REAL PAGES, NOT A `Vec`. The whole question this file answers is what
/// `range_confirmed_good` does to a WIDE request when part of it is not
/// readable, and that is decided by `VirtualQuery` region boundaries. A `Vec`
/// cannot have a `PAGE_NOACCESS` hole in it, so it cannot stage the case.
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
        // A fresh region has to be visible to the epoch-stamped region cache,
        // or a stale confirmed span from an earlier test could answer for it.
        page_cache_new_frame();
        arena
    }

    /// Fill the whole arena with a deterministic, position-dependent pattern,
    /// so a field served from the WRONG offset cannot accidentally equal the
    /// right one.
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

    /// Make one page unreadable, and invalidate the region cache so the change
    /// is the one the next `range_confirmed_good` sees.
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

/// The shape the staged selector table is built at.
///
/// A mask of 4,095 is four times smaller than the ~16k the live table is
/// assumed to hold; the point of the fixture is the per-field decisions, and
/// those do not vary with the table's size. [`d81_batched_table_price`] runs
/// the production shape.
const FIXTURE_SELECTOR_MASK: u32 = 4_095;
const FIXTURE_SELECTOR_MAX_DISTANCE: u8 = 8;

/// The three field windows d81 CONSUMES out of a 0x18-byte selector bucket.
/// Bytes 0..4, 5..8 and 12..0x10 are read by nothing and must stay outside
/// every comparison.
///
/// AN ALIAS FOR THE PRODUCTION CONSTANT, NOT A COPY OF IT (Codex review
/// 019fe416-e1ad-77e3-b70f-b8aee6554b4f, finding 2). While this was a local
/// transcription, moving the production value read to an offset that CAN
/// straddle a page left `no_consumed_field_can_straddle_a_page_boundary` green.
const SELECTOR_CONSUMED: [(usize, usize); 3] = D81_SELECTOR_CONSUMED_FIELDS;

// ---------------------------------------------------------------------------
// 1. Where the pair answers, it answers the live read's answer
// ---------------------------------------------------------------------------

/// Every rail slot the pair serves is byte-identical to the `read_u64_stable`
/// the walk used to issue at that address.
///
/// Ids are deliberately SPARSE, because the extent is sized from the id range
/// and a dense fixture cannot tell a correct extent from one that happens to
/// start at the table base.
#[test]
fn the_batched_rail_slots_answer_what_the_live_stable_reads_answer() {
    const PAGES: usize = 16;
    let arena = Arena::new(PAGES * RPM_PAGE_SIZE);
    arena.fill_pattern(0xD81F_2000);

    // A rail table that does not start at the arena base, and ids that do not
    // start at zero and are not dense.
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

/// Every selector field the pair serves is byte-identical to the
/// `read_u8_stable` / `read_u32_stable` / `read_u64_stable` the probe used to
/// issue at that address -- checked at EVERY bucket of the extent, including
/// the probe tail past the masked table.
#[test]
fn the_batched_selector_fields_answer_what_the_live_stable_reads_answer() {
    let slots =
        FIXTURE_SELECTOR_MASK as usize + FIXTURE_SELECTOR_MAX_DISTANCE as usize + 2;
    let arena = Arena::new(((slots * D81_SELECTOR_ENTRY_STRIDE) + 2 * RPM_PAGE_SIZE)
        / RPM_PAGE_SIZE
        * RPM_PAGE_SIZE);
    arena.fill_pattern(0xD81F_3000);

    let table = arena.base + D81_SELECTOR_ENTRY_STRIDE;
    let (base, len) =
        d81_selector_extent(table, FIXTURE_SELECTOR_MASK, FIXTURE_SELECTOR_MAX_DISTANCE)
            .expect("the extent is formable");
    assert_eq!(base, table, "the selector extent starts at the table itself");
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

// ---------------------------------------------------------------------------
// 2. What the pair loses when part of the extent is not readable
// ---------------------------------------------------------------------------

/// An unreadable page inside the batched extent costs the fast path for the
/// fields ON THAT PAGE and nothing else. Every other slot in the same extent
/// is still served from the image, and every slot on the denied page falls
/// back to a live read that refuses exactly as it refuses today.
///
/// THIS IS THE OWNER'S REFUSAL-GRANULARITY RULING, MEASURED. The ruling admits
/// a per-extent fallback if per-row scoping is impractical; it is not
/// impractical here, and the granularity is finer than per-row: a page.
#[test]
fn an_unreadable_page_inside_the_extent_costs_only_the_fields_on_that_page() {
    const PAGES: usize = 8;
    const DENIED: usize = 3;
    let arena = Arena::new(PAGES * RPM_PAGE_SIZE);
    arena.fill_pattern(0xD81F_4000);

    // The rail table starts at the arena base, so slot n lies on page n*8/4096
    // and the denied page holds exactly `RPM_PAGE_SIZE / 8` slots.
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

/// An unreadable page that lies entirely in the OVER-READ -- the slots between
/// the ids the walk actually visits -- costs nothing at all.
///
/// THIS IS THE POINT OF THE OWNER'S RULING, and the case that separates the
/// shipped design from a per-extent one. d81 refuses the WHOLE frame at its
/// first refusal (`return fail(...)` inside the walk), so a batching that
/// gapped on any unreadable byte of its requested span would turn an unreadable
/// page in a region NOTHING READS into the loss of the entire d81 hour. Here it
/// costs zero fields, because a page is only ever consulted when a field on it
/// is asked for.
#[test]
fn an_unreadable_page_in_the_over_read_costs_nothing() {
    const PAGES: usize = 8;
    const DENIED: usize = 3;
    let arena = Arena::new(PAGES * RPM_PAGE_SIZE);
    arena.fill_pattern(0xD81F_4400);

    let rail_table = arena.base;
    let slots_per_page = RPM_PAGE_SIZE / 8;
    // Every id EXCEPT the ones whose slot lies on the page about to be denied.
    let ids: Vec<u32> = (0..(arena.size / 8) as u32)
        .filter(|id| (*id as usize) / slots_per_page != DENIED)
        .collect();
    let (base, len) =
        d81_rail_slot_extent(rail_table, ids.iter().copied()).expect("the extent is formable");
    // The extent still SPANS the denied page: the ids straddle it.
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

/// A pair whose extent is unreadable in EVERY page declines to exist, so the
/// caller stays on one branch rather than carrying a pair that answers nothing.
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

// ---------------------------------------------------------------------------
// 3. Torn windows, and the dead bytes that must stay out of the comparison
// ---------------------------------------------------------------------------

/// A copy primitive that serves a private world and rewrites `mutations` into
/// the SECOND pass over any range that contains them.
///
/// A TEAR IS THE ONE THING A FIXTURE CANNOT STAGE -- the engine rewriting the
/// extent between the two copies is not something an offline test can schedule
/// against real pages. Injecting the copy is how this domain already reaches
/// its torn arms (`d81_read_stable_bytes_with`, `d81_read_u64_pair_with`,
/// `d81_read_coherent_active_entry_with`), and it is what makes the comparison
/// mask testable rather than merely asserted.
struct TornWorld {
    base: usize,
    bytes: Vec<u8>,
    /// `(offset, replacement)` applied only on the second pass.
    mutations: Vec<(usize, u8)>,
    /// Offsets whose page must refuse, as `(page, refuse)`.
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

    /// The primitive handed to `D81ExtentPair::copy_with`. Each logical PASS is
    /// one call for the whole extent, or one call per page in the retry, so
    /// "second pass" is tracked by alternating calls over the same range --
    /// which is exactly the adjacency the pair relies on.
    fn primitive(&self) -> impl FnMut(usize, &mut [u8]) -> bool + '_ {
        move |address: usize, buf: &mut [u8]| {
            let pass = self.pass.get();
            self.pass.set(pass + 1);
            let offset = address - self.base;
            if self
                .denied_pages
                .iter()
                .any(|page| {
                    let lo = page * RPM_PAGE_SIZE;
                    offset < lo + RPM_PAGE_SIZE && offset + buf.len() > lo
                })
            {
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

/// A write landing between the two copies costs the fast path for the FIELD it
/// touched and for no other field, not even the neighbouring one in the same
/// bucket.
#[test]
fn a_write_between_the_two_copies_refuses_only_the_field_it_touched() {
    const BASE: usize = 0x0002_0000_0000;
    let slots = 64usize;
    let len = slots * D81_SELECTOR_ENTRY_STRIDE;
    let mut world = TornWorld::new(BASE, len, 0xD81F_5000);
    // Flip one byte of bucket 17's KEY. Its marker and its value share the
    // bucket and must be unaffected.
    let torn_bucket = 17usize;
    let key_offset = torn_bucket * D81_SELECTOR_ENTRY_STRIDE + 8;
    world.mutations.push((key_offset, world.bytes[key_offset] ^ 0xFF));

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

/// A write into a selector bucket's DEAD bytes cannot manufacture a torn field.
///
/// Bytes 0..4, 5..8 and 12..0x10 of every 0x18-byte bucket are read by nothing
/// in d81. Whole-interval byte comparison would gap the consumer on a write
/// there; comparing only the CONSUMED bytes is the owner's ruling and this is
/// what enforces it. The rail header already refuses to let its four dead bytes
/// into the same judgement, for the same reason.
///
/// MUTATION RUN, and the mutation is the ALTERNATIVE DESIGN rather than an
/// arbitrary break: `D81ExtentPair::copy_with` changed to compare the two
/// images WHOLE and decline the pair on any difference --
/// `if first != second { return None; }` after both copies succeed. That is
/// exactly what a whole-interval witness comparison does, and it is the shape
/// the owner ruled against. Observed in `--release`:
///
/// ```text
/// thread '...::engine_junk_in_a_selector_bucket_s_dead_bytes_cannot_manufacture_a_torn_field'
///   panicked at collector\src\attach_log\tests\d81_batched_tables.rs:585:10:
/// both passes succeed, so the pair exists
/// ```
///
/// Restored; the transcript is in
/// `extractor/native/runs/fz_d81_batch/mutation_dead_bytes.log`, and the
/// patch that produced it is `run_mutations.sh` beside it.
#[test]
fn engine_junk_in_a_selector_bucket_s_dead_bytes_cannot_manufacture_a_torn_field() {
    const BASE: usize = 0x0003_0000_0000;
    let slots = 32usize;
    let len = slots * D81_SELECTOR_ENTRY_STRIDE;
    let mut world = TornWorld::new(BASE, len, 0xD81F_6000);
    let bucket = 9usize;
    let start = bucket * D81_SELECTOR_ENTRY_STRIDE;
    // Every byte of the bucket that d81 does NOT consume.
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

/// The page-granular retry is reached when the whole-extent request refuses,
/// and it recovers every page that is individually readable.
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
    // And a field STRADDLING the denied page's boundary is unanswerable, since
    // it needs bytes from a page neither image holds.
    let straddle = BASE + DENIED * RPM_PAGE_SIZE - 4;
    assert_eq!(
        pair.u64_at(straddle),
        None,
        "a field spanning into the denied page cannot be served"
    );
    // AND SO IS A STRADDLE BETWEEN TWO PAGES THAT BOTH COPIED FINE. On a
    // retried pair each page is its own A/B pair, so the two halves of such a
    // range were sampled at different instants: both halves can be
    // individually stable while the value changed in between, and answering
    // would publish an image no single sample ever held (Codex review
    // 019fe416-e1ad-77e3-b70f-b8aee6554b4f, finding 1).
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

    // The refusal is scoped to the RETRY path: a whole-extent pair is one A/B
    // pair over everything, so the same straddle is perfectly answerable there.
    let world = TornWorld::new(BASE, len, 0xD81F_7100);
    let whole = D81ExtentPair::copy_with(BASE, len, world.primitive())
        .expect("no page is denied, so the whole extent copies");
    assert!(
        whole.u64_at(clean_straddle).is_some(),
        "a whole-extent pair sampled every byte in one adjacent pair and must \
         not lose the straddling range"
    );
}

// ---------------------------------------------------------------------------
// 4. The extents are wide enough for every address the readers can form
// ---------------------------------------------------------------------------

/// The selector extent covers the LAST marker `select_secondary` can read.
///
/// The probe starts at the masked slot -- anywhere in `[0, mask]` -- and
/// advances one stride at a time WITHOUT wrapping while
/// `distance <= max_distance + 1`. From a start at `mask` the last key it can
/// compare is at bucket `mask + max_distance` and the terminating marker it
/// reads one step past that is at `mask + max_distance + 1`.
///
/// The assertion is that the pair SERVES those addresses, not merely that some
/// number matches a formula: an extent short by the probe tail would leave the
/// walk on the live read there, which is a price regression a coverage
/// arithmetic check cannot see.
///
/// MUTATION RUN: `d81_selector_extent`'s `checked_add(2)` changed to
/// `checked_add(1)`. Observed in `--release`:
///
/// ```text
/// thread '...::the_selector_extent_covers_the_last_marker_the_probe_can_read'
///   panicked at collector\src\attach_log\tests\d81_batched_tables.rs:...
/// the terminating marker one slot past the last admitted probe step must be
///   inside the image
/// ```
///
/// Restored; the transcript is in
/// `extractor/native/runs/fz_d81_batch/mutation_probe_tail.log`.
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

    // Walk the probe's own address sequence from the worst-case start.
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

/// The rail extent spans every id the walk reads, however sparse the ids are.
///
/// Lane FX's census names the defect this is against: a copy sized by the
/// NUMBER of provinces visited is short whenever the ids are not dense from
/// zero. Sizing from the id RANGE makes sparse ids widen the extent, never
/// shorten it.
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
    // The sizing the census warned against, made concrete: four ids spanning
    // 13,405 slots need 107 KB of extent, not 32 bytes.
    let sparse = [7u32, 100, 9_000, 13_412];
    let (_, len) = d81_rail_slot_extent(TABLE, sparse.iter().copied()).expect("formable");
    assert!(
        len > sparse.len() * 8,
        "an extent sized by the COUNT of visited ids would be short of the ids \
         themselves"
    );
    // No ids means no extent, and therefore no pair and no behaviour change.
    assert_eq!(d81_rail_slot_extent(TABLE, std::iter::empty()), None);
}

/// No field either reader consumes can straddle a page boundary, at ANY
/// admissible table base.
///
/// WHY IT MATTERS, and it is the one property the page-granular retry rests on:
/// on the retry path each page is its own A/B pair, so a field spanning two
/// pages would have its low bytes sampled in one pair and its high bytes in
/// another, at different instants, and the comparison would be over an image no
/// single sample ever held. It cannot happen because both bases are admitted
/// only at 8-byte alignment (`is_map_control_heap_pointer`) and every consumed
/// field is naturally aligned inside its stride -- but that is a property of the
/// CURRENT strides and offsets, so it is checked here rather than believed.
///
/// EXHAUSTIVE, not sampled. `gcd(0x18, 4096) = 8`, so the set of bucket starts
/// modulo a page is exactly the 8-aligned offsets, all 512 of which are tried;
/// the rail slot's stride is 8 and its offsets are the same 512.
///
/// OFF THE PRODUCTION CONSTANTS, NOT A TRANSCRIPTION OF THEM. While the fixture
/// carried its own copy of the offsets, moving the production value read to one
/// that CAN straddle a page left this test green -- Codex review
/// 019fe416-e1ad-77e3-b70f-b8aee6554b4f, finding 2. `SELECTOR_CONSUMED` IS
/// `D81_SELECTOR_CONSUMED_FIELDS` now, and it fails.
///
/// MUTATION RUN: `D81_SELECTOR_VALUE_OFF` changed from `0x10` to `0x14`.
/// Observed in `--release`:
///
/// ```text
/// thread '...::no_consumed_field_can_straddle_a_page_boundary' panicked at
///   collector\src\attach_log\tests\d81_batched_tables.rs:842:13:
/// the selector field at +0x14 (8 bytes) straddles a page when its bucket
///   starts at page offset 4072
/// ```
///
/// Restored; the transcript is in
/// `extractor/native/runs/fz_d81_batch/mutation_field_offset.log` and the patch
/// that produced it is `run_mutation_c.sh` beside it.
///
/// The other mutation it catches is the stride: `D81_SELECTOR_ENTRY_STRIDE`
/// changed to 0x14 fails the `% 8` assertion at the bottom, which is the
/// premise the sweep's exhaustiveness rests on.
#[test]
fn no_consumed_field_can_straddle_a_page_boundary() {
    let crosses = |offset: usize, width: usize| {
        offset / RPM_PAGE_SIZE != (offset + width - 1) / RPM_PAGE_SIZE
    };
    for base in (0..RPM_PAGE_SIZE).step_by(8) {
        // The rail slot: one 8-byte field at the bucket start.
        assert!(
            !crosses(base, 8),
            "a rail slot at page offset {base} straddles a page"
        );
        // The selector bucket's three consumed fields.
        for (field, width) in SELECTOR_CONSUMED {
            assert!(
                !crosses(base + field, width),
                "the selector field at +{field:#x} ({width} bytes) straddles a \
                 page when its bucket starts at page offset {base}"
            );
        }
    }
    // And the bucket starts really are 8-aligned for every slot index, which is
    // what makes the sweep above exhaustive rather than a sample.
    assert_eq!(
        D81_SELECTOR_ENTRY_STRIDE % 8,
        0,
        "a stride that is not a multiple of 8 puts buckets on offsets the sweep \
         above never tries"
    );
}

/// The extent cap declines rather than allocates, and declining is not a
/// behaviour change -- it simply leaves the live per-read sequence in place.
#[test]
fn an_extent_past_the_transient_cap_is_declined_rather_than_copied() {
    const BASE: usize = 0x0006_0000_0000;
    let mut attempts = 0usize;
    let pair = D81ExtentPair::copy_with(BASE, D81_BATCH_MAX_EXTENT_BYTES + 1, |_, _| {
        attempts += 1;
        true
    });
    assert!(pair.is_none(), "an over-cap extent produces no pair");
    assert_eq!(attempts, 0, "and it is declined before any copy is attempted");
    // The selector extent at a mask the header cannot rule out is over the cap
    // and is therefore declined, not truncated.
    let (_, len) = d81_selector_extent(BASE, u32::MAX - 2, 0).expect("the arithmetic still holds");
    assert!(len > D81_BATCH_MAX_EXTENT_BYTES);
    assert!(D81ExtentPair::copy(BASE, len).is_none());
}

// ---------------------------------------------------------------------------
// 5. The price
// ---------------------------------------------------------------------------

/// Guarded reads and bulk bytes performed by `body`, from the collector's OWN
/// per-domain read counter.
///
/// SAME DOMAIN ID AND SAME LOCK as `d81_frame_cost::guarded_reads`, and that
/// reuse is what makes it safe rather than a collision: `lock_rc_counters`
/// serialises every arming site in the crate, and each one drains every slot
/// before it runs, so two callers of slot 219 can never be in flight together.
/// `RC_ENABLED` is process-global; the lock is the only thing that makes any
/// count in this file a measurement.
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

/// What the two batched clusters cost against what they replaced, COUNTED.
///
/// WHY A BENCH AND NOT AN ASSERTION on the wall: `#[ignore]` for the reason
/// `d81_guarded_read_cost_model` and `d81_copy_bracket_cost_model` give -- a
/// wall-clock gate in this tree flakes under lane load. The CALL and BYTE
/// counts printed here are not modelled, they come off the collector's own
/// counter, and `the_two_clusters_are_a_call_reduction` gates the direction
/// without a clock. Run it deliberately:
///
/// ```text
/// cargo test -p collector --release d81_batched_table_price -- --ignored --nocapture
/// ```
///
/// WALL DOES NOT TRANSFER. Lane FX proved this directly for exactly this
/// fixture shape: the arena is one allocation, so `range_confirmed_good`
/// resolves on its MRU fast path on nearly every call, where live d81 crosses
/// many VirtualQuery regions. That UNDERSTATES the fixed per-call component,
/// which is the only thing the batching removes. The wall is therefore
/// published as a RANGE over interleaved pairs and as nothing else; only a
/// capture can price d81's frame.
#[test]
#[ignore]
fn d81_batched_table_price() {
    // The production shape, from the capture wire and lane FX's census.
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
    let selector_table = arena.base + (rail_bytes + RPM_PAGE_SIZE - 1) / RPM_PAGE_SIZE
        * RPM_PAGE_SIZE;
    // Markers that terminate the probe after exactly PROBE_STEPS steps, so the
    // live arm issues the census's call count and not a walk of the whole table.
    for slot in 0..selector_slots {
        arena.put_u8(
            selector_table + slot * D81_SELECTOR_ENTRY_STRIDE + 4,
            u8::try_from(PROBE_STEPS + 1).expect("small"),
        );
        arena.put_u32(selector_table + slot * D81_SELECTOR_ENTRY_STRIDE + 8, 0);
    }

    let probe_start = |lookup: usize| -> usize {
        selector_table
            + ((lookup * 7_919) & SELECTOR_MASK as usize) * D81_SELECTOR_ENTRY_STRIDE
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

    // Scalar reads bump no byte counter (`read_scalar` probes with 0), so the
    // scalar bytes are the widths the arms issue, which are exact by
    // construction; the BULK bytes are counted.
    let live_scalar_bytes = 2 * (PROVINCES as usize) * 8
        + RAILED * (PROBE_STEPS * 2 * 1 + PROBE_STEPS * 2 * 4 + 2 * 8);
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

    // Interleaved wall pairs, published as a RANGE and never as a point.
    let mut live_ms: Vec<f64> = Vec::with_capacity(TRIALS);
    let mut batched_ms: Vec<f64> = Vec::with_capacity(TRIALS);
    for trial in 0..TRIALS {
        // Alternate which arm goes first, so a drift over the run does not land
        // on one arm.
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
    // The two pairs' CONSTRUCTION alone -- two `vec![0u8; len]` allocations
    // and two guarded copies apiece. Reported separately because the zero-fill
    // is work the live arm does not do at all, and because if the batching ever
    // stops paying it is the first place to look.
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

/// The direction of the price, without a clock: the two clusters must be a
/// strict call reduction, or the change has no purpose.
///
/// A SMALLER SHAPE THAN THE BENCH, so it can run in the ordinary gate: the
/// counts scale with the shape and the RATIO does not.
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
        selector_table
            + ((lookup * 7_919) & SELECTOR_MASK as usize) * D81_SELECTOR_ENTRY_STRIDE
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

    // The live arm is two guarded calls per `_stable` scalar; the batched arm
    // is FOUR calls in total, two per extent pair, and no live read at all.
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

// ---------------------------------------------------------------------------
// 6. The wiring is a fast path and stays one
// ---------------------------------------------------------------------------

/// Both call sites use the batch ONLY through `or_else(<the live read>)`.
///
/// This is what makes every claim above about refusal vocabulary hold: the
/// batch cannot reach a refusal, so no `D81FailureReason`, no `index`, no
/// `subject`, no `read_witness` field and no elision can be produced by it.
/// A source pin rather than a behavioural one because there is no d81
/// end-to-end fixture to drive both shapes through; it fails the moment a
/// batched read is allowed to decide a refusal on its own.
#[test]
fn the_batched_reads_are_a_fast_path_and_never_a_refusal() {
    let source = include_str!("../../attach_log.rs");
    let walk = source
        .split("fn read_compact_rail_replay_state(")
        .nth(1)
        .expect("the observe half of d81");

    for (site, fallback) in [
        ("batch.u64_at(slot)", "read_u64_stable(slot)"),
        ("batch.u8_at(marker_address)", "read_u8_stable(marker_address)"),
        ("batch.u32_at(entry_key_address)", "read_u32_stable(entry_key_address)"),
        ("batch.u64_at(value_address)", "read_u64_stable(value_address)"),
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
    assert!(
        walk.contains("D81ExtentPair::copy(base, len)")
            && walk.matches("D81ExtentPair::copy(").count() == 2,
        "exactly the two funded clusters are batched; a third would need its \
         own equivalence proof"
    );
}
