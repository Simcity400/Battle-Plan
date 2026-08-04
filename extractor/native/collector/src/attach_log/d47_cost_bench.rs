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

    /// An arena guaranteed to sit inside `[HEAP_LO, HEAP_HI)`.
    ///
    /// The politics chain gates every pointer through
    /// `is_map_control_heap_pointer`, so a fixture the allocator happened to
    /// place outside that window would be refused before a single field was
    /// read and every measurement would be of the refusal. Same fixed-base
    /// retry loop `critical_path_tests` uses.
    fn in_heap_window(size: usize) -> BenchArena {
        for slot in 0..64usize {
            let want = (HEAP_LO + 0x4000_0000 + slot * size.next_power_of_two()) as *mut c_void;
            let got = unsafe {
                VirtualAlloc(
                    want,
                    size,
                    BENCH_MEM_RESERVE | BENCH_MEM_COMMIT_FLAG,
                    BENCH_PAGE_READWRITE,
                )
            };
            if !got.is_null() {
                assert!(is_map_control_heap_pointer(got as usize));
                return BenchArena {
                    base: got as usize,
                    size,
                };
            }
        }
        panic!("could not reserve a bench arena inside the heap window");
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

// ---------------------------------------------------------------------------
// The prewarm's OTHER string readers (lane X, 2026-08-04).
//
// Lane Q gave the ~13_405 censused `CCharacter` bodies the one-snapshot-per-
// string treatment. The rest of the prewarm's adopted objects — ~534
// `CUnitLeader`, ~3_358 `CAdvisor` and the `CUnitLeaderTrait` definitions —
// still read their strings by address, and the advisor probe was the worst of
// them: it read the idea token's 32-byte header as FOUR separate guarded
// `read_u64`s and then `read_msvc_string` re-read two or three of the same
// words. The country-leader roles are not in this list because they read no
// strings at all (`name: None, token: None`).
// ---------------------------------------------------------------------------

/// `domain47_prewarm_advisor_root_count` / `domain47_prewarm_ran` at t15.
const T15_TAIL_ADVISORS: usize = T15_FRAME_ADVISORS;
const T15_TAIL_UNIT_LEADERS: usize = T15_FRAME_UNIT_LEADERS;
const T15_TAIL_TRAITS: usize = T15_FRAME_TRAIT_TOKENS;

const TAIL_ADVISOR_STRIDE: usize = 0x200;
const TAIL_UNIT_LEADER_STRIDE: usize = 0x200;
const TAIL_TRAIT_STRIDE: usize = 0x100;

struct TailBenchWorld {
    _arena: BenchArena,
    advisors: Vec<usize>,
    unit_leaders: Vec<usize>,
    traits: Vec<usize>,
}

fn tail_bench_world() -> TailBenchWorld {
    let advisor_bytes = T15_TAIL_ADVISORS * TAIL_ADVISOR_STRIDE;
    let leader_bytes = T15_TAIL_UNIT_LEADERS * TAIL_UNIT_LEADER_STRIDE;
    let trait_bytes = T15_TAIL_TRAITS * TAIL_TRAIT_STRIDE;
    let arena = BenchArena::new(advisor_bytes + leader_bytes + trait_bytes + 0x10000);
    let advisor_base = arena.base;
    let leader_base = advisor_base + advisor_bytes;
    let trait_base = leader_base + leader_bytes;

    // Half SSO, half heap-backed, as the live mix is.
    let token_for = |idx: usize, kind: &str| {
        if idx % 2 == 0 {
            format!("{kind}_{idx}")
        } else {
            format!("GER_a_rather_longer_{kind}_token_{idx}")
        }
    };

    let mut advisors = Vec::with_capacity(T15_TAIL_ADVISORS);
    for idx in 0..T15_TAIL_ADVISORS {
        let addr = advisor_base + idx * TAIL_ADVISOR_STRIDE;
        arena.put_u64(addr, BENCH_CADVISOR_VTABLE);
        arena.put_msvc_string(
            addr + domain47::extractor::CADVISOR_IDEA_TOKEN_OFF as usize,
            addr + 0x100,
            &token_for(idx, "idea"),
        );
        arena.put_msvc_string(
            addr + domain47::extractor::CADVISOR_SLOT_OFF as usize,
            addr + 0x180,
            &token_for(idx, "slot"),
        );
        advisors.push(addr);
    }

    let mut unit_leaders = Vec::with_capacity(T15_TAIL_UNIT_LEADERS);
    for idx in 0..T15_TAIL_UNIT_LEADERS {
        let addr = leader_base + idx * TAIL_UNIT_LEADER_STRIDE;
        arena.put_msvc_string(
            addr + domain47::extractor::CUNIT_LEADER_TOKEN_OFF as usize,
            addr + 0x100,
            &token_for(idx, "leader"),
        );
        arena.put_msvc_string(
            addr + domain47::extractor::CUNIT_LEADER_NAME_OFF as usize,
            addr + 0x180,
            &token_for(idx, "Leader Name"),
        );
        unit_leaders.push(addr);
    }

    let mut traits = Vec::with_capacity(T15_TAIL_TRAITS);
    for idx in 0..T15_TAIL_TRAITS {
        let addr = trait_base + idx * TAIL_TRAIT_STRIDE;
        arena.put_msvc_string(
            addr + domain47::extractor::CUNIT_LEADER_TRAIT_TOKEN_OFF as usize,
            addr + 0x80,
            &token_for(idx, "trait"),
        );
        traits.push(addr);
    }

    TailBenchWorld {
        _arena: arena,
        advisors,
        unit_leaders,
        traits,
    }
}

/// The three loop bodies AS THEY WERE: by-address string reads throughout, and
/// the advisor's header read one guarded word at a time.
fn tail_loop_legacy(world: &TailBenchWorld) -> usize {
    let mut sink = 0usize;
    for addr in &world.unit_leaders {
        let name = domain47_read_msvc_display_string(
            addr + domain47::extractor::CUNIT_LEADER_NAME_OFF as usize,
            160,
        );
        let token =
            read_msvc_string(addr + domain47::extractor::CUNIT_LEADER_TOKEN_OFF as usize, 160);
        sink += name.map_or(0, |v| v.len()) + token.map_or(0, |v| v.len());
    }
    for addr in &world.advisors {
        let token_off = addr + domain47::extractor::CADVISOR_IDEA_TOKEN_OFF as usize;
        let header = (
            read_u64(token_off).unwrap_or(u64::MAX),
            read_u64(token_off + 0x08).unwrap_or(u64::MAX),
            read_u64(token_off + 0x10).unwrap_or(u64::MAX),
            read_u64(token_off + 0x18).unwrap_or(u64::MAX),
        );
        let idea_token = read_msvc_string(token_off, 160);
        let slot = read_msvc_string(addr + domain47::extractor::CADVISOR_SLOT_OFF as usize, 160);
        sink += (header.0 != u64::MAX) as usize
            + idea_token.map_or(0, |v| v.len())
            + slot.map_or(0, |v| v.len());
    }
    for addr in &world.traits {
        let first = read_msvc_string(
            addr + domain47::extractor::CUNIT_LEADER_TRAIT_TOKEN_OFF as usize,
            160,
        );
        let second = read_msvc_string(
            addr + domain47::extractor::CUNIT_LEADER_TRAIT_TOKEN_OFF as usize,
            160,
        );
        sink += (first == second) as usize + first.map_or(0, |v| v.len());
    }
    sink
}

/// The same three loop bodies as they are now, through the production helpers.
fn tail_loop_now(world: &TailBenchWorld) -> usize {
    let roots = ResolvedTypeRoots {
        cadvisor_vtable: BENCH_CADVISOR_VTABLE,
        ..ResolvedTypeRoots::default()
    };
    let mut sink = 0usize;
    for addr in &world.unit_leaders {
        let (name, token) = domain47_unit_leader_root_strings(*addr);
        sink += name.map_or(0, |v| v.len()) + token.map_or(0, |v| v.len());
    }
    for addr in &world.advisors {
        let probe = domain47_probe_advisor(&Domain47InProcessMemory, &roots, *addr as u64);
        sink += probe.token_header.is_some() as usize
            + probe.idea_token.map_or(0, |v| v.len())
            + probe.slot.map_or(0, |v| v.len());
    }
    for addr in &world.traits {
        let stable = domain47_trait_token_stable(*addr);
        sink += stable.is_some() as usize + stable.flatten().map_or(0, |v| v.len());
    }
    sink
}

/// In-process, run-by-run interleaved A/B of the prewarm's non-character
/// string readers.
#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_prewarm_tail_ab() {
    let world = tail_bench_world();
    // Both arms must actually resolve their strings, or this measures two
    // refusal paths against each other.
    page_cache_new_frame();
    let legacy_sink = tail_loop_legacy(&world);
    page_cache_new_frame();
    let now_sink = tail_loop_now(&world);
    assert!(legacy_sink > 0 && now_sink > 0, "fixture resolved nothing");
    // The two arms must read the SAME strings, not merely both read something:
    // an arm whose readers all refuse is fast and worthless.
    assert_eq!(
        legacy_sink, now_sink,
        "the two arms did not resolve the same strings"
    );

    let (legacy, now) =
        time_us_median_ab(5, || tail_loop_legacy(&world), || tail_loop_now(&world));
    println!(
        "d47_cost AB prewarm_tail: legacy {legacy:.1} us -> snapshot {now:.1} us ({:+.1}%) over \
         {T15_TAIL_UNIT_LEADERS} leaders + {T15_TAIL_ADVISORS} advisors + {T15_TAIL_TRAITS} traits",
        (now - legacy) / legacy * 100.0
    );
}

/// A `CUnitLeader` / `CUnitLeaderTrait` / `CAdvisor` body with two heap-backed
/// strings, laid out so a test can free-and-recycle either buffer.
///
/// `first_off` / `second_off` are the two `std::string` offsets, in the order
/// the reader visits them.
struct RecycleFixture {
    arena: BenchArena,
    object: usize,
    first_off: usize,
    second_off: usize,
    first_buffer: usize,
    second_old_buffer: usize,
    second_new_buffer: usize,
}

/// Same length as the token the stale header describes, and all printable, so
/// every validity check the reader makes — size, capacity, NUL, ASCII, UTF-8,
/// length — passes on the recycled bytes. That is what makes the hazard
/// silent: the wrong answer is well-formed, not absent.
const RECYCLED_SECOND: &str = "SOV_unrelated_othr_token";
const ORIGINAL_SECOND: &str = "GER_original_token_value";
const LIVE_SECOND: &str = "GER_NEW_TOKEN_VALUE_here";
const ORIGINAL_FIRST: &str = "A Rather Longer First String";

impl RecycleFixture {
    fn new(first_off: usize, second_off: usize) -> RecycleFixture {
        let arena = BenchArena::new(0x8000);
        let object = arena.base;
        let first_buffer = arena.base + 0x1000;
        let second_old_buffer = arena.base + 0x2000;
        let second_new_buffer = arena.base + 0x3000;
        arena.put_msvc_string(object + first_off, first_buffer, ORIGINAL_FIRST);
        arena.put_msvc_string(object + second_off, second_old_buffer, ORIGINAL_SECOND);
        RecycleFixture {
            arena,
            object,
            first_off,
            second_off,
            first_buffer,
            second_old_buffer,
            second_new_buffer,
        }
    }

    /// The engine repoints the SECOND string at a new buffer, and the
    /// allocator hands the old one to something else holding printable bytes.
    fn recycle_second(&self) {
        self.arena
            .put_msvc_string(self.object + self.second_off, self.second_new_buffer, LIVE_SECOND);
        assert_eq!(RECYCLED_SECOND.len(), ORIGINAL_SECOND.len());
        self.arena.put_bytes(self.second_old_buffer, RECYCLED_SECOND.as_bytes());
        self.arena
            .put_bytes(self.second_old_buffer + RECYCLED_SECOND.len(), &[0u8]);
    }

    /// The same, for the FIRST string — used where the two reads are of one
    /// string rather than two.
    fn recycle_first(&self, replacement: &str) {
        let new_buffer = self.arena.base + 0x4000;
        self.arena
            .put_msvc_string(self.object + self.first_off, new_buffer, replacement);
        // The recycled bytes are read through the STALE header, so they must
        // be exactly as long as the string that header describes — otherwise
        // the length check rejects them and the hazard stops being silent.
        assert_eq!(RECYCLED_FIRST.len(), ORIGINAL_FIRST.len());
        self.arena.put_bytes(self.first_buffer, RECYCLED_FIRST.as_bytes());
        self.arena
            .put_bytes(self.first_buffer + RECYCLED_FIRST.len(), &[0u8]);
    }
}

const RECYCLED_FIRST: &str = "A Rather Longer Other String";

/// A unit leader's token pointer must be captured after the NAME read, not
/// before it — the same hazard `d47_token_pointer_is_captured_after_the_name_read_not_before`
/// pins for character roots, now that unit leaders take the same treatment.
///
/// MUTATION: in `domain47_unit_leader_root_strings_with_between`, hoist the
/// token's `ObjectSnapshot` above the `between()` call — i.e. inline
/// `domain47_snapshot_msvc_token` and capture both headers up front. The token
/// then follows the dead pointer and this fails with `Some(RECYCLED_SECOND)`.
#[test]
fn d47_unit_leader_token_pointer_is_captured_after_the_name_read() {
    let fixture = RecycleFixture::new(
        domain47::extractor::CUNIT_LEADER_NAME_OFF as usize,
        domain47::extractor::CUNIT_LEADER_TOKEN_OFF as usize,
    );
    page_cache_new_frame();
    let (name, token) =
        domain47_unit_leader_root_strings_with_between(fixture.object, || fixture.recycle_second());
    assert_eq!(name.as_deref(), Some(ORIGINAL_FIRST));
    assert_eq!(
        token.as_deref(),
        Some(LIVE_SECOND),
        "the token must come from the pointer that was live when it was followed"
    );
}

/// An advisor's SLOT pointer must be captured after the idea token has been
/// read, not before it.
///
/// MUTATION: in `domain47_probe_advisor_with_between`, move the `between()`
/// call below the slot read (i.e. capture the slot's header while the idea
/// token is still being read). The slot then follows the dead pointer and this
/// fails with `Some(RECYCLED_SECOND)`.
#[test]
fn d47_advisor_slot_pointer_is_captured_after_the_idea_token_read() {
    let fixture = RecycleFixture::new(
        domain47::extractor::CADVISOR_IDEA_TOKEN_OFF as usize,
        domain47::extractor::CADVISOR_SLOT_OFF as usize,
    );
    // A CAdvisor body whose character chain is absent: this test is about the
    // two strings, and `domain47_probe_advisor` reads them whatever the chain
    // does.
    let roots = ResolvedTypeRoots::default();
    page_cache_new_frame();
    let probe = domain47_probe_advisor_with_between(
        &Domain47InProcessMemory,
        &roots,
        fixture.object as u64,
        || fixture.recycle_second(),
    );
    assert_eq!(probe.idea_token.as_deref(), Some(ORIGINAL_FIRST));
    assert_eq!(
        probe.slot.as_deref(),
        Some(LIVE_SECOND),
        "the slot must come from the pointer that was live when it was followed"
    );
}

/// A trait definition's token is read TWICE and the two must agree. Each read
/// needs its OWN header: serving both from one snapshot would compare a value
/// with itself, and the tear check could never fail.
///
/// MUTATION: in `domain47_trait_token_stable_with_between`, replace the second
/// `domain47_snapshot_msvc_token(...)` with `first.clone()` — the check
/// becomes `first == first`, the mutated string is accepted, and this fails
/// because it expects a refusal.
#[test]
fn d47_trait_token_tear_check_uses_two_independent_observations() {
    let fixture = RecycleFixture::new(
        domain47::extractor::CUNIT_LEADER_TRAIT_TOKEN_OFF as usize,
        // Unused by this site; point it somewhere harmless inside the body.
        domain47::extractor::CUNIT_LEADER_TRAIT_TOKEN_OFF as usize + 0x40,
    );
    // CONTROL: undisturbed, the two observations agree and the token reads.
    page_cache_new_frame();
    assert_eq!(
        domain47_trait_token_stable(fixture.object),
        Some(Some(ORIGINAL_FIRST.to_string())),
        "control read must succeed, or the refusal below proves nothing"
    );

    // The definition's token changes between the two observations.
    page_cache_new_frame();
    let torn = domain47_trait_token_stable_with_between(fixture.object, || {
        fixture.recycle_first("A Rather Longer Live String")
    });
    assert_eq!(
        torn, None,
        "a token that changed between the two observations must be reported TORN"
    );
}

/// The three new snapshot-served sites must return exactly what the
/// by-address readers they replaced return — both string forms.
///
/// MUTATION: in `domain47_snapshot_msvc_token`, pass `None` for the snapshot
/// and it still passes (the fallback IS the by-address reader). Instead: in
/// `read_msvc_string_with_snapshot`, take the SSO branch unconditionally (drop
/// the `cap < 16` test) — heap-backed strings come back wrong and this fails.
#[test]
fn d47_snapshot_helpers_match_the_by_address_readers() {
    let arena = BenchArena::new(0x8000);
    let heap = arena.base + 0x4000;
    let mut sso = 0usize;
    let mut heap_backed = 0usize;
    for (idx, text) in [
        "short",
        "A Rather Longer String That Cannot Be Inline",
        "x",
        "GER_character_token_12345678",
    ]
    .iter()
    .enumerate()
    {
        let addr = arena.base + idx * 0x40;
        arena.put_msvc_string(addr, heap + idx * 0x80, text);
        page_cache_new_frame();
        assert_eq!(
            domain47_snapshot_msvc_token(addr, DOMAIN47_CCHARACTER_STRING_MAX_LEN),
            read_msvc_string(addr, DOMAIN47_CCHARACTER_STRING_MAX_LEN),
            "token at {addr:#x}"
        );
        assert_eq!(
            domain47_snapshot_msvc_display_string(addr, DOMAIN47_CCHARACTER_STRING_MAX_LEN),
            domain47_read_msvc_display_string(addr, DOMAIN47_CCHARACTER_STRING_MAX_LEN),
            "display string at {addr:#x}"
        );
        if text.len() < 16 {
            sso += 1;
        } else {
            heap_backed += 1;
        }
    }
    assert!(sso > 0 && heap_backed > 0, "covered only one string form");
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

// ---------------------------------------------------------------------------
// The country-politics half of d47's critical span (lane X, 2026-08-04).
//
// `domain47_country_politics_roots` walks every country in the authoritative
// roster once per frame. Before this lane it took TWO complete
// `Domain47CountryPoliticsSnapshot`s per country and compared them for the
// tear check, and each of those walked the country's whole active-idea vector
// — per entry a heap-pointer gate, a vtable read, a
// `read_stable_msvc_string_snapshot` (itself two full string reads) and a
// global `IDEA_TOKEN_CACHE` mutex. Every idea token was therefore read FOUR
// times per country per frame.
//
// The fixture below is the shape `critical_path_tests` already uses for the
// politics chain, replicated across the whole roster and given a real
// active-idea vector. It lives inside `[HEAP_LO, HEAP_HI)` because every
// pointer in this chain is gated on that window.
// ---------------------------------------------------------------------------

/// Countries in the authoritative roster. The lane's own live evidence (see
/// `CPOLITICAL_PARTY_LEADER_VEC_OFF` in the extractor) records 437 countries
/// probed on build 870ebf.
const T15_POLITICS_COUNTRIES: usize = 437;
/// Active ideas per country: national spirits, the active laws, idea groups
/// and the appointed advisor ideas. This is the one number here that is an
/// ESTIMATE rather than a measurement — no t15 manifest field records it. The
/// A/B RATIO is what transfers and both arms scale with this, so the estimate
/// moves the absolute microseconds and not the conclusion.
const T15_IDEAS_PER_COUNTRY: usize = 12;
/// Distinct tokens per country. The fixture repeats one token from a SECOND
/// idea object so the `dedup` in `domain47_compose_politics_snapshot` has
/// something to remove — with every token distinct it would be unobservable.
const T15_DISTINCT_IDEAS_PER_COUNTRY: usize = T15_IDEAS_PER_COUNTRY - 1;

const POLITICS_COUNTRY_STRIDE: usize = 0x2000;
/// Room for the idea body (its vtable and the 32-byte `std::string` at
/// `IDEA_ID`, i.e. through `0x38`) AND a separate block for a heap-backed
/// token's characters. They must not overlap: a heap string's bytes written
/// inside its own header get overwritten by the size and capacity words, and
/// the reader then refuses the token — which would have made both arms measure
/// the refusal path.
const POLITICS_IDEA_STRIDE: usize = 0x80;
const POLITICS_IDEA_TOKEN_BYTES: usize = 0x40;

struct PoliticsBenchWorld {
    arena: BenchArena,
    module_base: usize,
    country_vtable: usize,
    rows: Vec<CountryObjectRow>,
    /// Unused arena space after the last country, for objects a test must
    /// build that are deliberately NOT part of any country's vector.
    spare: usize,
}

fn politics_bench_world() -> PoliticsBenchWorld {
    let size = T15_POLITICS_COUNTRIES * POLITICS_COUNTRY_STRIDE + 0x10000;
    let arena = BenchArena::in_heap_window(size);
    let base = arena.base;
    // The politics reader derives `module_base` by subtracting
    // `COUNTRY_VTABLE_RVA` from the country vtable and then only uses it to
    // form other vtable constants, which are compared against stored values.
    // So the fixture's own base serves, exactly as in `critical_path_tests`.
    let module_base = base;
    let country_vtable = module_base + COUNTRY_VTABLE_RVA;
    let status_vtable = module_base + POLITICAL_STATUS_VTABLE_RVA;
    let party_vtable = module_base + POLITICAL_PARTY_VTABLE_RVA;
    let group_vtable = module_base + IDEOLOGY_GROUP_VTABLE_RVA;
    let leader_vtable = module_base + COUNTRY_LEADER_VTABLE_RVA;
    let character_vtable = module_base + POLITICS_CHARACTER_VTABLE_RVA;
    let idea_vtable_primary = module_base + IDEA_VTABLE_RVA_PRIMARY;
    let idea_vtable_secondary = module_base + IDEA_VTABLE_RVA_SECONDARY;

    // Fixture-local offsets within one country's slot.
    const COUNTRY: usize = 0x0000;
    const STATUS: usize = 0x0200;
    const PARTY: usize = 0x0400;
    const GROUP: usize = 0x0600;
    const GROUP_TOKEN_BYTES: usize = 0x0700;
    const LEADER_VEC: usize = 0x0800;
    const COUNTRY_LEADER: usize = 0x0900;
    const CHARACTER: usize = 0x0A00;
    const RULER_NAME_BYTES: usize = 0x0C00;
    const RULER_TOKEN_BYTES: usize = 0x0D00;
    const IDEA_PTRS: usize = 0x0E00;
    const IDEAS: usize = 0x1000;

    let mut rows = Vec::with_capacity(T15_POLITICS_COUNTRIES);
    for idx in 0..T15_POLITICS_COUNTRIES {
        let slot = base + idx * POLITICS_COUNTRY_STRIDE;
        let tag_db_id = (idx + 1) as u32;
        arena.put_u64(slot + COUNTRY, country_vtable as u64);
        arena.put_u32(slot + COUNTRY + OBJ_TAG_DB_ID, tag_db_id);
        arena.put_u64(slot + COUNTRY + OBJ_POLITICAL_STATUS, (slot + STATUS) as u64);
        arena.put_u64(slot + STATUS, status_vtable as u64);
        arena.put_u64(
            slot + STATUS + POLITICAL_STATUS_COUNTRY_BACKLINK,
            (slot + COUNTRY) as u64,
        );
        arena.put_u64(
            slot + STATUS + POLITICAL_STATUS_RULING_PARTY,
            (slot + PARTY) as u64,
        );
        arena.put_u64(slot + PARTY, party_vtable as u64);
        arena.put_u64(
            slot + PARTY + POLITICAL_PARTY_IDEOLOGY_GROUP,
            (slot + GROUP) as u64,
        );
        arena.put_u64(slot + GROUP, group_vtable as u64);
        arena.put_msvc_string(
            slot + GROUP + IDEOLOGY_GROUP_ID,
            slot + GROUP_TOKEN_BYTES,
            "democratic",
        );

        // Singleton ruling-party leader vector: {data, packed size|capacity}.
        arena.put_u64(
            slot + PARTY + POLITICAL_PARTY_LEADER_VEC,
            (slot + LEADER_VEC) as u64,
        );
        arena.put_u64(
            slot + PARTY + POLITICAL_PARTY_LEADER_VEC + 8,
            (1u64 << 32) | 1u64,
        );
        arena.put_u64(slot + LEADER_VEC, (slot + COUNTRY_LEADER) as u64);
        arena.put_u64(slot + COUNTRY_LEADER, leader_vtable as u64);
        arena.put_u64(
            slot + COUNTRY_LEADER + COUNTRY_LEADER_CHARACTER_PTR,
            (slot + CHARACTER) as u64,
        );
        arena.put_u64(slot + CHARACTER, character_vtable as u64);
        arena.put_u32(
            slot + CHARACTER + POLITICS_CHARACTER_OFF_TYPE,
            POLITICS_CHARACTER_TYPE,
        );
        arena.put_u32(
            slot + CHARACTER + POLITICS_CHARACTER_OFF_ID,
            1000 + idx as u32,
        );
        // Per-country ruler strings. Identical strings across countries would
        // make a test that repoints one country's ruling party at another's
        // observe no change at all, and the cross-half coherence check would
        // look broken when it was merely being asked about two equal values.
        arena.put_msvc_string(
            slot + CHARACTER + POLITICS_CHARACTER_OFF_DISPLAY,
            slot + RULER_NAME_BYTES,
            &format!("A Rather Longer Ruler Name {idx}"),
        );
        arena.put_msvc_string(
            slot + CHARACTER + POLITICS_CHARACTER_OFF_TOKEN,
            slot + RULER_TOKEN_BYTES,
            &format!("ENG_ruler_token_{idx}"),
        );

        // The active-idea vector: data @ +0x50, capacity @ +0x58, size @ +0x5C.
        arena.put_u64(
            slot + STATUS + POLITICAL_STATUS_IDEAS_DATA,
            (slot + IDEA_PTRS) as u64,
        );
        // Capacity is deliberately NOT equal to size. With `cap == size` any
        // check that confuses the two, or hashes them in the wrong order, is
        // invisible.
        arena.put_u32(
            slot + STATUS + POLITICAL_STATUS_IDEAS_CAP,
            (T15_IDEAS_PER_COUNTRY + 4) as u32,
        );
        arena.put_u32(
            slot + STATUS + POLITICAL_STATUS_IDEAS_SIZE,
            T15_IDEAS_PER_COUNTRY as u32,
        );
        for entry in 0..T15_IDEAS_PER_COUNTRY {
            let idea = slot + IDEAS + entry * POLITICS_IDEA_STRIDE;
            arena.put_u64(slot + IDEA_PTRS + entry * 8, idea as u64);
            // Both accepted idea vtables occur, as they do on the live heap.
            arena.put_u64(
                idea,
                if entry % 3 == 2 {
                    idea_vtable_secondary as u64
                } else {
                    idea_vtable_primary as u64
                },
            );
            // Half SSO, half heap-backed: the live mix of short law tokens and
            // long "COUNTRY_advisor_name" idea tokens. The last entry repeats
            // the first one's token from a DIFFERENT idea object, so the
            // `dedup` in `domain47_compose_politics_snapshot` actually has
            // something to remove — with all tokens distinct it would be
            // unobservable.
            let token = if entry + 1 == T15_IDEAS_PER_COUNTRY {
                "law_0".to_string()
            } else if entry % 2 == 0 {
                format!("law_{entry}")
            } else {
                format!("ENG_long_idea_token_{idx}_{entry}")
            };
            arena.put_msvc_string(idea + IDEA_ID, idea + POLITICS_IDEA_TOKEN_BYTES, &token);
        }

        rows.push(politics_bench_country_row(slot + COUNTRY, tag_db_id));
    }

    let spare = base + T15_POLITICS_COUNTRIES * POLITICS_COUNTRY_STRIDE;
    PoliticsBenchWorld {
        arena,
        module_base,
        country_vtable,
        rows,
        spare,
    }
}

/// D47 reads exactly two fields off a `CountryObjectRow` — `object_ptr` and
/// `tag_db_id`. Everything else is absent here, as it is on that path.
fn politics_bench_country_row(object_ptr: usize, tag_db_id: u32) -> CountryObjectRow {
    CountryObjectRow {
        tag: format!("C{tag_db_id:03}"),
        tag_db_id,
        name: None,
        object_ptr,
        record_ptr: None,
        original_capital: None,
        original_tag: None,
        templates_locked: None,
        reserved_dynamic_country: None,
        stability: None,
        war_support: None,
        political_power: None,
        command_power: None,
        num_ships: None,
        manpower_pool: None,
        convoys: None,
        misc: None,
        fuel: None,
        production: None,
        focus: None,
        research: None,
        politics: None,
        politics_residuals: None,
        decisions: None,
        diplomacy: None,
    }
}

/// The status vtable the bracketed reader resolves each country's political
/// status against.
fn politics_status_vtable(world: &PoliticsBenchWorld) -> usize {
    world.module_base + POLITICAL_STATUS_VTABLE_RVA
}

/// Read a country's idea vector the way production does — starting from the
/// COUNTRY, so the political-status root is re-resolved by each observation.
fn politics_bracketed_ideas(world: &PoliticsBenchWorld, index: usize) -> Option<Vec<String>> {
    domain47_read_active_ideas_bracketed(
        world.rows[index].object_ptr,
        politics_status_vtable(world),
        world.module_base + IDEA_VTABLE_RVA_PRIMARY,
        world.module_base + IDEA_VTABLE_RVA_SECONDARY,
    )
}

fn politics_status_ptrs(world: &PoliticsBenchWorld) -> Vec<usize> {
    world
        .rows
        .iter()
        .map(|row| {
            validated_political_status_ptr(
                row.object_ptr,
                world.module_base + POLITICAL_STATUS_VTABLE_RVA,
            )
            .expect("fixture status pointer must validate")
        })
        .collect()
}

/// The per-country body of `domain47_country_politics_roots` AS IT WAS: two
/// complete snapshots, compared whole.
fn politics_walk_legacy(world: &PoliticsBenchWorld) -> usize {
    let mut sink = 0usize;
    for expected in &world.rows {
        let first =
            domain47_country_politics_snapshot(world.module_base, world.country_vtable, expected);
        let second =
            domain47_country_politics_snapshot(world.module_base, world.country_vtable, expected);
        if let Some(snapshot) = first.filter(|first| Some(first) == second.as_ref()) {
            sink += snapshot.active_idea_tokens.len();
        }
    }
    sink
}

/// The per-country body as it is now: the cheap identity half still observed
/// twice and compared, the expensive idea half walked ONCE under its own
/// structural bracket.
fn politics_walk_now(world: &PoliticsBenchWorld) -> usize {
    let mut sink = 0usize;
    for expected in &world.rows {
        if let Some(snapshot) =
            domain47_country_politics_row(world.module_base, world.country_vtable, expected)
        {
            sink += snapshot.active_idea_tokens.len();
        }
    }
    sink
}

/// In-process, run-by-run interleaved A/B of the whole country-politics walk.
#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_country_politics_ab() {
    let world = politics_bench_world();
    // Both arms count POST-dedup tokens, because that is what the composed
    // snapshot carries.
    let expected_tokens = T15_POLITICS_COUNTRIES * T15_DISTINCT_IDEAS_PER_COUNTRY;
    // A fixture that resolved nothing would make both arms measure the
    // rejection path, and the comparison would be of two refusals.
    page_cache_new_frame();
    assert_eq!(
        politics_walk_legacy(&world),
        expected_tokens,
        "legacy arm did not resolve every country's ideas"
    );
    page_cache_new_frame();
    assert_eq!(
        politics_walk_now(&world),
        expected_tokens,
        "current arm did not resolve every country's ideas"
    );

    let (legacy, now) = time_us_median_ab(
        5,
        || politics_walk_legacy(&world),
        || politics_walk_now(&world),
    );
    println!(
        "d47_cost AB country_politics: legacy {legacy:.1} us -> now {now:.1} us ({:+.1}%) \
         over {T15_POLITICS_COUNTRIES} countries x {T15_IDEAS_PER_COUNTRY} ideas",
        (now - legacy) / legacy * 100.0
    );
}

/// What the bracket COSTS, measured against the cheapest thing that produces
/// the same tokens.
///
/// These two arms are NOT equivalent and the comparison is not a win/loss:
/// `read_active_ideas` walks the vector once and is not bracketed at all,
/// while `domain47_read_active_ideas_bracketed` takes two full observations
/// and compares them. The bracketed arm therefore reads roughly twice the
/// bytes and is expected to be SLOWER here. What this number is good for is
/// pricing the bracket — the safety the country walk buys by not trusting a
/// single pass — separately from the decode-and-mutex saving, which is what
/// `d47_cost_country_politics_ab` actually measures against the form that
/// shipped.
#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_idea_walk_cache_vs_bracket_ab() {
    let world = politics_bench_world();
    let status_ptrs = politics_status_ptrs(&world);
    let object_ptrs: Vec<usize> = world.rows.iter().map(|row| row.object_ptr).collect();
    let status_vtable = politics_status_vtable(&world);
    let vt1 = world.module_base + IDEA_VTABLE_RVA_PRIMARY;
    let vt2 = world.module_base + IDEA_VTABLE_RVA_SECONDARY;
    let expected_tokens = T15_POLITICS_COUNTRIES * T15_IDEAS_PER_COUNTRY;

    let cached = || {
        let mut sink = 0usize;
        for status_ptr in &status_ptrs {
            sink += read_active_ideas(*status_ptr, vt1, vt2).map_or(0, |v| v.len());
        }
        sink
    };
    let bracketed = || {
        let mut sink = 0usize;
        for object_ptr in &object_ptrs {
            sink +=
                domain47_read_active_ideas_bracketed(*object_ptr, status_vtable, vt1, vt2)
                    .map_or(0, |v| v.len());
        }
        sink
    };
    page_cache_new_frame();
    assert_eq!(cached(), expected_tokens);
    page_cache_new_frame();
    assert_eq!(bracketed(), expected_tokens);

    let (cached_us, bracketed_us) = time_us_median_ab(5, cached, bracketed);
    println!(
        "d47_cost AB idea_walk: cached+unbracketed {cached_us:.1} us -> uncached+bracketed \
         {bracketed_us:.1} us ({:+.1}%)",
        (bracketed_us - cached_us) / cached_us * 100.0
    );
}

/// The restructured country walk must produce the SAME snapshot the two full
/// snapshots produced — every country, every field.
///
/// MUTATION: drop the `.sort()` or the `.dedup()` from
/// `domain47_compose_politics_snapshot`, or make
/// `domain47_read_active_ideas_bracketed_with_between` walk `size - 1`
/// entries — the per-country equality fails.
#[test]
fn d47_bracketed_country_politics_matches_the_double_snapshot() {
    let world = politics_bench_world();
    page_cache_new_frame();
    let mut compared = 0usize;
    for expected in &world.rows {
        let legacy =
            domain47_country_politics_snapshot(world.module_base, world.country_vtable, expected)
                .expect("fixture country must resolve on the legacy path");
        let identity = domain47_country_identity_snapshot(world.country_vtable, expected)
            .expect("fixture country must resolve its identity");
        let tokens = domain47_active_idea_tokens_bracketed(world.module_base, expected.object_ptr)
            .expect("fixture country must resolve its ideas");
        let now = domain47_compose_politics_snapshot(identity, tokens);
        assert_eq!(legacy, now, "country {} diverged", expected.tag_db_id);
        // Non-vacuity: this country carried all four value families, so the
        // equality is not an equality of four `None`s and an empty vector.
        assert_eq!(
            legacy.active_idea_tokens.len(),
            T15_DISTINCT_IDEAS_PER_COUNTRY,
            "the fixture must exercise dedup"
        );
        // Sorted order asserted INDEPENDENTLY. Both sides of the equality
        // above run through the same `domain47_compose_politics_snapshot`, so
        // dropping its `.sort()` would move both together and the equality
        // would not notice (Codex round 4, finding 3). The dedup is covered by
        // the count above for the same reason it needs covering separately.
        let mut sorted = now.active_idea_tokens.clone();
        sorted.sort();
        assert_eq!(
            now.active_idea_tokens, sorted,
            "the emitted token list must be sorted"
        );
        assert_eq!(legacy.ruling_party.as_deref(), Some("democratic"));
        assert!(legacy.ruler_name.is_some());
        assert!(legacy.ruler_token.is_some());
        compared += 1;
    }
    assert_eq!(compared, T15_POLITICS_COUNTRIES);
}

/// A country repointed at a DIFFERENT political-status object between the two
/// observations must be refused, not read twice from the first one.
///
/// This is the hazard of resolving `validated_political_status_ptr` once and
/// handing the resolved pointer to both observations. The victim status object
/// is fully valid and carries a different idea vector, and — the point of the
/// fixture — the ruler strings on both sides are made IDENTICAL, so the
/// identity bracket compares equal by value and cannot notice. Only an
/// observation that re-resolves the root from the country can.
///
/// MUTATION: make `domain47_observe_active_ideas` resolve the status root ONCE
/// and reuse it for both observations — which is what this code did before
/// Codex round 4. Both observations then read the ORIGINAL status object,
/// agree, and the stale idea list is published; this fails.
///
/// NOT a mutation this catches, stated so the claim above is not read as more
/// than it is: removing `status_ptr` from `Domain47IdeaObservation`'s equality
/// leaves this GREEN, because the two status objects also carry different idea
/// vectors and the header and entries differ anyway. The field earns its place
/// only where two status objects carry IDENTICAL vectors — in which case the
/// tokens published would have been the same either way. Re-resolving the root
/// per observation is the load-bearing property here; carrying it in the
/// comparison is belt-and-braces.
#[test]
fn d47_idea_walk_refuses_a_country_repointed_at_another_status_object() {
    let world = politics_bench_world();
    let vt1 = world.module_base + IDEA_VTABLE_RVA_PRIMARY;
    let vt2 = world.module_base + IDEA_VTABLE_RVA_SECONDARY;
    let country = world.rows[0].object_ptr;
    let victim_status = politics_status_ptrs(&world)[1];
    let own_status = politics_status_ptrs(&world)[0];
    assert_ne!(own_status, victim_status);

    // Make the two countries' ruler strings identical, so the identity
    // bracket sees no change and the ONLY thing that can refuse this is the
    // re-resolved status root.
    let victim_slot = world.arena.base + POLITICS_COUNTRY_STRIDE;
    world.arena.put_msvc_string(
        victim_slot + 0x0A00 + POLITICS_CHARACTER_OFF_DISPLAY,
        victim_slot + 0x0C00,
        "A Rather Longer Ruler Name 0",
    );
    world.arena.put_msvc_string(
        victim_slot + 0x0A00 + POLITICS_CHARACTER_OFF_TOKEN,
        victim_slot + 0x0D00,
        "ENG_ruler_token_0",
    );
    // CONTROL — taken BEFORE the victim's backlink is rewritten, because that
    // rewrite is what stops the victim resolving as its own country.
    page_cache_new_frame();
    let control = politics_bracketed_ideas(&world, 0).expect("control walk must succeed");
    assert_eq!(control.len(), T15_IDEAS_PER_COUNTRY);
    let victim_tokens = politics_bracketed_ideas(&world, 1).expect("victim must resolve");
    assert_ne!(
        control, victim_tokens,
        "the two countries must carry DIFFERENT idea lists, or the swap is invisible"
    );

    // The victim status must back-link to OUR country, or the re-resolved root
    // would be rejected by `validated_political_status_ptr`'s backlink check
    // rather than by the observation comparison, and the test would pass for
    // the wrong reason.
    world
        .arena
        .put_u64(victim_status + POLITICAL_STATUS_COUNTRY_BACKLINK, country as u64);
    page_cache_new_frame();
    assert_eq!(
        validated_political_status_ptr(country, politics_status_vtable(&world)),
        Some(own_status),
        "the country must still resolve its OWN status before the swap"
    );

    // The country is repointed at the victim's status object between the two
    // observations.
    page_cache_new_frame();
    let torn = domain47_read_active_ideas_bracketed_with_between(
        country,
        politics_status_vtable(&world),
        vt1,
        vt2,
        || {
            world
                .arena
                .put_u64(country + OBJ_POLITICAL_STATUS, victim_status as u64)
        },
    );
    assert!(
        torn.is_none(),
        "a country repointed at another status object must be refused, not read \
         twice from the first one: {torn:?}"
    );

    world
        .arena
        .put_u64(country + OBJ_POLITICAL_STATUS, own_status as u64);
    page_cache_new_frame();
    assert!(politics_bracketed_ideas(&world, 0).is_some());
}

/// A country whose RULER changed while its ideas were being read must be
/// refused, not published as an old ruler beside new ideas.
///
/// The ruler is repointed after the first observation reads the ideas, and
/// restored before the second observation reads its ruler. Neither observation
/// is internally coherent with the other, so the row must be refused.
///
/// MUTATION: drop `identity` from `Domain47CountryObservation`'s equality (or
/// compare only `ideas`). The two observations then agree on the ideas, the
/// mismatched ruler is published, and this fails.
#[test]
fn d47_country_row_refuses_a_ruler_that_changed_during_the_idea_walk() {
    let world = politics_bench_world();
    let expected = &world.rows[0];

    // CONTROL: undisturbed, the row resolves with all four value families.
    page_cache_new_frame();
    let control = domain47_country_politics_row(world.module_base, world.country_vtable, expected)
        .expect("control row must resolve");
    assert_eq!(control.ruling_party.as_deref(), Some("democratic"));
    assert_eq!(control.ruler_token.as_deref(), Some("ENG_ruler_token_0"));
    assert!(control.ruler_name.is_some());
    assert_eq!(
        control.active_idea_tokens.len(),
        T15_DISTINCT_IDEAS_PER_COUNTRY
    );

    // The country's ruling party is repointed at ANOTHER country's party after
    // the first observation has read the ruler and before it reads the ideas.
    // Everything the reader touches stays well-formed; only comparing the two
    // halves as ONE value can notice.
    let status = world.arena.base + 0x0200;
    let victim_party = world.arena.base + POLITICS_COUNTRY_STRIDE + 0x0400;
    let original = read_u64(status + POLITICAL_STATUS_RULING_PARTY).expect("party ptr must read");
    assert_ne!(original as usize, victim_party);

    page_cache_new_frame();
    let torn = domain47_country_politics_row_with_between(
        world.module_base,
        world.country_vtable,
        expected,
        // After the first observation's ideas, before its ruler: move to the
        // other country's party.
        || {
            world
                .arena
                .put_u64(status + POLITICAL_STATUS_RULING_PARTY, victim_party as u64)
        },
        // Between the observations: move back, so the second observation reads
        // the original ruler and the two disagree.
        || {
            world
                .arena
                .put_u64(status + POLITICAL_STATUS_RULING_PARTY, original)
        },
    );
    assert!(
        torn.is_none(),
        "a country whose ruler changed across the idea walk must be refused: {torn:?}"
    );

    // Restore, and confirm the fixture resolves again.
    world
        .arena
        .put_u64(status + POLITICAL_STATUS_RULING_PARTY, original);
    page_cache_new_frame();
    assert!(
        domain47_country_politics_row(world.module_base, world.country_vtable, expected).is_some()
    );
}

/// A country that moves to a different political-status object and MOVES BACK
/// must be refused — the cross-half ABA that Codex round 5 found.
///
/// The sequence, using both seams:
///  1. the first observation reads `A`'s tag and `A`'s ideas;
///  2. `mid_first` moves the country to a different valid status `B`, carrying
///     a different ruler and different ideas;
///  3. that same observation reads `B`'s ruler;
///  4. `between` moves the country back to `A`;
///  5. the second observation reads `A`'s ideas and `A`'s ruler.
///
/// The two observations differ, so the row is refused. Under the structure
/// this replaced — an identity pair and an idea pair confirmed INDEPENDENTLY —
/// each pair agreed with itself and the row published one status object's
/// ruler beside the other's ideas.
///
/// WHAT NO VERSION OF THIS CAN GUARANTEE, stated so the test is not read as
/// proving more than it does: observing twice and comparing cannot rule out an
/// adversarial REPEATING schedule. A writer that reproduces the same
/// interleaving during both observations yields two identical observations of
/// a combination that never existed, and no bounded number of reads closes
/// that — the two full snapshots this work replaced could be fooled the same
/// way. The property actually defended is narrower and checkable: this reader
/// reads in the ORIGINAL's order, so any schedule that fools it would have
/// fooled the original, and the four documented narrowings then reject strictly
/// more. Acceptance is a subset of the original's, not an absolute.
///
/// MUTATION: in `domain47_country_politics_row_with_between`, replace the two
/// whole-country observations with the nested form — one identity read, then
/// `domain47_read_active_ideas_bracketed_with_between` for the ideas, then a
/// second identity read compared only against the first. This publishes and
/// the test fails.
#[test]
fn d47_country_row_refuses_a_status_root_that_returned() {
    let world = politics_bench_world();
    let expected = &world.rows[0];
    let country = expected.object_ptr;
    let own_status = politics_status_ptrs(&world)[0];
    let victim_status = politics_status_ptrs(&world)[1];

    // CONTROL, taken before the victim's backlink is rewritten.
    page_cache_new_frame();
    let control = domain47_country_politics_row(world.module_base, world.country_vtable, expected)
        .expect("control row must resolve");
    let victim_ideas = politics_bracketed_ideas(&world, 1).expect("victim must resolve");
    assert_ne!(
        control.active_idea_tokens, victim_ideas,
        "the two status objects must carry DIFFERENT ideas, or the swap is invisible"
    );

    // The victim status must back-link to OUR country, or it would be rejected
    // by the backlink check and the test would pass for the wrong reason.
    world
        .arena
        .put_u64(victim_status + POLITICAL_STATUS_COUNTRY_BACKLINK, country as u64);

    page_cache_new_frame();
    let torn = domain47_country_politics_row_with_between(
        world.module_base,
        world.country_vtable,
        expected,
        // 2. after the first observation's ruler read: move to B.
        || {
            world
                .arena
                .put_u64(country + OBJ_POLITICAL_STATUS, victim_status as u64)
        },
        // 4. between the observations: move back to A.
        || {
            world
                .arena
                .put_u64(country + OBJ_POLITICAL_STATUS, own_status as u64)
        },
    );
    assert!(
        torn.is_none(),
        "a country whose status root moved and returned must be refused: {torn:?}"
    );

    page_cache_new_frame();
    assert!(
        domain47_country_politics_row(world.module_base, world.country_vtable, expected).is_some(),
        "the restored fixture must resolve again"
    );
}

/// The mutex-free token decode must return exactly what the shared cached
/// reader returns — for both string forms, both accepted idea vtables, and the
/// refusals.
///
/// MUTATION: drop the `is_script_token(&token)` validation from
/// `domain47_idea_token`, or drop its vtable check — the refusal cases below
/// start returning `Some` while `cached_idea_token` still returns `None`.
#[test]
fn d47_uncached_idea_token_matches_the_cached_reader() {
    let world = politics_bench_world();
    let vt1 = world.module_base + IDEA_VTABLE_RVA_PRIMARY;
    let vt2 = world.module_base + IDEA_VTABLE_RVA_SECONDARY;
    page_cache_new_frame();

    let mut sso = 0usize;
    let mut heap = 0usize;
    let mut first_idea = None;
    for status_ptr in politics_status_ptrs(&world) {
        let (data, size, _cap) =
            politics_ideas_header(status_ptr, None).expect("fixture ideas header must read");
        for index in 0..size as usize {
            let idea_ptr = politics_idea_ptr_at(data, index, None).expect("idea pointer must read");
            first_idea.get_or_insert(idea_ptr);
            let cached = cached_idea_token(idea_ptr, vt1, vt2);
            let uncached = domain47_idea_token(idea_ptr, vt1, vt2);
            assert_eq!(cached, uncached, "idea at {idea_ptr:#x}");
            match cached.as_deref() {
                Some(token) if token.len() < 16 => sso += 1,
                Some(_) => heap += 1,
                None => panic!("fixture idea at {idea_ptr:#x} did not read"),
            }
        }
    }
    assert!(sso > 0 && heap > 0, "fixture covered only one string form");

    // The refusals must agree too, or the equality above only proves the two
    // agree when everything is well-formed.
    let sample = first_idea.expect("fixture had at least one idea");
    let wrong_vtable = world.module_base + IDEA_VTABLE_RVA_PRIMARY + 0x1000;
    assert_eq!(
        cached_idea_token(sample, wrong_vtable, wrong_vtable),
        None,
        "a vtable mismatch must be refused by the cached reader"
    );
    assert_eq!(
        domain47_idea_token(sample, wrong_vtable, wrong_vtable),
        None,
        "a vtable mismatch must be refused identically"
    );
    // A pointer outside `[HEAP_LO, HEAP_HI)`.
    assert_eq!(cached_idea_token(0x1000, vt1, vt2), None);
    assert_eq!(domain47_idea_token(0x1000, vt1, vt2), None);

    // A READABLE idea whose id string is not a valid script token. Without
    // this, dropping `is_script_token` from `domain47_idea_token` would leave
    // this test green and the refusal equivalence would be half-proven.
    // It is built in spare arena space, deliberately outside every country's
    // vector, so it cannot make the walk fixtures refuse.
    let invalid = world.spare;
    world.arena.put_u64(invalid, vt1 as u64);
    world
        .arena
        .put_msvc_string(invalid + IDEA_ID, invalid + 0x40, "not a script token!");
    page_cache_new_frame();
    assert!(
        read_msvc_string_snapshot_once(invalid + IDEA_ID, MAX_POLITICS_TOKEN_LEN).is_some(),
        "the invalid-token fixture must be READABLE, or it proves nothing about the validator"
    );
    assert_eq!(
        cached_idea_token(invalid, vt1, vt2),
        None,
        "the cached reader must refuse a non-script token"
    );
    assert_eq!(
        domain47_idea_token(invalid, vt1, vt2),
        None,
        "an invalid token must be refused identically"
    );

    // AND THE PRODUCTION READER MUST REFUSE IT TOO. Everything above compares
    // two reference helpers; `domain47_read_active_ideas_bracketed` defers the
    // decode and the validation to its own tail, and that tail is what
    // production actually runs. Without this, removing `is_script_token` from
    // the production decode — or its pointer and vtable gates — would leave
    // this test green (Codex round 3, finding 2).
    let status_ptr = politics_status_ptrs(&world)[0];
    let (data, _size, _cap) = politics_ideas_header(status_ptr, None).expect("header must read");
    let original = politics_idea_ptr_at(data, 0, None).expect("idea pointer must read");
    page_cache_new_frame();
    assert!(
        politics_bracketed_ideas(&world, 0).is_some(),
        "control walk must succeed, or the refusal below proves nothing"
    );

    // An idea object that reads fine but carries the WRONG VTABLE.
    let wrong_vtable_idea = world.spare + 0x200;
    world.arena.put_u64(wrong_vtable_idea, (vt1 + 0x1000) as u64);
    world
        .arena
        .put_msvc_string(wrong_vtable_idea + IDEA_ID, wrong_vtable_idea + 0x40, "law_9");

    // An idea object that is COMPLETELY VALID except that its address fails
    // `is_map_control_heap_pointer`, because it is not 8-aligned. It has to be
    // valid in every other respect, or a downstream gate would refuse it and
    // deleting the heap gate would still look like a pass — which is what the
    // previous version of this fixture got wrong: it used the unmapped address
    // 0x1000, so the vtable READ failed and the heap gate was never the reason.
    let unaligned_idea = world.spare + 0x401;
    assert!(
        !is_map_control_heap_pointer(unaligned_idea),
        "the fixture must actually fail the heap gate"
    );
    world.arena.put_u64(unaligned_idea, vt1 as u64);
    world
        .arena
        .put_msvc_string(unaligned_idea + IDEA_ID, unaligned_idea + 0x40, "law_7");
    page_cache_new_frame();
    assert_eq!(
        read_u64(unaligned_idea).map(|v| v as usize),
        Some(vt1),
        "the fixture must be READABLE, or the heap gate is not what refuses it"
    );
    assert!(
        read_msvc_string_snapshot_once(unaligned_idea + IDEA_ID, MAX_POLITICS_TOKEN_LEN).is_some(),
        "the fixture's token must be readable too"
    );

    // An idea whose id string is not valid UTF-8.
    //
    // SCOPE, stated honestly: this pins the REFUSAL, not which check performs
    // it. `is_script_token` accepts only ASCII alphanumerics and `_-.:`, and
    // every ASCII sequence is valid UTF-8, so no input can fail the UTF-8
    // conversion while passing the validator. The two checks are therefore not
    // independently observable by outcome, and no test can attribute this
    // refusal to one of them.
    let non_utf8_idea = world.spare + 0x600;
    world.arena.put_u64(non_utf8_idea, vt1 as u64);
    // Long enough to be HEAP-backed. A short token takes the inline (SSO)
    // representation and its characters live in the header itself, so
    // overwriting the heap block would leave the token untouched — which is
    // exactly what the first version of this fixture did.
    world.arena.put_msvc_string(
        non_utf8_idea + IDEA_ID,
        non_utf8_idea + 0x40,
        "law_that_is_long_enough",
    );
    world
        .arena
        .put_bytes(non_utf8_idea + 0x40, &[0xFF, 0xFE, 0xFD, 0xFC, 0xFB]);
    page_cache_new_frame();
    assert!(
        read_msvc_string_snapshot_once(non_utf8_idea + IDEA_ID, MAX_POLITICS_TOKEN_LEN)
            .is_some_and(|s| String::from_utf8(s.bytes).is_err()),
        "the fixture's token must READ but not be valid UTF-8"
    );

    for (case, entry) in [
        ("a readable but non-script token", invalid),
        ("an idea whose vtable is not an accepted one", wrong_vtable_idea),
        ("a valid idea at an address that fails the heap gate", unaligned_idea),
        ("an id string that is not valid UTF-8", non_utf8_idea),
    ] {
        world.arena.put_u64(data, entry as u64);
        page_cache_new_frame();
        assert!(
            politics_bracketed_ideas(&world, 0).is_none(),
            "production must refuse a vector containing {case}"
        );
    }

    world.arena.put_u64(data, original as u64);
    page_cache_new_frame();
    assert!(
        politics_bracketed_ideas(&world, 0).is_some(),
        "restored fixture must walk cleanly again"
    );
}

/// An idea's id string rewritten IN PLACE between the two observations —
/// same pointer, same vtable, same length — must be refused.
///
/// This is the class the second review round found a pointer-and-vtable-only
/// bracket admits. Every per-entry check passes on both the old and the new
/// bytes; only a comparison that includes the string CONTENT can see it. With
/// a structural-only bracket the walk publishes a list mixing one entry read
/// before the rewrite with another read after it — a combination that never
/// existed in the engine, well-formed and unflagged.
///
/// MUTATION: in `Domain47IdeaEntry`, drop the `string` field from the derived
/// `PartialEq` (or compare only `idea_ptr` and `vtable` in
/// `domain47_read_active_ideas_bracketed_with_between`). The rewritten token
/// is then accepted and this fails.
#[test]
fn d47_idea_walk_refuses_an_id_string_rewritten_in_place() {
    let world = politics_bench_world();
    let vt1 = world.module_base + IDEA_VTABLE_RVA_PRIMARY;
    let vt2 = world.module_base + IDEA_VTABLE_RVA_SECONDARY;
    let status_ptr = politics_status_ptrs(&world)[0];
    let (data, _size, _cap) = politics_ideas_header(status_ptr, None).expect("header must read");
    let idea_ptr = politics_idea_ptr_at(data, 0, None).expect("idea pointer must read");

    // CONTROL.
    page_cache_new_frame();
    let before = politics_bracketed_ideas(&world, 0).expect("control walk must succeed");
    assert_eq!(before.len(), T15_IDEAS_PER_COUNTRY);
    let original = before[0].clone();

    // Rewrite the id string's characters in place, same length, still a valid
    // script token, leaving the pointer, the vtable, the size and the capacity
    // untouched. Nothing structural changes.
    let rewritten: String = original
        .chars()
        .map(|c| if c == 'l' { 'x' } else { c })
        .collect();
    assert_eq!(rewritten.len(), original.len());
    assert_ne!(rewritten, original);
    let chars_addr = {
        let snapshot = read_msvc_string_snapshot_once(idea_ptr + IDEA_ID, MAX_POLITICS_TOKEN_LEN)
            .expect("string snapshot must read");
        snapshot.chars_addr
    };
    page_cache_new_frame();
    let torn = domain47_read_active_ideas_bracketed_with_between(
        world.rows[0].object_ptr,
        politics_status_vtable(&world),
        vt1,
        vt2,
        || unsafe {
        std::ptr::copy_nonoverlapping(
            rewritten.as_ptr(),
            chars_addr as *mut u8,
            rewritten.len(),
        );
    },
    );
    assert!(
        torn.is_none(),
        "an id string rewritten between the observations must be refused: {torn:?}"
    );
}

/// A vector entry repointed between the two observations must be REFUSED.
///
/// The substituted entry is another live country's idea: correctly typed, in
/// the heap window, and decoding to a perfectly valid token. Nothing in the
/// per-entry path can object to it. Without the observation comparison the
/// reader would return a clean, well-formed token list describing a version of
/// the vector that no longer exists — the silently-wrong-data outcome, not an
/// absence.
///
/// MUTATION: in `domain47_read_active_ideas_bracketed_with_between`, drop the
/// `if opening != closing { return None; }` test. The torn read is then
/// accepted and the `is_none()` assertion fails. Dropping the `between()`
/// invocation instead means the mutation never fires between the observations,
/// the read succeeds, and the same assertion fails.
#[test]
fn d47_bracketed_idea_walk_refuses_a_vector_that_moved_under_it() {
    let world = politics_bench_world();
    let vt1 = world.module_base + IDEA_VTABLE_RVA_PRIMARY;
    let vt2 = world.module_base + IDEA_VTABLE_RVA_SECONDARY;
    let status_ptrs = politics_status_ptrs(&world);
    let status_ptr = status_ptrs[0];
    let victim_status = status_ptrs[1];
    let (data, _size, _cap) = politics_ideas_header(status_ptr, None).expect("header must read");
    let (victim_data, _s, _c) =
        politics_ideas_header(victim_status, None).expect("victim header must read");

    // CONTROL: undisturbed, the walk succeeds and returns every entry. Without
    // this the refusal below could be a fixture that never worked at all.
    page_cache_new_frame();
    assert_eq!(
        politics_bracketed_ideas(&world, 0).map(|v| v.len()),
        Some(T15_IDEAS_PER_COUNTRY),
        "control walk must succeed"
    );

    // Entry 0 — the one `between` fires immediately after, so it has already
    // been decoded and its pointer already folded into the opening hash.
    let original = politics_idea_ptr_at(data, 0, None).expect("idea pointer must read");
    let replacement =
        politics_idea_ptr_at(victim_data, 0, None).expect("victim idea pointer must read");
    assert_ne!(original, replacement);
    assert!(
        domain47_idea_token(replacement, vt1, vt2).is_some(),
        "the substituted idea must itself be well-formed, or the per-entry \
         path would reject it and the bracket would prove nothing"
    );

    page_cache_new_frame();
    let torn = domain47_read_active_ideas_bracketed_with_between(
        world.rows[0].object_ptr,
        politics_status_vtable(&world),
        vt1,
        vt2,
        || unsafe { std::ptr::write_unaligned(data as *mut u64, replacement as u64) },
    );
    assert!(
        torn.is_none(),
        "an entry repointed after the walk read it must be refused, not published: {torn:?}"
    );

    // Restore, and confirm the fixture walks cleanly again — so the refusal
    // above was the mutation and not a fixture the test broke permanently.
    unsafe {
        std::ptr::write_unaligned(data as *mut u64, original as u64);
    }
    page_cache_new_frame();
    assert_eq!(
        politics_bracketed_ideas(&world, 0).map(|v| v.len()),
        Some(T15_IDEAS_PER_COUNTRY),
    );
}

// --- what the prewarm's own clock bracket COSTS ---------------------------

/// A world in which `read_authenticated_scheduler_clock` succeeds.
///
/// The reader forms the singleton slot as `module_base + RVA` and DEREFERENCES
/// it, so unlike the vtable constants — which are only compared — that address
/// must be real memory. The fake module base is therefore derived by
/// SUBTRACTING the RVA from the arena, which puts the slot at the arena's own
/// base. The game-state pointer it holds must pass `is_heap_pointer_aligned`,
/// which is why the arena is taken from the heap window.
struct ClockBenchWorld {
    _arena: BenchArena,
    _elsewhere: BenchArena,
    module_base: usize,
    date_addr: usize,
    /// The CGameState singleton slot — the first address
    /// `read_authenticated_scheduler_clock` touches. Read on its own it
    /// performs the epoch transition and inserts this arena's span, which is
    /// what lets both bench arms enter the displacement read in the same
    /// cache state.
    slot_addr: usize,
    /// An address in a DIFFERENT committed region, used to displace the
    /// one-entry `REGION_MRU` the way the prewarm's own walk does.
    elsewhere_addr: usize,
}

fn clock_bench_world() -> ClockBenchWorld {
    const ARENA: usize = 0x10000;
    const GAME_STATE: usize = 0x1000;
    let arena = BenchArena::in_heap_window(ARENA);
    let elsewhere = BenchArena::in_heap_window(ARENA);
    assert_ne!(
        arena.base, elsewhere.base,
        "the displacing read must land in a different region"
    );
    let slot = arena.base;
    let game_state = arena.base + GAME_STATE;
    let date_addr = game_state + GS_CURRENT_DATE_OFFSET;
    assert!(
        is_heap_pointer_aligned(game_state, 8),
        "fixture game state must satisfy the reader's own pointer gate"
    );
    unsafe {
        (slot as *mut u64).write(game_state as u64);
        (date_addr as *mut i32).write(60_832_014);
        (elsewhere.base as *mut u64).write(0);
    }
    let module_base = slot
        .checked_sub(compact_contract::build_profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA)
        .expect("arena sits above the singleton RVA");
    ClockBenchWorld {
        _arena: arena,
        elsewhere_addr: elsewhere.base,
        _elsewhere: elsewhere,
        module_base,
        date_addr,
        slot_addr: slot,
    }
}

/// The median of the PAIRED differences, not the difference of two medians.
///
/// `time_us_median_ab` sorts each arm independently, so its two medians can
/// come from runs that saw different machine load and their difference is not
/// itself an observation. For a sub-microsecond quantity that matters: each
/// interleaved pair is one paired sample, and the spread of those samples is
/// the honest error bar. Returns (median, min, max) of `b - a` in microseconds.
fn time_us_paired_delta<A: FnMut() -> usize, B: FnMut() -> usize>(
    runs: usize,
    iterations: usize,
    mut a: A,
    mut b: B,
) -> (f64, f64, f64) {
    let mut deltas = Vec::with_capacity(runs);
    for _ in 0..runs {
        let a_us = time_us(iterations, &mut a).0;
        let b_us = time_us(iterations, &mut b).0;
        deltas.push(b_us - a_us);
    }
    deltas.sort_by(|x, y| x.partial_cmp(y).expect("no NaN timings"));
    (deltas[runs / 2], deltas[0], deltas[runs - 1])
}

/// Price the prewarm's clock bracket.
///
/// WHAT IS MEASURED: two `read_authenticated_scheduler_clock` calls, which is
/// what the bracket adds to a frame in ENGINE READS. It is deliberately NOT
/// "the whole change": `domain47_apply_prewarm_clock_verdict` and
/// `record_clock_bracket` are also on that path, and they are branch + enum
/// store + one uncontended mutex push, which this does not attempt to price.
///
/// TWO CACHE STATES, because production's is not knowable from here.
/// `range_confirmed_good` keeps a one-entry `REGION_MRU` plus a per-epoch
/// sorted span list, and `page_cache_new_frame` (which bumps the epoch and so
/// clears the list) is called ONCE PER FRAME by the capture loop — the prewarm
/// does not call it. So between the opening and closing clock reads:
///
///  * `warm` — the prewarm's walk displaces the one-entry MRU but the clock
///    region's span entry survives, so the closing read is an MRU miss plus a
///    span-list hit. Modelled by a read into a DIFFERENT committed region.
///  * `cold` — the prewarm touched enough distinct regions to hit
///    `REGION_CACHE_CAP` (4096), which CLEARS the whole span list, so the
///    closing read pays a fresh `VirtualQuery`. Modelled by an epoch bump.
///
/// Which one a live prewarm produces depends on how many distinct regions it
/// touches, which no artifact in this repo measures. Both are printed and the
/// truth is between them; quoting only one would be a claim this bench cannot
/// support. (An earlier version quoted only the `cold` figure and called it
/// "what the bracket adds" — that overstated it.)
///
/// WHAT THE DIFFERENCE IS, EXACTLY. Not "two clock reads in isolation": it is
/// two clock reads TOGETHER WITH the cache interaction they have with the work
/// between them, which is what a frame actually pays. Both arms open with the
/// same bare read of the singleton slot, so the epoch transition `time_us`
/// forces happens on the same side of the displacement read in both — without
/// that prologue the absent arm's displacement met a STALE epoch (clear +
/// VirtualQuery) while the present arm's met a CURRENT one (span miss +
/// VirtualQuery), and that asymmetry, not the clock reads, was part of the
/// difference.
///
/// The fake heap has two regions where the live heap has many, so the span
/// list this searches is shorter than production's: that understates the warm
/// figure. The error direction is therefore MIXED, not one-sided.
///
/// PRECISION, HONESTLY. At this magnitude the per-pair deltas are at or below
/// this machine's noise floor while other lanes build on it: observed
/// individual `warm` pairs have come back NEGATIVE (-0.922 us — the present arm
/// "faster" than its own baseline) and absurdly high (32.038 us) in the same
/// session. The MEDIAN of the nine pairs is stable across runs (0.117-0.216 us
/// over four); the printed RANGE is not a confidence interval and must not be
/// read as one. Quote the median, name the run count, and treat the whole
/// result as an order of magnitude rather than a figure.
///
/// The number to compare against is t17's own measured phase:
/// `acceptance_telemetry.prewarm_time_us` avg 21_784 us / p50 25_155 us over
/// 505 samples. Nothing is asserted on absolute timing — this machine runs
/// several lanes at once and such an assertion would be a flake.
#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_prewarm_clock_bracket() {
    let world = clock_bench_world();
    let elsewhere = world.elsewhere_addr;
    let slot = world.slot_addr;
    // THE SHARED PROLOGUE. `time_us` bumps the epoch before each body, and
    // whichever read runs first pays the transition. Without this the two arms
    // paid it in different places and the displacement read below met a
    // different cache state in each, which put an unrelated term in the
    // difference. One bare read of the slot moves that transition ahead of the
    // displacement in BOTH arms.
    let prologue = || read_u64(slot).map_or(0, |value| value as usize);
    let warm_absent = move || {
        let primed = prologue();
        primed + read_u64(elsewhere).map_or(0, |value| value as usize)
    };
    let warm_present = move || {
        let primed = prologue();
        let opening = read_authenticated_scheduler_clock(world.module_base, world.date_addr);
        let displaced = read_u64(elsewhere).map_or(0, |value| value as usize);
        let closing = read_authenticated_scheduler_clock(world.module_base, world.date_addr);
        primed
            + opening.map_or(0, |(hour, _)| hour as usize)
            + closing.map_or(0, |(hour, _)| hour as usize)
            + displaced
    };
    let cold_absent = || {
        page_cache_new_frame();
        0usize
    };
    let cold_present = || {
        let opening = read_authenticated_scheduler_clock(world.module_base, world.date_addr);
        page_cache_new_frame();
        let closing = read_authenticated_scheduler_clock(world.module_base, world.date_addr);
        opening.map_or(0, |(hour, _)| hour as usize) + closing.map_or(0, |(hour, _)| hour as usize)
    };
    // A fixture the reader refuses would make both present arms measure the
    // refusal path, which returns before the second stable read.
    page_cache_new_frame();
    assert!(
        read_authenticated_scheduler_clock(world.module_base, world.date_addr).is_some(),
        "clock fixture must authenticate, or the present arms measure the refusal path"
    );
    // The displacing read must SUCCEED, or the warm arm measures a rejection
    // and never displaces anything.
    assert!(
        read_u64(elsewhere).is_some(),
        "the displacing read must reach committed memory"
    );

    let (warm, warm_lo, warm_hi) = time_us_paired_delta(
        9,
        BENCH_ITERATIONS,
        warm_absent,
        warm_present,
    );
    let (cold, cold_lo, cold_hi) = time_us_paired_delta(
        9,
        BENCH_ITERATIONS,
        cold_absent,
        cold_present,
    );
    println!(
        "d47_cost AB prewarm_clock_bracket (median of paired deltas, 9 interleaved runs): \
         two clock reads plus their cache interaction with the work between them cost \
         {warm:.3} us [{warm_lo:.3}, {warm_hi:.3}] with the region span cache INTACT, and \
         {cold:.3} us [{cold_lo:.3}, {cold_hi:.3}] in an observed run of the CLEARED-cache \
         scenario (that one is not a stable constant — quote it with its range). A live \
         prewarm is somewhere between. Against t17's measured phase \
         (21784 us avg / 25155 us p50) that is {:.5}%-{:.5}% of it.",
        warm / 21_784.0 * 100.0,
        cold / 21_784.0 * 100.0
    );
}
