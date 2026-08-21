//! Lane PI: what ONE technology-instance visit costs in d13's witness walk,
//! and which part of it is worth attacking.
//!
//! # Why this file exists
//!
//! `tech_live` (d13) is the country bundle's LONGEST sub-read — 39.9 ms/frame
//! on oz_tip against 9.7 for the next one — so it alone sets the bundle
//! worker's wall, and d200's critical-path share is drawn from that wall on the
//! frames where the bundle's lane is the frame's longest
//! (`summarize_lane_spans` attributes only the argmax lane).
//!
//! # How big the walk really is
//!
//! NOT the size the wire suggests. The XUI09 vectors on the wire cover 323-324
//! countries, but `xui09_cross_seal_refusal` runs at EMISSION, long after
//! `read_country_technology_witness` has walked a country in full — so the
//! ~115 countries it discards were read and paid for.
//!
//! Counted, not estimated (`runs/pi_d200/d13_visit_census.py`, and confirmed
//! live to the decimal by `lw_d13_countries` / `lw_d13_states_visits`):
//! **439 countries x 552 technologies = 242,328 states visits, plus a
//! researched-subset re-read averaging 27.1 per country — 254,266 visits per
//! game hour**, which puts oz_tip's 39.8 ms at **156.6 ns per visit**. An
//! earlier draft of this header used lane LE's "~100-250 completed per country"
//! note and overstated the walk by ~35%; on this save it is 27.1.
//!
//! Each visit is four `read_into` calls (header `+0x0` 12 bytes twice, status
//! `+0x158` 28 bytes twice), the witness decision over them, and a
//! `TechnologyIdDictionary::admit`.
//!
//! # Three hypotheses, all refuted, nothing shipped
//!
//! This module is an ATTRIBUTION instrument, not a before/after: lane PI tried
//! three per-visit cuts and landed none of them.
//!
//! 1. `TechnologyIdDictionary::admit` — ~254k descents/frame, all but 552 of
//!    them redundant; priced by a scratch benchmark at ~7 ns/call, so ~1.78 ms
//!    of a 39.8 ms read (~4.5%). Real, but not the item.
//! 2. Fusing each A/B pair under ONE `range_confirmed_good` instead of two
//!    (~508k redundant confirmations/frame) — measured here at -4.0% to +8.8%
//!    against a 2.7% noise floor. Reverted, and it also carried two behavioural
//!    divergences across a `page_cache_new_frame` epoch bump.
//! 3. Hoisting the 80-byte `TechnologyWindowCopies` out of the per-instance
//!    call (~20.3 MB of dead zero-fill per game hour) — measured at -12.4% to
//!    +7.5% against a 6.2% noise floor. Reverted.
//!
//! # The arms
//!
//! * [`Arm::Shipped`] — the production [`read_technology_witness_fields`].
//! * [`Arm::ReadsOnly`] — its four raw copies and nothing else, so
//!   `Shipped - ReadsOnly` is what the witness DECISION costs.
//! * [`Arm::StatusOnly`] — one window's pair instead of two. A floor, not a
//!   proposal: the header window carries the vtable identity proof.
//!
//! # What the arms said, stated at the strength they support
//!
//! [`Arm::ReadsOnly`] tracks [`Arm::Shipped`] within noise, so the witness
//! decision is a few percent and there is nothing to win in the decode.
//! [`Arm::StatusOnly`] costs 52-61% of [`Arm::Shipped`], so cost scales with
//! how much of the object is fetched. Contiguous walk order is 4.5-5.4x faster
//! than shuffled (28-30 ns/visit against 133-153), which sizes what locality
//! would buy and which the recorder does not control: the order is the engine's
//! heap.
//!
//! WHAT THIS DOES NOT ESTABLISH, because a previous draft of this header
//! claimed it and a review took it apart: [`Arm::StatusOnly`] removes an
//! address window AND two guarded calls at once, so it cannot by itself
//! separate cache-line cost from per-call cost; and `stage_walk` fixes every
//! instance to a `0x200` stride rather than sweeping the real, unproved heap
//! alignment, so "exactly two cache lines per visit" is NOT proven — at
//! `tech_ptr % 64 == 16` the 28-byte status window straddles a line boundary.
//! A cut that removes a window, or amortises acquisition across instances, is
//! not ruled out by anything here; it would just have to change what the walk
//! reads, which is a correctness question rather than a performance one.
//!
//! The lever with real headroom remains not walking at all, which needs the
//! change signal the domain's own `compact_country_changed_domains` comment
//! says is still missing.
//!
//! This profile models the STATES loop only (95% of the visits); the completed
//! loop re-reads a ~27-per-country subset through the same function.
//!
//! `cargo test -p collector --release -- --ignored --nocapture d13_walk`

use super::*;

// ==================================================================
// The arena the walk reads
// ==================================================================

#[link(name = "kernel32")]
extern "system" {
    fn VirtualAlloc(
        address: *mut c_void,
        size: usize,
        alloc_type: Dword,
        protect: Dword,
    ) -> *mut c_void;
    fn VirtualFree(address: *mut c_void, size: usize, free_type: Dword) -> Bool;
}
const D13_MEM_RESERVE: Dword = 0x2000;
const D13_MEM_COMMIT: Dword = 0x1000;
const D13_MEM_RELEASE: Dword = 0x8000;
const D13_PAGE_READWRITE: Dword = 0x04;

struct WalkArena {
    chunks: Vec<(usize, usize)>,
}

impl WalkArena {
    fn new(chunk_count: usize, chunk_size: usize) -> WalkArena {
        let size = (chunk_size + 0xFFF) & !0xFFF;
        let chunks = (0..chunk_count)
            .map(|_| {
                let base = unsafe {
                    VirtualAlloc(
                        std::ptr::null_mut(),
                        size,
                        D13_MEM_RESERVE | D13_MEM_COMMIT,
                        D13_PAGE_READWRITE,
                    )
                };
                assert!(!base.is_null(), "d13 walk arena VirtualAlloc failed");
                (base as usize, size)
            })
            .collect();
        WalkArena { chunks }
    }

    fn write(&self, addr: usize, bytes: &[u8]) {
        assert!(
            self.chunks
                .iter()
                .any(|(base, size)| addr >= *base && addr + bytes.len() <= base + size),
            "d13 walk arena write outside every chunk"
        );
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), addr as *mut u8, bytes.len()) };
    }
}

impl Drop for WalkArena {
    fn drop(&mut self) {
        for (base, _) in &self.chunks {
            unsafe { VirtualFree(*base as *mut c_void, 0, D13_MEM_RELEASE) };
        }
    }
}

/// Deterministic order shuffle, so a country's pointer vector can be walked in
/// an order that is NOT the allocation order — which is what a real heap after
/// hundreds of game hours looks like, and what decides whether the walk's
/// prefetch hint has anything to work with.
struct Xorshift(u64);
impl Xorshift {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

// ==================================================================
// Staging one roster's worth of CTechnology instances
// ==================================================================

#[derive(Clone, Copy, Debug)]
struct WalkShape {
    countries: usize,
    techs: usize,
    /// Bytes between consecutive instances. A `CTechnology` is read out to
    /// `+0x1C8`, so anything below that overlaps; the real heap bucket is not
    /// pinned by any counter, so the profile sweeps it.
    stride: usize,
    /// How many VQ regions the instances are spread over.
    regions: usize,
    /// Walk the pointer vector in shuffled rather than allocation order.
    shuffled: bool,
}

impl WalkShape {
    /// The ATTEMPTED walk, not the published one: every country with a research
    /// row is walked, including the ~115 the cross-seal discards afterwards.
    fn production(regions: usize, shuffled: bool) -> WalkShape {
        WalkShape { countries: 439, techs: 552, stride: 0x200, regions, shuffled }
    }
}

struct StagedWalk {
    /// One entry per country: (status_ptr, instance pointers in walk order).
    countries: Vec<(usize, Vec<usize>)>,
    tech_vtables: [usize; 2],
    _arena: WalkArena,
    _status_arena: WalkArena,
}

fn stage_walk(shape: WalkShape) -> StagedWalk {
    let instances = shape.countries * shape.techs;
    let regions = shape.regions.max(1).min(instances);
    let per_region = instances.div_ceil(regions);
    let status_bytes = shape.countries * (0x100 + shape.techs * 8);
    let arena = WalkArena::new(regions, per_region * shape.stride);
    let status_arena = WalkArena::new(1, status_bytes.max(0x1000));
    let status_base = status_arena.chunks[0].0;
    // The two accepted vtables. Any values work: the walk compares the instance
    // word at `+0x0` against these two and nothing else.
    let tech_vtables = [0x1234_5678_9ab0usize, 0x1234_5678_9ab8usize];

    let mut rng = Xorshift(0x9E37_79B9_7F4A_7C15);
    let mut countries = Vec::with_capacity(shape.countries);
    let mut next_instance = 0usize;
    let mut cursor = status_base;
    for _country in 0..shape.countries {
        let status_ptr = cursor;
        cursor += 0x100;
        let vector_ptr = cursor;
        cursor += shape.techs * 8;

        let mut slots = Vec::with_capacity(shape.techs);
        for tech in 0..shape.techs {
            let region = next_instance / per_region;
            let within = next_instance % per_region;
            next_instance += 1;
            let addr = arena.chunks[region].0 + within * shape.stride;
            // vtable at +0x0, alternating between the two accepted values so
            // the comparison cannot be constant-folded by luck.
            arena.write(addr, &(tech_vtables[tech & 1] as u64).to_le_bytes());
            // definition index at +0x8: sparse, matching the wire (no capture
            // has ever set the XUI09 dense flag).
            arena.write(addr + TECHNOLOGY_DEF_INDEX, &((tech as u32) * 3 + 7).to_le_bytes());
            // owner back-pointer at +0x158, template at +0x160, classifier at
            // +0x170 — the three the status window carries.
            arena.write(addr + TECHNOLOGY_STATUS_BACKLINK, &(status_ptr as u64).to_le_bytes());
            let template = status_base + 0x40 + (tech % 16) * 8;
            arena.write(addr + TECHNOLOGY_TEMPLATE_PTR, &(template as u64).to_le_bytes());
            arena.write(addr + TECHNOLOGY_UI_STATE_ENUM, &(1 + (tech as u32 % 6)).to_le_bytes());
            slots.push(addr);
        }
        if shape.shuffled {
            for i in (1..slots.len()).rev() {
                let j = (rng.next_u64() % (i as u64 + 1)) as usize;
                slots.swap(i, j);
            }
        }
        for (index, slot) in slots.iter().enumerate() {
            status_arena.write(vector_ptr + index * 8, &(*slot as u64).to_le_bytes());
        }
        status_arena.write(
            status_ptr + TECH_STATUS_ALL_TECHNOLOGIES_DATA,
            &(vector_ptr as u64).to_le_bytes(),
        );
        status_arena.write(
            status_ptr + TECH_STATUS_ALL_TECHNOLOGIES_SIZE,
            &(shape.techs as u32).to_le_bytes(),
        );
        countries.push((status_ptr, slots));
    }

    StagedWalk {
        countries,
        tech_vtables,
        _arena: arena,
        _status_arena: status_arena,
    }
}

// ==================================================================
// The arms
// ==================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Arm {
    /// The production [`read_technology_witness_fields`].
    Shipped,
    /// Its four raw copies and nothing else.
    ReadsOnly,
    /// One window's pair instead of two. A floor, not a proposal.
    StatusOnly,
}

fn run_arm(arm: Arm, staged: &StagedWalk) -> u64 {
    let mut accepted = 0u64;
    let mut copies = TechnologyWindowCopies::default();
    for (status_ptr, slots) in &staged.countries {
        for &tech_ptr in slots {
            match arm {
                Arm::Shipped => {
                    if read_technology_witness_fields(tech_ptr, staged.tech_vtables, *status_ptr)
                        .is_ok()
                    {
                        accepted += 1;
                    }
                }
                Arm::ReadsOnly => {
                    let ok = read_into(tech_ptr, &mut copies.header_a)
                        && read_into(tech_ptr, &mut copies.header_b)
                        && read_into(
                            tech_ptr + TECHNOLOGY_STATUS_WINDOW_START,
                            &mut copies.status_a,
                        )
                        && read_into(
                            tech_ptr + TECHNOLOGY_STATUS_WINDOW_START,
                            &mut copies.status_b,
                        );
                    accepted += u64::from(ok);
                }
                Arm::StatusOnly => {
                    let ok = read_into(
                        tech_ptr + TECHNOLOGY_STATUS_WINDOW_START,
                        &mut copies.status_a,
                    ) && read_into(
                        tech_ptr + TECHNOLOGY_STATUS_WINDOW_START,
                        &mut copies.status_b,
                    );
                    accepted += u64::from(ok);
                }
            }
        }
    }
    accepted
}

fn micros(d: std::time::Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

/// Interleaved arms, order rotated per repetition, minimised — the shape lanes
/// PC and PE used, and for the same reason: this box cannot carry a claim
/// across two builds.
#[test]
#[ignore = "profile"]
fn profile_d13_walk_arms() {
    const REPS: usize = 31;
    const ARMS: [Arm; 3] = [Arm::Shipped, Arm::ReadsOnly, Arm::StatusOnly];
    println!(
        "{:>7} {:>6} {:>8} | {:>10} {:>10} {:>10} | {:>9} {:>11}",
        "regions", "shuf", "visits", "shipped_ms", "reads_ms", "status_ms", "ns/visit",
        "status/ship"
    );
    for shape in [
        WalkShape::production(1, false),
        WalkShape::production(1, true),
        WalkShape::production(64, true),
        WalkShape::production(439, true),
    ] {
        let staged = stage_walk(shape);
        let visits = (shape.countries * shape.techs) as u64;
        let mut samples: [Vec<f64>; 3] = Default::default();
        let mut accepted: [u64; 3] = [0; 3];
        for rep in 0..REPS {
            for slot in 0..ARMS.len() {
                let index = (slot + rep) % ARMS.len();
                page_cache_new_frame();
                let started = std::time::Instant::now();
                let got = run_arm(ARMS[index], &staged);
                samples[index].push(micros(started.elapsed()));
                accepted[index] = got;
            }
        }
        for (index, arm) in ARMS.iter().enumerate() {
            assert_eq!(
                accepted[index], visits,
                "{arm:?} did not accept every staged instance"
            );
        }
        let best = |v: &mut Vec<f64>| -> f64 {
            v.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
            v[0]
        };
        let shipped = best(&mut samples[0]);
        let reads = best(&mut samples[1]);
        let status = best(&mut samples[2]);
        println!(
            "{:>7} {:>6} {:>8} | {:>10.3} {:>10.3} {:>10.3} | {:>9.1} {:>10.1}%",
            shape.regions,
            shape.shuffled,
            visits,
            shipped / 1000.0,
            reads / 1000.0,
            status / 1000.0,
            shipped * 1000.0 / visits as f64,
            100.0 * status / shipped,
        );
    }
}

/// The noise floor of THIS box for THIS rig: the same arm, three times in a
/// row. Quote no cut narrower than this.
#[test]
#[ignore = "profile"]
fn profile_d13_walk_noise_floor() {
    const REPS: usize = 31;
    let staged = stage_walk(WalkShape::production(64, true));
    for round in 0..3 {
        let mut samples = Vec::new();
        for _ in 0..REPS {
            page_cache_new_frame();
            let started = std::time::Instant::now();
            let _ = run_arm(Arm::Shipped, &staged);
            samples.push(micros(started.elapsed()));
        }
        samples.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
        println!("round {round}: {:.3} ms", samples[0] / 1000.0);
    }
}

/// PINS THE READ SHAPE the attribution above is built on: one technology visit
/// is FOUR guarded reads, of TWO distinct ranges, each read twice in A/B order.
///
/// This is the fact every number in this module's header divides by, and it is
/// what three attempted cuts were measured against. It is read back off the d47
/// witness tap, which records `(addr, len)` for every guarded read on this
/// thread, so it describes what the process was actually asked for rather than
/// what the source appears to say.
///
/// kills: any future change to the walk's read shape — a fused pair, a merged
/// window, a dropped stable-read half — lands here and forces the header's
/// arithmetic to be re-derived rather than silently going stale.
#[test]
fn one_technology_visit_is_four_guarded_reads_of_two_ranges() {
    let shape = WalkShape { countries: 2, techs: 4, stride: 0x200, regions: 1, shuffled: false };
    let staged = stage_walk(shape);

    let capture = Domain47WitnessCapture::start(Domain47WitnessLog::default());
    let accepted = run_arm(Arm::Shipped, &staged);
    let log = capture.finish();
    assert!(!log.overflowed && !log.tap_fault, "the tap did not record cleanly");
    assert_eq!(accepted, (shape.countries * shape.techs) as u64);

    let reads: Vec<(usize, u32)> = log.records.iter().map(|r| (r.addr, r.len)).collect();
    assert_eq!(
        reads.len(),
        shape.countries * shape.techs * 4,
        "a visit must cost exactly four guarded reads"
    );
    for (country, (_status_ptr, slots)) in staged.countries.iter().enumerate() {
        for (tech, &tech_ptr) in slots.iter().enumerate() {
            let at = (country * shape.techs + tech) * 4;
            assert_eq!(
                &reads[at..at + 4],
                &[
                    (tech_ptr, TECHNOLOGY_HEADER_WINDOW as u32),
                    (tech_ptr, TECHNOLOGY_HEADER_WINDOW as u32),
                    (
                        tech_ptr + TECHNOLOGY_STATUS_WINDOW_START,
                        TECHNOLOGY_STATUS_WINDOW_LEN as u32
                    ),
                    (
                        tech_ptr + TECHNOLOGY_STATUS_WINDOW_START,
                        TECHNOLOGY_STATUS_WINDOW_LEN as u32
                    ),
                ],
                "visit {country}/{tech} did not read two ranges twice each"
            );
        }
    }
    // The two windows are 12 and 28 bytes, so one visit copies 80 bytes across
    // its four reads. How many CACHE LINES that touches is NOT pinned here and
    // is not claimed anywhere: it depends on the instance's real alignment, and
    // at `tech_ptr % 64 == 16` the 28-byte status window straddles a boundary.
    assert_eq!(TECHNOLOGY_HEADER_WINDOW, 0xC);
    assert_eq!(TECHNOLOGY_STATUS_WINDOW_LEN, 0x1C);
}
