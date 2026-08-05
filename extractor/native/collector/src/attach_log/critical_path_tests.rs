//! Output-equivalence pins for the d47/d29 critical-path reductions
//! (2026-08-03).
//!
//! Every test here exists to fail if one of those reductions starts changing
//! WHAT is captured rather than how fast it is captured. Each carries the
//! production mutation that breaks it.

use super::*;
use domain47::extractor::LaneMemory as Domain47LaneMemory;

// Test-local kernel32 bindings. Deliberately duplicated from the other
// test modules so this file is self-contained.
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
const CP_MEM_RESERVE: Dword = 0x2000;
const CP_MEM_RELEASE: Dword = 0x8000;
const CP_PAGE_READWRITE: Dword = 0x04;
const CP_PAGE_READONLY: Dword = 0x02;
const CP_PAGE: usize = 0x1000;

/// Byte-by-byte reference decode through the trait's ORIGINAL path:
/// N independent single-byte reads, each with its own region gate.
fn domain47_bytewise_reference<const N: usize>(addr: u64) -> Option<[u8; N]> {
    let mem = Domain47InProcessMemory;
    let mut out = [0u8; N];
    for (idx, byte) in out.iter_mut().enumerate() {
        *byte = mem.read_u8(addr.checked_add(idx as u64)?)?;
    }
    Some(out)
}

/// The bulk fast path must agree with the byte-by-byte path it replaced
/// on ordinary readable memory, and must still fail closed on an
/// unmapped address.
///
/// MUTATION: drop the `if read_into(..)` success test in
/// `Domain47InProcessMemory::read_array` and return the buffer
/// unconditionally — the unmapped assertions below start returning
/// `Some([0; N])` and this test fails.
#[test]
fn domain47_bulk_read_array_matches_bytewise_path() {
    page_cache_new_frame();
    let src: Vec<u8> = (0u8..=255).collect();
    let base = src.as_ptr() as u64;
    let mem = Domain47InProcessMemory;

    for off in [0u64, 1, 3, 7, 8, 64, 200] {
        let addr = base + off;
        assert_eq!(
            mem.read_array::<8>(addr),
            domain47_bytewise_reference::<8>(addr),
            "8-byte read @+{off}"
        );
        assert_eq!(
            mem.read_array::<4>(addr),
            domain47_bytewise_reference::<4>(addr),
            "4-byte read @+{off}"
        );
        assert_eq!(
            mem.read_array::<2>(addr),
            domain47_bytewise_reference::<2>(addr),
            "2-byte read @+{off}"
        );
        // And the decoded scalars the lane actually uses.
        let o = off as usize;
        assert_eq!(
            mem.read_u64_le(addr),
            Some(u64::from_le_bytes(src[o..o + 8].try_into().unwrap()))
        );
        assert_eq!(
            mem.read_u32_le(addr),
            Some(u32::from_le_bytes(src[o..o + 4].try_into().unwrap()))
        );
        assert_eq!(
            mem.read_i32_le(addr),
            Some(i32::from_le_bytes(src[o..o + 4].try_into().unwrap()))
        );
    }

    // Fail-closed on an unmapped address, exactly like the byte path.
    let bad = 0x1000u64;
    assert_eq!(mem.read_array::<8>(bad), None);
    assert_eq!(mem.read_array::<4>(bad), None);
    assert_eq!(mem.read_u64_le(bad), None);
    assert_eq!(domain47_bytewise_reference::<8>(bad), None);
}

/// THE case the pre-existing T1 note used to refuse a bulk swap: a range
/// that straddles two ADJACENT committed+readable regions. Byte-by-byte
/// accepts it (every byte sits in a confirmed region); the whole-range
/// single-region gate rejects it. The fallback is what keeps the two
/// paths equivalent, so this is the test that pins the fallback.
///
/// MUTATION: delete the byte-by-byte fallback loop from
/// `Domain47InProcessMemory::read_array` (return `None` when `read_into`
/// fails) — the straddling read below becomes `None` while the reference
/// path still yields the bytes, and this test fails.
#[test]
fn domain47_bulk_read_array_falls_back_across_a_region_boundary() {
    page_cache_new_frame();
    unsafe {
        let base = VirtualAlloc(
            ptr::null_mut(),
            2 * CP_PAGE,
            CP_MEM_RESERVE,
            CP_PAGE_READWRITE,
        );
        assert!(!base.is_null(), "reserve failed");
        let committed = VirtualAlloc(base, 2 * CP_PAGE, MEM_COMMIT, CP_PAGE_READWRITE);
        assert_eq!(committed, base, "commit failed");
        let bytes = base as *mut u8;
        for i in 0..(2 * CP_PAGE) {
            *bytes.add(i) = (i & 0xff) as u8;
        }
        // Re-protecting the SECOND page splits the allocation into two
        // VirtualQuery regions that are both committed and readable.
        let mut old: Dword = 0;
        let ok = VirtualProtect(
            (base as usize + CP_PAGE) as *mut c_void,
            CP_PAGE,
            CP_PAGE_READONLY,
            &mut old,
        );
        assert_ne!(ok, 0, "VirtualProtect split failed");
        page_cache_new_frame();

        // Confirm the split really happened: the whole-range gate must
        // reject a straddling range, or this test proves nothing.
        let straddle = base as usize + CP_PAGE - 4;
        assert!(
            !range_confirmed_good(straddle, 8),
            "expected two distinct regions; test fixture did not split"
        );

        let mem = Domain47InProcessMemory;
        let expect = {
            let mut a = [0u8; 8];
            for (i, b) in a.iter_mut().enumerate() {
                *b = ((CP_PAGE - 4 + i) & 0xff) as u8;
            }
            a
        };
        assert_eq!(
            domain47_bytewise_reference::<8>(straddle as u64),
            Some(expect),
            "byte-by-byte reference must accept the cross-region range"
        );
        assert_eq!(
            mem.read_array::<8>(straddle as u64),
            Some(expect),
            "bulk path must fall back and produce the same bytes"
        );
        assert_eq!(
            mem.read_u64_le(straddle as u64),
            Some(u64::from_le_bytes(expect))
        );

        let _ = VirtualFree(base, 0, CP_MEM_RELEASE);
    }
}

/// The naval-OOB window hoists (`validate_compact_task_force_body`,
/// `collect_compact_country_fleet_roots`, both CShip window sites) are
/// only sound because the readers that consume those windows reject a
/// non-`is_map_control_heap_pointer` body BEFORE they read a single
/// window field. This pins that leading rejection with a body that is
/// perfectly readable and whose bytes would otherwise validate.
///
/// MUTATION: delete `is_map_control_heap_pointer(body)` from
/// `validate_oob_body_with_window` or from
/// `read_cship_identity_with_window` — the fixtures below start
/// validating (VERIFIED: the OOB assertion returns `Some(7)`), which is
/// exactly the condition under which the hoists would start dropping
/// candidates the old code accepted. Merely REORDERING the check below
/// the field reads does NOT break this test, and correctly so: the
/// hoists depend on the rejection existing, not on where it sits.
#[test]
fn oob_and_ship_validators_reject_non_heap_bodies_before_reading_fields() {
    page_cache_new_frame();
    unsafe {
        // The fixtures must be READABLE but OUTSIDE [HEAP_LO, HEAP_HI),
        // so a validator that read fields first would accept them. The
        // process allocator hands out addresses inside that window, so
        // reserve below it explicitly.
        const SIZE: usize = 4 * CP_PAGE;
        let mut base_ptr: *mut c_void = ptr::null_mut();
        for slot in 1..32usize {
            let want = (slot * 0x1000_0000) as *mut c_void;
            let got =
                VirtualAlloc(want, SIZE, MEM_COMMIT | CP_MEM_RESERVE, CP_PAGE_READWRITE);
            if !got.is_null() && !is_map_control_heap_pointer(got as usize) {
                base_ptr = got;
                break;
            }
            if !got.is_null() {
                let _ = VirtualFree(got, 0, CP_MEM_RELEASE);
            }
        }
        assert!(
            !base_ptr.is_null(),
            "could not reserve a fixture below HEAP_LO"
        );
        let base = base_ptr as usize;
        assert!(!is_map_control_heap_pointer(base));

        // A CFleet-shaped object whose vtable and handle would validate
        // against module_base 0.
        let fleet_addr = base;
        ptr::write_unaligned(fleet_addr as *mut u64, FLEET_VTABLE_RVA as u64);
        let handle = (7u64 << 32) | u64::from(OOB_TYPE_FLEET);
        ptr::write_unaligned((fleet_addr + 8) as *mut u64, handle);
        page_cache_new_frame();
        assert_eq!(
            read_u64(fleet_addr),
            Some(FLEET_VTABLE_RVA as u64),
            "fixture must be readable, or this test proves nothing"
        );
        assert_eq!(
            validate_oob_body_with_window::<0>(
                0,
                fleet_addr,
                FLEET_VTABLE_RVA,
                OOB_TYPE_FLEET,
                None
            ),
            None,
            "non-heap body must be rejected before any field read"
        );
        assert!(matches!(
            validate_compact_task_force_body(0, fleet_addr, &BTreeMap::new(), true),
            // A non-heap body never reaches the identity gate, so it is not a
            // task force at all -- NOT a rejected one. Asserting the exact arm
            // is what keeps the hoist from turning blob noise into a reported
            // partial (see `CompactTaskForceCandidate`).
            CompactTaskForceCandidate::NotATaskForce
        ));

        // Same for the CShip identity reader, whose window is the
        // 0x714-byte one the ship-loop hoists skip.
        let ship_addr = base + 2 * CP_PAGE;
        ptr::write_unaligned(ship_addr as *mut u64, CSHIP_VTABLE_RVA as u64);
        let ship_handle = (11u64 << 32) | u64::from(CSHIP_TYPE);
        ptr::write_unaligned((ship_addr + CSHIP_OFF_HANDLE) as *mut u64, ship_handle);
        page_cache_new_frame();
        assert_eq!(
            read_u64(ship_addr),
            Some(CSHIP_VTABLE_RVA as u64),
            "ship fixture must be readable"
        );
        assert_eq!(
            read_cship_identity_with_window(0, ship_addr, None),
            None,
            "non-heap CShip body must be rejected before any field read"
        );
        assert_eq!(read_cship_instance_id_with_window(0, ship_addr, None), None);
        assert!(read_cship_scalars_with_window(0, ship_addr, None).is_none());

        let _ = VirtualFree(base_ptr, 0, CP_MEM_RELEASE);
    }
}

/// `domain47_country_ruler_strings` replaced a full
/// `read_country_politics` call whose party vector, active-idea vector
/// and election frequency D47 threw away. This drives BOTH functions
/// over one live fake country and asserts D47's three extracted values
/// are identical — including the case where the country has ideas (so
/// `has_any()` is true) but no resolvable ruling party or leader.
///
/// MUTATION: make `domain47_country_ruler_strings` return the ideology
/// group of the FIRST party instead of the RULING party (i.e. call
/// `read_political_parties_with_window` and take `party_ptrs[0]`), or
/// drop the `ruling_party.as_deref()` argument to
/// `read_active_country_leader` — the triples diverge and this fails.
#[test]
fn domain47_ruler_strings_match_the_shared_politics_reader() {
    page_cache_new_frame();
    unsafe {
        // The politics chain gates every pointer through
        // `is_map_control_heap_pointer`, so the fixture must live inside
        // [HEAP_LO, HEAP_HI). Reserve at a fixed high base.
        const SIZE: usize = 16 * CP_PAGE;
        let mut base_ptr: *mut c_void = ptr::null_mut();
        for slot in 0..8usize {
            let want = (HEAP_LO + 0x1000_0000 + slot * SIZE) as *mut c_void;
            let got = VirtualAlloc(want, SIZE, MEM_COMMIT | CP_MEM_RESERVE, CP_PAGE_READWRITE);
            if !got.is_null() {
                base_ptr = got;
                break;
            }
        }
        assert!(
            !base_ptr.is_null(),
            "could not reserve a fixture inside the heap window"
        );
        let base = base_ptr as usize;
        assert!(is_map_control_heap_pointer(base));
        let write_u64 = |off: usize, value: u64| {
            ptr::write_unaligned((base + off) as *mut u64, value);
        };

        // Layout inside the fixture (all offsets are fixture-local):
        //   0x0000 CCountry
        //   0x1000 CPoliticalStatus
        //   0x2000 CPoliticalParty (the ruling party)
        //   0x3000 CIdeologyGroup
        //   0x4000 ideology-group token string bytes
        //   0x5000 ruling-party leader vector (one CCountryLeader*)
        //   0x6000 CCountryLeader
        //   0x7000 CCharacter (the active ruler)
        //   0x8000 ruler display-name string bytes
        //   0x9000 ruler token string bytes
        const COUNTRY: usize = 0x0000;
        const STATUS: usize = 0x1000;
        const PARTY: usize = 0x2000;
        const GROUP: usize = 0x3000;
        const TOKEN_BYTES: usize = 0x4000;
        const LEADER_VEC: usize = 0x5000;
        const COUNTRY_LEADER: usize = 0x6000;
        const CHARACTER: usize = 0x7000;
        const RULER_NAME_BYTES: usize = 0x8000;
        const RULER_TOKEN_BYTES: usize = 0x9000;

        // A country_vtable that keeps module_base inside the fixture is
        // not required: the politics reader derives module_base by
        // subtracting COUNTRY_VTABLE_RVA and only uses it to form other
        // vtable constants, which are compared against stored values.
        let module_base = base;
        let country_vtable = module_base + COUNTRY_VTABLE_RVA;

        write_u64(COUNTRY, country_vtable as u64);
        write_u64(COUNTRY + OBJ_POLITICAL_STATUS, (base + STATUS) as u64);
        write_u64(STATUS, (module_base + POLITICAL_STATUS_VTABLE_RVA) as u64);
        write_u64(STATUS + POLITICAL_STATUS_COUNTRY_BACKLINK, (base + COUNTRY) as u64);
        write_u64(STATUS + POLITICAL_STATUS_RULING_PARTY, (base + PARTY) as u64);
        write_u64(PARTY, (module_base + POLITICAL_PARTY_VTABLE_RVA) as u64);
        write_u64(PARTY + POLITICAL_PARTY_IDEOLOGY_GROUP, (base + GROUP) as u64);
        write_u64(GROUP, (module_base + IDEOLOGY_GROUP_VTABLE_RVA) as u64);
        // MSVC std::string for the ideology-group token: a long-string
        // header (cap >= 16) pointing at the byte block.
        let token = b"democratic";
        ptr::copy_nonoverlapping(
            token.as_ptr(),
            (base + TOKEN_BYTES) as *mut u8,
            token.len(),
        );
        *((base + TOKEN_BYTES + token.len()) as *mut u8) = 0;
        write_u64(GROUP + IDEOLOGY_GROUP_ID, (base + TOKEN_BYTES) as u64);
        write_u64(GROUP + IDEOLOGY_GROUP_ID + 0x10, token.len() as u64);
        write_u64(GROUP + IDEOLOGY_GROUP_ID + 0x18, 31);

        // Active-ruler chain. Without it `leader` stays None on BOTH
        // paths and the ruler_name/ruler_token halves of the projection
        // would be compared vacuously (round-1 review finding).
        // Singleton leader vector: {data, packed size|capacity == 1|1}.
        write_u64(PARTY + POLITICAL_PARTY_LEADER_VEC, (base + LEADER_VEC) as u64);
        write_u64(
            PARTY + POLITICAL_PARTY_LEADER_VEC + 8,
            (1u64 << 32) | 1u64,
        );
        write_u64(LEADER_VEC, (base + COUNTRY_LEADER) as u64);
        write_u64(COUNTRY_LEADER, (module_base + COUNTRY_LEADER_VTABLE_RVA) as u64);
        write_u64(
            COUNTRY_LEADER + COUNTRY_LEADER_CHARACTER_PTR,
            (base + CHARACTER) as u64,
        );
        write_u64(CHARACTER, (module_base + POLITICS_CHARACTER_VTABLE_RVA) as u64);
        ptr::write_unaligned(
            (base + CHARACTER + POLITICS_CHARACTER_OFF_TYPE) as *mut u32,
            POLITICS_CHARACTER_TYPE,
        );
        ptr::write_unaligned(
            (base + CHARACTER + POLITICS_CHARACTER_OFF_ID) as *mut u32,
            1458u32,
        );
        let ruler_name = b"Maxwell Aitken";
        ptr::copy_nonoverlapping(
            ruler_name.as_ptr(),
            (base + RULER_NAME_BYTES) as *mut u8,
            ruler_name.len(),
        );
        *((base + RULER_NAME_BYTES + ruler_name.len()) as *mut u8) = 0;
        write_u64(
            CHARACTER + POLITICS_CHARACTER_OFF_DISPLAY,
            (base + RULER_NAME_BYTES) as u64,
        );
        write_u64(
            CHARACTER + POLITICS_CHARACTER_OFF_DISPLAY + 0x10,
            ruler_name.len() as u64,
        );
        write_u64(CHARACTER + POLITICS_CHARACTER_OFF_DISPLAY + 0x18, 31);
        let ruler_token = b"ENG_maxwell_aitken";
        ptr::copy_nonoverlapping(
            ruler_token.as_ptr(),
            (base + RULER_TOKEN_BYTES) as *mut u8,
            ruler_token.len(),
        );
        *((base + RULER_TOKEN_BYTES + ruler_token.len()) as *mut u8) = 0;
        write_u64(
            CHARACTER + POLITICS_CHARACTER_OFF_TOKEN,
            (base + RULER_TOKEN_BYTES) as u64,
        );
        write_u64(
            CHARACTER + POLITICS_CHARACTER_OFF_TOKEN + 0x10,
            ruler_token.len() as u64,
        );
        write_u64(CHARACTER + POLITICS_CHARACTER_OFF_TOKEN + 0x18, 31);

        page_cache_new_frame();

        let shared = read_country_politics(base + COUNTRY, country_vtable, None, true);
        let shared_triple = (
            shared.as_ref().and_then(|row| row.ruling_party.clone()),
            shared
                .as_ref()
                .and_then(|row| row.leader.as_ref())
                .and_then(|leader| leader.name.clone()),
            shared
                .as_ref()
                .and_then(|row| row.leader.as_ref())
                .and_then(|leader| leader.token.clone()),
        );
        let lean = domain47_country_ruler_strings(country_vtable, base + COUNTRY);
        assert_eq!(
            lean, shared_triple,
            "D47's lean ruler read must equal what it used to take out of read_country_politics"
        );
        // The fixture must actually exercise ALL THREE projected values,
        // otherwise an all-None comparison would pass vacuously — a
        // mutation that hard-coded `ruler_name`/`ruler_token` to None
        // would survive (round-1 review finding).
        assert_eq!(
            lean.0.as_deref(),
            Some("democratic"),
            "fixture did not resolve a ruling party"
        );
        assert_eq!(
            lean.1.as_deref(),
            Some("Maxwell Aitken"),
            "fixture did not resolve a ruler display name"
        );
        assert_eq!(
            lean.2.as_deref(),
            Some("ENG_maxwell_aitken"),
            "fixture did not resolve a ruler token"
        );

        // Now break the ruling-party pointer only: both paths must
        // collapse to the same all-absent triple.
        write_u64(STATUS + POLITICAL_STATUS_RULING_PARTY, 0);
        page_cache_new_frame();
        let shared = read_country_politics(base + COUNTRY, country_vtable, None, true);
        let shared_triple = (
            shared.as_ref().and_then(|row| row.ruling_party.clone()),
            shared
                .as_ref()
                .and_then(|row| row.leader.as_ref())
                .and_then(|leader| leader.name.clone()),
            shared
                .as_ref()
                .and_then(|row| row.leader.as_ref())
                .and_then(|leader| leader.token.clone()),
        );
        assert_eq!(
            domain47_country_ruler_strings(country_vtable, base + COUNTRY),
            shared_triple
        );
        assert_eq!(shared_triple, (None, None, None));

        let _ = VirtualFree(base_ptr, 0, CP_MEM_RELEASE);
    }
}

/// The negative half of the naval hoist proof shows the hoisted
/// predicate never accepts more than the validators did. This is the
/// POSITIVE half (round-1 review finding: a hoist that rejected ALL
/// pointers would drop every ship row and still pass the negative test).
/// It drives a real fleet/task-force/ship graph, living INSIDE
/// [HEAP_LO, HEAP_HI) so the hoisted predicate accepts it, through all
/// four hoisted sites and checks the rows that come out.
///
/// MUTATION: tighten either EARLY-EXIT hoist — the leading check in
/// `validate_compact_task_force_body`, or the `continue` in
/// `collect_compact_country_fleet_roots` — to, say,
/// `is_heap_pointer_aligned(_, 0x1000)`. VERIFIED: the task-force body
/// then fails to validate and this test fails ("heap task-force body
/// must still validate after the hoist"). The fixture is deliberately
/// NOT page-aligned so that mutation bites.
///
/// Measured while checking that mutation, and worth recording: the two
/// CSHIP `.then(..).flatten()` sites CANNOT drop a row even under a
/// wrongly-tightened predicate. A `None` window makes
/// `snapshot_*_or_read` fall back to LIVE per-field reads, which
/// succeed, so the same rows come out — only the read shape changes.
/// Row loss is therefore confined to the two early-exit sites, which is
/// what this test pins.
#[test]
fn hoisted_naval_sites_still_accept_and_emit_for_heap_bodies() {
    page_cache_new_frame();
    unsafe {
        const SIZE: usize = 32 * CP_PAGE;
        let mut base_ptr: *mut c_void = ptr::null_mut();
        for slot in 0..8usize {
            let want = (HEAP_LO + 0x2000_0000 + slot * SIZE) as *mut c_void;
            let got =
                VirtualAlloc(want, SIZE, MEM_COMMIT | CP_MEM_RESERVE, CP_PAGE_READWRITE);
            if !got.is_null() {
                base_ptr = got;
                break;
            }
        }
        assert!(
            !base_ptr.is_null(),
            "could not reserve a fixture inside the heap window"
        );
        let base = base_ptr as usize;
        assert!(is_map_control_heap_pointer(base));
        let write_u64 = |off: usize, value: u64| {
            ptr::write_unaligned((base + off) as *mut u64, value);
        };
        let write_u32 = |off: usize, value: u32| {
            ptr::write_unaligned((base + off) as *mut u32, value);
        };

        // module_base 0 keeps the pinned vtable RVAs usable verbatim as
        // the stored vtable values.
        const MODULE_BASE: usize = 0;
        // Deliberately NOT page-aligned: real engine objects are heap
        // allocations, and a page-aligned fixture would survive a
        // hoist predicate tightened to `is_heap_pointer_aligned(_,
        // 0x1000)` (verified: that mutation passes against an aligned
        // fixture, fails against this one).
        const FLEET: usize = 0x1010;
        const TASK_FORCE: usize = 0x2020;
        const SHIP_PTRS: usize = 0x3030;
        const SHIP_A: usize = 0x4040;
        const SHIP_B: usize = 0x8050;
        const DEF_BYTES: usize = 0xC060;

        write_u64(FLEET, FLEET_VTABLE_RVA as u64);
        write_u64(FLEET + 0x08, (5u64 << 32) | u64::from(OOB_TYPE_FLEET));
        write_u64(TASK_FORCE, TASK_FORCE_VTABLE_RVA as u64);
        write_u64(TASK_FORCE + 0x08, (9u64 << 32) | u64::from(OOB_TYPE_FLEET));
        write_u64(TASK_FORCE + TASK_FORCE_OFF_FLEET_PARENT, (base + FLEET) as u64);
        write_u32(TASK_FORCE + TASK_FORCE_OFF_SHIP_COUNT, 2);
        write_u64(TASK_FORCE + TASK_FORCE_OFF_SHIP_BEGIN, (base + SHIP_PTRS) as u64);
        write_u64(SHIP_PTRS, (base + SHIP_A) as u64);
        write_u64(SHIP_PTRS + 8, (base + SHIP_B) as u64);

        let definition = b"light_cruiser_1";
        ptr::copy_nonoverlapping(
            definition.as_ptr(),
            (base + DEF_BYTES) as *mut u8,
            definition.len(),
        );
        *((base + DEF_BYTES + definition.len()) as *mut u8) = 0;
        for (slot, instance_id, strength) in [(SHIP_A, 4001u32, 900u32), (SHIP_B, 4002, 800)]
        {
            write_u64(slot, CSHIP_VTABLE_RVA as u64);
            write_u64(
                slot + CSHIP_OFF_HANDLE,
                (u64::from(instance_id) << 32) | u64::from(CSHIP_TYPE),
            );
            write_u32(slot + CSHIP_OFF_SUNK_CONVOYS, 3);
            write_u32(slot + CSHIP_OFF_CATEGORY, 5); // light_cruiser
            write_u32(slot + CSHIP_OFF_STRENGTH, strength);
            write_u32(slot + CSHIP_OFF_ORGANISATION, 700);
            write_u32(slot + CSHIP_OFF_EXPERIENCE, 12);
            write_u32(slot + CSHIP_OFF_REF100, CSHIP_REF100_RAW);
            write_u64(slot + CSHIP_OFF_DEFINITION, (base + DEF_BYTES) as u64);
            write_u64(slot + CSHIP_OFF_DEFINITION + 0x10, definition.len() as u64);
            write_u64(slot + CSHIP_OFF_DEFINITION + 0x18, 31);
        }
        page_cache_new_frame();

        // Site 1+2: the task-force body validator (whose window read is
        // now hoisted behind the heap check) and the ship-member reader
        // inside it.
        let mut fleet_roots = BTreeMap::new();
        fleet_roots.insert(
            base + FLEET,
            CompactFleetRoot {
                fleet_id: 5,
                body_ptr: base + FLEET,
                owner_tag_db_id: Some(2),
            },
        );
        let CompactTaskForceCandidate::Accepted {
            task_force_id,
            fleet_root,
            ship_members,
            ..
        } = validate_compact_task_force_body(MODULE_BASE, base + TASK_FORCE, &fleet_roots, true)
        else {
            panic!("heap task-force body must still validate after the hoist");
        };
        assert_eq!(task_force_id, 9, "task force id");
        assert_eq!(fleet_root.fleet_id, 5, "parent fleet");
        assert_eq!(
            ship_members.ids.as_deref(),
            Some([4001u32, 4002].as_slice()),
            "ship members must still be read through the hoisted ship-window site"
        );

        // Site 3: the ship sampler.
        let rows = sample_compact_task_force_ships_with_windows(
            MODULE_BASE,
            base + TASK_FORCE,
            9,
            Some(2),
            true,
        )
        .expect("heap ship pointers must still sample after the hoist");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].instance_id, 4001);
        assert_eq!(rows[0].definition.as_deref(), Some("light_cruiser_1"));
        assert_eq!(rows[0].strength_raw, 900);
        assert_eq!(rows[0].sunk_convoys, 3);
        assert_eq!(rows[0].task_force_id, Some(9));
        assert_eq!(rows[0].owner_tag_db_id, Some(2));
        assert_eq!(rows[1].instance_id, 4002);
        assert_eq!(rows[1].strength_raw, 800);

        // Site 4: the fleet-root loop's `continue` hoist. Round-2 review
        // finding: an earlier version of this block re-implemented the
        // validation by hand instead of calling the production loop, so
        // tightening the loop's own predicate would have gone unnoticed.
        // Drive `collect_compact_country_fleet_roots` itself.
        const COUNTRY: usize = 0xD070;
        const FLEET_PTRS: usize = 0xE080;
        write_u64(COUNTRY + COUNTRY_FLEET_VECTOR_DATA, (base + FLEET_PTRS) as u64);
        write_u32(COUNTRY + COUNTRY_FLEET_VECTOR_CAPACITY, 1);
        write_u32(COUNTRY + COUNTRY_FLEET_VECTOR_COUNT, 1);
        write_u64(FLEET_PTRS, (base + FLEET) as u64);
        page_cache_new_frame();

        let country_window =
            compact_object_window::<COUNTRY_FLEET_WINDOW_LEN>(base + COUNTRY, true);
        let mut fleet_rows: BTreeMap<u32, CompactFleetRoot> = BTreeMap::new();
        let readable = collect_compact_country_fleet_roots(
            MODULE_BASE,
            base + COUNTRY,
            Some(2),
            &mut fleet_rows,
            country_window.as_ref(),
            true,
        );
        assert!(readable, "the country's fleet vector must still be readable");
        assert_eq!(fleet_rows.len(), 1, "the heap fleet root must still be adopted");
        let fleet_root = fleet_rows.get(&5).expect("fleet id 5");
        assert_eq!(fleet_root.body_ptr, base + FLEET);
        assert_eq!(fleet_root.owner_tag_db_id, Some(2));

        let _ = VirtualFree(base_ptr, 0, CP_MEM_RELEASE);
    }
}
