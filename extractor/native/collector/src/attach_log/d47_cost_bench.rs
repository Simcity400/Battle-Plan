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
    fn VirtualProtect(
        address: *mut c_void,
        size: usize,
        new_protect: Dword,
        old_protect: *mut Dword,
    ) -> Bool;
}
const BENCH_PAGE_READONLY: Dword = 0x02;
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
    ///
    /// THE STRIDE HAS A FLOOR (lane EJ, 2026-08-07). It used to be
    /// `size.next_power_of_two()`, which is a stride only relative to arenas
    /// of the SAME size: every arena starts probing at one shared base, so a
    /// single ~110 MiB fixture reserved across all 64 of a 64 KiB arena's
    /// slots and the small arena panicked — but only when the two ran
    /// concurrently, so it reproduced under `cargo test` and never alone. A
    /// 256 MiB floor steps a small arena past a large neighbour in one slot
    /// while leaving every large arena's stride exactly as it was.
    fn in_heap_window(size: usize) -> BenchArena {
        const MIN_ARENA_SLOT: usize = 0x1000_0000;
        let stride = size.next_power_of_two().max(MIN_ARENA_SLOT);
        for slot in 0..64usize {
            let want = (HEAP_LO + 0x4000_0000 + slot * stride) as *mut c_void;
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
                // Must match the body written above, or the frame walk
                // refuses every root as recycled and the benchmarks time the
                // refusal path instead of the read path.
                char_id: idx as u32,
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
        let character_idx = idx % T15_FRAME_CHARACTERS;
        country_leader_roots.push(CountryLeaderRoleRoot {
            country_leader_addr: addr as u64,
            // Must be the char id the character body carries, or the walk
            // refuses the role as recycled.
            char_id: character_idx as u32,
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
/// `domain47_read_exact_manager_roster` + the prewarm's root build: two
/// `read_character_census_fields` observations (each one guarded body copy
/// serving both the torn-detecting raw fields and the name-token id) and the
/// two per-character strings.
fn census_loop(world: &BenchWorld) -> usize {
    let mem = Domain47InProcessMemory;
    let mut sink = 0usize;
    for addr in &world.census_characters {
        let name_token_addr = *addr + DOMAIN47_CCHARACTER_NAME_TOKEN_ID_OFF;
        let (first, opening) = domain47::extractor::read_character_census_fields(
            &mem,
            &world.roots,
            *addr as u64,
            || read_u32_stable(name_token_addr),
        );
        let (second, closing) = domain47::extractor::read_character_census_fields(
            &mem,
            &world.roots,
            *addr as u64,
            || read_u32_stable(name_token_addr),
        );
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

/// The census loop body with the name-token bracket read SEPARATELY, as it
/// stood before lane AK: `read_u32_stable` twice — four guarded reads — on top
/// of the two body copies, for a `u32` those copies already contained.
///
/// Everything else is identical, including the two string reads, so the A/B
/// isolates the four reads and nothing else.
fn census_loop_separate_token_bracket(world: &BenchWorld) -> usize {
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

/// Guarded reads per timed pass in [`guarded_read_calls`]: one whole
/// `CCharacter` body window per character, which is the read shape the census
/// actually issues (`read_object_window` -> `read_into`).
const GUARDED_READ_CALLS_PER_PASS: usize = T15_CENSUS_CHARACTERS;

/// `GUARDED_READ_CALLS_PER_PASS` successful `read_into` calls and nothing else.
///
/// WHAT THIS EXISTS TO PRICE (lane ED, 2026-08-07). `read_scalar`, `read_bytes`
/// and `read_into` each carry one `#[cfg(feature = "range-tracer")]`
/// `note_read` call. With the feature OFF nothing is emitted; with it ON the
/// call is emitted and executed on EVERY successful guarded read, even after
/// the tracer has recorded its one frame and disarmed — `note_read`'s first
/// statement is the `CAPTURING` load that then returns. The residual cost of
/// that always-taken call is what a capture taken on a feature-enabled DLL
/// pays for its whole length, and it is invisible in the sidecar because the
/// sidecar only describes the one traced frame.
///
/// Priced per CALL, not per byte: the disarmed path returns before it looks at
/// `len` or at `bytes`, so the overhead is a function of the read COUNT alone.
/// That is why the arm issues one fixed-width read per iteration and reports
/// its own call count.
///
/// THE LIVE EXTRAPOLATION IS AN ESTIMATE, AND HERE IS WHY (Codex 019fdb7e,
/// finding 1). Multiplying this figure by a phase's read count assumes two
/// things this arm does not establish. First, that the per-call residual is the
/// same at every one of the three call sites: `read_scalar` passes a stack
/// array, `read_bytes` a freshly allocated `Vec`, `read_into` the caller's
/// buffer, and each caller's code generation around the call differs. Second,
/// that the read count used is the phase's. The only live count available is
/// the range census's UNBRACKETED total, which is every read taken outside a
/// worker's clock bracket — the prewarm dominates it but does not exhaust it.
/// So a figure derived this way bounds an order of magnitude; it is not a
/// measurement of the phase.
fn guarded_read_calls(world: &BenchWorld) -> usize {
    let mut sink = 0usize;
    let mut buf = [0u8; CHARACTER_BODY_WINDOW];
    for addr in &world.census_characters {
        if read_into(*addr, &mut buf) {
            sink += buf[0] as usize + 1;
        }
    }
    sink
}

/// The `CCharacter` window width the census copies per observation. Any width
/// in `read_into`'s domain prices the same call overhead; this one keeps the
/// arm's cache behaviour close to the census's.
const CHARACTER_BODY_WINDOW: usize = 0xB0;

/// Whether the tracer's residual MULTIPLIES in the regime the live prewarm is
/// actually in — NOT a second per-call figure.
///
/// WHY IT IS NOT A PER-CALL FIGURE, AND WHY THE NAME SAYS SO (Codex
/// 019fdb7e, finding 4). This arm times the whole production census BODY, and
/// that body issues several guarded reads per character — two
/// `read_character_census_fields` observations plus the two string reads, whose
/// heap-backed half takes a payload read as well. Dividing its microseconds by
/// the character count therefore yields ns PER CHARACTER, and dividing that by
/// a read count nobody counted here would be arithmetic on an estimate. Only
/// [`d47_cost_guarded_read_call`] issues one read per iteration and may be read
/// per call.
///
/// WHAT IT DOES DECIDE. A `note_read` call sits between the guarded copy and
/// the caller's use of the bytes. On a cache-hot contiguous arena a read costs
/// a few nanoseconds and there is no outstanding-miss parallelism for an
/// intervening call to spoil, so that fixture cannot expose a cost that comes
/// from losing it. The live prewarm is a scattered dependent walk where a read
/// costs tens of nanoseconds and keeping several misses in flight is most of
/// the throughput. If the tracer's cost multiplied there, this arm's
/// cross-build ratio would be much larger than the contiguous arm's; if it does
/// not, the contiguous per-call figure may be extrapolated. That is the whole
/// question it answers, and a ratio is enough to answer it.
///
/// One number per build; compare BETWEEN builds (`--features range-tracer` and
/// without), as the sibling explains.
#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_census_body_scattered() {
    let world = scattered_census_world();
    page_cache_new_frame();
    assert!(
        scattered_census_loop(&world, false) > 0,
        "scattered fixture read nothing"
    );
    let mut samples = Vec::with_capacity(9);
    for _ in 0..9 {
        samples.push(time_us(20, || scattered_census_loop(&world, false)).0);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).expect("no NaN timings"));
    let us = samples[samples.len() / 2];
    println!(
        "d47_cost census_body_scattered: {us:.1} us/pass over {} characters \
         ({:.1} ns/character, SEVERAL guarded reads each — compare builds by RATIO, \
         never divide this by a read count) range_tracer_compiled={}",
        T28_MANAGER_ENTRIES,
        us * 1000.0 / T28_MANAGER_ENTRIES as f64,
        cfg!(feature = "range-tracer"),
    );
}

/// Per-call cost of the traced read primitives, to be compared BETWEEN two
/// builds of this binary (`--features range-tracer` and without).
///
/// Not an A/B inside one process: the two arms differ by a compile-time `cfg`,
/// so the comparison is necessarily across binaries. Run both and divide the
/// microsecond difference by `calls`.
#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_guarded_read_call() {
    let world = bench_world();
    let sink = guarded_read_calls(&world);
    assert!(
        sink >= GUARDED_READ_CALLS_PER_PASS,
        "read arm did no successful guarded reads: sink={sink}"
    );
    let (us, _) = time_us_median(9, || guarded_read_calls(&world));
    println!(
        "d47_cost guarded_read_call: {us:.1} us/pass over {} successful read_into calls \
         ({:.2} ns/call) range_tracer_compiled={}",
        GUARDED_READ_CALLS_PER_PASS,
        us * 1000.0 / GUARDED_READ_CALLS_PER_PASS as f64,
        cfg!(feature = "range-tracer"),
    );
}

#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_census_loop() {
    let world = bench_world();
    let (us, sink) = time_us_median(5, || census_loop(&world));
    println!(
        "d47_cost census_loop range_tracer_compiled={}",
        cfg!(feature = "range-tracer")
    );
    println!(
        "d47_cost census_loop: {:.1} us/frame over {} characters ({:.1} ns/character) sink={}",
        us,
        T15_CENSUS_CHARACTERS,
        us * 1000.0 / T15_CENSUS_CHARACTERS as f64,
        sink
    );
}

/// Elements staged on a portrait-BEARING character in the A/B below. Two is the
/// shape a character with a large and a small portrait of one role carries; the
/// engine's ceiling is twelve.
const PORTRAIT_BENCH_ELEMENTS: usize = 2;

fn portrait_bench_vocabulary() -> Domain47PortraitVocabulary {
    Domain47PortraitVocabulary {
        roles: [
            "army".to_string(),
            "civilian".to_string(),
            "navy".to_string(),
            "operative".to_string(),
            "political".to_string(),
            "council".to_string(),
        ],
        sizes: ["large".to_string(), "small".to_string()],
    }
}

/// Point the first `bearers` census characters at a staged element array.
///
/// The arena must be `in_heap_window`: `domain47_character_portrait_inputs`
/// gates the vector's data pointer through `is_map_control_heap_pointer`, and a
/// fixture the allocator placed elsewhere would be REFUSED at the header — the
/// bench would then be timing the refusal rather than the walk.
fn stage_portrait_maps(world: &BenchWorld, arena: &BenchArena, bearers: usize) {
    let entry_len = domain47::extractor::CCHARACTER_PORTRAIT_ENTRY_LEN;
    let stride = entry_len * PORTRAIT_BENCH_ELEMENTS;
    for (index, addr) in world.census_characters.iter().take(bearers).enumerate() {
        let elements = arena.base + index * stride;
        for ordinal in 0..PORTRAIT_BENCH_ELEMENTS {
            let entry = elements + ordinal * entry_len;
            arena.put_u32(
                entry + domain47::extractor::CCHARACTER_PORTRAIT_ENTRY_TYPE_OFF,
                0,
            );
            arena.put_u32(
                entry + domain47::extractor::CCHARACTER_PORTRAIT_ENTRY_SIZE_OFF,
                ordinal as u32,
            );
            arena.put_bytes(
                entry + domain47::extractor::CCHARACTER_PORTRAIT_ENTRY_SET_OFF,
                &[1u8],
            );
            arena.put_u32(
                entry + domain47::extractor::CCHARACTER_PORTRAIT_ENTRY_KIND_OFF,
                domain47::extractor::PORTRAIT_ICON_KIND_GFX,
            );
            // SSO, like the engine's own short `GFX_` keys, so the walk pays
            // the element copy and no separate heap fetch.
            arena.put_msvc_string(
                entry + domain47::extractor::CCHARACTER_PORTRAIT_ENTRY_GFX_OFF,
                0,
                "GFX_p",
            );
        }
        // The vector HEADER lives in the character body, which belongs to the
        // world's arena; only the element array is in the heap-window one.
        world._arena.put_u64(
            *addr + domain47::extractor::CCHARACTER_PORTRAIT_MAP_DATA_OFF as usize,
            elements as u64,
        );
        world._arena.put_u32(
            *addr + domain47::extractor::CCHARACTER_PORTRAIT_MAP_COUNT_OFF as usize,
            PORTRAIT_BENCH_ELEMENTS as u32,
        );
    }
}

/// [`census_loop`] with the portrait read where production puts it: inside the
/// same per-character body, while that character's line is still hot.
///
/// Measuring it as a second pass over the roster would charge it every cache
/// miss the census already paid for, which is the opposite of conservative.
fn census_loop_with_portraits(
    world: &BenchWorld,
    vocabulary: &Domain47PortraitVocabulary,
) -> usize {
    let mem = Domain47InProcessMemory;
    let mut sink = 0usize;
    for addr in &world.census_characters {
        let name_token_addr = *addr + DOMAIN47_CCHARACTER_NAME_TOKEN_ID_OFF;
        let (first, opening) = domain47::extractor::read_character_census_fields(
            &mem,
            &world.roots,
            *addr as u64,
            || read_u32_stable(name_token_addr),
        );
        let (second, closing) = domain47::extractor::read_character_census_fields(
            &mem,
            &world.roots,
            *addr as u64,
            || read_u32_stable(name_token_addr),
        );
        let (name, token) = domain47_character_root_strings(*addr);
        sink += opening.is_some() as usize
            + closing.is_some() as usize
            + first.is_ok() as usize
            + second.is_ok() as usize
            + name.map_or(0, |v| v.len())
            + token.map_or(0, |v| v.len());
        sink += match domain47_character_portrait_inputs(*addr, 7, 11, vocabulary) {
            Ok(rows) => rows.len(),
            Err(_) => 1,
        };
    }
    sink
}

/// WHAT THE PORTRAIT FEED ADDS TO THE SERIAL PREWARM — task #147's residual.
///
/// t32 (before the feed) measured a 20.9 ms d47 prewarm and t36 (after it)
/// 31.6 ms, and nothing separated the feed's own cost from everything else that
/// changed between those captures. This is that separation: the same census
/// loop with and without the per-character portrait read, alternating run by
/// run, medians reported.
///
/// TWO CONFIGURATIONS, because the live bearer share is NOT on the wire — the
/// portrait family has been dark on every prewarm since the feed landed, so
/// `domain47_prewarm_portrait_rows` is `0` in every retained capture and cannot
/// say how many characters carry a map:
///
///  * FLOOR — no character carries a map. Every character still pays the
///    `read_i32_stable` count bracket (two guarded reads) before the reader
///    returns the engine's own empty case, so this is the cost that cannot be
///    avoided by any character having no portrait. It IS a floor: nothing live
///    can pay less.
///  * A SCENARIO — every character carries a two-element map with SSO
///    identities. Deliberately NOT called a ceiling, and no interval is drawn
///    from it. The engine's structural maximum is TWELVE elements, and a
///    heap-backed identity costs a fetch this fixture's SSO strings do not, so
///    a live world can exceed this number in both directions at once. (Codex
///    019fdc5d, finding 4.)
///
/// WHAT TRANSFERS is the per-ELEMENT cost the scenario yields, because the live
/// total is the bearer share times the elements each bearer carries times that
/// cost, plus one guarded fetch per non-SSO identity. Two of those three factors
/// are unmeasurable until a capture publishes a non-zero
/// `domain47_prewarm_portrait_rows`, which is exactly what this lane's
/// reclassification is for.
///
/// Conservative in the module header's direction on both arms.
///
/// # CAN THIS RUN OFF THE SERIAL PREWARM? Three options, none taken (lane ER)
///
///  * FOLD IT INTO THE CENSUS WALK — already done. The read sits inside the
///    census loop's per-character body precisely so the portrait header lands in
///    the line that body copy already fetched; the comment at the call site says
///    so. Nothing is left to move.
///  * SERVE THE COUNT FROM THE CENSUS SNAPSHOT, the way lane AK merged the
///    name-token bracket. `CCHARACTER_PORTRAIT_MAP_COUNT_OFF + 4` is exactly
///    `CCHARACTER_RAW_FIELDS_SPAN`, so the census body copy already contains the
///    count and the reader's `read_i32_stable` re-reads it for nothing. MEASURED
///    AND REJECTED: that is the whole of the FLOOR arm, which is at or below
///    this machine's run-to-run noise. Two guarded reads per character buy back
///    nothing here because they hit the same hot line.
///  * DEFER THE WALK TO THE WORKER PHASE. This is the only one with real
///    milliseconds in it, and it is NOT a bounded move. The portraits are part
///    of the adopted ROOT set the frame walk consumes, and every element
///    snapshot is deliberately taken, decoded and dropped inside the prewarm
///    that adopts the root — that is what makes a portrait row provably live in
///    the hour it is labelled with. A worker-phase read is an engine read off
///    the collector thread, which is a custody and snapshot-law question rather
///    than a d47 one, and it would move the read across the hour boundary the
///    prewarm exists to pin. Design-noted for whoever owns that decision; not
///    attempted here.
///
/// The element double-read is not a candidate either: `read_element` runs twice
/// the vector header, so a single read cannot tell a stable element from one
/// that changed under it. Halving the per-element cost by dropping it would buy
/// the milliseconds back by making the rows untrustworthy.
#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_census_portrait_ab() {
    let world = bench_world();
    let vocabulary = portrait_bench_vocabulary();

    let (with_floor, without_floor) = time_us_median_ab(
        9,
        || census_loop_with_portraits(&world, &vocabulary),
        || census_loop(&world),
    );
    println!(
        "d47_cost AB portrait FLOOR (no character carries a map): census {without_floor:.1} us \
         -> census+portraits {with_floor:.1} us ({:+.1} us, {:+.1}%) over {} characters \
         ({:.1} ns/character)",
        with_floor - without_floor,
        (with_floor - without_floor) / without_floor * 100.0,
        T15_CENSUS_CHARACTERS,
        (with_floor - without_floor) * 1000.0 / T15_CENSUS_CHARACTERS as f64,
    );

    let entry_len = domain47::extractor::CCHARACTER_PORTRAIT_ENTRY_LEN;
    let elements = BenchArena::in_heap_window(
        T15_CENSUS_CHARACTERS * entry_len * PORTRAIT_BENCH_ELEMENTS + 0x1000,
    );
    stage_portrait_maps(&world, &elements, T15_CENSUS_CHARACTERS);
    let (with_ceiling, without_ceiling) = time_us_median_ab(
        9,
        || census_loop_with_portraits(&world, &vocabulary),
        || census_loop(&world),
    );
    let delta = with_ceiling - without_ceiling;
    println!(
        "d47_cost AB portrait SCENARIO (every character carries {PORTRAIT_BENCH_ELEMENTS} \
         SSO elements; NOT a ceiling -- the engine's structural max is {} and a heap-backed \
         identity costs more): census {without_ceiling:.1} us -> census+portraits \
         {with_ceiling:.1} us ({:+.1} us, {:+.1}%) over {} characters ({:.1} ns/element)",
        domain47::extractor::MAX_CHARACTER_PORTRAIT_ENTRIES,
        delta,
        delta / without_ceiling * 100.0,
        T15_CENSUS_CHARACTERS,
        delta * 1000.0 / (T15_CENSUS_CHARACTERS * PORTRAIT_BENCH_ELEMENTS) as f64,
    );
    println!(
        "d47_cost AB portrait: the t32->t36 prewarm lift to attribute is ~10700 us. The \
         count bracket on a portrait-LESS character costs {:.1} us across the whole census, \
         at or below this machine's run-to-run noise, so the feed's serial cost is \
         essentially (bearers x elements each x {:.1} ns) plus one guarded fetch per \
         non-SSO identity. Bearers and elements-each are not on the wire while the family \
         is dark; this bench cannot close the attribution alone.",
        (with_floor - without_floor).max(0.0),
        delta * 1000.0 / (T15_CENSUS_CHARACTERS * PORTRAIT_BENCH_ELEMENTS) as f64,
    );
}

/// In-process A/B of lane AK's census change: the name-token bracket served
/// out of the body copies that were already being taken, against the four
/// separate guarded reads it replaced.
///
/// WHY THIS IS THE PREWARM'S BIGGEST AVAILABLE CUT. The d47 prewarm is 23.6 ms
/// of t21's 89.7 ms frame build (p50), all of it SERIAL on the collector thread
/// before any worker spawns, and every millisecond of it is a millisecond of
/// hour-boundary exposure for all 68 domains. The census is its widest loop —
/// 13,405 characters per frame at t15's roster width — and until now each of
/// those characters paid four guarded reads for one `u32` that the two body
/// copies beside them already contained.
///
/// Alternately run and reported as the median of paired deltas, for the reason
/// `time_us_median_ab` gives: this machine runs several lanes at once and the
/// difference of two independently-sorted medians is not itself an
/// observation.
///
/// CONSERVATIVE, in the direction the module header states: the fake heap is
/// one contiguous region so `range_confirmed_good` always hits its MRU entry,
/// while the live heap has many regions and a higher miss rate. Removing a
/// guarded call removes proportionally many chances to miss, so the live saving
/// is at least this and cannot be less.
#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_census_name_token_merge_ab() {
    let world = bench_world();
    let (now, legacy) = time_us_median_ab(
        9,
        || census_loop(&world),
        || census_loop_separate_token_bracket(&world),
    );
    println!(
        "d47_cost AB census_name_token: separate bracket {legacy:.1} us -> merged {now:.1} us \
         ({:+.1}%) over {} characters ({:.1} ns/character saved, 4 guarded reads each)",
        (now - legacy) / legacy * 100.0,
        T15_CENSUS_CHARACTERS,
        (legacy - now) * 1000.0 / T15_CENSUS_CHARACTERS as f64,
    );
}

/// In-process A/B of the roster index check's derived-set reuse: building the
/// two key sets once for both census passes, against rebuilding them for the
/// second.
///
/// Plain CPU, no engine reads on either arm — the lookup closure is a constant
/// here — so unlike the guarded-read arms this figure is not conservative in
/// either direction; it is what the live prewarm pays too.
///
/// The identities are t15's census width with the same shape the census
/// produces: one distinct name token per character, and the character addresses
/// the fixture laid out.
#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_roster_index_keys_reuse_ab() {
    let world = bench_world();
    let identities: Vec<(u32, usize)> = world
        .census_characters
        .iter()
        .enumerate()
        .map(|(ordinal, addr)| (ordinal as u32, *addr))
        .collect();
    let lookup = |_| Domain47ManagerIndexLookup::Absent;
    let reused = || {
        let keys = Domain47ManagerNameIndexKeys::from_identities(&identities);
        keys.check(lookup).distinct_name_token_count as usize
            + keys.check(lookup).distinct_name_token_count as usize
    };
    let rebuilt = || {
        domain47_manager_name_index_check(&identities, lookup).distinct_name_token_count as usize
            + domain47_manager_name_index_check(&identities, lookup).distinct_name_token_count
                as usize
    };
    let (now, legacy) = time_us_median_ab(9, reused, rebuilt);
    println!(
        "d47_cost AB roster_index_keys: rebuilt per pass {legacy:.1} us -> built once {now:.1} us \
         ({:+.1}%) over {} identities x 2 passes",
        (now - legacy) / legacy * 100.0,
        identities.len(),
    );
}

/// Sharing the KEYS must not share the LOOKUPS: the closing pass exists to
/// re-read the engine's derived name cache after the whole walk, and a pass
/// that replayed the opening pass's answers would be a check that cannot fail.
///
/// MUTATION: memoize `check`'s lookup results on
/// `Domain47ManagerNameIndexKeys` and this fails — the second pass performs no
/// probes and the roster is declared safe on the first pass's stale answers.
#[test]
fn the_closing_roster_index_pass_re_reads_the_engine() {
    let identities = [(7u32, 0x1000usize), (9, 0x2000), (11, 0x3000)];
    let keys = Domain47ManagerNameIndexKeys::from_identities(&identities);
    let mut probes = 0usize;
    assert!(keys
        .check(|_| {
            probes += 1;
            Domain47ManagerIndexLookup::Absent
        })
        .roster_is_safe());
    assert_eq!(probes, 3, "the opening pass probes every distinct name");

    // The SAME keys, and the engine has since started answering with a pointer
    // outside the roster. The closing pass must see that.
    let mut closing_probes = 0usize;
    assert!(
        !keys
            .check(|_| {
                closing_probes += 1;
                Domain47ManagerIndexLookup::Found(0xDEAD_BEEF)
            })
            .roster_is_safe(),
        "a closing pass that replayed the opening answers would call this safe"
    );
    assert_eq!(
        closing_probes, 1,
        "the closing pass probes for itself, and `all` stops at the first refusal"
    );
}

/// The name-token id the merged census read serves is the word at `0x18`, and
/// the whole `CCharacter` body window covers it.
///
/// WRITTEN AS A LITERAL, DELIBERATELY. The obvious version of this test — read
/// the token both ways and compare — CANNOT FAIL on the offset, and that is not
/// a hypothesis: the mutation run below was performed and the test passed.
/// Both readers resolve the offset through `CCHARACTER_NAME_TOKEN_ID_OFF`
/// (the collector's `DOMAIN47_CCHARACTER_NAME_TOKEN_ID_OFF` is a re-export of
/// it), and so does the fixture that writes the value, so moving the constant
/// moves the writer and both readers together and they keep agreeing on a byte
/// that is no longer the engine's.
///
/// So the offset is stated here independently. `0x18` is a pre-existing d47
/// offset — this lane did not derive it, it made it load-bearing for a WINDOW
/// read by serving it out of the raw-field body copy — and this is what makes
/// changing it a deliberate act.
///
/// MUTATION RUN: `CCHARACTER_NAME_TOKEN_ID_OFF` changed to `0x1C` and the suite
/// run in `--release`. OBSERVED FIRST TIME (before this assertion existed):
/// `test result: ok. 1 passed` — the equivalence check was vacuous for the
/// offset, which is why this assertion exists.
#[test]
fn d47_census_merged_and_separate_token_reads_observe_the_same_character() {
    /// The `CCharacter` name-token id offset, stated here and not imported.
    const NAME_TOKEN_ID_OFF: usize = 0x18;
    assert_eq!(
        DOMAIN47_CCHARACTER_NAME_TOKEN_ID_OFF, NAME_TOKEN_ID_OFF,
        "the census now serves the name-token id out of the CCharacter body \
         window, so moving this offset moves what the window is read for; \
         changing it must be deliberate"
    );
    assert!(
        NAME_TOKEN_ID_OFF + 4 <= domain47::extractor::CCHARACTER_RAW_FIELDS_SPAN,
        "the body window must cover the name-token id, or the merged read \
         cannot serve it"
    );

    let world = bench_world();
    let mem = Domain47InProcessMemory;
    let mut compared = 0usize;
    for addr in &world.census_characters {
        // By-address, at the LITERAL offset: independent of whatever the
        // production constant currently says.
        let literal_token = read_u32_stable(*addr + NAME_TOKEN_ID_OFF);
        let name_token_addr = *addr + DOMAIN47_CCHARACTER_NAME_TOKEN_ID_OFF;
        let (merged_fields, merged_token) = domain47::extractor::read_character_census_fields(
            &mem,
            &world.roots,
            *addr as u64,
            || read_u32_stable(name_token_addr),
        );
        let separate_token = read_u32_stable(name_token_addr);
        let separate_fields =
            domain47::extractor::read_character_raw_fields(&mem, &world.roots, *addr as u64);
        assert_eq!(
            merged_token, literal_token,
            "the merged read must serve the word at 0x18 at {addr:#x}"
        );
        assert_eq!(
            merged_token, separate_token,
            "merged and by-address name-token reads disagree at {addr:#x}"
        );
        assert!(
            merged_token.is_some(),
            "the fixture must produce a readable name-token id, or this test \
             compares two Nones"
        );
        assert_eq!(
            merged_fields, separate_fields,
            "merged and by-address raw-field reads disagree at {addr:#x}"
        );
        compared += 1;
    }
    assert_eq!(
        compared, T15_CENSUS_CHARACTERS,
        "every censused character must be compared"
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

/// WHAT A COPY-ONLY BRACKET WOULD HAVE TO GATHER FOR DOMAIN 47
/// (lane DB, 2026-08-06). Derived, not modelled: the per-object WINDOW is the
/// furthest field offset this lane actually reads out of each structure (the
/// `domain47::extractor` offset constants), and the per-frame COUNT is t28's
/// own wire (`sweep_span_audit --domain 47`, populated frames).
///
///   CCharacter        13_394 x 0xB0   =  2.248 MiB   (furthest field: unit
///                                                     leader ptr at 0xA8)
///   CUnitLeader        1_361 x 0xF50  =  5.088 MiB   (furthest field: the
///                                                     char-id copy at 0xF4C)
///   CAdvisor           8_615 x 0xA0   =  1.315 MiB
///   CCountryLeader     1_879 x 0x10   =  0.029 MiB
///   CUnitLeaderTrait     194 x 0x30   =  0.009 MiB
///   ------------------------------------------------
///   primary bodies                      8.688 MiB
///   heap string payloads               ~1.07 MiB in ~46_900 SEPARATE buffers
///
/// TWO NUMBERS DECIDE THE SHAPE, and neither is the megabytes:
///
///  * ~75_500 DISTINCT RANGES per frame before any coalescing. These are
///    individually heap-allocated engine objects and separately allocated
///    string buffers; they are not adjacent and cannot be gathered as a
///    handful of bulk copies. The 5.4M "rows" the domain publishes are
///    repeated computation over these objects, NOT unique bytes — reading a
///    row count as a byte count overstates the copy by an order of magnitude
///    in one direction and understates the RANGE count in the other.
///  * The CUnitLeader window is 3_920 bytes to reach ONE `u32` at 0xF4C, and
///    at 1_361 leaders that single decision is 5.09 MiB — 59% of the primary
///    bytes for a structure that is 10% of the objects. Copying the four
///    sub-windows this lane actually reads (0x00..0x60, 0x1B8, 0xDC8..0xDD8,
///    0xE60, 0xF4C) instead of one contiguous span is a ~16x difference on
///    the largest single term, so a copy-only conversion must be specified in
///    sub-windows or it will be dominated by bytes nothing reads.
///
/// This census is a bound on the COPY. It says nothing about the pointer
/// chasing needed to DISCOVER those addresses (party -> leader -> character,
/// character -> unit leader, leader -> trait vector, skill-source), which is
/// graph walking and stays in-bracket.
///
/// SPLIT DOMAIN 47'S SCHEDULED READ INTO OBSERVATION AND COMPUTE
/// (lane DB, 2026-08-06).
///
/// WHY THIS SPLIT AND NO OTHER. Under the copy-only bracket rule a clock
/// bracket may hold engine OBSERVATIONS and nothing else: everything that only
/// touches bytes this process already owns belongs after the bracket closes.
/// `read_native_scheduled_frame` (scheduled.rs) runs three phases back to back
/// inside the ONE `ReaderClockBracketGuard` that `spawn_gated` opens around the
/// whole worker, and only the first of them names an engine address:
///
///  1. `build_lane_frame_from_adapter_input_ref` — the only phase that reads
///     the game's memory.
///  2. `encode_frame_payload` — serialises the frame phase 1 just built.
///  3. `compact::decode_payload` — re-parses those same bytes to derive the
///     record's row and unit counts, its status/schema flags and its parse
///     errors.
///
/// Phases 2 and 3 cannot observe a torn read, cannot straddle an hour, and
/// cannot be misattributed, because they never touch the engine. Every
/// microsecond they cost is a microsecond the bracket is held open for no
/// observation, and it is exactly the quantity the copy-only conversion would
/// move out. This measures it rather than asserting it.
///
/// WHAT THIS IS NOT — three separate reasons, all of which inflate the
/// compute share rather than the observation share, so the printed percentage
/// is an UPPER bound on the live one and never a measurement of it:
///
///  * The absolute microseconds are this fixture's, on this machine, over a
///    CONTIGUOUS arena. The live heap is scattered and phase 1's cost there is
///    several times higher (see the module header), while phases 2 and 3 touch
///    only our own freshly written, cache-warm bytes and do not grow.
///  * The three phases are not timed as production runs them (Codex review
///    019fd6bf-a0af-7ca2-a974-880b6832bba6, OPEN 1). Each timed body also
///    DROPS what it built — phase 1 destroys the whole owned frame, phase 2
///    destroys a payload production hands onward — and `time_us` bumps the
///    page-cache epoch once per iteration where production bumps it once per
///    frame, outside this worker. So `total` is the sum of three
///    separately-timed bodies, not a reconstruction of one bracket.
///  * Consequently the RATIO between phases within a single run is the durable
///    reading (it survives a machine that is busy, which this one usually is);
///    the absolute numbers are not comparable across runs.
#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_read_bracket_observation_vs_compute() {
    let world = bench_world();
    let mem = Domain47InProcessMemory;

    let (walk_us, walk_sink) = time_us_median(5, || frame_walk_with(&world, &mem));

    // Phase 1's output, built once and reused, so phases 2 and 3 are timed
    // over the SAME frame the live read would encode rather than over a
    // fixture of their own.
    let frame = build_bench_frame(&world, &mem).frame;
    let (encode_us, _encode_sink) = time_us_median(5, || {
        domain47::extractor::encode_frame_payload(&frame).len()
    });
    let payload = domain47::extractor::encode_frame_payload(&frame);
    let (decode_us, decode_sink) = time_us_median(5, || {
        domain47::domain47_characters_leaders_cabinet_live::decode_payload(0, &payload)
            .row_count_total as usize
    });

    // A decode that parsed no row would make phase 3 look free for the wrong
    // reason, and would mean phases 1 and 2 had produced an empty frame.
    // Deliberately NOT asserted on the payload's length: a ROWLESS domain-47
    // payload still carries its fixed header, so `len() > 0` holds for a
    // frame that observed nothing and would pass here vacuously (Codex review
    // 019fd6bf-a0af-7ca2-a974-880b6832bba6, cosmetic). The decoded row count
    // is the only one of the two that can distinguish them.
    assert!(decode_sink > 0, "phase 3 decoded no rows");

    let total = walk_us + encode_us + decode_us;
    println!(
        "d47_cost read_bracket: observe(walk) {walk_us:.1} us | compute(encode) {encode_us:.1} us \
         | compute(decode gate) {decode_us:.1} us | total {total:.1} us -- compute is {:.1}% of \
         the bracket on this fixture ({} payload bytes, {decode_sink} rows decoded, \
         walk sink={walk_sink})",
        (encode_us + decode_us) / total * 100.0,
        payload.len(),
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
            char_id: root.char_id,
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

// ---------------------------------------------------------------------------
// The manager pointer vectors (lane DC, 2026-08-06).
//
// `domain47_read_exact_manager_roster` reads the character manager's two
// `T*[]` vectors THREE times per prewarm — opening, the opening re-read that
// proves the arrays stable before any CCharacter field is trusted, and the
// closing pass — and each read used to take every entry through
// `read_u64_stable`, i.e. TWO guarded 8-byte calls per entry. At t28's census
// width (`domain47_manager_historical_count` 2_690_856 / `record_count` 266 =
// 10_116, `domain47_manager_dynamic_count` 872_014 / 266 = 3_278) that is
// 13_394 entries x 2 reads x 3 passes = 80_364 guarded calls per prewarm, to
// move 321 KiB of bytes that are ADJACENT by construction.
//
// This is the one part of the d47 prewarm the contiguous bench arena models
// FAITHFULLY rather than optimistically: the live array is one heap
// allocation, so it is contiguous live too, and its region lookups hit the
// MRU entry live exactly as they do here. The scattered-object caveat in the
// module header does not apply to this measurement.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// A SCATTERED census fixture (lane DC, 2026-08-06).
//
// The module header states this bench's central limitation: its arena is one
// contiguous block with a 0x180 stride, so every object is on a page the last
// one warmed, `range_confirmed_good` always hits its MRU entry, and the
// hardware prefetcher predicts the whole walk. The live heap is the opposite —
// each `CCharacter` is an individual allocation in a multi-gigabyte heap, so
// the census is a gather over ~13_400 unrelated pages. That gap IS the
// measurement lane DB reported: 3.7 ms for the offline walk against 24.9 ms
// live, for the same reads over the same object counts.
//
// This fixture models part of that gap instead of merely noting it. Same
// production calls, same object count, but every object gets a 4 KiB page to
// itself — bodies on one page set, string buffers on a DISJOINT one — and the
// visit order is a deterministic shuffle.
//
// WHAT IT MODELS: page-level scatter. No two objects share a cache line, a
// page or a TLB entry, and the visit order has no stride, so the hardware
// prefetcher predicts nothing and each object costs its own TLB walk. That is
// the structural property of the live heap the contiguous arena erases.
//
// WHAT IT DOES NOT MODEL, and this is the honest limit (Codex 019fd6ea,
// finding 4): COLD DRAM. Only a few lines of each 4 KiB page are ever
// touched, so the real last-level footprint is a few MiB, not the 110 MiB the
// pages span — and on a host with a large L3 the timed passes stay largely
// cache-resident. `time_us` also warms the world once and then repeats it.
// So the penalty measured here is the TLB/locality term ALONE, with the DRAM
// term missing. The calibration test below prints it: 3.35x measured
// 2026-08-06, against lane DB's live-versus-offline 6.7x — about half the
// live penalty, reproduced. Everything measured on this fixture is therefore
// a LOWER bound on the live effect of a change that targets memory latency,
// never an estimate of it.
//
// It is not a live measurement and its absolute microseconds are this
// machine's; the durable reading is the RATIO between two arms timed on it
// back to back.
// ---------------------------------------------------------------------------

/// One 4 KiB page per object: enough that no two objects share a cache line,
/// a page, or a TLB entry, which is the live layout's effect.
const SCATTER_PAGE: usize = 0x1000;

struct ScatteredCensusWorld {
    _arena: BenchArena,
    roots: ResolvedTypeRoots,
    /// Character body addresses in VISIT order (the roster's order), which is
    /// unrelated to their address order.
    census_characters: Vec<usize>,
    /// Each character's string-buffer page, same order. Kept so the fixture's
    /// own structural claim — one object per page — is testable rather than
    /// asserted in a comment.
    string_buffers: Vec<usize>,
}

/// SplitMix64, so the shuffle is deterministic run to run and the two arms of
/// an A/B always walk the identical address sequence.
fn scatter_rng(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn scattered_census_world() -> ScatteredCensusWorld {
    let count = T28_MANAGER_ENTRIES;
    // TWO disjoint page sets: `count` pages of bodies followed by `count`
    // pages of string buffers. Putting a character's strings on ANOTHER
    // character's body page (the first shape of this fixture) meant every
    // used page was shared by two objects, which is exactly the spatial
    // locality the fixture exists to remove.
    let arena = BenchArena::in_heap_window(2 * count * SCATTER_PAGE + SCATTER_PAGE);
    let string_base = arena.base + count * SCATTER_PAGE;

    // A real permutation (Fisher-Yates), not a multiplicative sequence: a
    // constant index stride is a constant ADDRESS stride, which a strided
    // hardware prefetcher follows just as happily as a sequential one and
    // would quietly turn this fixture back into the contiguous case.
    let mut pages: Vec<usize> = (0..count).collect();
    let mut state = 0x5eed_d47_c0ffee_u64;
    for idx in (1..count).rev() {
        let swap = (scatter_rng(&mut state) % (idx as u64 + 1)) as usize;
        pages.swap(idx, swap);
    }
    // A second, independent shuffle for the string pages, so following a
    // string pointer is not a fixed displacement from the body that named it.
    let mut string_pages: Vec<usize> = (0..count).collect();
    let mut string_state = 0x5eed_d47_5721_1465_u64;
    for idx in (1..count).rev() {
        let swap = (scatter_rng(&mut string_state) % (idx as u64 + 1)) as usize;
        string_pages.swap(idx, swap);
    }

    let roots = ResolvedTypeRoots {
        ccharacter_vtable: BENCH_CCHARACTER_VTABLE,
        carmy_leader_vtable: BENCH_CARMY_LEADER_VTABLE,
        cnavy_leader_vtable: BENCH_CNAVY_LEADER_VTABLE,
        ccountry_leader_vtable: BENCH_CCOUNTRY_LEADER_VTABLE,
        cadvisor_vtable: BENCH_CADVISOR_VTABLE,
        cunit_leader_trait_vtable: BENCH_CUNIT_LEADER_TRAIT_VTABLE,
    };

    let mut census_characters = Vec::with_capacity(count);
    let mut string_buffers = Vec::with_capacity(count);
    for idx in 0..count {
        let addr = arena.base + pages[idx] * SCATTER_PAGE;
        // Each character's heap string buffers live on a page of their own in
        // the disjoint string half, because live they are separate
        // allocations that share nothing with the object that names them.
        let strings = string_base + string_pages[idx] * SCATTER_PAGE;
        arena.put_u64(addr, BENCH_CCHARACTER_VTABLE);
        arena.put_u32(addr + CCHARACTER_TYPE_OFF as usize, CCHARACTER_TYPE_ID);
        arena.put_u32(addr + CCHARACTER_ID_OFF as usize, idx as u32);
        arena.put_u32(addr + DOMAIN47_CCHARACTER_NAME_TOKEN_ID_OFF, idx as u32);
        arena.put_u32(
            addr + CCHARACTER_COUNTRY_TAG_DB_ID_OFF as usize,
            (idx % 200) as u32,
        );
        arena.put_u64(addr + CCHARACTER_UNIT_LEADER_PTR_OFF as usize, 0);
        // Half SSO, half heap-backed, the live mix.
        let name = if idx % 2 == 0 {
            format!("Name {idx}")
        } else {
            format!("A Rather Longer Character Name {idx}")
        };
        let token = format!("GER_character_token_{idx}");
        arena.put_msvc_string(addr + CCHARACTER_NAME_OFF as usize, strings, &name);
        arena.put_msvc_string(addr + CCHARACTER_TOKEN_OFF as usize, strings + 0x40, &token);
        census_characters.push(addr);
        string_buffers.push(strings);
    }

    ScatteredCensusWorld {
        _arena: arena,
        roots,
        census_characters,
        string_buffers,
    }
}

/// The production census body, verbatim, over a scattered world. `prefetch`
/// selects whether the loop issues the lookahead hint the collector issues.
fn scattered_census_loop(world: &ScatteredCensusWorld, prefetch: bool) -> usize {
    scattered_census_loop_at(
        world,
        prefetch.then_some(DOMAIN47_PREFETCH_DISTANCE),
    )
}

/// The same loop at an arbitrary lookahead, so the constant can be shown to
/// sit on a plateau rather than being asserted. `None` issues no hint.
fn scattered_census_loop_at(world: &ScatteredCensusWorld, distance: Option<usize>) -> usize {
    let mem = Domain47InProcessMemory;
    let mut sink = 0usize;
    for (ordinal, addr) in world.census_characters.iter().copied().enumerate() {
        if let Some(distance) = distance {
            if let Some(ahead) = world.census_characters.get(ordinal.wrapping_add(distance)) {
                domain47_prefetch_object(
                    *ahead,
                    domain47::extractor::CCHARACTER_RAW_FIELDS_SPAN,
                );
            }
        }
        let name_token_addr = addr + DOMAIN47_CCHARACTER_NAME_TOKEN_ID_OFF;
        let (first, opening) = domain47::extractor::read_character_census_fields(
            &mem,
            &world.roots,
            addr as u64,
            || read_u32_stable(name_token_addr),
        );
        let (second, closing) = domain47::extractor::read_character_census_fields(
            &mem,
            &world.roots,
            addr as u64,
            || read_u32_stable(name_token_addr),
        );
        let (name, token) = domain47_character_root_strings(addr);
        sink += opening.is_some() as usize
            + closing.is_some() as usize
            + first.is_ok() as usize
            + second.is_ok() as usize
            + name.map_or(0, |v| v.len())
            + token.map_or(0, |v| v.len());
    }
    sink
}

/// The SPLIT shape the prewarm actually had before lane DR: the census walks
/// every character body, the whole roster is validated and its two pointer
/// vectors re-read, and only THEN a second loop walks the same ~13_400 bodies
/// again to read the two strings that live inside them.
///
/// `scattered_census_loop` above — and every other census arm in this module —
/// models the MERGED loop, so every figure this instrument has ever printed
/// for "the census" understated production by exactly the difference these two
/// arms measure. The strings are the same reads in both arms and in the same
/// order; the only difference is WHEN they happen relative to the body copies
/// that already paid for their cache lines.
fn scattered_census_loop_two_pass(world: &ScatteredCensusWorld) -> usize {
    let mem = Domain47InProcessMemory;
    let mut sink = 0usize;
    for (ordinal, addr) in world.census_characters.iter().copied().enumerate() {
        if let Some(ahead) = world
            .census_characters
            .get(ordinal.wrapping_add(DOMAIN47_PREFETCH_DISTANCE))
        {
            domain47_prefetch_object(*ahead, domain47::extractor::CCHARACTER_RAW_FIELDS_SPAN);
        }
        let name_token_addr = addr + DOMAIN47_CCHARACTER_NAME_TOKEN_ID_OFF;
        let (first, opening) = domain47::extractor::read_character_census_fields(
            &mem,
            &world.roots,
            addr as u64,
            || read_u32_stable(name_token_addr),
        );
        let (second, closing) = domain47::extractor::read_character_census_fields(
            &mem,
            &world.roots,
            addr as u64,
            || read_u32_stable(name_token_addr),
        );
        sink += opening.is_some() as usize
            + closing.is_some() as usize
            + first.is_ok() as usize
            + second.is_ok() as usize;
    }
    // The second walk, with the same lookahead the production root-build loop
    // issues, over the same addresses in the same order.
    for (ordinal, addr) in world.census_characters.iter().copied().enumerate() {
        if let Some(ahead) = world
            .census_characters
            .get(ordinal.wrapping_add(DOMAIN47_PREFETCH_DISTANCE))
        {
            domain47_prefetch_object(*ahead, domain47::extractor::CCHARACTER_RAW_FIELDS_SPAN);
        }
        let (name, token) = domain47_character_root_strings(addr);
        sink += name.map_or(0, |v| v.len()) + token.map_or(0, |v| v.len());
    }
    sink
}

/// The same split, over the CONTIGUOUS fixture. Both fixtures are needed and
/// neither alone answers the question:
///
///  * the scattered fixture gives every character its own 4 KiB page, so its
///    ~55 MiB working set guarantees the second walk misses to DRAM — an UPPER
///    bound on the split's cost;
///  * the contiguous fixture packs bodies at `CHARACTER_STRIDE`, giving a
///    ~3.2 MiB working set, which is the same order as the live heap's ~2.3 MiB
///    of `CCharacter` bodies — so it models the case where the second walk hits
///    L3 rather than DRAM, and is the LOWER bound.
///
/// The live answer is between them, and both are reported.
fn census_loop_two_pass(world: &BenchWorld) -> usize {
    let mem = Domain47InProcessMemory;
    let mut sink = 0usize;
    for addr in &world.census_characters {
        let name_token_addr = *addr + DOMAIN47_CCHARACTER_NAME_TOKEN_ID_OFF;
        let (first, opening) = domain47::extractor::read_character_census_fields(
            &mem,
            &world.roots,
            *addr as u64,
            || read_u32_stable(name_token_addr),
        );
        let (second, closing) = domain47::extractor::read_character_census_fields(
            &mem,
            &world.roots,
            *addr as u64,
            || read_u32_stable(name_token_addr),
        );
        sink += opening.is_some() as usize
            + closing.is_some() as usize
            + first.is_ok() as usize
            + second.is_ok() as usize;
    }
    for addr in &world.census_characters {
        let (name, token) = domain47_character_root_strings(*addr);
        sink += name.map_or(0, |v| v.len()) + token.map_or(0, |v| v.len());
    }
    sink
}

/// What the prewarm's two-walk shape COSTS, on both fixtures.
///
/// AN INSTRUMENT, NOT A GATE: it prints a ratio and asserts only that both
/// arms read the same bytes, which is the property that makes the comparison
/// meaningful (a split arm that read less would be timing a different job).
#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_census_single_vs_two_pass_ab() {
    let contiguous = bench_world();
    assert_eq!(
        census_loop(&contiguous),
        census_loop_two_pass(&contiguous),
        "the split arm must read exactly what the merged arm reads"
    );
    let (merged_flat, split_flat) = time_us_median_ab(
        9,
        || census_loop(&contiguous),
        || census_loop_two_pass(&contiguous),
    );
    println!(
        "d47_cost AB census passes (contiguous, ~{:.1} MiB of bodies): two walks {split_flat:.1} us \
         -> one walk {merged_flat:.1} us ({:+.1}%) over {} characters",
        (T15_CENSUS_CHARACTERS * CHARACTER_STRIDE) as f64 / (1024.0 * 1024.0),
        (merged_flat - split_flat) / split_flat * 100.0,
        T15_CENSUS_CHARACTERS,
    );

    let scattered = scattered_census_world();
    page_cache_new_frame();
    assert_eq!(
        scattered_census_loop(&scattered, true),
        scattered_census_loop_two_pass(&scattered),
        "the split arm must read exactly what the merged arm reads"
    );
    let (merged_scatter, split_scatter) = time_us_median_ab_n(
        9,
        200,
        || scattered_census_loop(&scattered, true),
        || scattered_census_loop_two_pass(&scattered),
    );
    println!(
        "d47_cost AB census passes (scattered, one page per object): two walks {split_scatter:.1} \
         us -> one walk {merged_scatter:.1} us ({:+.1}%) over {} characters",
        (merged_scatter - split_scatter) / split_scatter * 100.0,
        T28_MANAGER_ENTRIES,
    );
}

// --- the half of the census that reads NOTHING (lane DR, 2026-08-07) -------
//
// Every arm above times ENGINE READS. None of them times the ordered
// containers the census drives beside those reads, and that omission is why
// the offline model and the live prewarm never reconciled: t32's scattered
// census models 7.1 ms against a 20.89 ms live p50.
//
// Per censused character the roster census also performs, with no memory read
// at all:
//   * `character_pointers.insert(addr)`         -- BTreeSet<usize>, duplicate-pointer gate
//   * `name_identities.push((token_id, addr))`  -- Vec
//   * `characters.push((addr, raw))`            -- Vec
// then, ONCE over the finished roster:
//   * `Domain47ManagerNameIndexKeys::from_identities` -- a SECOND BTreeSet<usize>
//     over the same 13_403 pointers, plus a BTreeSet<u32> of the distinct tokens
//   * `.check(..)` twice
// and then, per character again, in the root-build loop:
//   * `dedup.admit_character(..)`               -- BTreeMap<u32, _> probe + insert
//   * `next.characters.push(CharacterRoot { .. })`
//
// t32 widths, from `runs/live_unlock/t32/native_session_manifest.json` over
// `domain47_prewarm_ran` = 316: `domain47_manager_historical_count` 3_196_656
// and `_dynamic_count` 1_038_695 give 13_403 characters;
// `domain47_manager_distinct_name_token_count` 2_737_511 gives 8_663 distinct
// tokens.
//
// t32 also settles what the index probe costs: `domain47_manager_name_index_count`
// is 0 in every one of the 316 prewarms, and `domain47_manager_index_lookup`
// returns `Absent` on `header.count == 0` before issuing a single read. So the
// `check` passes are pure container work live, which is exactly what this arm
// times.
const T32_CENSUS_CHARACTERS: usize = 13_403;
const T32_DISTINCT_NAME_TOKENS: usize = 8_663;

/// The census's non-reading work, at t32 widths, driven through the
/// production containers and the production `admit_character`.
///
/// Identities are shaped the way the live roster is: `T32_DISTINCT_NAME_TOKENS`
/// distinct name tokens spread over `T32_CENSUS_CHARACTERS` characters, every
/// character at a distinct address with a distinct `char_id` (so `admit_character`
/// takes its `Insert` arm, as it does for essentially every live character —
/// t32's `domain47_adapter_deduplicated_character_rows` is 0).
fn census_bookkeeping(strings: bool) -> usize {
    census_bookkeeping_with(strings, true)
}

/// `reuse_roster_set` selects lane DR's change: `true` hands the census's own
/// duplicate-pointer `BTreeSet<usize>` to the name-index keys, `false` rebuilds
/// a second one from the identities the way `from_identities` does.
fn census_bookkeeping_with(strings: bool, reuse_roster_set: bool) -> usize {
    // `with_capacity(total)`, as production sizes them from the two vector
    // headers — `Vec::new()` here would time reallocations production does not
    // perform (Codex 019fd99d, finding 2).
    let mut character_pointers = BTreeSet::new();
    let mut name_identities: Vec<(u32, usize)> = Vec::with_capacity(T32_CENSUS_CHARACTERS);
    let mut characters: Vec<(usize, domain47::extractor::CharacterRawFields)> =
        Vec::with_capacity(T32_CENSUS_CHARACTERS);
    for index in 0..T32_CENSUS_CHARACTERS {
        // A plausible live spread of addresses: `CHARACTER_STRIDE` apart, which
        // is what makes the BTreeSet's comparisons realistic.
        let addr = 0x1_0000_0000usize + index * CHARACTER_STRIDE;
        // t32's `domain47_adapter_unit_leader_rows` 432_014 / 316 = 1_367 unit
        // leaders across 13_403 characters, so roughly one character in ten
        // carries a leader pointer and reaches `admit_leader`'s BTreeMap. A
        // fixture that left every pointer zero would exit that helper on its
        // first branch and time none of it.
        let unit_leader_ptr = if index % 10 == 0 {
            0x2_0000_0000u64 + index as u64 * 0x1000
        } else {
            0
        };
        domain47_admit_censused_character(
            &mut character_pointers,
            &mut name_identities,
            &mut characters,
            addr,
            (index % T32_DISTINCT_NAME_TOKENS) as u32,
            domain47::extractor::CharacterRawFields {
                character_addr: addr as u64,
                country_tag_db_id: (index % 200) as u32,
                char_id: index as u32,
                unit_leader_ptr,
            },
        );
    }
    // Taken before the set moves, so the reuse arm MOVES it exactly as
    // production does rather than cloning it — a clone here would hide most of
    // what the change saves.
    let censused_pointer_count = character_pointers.len();
    let keys = if reuse_roster_set {
        Domain47ManagerNameIndexKeys::from_roster(character_pointers, &name_identities)
    } else {
        Domain47ManagerNameIndexKeys::from_identities(&name_identities)
    };
    // `header.count == 0` is what t32 observed, so this is the live closure.
    let lookup = |_| Domain47ManagerIndexLookup::Absent;
    let opening = keys.check(lookup);
    let closing = keys.check(lookup);

    // Sources for the two owned strings, built ONCE and outside the loop.
    // Production's `name`/`token` come out of `read_msvc_string`, which
    // allocates a `String` and copies the decoded bytes into it — no formatting
    // machinery — so the loop below must do the same. `format!` here would have
    // charged this arm for integer-to-decimal conversion and `fmt` dispatch and
    // then reported the total as "allocator" (Codex 019fd99d, finding 3).
    const NAMES: [&str; 4] = [
        "Ivar Lykke",
        "A Rather Longer Character Name",
        "Wilhelm Keitel",
        "Konstantin Konstantinovich Rokossovsky",
    ];
    const TOKENS: [&str; 4] = [
        "NOR_ivar_lykke",
        "GER_a_rather_longer_character_token_value",
        "GER_wilhelm_keitel",
        "SOV_konstantin_rokossovsky",
    ];

    let mut dedup = Domain47CharacterDedup::default();
    // `Vec::new()`, as `NativeRootCache::default()` leaves it: production does
    // not pre-size the adopted root list either.
    let mut roots: Vec<CharacterRoot> = Vec::new();
    for (addr, raw) in &characters {
        if let Domain47CharacterAdmit::Insert =
            dedup.admit_character(raw.char_id, raw.country_tag_db_id, raw.unit_leader_ptr, roots.len())
        {
            dedup.admit_leader(raw.unit_leader_ptr, raw.country_tag_db_id);
            // `strings` selects whether the two owned strings are built, so
            // their allocate-and-copy share is separable from the containers'.
            let (name, token) = if strings {
                let slot = raw.char_id as usize % NAMES.len();
                (
                    Some(NAMES[slot].to_string()),
                    Some(TOKENS[slot].to_string()),
                )
            } else {
                (None, None)
            };
            roots.push(CharacterRoot {
                character_addr: *addr as u64,
                char_id: raw.char_id,
                name,
                token,
            });
        }
    }
    censused_pointer_count
        + opening.distinct_name_token_count as usize
        + closing.distinct_name_token_count as usize
        + roots.len()
}

/// How much of the prewarm never touches the game at all.
///
/// AN INSTRUMENT, NOT A GATE: it prints microseconds and asserts only that the
/// arms did the work (a bookkeeping arm optimised away to nothing would report
/// zero and mean nothing).
#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_census_bookkeeping() {
    assert!(census_bookkeeping(true) > 0, "bookkeeping arm did nothing");
    let (with_strings, _) = time_us_median(5, || census_bookkeeping(true));
    let (without_strings, _) = time_us_median(5, || census_bookkeeping(false));
    println!(
        "d47_cost census bookkeeping (no engine reads at all, t32 widths): {with_strings:.1} us \
         with the two owned root strings, {without_strings:.1} us without them \
         ({:.1} us to allocate and copy them) over {} characters / {} distinct name tokens. \
         t32's live prewarm p50 is 20890 us.",
        with_strings - without_strings,
        T32_CENSUS_CHARACTERS,
        T32_DISTINCT_NAME_TOKENS,
    );

    let (reused, rebuilt) = time_us_median_ab(
        9,
        || census_bookkeeping_with(true, true),
        || census_bookkeeping_with(true, false),
    );
    println!(
        "d47_cost AB roster pointer set: rebuilt {rebuilt:.1} us -> census set reused \
         {reused:.1} us ({:+.1}%), one BTreeSet<usize> of {} entries removed from the serial \
         prewarm",
        (reused - rebuilt) / rebuilt * 100.0,
        T32_CENSUS_CHARACTERS,
    );
}

/// `time_us_median_ab` at a caller-chosen iteration count. The scattered
/// fixture is ~55 MiB per pass, so `BENCH_ITERATIONS` (200) would be minutes
/// per sample for no extra resolution.
fn time_us_median_ab_n<A: FnMut() -> usize, B: FnMut() -> usize>(
    runs: usize,
    iterations: usize,
    mut a: A,
    mut b: B,
) -> (f64, f64) {
    let mut a_samples = Vec::with_capacity(runs);
    let mut b_samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        a_samples.push(time_us(iterations, &mut a).0);
        b_samples.push(time_us(iterations, &mut b).0);
    }
    a_samples.sort_by(|x, y| x.partial_cmp(y).expect("no NaN timings"));
    b_samples.sort_by(|x, y| x.partial_cmp(y).expect("no NaN timings"));
    (a_samples[runs / 2], b_samples[runs / 2])
}

/// What the contiguous arena hides: the SAME census loop, over the same object
/// count, once per-object-on-its-own-page-in-shuffled-order and once
/// contiguous. This is the instrument's own calibration — run it before
/// quoting anything else measured on the scattered fixture.
///
/// AN INSTRUMENT, NOT A GATE. It prints a ratio and asserts only that the
/// fixture read something, so removing the scatter would not fail it. The
/// scatter itself is gated by
/// `d47_scattered_fixture_gives_every_object_its_own_page`, which is an
/// ordinary test. Wall-time assertions are deliberately absent: this machine
/// is usually also building something, and a flaky gate is worse than none
/// (Codex 019fd6ea, finding 6).
#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_census_scatter_penalty() {
    let contiguous = bench_world();
    let scattered = scattered_census_world();
    page_cache_new_frame();
    assert!(scattered_census_loop(&scattered, false) > 0, "fixture read nothing");

    let (flat, scatter) = time_us_median_ab_n(
        9,
        200,
        || census_loop(&contiguous),
        || scattered_census_loop(&scattered, false),
    );
    println!(
        "d47_cost census scatter penalty: contiguous {flat:.1} us -> scattered {scatter:.1} us \
         ({:.2}x) over {} vs {} characters. Lane DB's live/offline ratio for the whole prewarm \
         was 24873/3700 = 6.7x.",
        scatter / flat,
        T15_CENSUS_CHARACTERS,
        T28_MANAGER_ENTRIES,
    );
}

/// A/B of the lookahead hint the collector's census now issues, on the
/// fixture that can see it.
///
/// AN INSTRUMENT, NOT A GATE, for the same reason as its sibling above: it
/// still passes if the hint becomes a no-op. What IS gated is the property
/// that matters for correctness — that the hint changes nothing that is read
/// — by `d47_census_prefetch_does_not_change_what_is_read`.
#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_census_prefetch_ab() {
    let world = scattered_census_world();
    // Both arms must read the same characters, or one is timing refusals.
    page_cache_new_frame();
    let plain = scattered_census_loop(&world, false);
    page_cache_new_frame();
    let hinted = scattered_census_loop(&world, true);
    assert!(plain > 0, "fixture read nothing");
    assert_eq!(
        plain, hinted,
        "the hint changed what was read, which it cannot legally do"
    );

    let (without, with) = time_us_median_ab_n(
        9,
        200,
        || scattered_census_loop(&world, false),
        || scattered_census_loop(&world, true),
    );
    println!(
        "d47_cost AB census prefetch (scattered fixture): no hint {without:.1} us -> hinted \
         {with:.1} us ({:+.1}%) over {} characters at distance {}.",
        (with - without) / without * 100.0,
        T28_MANAGER_ENTRIES,
        DOMAIN47_PREFETCH_DISTANCE,
    );
}

/// Is `DOMAIN47_PREFETCH_DISTANCE` on a plateau or on a cliff? A constant
/// picked by argument is only as good as the shape of the curve around it, so
/// print the curve rather than asserting the point.
#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_census_prefetch_distance_sweep() {
    let world = scattered_census_world();
    let mut line = String::new();
    // 9 runs x 200 iterations. At 5 x 20 the whole curve sat inside the
    // run-to-run noise of a machine that is usually also building something,
    // and a curve you cannot separate from noise is not a curve.
    for distance in [None, Some(1), Some(2), Some(4), Some(8), Some(16), Some(32)] {
        let (us, sink) = time_us_median_n(9, 200, || scattered_census_loop_at(&world, distance));
        assert!(sink > 0, "fixture read nothing");
        line.push_str(&match distance {
            None => format!("none {us:.1} us | "),
            Some(distance) => format!("{distance} {us:.1} us | "),
        });
    }
    println!("d47_cost census prefetch distance sweep (scattered fixture): {line}");
}

fn time_us_median_n<F: FnMut() -> usize>(
    runs: usize,
    iterations: usize,
    mut body: F,
) -> (f64, usize) {
    let mut samples = Vec::with_capacity(runs);
    let mut sink = 0usize;
    for _ in 0..runs {
        let (us, s) = time_us(iterations, &mut body);
        samples.push(us);
        sink = sink.wrapping_add(s);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).expect("no NaN timings"));
    (samples[runs / 2], sink)
}

/// The scattered fixture's structural claim, tested rather than asserted in a
/// comment: every body and every string buffer is alone on its page, and the
/// visit order is a real permutation.
///
/// The first version of this fixture put each character's strings on ANOTHER
/// character's body page, so every used page was shared by two objects and
/// half the "scattered" reads were spatially local (Codex 019fd6ea, finding
/// 4). That was invisible without this test.
///
/// MUTATION: derive the string page from `pages` instead of `string_pages`
/// (i.e. put strings back on a body page) and the disjointness assertion
/// fails.
#[test]
fn d47_scattered_fixture_gives_every_object_its_own_page() {
    let world = scattered_census_world();
    let mut pages = BTreeSet::new();
    for addr in world
        .census_characters
        .iter()
        .chain(world.string_buffers.iter())
    {
        assert!(
            pages.insert(addr / SCATTER_PAGE),
            "two objects share a page, so the fixture is not scattered"
        );
    }
    assert_eq!(pages.len(), T28_MANAGER_ENTRIES * 2);
    // A permutation, not a shuffled prefix: every body address must be
    // distinct AND the visit order must not be the address order, or the
    // hardware prefetcher walks it sequentially.
    let mut ascending = world.census_characters.clone();
    ascending.sort_unstable();
    ascending.dedup();
    assert_eq!(ascending.len(), T28_MANAGER_ENTRIES);
    assert_ne!(
        ascending, world.census_characters,
        "the visit order is the address order, so nothing is being scattered"
    );
}

/// The hint must not change ONE value the census reads. Same fixture, same
/// order, both arms compared field for field rather than by a summed sink.
///
/// This is the property that makes the change legal at all: `_mm_prefetch`
/// has no architectural effect, so a difference here would mean the loop
/// bodies had diverged.
#[test]
fn d47_census_prefetch_does_not_change_what_is_read() {
    let world = scattered_census_world();
    let mem = Domain47InProcessMemory;
    let observe = |prefetch: bool| {
        page_cache_new_frame();
        let mut out = Vec::with_capacity(world.census_characters.len());
        for (ordinal, addr) in world.census_characters.iter().copied().enumerate() {
            if prefetch {
                domain47_prefetch_ahead(
                    &world.census_characters,
                    ordinal,
                    domain47::extractor::CCHARACTER_RAW_FIELDS_SPAN,
                );
            }
            let name_token_addr = addr + DOMAIN47_CCHARACTER_NAME_TOKEN_ID_OFF;
            let (fields, token_id) = domain47::extractor::read_character_census_fields(
                &mem,
                &world.roots,
                addr as u64,
                || read_u32_stable(name_token_addr),
            );
            let (name, token) = domain47_character_root_strings(addr);
            out.push((fields.ok(), token_id, name, token));
        }
        out
    };
    let without = observe(false);
    let with = observe(true);
    assert_eq!(without, with);
    // A fixture that resolved nothing would make the comparison vacuous.
    assert_eq!(without.len(), T28_MANAGER_ENTRIES);
    assert!(without[0].0.is_some());
    assert!(without[1].2.is_some());
}

/// t28's `domain47_manager_historical_count` / `record_count`.
const T28_MANAGER_HISTORICAL: usize = 10_116;
/// t28's `domain47_manager_dynamic_count` / `record_count`.
const T28_MANAGER_DYNAMIC: usize = 3_278;
const T28_MANAGER_ENTRIES: usize = T28_MANAGER_HISTORICAL + T28_MANAGER_DYNAMIC;

/// One contiguous pointer array of `T28_MANAGER_ENTRIES` entries, every entry
/// a distinct address inside the heap window (so `is_map_control_heap_pointer`
/// admits it and neither arm measures a refusal).
struct ManagerVectorBenchWorld {
    _arena: BenchArena,
    data: usize,
    size: u32,
}

/// A synthetic entry VALUE: distinct, 8-aligned and inside `[HEAP_LO,
/// HEAP_HI)`, so `is_map_control_heap_pointer` admits it.
///
/// The pointer VALUES are what the vector reader validates; the ARRAY itself
/// only has to be readable, and it is never dereferenced through these
/// entries. Synthesising the values lets these fixtures use an ordinary
/// `VirtualAlloc` instead of `BenchArena::in_heap_window`, whose fixed retry
/// slots start at `HEAP_LO + 0x4000_0000` and are all swallowed by the
/// 128 MiB scattered-census arena when the two run concurrently — which is
/// how the test runner runs them.
fn synthetic_entry(ordinal: usize) -> u64 {
    (HEAP_LO + 0x1000 * (ordinal + 1)) as u64
}

fn manager_vector_bench_world() -> ManagerVectorBenchWorld {
    let bytes = T28_MANAGER_ENTRIES * 8;
    let arena = BenchArena::new(bytes + 0x1000);
    let data = arena.base;
    for ordinal in 0..T28_MANAGER_ENTRIES {
        arena.put_u64(data + ordinal * 8, synthetic_entry(ordinal));
    }
    ManagerVectorBenchWorld {
        _arena: arena,
        data,
        size: T28_MANAGER_ENTRIES as u32,
    }
}

/// The entry loop as it was before this lane: one `read_u64_stable` — two
/// guarded calls — per entry.
fn manager_vector_entries_legacy(
    data: usize,
    size: u32,
) -> Result<Vec<usize>, Domain47ManagerRosterFailureKind> {
    let mut entries = Vec::with_capacity(size as usize);
    for ordinal in 0..size {
        let entry_addr = (ordinal as usize)
            .checked_mul(8)
            .and_then(|offset| data.checked_add(offset))
            .ok_or(Domain47ManagerRosterFailureKind::VectorEntryInvalid)?;
        let character_addr = read_u64_stable(entry_addr)
            .and_then(|value| usize::try_from(value).ok())
            .filter(|addr| is_map_control_heap_pointer(*addr))
            .ok_or(Domain47ManagerRosterFailureKind::VectorEntryInvalid)?;
        entries.push(character_addr);
    }
    Ok(entries)
}

/// In-process, run-by-run interleaved A/B of the manager pointer-vector read,
/// reported as ONE prewarm's worth (three passes over both vectors).
///
/// AN INSTRUMENT, NOT A GATE: it still passes if the bulk reader reverts to
/// per-entry reads. Its correctness properties are gated separately and
/// deterministically by `d47_manager_vector_bulk_and_per_entry_reads_agree`,
/// `..._refuses_a_non_heap_entry_like_the_per_entry_read` and
/// `..._falls_back_to_per_entry_reads_on_a_refused_chunk` — and the
/// `retries == 0` assertion here is the one thing that would catch the bulk
/// path silently never being taken.
#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_manager_vector_ab() {
    let world = manager_vector_bench_world();
    // Both arms must resolve every entry, or one of them is timing a refusal.
    page_cache_new_frame();
    let legacy_entries = manager_vector_entries_legacy(world.data, world.size)
        .expect("legacy arm must read the whole vector");
    page_cache_new_frame();
    let mut retries = 0u32;
    let bulk_entries =
        domain47_read_manager_vector_entries(world.data, world.size, &mut retries)
            .expect("bulk arm must read the whole vector");
    assert_eq!(
        legacy_entries, bulk_entries,
        "the two arms did not read the same entries"
    );
    assert_eq!(legacy_entries.len(), T28_MANAGER_ENTRIES);
    // A fixture that fell back to the per-entry loop would be timing the arm
    // it is supposed to be timing against.
    assert_eq!(retries, 0, "the bulk arm fell back to per-entry reads");

    let (legacy, bulk) = time_us_median_ab(
        5,
        || {
            manager_vector_entries_legacy(world.data, world.size)
                .map_or(0, |entries| entries.len())
        },
        || {
            domain47_read_manager_vector_entries(world.data, world.size, &mut 0)
                .map_or(0, |entries| entries.len())
        },
    );
    // One prewarm reads the vectors three times; this fixture is both vectors'
    // width in one array, so one pass here is one pass over both.
    println!(
        "d47_cost AB manager_vector: per-entry {legacy:.1} us -> bulk {bulk:.1} us ({:+.1}%) \
         over {T28_MANAGER_ENTRIES} entries; per PREWARM (3 passes) {:.1} us -> {:.1} us, a \
         saving of {:.1} us against t28's 24873 us p50 prewarm = {:.1}% of it. Guarded calls \
         per prewarm: {} -> {}.",
        (bulk - legacy) / legacy * 100.0,
        legacy * 3.0,
        bulk * 3.0,
        (legacy - bulk) * 3.0,
        (legacy - bulk) * 3.0 / 24_873.0 * 100.0,
        T28_MANAGER_ENTRIES * 2 * 3,
        T28_MANAGER_ENTRIES.div_ceil(DOMAIN47_MANAGER_VECTOR_COPY_ENTRIES) * 2 * 3,
    );
}

/// The bulk reader and the per-entry reader must agree ENTRY FOR ENTRY, at a
/// width that crosses several chunks and does not divide evenly by the chunk
/// size (so the tail chunk is exercised).
///
/// MUTATION: decode the entry as big-endian in the bulk arm, or drop its
/// `is_map_control_heap_pointer` filter, and this fails.
#[test]
fn d47_manager_vector_bulk_and_per_entry_reads_agree() {
    let entries = DOMAIN47_MANAGER_VECTOR_COPY_ENTRIES * 3 + 37;
    let arena = BenchArena::new(0x10000);
    assert!(entries * 8 <= 0x10000);
    for ordinal in 0..entries {
        arena.put_u64(arena.base + ordinal * 8, synthetic_entry(ordinal));
    }
    page_cache_new_frame();
    let legacy = manager_vector_entries_legacy(arena.base, entries as u32)
        .expect("per-entry arm must read the whole vector");
    page_cache_new_frame();
    let mut retries = 0u32;
    let bulk = domain47_read_manager_vector_entries(arena.base, entries as u32, &mut retries)
        .expect("bulk arm must read the whole vector");
    assert_eq!(legacy, bulk);
    assert_eq!(bulk.len(), entries);
    // A contiguous, wholly-committed array must take the bulk path for every
    // chunk, or this compares two runs of the SAME per-entry reader.
    assert_eq!(retries, 0);
}

/// An entry the pointer filter refuses must fail the read through either
/// path, and must never be silently skipped.
///
/// MUTATION: make the bulk arm `continue` instead of returning on a refused
/// pointer and this fails — the vector would come back short with no error.
#[test]
fn d47_manager_vector_bulk_refuses_a_non_heap_entry_like_the_per_entry_read() {
    let entries = DOMAIN47_MANAGER_VECTOR_COPY_ENTRIES + 5;
    let arena = BenchArena::new(0x10000);
    assert!(entries * 8 <= 0x10000);
    for ordinal in 0..entries {
        arena.put_u64(arena.base + ordinal * 8, synthetic_entry(ordinal));
    }
    // One entry in the SECOND chunk is outside the heap window.
    let bad_ordinal = DOMAIN47_MANAGER_VECTOR_COPY_ENTRIES + 2;
    arena.put_u64(arena.base + bad_ordinal * 8, 0x10);
    page_cache_new_frame();
    let legacy = manager_vector_entries_legacy(arena.base, entries as u32);
    page_cache_new_frame();
    let bulk = domain47_read_manager_vector_entries(arena.base, entries as u32, &mut 0);
    assert_eq!(
        legacy.as_ref().err(),
        Some(&Domain47ManagerRosterFailureKind::VectorEntryInvalid)
    );
    assert_eq!(
        bulk.as_ref().err(),
        Some(&Domain47ManagerRosterFailureKind::VectorEntryInvalid)
    );
}

/// A fallback that happened BEFORE the read went on to fail must still be
/// counted.
///
/// Zero retries already means "no chunk fell back", so a reader who sees zero
/// on a failed census concludes the bulk path worked — when what actually
/// happened is that the count was discarded. That is why the accumulator is
/// the CALLER's and not a return value: `?` and early returns cannot lose it
/// (Codex 019fd70b, finding 1).
///
/// MUTATION: make `domain47_read_manager_vector_entries` return the count
/// instead of bumping the caller's, and this fails with 0.
#[test]
fn d47_manager_vector_counts_a_fallback_that_preceded_a_failure() {
    let arena = BenchArena::new(0x10000);
    let entries = DOMAIN47_MANAGER_VECTOR_COPY_ENTRIES * 2;
    let data = arena.base + 0x400;
    for ordinal in 0..entries {
        arena.put_u64(data + ordinal * 8, synthetic_entry(ordinal));
    }
    // An entry the pointer filter refuses, inside the chunk that straddles
    // the boundary — so the chunk falls back FIRST and then the per-entry
    // read of this ordinal fails.
    let bad_ordinal = DOMAIN47_MANAGER_VECTOR_COPY_ENTRIES + 10;
    arena.put_u64(data + bad_ordinal * 8, 0x10);
    let mut previous: Dword = 0;
    let protected = unsafe {
        VirtualProtect(
            (arena.base + 0x1000) as *mut c_void,
            0xF000,
            BENCH_PAGE_READONLY,
            &mut previous,
        )
    };
    assert!(protected != 0, "VirtualProtect must split the arena");

    page_cache_new_frame();
    let mut retries = 0u32;
    let outcome = domain47_read_manager_vector_entries(data, entries as u32, &mut retries);
    assert_eq!(
        outcome.err(),
        Some(Domain47ManagerRosterFailureKind::VectorEntryInvalid)
    );
    assert_eq!(
        retries, 1,
        "the fallback that happened before the failure must survive it"
    );
}

/// A chunk whose bulk copy cannot be taken falls back to the per-entry reads
/// and still returns every entry.
///
/// The refused range is produced the way a live one is: the array crosses a
/// PROTECTION boundary, so `VirtualQuery` reports two adjacent regions. Both
/// are committed and readable, so every 8-byte entry reads individually, but
/// `read_into` refuses the 2 KiB chunk that straddles them — the same
/// whole-range-inside-ONE-region rule `read_object_window` applies. A
/// bulk-only reader would fail the entire census here.
///
/// MUTATION: delete the `else` arm of the chunk loop (return `Err` when the
/// bulk copy is refused) and this fails.
#[test]
fn d47_manager_vector_falls_back_to_per_entry_reads_on_a_refused_chunk() {
    let arena = BenchArena::new(0x10000);
    // Two full chunks starting 0x400 into the arena, so the SECOND chunk
    // ([0xC00, 0x1400)) straddles the page boundary at 0x1000.
    let entries = DOMAIN47_MANAGER_VECTOR_COPY_ENTRIES * 2;
    let data = arena.base + 0x400;
    for ordinal in 0..entries {
        arena.put_u64(data + ordinal * 8, synthetic_entry(ordinal));
    }
    // Split the arena into two VirtualQuery regions without making either
    // unreadable.
    let mut previous: Dword = 0;
    let protected = unsafe {
        VirtualProtect(
            (arena.base + 0x1000) as *mut c_void,
            0xF000,
            BENCH_PAGE_READONLY,
            &mut previous,
        )
    };
    assert!(protected != 0, "VirtualProtect must split the arena");

    page_cache_new_frame();
    // The straddling chunk must really be refused, or this test passes
    // without ever reaching the fallback it exists to pin.
    let straddling = data + DOMAIN47_MANAGER_VECTOR_COPY_ENTRIES * 8;
    let mut probe = vec![0u8; DOMAIN47_MANAGER_VECTOR_COPY_ENTRIES * 8];
    assert!(
        !read_into(straddling, &mut probe),
        "the chunk crossing the protection boundary must be refused"
    );
    // ...and the entries inside it must still be individually readable, or
    // the fallback would fail too and the comparison below would be vacuous.
    assert!(read_u64_stable(straddling).is_some());
    page_cache_new_frame();
    let legacy = manager_vector_entries_legacy(data, entries as u32)
        .expect("every entry is individually readable");
    page_cache_new_frame();
    let mut retries = 0u32;
    let bulk = domain47_read_manager_vector_entries(data, entries as u32, &mut retries)
        .expect("the fallback must read what the per-entry loop reads");
    assert_eq!(legacy, bulk);
    assert_eq!(bulk.len(), entries);
    // Exactly one chunk straddles the boundary, and the retry must be
    // REPORTED: a fallback that happened silently would be a behaviour change
    // with no trace in the manifest (Codex 019fd6ea, finding 3).
    assert_eq!(retries, 1);
}

// =====================================================================
// Capture-floor stage S0: domain 47's range/byte/depth census, MEASURED
// =====================================================================

/// DOMAIN 47'S FRAME WALK, TRACED AND CENSUSED OFFLINE.
///
/// # What this replaces
///
/// The capture-floor plan anchors its whole range estimate on two figures for
/// this domain — "~5 MiB useful, ~75_500 distinct source ranges per frame" —
/// taken from lane DB's copy-only note above. That note DERIVED them: it
/// multiplied each structure's furthest-read field offset by an object count
/// off t28's wire. It never observed a read, so it could not see a field the
/// walk reads twice, a window the walk does not read to the end of, or a
/// string buffer the walk skips.
///
/// This measures them instead. It runs the SAME `frame_walk` the cost bench
/// times, through the SAME guarded read primitives, with
/// `attach_log::range_tracer` recording every `(address, length)` those
/// primitives actually served, and reduces the result with the production
/// `range_census`. Nothing here is modelled.
///
/// # What it is a census OF, precisely
///
///  * The FRAME WALK only. The prewarm census over the manager roster
///    (`read_character_census_fields` across `world.census_characters`) is a
///    separate phase with its own object count and is NOT included; it is
///    strictly additive to every number below.
///  * At T15's per-frame object counts, which is what `bench_world` builds:
///    5_227 characters, 3_358 advisors, 534 unit leaders, 733 country-leader
///    roles, 76 trait tokens. Lane DB's derived figure used t28's counts
///    (13_394 / 8_615 / 1_361 / 1_879 / 194), roughly 2.5x wider. The two are
///    therefore NOT directly comparable, and the per-object RATES printed
///    below are what carries across; the raw totals are this fixture's.
///  * Over a CONTIGUOUS arena. That does not change a range count or a byte
///    count — those are properties of what the reader asked for, not of where
///    the pages live — but it does mean the coalesced-range count here is an
///    OPTIMISTIC floor: the live heap scatters these objects, so adjacent
///    fixture objects that merge here would not merge there.
///
/// ```text
/// cargo test -p collector --release --features range-tracer \
///     d47_range_census_from_a_traced_frame -- --ignored --nocapture
/// ```
#[cfg(feature = "range-tracer")]
#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_range_census_from_a_traced_frame() {
    use super::range_tracer::{self, test_support};
    use crate::range_census::{read_trace, Census, DerivationSpan, DEFAULT_DERIVATION_SPANS};

    let _guard = test_support::lock();
    let mut path = std::env::temp_dir();
    path.push(format!("d47_range_census_{}.bin", std::process::id()));
    let path = path.to_string_lossy().into_owned();
    test_support::reset(&path, 64 * 1024 * 1024);

    let world = bench_world();
    // The fixture arena is wherever `VirtualAlloc` put it, which need not be
    // inside the live game's heap window, so point the pointer scan at the
    // arena itself. Without this every read would look like a root and the
    // depth column would be a uniform zero for the wrong reason.
    let arena_lo = world._arena.base;
    let arena_hi = arena_lo + world._arena.size;
    range_tracer::set_heap_window(arena_lo, arena_hi);

    range_tracer::on_frame_boundary();
    let restore = range_tracer::enter_domain(47);
    let sink = frame_walk(&world);
    range_tracer::leave_domain(restore);
    range_tracer::flush_after_bracket();
    range_tracer::on_frame_boundary();
    range_tracer::set_heap_window(0, 0);
    assert!(sink > 0, "the frame walk must have produced rows to census");

    let mut file = std::fs::File::open(&path).expect("tracer wrote a sidecar");
    let (header, entries) = read_trace(&mut file, 128 * 1024 * 1024).expect("sidecar parses");
    assert!(
        !header.truncated(),
        "the trace hit its entry cap; raise it rather than publishing a floor \
         as a census"
    );
    let census = Census::build(header, &entries, DerivationSpan(0x1_0000));
    let row = census
        .brackets
        .iter()
        .find(|row| row.bracket_tag == 47)
        .expect("every traced read is attributed to domain 47");

    let objects = T15_FRAME_CHARACTERS
        + T15_FRAME_ADVISORS
        + T15_FRAME_UNIT_LEADERS
        + T15_FRAME_COUNTRY_LEADER_ROLES
        + T15_FRAME_TRAIT_TOKENS;
    println!("\nDOMAIN 47 FRAME WALK, MEASURED (arena {arena_lo:#x}..{arena_hi:#x})");
    println!("  objects walked      {objects}");
    println!("  read events         {}", row.read_count);
    println!(
        "  bytes moved         {} ({:.3} MiB)",
        row.bytes_moved,
        row.bytes_moved as f64 / (1024.0 * 1024.0)
    );
    println!("  DISTINCT ranges     {}", row.distinct_ranges);
    println!(
        "  coalesced ranges    {}   (contiguous-arena floor; the live heap \
         scatters these)",
        row.coalesced_ranges
    );
    println!(
        "  UNIQUE bytes        {} ({:.3} MiB)",
        row.unique_bytes,
        row.unique_bytes as f64 / (1024.0 * 1024.0)
    );
    println!(
        "  re-read factor      {:.2}x",
        row.bytes_moved as f64 / row.unique_bytes.max(1) as f64
    );
    println!("  root reads          {}", row.root_reads);
    println!(
        "  ranges per object   {:.2}",
        row.distinct_ranges as f64 / objects as f64
    );
    println!(
        "  unique bytes/object {:.1}",
        row.unique_bytes as f64 / objects as f64
    );
    println!("  max depth by derivation span:");
    for (span, depth) in Census::depth_by_span(header, &entries, &DEFAULT_DERIVATION_SPANS) {
        println!("    span {:#010x}  depth {depth}", span.0);
    }

    // NON-VACUOUS FLOOR. The fixture walks `objects` distinct heap objects and
    // every one of them is read at least once, so a census reporting fewer
    // distinct ranges than objects has lost reads — which is exactly what a
    // tracer that recorded only the first read per address, or that dropped
    // worker-thread buffers, would produce.
    assert!(
        row.distinct_ranges >= objects as u64,
        "censused {} distinct ranges for {objects} walked objects; the tracer \
         has lost reads",
        row.distinct_ranges
    );
    assert!(
        row.unique_bytes > 0 && row.unique_bytes <= row.bytes_moved,
        "unique bytes ({}) must be positive and never exceed bytes moved ({})",
        row.unique_bytes,
        row.bytes_moved
    );
    assert!(
        row.max_depth >= 1,
        "the walk follows character -> unit-leader and leader -> trait-vector \
         pointers, so a depth of {} means the pointer provenance was not \
         recorded",
        row.max_depth
    );
    let _ = std::fs::remove_file(&path);
}

// === lane EJ, 2026-08-07: the shadow walk =================================
//
// TWO JOBS IN ONE SECTION, deliberately: the fixture that builds real MSVC
// `_Tree`s in real committed pages is the only in-repo world the walk can be
// driven against, so its SAFETY tests live beside the COST arm that shares it.
// The safety tests are ordinary tests and gate; the cost arm is `#[ignore]`d
// like every other instrument in this file.

/// Live per-prewarm widths for the two role containers, from the numbers
/// recorded in `prewarm_compact_characters_leaders_cabinet_roots`: D04 saw
/// 1_964 `CCountryLeader` objects deduping to 1_879 identities, and t32
/// adopted 6_407 advisor roots per prewarm.
const EJ_LEADER_ENTRIES: usize = 1_964;
const EJ_LEADER_CHARACTERS: usize = 1_879;
const EJ_ADVISOR_ENTRIES: usize = 6_407;

/// A small-allocation stride for `_Tree` nodes. Nodes are 0x30/0x50 bytes and
/// a real allocator packs them into shared pages — giving each its own page
/// would model an allocator that does not exist and would overstate the walk
/// by a TLB miss per node.
const EJ_NODE_STRIDE: usize = 0x80;

/// Write one MSVC `_Tree` into `arena`: a `_Myhead` sentinel plus
/// `values.len()` nodes in heap order (node `i`'s children are `2i+1` /
/// `2i+2`), every absent child pointing at the sentinel exactly as MSVC does.
///
/// WHAT IS AND IS NOT BUILT. The links are complete and consistent: every
/// node's `_Parent` is its actual parent (the root's is the sentinel), and the
/// sentinel's `_Left`/`_Right` are the real leftmost and rightmost nodes
/// reached by descending the child links, not the root and the last array
/// slot. What is NOT built is the ORDERING and the COLOURING: the nodes carry
/// no keys and every `_Color` is left zero, so this is a well-formed binary
/// tree with MSVC's link discipline rather than a legal `std::map`. That is
/// deliberate and is the fixture's stated limit — the walk consumes `_Left`,
/// `_Right`, `_Isnil` and the value, and nothing here should imply the fixture
/// would satisfy a future check on a field it does not write.
/// (Codex 019fdc2b, finding 2.)
fn ej_put_tree(
    arena: &BenchArena,
    head: usize,
    node_at: &dyn Fn(usize) -> usize,
    value_off: usize,
    values: &[u64],
) {
    // The sentinel: `_Isnil` set, all three links at itself (the empty shape).
    arena.put_u64(head + DOMAIN47_TREE_NODE_LEFT_OFF, head as u64);
    arena.put_u64(head + DOMAIN47_TREE_NODE_PARENT_OFF, head as u64);
    arena.put_u64(head + DOMAIN47_TREE_NODE_RIGHT_OFF, head as u64);
    arena.put_bytes(head + DOMAIN47_TREE_NODE_ISNIL_OFF, &[1u8]);
    if values.is_empty() {
        return;
    }
    for (index, value) in values.iter().enumerate() {
        let addr = node_at(index);
        let child = |slot: usize| {
            if slot < values.len() {
                node_at(slot)
            } else {
                head
            }
        };
        arena.put_u64(addr + DOMAIN47_TREE_NODE_LEFT_OFF, child(2 * index + 1) as u64);
        // The node's ACTUAL parent — `(i-1)/2` in heap order — and the
        // sentinel only for the root, which is what MSVC writes.
        arena.put_u64(
            addr + DOMAIN47_TREE_NODE_PARENT_OFF,
            if index == 0 {
                head as u64
            } else {
                node_at((index - 1) / 2) as u64
            },
        );
        arena.put_u64(
            addr + DOMAIN47_TREE_NODE_RIGHT_OFF,
            child(2 * index + 2) as u64,
        );
        arena.put_bytes(addr + DOMAIN47_TREE_NODE_ISNIL_OFF, &[0u8]);
        arena.put_u64(addr + value_off, *value);
    }
    // `_Myhead->_Parent` is the ROOT, and `_Left`/`_Right` are the leftmost
    // and rightmost nodes — DESCENDED, not guessed at as the root and the last
    // array slot, which is what they were and which is not where those links
    // point in any tree wider than one node. This walk reads none of the
    // three, so getting them wrong cost nothing except the fixture's claim to
    // be a whole tree (Codex 019fdc2b, finding 2).
    let descend = |mut index: usize, step: fn(usize) -> usize| {
        while step(index) < values.len() {
            index = step(index);
        }
        node_at(index)
    };
    arena.put_u64(head + DOMAIN47_TREE_NODE_PARENT_OFF, node_at(0) as u64);
    arena.put_u64(
        head + DOMAIN47_TREE_NODE_LEFT_OFF,
        descend(0, |index| 2 * index + 1) as u64,
    );
    arena.put_u64(
        head + DOMAIN47_TREE_NODE_RIGHT_OFF,
        descend(0, |index| 2 * index + 2) as u64,
    );
}

/// One `CCharacter` with both role maps, for the walk's own safety tests.
struct EjSingleCharacterWorld {
    arena: BenchArena,
    character: usize,
    leader_head: usize,
    leader_nodes: usize,
    leaders: Vec<u64>,
    advisors: Vec<u64>,
}

impl EjSingleCharacterWorld {
    fn new(leader_count: usize, advisor_count: usize) -> EjSingleCharacterWorld {
        let arena = BenchArena::in_heap_window(0x10000);
        let character = arena.base;
        let leader_head = arena.base + 0x1000;
        let advisor_head = arena.base + 0x1080;
        let leader_nodes = arena.base + 0x2000;
        let advisor_nodes = arena.base + 0x4000;
        // Values are never dereferenced by the walk, so they need only be
        // distinct, 8-aligned, in-window pointers.
        let leaders: Vec<u64> = (0..leader_count)
            .map(|index| (arena.base + 0x8000 + index * 0x40) as u64)
            .collect();
        let advisors: Vec<u64> = (0..advisor_count)
            .map(|index| (arena.base + 0xC000 + index * 0x40) as u64)
            .collect();
        ej_put_tree(
            &arena,
            leader_head,
            &|index| leader_nodes + index * EJ_NODE_STRIDE,
            DOMAIN47_LEADER_NODE_VALUE_OFF,
            &leaders,
        );
        ej_put_tree(
            &arena,
            advisor_head,
            &|index| advisor_nodes + index * EJ_NODE_STRIDE,
            DOMAIN47_ADVISOR_NODE_VALUE_OFF,
            &advisors,
        );
        arena.put_u64(character + DOMAIN47_LEADER_MAP_HEAD_OFF, leader_head as u64);
        arena.put_u64(character + DOMAIN47_LEADER_MAP_SIZE_OFF, leader_count as u64);
        arena.put_u64(character + DOMAIN47_ADVISOR_MAP_HEAD_OFF, advisor_head as u64);
        arena.put_u64(
            character + DOMAIN47_ADVISOR_MAP_SIZE_OFF,
            advisor_count as u64,
        );
        EjSingleCharacterWorld {
            arena,
            character,
            leader_head,
            leader_nodes,
            leaders,
            advisors,
        }
    }

    fn walk(&self) -> Domain47ShadowWalk {
        let mut walk = Domain47ShadowWalk::default();
        let mut stack = Vec::new();
        let mut scratch = Vec::new();
        page_cache_new_frame();
        domain47_shadow_walk_character(self.character, &mut walk, &mut stack, &mut scratch);
        walk
    }
}

/// The fixture's link discipline, pinned rather than claimed.
///
/// The walk reads `_Left`, `_Right`, `_Isnil` and the value, so a wrong
/// `_Parent` or a wrong sentinel extremum is invisible to every other test
/// here — which is exactly how they came to be wrong (Codex 019fdc2b,
/// finding 2). If a future check ever reads them, this is what says the
/// fixture is describable as a tree at all.
#[test]
fn d47_shadow_walk_fixture_links_are_a_whole_tree() {
    // Six nodes in heap order: 0 -> {1,2}, 1 -> {3,4}, 2 -> {5}. Leftmost by
    // descent is 3; rightmost is 2 (node 2's right child, 6, does not exist).
    let world = EjSingleCharacterWorld::new(6, 0);
    let node = |index: usize| world.leader_nodes + index * EJ_NODE_STRIDE;
    let head = world.leader_head;
    page_cache_new_frame();
    assert_eq!(
        read_u64(head + DOMAIN47_TREE_NODE_PARENT_OFF),
        Some(node(0) as u64),
        "the sentinel's parent is the root"
    );
    assert_eq!(
        read_u64(head + DOMAIN47_TREE_NODE_LEFT_OFF),
        Some(node(3) as u64),
        "the sentinel's left is the LEFTMOST node, not the root"
    );
    assert_eq!(
        read_u64(head + DOMAIN47_TREE_NODE_RIGHT_OFF),
        Some(node(2) as u64),
        "the sentinel's right is the RIGHTMOST node, not the last array slot"
    );
    assert_eq!(
        read_u64(node(0) + DOMAIN47_TREE_NODE_PARENT_OFF),
        Some(head as u64),
        "only the root's parent is the sentinel"
    );
    for index in 1..6usize {
        assert_eq!(
            read_u64(node(index) + DOMAIN47_TREE_NODE_PARENT_OFF),
            Some(node((index - 1) / 2) as u64),
            "node {index}'s parent link must be its actual parent"
        );
    }
}

#[test]
fn d47_shadow_walk_reaches_every_entry_of_both_maps() {
    let world = EjSingleCharacterWorld::new(3, 9);
    let walk = world.walk();
    assert_eq!(
        walk.leaders,
        world.leaders.iter().copied().collect::<BTreeSet<u64>>(),
        "the leader map's every value must be reached"
    );
    assert_eq!(
        walk.advisors,
        world.advisors.iter().copied().collect::<BTreeSet<u64>>(),
        "the advisor map's every value must be reached"
    );
    assert_eq!(walk.refused, 0, "a well-formed pair of maps refuses nothing");
}

#[test]
fn d47_shadow_walk_reads_each_map_at_the_value_offset_the_re_proved() {
    // The two maps have DIFFERENT value offsets (0x28 and 0x48) because their
    // keys are an `int` and a `std::string`. One offset used for both would
    // read a node header, or padding, as a pointer.
    let world = EjSingleCharacterWorld::new(1, 1);
    let walk = world.walk();
    assert_eq!(walk.refused, 0);
    assert!(walk.leaders.contains(&world.leaders[0]));
    assert!(walk.advisors.contains(&world.advisors[0]));
    assert_eq!(walk.leaders.len(), 1);
    assert_eq!(walk.advisors.len(), 1);
}

#[test]
fn d47_shadow_walk_refuses_a_map_whose_size_outruns_its_nodes() {
    let world = EjSingleCharacterWorld::new(3, 2);
    // A concurrent erase leaves `_Mysize` one ahead of what is reachable.
    world.arena.put_u64(
        world.character + DOMAIN47_LEADER_MAP_SIZE_OFF,
        world.leaders.len() as u64 + 1,
    );
    let walk = world.walk();
    assert_eq!(walk.refused, 1, "the torn map, and only it, must refuse");
    assert!(
        walk.leaders.is_empty(),
        "a refused map must contribute NOTHING: a partial merge would inflate \
         the walk-only residual with entries the walk never proved it had"
    );
    assert_eq!(
        walk.advisors.len(),
        world.advisors.len(),
        "the other map on the same character is unaffected"
    );
}

#[test]
fn d47_shadow_walk_refuses_a_map_with_more_nodes_than_its_size_claims() {
    let world = EjSingleCharacterWorld::new(3, 0);
    // The other direction: a concurrent insert leaves `_Mysize` behind.
    world
        .arena
        .put_u64(world.character + DOMAIN47_LEADER_MAP_SIZE_OFF, 2);
    let walk = world.walk();
    assert_eq!(walk.refused, 1);
    assert!(walk.leaders.is_empty());
}

#[test]
fn d47_shadow_walk_refuses_a_cycle_and_terminates() {
    let world = EjSingleCharacterWorld::new(3, 0);
    // The last node's right child points back at the root: a cycle no
    // `_Mysize` can describe.
    world.arena.put_u64(
        world.leader_nodes + 2 * EJ_NODE_STRIDE + DOMAIN47_TREE_NODE_RIGHT_OFF,
        world.leader_nodes as u64,
    );
    let walk = world.walk();
    assert_eq!(
        walk.refused, 1,
        "a cycle must refuse — and this test terminating at all is the bound"
    );
    assert!(walk.leaders.is_empty());
}

#[test]
fn d47_shadow_walk_refuses_an_implausible_size() {
    let world = EjSingleCharacterWorld::new(1, 0);
    world.arena.put_u64(
        world.character + DOMAIN47_LEADER_MAP_SIZE_OFF,
        DOMAIN47_SHADOW_WALK_MAX_NODES + 1,
    );
    let walk = world.walk();
    assert_eq!(walk.refused, 1, "a size past the hard cap must refuse");
    assert!(
        walk.leaders.is_empty(),
        "and must not be truncated to the cap: a truncated walk would \
         manufacture a sweep-only residual out of its own bound"
    );
}

#[test]
fn d47_shadow_walk_refuses_an_unreadable_node() {
    let world = EjSingleCharacterWorld::new(2, 0);
    // A node pointer inside the heap window but far outside anything this
    // process committed: `range_confirmed_good` refuses it.
    world.arena.put_u64(
        world.leader_nodes + DOMAIN47_TREE_NODE_LEFT_OFF,
        (HEAP_HI - 0x1000) as u64,
    );
    let walk = world.walk();
    assert_eq!(walk.refused, 1);
    assert!(walk.leaders.is_empty());
}

#[test]
fn d47_shadow_walk_refuses_a_node_that_claims_to_be_the_sentinel() {
    let world = EjSingleCharacterWorld::new(2, 0);
    world.arena.put_bytes(
        world.leader_nodes + EJ_NODE_STRIDE + DOMAIN47_TREE_NODE_ISNIL_OFF,
        &[1u8],
    );
    let walk = world.walk();
    assert_eq!(
        walk.refused, 1,
        "a nil node reached through a link that is not the sentinel is a shape \
         this tree cannot have"
    );
    assert!(walk.leaders.is_empty());
}

#[test]
fn d47_shadow_walk_refuses_a_value_that_is_not_a_heap_pointer() {
    let world = EjSingleCharacterWorld::new(2, 0);
    world
        .arena
        .put_u64(world.leader_nodes + DOMAIN47_LEADER_NODE_VALUE_OFF, 0x1234);
    let walk = world.walk();
    assert_eq!(walk.refused, 1);
    assert!(walk.leaders.is_empty());
}

#[test]
fn d47_shadow_walk_counts_an_empty_map_as_observed_without_reading_its_sentinel() {
    let world = EjSingleCharacterWorld::new(0, 2);
    // `_Mysize == 0` short-circuits BEFORE the sentinel is touched, which is
    // most of this instrument's budget on a roster where most characters hold
    // neither role. An unreadable head proves the read does not happen.
    world.arena.put_u64(
        world.character + DOMAIN47_LEADER_MAP_HEAD_OFF,
        (HEAP_HI - 0x1000) as u64,
    );
    let walk = world.walk();
    assert_eq!(
        walk.refused, 0,
        "an empty map is OBSERVED as empty, not refused"
    );
    assert!(walk.leaders.is_empty());
    assert_eq!(walk.advisors.len(), 2);
}

#[test]
fn d47_shadow_walk_refuses_both_maps_when_the_body_is_unreadable() {
    let mut walk = Domain47ShadowWalk::default();
    let mut stack = Vec::new();
    let mut scratch = Vec::new();
    page_cache_new_frame();
    domain47_shadow_walk_character(HEAP_HI - 0x1000, &mut walk, &mut stack, &mut scratch);
    assert_eq!(
        walk.refused, 2,
        "an unreadable body observed NEITHER map, and the counter is per map"
    );
    assert!(walk.leaders.is_empty() && walk.advisors.is_empty());
}

#[test]
fn d47_shadow_walk_reuses_its_stack_and_scratch_across_characters() {
    let first = EjSingleCharacterWorld::new(3, 4);
    let second = EjSingleCharacterWorld::new(2, 1);
    let mut walk = Domain47ShadowWalk::default();
    let mut stack = Vec::new();
    let mut scratch = Vec::new();
    page_cache_new_frame();
    domain47_shadow_walk_character(first.character, &mut walk, &mut stack, &mut scratch);
    domain47_shadow_walk_character(second.character, &mut walk, &mut stack, &mut scratch);
    assert_eq!(walk.refused, 0);
    assert_eq!(walk.leaders.len(), 5, "3 + 2 distinct leader pointers");
    assert_eq!(walk.advisors.len(), 5, "4 + 1 distinct advisor pointers");
}

// --- what shadowing costs --------------------------------------------------

/// A census-width world whose role maps are populated at live widths.
struct EjShadowWorld {
    _arena: BenchArena,
    roots: ResolvedTypeRoots,
    census_characters: Vec<usize>,
}

fn ej_shadow_world() -> EjShadowWorld {
    let count = T32_CENSUS_CHARACTERS;
    let node_slots = EJ_LEADER_ENTRIES + EJ_ADVISOR_ENTRIES;
    // One sentinel per NON-EMPTY map, plus one node per entry. Sentinels and
    // nodes share the same packed, shuffled region: they are the same size
    // class and a real allocator makes no distinction between them.
    let head_slots = EJ_LEADER_CHARACTERS + EJ_ADVISOR_ENTRIES;
    let node_bytes = (node_slots + head_slots) * EJ_NODE_STRIDE;
    let arena =
        BenchArena::in_heap_window(2 * count * SCATTER_PAGE + node_bytes + 2 * SCATTER_PAGE);
    let string_base = arena.base + count * SCATTER_PAGE;
    let node_base = string_base + count * SCATTER_PAGE + SCATTER_PAGE;

    let shuffled = |seed: u64, len: usize| {
        let mut order: Vec<usize> = (0..len).collect();
        let mut state = seed;
        for idx in (1..len).rev() {
            let swap = (scatter_rng(&mut state) % (idx as u64 + 1)) as usize;
            order.swap(idx, swap);
        }
        order
    };
    let pages = shuffled(0x5eed_d47_e10a_u64, count);
    let string_pages = shuffled(0x5eed_d47_e10b_u64, count);
    // The node region is shuffled too, so following a character's `_Myhead`
    // is not a fixed displacement from the body that named it.
    let node_slot_order = shuffled(0x5eed_d47_e10c_u64, node_slots + head_slots);
    let mut taken = 0usize;

    let roots = ResolvedTypeRoots {
        ccharacter_vtable: BENCH_CCHARACTER_VTABLE,
        carmy_leader_vtable: BENCH_CARMY_LEADER_VTABLE,
        cnavy_leader_vtable: BENCH_CNAVY_LEADER_VTABLE,
        ccountry_leader_vtable: BENCH_CCOUNTRY_LEADER_VTABLE,
        cadvisor_vtable: BENCH_CADVISOR_VTABLE,
        cunit_leader_trait_vtable: BENCH_CUNIT_LEADER_TRAIT_VTABLE,
    };

    let mut census_characters = Vec::with_capacity(count);
    for idx in 0..count {
        let addr = arena.base + pages[idx] * SCATTER_PAGE;
        let strings = string_base + string_pages[idx] * SCATTER_PAGE;
        arena.put_u64(addr, BENCH_CCHARACTER_VTABLE);
        arena.put_u32(addr + CCHARACTER_TYPE_OFF as usize, CCHARACTER_TYPE_ID);
        arena.put_u32(addr + CCHARACTER_ID_OFF as usize, idx as u32);
        arena.put_u32(addr + DOMAIN47_CCHARACTER_NAME_TOKEN_ID_OFF, idx as u32);
        arena.put_u32(
            addr + CCHARACTER_COUNTRY_TAG_DB_ID_OFF as usize,
            (idx % 200) as u32,
        );
        arena.put_u64(addr + CCHARACTER_UNIT_LEADER_PTR_OFF as usize, 0);
        let name = if idx % 2 == 0 {
            format!("Name {idx}")
        } else {
            format!("A Rather Longer Character Name {idx}")
        };
        let token = format!("GER_character_token_{idx}");
        arena.put_msvc_string(addr + CCHARACTER_NAME_OFF as usize, strings, &name);
        arena.put_msvc_string(addr + CCHARACTER_TOKEN_OFF as usize, strings + 0x40, &token);

        // `EJ_LEADER_ENTRIES` role objects spread over `EJ_LEADER_CHARACTERS`
        // characters (the surplus models D04's several-roles-per-character),
        // and one advisor each over the first `EJ_ADVISOR_ENTRIES`. Every
        // other character holds an EMPTY map, which is the live majority and
        // the case the walk short-circuits.
        let leader_entries = if idx < EJ_LEADER_ENTRIES - EJ_LEADER_CHARACTERS {
            2
        } else if idx < EJ_LEADER_CHARACTERS {
            1
        } else {
            0
        };
        let advisor_entries = usize::from(idx < EJ_ADVISOR_ENTRIES);
        for (head_off, size_off, value_off, entries) in [
            (
                DOMAIN47_LEADER_MAP_HEAD_OFF,
                DOMAIN47_LEADER_MAP_SIZE_OFF,
                DOMAIN47_LEADER_NODE_VALUE_OFF,
                leader_entries,
            ),
            (
                DOMAIN47_ADVISOR_MAP_HEAD_OFF,
                DOMAIN47_ADVISOR_MAP_SIZE_OFF,
                DOMAIN47_ADVISOR_NODE_VALUE_OFF,
                advisor_entries,
            ),
        ] {
            arena.put_u64(addr + size_off, entries as u64);
            if entries == 0 {
                // A live empty map still HAS a sentinel allocation; the walk
                // is what does not read it. Point at the character's own page
                // so the fixture is well formed without spending a slot.
                arena.put_u64(addr + head_off, (addr + 0x800) as u64);
                continue;
            }
            let mut slot = || {
                let at = node_base + node_slot_order[taken] * EJ_NODE_STRIDE;
                taken += 1;
                at
            };
            let head = slot();
            let nodes: Vec<usize> = (0..entries).map(|_| slot()).collect();
            // Distinct in-window values that are not any node's own address,
            // so a walk reading the wrong field cannot accidentally pass.
            let values: Vec<u64> = nodes.iter().map(|node| (node + 8) as u64).collect();
            ej_put_tree(&arena, head, &|index| nodes[index], value_off, &values);
            arena.put_u64(addr + head_off, head as u64);
        }
        census_characters.push(addr);
    }

    EjShadowWorld {
        _arena: arena,
        roots,
        census_characters,
    }
}

/// The production census body over `EjShadowWorld`, optionally shadowing.
///
/// The `false` arm issues exactly the reads the census issues today; the
/// `true` arm is those reads plus the walk, in the position production calls
/// it from. The difference between the two timings is the whole cost of the
/// instrument.
fn ej_census_loop(world: &EjShadowWorld, shadow: bool) -> usize {
    let mem = Domain47InProcessMemory;
    let mut walk = Domain47ShadowWalk::default();
    let mut stack = Vec::new();
    let mut scratch = Vec::new();
    let mut sink = 0usize;
    for (ordinal, addr) in world.census_characters.iter().copied().enumerate() {
        if let Some(ahead) = world
            .census_characters
            .get(ordinal.wrapping_add(DOMAIN47_PREFETCH_DISTANCE))
        {
            domain47_prefetch_object(*ahead, domain47::extractor::CCHARACTER_RAW_FIELDS_SPAN);
        }
        if shadow {
            domain47_shadow_walk_character(addr, &mut walk, &mut stack, &mut scratch);
        }
        let name_token_addr = addr + DOMAIN47_CCHARACTER_NAME_TOKEN_ID_OFF;
        let (first, opening) = domain47::extractor::read_character_census_fields(
            &mem,
            &world.roots,
            addr as u64,
            || read_u32_stable(name_token_addr),
        );
        let (second, closing) = domain47::extractor::read_character_census_fields(
            &mem,
            &world.roots,
            addr as u64,
            || read_u32_stable(name_token_addr),
        );
        let (name, token) = domain47_character_root_strings(addr);
        sink += opening.is_some() as usize
            + closing.is_some() as usize
            + first.is_ok() as usize
            + second.is_ok() as usize
            + name.map_or(0, |v| v.len())
            + token.map_or(0, |v| v.len());
    }
    sink + walk.leaders.len() + walk.advisors.len() + walk.refused as usize
}

/// The fixture claims live widths; this is where that claim is checked rather
/// than trusted, and it is also the walk's only end-to-end test at census
/// scale: over 13_403 characters the walk must reach every entry of every map
/// and refuse nothing.
#[test]
fn d47_shadow_walk_covers_a_census_width_world_without_refusing() {
    let world = ej_shadow_world();
    let mut walk = Domain47ShadowWalk::default();
    let mut stack = Vec::new();
    let mut scratch = Vec::new();
    page_cache_new_frame();
    for addr in &world.census_characters {
        domain47_shadow_walk_character(*addr, &mut walk, &mut stack, &mut scratch);
    }
    assert_eq!(walk.refused, 0, "a well-formed world refuses nothing");
    assert_eq!(walk.leaders.len(), EJ_LEADER_ENTRIES);
    assert_eq!(walk.advisors.len(), EJ_ADVISOR_ENTRIES);
}

/// WHAT THE SHADOW WALK COSTS, on the census fixture that already models the
/// prewarm's dominant loop.
///
/// AN INSTRUMENT, NOT A GATE. It prints the delta and asserts only that both
/// arms did their job — a wall-time assertion on this machine, which is
/// usually also building something, would be a flaky gate rather than a
/// measurement (the same reason every other arm in this file gives).
#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_shadow_walk() {
    let world = ej_shadow_world();
    page_cache_new_frame();
    let plain = ej_census_loop(&world, false);
    let shadowed = ej_census_loop(&world, true);
    assert!(plain > 0, "the census arm read nothing");
    assert!(
        shadowed > plain,
        "the shadow arm must do strictly more than the census arm"
    );

    let (without, with) = time_us_median_ab_n(
        9,
        20,
        || ej_census_loop(&world, false),
        || ej_census_loop(&world, true),
    );
    println!(
        "d47_cost shadow walk (scattered, one page per character): census {without:.1} us -> \
         census + walk {with:.1} us (+{:.1} us, {:+.1}%) over {} characters, {} leader entries, \
         {} advisor entries",
        with - without,
        (with - without) / without * 100.0,
        T32_CENSUS_CHARACTERS,
        EJ_LEADER_ENTRIES,
        EJ_ADVISOR_ENTRIES,
    );
}

// === CLASSIFYING A sweep_only RESIDUAL (lane FC, 2026-08-08) ==============
//
// The counter says a live object was reachable to the sweep and not to the
// walk; `domain47_shadow_walk_recheck` says which of the two worlds that came
// from. Every arm below is a different answer to the promotion question, so the
// tests are written to fail if any two of them are collapsed into one.

/// Characters laid out one per stride, each with both role maps, plus the
/// roster the re-check searches — the same `(addr, CharacterRawFields)` shape
/// `Domain47ManagerRosterSnapshot::characters` carries.
struct FcRecheckWorld {
    arena: BenchArena,
    /// What the census VISITS: `Domain47ManagerRosterSnapshot::characters`.
    roster: Vec<(usize, domain47::extractor::CharacterRawFields)>,
    /// Every body written into the arena, roster member or not.
    laid_out: Vec<usize>,
}

/// Room for both map heads, both node arrays and the entry values of one
/// character, with the next character's body a long way clear of them.
const FC_CHARACTER_STRIDE: usize = 0x1_0000;

/// The country every body carries unless a test says otherwise.
const FC_COUNTRY: u32 = 1;

/// A sweep-side owner naming NO body: the re-check's own arms never look at
/// `character_addr`, so the tests that predate the locator keep the shape they
/// had and the locator's own tests state their body explicitly.
fn fc_bench_owner(char_id: u32) -> Domain47RoleOwner {
    Domain47RoleOwner {
        country_tag_db_id: FC_COUNTRY,
        char_id,
        character_addr: 0,
    }
}

/// A distinguishable element record for the latch tests. EVERY field is
/// derived from `char_id`, so a latch that moved only some of them lands on a
/// struct that equals no `fc_element(..)` at all.
fn fc_element(char_id: u32, axis: Domain47ShadowWalkMapAxis) -> Domain47ShadowWalkElement {
    Domain47ShadowWalkElement {
        char_id,
        axis: axis as u32,
        address: 0x1_0000 + u64::from(char_id),
        owner_address: 0x2_0000 + u64::from(char_id),
        owner_censused: Domain47ShadowWalkOwnerBody::NotCensused as u32,
        owner_roster_char_id: char_id + 1,
        owner_map: Domain47ShadowWalkOwnerMap::Holds as u32,
        owner_map_size: u64::from(char_id) + 2,
        candidate_bodies: char_id + 3,
        candidate_entries: char_id + 4,
    }
}

impl FcRecheckWorld {
    /// One body per `(char_id, leader values, advisor values)` triple. The same
    /// `char_id` may appear more than once: the heap really does carry
    /// duplicate `CCharacter` objects for one logical character, and which of
    /// them holds the role is exactly what a re-check may not assume.
    fn new(bodies: &[(u32, Vec<u64>, Vec<u64>)]) -> FcRecheckWorld {
        FcRecheckWorld::with_countries(
            &bodies
                .iter()
                .map(|(char_id, leaders, advisors)| {
                    (FC_COUNTRY, *char_id, leaders.clone(), advisors.clone())
                })
                .collect::<Vec<_>>(),
        )
    }

    /// The same, with each body's country stated: `char_id` is not an identity
    /// in this world, and the tests that turn on the country say so.
    fn with_countries(bodies: &[(u32, u32, Vec<u64>, Vec<u64>)]) -> FcRecheckWorld {
        FcRecheckWorld::with_off_roster(bodies, 0)
    }

    /// The same, with the LAST `off_roster` bodies laid out in memory but NOT
    /// put in the roster.
    ///
    /// The manager vectors are the census's visited set, so a `CCharacter` the
    /// roster does not carry is a body the shadow walk provably never walks —
    /// and a live role object hanging off one is the world lane FS is chasing.
    /// A test cannot express that with a roster-only fixture, which is why the
    /// arena and the roster stop being the same list here.
    fn with_off_roster(
        bodies: &[(u32, u32, Vec<u64>, Vec<u64>)],
        off_roster: usize,
    ) -> FcRecheckWorld {
        assert!(
            off_roster <= bodies.len(),
            "more off-roster bodies than bodies"
        );
        let censused = bodies.len() - off_roster;
        let arena = BenchArena::in_heap_window(FC_CHARACTER_STRIDE * (bodies.len() + 1));
        let mut roster = Vec::new();
        let mut laid_out = Vec::new();
        for (ordinal, (country_tag_db_id, char_id, leaders, advisors)) in
            bodies.iter().enumerate()
        {
            let character = arena.base + ordinal * FC_CHARACTER_STRIDE;
            let leader_head = character + 0x1000;
            let advisor_head = character + 0x1080;
            let leader_nodes = character + 0x2000;
            let advisor_nodes = character + 0x6000;
            ej_put_tree(
                &arena,
                leader_head,
                &|index| leader_nodes + index * EJ_NODE_STRIDE,
                DOMAIN47_LEADER_NODE_VALUE_OFF,
                leaders,
            );
            ej_put_tree(
                &arena,
                advisor_head,
                &|index| advisor_nodes + index * EJ_NODE_STRIDE,
                DOMAIN47_ADVISOR_NODE_VALUE_OFF,
                advisors,
            );
            arena.put_u64(character + DOMAIN47_LEADER_MAP_HEAD_OFF, leader_head as u64);
            arena.put_u64(character + DOMAIN47_LEADER_MAP_SIZE_OFF, leaders.len() as u64);
            arena.put_u64(character + DOMAIN47_ADVISOR_MAP_HEAD_OFF, advisor_head as u64);
            arena.put_u64(
                character + DOMAIN47_ADVISOR_MAP_SIZE_OFF,
                advisors.len() as u64,
            );
            laid_out.push(character);
            if ordinal < censused {
                roster.push((
                    character,
                    domain47::extractor::CharacterRawFields {
                        character_addr: character as u64,
                        country_tag_db_id: *country_tag_db_id,
                        char_id: *char_id,
                        unit_leader_ptr: 0,
                    },
                ));
            }
        }
        FcRecheckWorld {
            arena,
            roster,
            laid_out,
        }
    }

    /// A role-object address held by no map — distinct, 8-aligned and inside
    /// the heap window, so the walk's own pointer gate admits it.
    ///
    /// Off the END of every laid-out body rather than off the end of the
    /// ROSTER, so an off-roster body cannot share a stride with a role address.
    fn role(&self, ordinal: usize) -> u64 {
        (self.arena.base + FC_CHARACTER_STRIDE * self.laid_out.len() + 0x40 * (ordinal + 1)) as u64
    }

    fn recheck(
        &self,
        target: u64,
        char_id: u32,
        axis: Domain47ShadowWalkMapAxis,
        identity_unchanged: bool,
    ) -> Domain47ShadowWalkRecheckOutcome {
        self.recheck_owned(target, fc_bench_owner(char_id), axis, identity_unchanged)
    }

    /// The full re-check report, for the two words that say what it EXAMINED.
    fn recheck_report(
        &self,
        target: u64,
        char_id: u32,
        axis: Domain47ShadowWalkMapAxis,
    ) -> Domain47ShadowWalkRecheckReport {
        let mut stack = Vec::new();
        let mut scratch = Vec::new();
        let mut found = BTreeSet::new();
        page_cache_new_frame();
        domain47_shadow_walk_recheck(
            target,
            fc_bench_owner(char_id),
            axis,
            &self.roster,
            &mut stack,
            &mut scratch,
            &mut found,
            |_, _| true,
        )
    }

    /// Probe the body an owner names, the way the production call site does.
    fn owner_probe(
        &self,
        target: u64,
        owner: Domain47RoleOwner,
        axis: Domain47ShadowWalkMapAxis,
    ) -> Domain47ShadowWalkOwnerProbe {
        let mut stack = Vec::new();
        let mut scratch = Vec::new();
        let mut found = BTreeSet::new();
        page_cache_new_frame();
        domain47_shadow_walk_owner_probe(
            target,
            owner,
            axis,
            &self.roster,
            &mut stack,
            &mut scratch,
            &mut found,
        )
    }

    /// The address of the `ordinal`-th roster body, which is what a sweep-side
    /// owner's `character_addr` would hold when that body is the one the role
    /// object points at.
    fn body(&self, ordinal: usize) -> u64 {
        self.laid_out[ordinal] as u64
    }

    /// The same, recording what the ABA guard was handed. The guard's argument
    /// is the property `d47_shadow_walk_recheck_hands_the_guard_the_whole_owner`
    /// pins, so it is captured rather than asserted inline.
    fn recheck_seen(
        &self,
        target: u64,
        owner: Domain47RoleOwner,
        axis: Domain47ShadowWalkMapAxis,
    ) -> (
        Domain47ShadowWalkRecheckOutcome,
        Option<(u64, Domain47RoleOwner)>,
    ) {
        let mut stack = Vec::new();
        let mut scratch = Vec::new();
        let mut found = BTreeSet::new();
        let mut seen = None;
        page_cache_new_frame();
        let report = domain47_shadow_walk_recheck(
            target,
            owner,
            axis,
            &self.roster,
            &mut stack,
            &mut scratch,
            &mut found,
            |addr, handed| {
                seen = Some((addr, handed));
                true
            },
        );
        (report.outcome, seen)
    }

    fn recheck_owned(
        &self,
        target: u64,
        owner: Domain47RoleOwner,
        axis: Domain47ShadowWalkMapAxis,
        identity_unchanged: bool,
    ) -> Domain47ShadowWalkRecheckOutcome {
        let mut stack = Vec::new();
        let mut scratch = Vec::new();
        let mut found = BTreeSet::new();
        page_cache_new_frame();
        domain47_shadow_walk_recheck(
            target,
            owner,
            axis,
            &self.roster,
            &mut stack,
            &mut scratch,
            &mut found,
            |addr, handed| {
                assert_eq!(addr, target, "the ABA guard must re-read the TARGET");
                assert_eq!(handed, owner, "and must be told the WHOLE owner");
                identity_unchanged
            },
        )
        .outcome
    }
}

/// THE REACHED-ON-RECHECK ARM. The owning character's map holds the entry now
/// and the object there is identity-compatible, which is CONSISTENT WITH the
/// gap between the walk's instant and the sweep's rather than proof of it — and
/// the same target checked against the OTHER axis must not come back as a hit.
///
/// MUTATION: make `domain47_shadow_walk_recheck` walk both axes instead of the
/// one it was given (the obvious "be thorough" change) and the advisor
/// assertion fails with `WalkHit` — a leader-map entry would be reported as
/// proof that the advisor map is complete.
#[test]
fn d47_shadow_walk_recheck_hit_is_the_reached_on_recheck_arm() {
    let world = FcRecheckWorld::new(&[(7, vec![0], Vec::new())]);
    let target = world.role(0);
    world.arena.put_u64(
        world.roster[0].0 + 0x2000 + DOMAIN47_LEADER_NODE_VALUE_OFF,
        target,
    );
    assert_eq!(
        world.recheck(target, 7, Domain47ShadowWalkMapAxis::Leader, true),
        Domain47ShadowWalkRecheckOutcome::WalkHit,
        "the walk reaches it now and the object is identity-compatible, which \
         is consistent with insertion timing rather than a blind spot"
    );
    assert_eq!(
        world.recheck(target, 7, Domain47ShadowWalkMapAxis::Advisor, true),
        Domain47ShadowWalkRecheckOutcome::WalkMissed,
        "the advisor map does not hold it, and the axis the sweep named is the \
         only one that answers for it"
    );
}

/// THE STRUCTURAL ARM. The owning character was found and its map walked
/// cleanly, and the entry is not in it.
///
/// MUTATION: return `WalkHit` whenever a body walks cleanly (i.e. drop the
/// `found.contains(&target)` test) and this fails — the instrument would then
/// call every residual benign, which is the answer that unblocks a deletion
/// that must stay blocked.
#[test]
fn d47_shadow_walk_recheck_miss_is_the_candidate_structural_gap() {
    let world = FcRecheckWorld::new(&[(7, vec![0], vec![0])]);
    let target = world.role(0);
    assert_eq!(
        world.recheck(target, 7, Domain47ShadowWalkMapAxis::Leader, true),
        Domain47ShadowWalkRecheckOutcome::WalkMissed
    );
}

/// A character the census never saw is its OWN outcome, not a plain miss.
///
/// Both are candidate structural gaps, and they are different faults: one says
/// the map does not hold the object, the other says the walk never visits the
/// character at all — which is the shape a re-check that followed the role
/// object's own `CCharacter*` would have reported as a HIT.
///
/// MUTATION: collapse the `(0, _)` arm into `WalkMissed` and this fails; the
/// roster-side gap would then be invisible on the wire.
#[test]
fn d47_shadow_walk_recheck_reports_an_uncensused_owner_on_its_own_code() {
    let world = FcRecheckWorld::new(&[(7, vec![0], Vec::new())]);
    let target = world.role(0);
    assert_eq!(
        world.recheck(target, 999, Domain47ShadowWalkMapAxis::Leader, true),
        Domain47ShadowWalkRecheckOutcome::OwnerNotCensused
    );
    assert!(
        Domain47ShadowWalkRecheckOutcome::OwnerNotCensused
            > Domain47ShadowWalkRecheckOutcome::WalkHit,
        "a structural candidate must outrank a reached-on-recheck element, or \
         a prewarm holding both would publish the benign one"
    );
}

/// THE ABA GUARD. A container hit whose object no longer carries the `char_id`
/// the sweep attributed to it is an address the allocator reused, and it is
/// reported as unclassified rather than as proof of skew.
///
/// MUTATION: drop the `identity_unchanged` call and return `WalkHit`
/// unconditionally — this fails with `WalkHit`, i.e. a structural gap whose
/// address was recycled into a live entry would be published as benign.
#[test]
fn d47_shadow_walk_recheck_refuses_a_hit_whose_object_identity_moved() {
    let world = FcRecheckWorld::new(&[(7, vec![0], Vec::new())]);
    let target = world.role(0);
    world.arena.put_u64(
        world.roster[0].0 + 0x2000 + DOMAIN47_LEADER_NODE_VALUE_OFF,
        target,
    );
    assert_eq!(
        world.recheck(target, 7, Domain47ShadowWalkMapAxis::Leader, false),
        Domain47ShadowWalkRecheckOutcome::Refused
    );
}

/// A body whose map REFUSES under the walk's own guards proves nothing, and
/// must not be published as a miss.
///
/// MUTATION: treat a refused body as a clean walk (set `walked_clean` before
/// the refusal test) and this fails with `WalkMissed` — an unreadable map would
/// then manufacture the exact structural-gap evidence that keeps the sweep.
#[test]
fn d47_shadow_walk_recheck_refuses_when_the_only_owner_map_cannot_be_walked() {
    let world = FcRecheckWorld::new(&[(7, vec![0, 0], Vec::new())]);
    // `_Mysize` one ahead of what is reachable: the tear check refuses.
    world
        .arena
        .put_u64(world.roster[0].0 + DOMAIN47_LEADER_MAP_SIZE_OFF, 3);
    let target = world.role(0);
    assert_eq!(
        world.recheck(target, 7, Domain47ShadowWalkMapAxis::Leader, true),
        Domain47ShadowWalkRecheckOutcome::Refused
    );
}

/// ONE REFUSED BODY POISONS THE VERDICT (Codex 019fde9b, finding 2).
///
/// The heap carries duplicate `CCharacter` bodies for one logical character and
/// the role object may hang off ANY of them, so a body whose map could not be
/// read is a body that might have held it. A clean walk of a DIFFERENT body
/// therefore does not license `WalkMissed`: the honest verdict is `Refused`.
///
/// MUTATION: drop the `any_refused` term from the final match (return
/// `WalkMissed` whenever any body walked cleanly) and this fails — an
/// unreadable duplicate would manufacture structural-gap evidence out of a
/// body nobody could look at.
#[test]
fn d47_shadow_walk_recheck_refuses_when_a_candidate_body_could_not_be_walked() {
    let world = FcRecheckWorld::new(&[(7, vec![0, 0], Vec::new()), (7, vec![0], Vec::new())]);
    world
        .arena
        .put_u64(world.roster[0].0 + DOMAIN47_LEADER_MAP_SIZE_OFF, 3);
    let target = world.role(0);
    assert_eq!(
        world.recheck(target, 7, Domain47ShadowWalkMapAxis::Leader, true),
        Domain47ShadowWalkRecheckOutcome::Refused
    );
}

/// The multi-body path still CONCLUDES when nothing refused: two duplicate
/// bodies, both walked cleanly, neither holding the entry, is a miss.
///
/// Without this the fix above could be satisfied by a re-check that answers
/// `Refused` for every world with more than one body.
#[test]
fn d47_shadow_walk_recheck_misses_across_two_clean_duplicate_bodies() {
    let world = FcRecheckWorld::new(&[(7, vec![0], Vec::new()), (7, vec![0], Vec::new())]);
    let target = world.role(0);
    assert_eq!(
        world.recheck(target, 7, Domain47ShadowWalkMapAxis::Leader, true),
        Domain47ShadowWalkRecheckOutcome::WalkMissed
    );
}

/// A duplicate body that is NOT the first one may hold the entry, and finding
/// it there is still a hit.
#[test]
fn d47_shadow_walk_recheck_finds_the_entry_on_a_later_duplicate_body() {
    let world = FcRecheckWorld::new(&[(7, vec![0], Vec::new()), (7, vec![0], Vec::new())]);
    let target = world.role(0);
    world.arena.put_u64(
        world.roster[1].0 + 0x2000 + DOMAIN47_LEADER_NODE_VALUE_OFF,
        target,
    );
    assert_eq!(
        world.recheck(target, 7, Domain47ShadowWalkMapAxis::Leader, true),
        Domain47ShadowWalkRecheckOutcome::WalkHit
    );
}

/// THE OWNER IS A PAIR (Codex 019fde9b, finding 1).
///
/// `char_id` is not an identity in this world — `domain47_read_exact_manager_roster`
/// states that union-wide uniqueness is unproven, and the dedup quarantines
/// duplicates that carry one id under two countries. The ABA guard must
/// therefore be handed the COUNTRY as well, or an address recycled into another
/// country's role object would be published as proof of a benign skew.
///
/// MUTATION: narrow the guard's argument back to a bare `char_id` (or have the
/// call site close over it and compare only the number) and this fails to
/// compile or fails the `country_tag_db_id` assertion.
///
/// WHAT IT DOES NOT PROVE: that PRODUCTION's guard uses both halves. This test
/// supplies its own closure, so removing a `country_tag_db_id` comparison from
/// the real predicate would leave it green (Codex 019fdead, finding 2). That
/// property belongs to `domain47_leader_identity_matches_owner` /
/// `domain47_advisor_probe_matches_owner`, which the two tests below exercise
/// directly — the pair is what makes the guarantee whole.
#[test]
fn d47_shadow_walk_recheck_hands_the_guard_the_whole_owner() {
    let world = FcRecheckWorld::new(&[(7, vec![0], Vec::new())]);
    let target = world.role(0);
    world.arena.put_u64(
        world.roster[0].0 + 0x2000 + DOMAIN47_LEADER_NODE_VALUE_OFF,
        target,
    );
    let owner = Domain47RoleOwner {
        country_tag_db_id: FC_COUNTRY,
        char_id: 7,
        character_addr: world.body(0),
    };
    let (outcome, seen) = world.recheck_seen(target, owner, Domain47ShadowWalkMapAxis::Leader);
    assert_eq!(outcome, Domain47ShadowWalkRecheckOutcome::WalkHit);
    assert_eq!(
        seen,
        Some((target, owner)),
        "the guard must receive the address AND the whole owner it has to \
         re-prove, country included"
    );
}

/// CANDIDATE BODIES ARE FOUND BY `char_id`, NOT BY THE PAIR — deliberately.
///
/// The production walk visits every roster body whatever country it claims, so
/// a body holding the entry under a DIFFERENT country still proves the walk
/// reaches it. Narrowing the roster lookup to the exact pair would report this
/// world as a structural gap, which is the opposite error to the one finding 1
/// is about; both directions are wrong and this pins the one the fix could
/// easily have introduced.
#[test]
fn d47_shadow_walk_recheck_examines_every_body_carrying_the_id() {
    let world = FcRecheckWorld::with_countries(&[
        (FC_COUNTRY, 7, vec![0], Vec::new()),
        (FC_COUNTRY + 1, 7, vec![0], Vec::new()),
    ]);
    let target = world.role(0);
    // The entry hangs off the body whose COUNTRY differs from the owner's.
    world.arena.put_u64(
        world.roster[1].0 + 0x2000 + DOMAIN47_LEADER_NODE_VALUE_OFF,
        target,
    );
    assert_eq!(
        world.recheck(target, 7, Domain47ShadowWalkMapAxis::Leader, true),
        Domain47ShadowWalkRecheckOutcome::WalkHit,
        "the walk visits that body, so the re-check must too"
    );
}

/// THE BOUND. A `char_id` resolving to more bodies than the re-check will walk
/// stops it, and stopping early is reported as unclassified — never as "the
/// walk cannot reach it", which is a claim about bodies it did not look at.
///
/// MUTATION: delete the `DOMAIN47_SHADOW_WALK_RECHECK_BODY_MAX` test and this
/// fails with `WalkHit`, which is the honest answer for this fixture and a
/// promise the bounded walk cannot keep on a torn roster.
#[test]
fn d47_shadow_walk_recheck_refuses_rather_than_outrunning_its_body_bound() {
    let bodies: Vec<(u32, Vec<u64>, Vec<u64>)> = (0..=DOMAIN47_SHADOW_WALK_RECHECK_BODY_MAX)
        .map(|_| (7u32, vec![0u64], Vec::new()))
        .collect();
    let world = FcRecheckWorld::new(&bodies);
    let target = world.role(0);
    // The entry sits in the body just PAST the bound.
    let last = world.roster[DOMAIN47_SHADOW_WALK_RECHECK_BODY_MAX as usize].0;
    world
        .arena
        .put_u64(last + 0x2000 + DOMAIN47_LEADER_NODE_VALUE_OFF, target);
    assert_eq!(
        world.recheck(target, 7, Domain47ShadowWalkMapAxis::Leader, true),
        Domain47ShadowWalkRecheckOutcome::Refused,
        "the re-check stopped before that body and must say so"
    );
}

/// The prewarm's three describing words name the FIRST element at the HIGHEST
/// outcome the prewarm reached, and the tally counts every element.
///
/// MUTATION 1: fold with `>=` instead of `>` and the "first at the top
/// severity" assertion fails — the LAST structural candidate would be reported
/// instead of the first.
/// MUTATION 2: drop the severity ordering (record every outcome
/// unconditionally) and the words describe the trailing skew, which is how a
/// structural gap in the same prewarm goes unpublished.
#[test]
fn d47_shadow_walk_recheck_summary_keeps_the_first_worst_element() {
    let mut summary = Domain47ShadowWalkRecheckSummary::default();
    summary.observe(
        Domain47ShadowWalkRecheckOutcome::WalkHit,
        fc_element(11, Domain47ShadowWalkMapAxis::Leader),
    );
    summary.observe(
        Domain47ShadowWalkRecheckOutcome::WalkMissed,
        fc_element(22, Domain47ShadowWalkMapAxis::Advisor),
    );
    summary.observe(
        Domain47ShadowWalkRecheckOutcome::WalkMissed,
        fc_element(33, Domain47ShadowWalkMapAxis::Leader),
    );
    summary.observe(
        Domain47ShadowWalkRecheckOutcome::WalkHit,
        fc_element(44, Domain47ShadowWalkMapAxis::Leader),
    );
    summary.observe(
        Domain47ShadowWalkRecheckOutcome::Refused,
        fc_element(55, Domain47ShadowWalkMapAxis::Leader),
    );
    assert_eq!(summary.outcome, Domain47ShadowWalkRecheckOutcome::WalkMissed);
    assert_eq!(
        summary.element.char_id, 22,
        "the FIRST element at the top severity"
    );
    assert_eq!(
        summary.element.axis,
        Domain47ShadowWalkMapAxis::Advisor as u32
    );
    // The LOCATION words move with it, not independently (lane FS): the
    // element struct is latched whole, so the body address beside the outcome
    // is the body of the element the outcome describes.
    assert_eq!(
        summary.element,
        fc_element(22, Domain47ShadowWalkMapAxis::Advisor),
        "every describing word comes from ONE element"
    );
    assert_eq!((summary.hits, summary.misses, summary.refused), (2, 2, 1));
    assert_eq!(
        summary.performed(),
        5,
        "every re-checked element lands in exactly one of the three counters, \
         so their sum is what was examined"
    );
    // A prewarm with nothing to classify describes no element at all.
    let untouched = Domain47ShadowWalkRecheckSummary::default();
    assert_eq!(
        untouched.outcome,
        Domain47ShadowWalkRecheckOutcome::NotChecked
    );
    assert_eq!(untouched.performed(), 0);
}

/// THE PRODUCTION LEADER GUARD compares BOTH halves of the owner.
///
/// This is the predicate the re-check's ABA guard actually calls, so it is
/// where the country comparison has to be pinned.
///
/// MUTATION: delete the `country_tag_db_id` term from
/// `domain47_leader_identity_matches_owner` and the different-country case
/// returns true — an address recycled into ANOTHER country's leader object
/// would be published as a benign, reached-on-recheck residual.
#[test]
fn d47_leader_owner_guard_refuses_a_same_id_different_country_object() {
    let owner = Domain47RoleOwner {
        country_tag_db_id: 11,
        char_id: 7,
        character_addr: 0x9000,
    };
    assert!(domain47_leader_identity_matches_owner(&Ok((11, 7, 0x9000)), owner));
    assert!(
        !domain47_leader_identity_matches_owner(&Ok((12, 7, 0x9000)), owner),
        "same char_id under a different country is a DIFFERENT character; this \
         world does not prove char_id unique"
    );
    assert!(!domain47_leader_identity_matches_owner(&Ok((11, 8, 0x9000)), owner));
    assert!(
        !domain47_leader_identity_matches_owner(
            &Err(domain47::extractor::ExtractError::NullPointer(
                "CCountryLeader.character"
            )),
            owner
        ),
        "an identity that cannot be re-read is not a match"
    );
}

/// THE PRODUCTION ADVISOR GUARD, same property, plus the classification it
/// must not skip: an object that has stopped being `Appointed` is no longer
/// the thing the sweep's live set admitted.
///
/// MUTATION 1: delete the `country_tag_db_id` term and the different-country
/// case returns true.
/// MUTATION 2: drop the `Appointed` test and the token-less probe returns
/// true — a dead advisor would count as reached.
#[test]
fn d47_advisor_owner_guard_refuses_a_different_country_and_an_unappointed_object() {
    let owner = Domain47RoleOwner {
        country_tag_db_id: 11,
        char_id: 7,
        character_addr: 0x9000,
    };
    let probe = |country_tag_db_id: u32, char_id: u32, idea_token: Option<&str>| {
        Domain47AdvisorProbe {
            character: Ok(domain47::extractor::AdvisorRawFields {
                advisor_addr: 0x1000,
                character_addr: 0x2000,
                country_tag_db_id,
                char_id,
            }),
            token_header: None,
            idea_token: idea_token.map(str::to_owned),
            slot: None,
        }
    };
    assert!(domain47_advisor_probe_matches_owner(
        &probe(11, 7, Some("advisor_token")),
        owner
    ));
    assert!(
        !domain47_advisor_probe_matches_owner(&probe(12, 7, Some("advisor_token")), owner),
        "same char_id under a different country is a different character"
    );
    assert!(!domain47_advisor_probe_matches_owner(
        &probe(11, 8, Some("advisor_token")),
        owner
    ));
    assert!(
        !domain47_advisor_probe_matches_owner(&probe(11, 7, None), owner),
        "a token-less object classifies Dead and is no longer the advisor the \
         sweep's live set admitted"
    );
    assert!(
        !domain47_advisor_probe_matches_owner(
            &Domain47AdvisorProbe {
                character: Err(domain47::extractor::ExtractError::NullPointer(
                    "CAdvisor.character"
                )),
                token_header: None,
                idea_token: Some("advisor_token".to_owned()),
                slot: None,
            },
            owner
        ),
        "a broken character chain is not a match"
    );
}

/// `NotChecked` is the absence of a re-check and may not be counted as one:
/// folding it into any counter would make `performed()` claim work that never
/// happened.
///
/// MUTATION: route the `NotChecked` arm into `refused` (the natural catch-all)
/// and this fails.
#[test]
fn d47_shadow_walk_recheck_summary_never_counts_a_check_that_did_not_run() {
    let mut summary = Domain47ShadowWalkRecheckSummary::default();
    summary.observe(
        Domain47ShadowWalkRecheckOutcome::NotChecked,
        fc_element(77, Domain47ShadowWalkMapAxis::Leader),
    );
    assert_eq!(summary, Domain47ShadowWalkRecheckSummary::default());
}

// === WHERE THE sweep_only ELEMENT IS (lane FS, 2026-08-08) ================
//
// `domain47_shadow_walk_owner_probe` answers the question the outcome codes
// cannot: the re-check says the walk did not reach the object, and these say
// which body the sweep attributed it to, whether the census visits that body,
// and whether that body's own map holds it. Each test below is one of the four
// worlds a bare `WalkMissed` collapses together, and they are written so that
// collapsing any two of them fails.

/// AN UNCENSUSED BODY HOLDING THE ENTRY — hypotheses (a) and (b).
///
/// The roster carries a duplicate body for `char_id` 7 whose leader map is
/// empty, and a SECOND body carrying the same character is laid out in memory
/// and left off the roster with the live role object in its map. The re-check
/// reports `WalkMissed` because the only body it may examine is the roster's;
/// the probe names the other one and says the census never visits it.
///
/// This is the shape that keeps the sweep alive, and the instrument as it stood
/// could not tell it from the shape that would retire it.
///
/// MUTATION: have the probe look the owner up by `char_id` instead of by the
/// body pointer the sweep resolved, and `Censused` comes back for a body the
/// roster does not carry — the finding inverts.
#[test]
fn d47_shadow_walk_owner_probe_names_an_uncensused_body_holding_the_entry() {
    // Two bodies for char 7: the roster's (empty leader map) and an off-roster
    // one that will hold the entry.
    let world = FcRecheckWorld::with_off_roster(
        &[
            (FC_COUNTRY, 7, Vec::new(), Vec::new()),
            (FC_COUNTRY, 7, vec![0], Vec::new()),
        ],
        1,
    );
    let target = world.role(0);
    world.arena.put_u64(
        world.laid_out[1] + 0x2000 + DOMAIN47_LEADER_NODE_VALUE_OFF,
        target,
    );
    assert_eq!(
        world.recheck(target, 7, Domain47ShadowWalkMapAxis::Leader, true),
        Domain47ShadowWalkRecheckOutcome::WalkMissed,
        "the roster body walks clean and does not hold it, which is all the \
         re-check can say"
    );
    let owner = Domain47RoleOwner {
        country_tag_db_id: FC_COUNTRY,
        char_id: 7,
        character_addr: world.body(1),
    };
    let probe = world.owner_probe(target, owner, Domain47ShadowWalkMapAxis::Leader);
    assert_eq!(
        probe.censused,
        Domain47ShadowWalkOwnerBody::NotCensused,
        "the manager vectors do not carry this body, so the census-time walk \
         never walked its maps"
    );
    assert_eq!(
        probe.map,
        Domain47ShadowWalkOwnerMap::Holds,
        "and that body's own leader map DOES hold the object the sweep saw"
    );
    assert_eq!(probe.map_size, 1);
    assert_eq!(probe.roster_char_id, 0, "no roster entry, no roster char_id");
}

/// A CENSUSED BODY WHOSE ROSTER `char_id` DISAGREES — hypothesis (c).
///
/// The owning body IS in the roster and DOES hold the entry, but the roster
/// recorded a different `char_id` for it than the sweep's identity read
/// attributed, so the re-check's candidate filter never selects it and reports
/// `WalkMissed` off an unrelated duplicate. The probe publishes both numbers
/// and the disagreement is the finding.
///
/// MUTATION: report the OWNER's `char_id` instead of the roster's and the two
/// words agree on every capture, which is the one thing this pair may not do.
#[test]
fn d47_shadow_walk_owner_probe_publishes_the_roster_id_of_a_censused_body() {
    // Body 0 is the decoy the filter selects; body 1 is the real owner, in the
    // roster under char_id 8.
    let world = FcRecheckWorld::new(&[(7, Vec::new(), Vec::new()), (8, vec![0], Vec::new())]);
    let target = world.role(0);
    world.arena.put_u64(
        world.laid_out[1] + 0x2000 + DOMAIN47_LEADER_NODE_VALUE_OFF,
        target,
    );
    assert_eq!(
        world.recheck(target, 7, Domain47ShadowWalkMapAxis::Leader, true),
        Domain47ShadowWalkRecheckOutcome::WalkMissed
    );
    let owner = Domain47RoleOwner {
        country_tag_db_id: FC_COUNTRY,
        char_id: 7,
        character_addr: world.body(1),
    };
    let probe = world.owner_probe(target, owner, Domain47ShadowWalkMapAxis::Leader);
    assert_eq!(probe.censused, Domain47ShadowWalkOwnerBody::Censused);
    assert_eq!(probe.map, Domain47ShadowWalkOwnerMap::Holds);
    assert_eq!(
        probe.roster_char_id, 8,
        "the ROSTER's number for that body, which is what the candidate filter \
         compares — and it is not the owner's 7"
    );
}

/// NO CHARACTER MAP HOLDS IT — hypothesis (d), the structural gap.
///
/// The owning body is censused, its leader map walked cleanly, and the entry
/// is not in it. `map_size` separates the two readings of that: a container
/// with other entries and not this one, versus an empty container.
///
/// MUTATION: report `CleanWithout` when the header read refuses and the
/// refusal in the test below is published as "the container does not hold it",
/// which is a conclusion the probe never reached.
#[test]
fn d47_shadow_walk_owner_probe_reports_a_clean_container_without_the_entry() {
    let world = FcRecheckWorld::new(&[(7, vec![0], Vec::new())]);
    let target = world.role(0);
    // A DIFFERENT live role object in the container. Written after
    // construction because the walk admits only heap-window pointers and
    // `role()` is the fixture's supply of them, which is not knowable until the
    // arena exists.
    world.arena.put_u64(
        world.laid_out[0] + 0x2000 + DOMAIN47_LEADER_NODE_VALUE_OFF,
        world.role(1),
    );
    let owner = Domain47RoleOwner {
        country_tag_db_id: FC_COUNTRY,
        char_id: 7,
        character_addr: world.body(0),
    };
    let probe = world.owner_probe(target, owner, Domain47ShadowWalkMapAxis::Leader);
    assert_eq!(probe.censused, Domain47ShadowWalkOwnerBody::Censused);
    assert_eq!(probe.map, Domain47ShadowWalkOwnerMap::CleanWithout);
    assert_eq!(
        probe.map_size, 1,
        "a container that HAS an entry, and it is not this one — which is a \
         different finding from an empty container"
    );
    assert_eq!(probe.roster_char_id, 7);

    // The empty-container reading, on the same axis and the same code.
    let empty = FcRecheckWorld::new(&[(7, Vec::new(), Vec::new())]);
    let empty_probe = empty.owner_probe(
        empty.role(0),
        Domain47RoleOwner {
            country_tag_db_id: FC_COUNTRY,
            char_id: 7,
            character_addr: empty.body(0),
        },
        Domain47ShadowWalkMapAxis::Leader,
    );
    assert_eq!(empty_probe.map, Domain47ShadowWalkOwnerMap::CleanWithout);
    assert_eq!(
        empty_probe.map_size, 0,
        "and the size is what tells the two apart"
    );
}

/// A REFUSAL IS NOT AN ANSWER, and the size it refused on is still published.
///
/// `_Mysize` claims more nodes than the tree has, which is the walk's own tear
/// check. The probe must publish `Refused` — never `CleanWithout`, which would
/// read as "no character map holds this" off a map nobody walked — and the raw
/// size, because a size the walk will not trust is exactly what a reader of the
/// refusal wants.
///
/// MUTATION: clamp `map_size` to `DOMAIN47_SHADOW_WALK_MAX_NODES` or zero it on
/// a refusal and the assertion below fails.
#[test]
fn d47_shadow_walk_owner_probe_refuses_a_map_it_could_not_walk() {
    let world = FcRecheckWorld::new(&[(7, vec![0], Vec::new())]);
    let target = world.role(0);
    world
        .arena
        .put_u64(world.laid_out[0] + DOMAIN47_LEADER_MAP_SIZE_OFF, 9);
    let probe = world.owner_probe(
        target,
        Domain47RoleOwner {
            country_tag_db_id: FC_COUNTRY,
            char_id: 7,
            character_addr: world.body(0),
        },
        Domain47ShadowWalkMapAxis::Leader,
    );
    assert_eq!(
        probe.map,
        Domain47ShadowWalkOwnerMap::Refused,
        "one node reachable under a size claiming nine is the walk's tear \
         check, and a refusal is not a conclusion"
    );
    assert_eq!(probe.map_size, 9, "the size it refused on, raw");
    assert_eq!(
        probe.censused,
        Domain47ShadowWalkOwnerBody::Censused,
        "roster membership is a pointer compare and does not depend on the walk"
    );
}

/// AN OWNER WITH NO BODY ADDRESS DESCRIBES NOTHING.
///
/// Zero is the reserved "nothing to describe" every other d47 locator uses, and
/// a probe that reported `NotCensused` for it would publish the strongest
/// finding this instrument has off an owner that never named a body.
///
/// MUTATION: drop the zero test and the roster lookup fails to find address 0,
/// which reports `NotCensused` — a manufactured structural finding.
#[test]
fn d47_shadow_walk_owner_probe_describes_nothing_without_a_body_address() {
    let world = FcRecheckWorld::new(&[(7, vec![0], Vec::new())]);
    let probe = world.owner_probe(
        world.role(0),
        fc_bench_owner(7),
        Domain47ShadowWalkMapAxis::Leader,
    );
    assert_eq!(probe, Domain47ShadowWalkOwnerProbe::default());
    assert_eq!(probe.censused, Domain47ShadowWalkOwnerBody::NotDerived);
    assert_eq!(probe.map, Domain47ShadowWalkOwnerMap::NotAttempted);
}

/// THE RE-CHECK REPORTS WHAT IT EXAMINED, and only what it examined.
///
/// Two roster bodies carry `char_id` 7, holding two and three entries; neither
/// holds the target. `candidate_bodies` 2 is what says a duplicate could have
/// been masking an uncensused owner, and `candidate_entries` 5 is what says the
/// containers were not empty.
///
/// MUTATION: reduce the accumulation to presence — `usize::from(!found.is_empty())`
/// instead of `found.len()` — and the five-entry assertion reads 2, which is a
/// count of containers wearing the name of a count of objects.
///
/// The refusing half below is a REGRESSION guard rather than a mutation target,
/// and the difference is worth stating: moving the accumulation above the
/// refusal `continue` would NOT turn it red, because `found` is cleared before
/// each body and `domain47_shadow_walk_map` merges `scratch` into `found` only
/// after its tear check passes — a refused walk leaves `found` empty by
/// construction. What the assertion pins is that the zero stays zero if that
/// construction ever changes (a partial merge, a `found` reused across bodies).
/// (Codex 019fe00d, finding 3: the mutation this comment used to claim was not
/// reachable.)
#[test]
fn d47_shadow_walk_recheck_report_counts_the_bodies_and_entries_it_examined() {
    let world = FcRecheckWorld::new(&[
        (7, vec![0, 0], Vec::new()),
        (7, vec![0, 0, 0], Vec::new()),
    ]);
    let target = world.role(0);
    // Five other live role objects across the two containers, written after
    // construction for the reason the test above states: only `role()` supplies
    // addresses the walk's pointer gate admits.
    for (ordinal, (body, node)) in [(0usize, 0usize), (0, 1), (1, 0), (1, 1), (1, 2)]
        .into_iter()
        .enumerate()
    {
        world.arena.put_u64(
            world.laid_out[body] + 0x2000 + node * EJ_NODE_STRIDE + DOMAIN47_LEADER_NODE_VALUE_OFF,
            world.role(ordinal + 1),
        );
    }
    let report = world.recheck_report(target, 7, Domain47ShadowWalkMapAxis::Leader);
    assert_eq!(report.outcome, Domain47ShadowWalkRecheckOutcome::WalkMissed);
    assert_eq!(report.candidate_bodies, 2);
    assert_eq!(
        report.candidate_entries, 5,
        "both containers were walked and both had contents"
    );

    // A body whose map refuses contributes NO entries: a zero there would read
    // as an empty container rather than as one that was never observed.
    let refusing = FcRecheckWorld::new(&[(7, vec![0, 0], Vec::new())]);
    refusing
        .arena
        .put_u64(refusing.laid_out[0] + DOMAIN47_LEADER_MAP_SIZE_OFF, 9);
    let refused = refusing.recheck_report(refusing.role(0), 7, Domain47ShadowWalkMapAxis::Leader);
    assert_eq!(refused.outcome, Domain47ShadowWalkRecheckOutcome::Refused);
    assert_eq!(refused.candidate_bodies, 1);
    assert_eq!(
        refused.candidate_entries, 0,
        "a refused container was not observed and contributes nothing"
    );
}

/// THE SWEEP'S TEAR CHECK COMPARES THE IDENTITY, NOT THE BODY IT CAME THROUGH.
///
/// The leader identity read returns the `CCharacter` body it walked through as
/// a third element (lane FS). The double-read tear check in the sweep loop must
/// keep comparing the PAIR: this world carries duplicate `CCharacter` bodies
/// for one logical character, so two passes resolving the same character
/// through different bodies observed one identity. Refusing them would push an
/// ordinary hit into `skipped_root_rows`, which feeds
/// `domain47_prewarm_adoption_blocked` — an observe-only instrument would have
/// started blocking d47 adoptions.
///
/// The test is written against `domain47_classify_leader_identity` — the whole
/// acceptance rule the sweep loop calls — and not against
/// `domain47_identity_pair`. A test of the pair helper alone proves only that a
/// narrower comparison EXISTS, never that the loop uses it, so the mutation it
/// claimed to catch would have walked straight past it.
/// (Codex 019fe00d, finding 2.)
///
/// MUTATION 1: change the accept arm to `(Ok(a), Ok(b)) if a == b` and the
/// different-body case below returns `Torn` — the whole-tuple comparison this
/// exists to keep out.
/// MUTATION 2: fold the differing-error case into `Excluded` and the
/// differing-error assertion fails: two passes that failed DIFFERENTLY did not
/// observe one world, and counting that as a stable exclusion hides a torn
/// read.
#[test]
fn d47_leader_identity_tear_check_compares_the_pair_and_not_the_body() {
    let moved_body = (7u32, 3737u32, 0xA000u64);
    let same_identity_other_body = (7u32, 3737u32, 0xB000u64);
    let other_identity = (7u32, 3738u32, 0xA000u64);
    assert_eq!(
        domain47_classify_leader_identity(Ok(moved_body), Ok(same_identity_other_body)),
        Domain47LeaderIdentityVerdict::Accept {
            country_tag_db_id: 7,
            char_id: 3737,
            character_addr: 0xA000,
        },
        "the same character reached through two duplicate bodies is ONE \
         identity: accepted, with the FIRST observation's body address carried \
         along. Refusing it would push an ordinary hit into skipped_root_rows"
    );
    assert_ne!(
        moved_body, same_identity_other_body,
        "and a whole-tuple `==` is what would refuse it"
    );
    assert_eq!(
        domain47_classify_leader_identity(Ok(moved_body), Ok(other_identity)),
        Domain47LeaderIdentityVerdict::Torn,
        "a real identity disagreement is still torn"
    );
    let null = |field| domain47::extractor::ExtractError::NullPointer(field);
    assert_eq!(
        domain47_classify_leader_identity(
            Err(null("CCountryLeader.character")),
            Err(null("CCountryLeader.character"))
        ),
        Domain47LeaderIdentityVerdict::Excluded,
        "two passes that failed identically are a stable exclusion"
    );
    assert_eq!(
        domain47_classify_leader_identity(
            Err(null("CCountryLeader.character")),
            Err(null("CCharacter.char_id"))
        ),
        Domain47LeaderIdentityVerdict::Torn,
        "and two that failed differently are not"
    );
    assert_eq!(
        domain47_classify_leader_identity(Ok(moved_body), Err(null("CCountryLeader.character"))),
        Domain47LeaderIdentityVerdict::Torn
    );
}

/// THE TWO ABA GUARDS IGNORE THE BODY LOCATOR.
///
/// `Domain47RoleOwner` now carries the `CCharacter` address the sweep resolved,
/// and it is a LOCATOR: an address is exactly what an ABA reuse moves, and a
/// role object legitimately re-resolved through a duplicate body of the same
/// character must still be a hit. Both guards compare the country and the
/// character and nothing else.
///
/// MUTATION: add `character_addr` to either guard's comparison and the
/// different-body cases below start refusing, turning ordinary re-check hits
/// into `Refused` and hiding the residual behind an inconclusive verdict.
#[test]
fn d47_owner_guards_ignore_the_body_locator() {
    let owner = Domain47RoleOwner {
        country_tag_db_id: 11,
        char_id: 7,
        character_addr: 0x9000,
    };
    assert!(
        domain47_leader_identity_matches_owner(&Ok((11, 7, 0xA000)), owner),
        "the identity is the pair; the body it was reached through is not part \
         of it"
    );
    assert!(domain47_leader_identity_matches_owner(
        &Ok((11, 7, 0x9000)),
        owner
    ));
    assert!(
        !domain47_leader_identity_matches_owner(&Ok((12, 7, 0x9000)), owner),
        "and the pair still decides"
    );
    let advisor = |character_addr: u64| Domain47AdvisorProbe {
        character: Ok(domain47::extractor::AdvisorRawFields {
            advisor_addr: 0x1000,
            character_addr,
            country_tag_db_id: 11,
            char_id: 7,
        }),
        token_header: None,
        idea_token: Some("advisor_token".to_owned()),
        slot: None,
    };
    assert!(domain47_advisor_probe_matches_owner(&advisor(0xA000), owner));
    assert!(domain47_advisor_probe_matches_owner(&advisor(0x9000), owner));
}

// === THE ACCEPTANCE A/B (lane FU, 2026-08-08) =============================
//
// WHAT THIS ANSWERS. Lane FT established from the engine that `CCountryLeader`
// has exactly ONE owning container -- the `_Tree` at `CCharacter+0x98` -- and
// that the heap sweep's extra population is stale-but-alive objects. That is an
// OWNERSHIP fact. It is not by itself a promotion proof, because d47 does not
// publish objects, it publishes ROWS: the same object list run through the
// acceptance rules, the tag-validity test, and a per-identity dedup that keeps
// ONE root per (country, char_id). Two enumerations can differ in objects and
// still agree on rows, or agree on objects and disagree on rows.
//
// So the promotion proof is a POPULATION comparison, and this is it, offline:
// stage one world, enumerate it BOTH ways, run the SAME two admission functions
// production runs over each enumeration, and compare what they admit.
//
// THE FIVE CLASSES the fixture stages, and what each is for
// (`FuRoleClass`, and the table the first test prints):
//
//   MapHeldLive    -- a role object a censused character's map holds, whose
//                     identity read validates. Both paths must admit it.
//   StaleDuplicate -- a live-looking object NO map holds, whose character is the
//                     SAME (country, char_id) as a MapHeldLive object, placed at
//                     a LOWER address so it wins the sweep's per-identity dedup.
//                     This is FT's "can LOSE a published row": the sweep keeps
//                     the stale address and discards the map-held one, and the
//                     frame walk then refuses the root it kept.
//   StaleOrphan    -- a live-looking object no map holds, whose character holds
//                     no current role at all. FT's "can INVENT one": the sweep
//                     publishes a row for a role that is no longer held.
//   MapHeldRefused -- a role object a map DOES hold whose identity read fails.
//                     The walk reaches it; the sweep's live set does not carry
//                     it; NEITHER admits it. This is the fixture's model of the
//                     ~997 walk-only objects per prewarm t37..t43 observed, and
//                     it is why `walk_only` being nonzero is not a defect.
//   Detached       -- a dead object no map holds and no identity read accepts.
//                     Neither path admits it; it exists so "the sweep sees more
//                     objects" is represented honestly.
//
// WHAT THE TESTS BELOW PIN, in order of what would be worst to get wrong:
//
//  1. every identity the WALK admits is one the SWEEP admits (no invention);
//  2. every identity the SWEEP admits and the walk does not is stale-derived
//     (no silent loss);
//  3. the sweep really does lose and invent rows here, so (1) and (2) are not
//     passing on an empty difference;
//  4. every walk-only OBJECT is refused by the same rules, so the walk's wider
//     object reach costs no row.

/// What one staged role object is, for the A/B table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum FuRoleClass {
    MapHeldLive,
    MapHeldRefused,
    StaleDuplicate,
    StaleOrphan,
    Detached,
}

impl FuRoleClass {
    fn name(self) -> &'static str {
        match self {
            FuRoleClass::MapHeldLive => "map-held live",
            FuRoleClass::MapHeldRefused => "map-held, identity refused",
            FuRoleClass::StaleDuplicate => "stale duplicate (identity also map-held)",
            FuRoleClass::StaleOrphan => "stale orphan (identity holds no role)",
            FuRoleClass::Detached => "detached / dead",
        }
    }
}

/// Characters the A/B world censuses. Small on purpose: this fixture is a
/// POPULATION proof, not a cost measurement, and every object in it has to be
/// individually accounted for in the table the test prints.
const FU_CHARACTERS: usize = 24;
const FU_CHARACTER_STRIDE: usize = 0x400;
const FU_ROLE_STRIDE: usize = 0x200;
const FU_NODE_STRIDE: usize = 0x80;
/// The first character whose leader map is EMPTY, so a `StaleOrphan` leader can
/// name a character that holds no current role.
const FU_EMPTY_LEADER_CHARACTER: usize = 20;
const FU_EMPTY_ADVISOR_CHARACTER: usize = 21;

/// One world, staged twice over: as role maps hanging off censused characters
/// (what the walk sees) and as a flat ascending address list (what the sweep
/// publishes).
struct FuAcceptanceWorld {
    _arena: BenchArena,
    roots: ResolvedTypeRoots,
    /// The census's visiting order, the shape
    /// `Domain47ManagerRosterSnapshot::characters` carries.
    roster: Vec<usize>,
    /// EVERY `CCountryLeader` / `CAdvisor` in the arena, ASCENDING by address --
    /// the order `heap_sweep` publishes hits in. Nothing here is filtered: the
    /// sweep sees the stale and the detached objects too, which is the whole
    /// difference the A/B is about.
    sweep_leader_hits: Vec<usize>,
    sweep_advisor_hits: Vec<usize>,
    leader_classes: BTreeMap<usize, FuRoleClass>,
    advisor_classes: BTreeMap<usize, FuRoleClass>,
}

/// A staged role object, before it is placed.
struct FuPlannedRole {
    class: FuRoleClass,
    /// Which censused character this object's back-pointer names.
    owner: usize,
    /// Which character's MAP holds it, if any.
    holder: Option<usize>,
    /// Advisors only: the idea token, or `None` for an unappointed object.
    token: Option<String>,
    /// Point the back-pointer at something that is not a `CCharacter`.
    break_chain: bool,
}

fn fu_character_id(index: usize) -> u32 {
    1_000 + index as u32
}

fn fu_country_tag(index: usize) -> u32 {
    1 + (index % 7) as u32
}

fn fu_acceptance_world() -> FuAcceptanceWorld {
    // Leaders and advisors are planned first so the STALE objects can be given
    // LOWER addresses than the map-held object they duplicate. That ordering is
    // not decoration: `heap_sweep` publishes ascending addresses and
    // `domain47_admit_country_leader_roles` keeps the FIRST root it sees for an
    // identity, so a stale object at a higher address would simply be deduped
    // away and the row loss FT described would never reproduce.
    let mut leaders: Vec<FuPlannedRole> = Vec::new();
    let mut advisors: Vec<FuPlannedRole> = Vec::new();

    // --- stale first (lowest addresses) ---
    leaders.push(FuPlannedRole {
        class: FuRoleClass::StaleDuplicate,
        owner: 0,
        holder: None,
        token: None,
        break_chain: false,
    });
    leaders.push(FuPlannedRole {
        class: FuRoleClass::StaleOrphan,
        owner: FU_EMPTY_LEADER_CHARACTER,
        holder: None,
        token: None,
        break_chain: false,
    });
    advisors.push(FuPlannedRole {
        class: FuRoleClass::StaleDuplicate,
        owner: 1,
        holder: None,
        token: Some("eco_advisor".to_string()),
        break_chain: false,
    });
    advisors.push(FuPlannedRole {
        class: FuRoleClass::StaleOrphan,
        owner: FU_EMPTY_ADVISOR_CHARACTER,
        holder: None,
        token: Some("orphan_advisor".to_string()),
        break_chain: false,
    });

    // --- the live world ---
    for index in 0..8usize {
        leaders.push(FuPlannedRole {
            class: FuRoleClass::MapHeldLive,
            owner: index,
            holder: Some(index),
            token: None,
            break_chain: false,
        });
    }
    // Two roles on ONE character (the per-ideology case): a second object whose
    // identity is already published, so both paths must dedup it to one row.
    leaders.push(FuPlannedRole {
        class: FuRoleClass::MapHeldLive,
        owner: 3,
        holder: Some(3),
        token: None,
        break_chain: false,
    });
    // A role held by a character the census visits, whose identity read fails:
    // the walk reaches it, the sweep's live set does not carry it.
    for index in 8..10usize {
        leaders.push(FuPlannedRole {
            class: FuRoleClass::MapHeldRefused,
            owner: index,
            holder: Some(index),
            token: None,
            break_chain: true,
        });
    }
    for index in 0..6usize {
        advisors.push(FuPlannedRole {
            class: FuRoleClass::MapHeldLive,
            owner: index,
            holder: Some(index),
            token: Some(if index == 1 {
                // The identity the stale duplicate above also claims.
                "eco_advisor".to_string()
            } else {
                format!("advisor_token_{index}")
            }),
            break_chain: false,
        });
    }
    // Appointed-but-tokenless: the oracle-validated `Dead` class, held by a map.
    advisors.push(FuPlannedRole {
        class: FuRoleClass::MapHeldRefused,
        owner: 12,
        holder: Some(12),
        token: None,
        break_chain: false,
    });

    // --- the detached tail ---
    for index in 0..3usize {
        leaders.push(FuPlannedRole {
            class: FuRoleClass::Detached,
            owner: index,
            holder: None,
            token: None,
            break_chain: true,
        });
        advisors.push(FuPlannedRole {
            class: FuRoleClass::Detached,
            owner: index,
            holder: None,
            token: None,
            break_chain: true,
        });
    }

    let node_slots = leaders.len() + advisors.len() + 2 * FU_CHARACTERS;
    let size = FU_CHARACTERS * FU_CHARACTER_STRIDE
        + (leaders.len() + advisors.len()) * FU_ROLE_STRIDE
        + (node_slots + 1) * FU_NODE_STRIDE
        + 0x4000;
    let arena = BenchArena::in_heap_window(size);
    let char_base = arena.base;
    let leader_base = char_base + FU_CHARACTERS * FU_CHARACTER_STRIDE;
    let advisor_base = leader_base + leaders.len() * FU_ROLE_STRIDE;
    let node_base = advisor_base + advisors.len() * FU_ROLE_STRIDE;

    let roots = ResolvedTypeRoots {
        ccharacter_vtable: BENCH_CCHARACTER_VTABLE,
        carmy_leader_vtable: BENCH_CARMY_LEADER_VTABLE,
        cnavy_leader_vtable: BENCH_CNAVY_LEADER_VTABLE,
        ccountry_leader_vtable: BENCH_CCOUNTRY_LEADER_VTABLE,
        cadvisor_vtable: BENCH_CADVISOR_VTABLE,
        cunit_leader_trait_vtable: BENCH_CUNIT_LEADER_TRAIT_VTABLE,
    };

    let character_at = |index: usize| char_base + index * FU_CHARACTER_STRIDE;
    for index in 0..FU_CHARACTERS {
        let addr = character_at(index);
        arena.put_u64(addr, BENCH_CCHARACTER_VTABLE);
        arena.put_u32(addr + CCHARACTER_TYPE_OFF as usize, CCHARACTER_TYPE_ID);
        arena.put_u32(addr + CCHARACTER_ID_OFF as usize, fu_character_id(index));
        arena.put_u32(
            addr + CCHARACTER_COUNTRY_TAG_DB_ID_OFF as usize,
            fu_country_tag(index),
        );
        arena.put_u64(addr + CCHARACTER_UNIT_LEADER_PTR_OFF as usize, 0);
    }

    // Place the role objects, then build each holder's `_Tree` over the ones it
    // holds.
    let mut leader_classes = BTreeMap::new();
    let mut advisor_classes = BTreeMap::new();
    let mut leader_entries: BTreeMap<usize, Vec<u64>> = BTreeMap::new();
    let mut advisor_entries: BTreeMap<usize, Vec<u64>> = BTreeMap::new();
    let mut sweep_leader_hits = Vec::new();
    let mut sweep_advisor_hits = Vec::new();
    // A non-`CCharacter` body a broken chain can point at: readable, in window,
    // and carrying the wrong vtable, which is what the identity read refuses on.
    let decoy = node_base + node_slots * FU_NODE_STRIDE;
    arena.put_u64(decoy, BENCH_CUNIT_LEADER_TRAIT_VTABLE);

    for (ordinal, plan) in leaders.iter().enumerate() {
        let addr = leader_base + ordinal * FU_ROLE_STRIDE;
        arena.put_u64(addr, BENCH_CCOUNTRY_LEADER_VTABLE);
        let character = if plan.break_chain {
            decoy
        } else {
            character_at(plan.owner)
        };
        arena.put_u64(
            addr + CCOUNTRY_LEADER_CHARACTER_PTR_OFF as usize,
            character as u64,
        );
        leader_classes.insert(addr, plan.class);
        sweep_leader_hits.push(addr);
        if let Some(holder) = plan.holder {
            leader_entries.entry(holder).or_default().push(addr as u64);
        }
    }
    for (ordinal, plan) in advisors.iter().enumerate() {
        let addr = advisor_base + ordinal * FU_ROLE_STRIDE;
        arena.put_u64(addr, BENCH_CADVISOR_VTABLE);
        let character = if plan.break_chain {
            decoy
        } else {
            character_at(plan.owner)
        };
        arena.put_u64(addr + CADVISOR_CHARACTER_PTR_OFF as usize, character as u64);
        if let Some(token) = plan.token.as_ref() {
            arena.put_msvc_string(
                addr + domain47::extractor::CADVISOR_IDEA_TOKEN_OFF as usize,
                addr + 0x100,
                token,
            );
        }
        advisor_classes.insert(addr, plan.class);
        sweep_advisor_hits.push(addr);
        if let Some(holder) = plan.holder {
            advisor_entries.entry(holder).or_default().push(addr as u64);
        }
    }

    let mut node_slot = 0usize;
    let mut take_slot = |count: usize| {
        let head = node_base + node_slot * FU_NODE_STRIDE;
        node_slot += 1;
        let nodes: Vec<usize> = (0..count)
            .map(|_| {
                let at = node_base + node_slot * FU_NODE_STRIDE;
                node_slot += 1;
                at
            })
            .collect();
        (head, nodes)
    };
    for index in 0..FU_CHARACTERS {
        let addr = character_at(index);
        for (head_off, size_off, value_off, entries) in [
            (
                DOMAIN47_LEADER_MAP_HEAD_OFF,
                DOMAIN47_LEADER_MAP_SIZE_OFF,
                DOMAIN47_LEADER_NODE_VALUE_OFF,
                leader_entries.remove(&index).unwrap_or_default(),
            ),
            (
                DOMAIN47_ADVISOR_MAP_HEAD_OFF,
                DOMAIN47_ADVISOR_MAP_SIZE_OFF,
                DOMAIN47_ADVISOR_NODE_VALUE_OFF,
                advisor_entries.remove(&index).unwrap_or_default(),
            ),
        ] {
            arena.put_u64(addr + size_off, entries.len() as u64);
            if entries.is_empty() {
                // A live empty map still HAS a sentinel; the walk short-circuits
                // on `_Mysize == 0` before reading it.
                arena.put_u64(addr + head_off, (addr + 0x200) as u64);
                continue;
            }
            let (head, nodes) = take_slot(entries.len());
            ej_put_tree(&arena, head, &|slot| nodes[slot], value_off, &entries);
            arena.put_u64(addr + head_off, head as u64);
        }
    }

    sweep_leader_hits.sort_unstable();
    sweep_advisor_hits.sort_unstable();
    FuAcceptanceWorld {
        _arena: arena,
        roots,
        roster: (0..FU_CHARACTERS).map(character_at).collect(),
        sweep_leader_hits,
        sweep_advisor_hits,
        leader_classes,
        advisor_classes,
    }
}

/// What ONE discovery path admitted.
#[derive(Default)]
struct FuAdmitted {
    leader_roots: Vec<CountryLeaderRoleRoot>,
    advisor_roots: Vec<AdvisorRoot>,
    /// Distinct role-object pointers the path enumerated (the walk's sets; for
    /// the sweep, its whole hit list).
    leader_objects: BTreeSet<u64>,
    advisor_objects: BTreeSet<u64>,
    /// The sweep's LIVE set, for the walk-only classification.
    leader_live: Domain47RoleLiveSet,
    advisor_live: Domain47RoleLiveSet,
    skipped_root_rows: u32,
    excluded_scan_hits: u32,
}

impl FuAdmitted {
    /// The published ROW identities, which is what the two paths are compared
    /// on: `(country_tag_db_id, char_id)` for a leader row -- exactly what
    /// `seen_role_identities` dedups on -- and that pair plus the idea token for
    /// an advisor row, which is `advisor_root_by_identity`'s key.
    ///
    /// THE WHOLE KEY, NOT THE CONVENIENT HALF (Codex 019fe1e5, finding 4).
    /// Comparing `char_id` alone would let a role attributed to the WRONG
    /// COUNTRY pass as population equality, which is the one mistake d47's
    /// dedup carries a country in its key to prevent: this world does not
    /// guarantee `char_id` uniqueness across the two manager vectors, and
    /// `Domain47CharacterAdmit::Ambiguous` exists for the duplicates that carry
    /// one `char_id` under two countries.
    ///
    /// The country is READ BACK off the object rather than taken from the root,
    /// because `CountryLeaderRoleRoot` deliberately no longer carries one (the
    /// row's country comes from the frame walk). Reading it here is the same
    /// identity read the admission used to accept the object.
    fn leader_identities(&self, world: &FuAcceptanceWorld) -> BTreeSet<(u32, u32)> {
        let mem = Domain47InProcessMemory;
        page_cache_new_frame();
        self.leader_roots
            .iter()
            .map(|root| {
                let (country, char_id, _) =
                    domain47_country_leader_identity(&mem, &world.roots, root.country_leader_addr)
                        .expect("an admitted leader root resolves");
                assert_eq!(char_id, root.char_id, "the root's identity half must agree");
                (country, char_id)
            })
            .collect()
    }

    fn advisor_identities(&self, world: &FuAcceptanceWorld) -> BTreeSet<(u32, u32, String)> {
        let mem = Domain47InProcessMemory;
        page_cache_new_frame();
        self.advisor_roots
            .iter()
            .map(|root| {
                let raw = domain47::extractor::read_advisor_raw_fields(
                    &mem,
                    &world.roots,
                    root.advisor_addr,
                )
                .expect("an admitted advisor root resolves");
                (raw.country_tag_db_id, raw.char_id, root.idea_token.clone())
            })
            .collect()
    }
}

/// PRODUCTION's discovery, reproduced call for call: per roster character, walk
/// the two role containers, then admit what that character's maps just
/// contributed.
///
/// The three calls and their ORDER are the same three
/// `prewarm_compact_characters_leaders_cabinet_roots` makes inside its census
/// loop; `d47_the_prewarm_admits_roles_from_the_walk_inside_the_census_loop`
/// is what keeps this reproduction honest if that loop is ever rewritten.
fn fu_admit_by_walk(world: &FuAcceptanceWorld) -> FuAdmitted {
    let mem = Domain47InProcessMemory;
    let mut out = FuAdmitted::default();
    let mut walk = Domain47ShadowWalk::default();
    let mut stack = Vec::new();
    let mut scratch = Vec::new();
    let mut seen = BTreeSet::new();
    let mut by_identity = BTreeMap::new();
    page_cache_new_frame();
    for addr in world.roster.iter().copied() {
        domain47_shadow_walk_character(addr, &mut walk, &mut stack, &mut scratch);
        domain47_admit_country_leader_roles(
            &mem,
            &world.roots,
            &walk.fresh_leaders,
            None,
            &mut seen,
            &mut out.leader_roots,
            &mut out.skipped_root_rows,
            &mut out.excluded_scan_hits,
        );
        domain47_admit_advisor_roots(
            &mem,
            &world.roots,
            &walk.fresh_advisors,
            None,
            &mut by_identity,
            &mut out.advisor_roots,
            &mut out.skipped_root_rows,
            &mut out.excluded_scan_hits,
        );
    }
    assert_eq!(walk.refused, 0, "the A/B fixture must refuse no map");
    out.leader_objects = walk.leaders;
    out.advisor_objects = walk.advisors;
    out
}

/// THE OLD discovery: the sweep's whole hit list through the same two functions.
fn fu_admit_by_sweep(world: &FuAcceptanceWorld) -> FuAdmitted {
    let mem = Domain47InProcessMemory;
    let mut out = FuAdmitted::default();
    let mut seen = BTreeSet::new();
    let mut by_identity = BTreeMap::new();
    page_cache_new_frame();
    domain47_admit_country_leader_roles(
        &mem,
        &world.roots,
        &world.sweep_leader_hits,
        Some(&mut out.leader_live),
        &mut seen,
        &mut out.leader_roots,
        &mut out.skipped_root_rows,
        &mut out.excluded_scan_hits,
    );
    domain47_admit_advisor_roots(
        &mem,
        &world.roots,
        &world.sweep_advisor_hits,
        Some(&mut out.advisor_live),
        &mut by_identity,
        &mut out.advisor_roots,
        &mut out.skipped_root_rows,
        &mut out.excluded_scan_hits,
    );
    out.leader_objects = world.sweep_leader_hits.iter().map(|a| *a as u64).collect();
    out.advisor_objects = world.sweep_advisor_hits.iter().map(|a| *a as u64).collect();
    out
}

/// THE A/B TABLE. Printed rather than only asserted, because the promotion
/// argument is a population comparison and a reader of this lane's report needs
/// the numbers, not a green tick.
#[test]
fn d47_walk_and_sweep_admit_the_same_population() {
    let world = fu_acceptance_world();
    let walk = fu_admit_by_walk(&world);
    let sweep = fu_admit_by_sweep(&world);

    let mut counts: BTreeMap<(&'static str, FuRoleClass), usize> = BTreeMap::new();
    for class in world.leader_classes.values() {
        *counts.entry(("leader", *class)).or_default() += 1;
    }
    for class in world.advisor_classes.values() {
        *counts.entry(("advisor", *class)).or_default() += 1;
    }
    println!("d47 acceptance A/B -- staged population");
    for ((family, class), count) in &counts {
        println!("  {family:8} {:40} {count}", class.name());
    }
    println!(
        "  objects enumerated: walk leaders {} advisors {} | sweep leaders {} advisors {}",
        walk.leader_objects.len(),
        walk.advisor_objects.len(),
        sweep.leader_objects.len(),
        sweep.advisor_objects.len(),
    );
    println!(
        "  ROOTS ADMITTED:     walk leaders {} advisors {} | sweep leaders {} advisors {}",
        walk.leader_roots.len(),
        walk.advisor_roots.len(),
        sweep.leader_roots.len(),
        sweep.advisor_roots.len(),
    );
    println!(
        "  excluded_scan_hits: walk {} | sweep {}   skipped_root_rows: walk {} | sweep {}",
        walk.excluded_scan_hits,
        sweep.excluded_scan_hits,
        walk.skipped_root_rows,
        sweep.skipped_root_rows,
    );

    // (1) THE WALK INVENTS NOTHING. Every row identity it publishes is one the
    // sweep publishes too.
    let walk_leader_ids = walk.leader_identities(&world);
    let sweep_leader_ids = sweep.leader_identities(&world);
    assert!(
        walk_leader_ids.is_subset(&sweep_leader_ids),
        "walk-only leader identities {:?}",
        walk_leader_ids
            .difference(&sweep_leader_ids)
            .collect::<Vec<_>>(),
    );
    let walk_advisor_ids = walk.advisor_identities(&world);
    let sweep_advisor_ids = sweep.advisor_identities(&world);
    assert!(
        walk_advisor_ids.is_subset(&sweep_advisor_ids),
        "walk-only advisor identities {:?}",
        walk_advisor_ids
            .difference(&sweep_advisor_ids)
            .collect::<Vec<_>>(),
    );

    // (2) EVERY IDENTITY THE WALK DROPS IS STALE-DERIVED. This is the whole
    // promotion claim: the sweep's surplus rows are the stale objects FT named
    // and nothing else.
    let dropped_leaders: Vec<(u32, u32)> = sweep_leader_ids
        .difference(&walk_leader_ids)
        .copied()
        .collect();
    assert_eq!(
        dropped_leaders,
        vec![(
            fu_country_tag(FU_EMPTY_LEADER_CHARACTER),
            fu_character_id(FU_EMPTY_LEADER_CHARACTER)
        )],
        "the only leader row the walk does not publish must be the stale orphan's"
    );
    let dropped_advisors: Vec<(u32, u32, String)> = sweep_advisor_ids
        .difference(&walk_advisor_ids)
        .cloned()
        .collect();
    assert_eq!(
        dropped_advisors,
        vec![(
            fu_country_tag(FU_EMPTY_ADVISOR_CHARACTER),
            fu_character_id(FU_EMPTY_ADVISOR_CHARACTER),
            "orphan_advisor".to_string()
        )],
        "the only advisor row the walk does not publish must be the stale orphan's"
    );

    // (3) THE DIFFERENCE IS REAL IN BOTH DIRECTIONS, so (1) and (2) are not
    // passing on an empty comparison.
    //
    // INVENTED: the sweep publishes a country-leader row for a character whose
    // map holds no role at all.
    assert!(
        sweep_leader_ids.contains(&(
            fu_country_tag(FU_EMPTY_LEADER_CHARACTER),
            fu_character_id(FU_EMPTY_LEADER_CHARACTER)
        )),
        "the fixture must actually stage the invented row"
    );
    // LOST: the sweep's row for character 0 is rooted at the STALE address, and
    // the map-held object it beat was discarded. That root is the one the frame
    // walk would later refuse -- the row is gone, and nothing on the wire says
    // so. The walk's root for the same identity is the map-held object.
    let stale_leader = *world
        .leader_classes
        .iter()
        .find(|(_, class)| **class == FuRoleClass::StaleDuplicate)
        .expect("the fixture stages a stale duplicate")
        .0 as u64;
    let sweep_root_for_zero = sweep
        .leader_roots
        .iter()
        .find(|root| root.char_id == fu_character_id(0))
        .expect("the sweep publishes character 0's role");
    assert_eq!(
        sweep_root_for_zero.country_leader_addr, stale_leader,
        "the stale duplicate must win the sweep's dedup, which is the row loss"
    );
    let walk_root_for_zero = walk
        .leader_roots
        .iter()
        .find(|root| root.char_id == fu_character_id(0))
        .expect("the walk publishes character 0's role");
    assert_ne!(
        walk_root_for_zero.country_leader_addr, stale_leader,
        "the walk must root the row at the object a map actually holds"
    );
    assert_eq!(
        world.leader_classes[&(walk_root_for_zero.country_leader_addr as usize)],
        FuRoleClass::MapHeldLive,
    );

    // (4) THE WALK'S WIDER OBJECT REACH COSTS NO ROW. Every object the walk
    // enumerated that the sweep's LIVE set does not carry is one the shared
    // acceptance rules refuse -- which is what the ~997 `walk_only` objects
    // t37..t43 published are, and why that direction never blocked promotion.
    let mut walk_only_leaders = 0usize;
    for addr in walk.leader_objects.iter() {
        if sweep.leader_live.contains_key(addr) {
            continue;
        }
        walk_only_leaders += 1;
        assert_eq!(
            world.leader_classes[&(*addr as usize)],
            FuRoleClass::MapHeldRefused,
            "walk-only leader object {addr:#x} must be one the rules refuse"
        );
        assert!(
            !walk
                .leader_roots
                .iter()
                .any(|root| root.country_leader_addr == *addr),
            "a refused object must not become a root"
        );
    }
    let mut walk_only_advisors = 0usize;
    for addr in walk.advisor_objects.iter() {
        if sweep.advisor_live.contains_key(addr) {
            continue;
        }
        walk_only_advisors += 1;
        assert_eq!(
            world.advisor_classes[&(*addr as usize)],
            FuRoleClass::MapHeldRefused,
            "walk-only advisor object {addr:#x} must be one the rules refuse"
        );
    }
    println!("  walk_only objects:  leaders {walk_only_leaders} advisors {walk_only_advisors}");
    assert!(
        walk_only_leaders > 0 && walk_only_advisors > 0,
        "the fixture must stage the walk-only direction, or (4) proves nothing"
    );
}

/// A ROLE ON A DUPLICATE BODY IS ADMITTED ONCE.
///
/// The walk runs over EVERY roster body, including the duplicates the character
/// dedup merges away, because the sweep admitted their roles too. What stops
/// that from double-publishing is the distinct-pointer set behind the
/// per-character hand-off, and this is what pins it.
#[test]
fn d47_walk_admits_a_role_once_across_duplicate_bodies() {
    let world = fu_acceptance_world();
    let mem = Domain47InProcessMemory;
    let mut walk = Domain47ShadowWalk::default();
    let mut stack = Vec::new();
    let mut scratch = Vec::new();
    let mut seen = BTreeSet::new();
    let mut roots = Vec::new();
    let mut skipped = 0u32;
    let mut excluded = 0u32;
    page_cache_new_frame();
    // Character 3 holds TWO role objects; visiting its body twice models a
    // duplicate `CCharacter` the heap carries for one logical character.
    let body = world.roster[3];
    let mut admitted_per_visit = Vec::new();
    for _ in 0..2 {
        domain47_shadow_walk_character(body, &mut walk, &mut stack, &mut scratch);
        let before = roots.len();
        domain47_admit_country_leader_roles(
            &mem,
            &world.roots,
            &walk.fresh_leaders,
            None,
            &mut seen,
            &mut roots,
            &mut skipped,
            &mut excluded,
        );
        admitted_per_visit.push((walk.fresh_leaders.len(), roots.len() - before));
    }
    assert_eq!(
        admitted_per_visit[0].0, 2,
        "the first visit contributes both of this character's role objects"
    );
    assert_eq!(
        admitted_per_visit[0].1, 1,
        "and they dedup to ONE row: same (country, char_id)"
    );
    assert_eq!(
        admitted_per_visit[1],
        (0, 0),
        "the second visit contributes NOTHING: the distinct-pointer set already \
         holds both objects, so a duplicate body cannot double-publish"
    );
}

/// The walk's per-character hand-off is CLEARED on a body it could not read.
///
/// Without the clear, a character whose body is unreadable would leave the
/// previous character's pointers in `fresh_*` and the caller would re-run the
/// full double identity read on every one of them -- for every unreadable body
/// in the roster. The mutation that reintroduces it is deleting two `clear()`
/// lines from `domain47_shadow_walk_character`.
#[test]
fn d47_walk_clears_its_hand_off_on_an_unreadable_body() {
    let world = EjSingleCharacterWorld::new(3, 2);
    let mut walk = Domain47ShadowWalk::default();
    let mut stack = Vec::new();
    let mut scratch = Vec::new();
    page_cache_new_frame();
    domain47_shadow_walk_character(world.character, &mut walk, &mut stack, &mut scratch);
    assert_eq!(walk.fresh_leaders.len(), 3);
    assert_eq!(walk.fresh_advisors.len(), 2);
    domain47_shadow_walk_character(HEAP_HI - 0x1000, &mut walk, &mut stack, &mut scratch);
    assert!(
        walk.fresh_leaders.is_empty() && walk.fresh_advisors.is_empty(),
        "an unreadable body contributes nothing, and must not re-offer the \
         previous body's pointers"
    );
    assert_eq!(walk.refused, 2);
}

/// A REFUSED map contributes nothing to the hand-off either, for the same
/// reason it contributes nothing to the distinct-pointer set: a partially
/// walked container is not an enumeration, and admitting off it would publish
/// roots the walk never proved the character held.
#[test]
fn d47_walk_hands_off_nothing_from_a_refused_map() {
    let world = EjSingleCharacterWorld::new(3, 2);
    world.arena.put_u64(
        world.character + DOMAIN47_LEADER_MAP_SIZE_OFF,
        world.leaders.len() as u64 + 1,
    );
    let mut walk = Domain47ShadowWalk::default();
    let mut stack = Vec::new();
    let mut scratch = Vec::new();
    page_cache_new_frame();
    domain47_shadow_walk_character(world.character, &mut walk, &mut stack, &mut scratch);
    assert_eq!(walk.refused, 1);
    assert!(
        walk.fresh_leaders.is_empty(),
        "a refused map must offer no roots"
    );
    assert_eq!(
        walk.fresh_advisors.len(),
        2,
        "the other map on the same character is unaffected"
    );
}

/// THE SHADOW'S COUNTERS MOVE, which is why the prewarm gives it its own.
///
/// `skipped_root_rows` feeds `domain47_prewarm_adoption_blocked`, so a shadow
/// sharing production's counters could BLOCK an adoption on the strength of
/// objects production never enumerated. This pins that the sweep's hit list
/// really does drive those counters -- the claim "the shadow's counters are its
/// own" is only worth making about counters that move.
#[test]
fn d47_sweep_shadow_counters_are_its_own() {
    let world = fu_acceptance_world();
    let mem = Domain47InProcessMemory;
    let mut live = Domain47RoleLiveSet::new();
    let mut seen = BTreeSet::new();
    let mut roots = Vec::new();
    let mut skipped = 0u32;
    let mut excluded = 0u32;
    page_cache_new_frame();
    domain47_admit_country_leader_roles(
        &mem,
        &world.roots,
        &world.sweep_leader_hits,
        Some(&mut live),
        &mut seen,
        &mut roots,
        &mut skipped,
        &mut excluded,
    );
    assert!(
        excluded > 0,
        "the sweep's hit list must exclude the detached objects"
    );
    assert!(!live.is_empty(), "and must still admit the live ones");
    assert!(
        live.len() > roots.len(),
        "the live set is taken BEFORE the per-identity dedup, or the residual \
         would report d47's row policy as a reachability fact"
    );
}

/// THE PREWARM ADMITS ROLES FROM THE WALK, INSIDE ITS CENSUS LOOP.
///
/// `fu_admit_by_walk` reproduces production's three calls; this is what says the
/// reproduction still describes production. Source text and not behaviour,
/// because the branch is otherwise reachable only through live process memory
/// and a running sweep service -- the same reason
/// `domain47_prewarm_holds_instead_of_recensusing` is a function.
///
/// It fails if the walk stops feeding the admission, if the admission moves out
/// of the census loop (losing the hot-body property the conversion exists for),
/// or if either sweep-hit loop comes back.
#[test]
fn d47_the_prewarm_admits_roles_from_the_walk_inside_the_census_loop() {
    let source = super::tests::collector_source_lf();
    let loop_head = source
        .find("for (ordinal, (addr, raw)) in manager_roster.characters.iter().enumerate() {")
        .expect("the census loop");
    let loop_tail = source[loop_head..]
        .find("\n    // Drop every root belonging to a quarantined identity")
        .expect("the end of the census loop")
        + loop_head;
    let body = &source[loop_head..loop_tail];
    assert!(
        body.contains("domain47_shadow_walk_character(\n            *addr,"),
        "the census loop must walk THIS iteration's body"
    );
    // WHAT FEEDS EACH ADMISSION, not merely that both are called (Codex
    // 019fe227, finding 4). `fresh_leaders` and `fresh_advisors` are the same
    // type, so swapping them -- or passing an empty slice, or a stale list --
    // compiles, silently misroutes or drops every role root, and satisfies a
    // call-presence assertion. Each call name is matched together with the
    // hand-off it must be given.
    for (call, feed) in [
        (
            "domain47_admit_country_leader_roles(",
            "&shadow_walk.fresh_leaders,",
        ),
        ("domain47_admit_advisor_roots(", "&shadow_walk.fresh_advisors,"),
    ] {
        let paired = format!("{call}\n            &mem,\n            &roots,\n            {feed}");
        assert!(
            body.contains(&paired),
            "the census loop must admit from the walk's own per-character \
             hand-off; expected\n{paired}"
        );
    }
    assert!(
        body.find("domain47_shadow_walk_character(").unwrap()
            < body.find("domain47_admit_country_leader_roles(").unwrap(),
        "the walk must run before the admission it feeds"
    );
    // AND BOTH ADMISSIONS PRECEDE THE CHARACTER DEDUP (Codex 019fe238,
    // finding 2). All three of `dedup.admit_character`'s arms `continue` on the
    // bodies the census does not keep — a merged duplicate and a quarantined
    // identity — and a role object hangs off ONE body, which may be exactly one
    // of those. The sweep admitted such an object; a walk that ran after the
    // dedup would silently drop its row. Ordering, not presence, is what makes
    // the population equal.
    let dedup_at = body
        .find("match dedup.admit_character(")
        .expect("the census loop's character dedup");
    for call in [
        "domain47_admit_country_leader_roles(",
        "domain47_admit_advisor_roots(",
    ] {
        assert!(
            body.find(call).unwrap() < dedup_at,
            "{call} must run BEFORE dedup.admit_character, whose every arm \
             `continue`s past the bodies a role may still hang off"
        );
    }
    // The sweep's own enumeration may appear ONLY inside the shadow block.
    let shadow = source
        .find("// THE SWEEP SHADOW (lane FU) - observe-only, OFF by default.")
        .expect("the shadow block");
    assert!(
        shadow > loop_tail,
        "the shadow runs after the census loop, not inside it"
    );
    assert!(
        !source[..shadow].contains("country_leader_hits"),
        "no country-leader sweep-hit consumption may survive outside the shadow"
    );
    assert!(
        !source[..shadow].contains("advisor_hits"),
        "no advisor sweep-hit consumption may survive outside the shadow"
    );
}

// === WHAT THE CONVERSION COSTS, AND SAVES (lane FU) ========================
//
// The A/B above proves the two enumerations admit the same population. This
// measures what swapping them costs, on a world at the live per-prewarm widths
// this file already records:
//
//  * 13_403 censused characters (`T32_CENSUS_CHARACTERS`);
//  * 1_964 `CCountryLeader` objects over 1_879 characters (`EJ_*`);
//  * 6_407 `CAdvisor` objects, one per character over the first 6_407;
//  * plus the DEAD population the sweep additionally enumerates and the walk
//    never touches: D04 observed ~1_005 dead-pointer `CAdvisor` objects and
//    t32 published `domain47_prewarm_excluded_scan_hits` at 1_393 per prewarm
//    across all families.
//
// THE TWO ARMS, and the difference between them is the whole change:
//
//  * SWEEP -- the admission loops run once, over a hit list in ASCENDING
//    ADDRESS order, after the census loop has finished. Each identity read
//    chases the role object's back-pointer into a `CCharacter` body chosen by
//    the heap's layout rather than by anything this loop just touched, and the
//    dead objects are probed exactly as hard as the live ones before being
//    excluded.
//  * WALK -- the admission runs inside the census loop, on the pointers the
//    character's own maps just yielded, so the body the back-pointer chases
//    into is the one this iteration prefetched and read; and the dead objects
//    are never enumerated at all.
//
// AN INSTRUMENT, NOT A GATE, like every other cost arm in this file: it prints
// the delta and asserts only that both arms did their job. A wall-time
// assertion on a machine that is usually also building something is a flaky
// gate rather than a measurement.

/// Dead `CAdvisor` objects the heap carries beside the live ones (D04).
const FU_DEAD_ADVISORS: usize = 1_005;
/// Dead / stale `CCountryLeader` objects, sized from t32's per-prewarm
/// `excluded_scan_hits` (1_393 across all families) net of the advisor share.
const FU_DEAD_LEADERS: usize = 388;

/// A census-width world whose role maps hold REAL role objects.
///
/// `ej_shadow_world` gives its map entries dummy in-window values, which is all
/// the walk itself needs; the admission functions dereference them, so this
/// fixture writes a whole `CCountryLeader` / `CAdvisor` behind every entry, on
/// its own page, in a shuffled order — the same one-object-per-page discipline
/// `scattered_census_world` uses and for the same reason.
struct FuCostWorld {
    _arena: BenchArena,
    roots: ResolvedTypeRoots,
    census_characters: Vec<usize>,
    /// Ascending by address: what the sweep publishes, live and dead together.
    sweep_leader_hits: Vec<usize>,
    sweep_advisor_hits: Vec<usize>,
}

fn fu_cost_world() -> FuCostWorld {
    let count = T32_CENSUS_CHARACTERS;
    let leader_objects = EJ_LEADER_ENTRIES + FU_DEAD_LEADERS;
    let advisor_objects = EJ_ADVISOR_ENTRIES + FU_DEAD_ADVISORS;
    let role_objects = leader_objects + advisor_objects;
    let node_slots = EJ_LEADER_ENTRIES + EJ_ADVISOR_ENTRIES + EJ_LEADER_CHARACTERS
        + EJ_ADVISOR_ENTRIES;
    let arena = BenchArena::in_heap_window(
        (count + role_objects) * SCATTER_PAGE + node_slots * EJ_NODE_STRIDE + 4 * SCATTER_PAGE,
    );
    let role_base = arena.base + count * SCATTER_PAGE;
    let node_base = role_base + role_objects * SCATTER_PAGE + SCATTER_PAGE;
    let decoy = node_base + node_slots * EJ_NODE_STRIDE;
    arena.put_u64(decoy, BENCH_CUNIT_LEADER_TRAIT_VTABLE);

    let shuffled = |seed: u64, len: usize| {
        let mut order: Vec<usize> = (0..len).collect();
        let mut state = seed;
        for idx in (1..len).rev() {
            let swap = (scatter_rng(&mut state) % (idx as u64 + 1)) as usize;
            order.swap(idx, swap);
        }
        order
    };
    let body_pages = shuffled(0x5eed_d47f_0001_u64, count);
    let role_pages = shuffled(0x5eed_d47f_0002_u64, role_objects);
    let node_order = shuffled(0x5eed_d47f_0003_u64, node_slots);

    let roots = ResolvedTypeRoots {
        ccharacter_vtable: BENCH_CCHARACTER_VTABLE,
        carmy_leader_vtable: BENCH_CARMY_LEADER_VTABLE,
        cnavy_leader_vtable: BENCH_CNAVY_LEADER_VTABLE,
        ccountry_leader_vtable: BENCH_CCOUNTRY_LEADER_VTABLE,
        cadvisor_vtable: BENCH_CADVISOR_VTABLE,
        cunit_leader_trait_vtable: BENCH_CUNIT_LEADER_TRAIT_VTABLE,
    };

    let character_at = |index: usize| arena.base + body_pages[index] * SCATTER_PAGE;
    for index in 0..count {
        let addr = character_at(index);
        arena.put_u64(addr, BENCH_CCHARACTER_VTABLE);
        arena.put_u32(addr + CCHARACTER_TYPE_OFF as usize, CCHARACTER_TYPE_ID);
        arena.put_u32(addr + CCHARACTER_ID_OFF as usize, index as u32);
        arena.put_u32(
            addr + CCHARACTER_COUNTRY_TAG_DB_ID_OFF as usize,
            1 + (index % 200) as u32,
        );
        arena.put_u64(addr + CCHARACTER_UNIT_LEADER_PTR_OFF as usize, 0);
    }

    let mut taken_role = 0usize;
    let mut role_slot = || {
        let at = role_base + role_pages[taken_role] * SCATTER_PAGE;
        taken_role += 1;
        at
    };
    let mut sweep_leader_hits = Vec::with_capacity(leader_objects);
    let mut sweep_advisor_hits = Vec::with_capacity(advisor_objects);
    let mut leader_entries: Vec<Vec<u64>> = vec![Vec::new(); count];
    let mut advisor_entries: Vec<Vec<u64>> = vec![Vec::new(); count];

    for index in 0..EJ_LEADER_ENTRIES {
        // The surplus over `EJ_LEADER_CHARACTERS` models D04's several-roles-
        // per-character, exactly as `ej_shadow_world` spreads it.
        let holder = index % EJ_LEADER_CHARACTERS;
        let addr = role_slot();
        arena.put_u64(addr, BENCH_CCOUNTRY_LEADER_VTABLE);
        arena.put_u64(
            addr + CCOUNTRY_LEADER_CHARACTER_PTR_OFF as usize,
            character_at(holder) as u64,
        );
        sweep_leader_hits.push(addr);
        leader_entries[holder].push(addr as u64);
    }
    for _ in 0..FU_DEAD_LEADERS {
        let addr = role_slot();
        arena.put_u64(addr, BENCH_CCOUNTRY_LEADER_VTABLE);
        arena.put_u64(addr + CCOUNTRY_LEADER_CHARACTER_PTR_OFF as usize, decoy as u64);
        sweep_leader_hits.push(addr);
    }
    for index in 0..EJ_ADVISOR_ENTRIES {
        let addr = role_slot();
        arena.put_u64(addr, BENCH_CADVISOR_VTABLE);
        arena.put_u64(
            addr + CADVISOR_CHARACTER_PTR_OFF as usize,
            character_at(index) as u64,
        );
        arena.put_msvc_string(
            addr + domain47::extractor::CADVISOR_IDEA_TOKEN_OFF as usize,
            addr + 0x400,
            &format!("advisor_idea_token_{index}"),
        );
        sweep_advisor_hits.push(addr);
        advisor_entries[index].push(addr as u64);
    }
    for _ in 0..FU_DEAD_ADVISORS {
        // The oracle's dead-pointer shape: a `CAdvisor` whose character chain
        // does not resolve. It costs the sweep a full probe pair before it is
        // excluded, and the walk never sees it.
        let addr = role_slot();
        arena.put_u64(addr, BENCH_CADVISOR_VTABLE);
        arena.put_u64(addr + CADVISOR_CHARACTER_PTR_OFF as usize, decoy as u64);
        sweep_advisor_hits.push(addr);
    }

    let mut taken_node = 0usize;
    let mut node_slot = || {
        let at = node_base + node_order[taken_node] * EJ_NODE_STRIDE;
        taken_node += 1;
        at
    };
    for index in 0..count {
        let addr = character_at(index);
        for (head_off, size_off, value_off, entries) in [
            (
                DOMAIN47_LEADER_MAP_HEAD_OFF,
                DOMAIN47_LEADER_MAP_SIZE_OFF,
                DOMAIN47_LEADER_NODE_VALUE_OFF,
                std::mem::take(&mut leader_entries[index]),
            ),
            (
                DOMAIN47_ADVISOR_MAP_HEAD_OFF,
                DOMAIN47_ADVISOR_MAP_SIZE_OFF,
                DOMAIN47_ADVISOR_NODE_VALUE_OFF,
                std::mem::take(&mut advisor_entries[index]),
            ),
        ] {
            arena.put_u64(addr + size_off, entries.len() as u64);
            if entries.is_empty() {
                arena.put_u64(addr + head_off, (addr + 0x800) as u64);
                continue;
            }
            let head = node_slot();
            let nodes: Vec<usize> = (0..entries.len()).map(|_| node_slot()).collect();
            ej_put_tree(&arena, head, &|slot| nodes[slot], value_off, &entries);
            arena.put_u64(addr + head_off, head as u64);
        }
    }

    sweep_leader_hits.sort_unstable();
    sweep_advisor_hits.sort_unstable();
    let census_characters: Vec<usize> = (0..count).map(character_at).collect();
    FuCostWorld {
        _arena: arena,
        roots,
        census_characters,
        sweep_leader_hits,
        sweep_advisor_hits,
    }
}

/// The census loop, with role discovery and admission done the WALK way.
///
/// Returns the two root counts SEPARATELY, and refuses nothing (Codex
/// 019fe1fa, finding 5). A single total would let a lost leader root cancel
/// against a gained advisor root, or against a refusal counted into the same
/// sum -- which is precisely the population difference the equality test below
/// exists to detect.
fn fu_cost_walk_arm(world: &FuCostWorld) -> (usize, usize) {
    let mem = Domain47InProcessMemory;
    let mut walk = Domain47ShadowWalk::default();
    let mut stack = Vec::new();
    let mut scratch = Vec::new();
    let mut seen = BTreeSet::new();
    let mut by_identity = BTreeMap::new();
    let mut leader_roots = Vec::new();
    let mut advisor_roots = Vec::new();
    let mut skipped = 0u32;
    let mut excluded = 0u32;
    for (ordinal, addr) in world.census_characters.iter().copied().enumerate() {
        if let Some(ahead) = world
            .census_characters
            .get(ordinal.wrapping_add(DOMAIN47_PREFETCH_DISTANCE))
        {
            domain47_prefetch_object(*ahead, domain47::extractor::CCHARACTER_RAW_FIELDS_SPAN);
        }
        domain47_shadow_walk_character(addr, &mut walk, &mut stack, &mut scratch);
        domain47_admit_country_leader_roles(
            &mem,
            &world.roots,
            &walk.fresh_leaders,
            None,
            &mut seen,
            &mut leader_roots,
            &mut skipped,
            &mut excluded,
        );
        domain47_admit_advisor_roots(
            &mem,
            &world.roots,
            &walk.fresh_advisors,
            None,
            &mut by_identity,
            &mut advisor_roots,
            &mut skipped,
            &mut excluded,
        );
    }
    assert_eq!(walk.refused, 0, "the cost fixture must refuse no map");
    (leader_roots.len(), advisor_roots.len())
}

/// The census loop, with the sweep's hit lists admitted after it. Returns the
/// two root counts separately, for the reason  gives.
///
/// `shadow_walk` is what separates the two things this arm has to be:
///
///  * `true` — THE SHIPPED BASELINE at this lane's base commit. Lane EJ's walk
///    is production-ON there (`domain47_shadow_walk` defaults true), so every
///    capture up to t43 paid for the container walk AND the sweep's hit loops.
///    Against `fu_cost_walk_arm` this is the delta the conversion actually
///    produces.
///  * `false` — the counterfactual: what d47 would cost if the walk had never
///    been added. Against `fu_cost_walk_arm` this is what the container walk
///    costs as a MECHANISM, which is a different and more pessimistic question
///    and is reported separately rather than quietly conflated with the first.
fn fu_cost_sweep_arm(world: &FuCostWorld, shadow_walk: bool) -> (usize, usize) {
    let mem = Domain47InProcessMemory;
    let mut walk = Domain47ShadowWalk::default();
    let mut stack = Vec::new();
    let mut scratch = Vec::new();
    let mut seen = BTreeSet::new();
    let mut by_identity = BTreeMap::new();
    let mut leader_roots = Vec::new();
    let mut advisor_roots = Vec::new();
    let mut skipped = 0u32;
    let mut excluded = 0u32;
    for (ordinal, addr) in world.census_characters.iter().copied().enumerate() {
        if let Some(ahead) = world
            .census_characters
            .get(ordinal.wrapping_add(DOMAIN47_PREFETCH_DISTANCE))
        {
            domain47_prefetch_object(*ahead, domain47::extractor::CCHARACTER_RAW_FIELDS_SPAN);
        }
        // The census pass still touches every body; only the role discovery
        // differs between the arms.
        domain47_prefetch_object(addr, domain47::extractor::CCHARACTER_RAW_FIELDS_SPAN);
        if shadow_walk {
            domain47_shadow_walk_character(addr, &mut walk, &mut stack, &mut scratch);
        }
    }
    domain47_admit_country_leader_roles(
        &mem,
        &world.roots,
        &world.sweep_leader_hits,
        None,
        &mut seen,
        &mut leader_roots,
        &mut skipped,
        &mut excluded,
    );
    domain47_admit_advisor_roots(
        &mem,
        &world.roots,
        &world.sweep_advisor_hits,
        None,
        &mut by_identity,
        &mut advisor_roots,
        &mut skipped,
        &mut excluded,
    );
    assert_eq!(
        walk.refused, 0,
        "the shadow arm's instrument must refuse no map either"
    );
    (leader_roots.len(), advisor_roots.len())
}

/// BOTH ARMS ADMIT THE SAME ROWS AT LIVE WIDTH, which is what makes the timing
/// below a comparison of two ways to do one job rather than of two jobs.
///
/// This is the A/B above re-run at census scale, with no stale objects staged:
/// the only difference between the arms is the dead population the sweep
/// additionally enumerates, and it must cost rows on neither side.
#[test]
fn d47_walk_and_sweep_admit_the_same_rows_at_census_width() {
    let world = fu_cost_world();
    page_cache_new_frame();
    let walk = fu_cost_walk_arm(&world);
    page_cache_new_frame();
    let sweep = fu_cost_sweep_arm(&world, false);
    // FAMILY BY FAMILY, not on a total: a lost leader root and a gained advisor
    // root sum to the same number, and so does a lost root beside a refusal.
    assert_eq!(
        walk.0, sweep.0,
        "the two discovery paths must publish the same COUNTRY-LEADER roots"
    );
    assert_eq!(
        walk.1, sweep.1,
        "and the same ADVISOR roots"
    );
    assert_eq!(
        sweep,
        (EJ_LEADER_CHARACTERS, EJ_ADVISOR_ENTRIES),
        "and both must be the live width, not a truncated walk"
    );
}

/// WHAT THE CONVERSION COSTS, per prewarm, on the census-width world.
///
/// TWO COMPARISONS, because there are two honest questions and answering only
/// the flattering one would be the measurement equivalent of a vacuous test:
///
///  1. AGAINST THE SHIPPED BASELINE (walk + sweep loops), which is what every
///     capture through t43 actually ran. This is the delta the transition
///     produces.
///  2. AGAINST A SWEEP-ONLY d47 (no walk at all), which is what the container
///     walk costs as a mechanism. Expect this one to be a REGRESSION: the walk
///     traverses ~8_300 `_Tree` nodes it does not otherwise have to touch, and
///     that is more work than the back-chase locality and the ~1_400 skipped
///     dead-object probes give back.
#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_walk_vs_sweep_discovery() {
    let world = fu_cost_world();
    page_cache_new_frame();
    // The timed closures return a scalar sink, which is all a wall-clock arm
    // needs; the ROW equality the two arms must satisfy is asserted family by
    // family in the test above, not inferred from a total here.
    let walk_arm = || {
        let (leaders, advisors) = fu_cost_walk_arm(&world);
        leaders + advisors
    };
    let sweep_arm = |shadow: bool| {
        let (leaders, advisors) = fu_cost_sweep_arm(&world, shadow);
        leaders + advisors
    };
    assert!(walk_arm() > 0);
    assert!(sweep_arm(true) > 0);
    let (shipped, walk) = time_us_median_ab_n(
        7,
        3,
        || sweep_arm(true),
        walk_arm,
    );
    let (sweep_only, walk_again) = time_us_median_ab_n(
        7,
        3,
        || sweep_arm(false),
        walk_arm,
    );
    let pct = |before: f64, after: f64| (after - before) / before * 100.0;
    println!(
        "d47_cost role discovery over {} characters, {} live + {} dead leader objects, \
         {} live + {} dead advisor objects:\n  \
         vs SHIPPED (walk instrument + sweep loops): {shipped:.1} us -> {walk:.1} us \
         ({:+.1} us, {:+.1}%)\n  \
         vs SWEEP-ONLY (no walk at all):            {sweep_only:.1} us -> {walk_again:.1} us \
         ({:+.1} us, {:+.1}%)",
        T32_CENSUS_CHARACTERS,
        EJ_LEADER_ENTRIES,
        FU_DEAD_LEADERS,
        EJ_ADVISOR_ENTRIES,
        FU_DEAD_ADVISORS,
        walk - shipped,
        pct(shipped, walk),
        walk_again - sweep_only,
        pct(sweep_only, walk_again),
    );
}

// === WHAT A REFUSED CONTAINER COSTS (lane FU round 2, Codex 019fe1c9) ======
//
// While the walk was observe-only a refusal cost a counter. It is production
// discovery now, so a container the walk could not read is role rows this frame
// does not have -- and the frame must say so on the wire rather than publish the
// rest under a family it declares observed. These pin the three halves of that:
// which families a refusal costs, that the refusal is DETECTED in the two shapes
// the count check alone would miss, and that the prewarm acts on it.

/// A LEADER-map refusal costs the country-leader family and NOTHING else.
///
/// MUTATION: collapse `refused_leader` / `refused_advisor` back into the single
/// `refused` counter and make `unobserved_families` test it, and this fails --
/// one unreadable leader map would take the advisor roster with it, which is a
/// whole family of rows lost to a gap that never touched it.
#[test]
fn d47_a_refused_leader_map_costs_only_the_leader_family() {
    let world = EjSingleCharacterWorld::new(3, 2);
    // A concurrent erase leaves `_Mysize` one ahead of what is reachable.
    world.arena.put_u64(
        world.character + DOMAIN47_LEADER_MAP_SIZE_OFF,
        world.leaders.len() as u64 + 1,
    );
    let walk = world.walk();
    assert_eq!(walk.refused_leader, 1);
    assert_eq!(walk.refused_advisor, 0);
    assert_eq!(
        walk.unobserved_families(),
        domain47::FAMILY_COUNTRY_LEADER_ROLE
    );
    assert_eq!(
        walk.refused_first_map_axis(),
        Some(Domain47ShadowWalkMapAxis::Leader),
        "the locator must name the axis that refused"
    );
    assert_eq!(walk.refused_first_body, world.character as u64);
}

/// An ADVISOR-map refusal costs the roster AND the appointment.
///
/// The appointed-advisor family is declared by `domain47_compact` to be the
/// exact intersection of the roster with each country's active ideas, so a
/// roster with a hole cannot support an appointment claim: an advisor missing
/// from the roster reads as "not appointed", which is a wrong answer every
/// downstream check would pass.
#[test]
fn d47_a_refused_advisor_map_costs_the_roster_and_the_appointment() {
    let world = EjSingleCharacterWorld::new(2, 3);
    world.arena.put_u64(
        world.character + DOMAIN47_ADVISOR_MAP_SIZE_OFF,
        DOMAIN47_SHADOW_WALK_MAX_NODES + 1,
    );
    let walk = world.walk();
    assert_eq!(walk.refused_advisor, 1);
    assert_eq!(walk.refused_leader, 0);
    assert_eq!(
        walk.unobserved_families(),
        domain47::FAMILY_ADVISOR_ROSTER | domain47::FAMILY_APPOINTED_ADVISOR
    );
}

/// A clean walk costs no family, or every capture would declare these three
/// absent forever and the assertions above would be true for free.
#[test]
fn d47_a_clean_role_walk_costs_no_family() {
    let world = EjSingleCharacterWorld::new(3, 4);
    let walk = world.walk();
    assert_eq!(walk.refused, 0);
    assert_eq!(walk.unobserved_families(), 0);
    assert_eq!(walk.refused_first_map_axis(), None);
    assert_eq!(walk.refused_first_body, 0);
}

/// TWO NODES YIELDING ONE POINTER REFUSES THE MAP.
///
/// The shape a rebalance takes when it moves a subtree under a walk in
/// progress: one node is visited twice and another is never reached, and the
/// count check cannot see it because the total is preserved. Neither container
/// can legally hold one pointer twice -- the engine's teardown destroys
/// `node->value` once per node, so a duplicate would be a double free -- which
/// is what makes refusing safe.
///
/// MUTATION: delete the `staged.windows(2)` check and this fails. Before it
/// existed, the walk published two entries where the world has three and called
/// the map clean.
#[test]
fn d47_walk_refuses_a_map_that_yields_one_pointer_twice() {
    let world = EjSingleCharacterWorld::new(3, 2);
    // Node 1 now carries node 0's value: three nodes, three pops, two distinct
    // pointers. `visited == size` still holds.
    world.arena.put_u64(
        world.leader_nodes + EJ_NODE_STRIDE + DOMAIN47_LEADER_NODE_VALUE_OFF,
        world.leaders[0],
    );
    let walk = world.walk();
    assert_eq!(
        walk.refused_leader, 1,
        "a value reached twice inside ONE map is a walk that did not enumerate it"
    );
    assert!(walk.leaders.is_empty(), "and it contributes nothing");
    assert_eq!(
        walk.advisors.len(),
        2,
        "the other map on the same character is unaffected"
    );
}

/// THE SAME NODE REACHED TWICE REFUSES, EVEN WITH A NULL VALUE.
///
/// The duplicate check that matters is on the NODE, not on the value. A node
/// whose value is null contributes nothing to the value list at all, so a
/// rebalance that re-reaches a null node and misses a real one preserves
/// `visited == size`, leaves both headers untouched, produces no duplicate
/// VALUE — and would publish the map one role pointer short while calling it
/// clean. (Codex 019fe1fa, finding 2.)
///
/// MUTATION: delete the `visited_nodes` check and keep only the value check,
/// and this fails while `d47_walk_refuses_a_map_that_yields_one_pointer_twice`
/// still passes. That is the exact gap the value check alone left.
#[test]
fn d47_walk_refuses_a_map_that_reaches_one_null_node_twice() {
    let world = EjSingleCharacterWorld::new(3, 0);
    let node = |index: usize| world.leader_nodes + index * EJ_NODE_STRIDE;
    // BOTH of the root's children now point at node 2. Node 1 becomes
    // unreachable and node 2 is reached twice, so the walk still pops exactly
    // three nodes and `visited == size` holds -- the count check is blind to
    // it, which is what makes this the interesting topology.
    world
        .arena
        .put_u64(node(0) + DOMAIN47_TREE_NODE_LEFT_OFF, node(2) as u64);
    world
        .arena
        .put_u64(node(0) + DOMAIN47_TREE_NODE_RIGHT_OFF, node(2) as u64);
    // Node 2 carries NO object -- a map entry with no leader, which the walk
    // tolerates by design and which therefore contributes nothing to the value
    // list. Without the node check, the duplicate is invisible on every axis:
    // the count adds up, the headers never moved, and no value repeats.
    world
        .arena
        .put_u64(node(2) + DOMAIN47_LEADER_NODE_VALUE_OFF, 0);
    let walk = world.walk();
    assert_eq!(
        walk.refused_leader, 1,
        "a node reached twice is a walk that missed another node, whatever its \
         value was"
    );
    assert!(walk.leaders.is_empty());
    assert_eq!(
        walk.unobserved_families(),
        domain47::FAMILY_COUNTRY_LEADER_ROLE
    );
}

/// A LEGAL MAP WITH A NULL ENTRY IS STILL ACCEPTED.
///
/// The node check must not turn "this entry holds no object" into a refusal:
/// that is an observation the walk has always tolerated, and refusing it would
/// cost the family on a world nothing is wrong with.
#[test]
fn d47_walk_accepts_a_map_entry_with_no_object() {
    let world = EjSingleCharacterWorld::new(3, 0);
    world.arena.put_u64(
        world.leader_nodes + 2 * EJ_NODE_STRIDE + DOMAIN47_LEADER_NODE_VALUE_OFF,
        0,
    );
    let walk = world.walk();
    assert_eq!(walk.refused, 0, "a null entry is observed, not refused");
    assert_eq!(
        walk.fresh_leaders.len(),
        2,
        "and it contributes no pointer, while the other two do"
    );
    assert_eq!(walk.unobserved_families(), 0);
}

/// TWO null entries in one map are legal too, which is why the check is on the
/// node and could not have been on the value.
#[test]
fn d47_walk_accepts_two_map_entries_with_no_object() {
    let world = EjSingleCharacterWorld::new(3, 0);
    for index in [1usize, 2] {
        world.arena.put_u64(
            world.leader_nodes + index * EJ_NODE_STRIDE + DOMAIN47_LEADER_NODE_VALUE_OFF,
            0,
        );
    }
    let walk = world.walk();
    assert_eq!(
        walk.refused, 0,
        "two entries with no object are two observations, not a duplicate"
    );
    assert_eq!(walk.fresh_leaders.len(), 1);
}

/// A POINTER TWO DIFFERENT CHARACTERS HOLD IS STILL LEGAL.
///
/// The duplicate check above is PER MAP. Duplicate `CCharacter` bodies for one
/// logical character really do exist in this heap and really can hold the same
/// role object, and refusing that would cost a family on every prewarm.
#[test]
fn d47_walk_accepts_one_pointer_held_by_two_characters() {
    let first = EjSingleCharacterWorld::new(2, 0);
    let second = EjSingleCharacterWorld::new(2, 0);
    // Point the second character's first entry at the first character's object.
    second.arena.put_u64(
        second.leader_nodes + DOMAIN47_LEADER_NODE_VALUE_OFF,
        first.leaders[0],
    );
    let mut walk = Domain47ShadowWalk::default();
    let mut stack = Vec::new();
    let mut scratch = Vec::new();
    page_cache_new_frame();
    domain47_shadow_walk_character(first.character, &mut walk, &mut stack, &mut scratch);
    assert_eq!(walk.fresh_leaders.len(), 2);
    domain47_shadow_walk_character(second.character, &mut walk, &mut stack, &mut scratch);
    assert_eq!(walk.refused, 0, "two characters may hold one role object");
    assert_eq!(
        walk.fresh_leaders.len(),
        1,
        "and it is offered for admission ONCE: the shared pointer is already in \
         the distinct set"
    );
    assert_eq!(walk.leaders.len(), 3);
}

/// A MAP HEADER THAT MOVED UNDER THE WALK REFUSES BOTH AXES.
///
/// The count check sees an insert or erase; it cannot see a container that was
/// relocated, nor a body being written while the first half of the header was
/// read. The closing read is what does, and this is the only thing that
/// exercises it -- both header reads live inside one call, so the mutation has
/// to arrive through the test seam.
///
/// MUTATION: delete the closing read (make `closed` unconditionally true) and
/// this fails. Before it existed, a relocated container's stale enumeration was
/// adopted as complete.
#[test]
fn d47_walk_refuses_a_character_whose_map_header_moved_under_it() {
    let world = EjSingleCharacterWorld::new(3, 2);
    let mut walk = Domain47ShadowWalk::default();
    let mut stack = Vec::new();
    let mut scratch = Vec::new();
    page_cache_new_frame();
    domain47_shadow_walk_character_with_between(
        world.character,
        &mut walk,
        &mut stack,
        &mut scratch,
        || {
            // The engine reallocates the leader map: `_Myhead` now names a
            // different sentinel. Both axes were walked cleanly before this.
            world.arena.put_u64(
                world.character + DOMAIN47_LEADER_MAP_HEAD_OFF,
                (world.arena.base + 0x3000) as u64,
            );
        },
    );
    assert_eq!(
        walk.refused_leader, 1,
        "the container whose header moved was not enumerated"
    );
    assert_eq!(
        walk.refused_advisor, 0,
        "and the OTHER container keeps its family: its own header did not move \
         and its walk agreed with it, so refusing it would declare an advisor \
         roster unobserved over a container this frame read perfectly"
    );
    assert!(walk.leaders.is_empty());
    assert!(walk.fresh_leaders.is_empty());
    assert_eq!(walk.fresh_advisors.len(), 2);
    assert_eq!(
        walk.unobserved_families(),
        domain47::FAMILY_COUNTRY_LEADER_ROLE
    );
}

/// A UNIT-LEADER ASSIGNMENT UNDER THE WALK IS NOT A ROLE-CONTAINER TEAR.
///
/// `CCHARACTER_UNIT_LEADER_PTR_OFF` is `+0xA8` — inside the 40-byte header
/// snapshot, between the leader map's `_Mysize` at `+0xA0` and the advisor
/// map's `_Myhead` at `+0xB0`. A byte-for-byte closing comparison reads a
/// character being given or losing a unit leader (an ordinary, frequent event)
/// as both role containers having moved, and costs the frame three families
/// while both maps sat perfectly still.
///
/// MUTATION: compare `closing.buf == headers.buf` instead of the four header
/// words and this fails. That is what the first version of the bracket did.
/// (Codex 019fe1e5, finding 1.)
#[test]
fn d47_walk_accepts_a_unit_leader_assigned_under_it() {
    let world = EjSingleCharacterWorld::new(3, 2);
    let mut walk = Domain47ShadowWalk::default();
    let mut stack = Vec::new();
    let mut scratch = Vec::new();
    page_cache_new_frame();
    domain47_shadow_walk_character_with_between(
        world.character,
        &mut walk,
        &mut stack,
        &mut scratch,
        || {
            // The engine appoints a unit leader. Neither `_Myhead` nor
            // `_Mysize` of either map is touched.
            world.arena.put_u64(
                world.character + CCHARACTER_UNIT_LEADER_PTR_OFF as usize,
                (world.arena.base + 0x5000) as u64,
            );
        },
    );
    assert_eq!(
        walk.refused, 0,
        "a field between the two map headers is not a map header"
    );
    assert_eq!(walk.fresh_leaders.len(), 3);
    assert_eq!(walk.fresh_advisors.len(), 2);
    assert_eq!(walk.unobserved_families(), 0);
}

/// And the ADVISOR map's own header still closes the bracket, so the narrowing
/// did not simply stop looking at the second half of the snapshot.
#[test]
fn d47_walk_refuses_a_character_whose_advisor_header_moved_under_it() {
    let world = EjSingleCharacterWorld::new(3, 2);
    let mut walk = Domain47ShadowWalk::default();
    let mut stack = Vec::new();
    let mut scratch = Vec::new();
    page_cache_new_frame();
    domain47_shadow_walk_character_with_between(
        world.character,
        &mut walk,
        &mut stack,
        &mut scratch,
        || {
            world
                .arena
                .put_u64(world.character + DOMAIN47_ADVISOR_MAP_SIZE_OFF, 9);
        },
    );
    assert_eq!(
        walk.refused_advisor, 1,
        "the advisor half of the closing read is live, so narrowing the \
         comparison to two words did not simply stop looking at it"
    );
    assert_eq!(walk.refused_leader, 0);
    assert!(walk.advisors.is_empty());
    assert_eq!(walk.fresh_leaders.len(), 3);
    assert_eq!(
        walk.unobserved_families(),
        domain47::FAMILY_ADVISOR_ROSTER | domain47::FAMILY_APPOINTED_ADVISOR
    );
}

/// A closing read that FAILED ENTIRELY refuses both axes, which the per-axis
/// narrowing must not have weakened.
///
/// The narrowing is about what a SUCCESSFUL closing read is evidence for; a
/// body that cannot be re-read is evidence for neither container. That case is
/// already pinned by `d47_shadow_walk_refuses_both_maps_when_the_body_is_unreadable`
/// — the opening read fails there, and both reads go through
/// `domain47_shadow_walk_headers`, so one unreadable body exercises the same
/// fail-closed answer. This note exists so a reader looking for the arm finds
/// where it is covered rather than assuming it is not.

/// The same seam with NO mutation: the bracket must not refuse a quiet world.
#[test]
fn d47_walk_accepts_a_character_whose_header_did_not_move() {
    let world = EjSingleCharacterWorld::new(3, 2);
    let mut walk = Domain47ShadowWalk::default();
    let mut stack = Vec::new();
    let mut scratch = Vec::new();
    page_cache_new_frame();
    domain47_shadow_walk_character_with_between(
        world.character,
        &mut walk,
        &mut stack,
        &mut scratch,
        || {},
    );
    assert_eq!(walk.refused, 0);
    assert_eq!(walk.fresh_leaders.len(), 3);
    assert_eq!(walk.fresh_advisors.len(), 2);
}

/// THE PREWARM ACTS ON A REFUSAL: it clears the family bit AND the rows.
///
/// Source text, because the branch runs only against live process memory. The
/// pairing is the property -- a declared-absent family carrying rows is the
/// contradiction `domain47_compact`'s "did not observe" parse error exists to
/// catch, and a cleared row list under a family still declared present is the
/// silent deletion the whole rule exists to prevent.
#[test]
fn d47_the_prewarm_drops_the_rows_of_a_family_the_walk_refused() {
    let source = super::tests::collector_source_lf();
    let at = source
        .find("let role_walk_unobserved = shadow_walk.unobserved_families();")
        .expect("the prewarm consumes the walk's refusal mask");
    let block = &source[at..at + 900];
    // THE PAIRINGS, not the presence of the strings (Codex 019fe1e5, finding
    // 4). Both `clear()` calls and the mask update appearing near each other
    // is satisfied by a version that clears the ADVISOR rows on a LEADER
    // refusal — which publishes a leader family declared present with its rows
    // gone (a false deletion) beside an advisor family declared absent while
    // carrying rows (the decoder's "did not observe" parse error). Each
    // condition is therefore matched together with the statement it guards.
    for (condition, statement) in [
        (
            "if role_walk_unobserved & domain47::FAMILY_COUNTRY_LEADER_ROLE != 0 {",
            "next.country_leader_roles.clear();",
        ),
        (
            "if role_walk_unobserved & domain47::FAMILY_ADVISOR_ROSTER != 0 {",
            "next.advisors.clear();",
        ),
    ] {
        let paired = format!("{condition}\n        {statement}");
        assert!(
            block.contains(&paired),
            "the prewarm must clear exactly the rows of the family it declares \
             unobserved; expected\n{paired}"
        );
    }
    for required in [
        "next.known_families &= !role_walk_unobserved;",
        "Domain47FamilyGapReason::RoleContainerWalkRefused",
    ] {
        assert!(
            block.contains(required),
            "the prewarm must {required} when the walk refused a container"
        );
    }
}

/// THE SWEEP SHADOW'S COUNTERS ARE ITS OWN, at the production call site.
///
/// The unit test below proves the sweep's hit list really does drive
/// `skipped_root_rows` and `excluded_scan_hits`; this proves the shadow block
/// does not hand it PRODUCTION's. `skipped_root_rows != 0` feeds
/// `domain47_prewarm_adoption_blocked`, so a torn object only the sweep
/// enumerated could otherwise block an adoption the walk had every right to
/// make -- and rowless-PARTIAL the whole domain on the strength of a
/// measurement. (Codex 019fe1c9, finding 4.)
#[test]
fn d47_the_sweep_shadow_block_passes_its_own_counters() {
    let source = super::tests::collector_source_lf();
    let at = source
        .find("// THE SWEEP SHADOW (lane FU) - observe-only, OFF by default.")
        .expect("the shadow block");
    let end = source[at..]
        .find("\n    // Wave-3 870ebf unblock: CUnitLeaderTrait")
        .expect("the end of the shadow block")
        + at;
    let block = &source[at..end];
    assert!(
        block.contains("let mut shadow_skipped = 0u32;")
            && block.contains("let mut shadow_excluded = 0u32;"),
        "the shadow must own its refusal counters"
    );
    // THE NAMES DO NOT APPEAR IN THE CODE AT ALL, which is stronger than
    // excluding the two `&mut` spellings (Codex 019fe1e5, finding 4): `let
    // alias = &mut skipped_root_rows;`, `skipped_root_rows += shadow_skipped;`
    // and every other route into production's counters is a USE of the
    // identifier, and a block that never names them cannot reach them.
    //
    // Comment lines are stripped first, because the block's own prose explains
    // WHY it must not touch those two counters and naming them there is the
    // point.
    let code: String = block
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    for forbidden in ["skipped_root_rows", "excluded_scan_hits"] {
        assert!(
            !code.contains(forbidden),
            "the shadow block's CODE must not mention production's {forbidden}: \
             it blocks adoption, and an observe-only measurement may not"
        );
    }
    assert!(
        !block.contains("next.country_leader_roles") && !block.contains("next.advisors"),
        "and it must not be handed production's root lists either"
    );
    // What the shadow's own counters DO feed: the two population words and the
    // torn-hit word, which are diagnostics. Pinned positively, because "goes
    // nowhere" was the previous claim and it is no longer true.
    assert!(
        block.contains("shadow_sweep_torn_hits = shadow_skipped;"),
        "the shadow's torn-read count must reach the wire, or a zero residual \
         taken off a prewarm that lost hits reads as a clear"
    );
}

/// THE STALE ROOT THE SWEEP KEEPS IS THE ONE THAT STOPS RESOLVING.
///
/// The population A/B shows WHICH ADDRESS wins the sweep's per-identity dedup.
/// That alone is not the row loss lane FT described (Codex 019fe1c9, finding 3):
/// under static staged memory the stale object still reads cleanly, so the frame
/// walk would happily build a row from it. What makes it a LOSS is that the
/// object is stale -- the engine has finished with it -- so by the time the
/// frame walk re-reads the root, the address no longer describes the role.
///
/// This stages exactly that, in two steps a static fixture can perform:
///
///   1. admit both ways, and confirm the sweep rooted the row at the stale
///      object while the walk rooted it at the map-held one;
///   2. RECYCLE the stale object -- the allocator hands its storage to
///      something else -- and re-read BOTH roots through the same identity
///      reader the frame walk's own root check uses.
///
/// The sweep's root stops resolving; the walk's does not. The sweep's row is
/// gone, and the map-held object that would have carried it was discarded by
/// the dedup, so nothing recovers it.
#[test]
fn d47_a_recycled_stale_root_loses_the_row_the_sweep_published() {
    let world = fu_acceptance_world();
    let mem = Domain47InProcessMemory;
    let walk = fu_admit_by_walk(&world);
    let sweep = fu_admit_by_sweep(&world);

    let stale = *world
        .leader_classes
        .iter()
        .find(|(_, class)| **class == FuRoleClass::StaleDuplicate)
        .expect("the fixture stages a stale duplicate")
        .0;
    let sweep_root = sweep
        .leader_roots
        .iter()
        .find(|root| root.char_id == fu_character_id(0))
        .expect("the sweep publishes character 0's role")
        .country_leader_addr;
    let walk_root = walk
        .leader_roots
        .iter()
        .find(|root| root.char_id == fu_character_id(0))
        .expect("the walk publishes character 0's role")
        .country_leader_addr;
    assert_eq!(sweep_root, stale as u64);
    assert_ne!(walk_root, stale as u64);
    assert!(
        !walk.leader_objects.contains(&(stale as u64)),
        "the stale object is in NO character's map, which is what makes it stale"
    );

    // Both roots resolve while the world is quiet: the loss is not staged yet.
    page_cache_new_frame();
    assert!(domain47_country_leader_identity(&mem, &world.roots, sweep_root).is_ok());
    assert!(domain47_country_leader_identity(&mem, &world.roots, walk_root).is_ok());

    // The allocator reuses the finished object's storage.
    world._arena.put_u64(stale, BENCH_CUNIT_LEADER_TRAIT_VTABLE);
    world
        ._arena
        .put_u64(stale + CCOUNTRY_LEADER_CHARACTER_PTR_OFF as usize, 0);
    page_cache_new_frame();
    assert!(
        domain47_country_leader_identity(&mem, &world.roots, sweep_root).is_err(),
        "the sweep's root no longer describes a country-leader role, and the \
         map-held object it beat in the dedup was discarded"
    );
    assert!(
        domain47_country_leader_identity(&mem, &world.roots, walk_root).is_ok(),
        "the walk's root is the object a character map holds, and it is unaffected"
    );
}
