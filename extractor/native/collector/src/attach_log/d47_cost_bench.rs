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

const T15_MANAGER_HISTORICAL: usize = 10_116;
const T15_MANAGER_DYNAMIC: usize = 3_289;
const T15_CENSUS_CHARACTERS: usize = T15_MANAGER_HISTORICAL + T15_MANAGER_DYNAMIC;
const T15_FRAME_CHARACTERS: usize = 5_227;
const T15_FRAME_ADVISORS: usize = 3_358;
const T15_FRAME_UNIT_LEADERS: usize = 534;
const T15_FRAME_COUNTRY_LEADER_ROLES: usize = 733;
const T15_FRAME_TRAIT_TOKENS: usize = 76;
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
    census_characters: Vec<usize>,
    character_roots: Vec<CharacterRoot>,
    unit_leader_roots: Vec<UnitLeaderRoot>,
    trait_roots: Vec<TraitDefinitionRoot>,
    country_leader_roots: Vec<CountryLeaderRoleRoot>,
    advisor_roots: Vec<AdvisorRoot>,
}

fn bench_world() -> BenchWorld {
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
        arena.put_u64(
            addr + CUNIT_LEADER_SKILL_SOURCE_PTR_OFF as usize,
            skill as u64,
        );
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
        let leader_ptr = if idx < T15_FRAME_UNIT_LEADERS {
            (unit_leader_base + idx * UNIT_LEADER_STRIDE) as u64
        } else {
            0
        };
        arena.put_u64(addr + CCHARACTER_UNIT_LEADER_PTR_OFF as usize, leader_ptr);
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

struct LegacyDomain47Memory;

impl domain47::extractor::LaneMemory for LegacyDomain47Memory {
    fn read_u8(&self, addr: u64) -> Option<u8> {
        Domain47InProcessMemory.read_u8(addr)
    }

    fn read_array<const N: usize>(&self, addr: u64) -> Option<[u8; N]> {
        Domain47InProcessMemory.read_array::<N>(addr)
    }
}

fn census_loop_legacy(world: &BenchWorld) -> usize {
    let mem = LegacyDomain47Memory;
    let mut sink = 0usize;
    for addr in &world.census_characters {
        let name_token_addr = *addr + DOMAIN47_CCHARACTER_NAME_TOKEN_ID_OFF;
        let opening = read_u32_stable(name_token_addr);
        let first =
            domain47::extractor::read_character_raw_fields(&mem, &world.roots, *addr as u64);
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

fn census_loop_separate_token_bracket(world: &BenchWorld) -> usize {
    let mem = Domain47InProcessMemory;
    let mut sink = 0usize;
    for addr in &world.census_characters {
        let name_token_addr = *addr + DOMAIN47_CCHARACTER_NAME_TOKEN_ID_OFF;
        let opening = read_u32_stable(name_token_addr);
        let first =
            domain47::extractor::read_character_raw_fields(&mem, &world.roots, *addr as u64);
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
    domain47::adapter::build_lane_frame_from_adapter_input_ref(
        mem,
        &world.roots,
        &bench_input(world),
    )
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
const BENCH_ITERATIONS: usize = 200;

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
    page_cache_new_frame();
    let mut sink = body();
    let started = Instant::now();
    for _ in 0..iterations {
        page_cache_new_frame();
        sink += body();
    }
    let elapsed = started.elapsed();
    (elapsed.as_nanos() as f64 / iterations as f64 / 1000.0, sink)
}

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

#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_read_path_ab() {
    let world = bench_world();
    let legacy_mem = LegacyDomain47Memory;
    let snapshot_mem = Domain47InProcessMemory;

    let (census_legacy, census_now) =
        time_us_median_ab(5, || census_loop_legacy(&world), || census_loop(&world));
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

const GUARDED_READ_CALLS_PER_PASS: usize = T15_CENSUS_CHARACTERS;

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

const CHARACTER_BODY_WINDOW: usize = 0xB0;

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
            arena.put_msvc_string(
                entry + domain47::extractor::CCHARACTER_PORTRAIT_ENTRY_GFX_OFF,
                0,
                "GFX_p",
            );
        }
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

/// run in `--release`. OBSERVED FIRST TIME (before this assertion existed):
#[test]
fn d47_census_merged_and_separate_token_reads_observe_the_same_character() {
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

#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_read_bracket_observation_vs_compute() {
    let world = bench_world();
    let mem = Domain47InProcessMemory;

    let (walk_us, walk_sink) = time_us_median(5, || frame_walk_with(&world, &mem));

    let frame = build_bench_frame(&world, &mem).frame;
    let (encode_us, _encode_sink) = time_us_median(5, || {
        domain47::extractor::encode_frame_payload(&frame).len()
    });
    let payload = domain47::extractor::encode_frame_payload(&frame);
    let (decode_us, decode_sink) = time_us_median(5, || {
        domain47::domain47_characters_leaders_cabinet_live::decode_payload(0, &payload)
            .row_count_total as usize
    });

    assert!(decode_sink > 0, "phase 3 decoded no rows");

    let total = walk_us + encode_us + decode_us;
    println!(
        "d47_cost read_bracket: observe(walk) {walk_us:.1} us/invocation | \
         compute(encode) {encode_us:.1} us/invocation | compute(decode gate) \
         {decode_us:.1} us/invocation | total {total:.1} us/invocation -- compute is \
         {:.1}% of the bracket on this fixture ({} payload bytes for ONE frame; \
         sinks are totals over all timed calls, warm-ups included, and are \
         liveness checks rather than per-frame quantities: {decode_sink} rows \
         decoded, walk sink={walk_sink})",
        (encode_us + decode_us) / total * 100.0,
        payload.len(),
    );
}

#[test]
fn d47_snapshot_and_field_by_field_reads_build_the_same_frame() {
    let world = bench_world();
    page_cache_new_frame();
    let legacy = build_bench_frame(&world, &LegacyDomain47Memory);
    page_cache_new_frame();
    let snapshot = build_bench_frame(&world, &Domain47InProcessMemory);
    let without_timings = |mut stats: domain47::adapter::AdapterBuildStats| {
        stats.walk_characters_us = 0;
        stats.walk_unit_leaders_us = 0;
        stats.walk_country_leader_roles_us = 0;
        stats.walk_active_rulers_us = 0;
        stats.walk_advisors_us = 0;
        stats.walk_appointed_advisors_us = 0;
        stats.walk_portraits_us = 0;
        stats
    };
    assert_eq!(
        without_timings(legacy.stats.clone()),
        without_timings(snapshot.stats.clone()),
        "one-call object reads changed the adapter build stats"
    );
    assert_eq!(
        legacy.frame, snapshot.frame,
        "one-call object reads changed the emitted frame"
    );
    assert_eq!(legacy.stats.character_rows as usize, T15_FRAME_CHARACTERS);
    assert_eq!(
        legacy.stats.advisor_roster_rows as usize,
        T15_FRAME_ADVISORS
    );
    assert_eq!(
        legacy.stats.unit_leader_rows as usize,
        T15_FRAME_UNIT_LEADERS
    );
    assert_eq!(
        legacy.stats.country_leader_role_rows as usize,
        T15_FRAME_COUNTRY_LEADER_ROLES
    );
}

#[test]
fn d47_token_pointer_is_captured_after_the_name_read_not_before() {
    const RECYCLED_TOKEN: &str = "SOV_unrelated_othr_token";
    let arena = BenchArena::new(0x4000);
    let character = arena.base;
    let old_buffer = arena.base + 0x1000;
    let new_buffer = arena.base + 0x2000;
    let name_buffer = arena.base + 0x3000;

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
        arena.put_msvc_string(
            character + CCHARACTER_TOKEN_OFF as usize,
            new_buffer,
            "GER_NEW_TOKEN_VALUE_here",
        );
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

/// `Hajr<e-diaeresis> Fetahu`, character 69 of rc_vanilla7, whose token the strict
/// byte rule refused. Written as an escape: this file is read back by
/// source-law scans and a raw non-ASCII byte here would make the test's
/// meaning depend on the checkout's encoding.
const GENERATED_NON_ASCII_NAME: &str = "Hajr\u{eb} Fetahu";

#[test]
fn d47_identity_token_publishes_the_generated_name_bytes_the_strict_rule_refused() {
    let arena = BenchArena::new(0x4000);
    let character = arena.base;
    let name_buffer = arena.base + 0x1000;
    let token_buffer = arena.base + 0x2000;

    arena.put_msvc_string(
        character + CCHARACTER_NAME_OFF as usize,
        name_buffer,
        GENERATED_NON_ASCII_NAME,
    );
    arena.put_msvc_string(
        character + CCHARACTER_TOKEN_OFF as usize,
        token_buffer,
        GENERATED_NON_ASCII_NAME,
    );

    page_cache_new_frame();
    let (name, token) = domain47_character_root_strings(character);
    assert_eq!(name.as_deref(), Some(GENERATED_NON_ASCII_NAME));
    assert_eq!(
        token.as_deref(),
        Some(GENERATED_NON_ASCII_NAME),
        "the engine's own name is localize(token), so these are one byte string"
    );

    assert_eq!(
        read_msvc_string(
            character + CCHARACTER_TOKEN_OFF as usize,
            DOMAIN47_CCHARACTER_STRING_MAX_LEN,
        ),
        None,
        "the strict 32..127 rule this site no longer uses refuses these bytes"
    );
}

#[test]
fn d47_identity_token_is_byte_identical_for_an_authored_ascii_key() {
    const KEY: &str = "ALB_midhat_frasheri";
    let localised = "Midhat Frash\u{eb}ri";
    let arena = BenchArena::new(0x4000);
    let character = arena.base;

    arena.put_msvc_string(
        character + CCHARACTER_NAME_OFF as usize,
        arena.base + 0x1000,
        localised,
    );
    arena.put_msvc_string(
        character + CCHARACTER_TOKEN_OFF as usize,
        arena.base + 0x2000,
        KEY,
    );

    page_cache_new_frame();
    let (name, token) = domain47_character_root_strings(character);
    assert_eq!(name.as_deref(), Some(localised));
    assert_eq!(token.as_deref(), Some(KEY));
}

#[test]
fn d47_identity_token_still_refuses_a_control_character_payload() {
    let arena = BenchArena::new(0x4000);
    let character = arena.base;

    arena.put_msvc_string(
        character + CCHARACTER_NAME_OFF as usize,
        arena.base + 0x1000,
        "Readable Name",
    );
    arena.put_msvc_string(
        character + CCHARACTER_TOKEN_OFF as usize,
        arena.base + 0x2000,
        "GER_token\u{1}value",
    );

    page_cache_new_frame();
    let (name, token) = domain47_character_root_strings(character);
    assert_eq!(name.as_deref(), Some("Readable Name"));
    assert_eq!(token, None);
}

#[test]
fn d47_unit_leader_identity_token_takes_the_decode_the_character_copy_needs() {
    let arena = BenchArena::new(0x4000);
    let leader = arena.base;

    arena.put_msvc_string(
        leader + domain47::extractor::CUNIT_LEADER_NAME_OFF as usize,
        arena.base + 0x1000,
        GENERATED_NON_ASCII_NAME,
    );
    arena.put_msvc_string(
        leader + domain47::extractor::CUNIT_LEADER_TOKEN_OFF as usize,
        arena.base + 0x2000,
        GENERATED_NON_ASCII_NAME,
    );

    page_cache_new_frame();
    let (name, token) = domain47_unit_leader_root_strings(leader);
    assert_eq!(name.as_deref(), Some(GENERATED_NON_ASCII_NAME));
    assert_eq!(token.as_deref(), Some(GENERATED_NON_ASCII_NAME));
}

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

fn tail_loop_legacy(world: &TailBenchWorld) -> usize {
    let mut sink = 0usize;
    for addr in &world.unit_leaders {
        let name = domain47_read_msvc_display_string(
            addr + domain47::extractor::CUNIT_LEADER_NAME_OFF as usize,
            160,
        );
        let token = read_msvc_string(
            addr + domain47::extractor::CUNIT_LEADER_TOKEN_OFF as usize,
            160,
        );
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

#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_prewarm_tail_ab() {
    let world = tail_bench_world();
    page_cache_new_frame();
    let legacy_sink = tail_loop_legacy(&world);
    page_cache_new_frame();
    let now_sink = tail_loop_now(&world);
    assert!(legacy_sink > 0 && now_sink > 0, "fixture resolved nothing");
    assert_eq!(
        legacy_sink, now_sink,
        "the two arms did not resolve the same strings"
    );

    let (legacy, now) = time_us_median_ab(5, || tail_loop_legacy(&world), || tail_loop_now(&world));
    println!(
        "d47_cost AB prewarm_tail: legacy {legacy:.1} us -> snapshot {now:.1} us ({:+.1}%) over \
         {T15_TAIL_UNIT_LEADERS} leaders + {T15_TAIL_ADVISORS} advisors + {T15_TAIL_TRAITS} traits",
        (now - legacy) / legacy * 100.0
    );
}

struct RecycleFixture {
    arena: BenchArena,
    object: usize,
    first_off: usize,
    second_off: usize,
    first_buffer: usize,
    second_old_buffer: usize,
    second_new_buffer: usize,
}

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

    fn recycle_second(&self) {
        self.arena.put_msvc_string(
            self.object + self.second_off,
            self.second_new_buffer,
            LIVE_SECOND,
        );
        assert_eq!(RECYCLED_SECOND.len(), ORIGINAL_SECOND.len());
        self.arena
            .put_bytes(self.second_old_buffer, RECYCLED_SECOND.as_bytes());
        self.arena
            .put_bytes(self.second_old_buffer + RECYCLED_SECOND.len(), &[0u8]);
    }

    fn recycle_first(&self, replacement: &str) {
        let new_buffer = self.arena.base + 0x4000;
        self.arena
            .put_msvc_string(self.object + self.first_off, new_buffer, replacement);
        assert_eq!(RECYCLED_FIRST.len(), ORIGINAL_FIRST.len());
        self.arena
            .put_bytes(self.first_buffer, RECYCLED_FIRST.as_bytes());
        self.arena
            .put_bytes(self.first_buffer + RECYCLED_FIRST.len(), &[0u8]);
    }
}

const RECYCLED_FIRST: &str = "A Rather Longer Other String";

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

#[test]
fn d47_advisor_slot_pointer_is_captured_after_the_idea_token_read() {
    let fixture = RecycleFixture::new(
        domain47::extractor::CADVISOR_IDEA_TOKEN_OFF as usize,
        domain47::extractor::CADVISOR_SLOT_OFF as usize,
    );
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

#[test]
fn d47_trait_token_tear_check_uses_two_independent_observations() {
    let fixture = RecycleFixture::new(
        domain47::extractor::CUNIT_LEADER_TRAIT_TOKEN_OFF as usize,
        domain47::extractor::CUNIT_LEADER_TRAIT_TOKEN_OFF as usize + 0x40,
    );
    page_cache_new_frame();
    assert_eq!(
        domain47_trait_token_stable(fixture.object),
        Some(Some(ORIGINAL_FIRST.to_string())),
        "control read must succeed, or the refusal below proves nothing"
    );

    page_cache_new_frame();
    let torn = domain47_trait_token_stable_with_between(fixture.object, || {
        fixture.recycle_first("A Rather Longer Live String")
    });
    assert_eq!(
        torn, None,
        "a token that changed between the two observations must be reported TORN"
    );
}

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
    assert!(sso > 0 && heap > 0, "fixture covered only one string form");
}

#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_removed_frame_walk_bookkeeping() {
    let world = bench_world();
    let rows = build_bench_frame(&world, &Domain47InProcessMemory)
        .frame
        .characters;
    let identity_rows = rows.len();

    let (identity_us, _) = time_us_median(5, || {
        let mut map =
            std::collections::BTreeMap::<(u32, u32), (Option<String>, Option<String>)>::new();
        for row in &rows {
            map.insert(
                (row.country_tag_db_id, row.char_id),
                (row.name.clone(), row.token.clone()),
            );
        }
        map.len()
    });

    let known_inserts = identity_rows
        + T15_FRAME_UNIT_LEADERS
        + T15_FRAME_COUNTRY_LEADER_ROLES
        + T15_FRAME_ADVISORS;
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
            let r = domain47::adapter::build_lane_frame_from_adapter_input_ref(
                &mem,
                &world.roots,
                &input,
            );
            r.frame.characters.len()
                + r.frame.unit_leaders.len()
                + r.frame.advisor_roster.len()
                + r.frame.country_leader_roles.len()
        });
        println!("d47_cost frame_walk/{name}: {us:.1} us/frame sink={sink}");
    }
}

const T15_POLITICS_COUNTRIES: usize = 437;
const T15_IDEAS_PER_COUNTRY: usize = 12;
const T15_DISTINCT_IDEAS_PER_COUNTRY: usize = T15_IDEAS_PER_COUNTRY - 1;

const POLITICS_COUNTRY_STRIDE: usize = 0x2000;
const POLITICS_IDEA_STRIDE: usize = 0x80;
const POLITICS_IDEA_TOKEN_BYTES: usize = 0x40;

struct PoliticsBenchWorld {
    arena: BenchArena,
    module_base: usize,
    country_vtable: usize,
    rows: Vec<CountryObjectRow>,
    spare: usize,
}

fn politics_bench_world() -> PoliticsBenchWorld {
    let size = T15_POLITICS_COUNTRIES * POLITICS_COUNTRY_STRIDE + 0x10000;
    let arena = BenchArena::in_heap_window(size);
    let base = arena.base;
    let module_base = base;
    let country_vtable = module_base + COUNTRY_VTABLE_RVA;
    let status_vtable = module_base + POLITICAL_STATUS_VTABLE_RVA;
    let party_vtable = module_base + POLITICAL_PARTY_VTABLE_RVA;
    let group_vtable = module_base + IDEOLOGY_GROUP_VTABLE_RVA;
    let leader_vtable = module_base + COUNTRY_LEADER_VTABLE_RVA;
    let character_vtable = module_base + POLITICS_CHARACTER_VTABLE_RVA;
    let idea_vtable_primary = module_base + IDEA_VTABLE_RVA_PRIMARY;
    let idea_vtable_secondary = module_base + IDEA_VTABLE_RVA_SECONDARY;

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
        arena.put_u64(
            slot + COUNTRY + OBJ_POLITICAL_STATUS,
            (slot + STATUS) as u64,
        );
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

        arena.put_u64(
            slot + STATUS + POLITICAL_STATUS_IDEAS_DATA,
            (slot + IDEA_PTRS) as u64,
        );
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
            arena.put_u64(
                idea,
                if entry % 3 == 2 {
                    idea_vtable_secondary as u64
                } else {
                    idea_vtable_primary as u64
                },
            );
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

fn politics_status_vtable(world: &PoliticsBenchWorld) -> usize {
    world.module_base + POLITICAL_STATUS_VTABLE_RVA
}

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

#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_country_politics_ab() {
    let world = politics_bench_world();
    let expected_tokens = T15_POLITICS_COUNTRIES * T15_DISTINCT_IDEAS_PER_COUNTRY;
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
            sink += domain47_read_active_ideas_bracketed(*object_ptr, status_vtable, vt1, vt2)
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
        assert_eq!(
            legacy.active_idea_tokens.len(),
            T15_DISTINCT_IDEAS_PER_COUNTRY,
            "the fixture must exercise dedup"
        );
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

#[test]
fn d47_idea_walk_refuses_a_country_repointed_at_another_status_object() {
    let world = politics_bench_world();
    let vt1 = world.module_base + IDEA_VTABLE_RVA_PRIMARY;
    let vt2 = world.module_base + IDEA_VTABLE_RVA_SECONDARY;
    let country = world.rows[0].object_ptr;
    let victim_status = politics_status_ptrs(&world)[1];
    let own_status = politics_status_ptrs(&world)[0];
    assert_ne!(own_status, victim_status);

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
    page_cache_new_frame();
    let control = politics_bracketed_ideas(&world, 0).expect("control walk must succeed");
    assert_eq!(control.len(), T15_IDEAS_PER_COUNTRY);
    let victim_tokens = politics_bracketed_ideas(&world, 1).expect("victim must resolve");
    assert_ne!(
        control, victim_tokens,
        "the two countries must carry DIFFERENT idea lists, or the swap is invisible"
    );

    world.arena.put_u64(
        victim_status + POLITICAL_STATUS_COUNTRY_BACKLINK,
        country as u64,
    );
    page_cache_new_frame();
    assert_eq!(
        validated_political_status_ptr(country, politics_status_vtable(&world)),
        Some(own_status),
        "the country must still resolve its OWN status before the swap"
    );

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

#[test]
fn d47_country_row_refuses_a_ruler_that_changed_during_the_idea_walk() {
    let world = politics_bench_world();
    let expected = &world.rows[0];

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

    let status = world.arena.base + 0x0200;
    let victim_party = world.arena.base + POLITICS_COUNTRY_STRIDE + 0x0400;
    let original = read_u64(status + POLITICAL_STATUS_RULING_PARTY).expect("party ptr must read");
    assert_ne!(original as usize, victim_party);

    page_cache_new_frame();
    let torn = domain47_country_politics_row_with_between(
        world.module_base,
        world.country_vtable,
        expected,
        || {
            world
                .arena
                .put_u64(status + POLITICAL_STATUS_RULING_PARTY, victim_party as u64)
        },
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

    world
        .arena
        .put_u64(status + POLITICAL_STATUS_RULING_PARTY, original);
    page_cache_new_frame();
    assert!(
        domain47_country_politics_row(world.module_base, world.country_vtable, expected).is_some()
    );
}

#[test]
fn d47_country_row_refuses_a_status_root_that_returned() {
    let world = politics_bench_world();
    let expected = &world.rows[0];
    let country = expected.object_ptr;
    let own_status = politics_status_ptrs(&world)[0];
    let victim_status = politics_status_ptrs(&world)[1];

    page_cache_new_frame();
    let control = domain47_country_politics_row(world.module_base, world.country_vtable, expected)
        .expect("control row must resolve");
    let victim_ideas = politics_bracketed_ideas(&world, 1).expect("victim must resolve");
    assert_ne!(
        control.active_idea_tokens, victim_ideas,
        "the two status objects must carry DIFFERENT ideas, or the swap is invisible"
    );

    world.arena.put_u64(
        victim_status + POLITICAL_STATUS_COUNTRY_BACKLINK,
        country as u64,
    );

    page_cache_new_frame();
    let torn = domain47_country_politics_row_with_between(
        world.module_base,
        world.country_vtable,
        expected,
        || {
            world
                .arena
                .put_u64(country + OBJ_POLITICAL_STATUS, victim_status as u64)
        },
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
    assert_eq!(cached_idea_token(0x1000, vt1, vt2), None);
    assert_eq!(domain47_idea_token(0x1000, vt1, vt2), None);

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

    let status_ptr = politics_status_ptrs(&world)[0];
    let (data, _size, _cap) = politics_ideas_header(status_ptr, None).expect("header must read");
    let original = politics_idea_ptr_at(data, 0, None).expect("idea pointer must read");
    page_cache_new_frame();
    assert!(
        politics_bracketed_ideas(&world, 0).is_some(),
        "control walk must succeed, or the refusal below proves nothing"
    );

    let wrong_vtable_idea = world.spare + 0x200;
    world
        .arena
        .put_u64(wrong_vtable_idea, (vt1 + 0x1000) as u64);
    world.arena.put_msvc_string(
        wrong_vtable_idea + IDEA_ID,
        wrong_vtable_idea + 0x40,
        "law_9",
    );

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

    let non_utf8_idea = world.spare + 0x600;
    world.arena.put_u64(non_utf8_idea, vt1 as u64);
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
        (
            "an idea whose vtable is not an accepted one",
            wrong_vtable_idea,
        ),
        (
            "a valid idea at an address that fails the heap gate",
            unaligned_idea,
        ),
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

#[test]
fn d47_idea_walk_refuses_an_id_string_rewritten_in_place() {
    let world = politics_bench_world();
    let vt1 = world.module_base + IDEA_VTABLE_RVA_PRIMARY;
    let vt2 = world.module_base + IDEA_VTABLE_RVA_SECONDARY;
    let status_ptr = politics_status_ptrs(&world)[0];
    let (data, _size, _cap) = politics_ideas_header(status_ptr, None).expect("header must read");
    let idea_ptr = politics_idea_ptr_at(data, 0, None).expect("idea pointer must read");

    page_cache_new_frame();
    let before = politics_bracketed_ideas(&world, 0).expect("control walk must succeed");
    assert_eq!(before.len(), T15_IDEAS_PER_COUNTRY);
    let original = before[0].clone();

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

    page_cache_new_frame();
    assert_eq!(
        politics_bracketed_ideas(&world, 0).map(|v| v.len()),
        Some(T15_IDEAS_PER_COUNTRY),
        "control walk must succeed"
    );

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

    unsafe {
        std::ptr::write_unaligned(data as *mut u64, original as u64);
    }
    page_cache_new_frame();
    assert_eq!(
        politics_bracketed_ideas(&world, 0).map(|v| v.len()),
        Some(T15_IDEAS_PER_COUNTRY),
    );
}

struct ClockBenchWorld {
    _arena: BenchArena,
    _elsewhere: BenchArena,
    module_base: usize,
    date_addr: usize,
    slot_addr: usize,
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

#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_prewarm_clock_bracket() {
    let world = clock_bench_world();
    let elsewhere = world.elsewhere_addr;
    let slot = world.slot_addr;
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
    page_cache_new_frame();
    assert!(
        read_authenticated_scheduler_clock(world.module_base, world.date_addr).is_some(),
        "clock fixture must authenticate, or the present arms measure the refusal path"
    );
    assert!(
        read_u64(elsewhere).is_some(),
        "the displacing read must reach committed memory"
    );

    let (warm, warm_lo, warm_hi) =
        time_us_paired_delta(9, BENCH_ITERATIONS, warm_absent, warm_present);
    let (cold, cold_lo, cold_hi) =
        time_us_paired_delta(9, BENCH_ITERATIONS, cold_absent, cold_present);
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

const SCATTER_PAGE: usize = 0x1000;

struct ScatteredCensusWorld {
    _arena: BenchArena,
    roots: ResolvedTypeRoots,
    census_characters: Vec<usize>,
    string_buffers: Vec<usize>,
}

fn scatter_rng(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn scattered_census_world() -> ScatteredCensusWorld {
    let count = T28_MANAGER_ENTRIES;
    let arena = BenchArena::in_heap_window(2 * count * SCATTER_PAGE + SCATTER_PAGE);
    let string_base = arena.base + count * SCATTER_PAGE;

    let mut pages: Vec<usize> = (0..count).collect();
    let mut state = 0x5eed_d47_c0ffee_u64;
    for idx in (1..count).rev() {
        let swap = (scatter_rng(&mut state) % (idx as u64 + 1)) as usize;
        pages.swap(idx, swap);
    }
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

fn scattered_census_loop(world: &ScatteredCensusWorld, prefetch: bool) -> usize {
    scattered_census_loop_at(world, prefetch.then_some(DOMAIN47_PREFETCH_DISTANCE))
}

fn scattered_census_loop_at(world: &ScatteredCensusWorld, distance: Option<usize>) -> usize {
    let mem = Domain47InProcessMemory;
    let mut sink = 0usize;
    for (ordinal, addr) in world.census_characters.iter().copied().enumerate() {
        if let Some(distance) = distance {
            if let Some(ahead) = world.census_characters.get(ordinal.wrapping_add(distance)) {
                domain47_prefetch_object(*ahead, domain47::extractor::CCHARACTER_RAW_FIELDS_SPAN);
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

const T32_CENSUS_CHARACTERS: usize = 13_403;
const T32_DISTINCT_NAME_TOKENS: usize = 8_663;

fn census_bookkeeping(strings: bool) -> usize {
    census_bookkeeping_with(strings, true)
}

fn census_bookkeeping_with(strings: bool, reuse_roster_set: bool) -> usize {
    let roster_addresses: Vec<usize> = (0..T32_CENSUS_CHARACTERS)
        .map(|index| 0x1_0000_0000usize + index * CHARACTER_STRIDE)
        .collect();
    let mut admission =
        Domain47CensusAdmission::new(roster_addresses.iter(), T32_CENSUS_CHARACTERS);
    for index in 0..T32_CENSUS_CHARACTERS {
        let addr = 0x1_0000_0000usize + index * CHARACTER_STRIDE;
        let unit_leader_ptr = if index % 10 == 0 {
            0x2_0000_0000u64 + index as u64 * 0x1000
        } else {
            0
        };
        assert!(
            admission.admit(
                addr,
                (index % T32_DISTINCT_NAME_TOKENS) as u32,
                domain47::extractor::CharacterRawFields {
                    character_addr: addr as u64,
                    country_tag_db_id: (index % 200) as u32,
                    char_id: index as u32,
                    unit_leader_ptr,
                },
            ),
            "the bookkeeping fixture must hold no duplicate pointer"
        );
    }
    let Domain47CensusAdmission {
        character_pointers,
        name_identities,
        characters,
        ..
    } = admission;
    let censused_pointer_count = character_pointers.len();
    let keys = if reuse_roster_set {
        Domain47ManagerNameIndexKeys::from_roster(character_pointers, &name_identities)
    } else {
        Domain47ManagerNameIndexKeys::from_identities(&name_identities)
    };
    let lookup = |_| Domain47ManagerIndexLookup::Absent;
    let opening = keys.check(lookup);
    let closing = keys.check(lookup);

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
    let mut roots: Vec<CharacterRoot> = Vec::new();
    for (addr, raw) in &characters {
        if let Domain47CharacterAdmit::Insert = dedup.admit_character(
            raw.char_id,
            raw.country_tag_db_id,
            raw.unit_leader_ptr,
            roots.len(),
        ) {
            dedup.admit_leader(raw.unit_leader_ptr, raw.country_tag_db_id);
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
    let string_bytes: usize = roots
        .iter()
        .map(|root| {
            root.name.as_ref().map_or(0, |value| value.len())
                + root.token.as_ref().map_or(0, |value| value.len())
        })
        .sum();
    censused_pointer_count
        + opening.distinct_name_token_count as usize
        + closing.distinct_name_token_count as usize
        + roots.len()
        + string_bytes
}

#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_census_bookkeeping() {
    let with = census_bookkeeping(true);
    let without = census_bookkeeping(false);
    assert!(without > 0, "bookkeeping arm did nothing");
    assert!(
        with > without,
        "the string arm allocated nothing, so the difference below is not the \
         cost of the two owned root strings"
    );
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
        "d47_cost AB roster pointer list: rebuilt {rebuilt:.1} us -> census list reused \
         {reused:.1} us ({:+.1}%), one Vec<usize> collect of {} entries and its sort \
         removed from the serial prewarm",
        (reused - rebuilt) / rebuilt * 100.0,
        T32_CENSUS_CHARACTERS,
    );
}

fn bookkeeping_addresses(shuffled: bool) -> Vec<usize> {
    let mut addrs: Vec<usize> = (0..T32_CENSUS_CHARACTERS)
        .map(|index| 0x1_0000_0000usize + index * CHARACTER_STRIDE)
        .collect();
    if shuffled {
        let mut state = 0x5eed_d47_b0_0c_11e5_u64;
        for idx in (1..addrs.len()).rev() {
            let swap = (scatter_rng(&mut state) % (idx as u64 + 1)) as usize;
            addrs.swap(idx, swap);
        }
    }
    addrs
}

fn sorted_fingerprint<I: IntoIterator<Item = usize>>(sequence: I) -> usize {
    sequence
        .into_iter()
        .enumerate()
        .fold(0usize, |acc, (position, value)| {
            acc.wrapping_add(value.wrapping_mul(position.wrapping_add(1)))
        })
}

fn census_containers_btree(addrs: &[usize]) -> usize {
    let mut character_pointers = BTreeSet::new();
    let mut name_identities: Vec<(u32, usize)> = Vec::with_capacity(addrs.len());
    for (index, addr) in addrs.iter().copied().enumerate() {
        if !character_pointers.insert(addr) {
            return 0;
        }
        name_identities.push(((index % T32_DISTINCT_NAME_TOKENS) as u32, addr));
    }
    let censused_pointer_count = character_pointers.len();
    let mut indexed_names = BTreeSet::new();
    for (name_token_id, _) in &name_identities {
        if (*name_token_id as i32) >= 0 {
            indexed_names.insert(*name_token_id);
        }
    }
    let never_a_real_token = core::hint::black_box(u32::MAX);
    let mut visited = 0usize;
    let mut check = |visited: &mut usize| {
        let all_in_roster = indexed_names.iter().all(|name_token_id| {
            *visited += 1;
            match if *name_token_id == never_a_real_token {
                Domain47ManagerIndexLookup::Invalid
            } else {
                Domain47ManagerIndexLookup::Absent
            } {
                Domain47ManagerIndexLookup::Absent => true,
                Domain47ManagerIndexLookup::Found(indexed_pointer) => {
                    character_pointers.contains(&indexed_pointer)
                }
                Domain47ManagerIndexLookup::Invalid => false,
            }
        });
        (all_in_roster as usize, indexed_names.len())
    };
    let (opening_ok, opening_count) = check(&mut visited);
    visited = core::hint::black_box(visited);
    core::hint::black_box(&indexed_names);
    let (closing_ok, closing_count) = check(&mut visited);
    assert_eq!(
        visited,
        2 * indexed_names.len(),
        "both index passes must traverse every key"
    );
    assert!(opening_ok == 1 && closing_ok == 1, "fixture check refused");
    // the elapsed time and discards the sink, so in a release build nothing
    core::hint::black_box(
        censused_pointer_count
            + opening_count
            + closing_count
            + visited
            + sorted_fingerprint(character_pointers.iter().copied()),
    )
}

fn census_containers_sorted_vec(addrs: &[usize]) -> usize {
    let mut ordered: Vec<(usize, usize)> = addrs.iter().copied().zip(0usize..).collect();
    ordered.sort_unstable();
    let mut duplicate_scan_index: Option<usize> = None;
    for pair in ordered.windows(2) {
        if pair[0].0 == pair[1].0 {
            let second_sighting = pair[1].1;
            duplicate_scan_index = Some(
                duplicate_scan_index
                    .map_or(second_sighting, |held: usize| held.min(second_sighting)),
            );
        }
    }
    let mut character_pointers: Vec<usize> = Vec::with_capacity(addrs.len());
    let mut name_identities: Vec<(u32, usize)> = Vec::with_capacity(addrs.len());
    for (index, addr) in addrs.iter().copied().enumerate() {
        if duplicate_scan_index == Some(index) {
            return 0;
        }
        character_pointers.push(addr);
        name_identities.push(((index % T32_DISTINCT_NAME_TOKENS) as u32, addr));
    }
    character_pointers.sort_unstable();
    character_pointers.dedup();
    let censused_pointer_count = character_pointers.len();
    let mut indexed_names: Vec<u32> = name_identities
        .iter()
        .filter(|(name_token_id, _)| (*name_token_id as i32) >= 0)
        .map(|(name_token_id, _)| *name_token_id)
        .collect();
    indexed_names.sort_unstable();
    indexed_names.dedup();
    let never_a_real_token = core::hint::black_box(u32::MAX);
    let mut visited = 0usize;
    let mut check = |visited: &mut usize| {
        let all_in_roster = indexed_names.iter().all(|name_token_id| {
            *visited += 1;
            match if *name_token_id == never_a_real_token {
                Domain47ManagerIndexLookup::Invalid
            } else {
                Domain47ManagerIndexLookup::Absent
            } {
                Domain47ManagerIndexLookup::Absent => true,
                Domain47ManagerIndexLookup::Found(indexed_pointer) => {
                    character_pointers.binary_search(&indexed_pointer).is_ok()
                }
                Domain47ManagerIndexLookup::Invalid => false,
            }
        });
        (all_in_roster as usize, indexed_names.len())
    };
    let (opening_ok, opening_count) = check(&mut visited);
    visited = core::hint::black_box(visited);
    core::hint::black_box(&indexed_names);
    let (closing_ok, closing_count) = check(&mut visited);
    assert_eq!(
        visited,
        2 * indexed_names.len(),
        "both index passes must traverse every key"
    );
    assert!(opening_ok == 1 && closing_ok == 1, "fixture check refused");
    core::hint::black_box(
        censused_pointer_count
            + opening_count
            + closing_count
            + visited
            + sorted_fingerprint(character_pointers.iter().copied()),
    )
}

// design nobody is choosing between, and it had acquired two defects of its own

#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_census_container_shape_ab() {
    for (label, shuffled) in [("ascending", false), ("shuffled", true)] {
        let addrs = bookkeeping_addresses(shuffled);
        let btree = census_containers_btree(&addrs);
        let sorted = census_containers_sorted_vec(&addrs);
        assert!(btree > 0, "{label}: BTreeSet arm refused its own fixture");
        assert_eq!(
            btree, sorted,
            "{label}: the sorted-vector arm did not reproduce the ordered \
             containers' result, so the ratio below would compare two \
             different computations"
        );
        let mut with_duplicate = addrs.clone();
        with_duplicate.push(addrs[0]);
        assert_eq!(
            census_containers_btree(&with_duplicate),
            0,
            "{label}: the BTreeSet arm admitted a repeated pointer"
        );
        assert_eq!(
            census_containers_sorted_vec(&with_duplicate),
            0,
            "{label}: the sorted-vector arm admitted a repeated pointer"
        );
        let (btree_us, sorted_us, balanced) = time_balanced_ratio_ab_n(
            10,
            20,
            || census_containers_btree(&addrs),
            || census_containers_sorted_vec(&addrs),
        );
        println!(
            "d47_cost census containers ({label} addresses, t32 widths, \
             {} pointers / {} distinct name tokens): BTreeSet {btree_us:.1} us \
             -> sorted Vec {sorted_us:.1} us, balanced ratio {balanced:.3} \
             ({:+.1}% and {:.1} us saved at this width)",
            T32_CENSUS_CHARACTERS,
            T32_DISTINCT_NAME_TOKENS,
            (balanced - 1.0) * 100.0,
            btree_us - sorted_us,
        );
    }
}

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

#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_census_scatter_penalty() {
    let contiguous = bench_world();
    let scattered = scattered_census_world();
    page_cache_new_frame();
    assert!(
        scattered_census_loop(&scattered, false) > 0,
        "fixture read nothing"
    );

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

#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_census_prefetch_ab() {
    let world = scattered_census_world();
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

#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_census_prefetch_distance_sweep() {
    let world = scattered_census_world();
    let mut line = String::new();
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

fn time_balanced_ratio_ab_n<A: FnMut() -> usize, B: FnMut() -> usize>(
    runs: usize,
    iterations: usize,
    mut a: A,
    mut b: B,
) -> (f64, f64, f64) {
    assert!(
        runs >= 2 && runs % 2 == 0,
        "an order-balanced statistic needs an even run count; {runs} would leave \
         one order in the majority"
    );
    let mut a_samples = Vec::with_capacity(runs);
    let mut b_samples = Vec::with_capacity(runs);
    let mut ab_ratios = Vec::with_capacity(runs / 2);
    let mut ba_ratios = Vec::with_capacity(runs / 2);
    for run in 0..runs {
        let a_first = run % 2 == 0;
        let (a_us, b_us) = if a_first {
            let a_us = time_us(iterations, &mut a).0;
            let b_us = time_us(iterations, &mut b).0;
            (a_us, b_us)
        } else {
            let b_us = time_us(iterations, &mut b).0;
            let a_us = time_us(iterations, &mut a).0;
            (a_us, b_us)
        };
        a_samples.push(a_us);
        b_samples.push(b_us);
        if a_first {
            ab_ratios.push(b_us / a_us);
        } else {
            ba_ratios.push(b_us / a_us);
        }
    }
    let median = |mut samples: Vec<f64>| {
        samples.sort_by(|x: &f64, y: &f64| x.partial_cmp(y).expect("no NaN timings"));
        samples[samples.len() / 2]
    };
    let balanced = (median(ab_ratios) * median(ba_ratios)).sqrt();
    (median(a_samples), median(b_samples), balanced)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Domain47ProbeReads {
    identity: usize,
    headers: usize,
}

fn ej_probe_word_loop(world: &EjShadowWorld) -> Domain47ProbeReads {
    let mut reads = Domain47ProbeReads::default();
    for (ordinal, addr) in world.census_characters.iter().copied().enumerate() {
        if let Some(ahead) = world
            .census_characters
            .get(ordinal.wrapping_add(DOMAIN47_PREFETCH_DISTANCE))
        {
            domain47_prefetch_object(*ahead, domain47::extractor::CCHARACTER_RAW_FIELDS_SPAN);
        }
        reads.identity += read_u64_stable(addr).is_some() as usize;
    }
    reads
}

fn ej_probe_headers_loop(world: &EjShadowWorld) -> Domain47ProbeReads {
    let mut reads = Domain47ProbeReads::default();
    for (ordinal, addr) in world.census_characters.iter().copied().enumerate() {
        if let Some(ahead) = world
            .census_characters
            .get(ordinal.wrapping_add(DOMAIN47_PREFETCH_DISTANCE))
        {
            domain47_prefetch_object(*ahead, domain47::extractor::CCHARACTER_RAW_FIELDS_SPAN);
        }
        reads.headers += domain47_shadow_walk_headers(addr)
            .is_some_and(|headers| headers.u64(0).is_some()) as usize;
    }
    reads
}

fn ej_probe_both_loop(world: &EjShadowWorld) -> Domain47ProbeReads {
    let mem = Domain47InProcessMemory;
    let mut reads = Domain47ProbeReads::default();
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
        reads.identity += (first.is_ok() && opening.is_some()) as usize;
        reads.headers += domain47_shadow_walk_headers(addr)
            .is_some_and(|headers| headers.u64(0).is_some()) as usize;
    }
    reads
}

#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_change_signal_probe_floor_ab() {
    let world = ej_shadow_world();
    let width = world.census_characters.len();
    page_cache_new_frame();

    let word = ej_probe_word_loop(&world);
    assert_eq!(
        word.identity, width,
        "the one-word arm must read its word from every character"
    );
    page_cache_new_frame();
    let headers = ej_probe_headers_loop(&world);
    assert_eq!(
        headers.headers, width,
        "the header arm must read the header window of every character"
    );
    page_cache_new_frame();
    let both = ej_probe_both_loop(&world);
    assert_eq!(
        (both.identity, both.headers),
        (width, width),
        "the identity+headers arm must read BOTH components of every character, \
         not one of them"
    );
    page_cache_new_frame();
    let (with_walk, denominator_walk) = ej_census_loop_with_walk(&world, true);
    page_cache_new_frame();
    let timed_with_walk = ej_census_loop(&world, true);
    assert_eq!(
        timed_with_walk, with_walk,
        "the wrapper the timed arm calls does not do what the guard below \
         checked, so the guard is about a different function"
    );
    page_cache_new_frame();
    let without_walk = ej_census_loop(&world, false);
    assert!(without_walk > 0, "the denominator arm read nothing");
    assert_ne!(
        timed_with_walk, without_walk,
        "the timed denominator and the walk-less arm cost the same sink, so the \
         `shadow` argument reached nothing"
    );
    assert_eq!(
        denominator_walk.refused, 0,
        "the denominator's role walk refused a map, so its cost is a refusal \
         path's and not a traversal's"
    );
    assert_eq!(
        (
            denominator_walk.leaders.len(),
            denominator_walk.advisors.len()
        ),
        (EJ_LEADER_ENTRIES, EJ_ADVISOR_ENTRIES),
        "the denominator's role walk did not reach every entry of every map"
    );
    assert_eq!(
        with_walk,
        without_walk + EJ_LEADER_ENTRIES + EJ_ADVISOR_ENTRIES,
        "the denominator's role walk did not reach every entry of every map with \
         zero refusals, so this is not the census-plus-walk shape the ratio is \
         divided by"
    );

    let (full, probe_word, word_ratio) = time_balanced_ratio_ab_n(
        10,
        200,
        || ej_census_loop(&world, true),
        || ej_probe_word_loop(&world).identity,
    );
    let (full_two, probe_headers, header_ratio) = time_balanced_ratio_ab_n(
        10,
        200,
        || ej_census_loop(&world, true),
        || ej_probe_headers_loop(&world).headers,
    );
    let (full_three, probe_both, both_ratio) = time_balanced_ratio_ab_n(
        10,
        200,
        || ej_census_loop(&world, true),
        || {
            let reads = ej_probe_both_loop(&world);
            reads.identity + reads.headers
        },
    );

    println!(
        "d47_cost change-signal probe floor (scattered, one page per character, {width} \
         characters, denominator = the FUSED census + role walk shape): full {full:.1} / \
         {full_two:.1} / {full_three:.1} us -- probe ONE WORD {probe_word:.1} us (balanced \
         {:.1}% of full, ceiling on any body-resident-signal incremental saving {:.1}%) | probe \
         MAP HEADERS {probe_headers:.1} us (balanced {:.1}%, ceiling {:.1}%) | probe \
         IDENTITY+HEADERS {probe_both:.1} us (balanced {:.1}%, ceiling {:.1}%)",
        word_ratio * 100.0,
        (1.0 - word_ratio) * 100.0,
        header_ratio * 100.0,
        (1.0 - header_ratio) * 100.0,
        both_ratio * 100.0,
        (1.0 - both_ratio) * 100.0,
    );
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
    let mut ascending = world.census_characters.clone();
    ascending.sort_unstable();
    ascending.dedup();
    assert_eq!(ascending.len(), T28_MANAGER_ENTRIES);
    assert_ne!(
        ascending, world.census_characters,
        "the visit order is the address order, so nothing is being scattered"
    );
}

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
    assert_eq!(without.len(), T28_MANAGER_ENTRIES);
    assert!(without[0].0.is_some());
    assert!(without[1].2.is_some());
}

const T28_MANAGER_HISTORICAL: usize = 10_116;
const T28_MANAGER_DYNAMIC: usize = 3_278;
const T28_MANAGER_ENTRIES: usize = T28_MANAGER_HISTORICAL + T28_MANAGER_DYNAMIC;

struct ManagerVectorBenchWorld {
    _arena: BenchArena,
    data: usize,
    size: u32,
}

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

#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_manager_vector_ab() {
    let world = manager_vector_bench_world();
    page_cache_new_frame();
    let legacy_entries = manager_vector_entries_legacy(world.data, world.size)
        .expect("legacy arm must read the whole vector");
    page_cache_new_frame();
    let mut retries = 0u32;
    let bulk_entries = domain47_read_manager_vector_entries(world.data, world.size, &mut retries)
        .expect("bulk arm must read the whole vector");
    assert_eq!(
        legacy_entries, bulk_entries,
        "the two arms did not read the same entries"
    );
    assert_eq!(legacy_entries.len(), T28_MANAGER_ENTRIES);
    assert_eq!(retries, 0, "the bulk arm fell back to per-entry reads");

    let (legacy, bulk) = time_us_median_ab(
        5,
        || manager_vector_entries_legacy(world.data, world.size).map_or(0, |entries| entries.len()),
        || {
            domain47_read_manager_vector_entries(world.data, world.size, &mut 0)
                .map_or(0, |entries| entries.len())
        },
    );
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
    assert_eq!(retries, 0);
}

#[test]
fn d47_manager_vector_bulk_refuses_a_non_heap_entry_like_the_per_entry_read() {
    let entries = DOMAIN47_MANAGER_VECTOR_COPY_ENTRIES + 5;
    let arena = BenchArena::new(0x10000);
    assert!(entries * 8 <= 0x10000);
    for ordinal in 0..entries {
        arena.put_u64(arena.base + ordinal * 8, synthetic_entry(ordinal));
    }
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

#[test]
fn d47_manager_vector_counts_a_fallback_that_preceded_a_failure() {
    let arena = BenchArena::new(0x10000);
    let entries = DOMAIN47_MANAGER_VECTOR_COPY_ENTRIES * 2;
    let data = arena.base + 0x400;
    for ordinal in 0..entries {
        arena.put_u64(data + ordinal * 8, synthetic_entry(ordinal));
    }
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

#[test]
fn d47_manager_vector_falls_back_to_per_entry_reads_on_a_refused_chunk() {
    let arena = BenchArena::new(0x10000);
    let entries = DOMAIN47_MANAGER_VECTOR_COPY_ENTRIES * 2;
    let data = arena.base + 0x400;
    for ordinal in 0..entries {
        arena.put_u64(data + ordinal * 8, synthetic_entry(ordinal));
    }
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
    let straddling = data + DOMAIN47_MANAGER_VECTOR_COPY_ENTRIES * 8;
    let mut probe = vec![0u8; DOMAIN47_MANAGER_VECTOR_COPY_ENTRIES * 8];
    assert!(
        !read_into(straddling, &mut probe),
        "the chunk crossing the protection boundary must be refused"
    );
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
    assert_eq!(retries, 1);
}

/// cargo test -p collector --release --features range-tracer \
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

const EJ_LEADER_ENTRIES: usize = 1_964;
const EJ_LEADER_CHARACTERS: usize = 1_879;
const EJ_ADVISOR_ENTRIES: usize = 6_407;

const EJ_NODE_STRIDE: usize = 0x80;

fn ej_put_tree(
    arena: &BenchArena,
    head: usize,
    node_at: &dyn Fn(usize) -> usize,
    value_off: usize,
    values: &[u64],
) {
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
        arena.put_u64(
            addr + DOMAIN47_TREE_NODE_LEFT_OFF,
            child(2 * index + 1) as u64,
        );
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
        arena.put_u64(
            character + DOMAIN47_LEADER_MAP_SIZE_OFF,
            leader_count as u64,
        );
        arena.put_u64(
            character + DOMAIN47_ADVISOR_MAP_HEAD_OFF,
            advisor_head as u64,
        );
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

#[test]
fn d47_shadow_walk_fixture_links_are_a_whole_tree() {
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
    assert_eq!(
        walk.refused, 0,
        "a well-formed pair of maps refuses nothing"
    );
}

#[test]
fn d47_shadow_walk_reads_each_map_at_the_value_offset_the_re_proved() {
    let world = EjSingleCharacterWorld::new(1, 1);
    let walk = world.walk();
    assert_eq!(walk.refused, 0);
    assert!(walk.leaders.contains(&world.leaders[0]));
    assert!(walk.advisors.contains(&world.advisors[0]));
    assert_eq!(walk.leaders.len(), 1);
    assert_eq!(walk.advisors.len(), 1);
}

/// argument at all. The `CCountryLeader` ownership census a few hundred lines
#[test]
fn d47_the_role_map_header_signal_lies_and_an_incremental_walk_would_publish_stale() {
    let world = EjSingleCharacterWorld::new(3, 3);

    page_cache_new_frame();
    let previous_signal =
        domain47_shadow_walk_headers(world.character).expect("hour N headers must read");
    let previous_root = read_u64(world.leader_head + DOMAIN47_TREE_NODE_PARENT_OFF)
        .expect("hour N root pointer must read");
    let previous = world.walk();
    assert_eq!(previous.refused, 0, "hour N must observe both maps cleanly");
    assert_eq!(previous.leaders.len(), 3);
    assert_eq!(
        previous_root,
        (world.leader_nodes) as u64,
        "the root is node 0, so node 1 is a leaf and not the root"
    );

    let replacement = (world.arena.base + 0x9000) as u64;
    assert!(
        !world.leaders.contains(&replacement),
        "the replacement must be a role object the previous hour did not hold, \
         or this fixture stages no change at all"
    );
    world.arena.put_u64(
        world.leader_nodes + EJ_NODE_STRIDE + DOMAIN47_LEADER_NODE_VALUE_OFF,
        replacement,
    );

    page_cache_new_frame();
    let signal = domain47_shadow_walk_headers(world.character).expect("hour N+1 headers must read");
    for axis in [
        Domain47ShadowWalkMapAxis::Leader,
        Domain47ShadowWalkMapAxis::Advisor,
    ] {
        assert!(
            domain47_shadow_walk_headers_describe_the_same_map(&previous_signal, &signal, axis),
            "the candidate signal reports {axis:?} unchanged — that is the lie \
             this fixture exists to stage, so it must hold"
        );
    }
    assert_eq!(
        read_u64(world.leader_head + DOMAIN47_TREE_NODE_PARENT_OFF),
        Some(previous_root),
        "the root pointer is unchanged too, so widening the signal to \
         (_Myhead, _Mysize, _Myhead->_Parent) does not close this"
    );

    let fresh = world.walk();
    assert_eq!(fresh.refused, 0, "the rewritten map is still well-formed");
    assert_ne!(
        fresh.leaders, previous.leaders,
        "the population changed, and the signal did not"
    );
    assert!(
        fresh.leaders.contains(&replacement),
        "the walk must reach the role the previous hour did not hold"
    );

    let incremental = |previous_signal: &ObjectSnapshot<DOMAIN47_CHARACTER_MAP_HEADERS_LEN>,
                       previous_population: &BTreeSet<u64>| {
        page_cache_new_frame();
        let signal = domain47_shadow_walk_headers(world.character);
        let unchanged = signal.as_ref().is_some_and(|signal| {
            domain47_shadow_walk_headers_describe_the_same_map(
                previous_signal,
                signal,
                Domain47ShadowWalkMapAxis::Leader,
            )
        });
        if unchanged {
            previous_population.clone()
        } else {
            world.walk().leaders
        }
    };
    assert_eq!(
        incremental(&previous_signal, &previous.leaders),
        previous.leaders,
        "the gated design publishes the PREVIOUS hour's role set under this \
         hour's label — a stale publish, which is what makes this signal \
         unusable rather than merely imprecise"
    );
    assert_ne!(
        incremental(&previous_signal, &previous.leaders),
        fresh.leaders,
        "and what it publishes is not what the hour holds",
    );
}

#[test]
fn d47_an_unchanged_census_identity_says_nothing_about_the_roles_beneath_it() {
    let world = EjSingleCharacterWorld::new(2, 0);
    let mem = Domain47InProcessMemory;
    let roots = ResolvedTypeRoots {
        ccharacter_vtable: BENCH_CCHARACTER_VTABLE,
        carmy_leader_vtable: BENCH_CARMY_LEADER_VTABLE,
        cnavy_leader_vtable: BENCH_CNAVY_LEADER_VTABLE,
        ccountry_leader_vtable: BENCH_CCOUNTRY_LEADER_VTABLE,
        cadvisor_vtable: BENCH_CADVISOR_VTABLE,
        cunit_leader_trait_vtable: BENCH_CUNIT_LEADER_TRAIT_VTABLE,
    };
    world
        .arena
        .put_u64(world.character, BENCH_CCHARACTER_VTABLE);
    world.arena.put_u32(
        world.character + CCHARACTER_TYPE_OFF as usize,
        CCHARACTER_TYPE_ID,
    );
    world
        .arena
        .put_u32(world.character + CCHARACTER_ID_OFF as usize, 4711);
    world
        .arena
        .put_u32(world.character + DOMAIN47_CCHARACTER_NAME_TOKEN_ID_OFF, 91);
    world.arena.put_u32(
        world.character + CCHARACTER_COUNTRY_TAG_DB_ID_OFF as usize,
        17,
    );
    world
        .arena
        .put_u64(world.character + CCHARACTER_UNIT_LEADER_PTR_OFF as usize, 0);
    let read_identity = || {
        page_cache_new_frame();
        domain47::extractor::read_character_census_fields(
            &mem,
            &roots,
            world.character as u64,
            || read_u32_stable(world.character + DOMAIN47_CCHARACTER_NAME_TOKEN_ID_OFF),
        )
    };

    let (identity_before, token_before) = read_identity();
    let identity_before = identity_before.expect("hour N identity must read");
    let before = world.walk().leaders;

    let replacement = (world.arena.base + 0x9000) as u64;
    world.arena.put_u64(
        world.leader_nodes + DOMAIN47_LEADER_NODE_VALUE_OFF,
        replacement,
    );

    let (identity_after, token_after) = read_identity();
    let identity_after = identity_after.expect("hour N+1 identity must read");
    let after = world.walk().leaders;

    assert_eq!(
        (identity_before, token_before),
        (identity_after, token_after),
        "every field the census observes about this character — and therefore \
         the one pointer the manager vectors carry about it — is unchanged"
    );
    assert_ne!(
        before, after,
        "and the roles hanging off it are not, so neither membership nor \
         identity can license skipping the walk"
    );
    assert!(
        after.contains(&replacement),
        "the walk reached the role the previous hour did not hold"
    );
}

#[test]
fn d47_shadow_walk_refuses_a_map_whose_size_outruns_its_nodes() {
    let world = EjSingleCharacterWorld::new(3, 2);
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

struct EjShadowWorld {
    _arena: BenchArena,
    roots: ResolvedTypeRoots,
    census_characters: Vec<usize>,
}

fn ej_shadow_world() -> EjShadowWorld {
    let count = T32_CENSUS_CHARACTERS;
    let node_slots = EJ_LEADER_ENTRIES + EJ_ADVISOR_ENTRIES;
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

fn ej_census_loop(world: &EjShadowWorld, shadow: bool) -> usize {
    ej_census_loop_with_walk(world, shadow).0
}

fn ej_census_loop_with_walk(world: &EjShadowWorld, shadow: bool) -> (usize, Domain47ShadowWalk) {
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
    let sink = sink + walk.leaders.len() + walk.advisors.len() + walk.refused as usize;
    (sink, walk)
}

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

struct FcRecheckWorld {
    arena: BenchArena,
    roster: Vec<(usize, domain47::extractor::CharacterRawFields)>,
    laid_out: Vec<usize>,
}

const FC_CHARACTER_STRIDE: usize = 0x1_0000;

const FC_COUNTRY: u32 = 1;

fn fc_bench_owner(char_id: u32) -> Domain47RoleOwner {
    Domain47RoleOwner {
        country_tag_db_id: FC_COUNTRY,
        char_id,
        character_addr: 0,
    }
}

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

    fn with_countries(bodies: &[(u32, u32, Vec<u64>, Vec<u64>)]) -> FcRecheckWorld {
        FcRecheckWorld::with_off_roster(bodies, 0)
    }

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
        for (ordinal, (country_tag_db_id, char_id, leaders, advisors)) in bodies.iter().enumerate()
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
            arena.put_u64(
                character + DOMAIN47_LEADER_MAP_SIZE_OFF,
                leaders.len() as u64,
            );
            arena.put_u64(
                character + DOMAIN47_ADVISOR_MAP_HEAD_OFF,
                advisor_head as u64,
            );
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

    fn body(&self, ordinal: usize) -> u64 {
        self.laid_out[ordinal] as u64
    }

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

#[test]
fn d47_shadow_walk_recheck_miss_is_the_candidate_structural_gap() {
    let world = FcRecheckWorld::new(&[(7, vec![0], vec![0])]);
    let target = world.role(0);
    assert_eq!(
        world.recheck(target, 7, Domain47ShadowWalkMapAxis::Leader, true),
        Domain47ShadowWalkRecheckOutcome::WalkMissed
    );
}

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

#[test]
fn d47_shadow_walk_recheck_refuses_when_the_only_owner_map_cannot_be_walked() {
    let world = FcRecheckWorld::new(&[(7, vec![0, 0], Vec::new())]);
    world
        .arena
        .put_u64(world.roster[0].0 + DOMAIN47_LEADER_MAP_SIZE_OFF, 3);
    let target = world.role(0);
    assert_eq!(
        world.recheck(target, 7, Domain47ShadowWalkMapAxis::Leader, true),
        Domain47ShadowWalkRecheckOutcome::Refused
    );
}

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

#[test]
fn d47_shadow_walk_recheck_misses_across_two_clean_duplicate_bodies() {
    let world = FcRecheckWorld::new(&[(7, vec![0], Vec::new()), (7, vec![0], Vec::new())]);
    let target = world.role(0);
    assert_eq!(
        world.recheck(target, 7, Domain47ShadowWalkMapAxis::Leader, true),
        Domain47ShadowWalkRecheckOutcome::WalkMissed
    );
}

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

#[test]
fn d47_shadow_walk_recheck_examines_every_body_carrying_the_id() {
    let world = FcRecheckWorld::with_countries(&[
        (FC_COUNTRY, 7, vec![0], Vec::new()),
        (FC_COUNTRY + 1, 7, vec![0], Vec::new()),
    ]);
    let target = world.role(0);
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

#[test]
fn d47_shadow_walk_recheck_refuses_rather_than_outrunning_its_body_bound() {
    let bodies: Vec<(u32, Vec<u64>, Vec<u64>)> = (0..=DOMAIN47_SHADOW_WALK_RECHECK_BODY_MAX)
        .map(|_| (7u32, vec![0u64], Vec::new()))
        .collect();
    let world = FcRecheckWorld::new(&bodies);
    let target = world.role(0);
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
    assert_eq!(
        summary.outcome,
        Domain47ShadowWalkRecheckOutcome::WalkMissed
    );
    assert_eq!(
        summary.element.char_id, 22,
        "the FIRST element at the top severity"
    );
    assert_eq!(
        summary.element.axis,
        Domain47ShadowWalkMapAxis::Advisor as u32
    );
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
    let untouched = Domain47ShadowWalkRecheckSummary::default();
    assert_eq!(
        untouched.outcome,
        Domain47ShadowWalkRecheckOutcome::NotChecked
    );
    assert_eq!(untouched.performed(), 0);
}

#[test]
fn d47_leader_owner_guard_refuses_a_same_id_different_country_object() {
    let owner = Domain47RoleOwner {
        country_tag_db_id: 11,
        char_id: 7,
        character_addr: 0x9000,
    };
    assert!(domain47_leader_identity_matches_owner(
        &Ok((11, 7, 0x9000)),
        owner
    ));
    assert!(
        !domain47_leader_identity_matches_owner(&Ok((12, 7, 0x9000)), owner),
        "same char_id under a different country is a DIFFERENT character; this \
         world does not prove char_id unique"
    );
    assert!(!domain47_leader_identity_matches_owner(
        &Ok((11, 8, 0x9000)),
        owner
    ));
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

#[test]
fn d47_advisor_owner_guard_refuses_a_different_country_and_an_unappointed_object() {
    let owner = Domain47RoleOwner {
        country_tag_db_id: 11,
        char_id: 7,
        character_addr: 0x9000,
    };
    let probe =
        |country_tag_db_id: u32, char_id: u32, idea_token: Option<&str>| Domain47AdvisorProbe {
            character: Ok(domain47::extractor::AdvisorRawFields {
                advisor_addr: 0x1000,
                character_addr: 0x2000,
                country_tag_db_id,
                char_id,
            }),
            token_header: None,
            idea_token: idea_token.map(str::to_owned),
            slot: None,
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

#[test]
fn d47_shadow_walk_recheck_summary_never_counts_a_check_that_did_not_run() {
    let mut summary = Domain47ShadowWalkRecheckSummary::default();
    summary.observe(
        Domain47ShadowWalkRecheckOutcome::NotChecked,
        fc_element(77, Domain47ShadowWalkMapAxis::Leader),
    );
    assert_eq!(summary, Domain47ShadowWalkRecheckSummary::default());
}

#[test]
fn d47_shadow_walk_owner_probe_names_an_uncensused_body_holding_the_entry() {
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
    assert_eq!(
        probe.roster_char_id, 0,
        "no roster entry, no roster char_id"
    );
}

#[test]
fn d47_shadow_walk_owner_probe_publishes_the_roster_id_of_a_censused_body() {
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

#[test]
fn d47_shadow_walk_owner_probe_reports_a_clean_container_without_the_entry() {
    let world = FcRecheckWorld::new(&[(7, vec![0], Vec::new())]);
    let target = world.role(0);
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

#[test]
fn d47_shadow_walk_recheck_report_counts_the_bodies_and_entries_it_examined() {
    let world = FcRecheckWorld::new(&[(7, vec![0, 0], Vec::new()), (7, vec![0, 0, 0], Vec::new())]);
    let target = world.role(0);
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
    assert!(domain47_advisor_probe_matches_owner(
        &advisor(0xA000),
        owner
    ));
    assert!(domain47_advisor_probe_matches_owner(
        &advisor(0x9000),
        owner
    ));
}

// OWNERSHIP fact. It is not by itself a promotion proof, because d47 does not

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

const FU_CHARACTERS: usize = 24;
const FU_CHARACTER_STRIDE: usize = 0x400;
const FU_ROLE_STRIDE: usize = 0x200;
const FU_NODE_STRIDE: usize = 0x80;
const FU_EMPTY_LEADER_CHARACTER: usize = 20;
const FU_EMPTY_ADVISOR_CHARACTER: usize = 21;

struct FuAcceptanceWorld {
    _arena: BenchArena,
    roots: ResolvedTypeRoots,
    roster: Vec<usize>,
    sweep_leader_hits: Vec<usize>,
    sweep_advisor_hits: Vec<usize>,
    leader_classes: BTreeMap<usize, FuRoleClass>,
    advisor_classes: BTreeMap<usize, FuRoleClass>,
}

struct FuPlannedRole {
    class: FuRoleClass,
    owner: usize,
    holder: Option<usize>,
    token: Option<String>,
    break_chain: bool,
}

fn fu_character_id(index: usize) -> u32 {
    1_000 + index as u32
}

fn fu_country_tag(index: usize) -> u32 {
    1 + (index % 7) as u32
}

fn fu_acceptance_world() -> FuAcceptanceWorld {
    let mut leaders: Vec<FuPlannedRole> = Vec::new();
    let mut advisors: Vec<FuPlannedRole> = Vec::new();

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

    for index in 0..8usize {
        leaders.push(FuPlannedRole {
            class: FuRoleClass::MapHeldLive,
            owner: index,
            holder: Some(index),
            token: None,
            break_chain: false,
        });
    }
    leaders.push(FuPlannedRole {
        class: FuRoleClass::MapHeldLive,
        owner: 3,
        holder: Some(3),
        token: None,
        break_chain: false,
    });
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
                "eco_advisor".to_string()
            } else {
                format!("advisor_token_{index}")
            }),
            break_chain: false,
        });
    }
    advisors.push(FuPlannedRole {
        class: FuRoleClass::MapHeldRefused,
        owner: 12,
        holder: Some(12),
        token: None,
        break_chain: false,
    });

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

    let mut leader_classes = BTreeMap::new();
    let mut advisor_classes = BTreeMap::new();
    let mut leader_entries: BTreeMap<usize, Vec<u64>> = BTreeMap::new();
    let mut advisor_entries: BTreeMap<usize, Vec<u64>> = BTreeMap::new();
    let mut sweep_leader_hits = Vec::new();
    let mut sweep_advisor_hits = Vec::new();
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

#[derive(Default)]
struct FuAdmitted {
    leader_roots: Vec<CountryLeaderRoleRoot>,
    advisor_roots: Vec<AdvisorRoot>,
    leader_objects: BTreeSet<u64>,
    advisor_objects: BTreeSet<u64>,
    leader_live: Domain47RoleLiveSet,
    advisor_live: Domain47RoleLiveSet,
    skipped_root_rows: u32,
    excluded_scan_hits: u32,
}

impl FuAdmitted {
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

fn sweep_hit_candidates(hits: &[usize]) -> Vec<Domain47WalkedRole> {
    hits.iter()
        .map(|&addr| Domain47WalkedRole { addr, node_addr: 0 })
        .collect()
}

fn fu_admit_by_sweep(world: &FuAcceptanceWorld) -> FuAdmitted {
    let mem = Domain47InProcessMemory;
    let mut out = FuAdmitted::default();
    let mut seen = BTreeSet::new();
    let mut by_identity = BTreeMap::new();
    page_cache_new_frame();
    domain47_admit_country_leader_roles(
        &mem,
        &world.roots,
        &sweep_hit_candidates(&world.sweep_leader_hits),
        Some(&mut out.leader_live),
        &mut seen,
        &mut out.leader_roots,
        &mut out.skipped_root_rows,
        &mut out.excluded_scan_hits,
    );
    domain47_admit_advisor_roots(
        &mem,
        &world.roots,
        &sweep_hit_candidates(&world.sweep_advisor_hits),
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

    assert!(
        sweep_leader_ids.contains(&(
            fu_country_tag(FU_EMPTY_LEADER_CHARACTER),
            fu_character_id(FU_EMPTY_LEADER_CHARACTER)
        )),
        "the fixture must actually stage the invented row"
    );
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
        (2, 0),
        "the second visit offers both edges AGAIN (each is separately \
         edge-re-validated, lane HT) and publishes NOTHING new: the \
         per-identity dedup holds"
    );
}

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
        &sweep_hit_candidates(&world.sweep_leader_hits),
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

#[test]
fn d47_admission_refuses_a_role_whose_map_edge_was_swapped_after_the_walk() {
    let world = fu_acceptance_world();
    let mem = Domain47InProcessMemory;
    let mut walk = Domain47ShadowWalk::default();
    let mut stack = Vec::new();
    let mut scratch = Vec::new();
    page_cache_new_frame();
    domain47_shadow_walk_character(world.roster[0], &mut walk, &mut stack, &mut scratch);
    let candidate = walk
        .fresh_advisors
        .first()
        .copied()
        .expect("character 0's advisor map holds a live object");
    assert_ne!(
        candidate.node_addr, 0,
        "the walk must carry the owning node"
    );

    let mut by_identity = BTreeMap::new();
    let mut out = Vec::new();
    let mut skipped = 0u32;
    let mut excluded = 0u32;
    domain47_admit_advisor_roots(
        &mem,
        &world.roots,
        &walk.fresh_advisors,
        None,
        &mut by_identity,
        &mut out,
        &mut skipped,
        &mut excluded,
    );
    assert_eq!(out.len(), 1, "control: the intact edge admits");
    assert_eq!(skipped, 0, "control: nothing tore");

    world._arena.put_u64(
        candidate.node_addr as usize + DOMAIN47_ADVISOR_NODE_VALUE_OFF,
        candidate.addr as u64 ^ 0x40,
    );
    let mut by_identity = BTreeMap::new();
    let mut out = Vec::new();
    let mut skipped = 0u32;
    let mut excluded = 0u32;
    page_cache_new_frame();
    domain47_admit_advisor_roots(
        &mem,
        &world.roots,
        &walk.fresh_advisors,
        None,
        &mut by_identity,
        &mut out,
        &mut skipped,
        &mut excluded,
    );
    assert_eq!(
        out.len(),
        0,
        "a role whose map edge no longer names it must not be adopted"
    );
    assert_eq!(
        skipped, 1,
        "and the refusal is torn-class, visible in skipped_root_rows"
    );

    assert!(domain47_role_edge_still_names(
        Domain47WalkedRole {
            addr: candidate.addr,
            node_addr: 0,
        },
        DOMAIN47_ADVISOR_NODE_VALUE_OFF,
    ));
}

#[test]
fn d47_admission_refuses_a_null_edge_that_gained_an_object() {
    let world = EjSingleCharacterWorld::new(1, 2);
    let roots = ResolvedTypeRoots::default();
    let advisor_nodes = world.arena.base + 0x4000;
    world.arena.put_u64(
        advisor_nodes + EJ_NODE_STRIDE + DOMAIN47_ADVISOR_NODE_VALUE_OFF,
        0,
    );
    let mem = Domain47InProcessMemory;
    let mut walk = Domain47ShadowWalk::default();
    let mut stack = Vec::new();
    let mut scratch = Vec::new();
    page_cache_new_frame();
    domain47_shadow_walk_character(world.character, &mut walk, &mut stack, &mut scratch);
    let null_candidates = walk
        .fresh_advisors
        .iter()
        .filter(|role| role.addr == 0)
        .count();
    assert_eq!(null_candidates, 1, "premise: the null edge is offered");

    let mut by_identity = BTreeMap::new();
    let mut out = Vec::new();
    let mut skipped = 0u32;
    let mut excluded = 0u32;
    domain47_admit_advisor_roots(
        &mem,
        &roots,
        &walk.fresh_advisors,
        None,
        &mut by_identity,
        &mut out,
        &mut skipped,
        &mut excluded,
    );
    assert_eq!(skipped, 0, "control: a still-null edge refuses nothing");

    world.arena.put_u64(
        advisor_nodes + EJ_NODE_STRIDE + DOMAIN47_ADVISOR_NODE_VALUE_OFF,
        (world.arena.base + 0xC000 + 0x40) as u64,
    );
    let mut by_identity = BTreeMap::new();
    let mut out = Vec::new();
    let mut skipped = 0u32;
    let mut excluded = 0u32;
    page_cache_new_frame();
    domain47_admit_advisor_roots(
        &mem,
        &roots,
        &walk.fresh_advisors,
        None,
        &mut by_identity,
        &mut out,
        &mut skipped,
        &mut excluded,
    );
    assert_eq!(
        skipped, 1,
        "a null edge that gained an object is a torn-class refusal, not a \
         silent omission"
    );
}

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
    for (call, feed) in [
        (
            "domain47_admit_country_leader_roles(",
            "&shadow_walk.fresh_leaders,",
        ),
        (
            "domain47_admit_advisor_roots(",
            "&shadow_walk.fresh_advisors,",
        ),
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

const FU_DEAD_ADVISORS: usize = 1_005;
const FU_DEAD_LEADERS: usize = 388;

struct FuCostWorld {
    _arena: BenchArena,
    roots: ResolvedTypeRoots,
    census_characters: Vec<usize>,
    sweep_leader_hits: Vec<usize>,
    sweep_advisor_hits: Vec<usize>,
}

fn fu_cost_world() -> FuCostWorld {
    let count = T32_CENSUS_CHARACTERS;
    let leader_objects = EJ_LEADER_ENTRIES + FU_DEAD_LEADERS;
    let advisor_objects = EJ_ADVISOR_ENTRIES + FU_DEAD_ADVISORS;
    let role_objects = leader_objects + advisor_objects;
    let node_slots =
        EJ_LEADER_ENTRIES + EJ_ADVISOR_ENTRIES + EJ_LEADER_CHARACTERS + EJ_ADVISOR_ENTRIES;
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
        arena.put_u64(
            addr + CCOUNTRY_LEADER_CHARACTER_PTR_OFF as usize,
            decoy as u64,
        );
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
        domain47_prefetch_object(addr, domain47::extractor::CCHARACTER_RAW_FIELDS_SPAN);
        if shadow_walk {
            domain47_shadow_walk_character(addr, &mut walk, &mut stack, &mut scratch);
        }
    }
    domain47_admit_country_leader_roles(
        &mem,
        &world.roots,
        &sweep_hit_candidates(&world.sweep_leader_hits),
        None,
        &mut seen,
        &mut leader_roots,
        &mut skipped,
        &mut excluded,
    );
    domain47_admit_advisor_roots(
        &mem,
        &world.roots,
        &sweep_hit_candidates(&world.sweep_advisor_hits),
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

#[test]
fn d47_walk_and_sweep_admit_the_same_rows_at_census_width() {
    let world = fu_cost_world();
    page_cache_new_frame();
    let walk = fu_cost_walk_arm(&world);
    page_cache_new_frame();
    let sweep = fu_cost_sweep_arm(&world, false);
    assert_eq!(
        walk.0, sweep.0,
        "the two discovery paths must publish the same COUNTRY-LEADER roots"
    );
    assert_eq!(walk.1, sweep.1, "and the same ADVISOR roots");
    assert_eq!(
        sweep,
        (EJ_LEADER_CHARACTERS, EJ_ADVISOR_ENTRIES),
        "and both must be the live width, not a truncated walk"
    );
}

#[test]
#[ignore = "measurement instrument; run explicitly with --ignored"]
fn d47_cost_walk_vs_sweep_discovery() {
    let world = fu_cost_world();
    page_cache_new_frame();
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
    let (shipped, walk) = time_us_median_ab_n(7, 3, || sweep_arm(true), walk_arm);
    let (sweep_only, walk_again) = time_us_median_ab_n(7, 3, || sweep_arm(false), walk_arm);
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

#[test]
fn d47_a_refused_leader_map_costs_only_the_leader_family() {
    let world = EjSingleCharacterWorld::new(3, 2);
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

#[test]
fn d47_a_clean_role_walk_costs_no_family() {
    let world = EjSingleCharacterWorld::new(3, 4);
    let walk = world.walk();
    assert_eq!(walk.refused, 0);
    assert_eq!(walk.unobserved_families(), 0);
    assert_eq!(walk.refused_first_map_axis(), None);
    assert_eq!(walk.refused_first_body, 0);
}

/// `node->value` once per node, so a duplicate would be a double free -- which
#[test]
fn d47_walk_refuses_a_map_that_yields_one_pointer_twice() {
    let world = EjSingleCharacterWorld::new(3, 2);
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

#[test]
fn d47_walk_refuses_a_map_that_reaches_one_null_node_twice() {
    let world = EjSingleCharacterWorld::new(3, 0);
    let node = |index: usize| world.leader_nodes + index * EJ_NODE_STRIDE;
    world
        .arena
        .put_u64(node(0) + DOMAIN47_TREE_NODE_LEFT_OFF, node(2) as u64);
    world
        .arena
        .put_u64(node(0) + DOMAIN47_TREE_NODE_RIGHT_OFF, node(2) as u64);
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
        3,
        "every edge is offered — two live and one null (lane HT: the null \
         edge is re-checked at admission time)"
    );
    assert_eq!(
        walk.fresh_leaders
            .iter()
            .filter(|role| role.addr == 0)
            .count(),
        1,
        "the null entry rides along as a null-edge candidate"
    );
    assert_eq!(
        walk.leaders.len(),
        2,
        "and contributes no pointer to the distinct set, while the other two do"
    );
    assert_eq!(walk.unobserved_families(), 0);
}

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
    assert_eq!(
        walk.fresh_leaders.len(),
        3,
        "one live edge and two null-edge candidates (lane HT)"
    );
    assert_eq!(walk.leaders.len(), 1);
}

#[test]
fn d47_walk_accepts_one_pointer_held_by_two_characters() {
    let first = EjSingleCharacterWorld::new(2, 0);
    let second = EjSingleCharacterWorld::new(2, 0);
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
        2,
        "every holder's edge is offered (lane HT) — the shared pointer's \
         second edge is separately re-validated at admission, while the \
         per-identity dedup still publishes the role once"
    );
    assert_eq!(
        walk.leaders.len(),
        3,
        "the distinct set counts the shared pointer once"
    );
}

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

#[test]
fn d47_the_prewarm_drops_the_rows_of_a_family_the_walk_refused() {
    let source = super::tests::collector_source_lf();
    let at = source
        .find("let role_walk_unobserved = shadow_walk.unobserved_families();")
        .expect("the prewarm consumes the walk's refusal mask");
    let block = &source[at..at + 900];
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
    assert!(
        block.contains("shadow_sweep_torn_hits = shadow_skipped;"),
        "the shadow's torn-read count must reach the wire, or a zero residual \
         taken off a prewarm that lost hits reads as a clear"
    );
}

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

    page_cache_new_frame();
    assert!(domain47_country_leader_identity(&mem, &world.roots, sweep_root).is_ok());
    assert!(domain47_country_leader_identity(&mem, &world.roots, walk_root).is_ok());

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
