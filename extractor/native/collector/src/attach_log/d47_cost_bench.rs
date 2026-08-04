//! Deterministic cost model for domain 47's two hot loops (lane Q, 2026-08-04).
//!
//! WHAT THIS IS. A live capture is the only instrument that produces
//! `lw_d47_crit_us`, and it needs the game running. This module is the
//! offline stand-in: it lays out a fake CCharacter/CAdvisor/CUnitLeader/
//! CCountryLeader world in real `VirtualAlloc` pages and drives it through
//! the SAME production functions and the SAME guarded-read primitives
//! (`read_into` -> `range_confirmed_good` -> `seh_guarded_copy`) that the
//! collector uses against hoi4.exe.
//!
//! WHAT IS MEASURED vs INFERRED:
//!  * MEASURED here: the per-object CPU cost of the two loops, on this
//!    machine, through the real primitives, at the real per-frame object
//!    counts taken from run t15's manifest.
//!  * INFERRED: that a ratio measured here transfers to the live capture.
//!    The fake heap is one contiguous region, so `range_confirmed_good`
//!    always hits its MRU entry; the live heap has many regions and a
//!    higher miss rate. A miss costs a `VirtualQuery`, and a reduction in
//!    the NUMBER of guarded calls removes proportionally many chances to
//!    miss. So this model is CONSERVATIVE: it understates the live saving
//!    of removing a guarded call, and it cannot overstate it.
//!
//! Object counts are t15's own, derived from
//! `runs/live_unlock/t15/native_session_manifest.json`:
//! `domain47_manager_historical_count`/`_dynamic_count` divided by
//! `domain47_prewarm_ran` (236) give the census width; the
//! `domain47_adapter_*_rows` totals over the same 236 records give the
//! frame-walk widths.
//!
//! These entry points are `#[ignore]`d: they are an instrument, not a
//! gate. Run with
//! `cargo test -p collector --release -- --ignored --nocapture d47_cost`.

use super::*;
use domain47::adapter::{
    AdapterInputRef, AdvisorRoot, CharacterRoot, CountryLeaderRoleRoot, TraitDefinitionRoot,
    UnitLeaderRoot,
};
use domain47::extractor::{
    ResolvedTypeRoots, CADVISOR_CHARACTER_PTR_OFF, CCHARACTER_COUNTRY_TAG_DB_ID_OFF,
    CCHARACTER_ID_OFF, CCHARACTER_NAME_OFF, CCHARACTER_TOKEN_OFF, CCHARACTER_TYPE_ID,
    CCHARACTER_TYPE_OFF, CCHARACTER_UNIT_LEADER_PTR_OFF, CCOUNTRY_LEADER_CHARACTER_PTR_OFF,
    CUNIT_LEADER_CHARACTER_ID_COPY_OFF, CUNIT_LEADER_ID_OFF, CUNIT_LEADER_SKILL_SOURCE_PTR_OFF,
    CUNIT_LEADER_SKILL_VALUE_OFF, CUNIT_LEADER_TRAIT_CAPACITY_OFF, CUNIT_LEADER_TRAIT_DATA_OFF,
    CUNIT_LEADER_TRAIT_SIZE_OFF, CUNIT_LEADER_TYPE_ID, CUNIT_LEADER_TYPE_OFF,
};

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
const BENCH_MEM_RESERVE: Dword = 0x2000;
const BENCH_MEM_COMMIT_FLAG: Dword = 0x1000;
const BENCH_MEM_RELEASE: Dword = 0x8000;
const BENCH_PAGE_READWRITE: Dword = 0x04;

// --- t15 per-frame object counts ---------------------------------------
/// `domain47_manager_historical_count` 2_387_376 / `domain47_prewarm_ran` 236.
const T15_MANAGER_HISTORICAL: usize = 10_116;
/// `domain47_manager_dynamic_count` 776_204 / 236.
const T15_MANAGER_DYNAMIC: usize = 3_289;
/// Characters the census walks each frame.
const T15_CENSUS_CHARACTERS: usize = T15_MANAGER_HISTORICAL + T15_MANAGER_DYNAMIC;
/// `domain47_adapter_character_rows` 1_233_668 / 236.
const T15_FRAME_CHARACTERS: usize = 5_227;
/// `domain47_adapter_advisor_roster_rows` 792_580 / 236.
const T15_FRAME_ADVISORS: usize = 3_358;
/// `domain47_adapter_unit_leader_rows` 125_958 / 236.
const T15_FRAME_UNIT_LEADERS: usize = 534;
/// `domain47_adapter_country_leader_role_rows` 172_877 / 236.
const T15_FRAME_COUNTRY_LEADER_ROLES: usize = 733;
/// `domain47_prewarm_trait_token_count` 17_848 / 236.
const T15_FRAME_TRAIT_TOKENS: usize = 76;
/// `domain47_adapter_unit_leader_trait_rows` 316_946 / 236, spread over the
/// unit leaders (2.5 traits each).
const T15_TRAITS_PER_LEADER: usize = 3;

const CHARACTER_STRIDE: usize = 0x180;
const ADVISOR_STRIDE: usize = 0x100;
const COUNTRY_LEADER_STRIDE: usize = 0x100;
const TRAIT_STRIDE: usize = 0x40;
const UNIT_LEADER_STRIDE: usize = 0x1000;
const SKILL_SOURCE_STRIDE: usize = 0x200;
const TRAIT_VEC_STRIDE: usize = 0x40;

const BENCH_CCHARACTER_VTABLE: u64 = 0x1234_5678_0000_1000;
const BENCH_CARMY_LEADER_VTABLE: u64 = 0x1234_5678_0000_2000;
const BENCH_CNAVY_LEADER_VTABLE: u64 = 0x1234_5678_0000_3000;
const BENCH_CCOUNTRY_LEADER_VTABLE: u64 = 0x1234_5678_0000_4000;
const BENCH_CADVISOR_VTABLE: u64 = 0x1234_5678_0000_5000;
const BENCH_CUNIT_LEADER_TRAIT_VTABLE: u64 = 0x1234_5678_0000_6000;

/// One `VirtualAlloc` arena freed on drop.
struct BenchArena {
    base: usize,
    size: usize,
}

impl BenchArena {
    fn new(size: usize) -> BenchArena {
        let base = unsafe {
            VirtualAlloc(
                std::ptr::null_mut(),
                size,
                BENCH_MEM_RESERVE | BENCH_MEM_COMMIT_FLAG,
                BENCH_PAGE_READWRITE,
            )
        };
        assert!(!base.is_null(), "bench arena VirtualAlloc failed");
        BenchArena {
            base: base as usize,
            size,
        }
    }

    fn put_u32(&self, addr: usize, value: u32) {
        assert!(addr >= self.base && addr + 4 <= self.base + self.size);
        unsafe { std::ptr::write_unaligned(addr as *mut u32, value) };
    }

    fn put_u64(&self, addr: usize, value: u64) {
        assert!(addr >= self.base && addr + 8 <= self.base + self.size);
        unsafe { std::ptr::write_unaligned(addr as *mut u64, value) };
    }

    fn put_bytes(&self, addr: usize, bytes: &[u8]) {
        assert!(addr >= self.base && addr + bytes.len() <= self.base + self.size);
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), addr as *mut u8, bytes.len()) };
    }

    /// Write an MSVC `std::string` at `addr`. Strings <= 15 bytes take the
    /// SSO form (characters inline); longer ones point at `heap` and are
    /// written there. The engine's real character names are mostly SSO —
    /// `"GER_erwin_rommel"` is 16 — so the mix matters and both are built.
    fn put_msvc_string(&self, addr: usize, heap: usize, text: &str) {
        let bytes = text.as_bytes();
        let size = bytes.len();
        if size < 16 {
            self.put_bytes(addr, bytes);
            self.put_bytes(addr + size, &[0u8]);
            self.put_u64(addr + 0x10, size as u64);
            self.put_u64(addr + 0x18, 15);
        } else {
            self.put_bytes(heap, bytes);
            self.put_bytes(heap + size, &[0u8]);
            self.put_u64(addr, heap as u64);
            self.put_u64(addr + 0x10, size as u64);
            self.put_u64(addr + 0x18, (size + 16) as u64);
        }
    }
}

impl Drop for BenchArena {
    fn drop(&mut self) {
        unsafe { VirtualFree(self.base as *mut c_void, 0, BENCH_MEM_RELEASE) };
    }
}

struct BenchWorld {
    _arena: BenchArena,
    roots: ResolvedTypeRoots,
    /// Every censused CCharacter address, in manager-roster order.
    census_characters: Vec<usize>,
    character_roots: Vec<CharacterRoot>,
    unit_leader_roots: Vec<UnitLeaderRoot>,
    trait_roots: Vec<TraitDefinitionRoot>,
    country_leader_roots: Vec<CountryLeaderRoleRoot>,
    advisor_roots: Vec<AdvisorRoot>,
}

fn bench_world() -> BenchWorld {
    // Character bodies + their long-name heap buffers, then the other
    // classes. Sized generously; one contiguous committed region.
    let char_bytes = T15_CENSUS_CHARACTERS * CHARACTER_STRIDE;
    let string_heap_bytes = T15_CENSUS_CHARACTERS * 0x80;
    let advisor_bytes = T15_FRAME_ADVISORS * ADVISOR_STRIDE;
    let country_leader_bytes = T15_FRAME_COUNTRY_LEADER_ROLES * COUNTRY_LEADER_STRIDE;
    let trait_bytes = T15_FRAME_TRAIT_TOKENS * TRAIT_STRIDE;
    let unit_leader_bytes = T15_FRAME_UNIT_LEADERS * UNIT_LEADER_STRIDE;
    let skill_bytes = T15_FRAME_UNIT_LEADERS * SKILL_SOURCE_STRIDE;
    let trait_vec_bytes = T15_FRAME_UNIT_LEADERS * TRAIT_VEC_STRIDE;
    let total = char_bytes
        + string_heap_bytes
        + advisor_bytes
        + country_leader_bytes
        + trait_bytes
        + unit_leader_bytes
        + skill_bytes
        + trait_vec_bytes
        + 0x10000;
    let arena = BenchArena::new(total);

    let char_base = arena.base;
    let string_heap_base = char_base + char_bytes;
    let advisor_base = string_heap_base + string_heap_bytes;
    let country_leader_base = advisor_base + advisor_bytes;
    let trait_base = country_leader_base + country_leader_bytes;
    let unit_leader_base = trait_base + trait_bytes;
    let skill_base = unit_leader_base + unit_leader_bytes;
    let trait_vec_base = skill_base + skill_bytes;

    let roots = ResolvedTypeRoots {
        ccharacter_vtable: BENCH_CCHARACTER_VTABLE,
        carmy_leader_vtable: BENCH_CARMY_LEADER_VTABLE,
        cnavy_leader_vtable: BENCH_CNAVY_LEADER_VTABLE,
        ccountry_leader_vtable: BENCH_CCOUNTRY_LEADER_VTABLE,
        cadvisor_vtable: BENCH_CADVISOR_VTABLE,
        cunit_leader_trait_vtable: BENCH_CUNIT_LEADER_TRAIT_VTABLE,
    };

    // Unit leaders first: characters point at them.
    let mut unit_leader_roots = Vec::with_capacity(T15_FRAME_UNIT_LEADERS);
    let mut trait_roots = Vec::with_capacity(T15_FRAME_TRAIT_TOKENS);
    for idx in 0..T15_FRAME_TRAIT_TOKENS {
        let addr = trait_base + idx * TRAIT_STRIDE;
        arena.put_u64(addr, BENCH_CUNIT_LEADER_TRAIT_VTABLE);
        trait_roots.push(TraitDefinitionRoot {
            trait_addr: addr as u64,
            token: format!("trait_{idx:03}"),
        });
    }
    for idx in 0..T15_FRAME_UNIT_LEADERS {
        let addr = unit_leader_base + idx * UNIT_LEADER_STRIDE;
        let kind = if idx % 4 == 3 {
            domain47::UNIT_LEADER_NAVY
        } else {
            domain47::UNIT_LEADER_ARMY
        };
        let vtable = if kind == domain47::UNIT_LEADER_NAVY {
            BENCH_CNAVY_LEADER_VTABLE
        } else {
            BENCH_CARMY_LEADER_VTABLE
        };
        arena.put_u64(addr, vtable);
        arena.put_u32(addr + CUNIT_LEADER_TYPE_OFF as usize, CUNIT_LEADER_TYPE_ID);
        arena.put_u32(addr + CUNIT_LEADER_ID_OFF as usize, 7000 + idx as u32);
        arena.put_u32(
            addr + CUNIT_LEADER_CHARACTER_ID_COPY_OFF as usize,
            idx as u32,
        );
        let skill = skill_base + idx * SKILL_SOURCE_STRIDE;
        arena.put_u64(addr + CUNIT_LEADER_SKILL_SOURCE_PTR_OFF as usize, skill as u64);
        arena.put_u32(skill + CUNIT_LEADER_SKILL_VALUE_OFF as usize, 4);
        let vec_addr = trait_vec_base + idx * TRAIT_VEC_STRIDE;
        arena.put_u64(addr + CUNIT_LEADER_TRAIT_DATA_OFF as usize, vec_addr as u64);
        arena.put_u32(
            addr + CUNIT_LEADER_TRAIT_CAPACITY_OFF as usize,
            T15_TRAITS_PER_LEADER as u32,
        );
        arena.put_u32(
            addr + CUNIT_LEADER_TRAIT_SIZE_OFF as usize,
            T15_TRAITS_PER_LEADER as u32,
        );
        for slot in 0..T15_TRAITS_PER_LEADER {
            let trait_addr = trait_roots[(idx + slot) % T15_FRAME_TRAIT_TOKENS].trait_addr;
            arena.put_u64(vec_addr + slot * 8, trait_addr);
        }
        unit_leader_roots.push(UnitLeaderRoot {
            leader_addr: addr as u64,
            country_tag_db_id: (idx % 200) as u32,
            leader_kind: kind,
            name: Some(format!("Leader Name {idx}")),
            token: Some(format!("LEADER_TOKEN_{idx}")),
        });
    }

    // CCharacter bodies. The census walks all of them; the adopted root
    // list is the frame-walk prefix.
    let mut census_characters = Vec::with_capacity(T15_CENSUS_CHARACTERS);
    let mut character_roots = Vec::with_capacity(T15_FRAME_CHARACTERS);
    for idx in 0..T15_CENSUS_CHARACTERS {
        let addr = char_base + idx * CHARACTER_STRIDE;
        let heap = string_heap_base + idx * 0x80;
        arena.put_u64(addr, BENCH_CCHARACTER_VTABLE);
        arena.put_u32(addr + CCHARACTER_TYPE_OFF as usize, CCHARACTER_TYPE_ID);
        arena.put_u32(addr + CCHARACTER_ID_OFF as usize, idx as u32);
        arena.put_u32(addr + DOMAIN47_CCHARACTER_NAME_TOKEN_ID_OFF, idx as u32);
        arena.put_u32(
            addr + CCHARACTER_COUNTRY_TAG_DB_ID_OFF as usize,
            (idx % 200) as u32,
        );
        // Roughly a third of characters are unit leaders in the live world
        // (534 leaders adopted from 13_405 censused is far fewer, but the
        // pointer field is read unconditionally either way).
        let leader_ptr = if idx < T15_FRAME_UNIT_LEADERS {
            (unit_leader_base + idx * UNIT_LEADER_STRIDE) as u64
        } else {
            0
        };
        arena.put_u64(addr + CCHARACTER_UNIT_LEADER_PTR_OFF as usize, leader_ptr);
        // Half SSO, half heap: the live mix of short given names and long
        // "COUNTRY_forename_surname" tokens.
        let name = if idx % 2 == 0 {
            format!("Name {idx}")
        } else {
            format!("A Rather Longer Character Name {idx}")
        };
        let token = format!("GER_character_token_{idx}");
        arena.put_msvc_string(addr + CCHARACTER_NAME_OFF as usize, heap, &name);
        arena.put_msvc_string(addr + CCHARACTER_TOKEN_OFF as usize, heap + 0x40, &token);
        census_characters.push(addr);
        if idx < T15_FRAME_CHARACTERS {
            character_roots.push(CharacterRoot {
                character_addr: addr as u64,
                name: Some(name),
                token: Some(token),
            });
        }
    }

    let mut advisor_roots = Vec::with_capacity(T15_FRAME_ADVISORS);
    for idx in 0..T15_FRAME_ADVISORS {
        let addr = advisor_base + idx * ADVISOR_STRIDE;
        arena.put_u64(addr, BENCH_CADVISOR_VTABLE);
        let character = char_base + (idx % T15_FRAME_CHARACTERS) * CHARACTER_STRIDE;
        arena.put_u64(addr + CADVISOR_CHARACTER_PTR_OFF as usize, character as u64);
        advisor_roots.push(AdvisorRoot {
            advisor_addr: addr as u64,
            idea_token: format!("idea_token_{idx}"),
            slot: Some(format!("slot_{}", idx % 12)),
            ledger: Some("civilian".to_string()),
        });
    }

    let mut country_leader_roots = Vec::with_capacity(T15_FRAME_COUNTRY_LEADER_ROLES);
    for idx in 0..T15_FRAME_COUNTRY_LEADER_ROLES {
        let addr = country_leader_base + idx * COUNTRY_LEADER_STRIDE;
        arena.put_u64(addr, BENCH_CCOUNTRY_LEADER_VTABLE);
        let character = char_base + (idx % T15_FRAME_CHARACTERS) * CHARACTER_STRIDE;
        arena.put_u64(
            addr + CCOUNTRY_LEADER_CHARACTER_PTR_OFF as usize,
            character as u64,
        );
        country_leader_roots.push(CountryLeaderRoleRoot {
            country_leader_addr: addr as u64,
            country_tag_db_id: (idx % 200) as u32,
            name: Some(format!("Country Leader {idx}")),
            token: Some(format!("LEADER_ROLE_{idx}")),
        });
    }

    BenchWorld {
        _arena: arena,
        roots,
        census_characters,
        character_roots,
        unit_leader_roots,
        trait_roots,
        country_leader_roots,
        advisor_roots,
    }
}

/// The read path AS IT WAS before this lane: identical in every respect to
/// `Domain47InProcessMemory` except that it does not offer the one-call
/// object copy, so `LaneMemory::read_object_window` returns its `None` default and
/// every reader takes its field-by-field branch.
///
/// This exists so the before/after comparison happens inside ONE process,
/// alternating between the two paths. Comparing two separate `cargo test`
/// runs on this machine does not work: other lanes build and test
/// concurrently and the run-to-run spread was larger than the effect.
struct LegacyDomain47Memory;

impl domain47::extractor::LaneMemory for LegacyDomain47Memory {
    fn read_u8(&self, addr: u64) -> Option<u8> {
        Domain47InProcessMemory.read_u8(addr)
    }

    fn read_array<const N: usize>(&self, addr: u64) -> Option<[u8; N]> {
        Domain47InProcessMemory.read_array::<N>(addr)
    }

    // `read_object_window` deliberately left at the trait default (`None`).
}

/// The census loop body as it was before this lane: two by-address string
/// reads instead of one snapshot-served pair, and the field-by-field raw
/// field reader.
fn census_loop_legacy(world: &BenchWorld) -> usize {
    let mem = LegacyDomain47Memory;
    let mut sink = 0usize;
    for addr in &world.census_characters {
        let name_token_addr = *addr + DOMAIN47_CCHARACTER_NAME_TOKEN_ID_OFF;
        let opening = read_u32_stable(name_token_addr);
        let first = domain47::extractor::read_character_raw_fields(&mem, &world.roots, *addr as u64);
        let second =
            domain47::extractor::read_character_raw_fields(&mem, &world.roots, *addr as u64);
        let closing = read_u32_stable(name_token_addr);
        let name = domain47_read_msvc_display_string(
            *addr + domain47::extractor::CCHARACTER_NAME_OFF as usize,
            DOMAIN47_CCHARACTER_STRING_MAX_LEN,
        );
        let token = read_msvc_string(
            *addr + domain47::extractor::CCHARACTER_TOKEN_OFF as usize,
            DOMAIN47_CCHARACTER_STRING_MAX_LEN,
        );
        sink += opening.is_some() as usize
            + closing.is_some() as usize
            + first.is_ok() as usize
            + second.is_ok() as usize
            + name.map_or(0, |v| v.len())
            + token.map_or(0, |v| v.len());
    }
    sink
}

/// The census loop body, verbatim from
/// `domain47_read_exact_manager_roster` + the prewarm's root build: the
/// bracketing stable name-token reads, the torn-detecting double read of
/// the raw fields, and the two per-character strings.
fn census_loop(world: &BenchWorld) -> usize {
    let mem = Domain47InProcessMemory;
    let mut sink = 0usize;
    for addr in &world.census_characters {
        let name_token_addr = *addr + DOMAIN47_CCHARACTER_NAME_TOKEN_ID_OFF;
        let opening = read_u32_stable(name_token_addr);
        let first = domain47::extractor::read_character_raw_fields(&mem, &world.roots, *addr as u64);
        let second =
            domain47::extractor::read_character_raw_fields(&mem, &world.roots, *addr as u64);
        let closing = read_u32_stable(name_token_addr);
        let (name, token) = domain47_character_root_strings(*addr);
        sink += opening.is_some() as usize
            + closing.is_some() as usize
            + first.is_ok() as usize
            + second.is_ok() as usize
            + name.map_or(0, |v| v.len())
            + token.map_or(0, |v| v.len());
    }
    sink
}

/// The production frame walk, over t15's adopted root counts. `countries`
/// is empty: the active-ruler chain is 171 of 9_928 roots and its fixture
/// is not built here. Excluded, not sampled away — the character, advisor,
/// unit-leader and country-leader loops are the 98.3% this lane targets.
fn frame_walk(world: &BenchWorld) -> usize {
    frame_walk_with(world, &Domain47InProcessMemory)
}

fn bench_input<'a>(world: &'a BenchWorld) -> AdapterInputRef<'a> {
    AdapterInputRef {
        elapsed_wall_ms: 0,
        status_flags: 0,
        skipped_input_rows: 0,
        characters: &world.character_roots,
        unit_leaders: &world.unit_leader_roots,
        unit_leader_trait_tokens: &world.trait_roots,
        country_leader_roles: &world.country_leader_roots,
        countries: &[],
        advisors: &world.advisor_roots,
        portraits: &[],
    }
}

fn build_bench_frame<M: domain47::extractor::LaneMemory>(
    world: &BenchWorld,
    mem: &M,
) -> domain47::adapter::AdapterBuildResult {
    domain47::adapter::build_lane_frame_from_adapter_input_ref(mem, &world.roots, &bench_input(world))
}

fn frame_walk_with<M: domain47::extractor::LaneMemory>(world: &BenchWorld, mem: &M) -> usize {
    let result = build_bench_frame(world, mem);
    assert_eq!(
        result.stats.rejected_roots, 0,
        "bench fixture must not reject roots"
    );
    assert_eq!(result.stats.dead_roots, 0, "bench fixture must not be dead");
    result.frame.characters.len()
        + result.frame.unit_leaders.len()
        + result.frame.unit_leader_traits.len()
        + result.frame.country_leader_roles.len()
        + result.frame.advisor_roster.len()
}
/// Frames per timed run. At 20 the run-to-run spread on the smaller
/// sub-loops was wider than the effects being measured; at 200 the spread
/// is a few percent and a real change is unambiguous.
const BENCH_ITERATIONS: usize = 200;

/// Report the MEDIAN of several timed runs, not one run. A single run picks
/// up whatever else this machine was doing (other lanes build and test
/// concurrently), and the mean lets one such run dominate.
fn time_us_median<F: FnMut() -> usize>(runs: usize, mut body: F) -> (f64, usize) {
    let mut samples = Vec::with_capacity(runs);
    let mut sink = 0usize;
    for _ in 0..runs {
        let (us, s) = time_us(BENCH_ITERATIONS, &mut body);
        samples.push(us);
        sink = sink.wrapping_add(s);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).expect("no NaN timings"));
    (samples[runs / 2], sink)
}

fn time_us<F: FnMut() -> usize>(iterations: usize, mut body: F) -> (f64, usize) {
    // One warm pass so the region cache and the page working set are hot,
    // exactly as they are mid-capture.
    page_cache_new_frame();
    let mut sink = body();
    let started = Instant::now();
    for _ in 0..iterations {
        // The collector bumps the page-cache epoch once per frame, so the
        // region cache starts every frame cold. Model that.
        page_cache_new_frame();
        sink += body();
    }
    let elapsed = started.elapsed();
    (
        elapsed.as_nanos() as f64 / iterations as f64 / 1000.0,
        sink,
    )
}

/// Run two variants ALTERNATELY and report the median of each.
///
/// Taking one variant's five runs and then the other's does not survive this
/// machine: other lanes build and test at the same time, so load drifts
/// during the test and whichever variant ran while it was busy looks worse.
/// Measured swings of tens of percent, including sign flips, came from that
/// alone. Interleaving run-by-run means both variants see the same load
/// profile, and the median then discards the runs that were unlucky.
fn time_us_median_ab<A: FnMut() -> usize, B: FnMut() -> usize>(
    runs: usize,
    mut a: A,
    mut b: B,
) -> (f64, f64) {
    let mut a_samples = Vec::with_capacity(runs);
    let mut b_samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        a_samples.push(time_us(BENCH_ITERATIONS, &mut a).0);
        b_samples.push(time_us(BENCH_ITERATIONS, &mut b).0);
    }
    a_samples.sort_by(|x, y| x.partial_cmp(y).expect("no NaN timings"));
    b_samples.sort_by(|x, y| x.partial_cmp(y).expect("no NaN timings"));
    (a_samples[runs / 2], b_samples[runs / 2])
}

/// In-process A/B of the read-path change, on both loops.
///
/// `legacy` is the field-by-field reader this lane replaced; `snapshot` is
/// the one-call form. Same process, same fixture, same iteration count,
/// alternating run by run, medians reported — so the difference is the code
/// and not what else this machine was running.
///
/// This isolates ONE of the lane's changes. The legacy arm still contains
/// the lane's other changes (the frame-walk bookkeeping, the halved string
/// allocations), because those are structural and cannot be swapped through
/// a memory implementation. `d47_cost_removed_frame_walk_bookkeeping`
/// measures what the bookkeeping cost separately.
#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_read_path_ab() {
    let world = bench_world();
    let legacy_mem = LegacyDomain47Memory;
    let snapshot_mem = Domain47InProcessMemory;

    let (census_legacy, census_now) = time_us_median_ab(
        5,
        || census_loop_legacy(&world),
        || census_loop(&world),
    );
    println!(
        "d47_cost AB census: legacy {census_legacy:.1} us -> snapshot {census_now:.1} us \
         ({:+.1}%) over {T15_CENSUS_CHARACTERS} characters",
        (census_now - census_legacy) / census_legacy * 100.0
    );

    let (walk_legacy, walk_now) = time_us_median_ab(
        5,
        || frame_walk_with(&world, &legacy_mem),
        || frame_walk_with(&world, &snapshot_mem),
    );
    println!(
        "d47_cost AB frame_walk: legacy {walk_legacy:.1} us -> snapshot {walk_now:.1} us ({:+.1}%)",
        (walk_now - walk_legacy) / walk_legacy * 100.0
    );
    println!(
        "d47_cost AB total (census + frame walk): legacy {:.1} us -> snapshot {:.1} us ({:+.1}%)",
        census_legacy + walk_legacy,
        census_now + walk_now,
        (census_now + walk_now - census_legacy - walk_legacy) / (census_legacy + walk_legacy)
            * 100.0
    );
}

#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_census_loop() {
    let world = bench_world();
    let (us, sink) = time_us_median(5, || census_loop(&world));
    println!(
        "d47_cost census_loop: {:.1} us/frame over {} characters ({:.1} ns/character) sink={}",
        us,
        T15_CENSUS_CHARACTERS,
        us * 1000.0 / T15_CENSUS_CHARACTERS as f64,
        sink
    );
}

#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_frame_walk() {
    let world = bench_world();
    let (us, sink) = time_us_median(5, || frame_walk(&world));
    println!(
        "d47_cost frame_walk: {:.1} us/frame over {} chars + {} advisors + {} leaders + {} roles sink={}",
        us,
        T15_FRAME_CHARACTERS,
        T15_FRAME_ADVISORS,
        T15_FRAME_UNIT_LEADERS,
        T15_FRAME_COUNTRY_LEADER_ROLES,
        sink
    );
}

/// The two read paths must not merely cost differently — they must produce
/// the SAME frame. Same fixture, same roots, both memories, byte-for-byte
/// equal `LaneFrame` and equal build stats.
///
/// MUTATION: make `character_raw_fields_from_body` read `char_id` from
/// `CCHARACTER_COUNTRY_TAG_DB_ID_OFF` (or swap any two field offsets) and
/// this fails; the cost benchmarks above keep passing.
#[test]
fn d47_snapshot_and_field_by_field_reads_build_the_same_frame() {
    let world = bench_world();
    page_cache_new_frame();
    let legacy = build_bench_frame(&world, &LegacyDomain47Memory);
    page_cache_new_frame();
    let snapshot = build_bench_frame(&world, &Domain47InProcessMemory);
    assert_eq!(
        legacy.stats, snapshot.stats,
        "one-call object reads changed the adapter build stats"
    );
    assert_eq!(
        legacy.frame, snapshot.frame,
        "one-call object reads changed the emitted frame"
    );
    // A fixture that emitted nothing would make the assertions above pass
    // for the wrong reason.
    assert_eq!(legacy.stats.character_rows as usize, T15_FRAME_CHARACTERS);
    assert_eq!(legacy.stats.advisor_roster_rows as usize, T15_FRAME_ADVISORS);
    assert_eq!(
        legacy.stats.unit_leader_rows as usize,
        T15_FRAME_UNIT_LEADERS
    );
    assert_eq!(
        legacy.stats.country_leader_role_rows as usize,
        T15_FRAME_COUNTRY_LEADER_ROLES
    );
}

/// The token's heap pointer must be captured immediately before it is
/// followed, not before the name is read.
///
/// This is the interleaving the first review round found. The token is
/// heap-backed. Between the name read and the token read, the engine
/// repoints the token at a NEW buffer and the allocator hands the OLD buffer
/// to something else that happens to hold printable bytes. A reader that
/// captured the token's pointer before reading the name follows the dead
/// pointer, and every check it makes — size, capacity, NUL, ASCII, UTF-8,
/// length — passes on the recycled bytes. It publishes a well-formed WRONG
/// token, with no rejection and no PARTIAL flag, which is exactly the
/// silently-wrong-data outcome this project refuses.
///
/// MUTATION: in `domain47_character_root_strings_with_between`, hoist the
/// `token_header` snapshot above the `between()` call (i.e. capture both
/// headers up front, which is what the first implementation of this reader
/// did). This test then follows the dead pointer and fails with
/// `Some(RECYCLED_TOKEN)` in place of the token that was actually live.
#[test]
fn d47_token_pointer_is_captured_after_the_name_read_not_before() {
    // Same length as the token the stale header describes, all printable.
    const RECYCLED_TOKEN: &str = "SOV_unrelated_othr_token";
    let arena = BenchArena::new(0x4000);
    let character = arena.base;
    let old_buffer = arena.base + 0x1000;
    let new_buffer = arena.base + 0x2000;
    let name_buffer = arena.base + 0x3000;

    // A heap-backed name, so reading it is not a no-op, and a heap-backed
    // token pointing at `old_buffer`.
    arena.put_msvc_string(
        character + CCHARACTER_NAME_OFF as usize,
        name_buffer,
        "A Rather Longer Character Name",
    );
    arena.put_msvc_string(
        character + CCHARACTER_TOKEN_OFF as usize,
        old_buffer,
        "GER_original_token_value",
    );

    page_cache_new_frame();
    let (name, token) = domain47_character_root_strings_with_between(character, || {
        // The engine repoints the token at a new buffer...
        arena.put_msvc_string(
            character + CCHARACTER_TOKEN_OFF as usize,
            new_buffer,
            "GER_NEW_TOKEN_VALUE_here",
        );
        // ...and the allocator reuses the old one for unrelated bytes that
        // are perfectly printable and EXACTLY as long as the token the
        // stale header describes, so every validity check the reader makes
        // passes on them. That is what makes this hazard silent: the wrong
        // answer is well-formed, not absent.
        assert_eq!(RECYCLED_TOKEN.len(), "GER_original_token_value".len());
        arena.put_bytes(old_buffer, RECYCLED_TOKEN.as_bytes());
        arena.put_bytes(old_buffer + RECYCLED_TOKEN.len(), &[0u8]);
    });

    assert_eq!(name.as_deref(), Some("A Rather Longer Character Name"));
    assert_eq!(
        token.as_deref(),
        Some("GER_NEW_TOKEN_VALUE_here"),
        "the token must come from the pointer that was live when it was followed, \
         not from a pointer captured before the name read"
    );
}

/// The prewarm's snapshot-served `(name, token)` pair must equal what the
/// by-address readers return, for BOTH string forms — the fixture writes
/// short (SSO, characters inside the body) and long (heap-backed, characters
/// outside it) names, and the snapshot only covers the body.
///
/// MUTATION: in `domain47_read_msvc_display_string_with_snapshot`, take the
/// SSO branch unconditionally (drop the `cap < 16` test) and the long names
/// come back wrong or absent; this fails.
#[test]
fn d47_snapshot_strings_match_the_by_address_readers() {
    let world = bench_world();
    page_cache_new_frame();
    let mut sso = 0usize;
    let mut heap = 0usize;
    for addr in &world.census_characters {
        let (name, token) = domain47_character_root_strings(*addr);
        let legacy_name = domain47_read_msvc_display_string(
            *addr + domain47::extractor::CCHARACTER_NAME_OFF as usize,
            DOMAIN47_CCHARACTER_STRING_MAX_LEN,
        );
        let legacy_token = read_msvc_string(
            *addr + domain47::extractor::CCHARACTER_TOKEN_OFF as usize,
            DOMAIN47_CCHARACTER_STRING_MAX_LEN,
        );
        assert_eq!(name, legacy_name, "display name at {addr:#x}");
        assert_eq!(token, legacy_token, "script token at {addr:#x}");
        match legacy_name.as_deref() {
            Some(text) if text.len() < 16 => sso += 1,
            Some(_) => heap += 1,
            None => panic!("bench fixture name at {addr:#x} did not read"),
        }
    }
    // Both string forms must actually be present, or this test proves only
    // half of what it claims.
    assert!(sso > 0 && heap > 0, "fixture covered only one string form");
}

/// What the two frame-walk bookkeeping changes cost when they were still
/// there.
///
/// Neither can be A/B-ed through a memory trait — they are structural — so
/// instead this rebuilds, over the SAME character rows the walk produces,
/// exactly the two things that were deleted:
///
///  * the old identity map: a `BTreeMap` from `(country, char_id)` to a
///    CLONE of that character's name and token, built for every character
///    so that ~171 countries could backfill a ruler name;
///  * the `known_characters` set: an insert per character, per unit leader,
///    per role and per advisor, read only by the portrait filter.
///
/// Both were pure additive cost on top of the walk, so timing them in
/// isolation measures what removing them removed.
#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_removed_frame_walk_bookkeeping() {
    let world = bench_world();
    let rows = build_bench_frame(&world, &Domain47InProcessMemory).frame.characters;
    let identity_rows = rows.len();

    let (identity_us, _) = time_us_median(5, || {
        let mut map = std::collections::BTreeMap::<(u32, u32), (Option<String>, Option<String>)>::new();
        for row in &rows {
            map.insert(
                (row.country_tag_db_id, row.char_id),
                (row.name.clone(), row.token.clone()),
            );
        }
        map.len()
    });

    let known_inserts =
        identity_rows + T15_FRAME_UNIT_LEADERS + T15_FRAME_COUNTRY_LEADER_ROLES + T15_FRAME_ADVISORS;
    let (known_us, _) = time_us_median(5, || {
        let mut set = std::collections::BTreeSet::<(u32, u32)>::new();
        for row in &rows {
            set.insert((row.country_tag_db_id, row.char_id));
        }
        for idx in 0..(T15_FRAME_UNIT_LEADERS + T15_FRAME_COUNTRY_LEADER_ROLES + T15_FRAME_ADVISORS)
        {
            set.insert(((idx % 200) as u32, idx as u32));
        }
        set.len()
    });

    println!(
        "d47_cost removed bookkeeping: identity map with cloned strings {identity_us:.1} us/frame \
         over {identity_rows} rows; known_characters set {known_us:.1} us/frame over \
         {known_inserts} inserts; total {:.1} us/frame",
        identity_us + known_us
    );
}

/// How much of the frame walk is `String` cloning?
///
/// Runs the identical walk twice: once over the real roots, once over roots
/// whose ancillary strings are all absent. The strings are read by the
/// PREWARM and merely copied here, so the difference is the per-frame cost
/// of copying already-known text into the frame — nothing to do with how
/// much is captured. This is a diagnostic, not a proposal to drop strings.
#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_frame_walk_string_share() {
    let world = bench_world();
    let mem = Domain47InProcessMemory;
    let stringless_characters: Vec<CharacterRoot> = world
        .character_roots
        .iter()
        .map(|root| CharacterRoot {
            character_addr: root.character_addr,
            name: None,
            token: None,
        })
        .collect();
    let stringless_advisors: Vec<AdvisorRoot> = world
        .advisor_roots
        .iter()
        .map(|root| AdvisorRoot {
            advisor_addr: root.advisor_addr,
            idea_token: String::new(),
            slot: None,
            ledger: None,
        })
        .collect();
    let base = AdapterInputRef {
        elapsed_wall_ms: 0,
        status_flags: 0,
        skipped_input_rows: 0,
        characters: &world.character_roots,
        unit_leaders: &[],
        unit_leader_trait_tokens: &[],
        country_leader_roles: &[],
        countries: &[],
        advisors: &world.advisor_roots,
        portraits: &[],
    };
    let stringless = AdapterInputRef {
        characters: &stringless_characters,
        advisors: &stringless_advisors,
        ..base
    };
    for (name, input) in [("with_strings", base), ("without_strings", stringless)] {
        let (us, sink) = time_us_median(5, || {
            let r = domain47::adapter::build_lane_frame_from_adapter_input_ref(
                &mem,
                &world.roots,
                &input,
            );
            r.frame.characters.len() + r.frame.advisor_roster.len()
        });
        println!("d47_cost frame_walk_strings/{name}: {us:.1} us/frame sink={sink}");
    }
}

/// Isolate the four frame-walk sub-loops by running the walk with one root
/// family at a time, so a change can be attributed instead of guessed at.
#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_frame_walk_by_family() {
    let world = bench_world();
    let mem = Domain47InProcessMemory;
    let empty = AdapterInputRef {
        elapsed_wall_ms: 0,
        status_flags: 0,
        skipped_input_rows: 0,
        characters: &[],
        unit_leaders: &[],
        unit_leader_trait_tokens: &[],
        country_leader_roles: &[],
        countries: &[],
        advisors: &[],
        portraits: &[],
    };
    let families: [(&str, AdapterInputRef<'_>); 4] = [
        (
            "characters",
            AdapterInputRef {
                characters: &world.character_roots,
                ..empty
            },
        ),
        (
            "advisors",
            AdapterInputRef {
                advisors: &world.advisor_roots,
                ..empty
            },
        ),
        (
            "unit_leaders",
            AdapterInputRef {
                unit_leaders: &world.unit_leader_roots,
                unit_leader_trait_tokens: &world.trait_roots,
                ..empty
            },
        ),
        (
            "country_leader_roles",
            AdapterInputRef {
                country_leader_roles: &world.country_leader_roots,
                ..empty
            },
        ),
    ];
    for (name, input) in families {
        let (us, sink) = time_us_median(5, || {
            let r =
                domain47::adapter::build_lane_frame_from_adapter_input_ref(&mem, &world.roots, &input);
            r.frame.characters.len()
                + r.frame.unit_leaders.len()
                + r.frame.advisor_roster.len()
                + r.frame.country_leader_roles.len()
        });
        println!("d47_cost frame_walk/{name}: {us:.1} us/frame sink={sink}");
    }
}
