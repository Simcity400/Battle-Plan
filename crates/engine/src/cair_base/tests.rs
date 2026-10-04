use super::*;
use crate::memory::SliceMemory;

const IMAGE: usize = 0x140000000;
const GAME: usize = 0x1000;
const MANAGER: usize = 0x2000;
const STATE_TABLE: usize = 0x2400;
const HOST_TABLE: usize = 0x2500;
const STATE_BASE: usize = 0x3000;
const HOST_BASE: usize = 0x3200;
const STATE_OBJECT: usize = 0x4000;
const PROVINCE_OBJECT: usize = 0x5000;

fn qword(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at - GAME..at - GAME + 8].copy_from_slice(&value.to_le_bytes());
}

fn dword(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at - GAME..at - GAME + 4].copy_from_slice(&value.to_le_bytes());
}

fn array(bytes: &mut [u8], at: usize, data: usize, capacity: u32, count: u32) {
    qword(bytes, at, data as u64);
    dword(bytes, at + 8, capacity);
    dword(bytes, at + 12, count);
}

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x5000];
    qword(
        &mut bytes,
        GAME + cgame_state::STRATEGIC_AIR_MANAGER,
        MANAGER as u64,
    );
    qword(
        &mut bytes,
        MANAGER,
        (IMAGE + cstrategic_air_manager::VTABLE_RVA) as u64,
    );
    array(
        &mut bytes,
        MANAGER + cstrategic_air_manager::STATE_AIR_BASES,
        STATE_TABLE,
        4,
        2,
    );
    array(
        &mut bytes,
        MANAGER + cstrategic_air_manager::STATELESS_AIR_BASES,
        HOST_TABLE,
        2,
        1,
    );
    qword(&mut bytes, STATE_TABLE + 8, STATE_BASE as u64);
    qword(&mut bytes, HOST_TABLE, HOST_BASE as u64);
    for (pointer, id) in [(STATE_BASE, 7_u64), (HOST_BASE, 9)] {
        qword(&mut bytes, pointer, (IMAGE + VTABLE_RVA) as u64);
        qword(&mut bytes, pointer + HANDLE, (id << 32) | u64::from(TYPE));
    }
    qword(&mut bytes, STATE_BASE + STATE, STATE_OBJECT as u64);
    qword(&mut bytes, STATE_BASE + PROVINCE, PROVINCE_OBJECT as u64);
    dword(&mut bytes, STATE_BASE + CAPACITY, 1200);
    qword(
        &mut bytes,
        STATE_OBJECT,
        (IMAGE + cstate::VTABLE_RVA) as u64,
    );
    dword(&mut bytes, STATE_OBJECT + cstate::ID, 1);
    qword(
        &mut bytes,
        PROVINCE_OBJECT,
        (IMAGE + cprovince::VTABLE_RVA) as u64,
    );
    dword(&mut bytes, PROVINCE_OBJECT + cprovince::ID, 14);
    qword(
        &mut bytes,
        PROVINCE_OBJECT + cprovince::STATE,
        STATE_OBJECT as u64,
    );
    qword(&mut bytes, HOST_BASE + HOST_REFERENCE, (123_u64 << 32) | 51);
    dword(&mut bytes, HOST_BASE + CAPACITY, (-17_i32) as u32);
    dword(&mut bytes, HOST_BASE + KIND, 1);
    bytes
}

#[test]
fn state_holes_and_dense_hosts_preserve_native_fields_and_count() {
    let bytes = fixture();
    let memory = SliceMemory {
        base: GAME,
        bytes: &bytes,
    };
    let mut output = [Row::default(); 4];
    assert_eq!(
        walk(&memory, IMAGE, GAME, &mut output),
        Outcome::Rows { len: 2 }
    );
    assert_eq!(
        output[0],
        Row {
            id: (7_u64 << 32) | 65,
            state: Some(1),
            province: Some(14),
            host: 0,
            capacity: 1200,
            kind: 0,
        }
    );
    assert_eq!(
        output[1],
        Row {
            id: (9_u64 << 32) | 65,
            state: None,
            province: None,
            host: (123_u64 << 32) | 51,
            capacity: -17,
            kind: 1,
        }
    );
    assert_eq!(
        walk(&memory, IMAGE, GAME, &mut output[..1]),
        Outcome::Truncated {
            len: 1,
            unread_rows: 1,
        }
    );
    assert_eq!(
        walk(&memory, IMAGE, GAME, &mut []),
        Outcome::Truncated {
            len: 0,
            unread_rows: 2,
        }
    );
}

#[test]
fn malformed_membership_and_failed_rows_do_not_become_absence() {
    let mut bytes = fixture();
    let mut output = [Row::default(); 4];
    qword(&mut bytes, HOST_TABLE, 0);
    assert!(matches!(
        walk(
            &SliceMemory {
                base: GAME,
                bytes: &bytes
            },
            IMAGE,
            GAME,
            &mut output
        ),
        Outcome::Unavailable {
            reason: "stateless air base roster contains null base"
        }
    ));
    qword(&mut bytes, HOST_TABLE, HOST_BASE as u64);
    dword(&mut bytes, STATE_OBJECT + cstate::ID, 3);
    assert!(matches!(
        walk(
            &SliceMemory {
                base: GAME,
                bytes: &bytes
            },
            IMAGE,
            GAME,
            &mut output
        ),
        Outcome::Unavailable {
            reason: "air base state differs from roster slot"
        }
    ));
    dword(&mut bytes, STATE_OBJECT + cstate::ID, 1);
    qword(&mut bytes, PROVINCE_OBJECT + cprovince::STATE, 0);
    assert!(matches!(
        walk(
            &SliceMemory {
                base: GAME,
                bytes: &bytes
            },
            IMAGE,
            GAME,
            &mut output
        ),
        Outcome::Unavailable {
            reason: "air base province belongs to another state"
        }
    ));
}

const COUNTRY_TABLE: usize = 0x2800;
const LINK_TABLE: usize = 0x2840;
const COUNTRY_A: usize = 0x4100;
const COUNTRY_B: usize = 0x4200;
const CONTAINER_TABLE: usize = 0x2600;
const CONTAINER_A: usize = 0x3400;
const CONTAINER_B: usize = 0x3600;
const POOL_TABLE_A: usize = 0x2700;
const POOL_TABLE_B: usize = 0x2740;
const POOL_A: usize = 0x3800;
const POOL_B: usize = 0x3900;
const WING_TABLE_A: usize = 0x2780;
const WING_TABLE_B: usize = 0x27c0;
const WING_A: usize = 0x3a00;
const WING_B: usize = 0x3b00;

fn wing(bytes: &mut [u8], at: usize, handle: u64) {
    qword(bytes, at, (IMAGE + cair_wing::VTABLE_RVA) as u64);
    qword(
        bytes,
        at + cair_wing::SECONDARY,
        (IMAGE + cair_wing::SECONDARY_VTABLE_RVA) as u64,
    );
    qword(bytes, at + cair_wing::REFERENCE_TYPE, handle);
}

/// The state base carries two containers: reserved slot zero, which the
/// draw never selects but which the engine can still create, and slot two
/// with two pools. The host base carries no container at all.
fn registration_fixture() -> Vec<u8> {
    let mut bytes = fixture();
    // Two canonical countries: id 5 at slot 1 and id 9 at slot 2.
    array(
        &mut bytes,
        GAME + cgame_state::COUNTRIES,
        COUNTRY_TABLE,
        3,
        3,
    );
    array(
        &mut bytes,
        GAME + cgame_state::COUNTRY_LINKS,
        LINK_TABLE,
        10,
        10,
    );
    qword(&mut bytes, COUNTRY_TABLE + 8, COUNTRY_A as u64);
    qword(&mut bytes, COUNTRY_TABLE + 16, COUNTRY_B as u64);
    for (link, slot) in [(1_usize, 1_u32), (5, 1), (2, 2), (9, 2)] {
        dword(&mut bytes, LINK_TABLE + link * 4, slot);
    }
    for (pointer, id) in [(COUNTRY_A, 5_u32), (COUNTRY_B, 9)] {
        qword(&mut bytes, pointer, (IMAGE + ccountry::VTABLE_RVA) as u64);
        dword(&mut bytes, pointer + ccountry::ID, id);
    }
    array(
        &mut bytes,
        STATE_BASE + COUNTRY_CONTAINERS,
        CONTAINER_TABLE,
        4,
        4,
    );
    qword(&mut bytes, CONTAINER_TABLE, CONTAINER_A as u64);
    qword(&mut bytes, CONTAINER_TABLE + 16, CONTAINER_B as u64);
    for (pointer, country, pools, count) in [
        (CONTAINER_A, 0_u32, POOL_TABLE_A, 1_u32),
        (CONTAINER_B, 9, POOL_TABLE_B, 1),
    ] {
        qword(
            &mut bytes,
            pointer,
            (IMAGE + ccountry_air_container::VTABLE_RVA) as u64,
        );
        dword(
            &mut bytes,
            pointer + ccountry_air_container::COUNTRY,
            country,
        );
        array(
            &mut bytes,
            pointer + ccountry_air_container::POOLS,
            pools,
            count,
            count,
        );
    }
    qword(&mut bytes, POOL_TABLE_A, POOL_A as u64);
    qword(&mut bytes, POOL_TABLE_B, POOL_B as u64);
    for (pool, table, count) in [(POOL_A, WING_TABLE_A, 2_u32), (POOL_B, WING_TABLE_B, 0)] {
        qword(
            &mut bytes,
            pool,
            (IMAGE + cair_wing_pool::VTABLE_RVA) as u64,
        );
        array(
            &mut bytes,
            pool + cair_wing_pool::WINGS,
            table,
            count.max(1),
            count,
        );
    }
    qword(&mut bytes, WING_TABLE_A, WING_A as u64);
    qword(&mut bytes, WING_TABLE_A + 8, WING_B as u64);
    wing(
        &mut bytes,
        WING_A,
        (11_u64 << 32) | u64::from(cair_wing::TYPE),
    );
    wing(&mut bytes, WING_B, u64::MAX);
    bytes
}

fn registered(
    bytes: &[u8],
    rows: usize,
    arena: usize,
) -> (RegistrationWalk, Vec<RegistrationRow>, Vec<u64>) {
    let memory = SliceMemory { base: GAME, bytes };
    // Every resolver length, from none through the full live link count,
    // must publish the same walk; only the number of repeated resolves differs.
    let [first, rest @ ..] = [0, 1, 3, 10, 16].map(|cache| {
        let mut registrations = vec![RegistrationRow::default(); rows];
        let mut wings = vec![0; arena];
        let mut resolved = vec![Some(stale_country()); cache];
        let walk = walk_registrations(
            &memory,
            IMAGE,
            GAME,
            RegistrationBuffers {
                registrations: &mut registrations,
                wings: &mut wings,
                resolved: &mut resolved,
            },
        );
        (walk, registrations, wings)
    });
    for other in rest {
        assert_eq!(other, first);
    }
    first
}

/// A cell left over from an earlier walk; it must never be served.
fn stale_country() -> ccountry::Country {
    ccountry::Country {
        pointer: 0xdead,
        canonical_slot: 1,
        id: 9,
    }
}

#[test]
fn registered_slots_keep_native_order_reserved_slots_and_empty_containers() {
    let (walk, rows, wings) = registered(&registration_fixture(), 4, 8);
    assert_eq!(walk.outcome, Outcome::Rows { len: 2 });
    assert_eq!((walk.wings_len, walk.required_wings), (2, 2));
    assert_eq!(
        rows[0],
        RegistrationRow {
            base: (7_u64 << 32) | 65,
            slot: 0,
            capacity_raw: 0,
            wings_start: 0,
            wings_len: 2,
        }
    );
    // A present but empty container is a registration with no wings.
    assert_eq!(
        rows[1],
        RegistrationRow {
            base: (7_u64 << 32) | 65,
            slot: 2,
            capacity_raw: 0,
            wings_start: 2,
            wings_len: 0,
        }
    );
    assert_eq!(
        &wings[..2],
        [(11_u64 << 32) | u64::from(cair_wing::TYPE), u64::MAX]
    );
}

#[test]
fn registration_scratch_reuse_preserves_shorter_bases_and_fault_prefixes() {
    use std::cell::RefCell;

    const SHORT_CONTAINERS: usize = 0x2a00;
    const SHORT_POOLS: usize = 0x2a10;
    const SHORT_WINGS: usize = 0x2a20;
    const CONTAINER: usize = 0x3c00;
    const POOL: usize = 0x3d00;
    const WING: usize = 0x3e00;
    let mut bytes = registration_fixture();
    // The first base fills longer container, pool and wing prefixes. Keep
    // duplicate wing occurrences, then use only one entry of each on host.
    array(
        &mut bytes,
        CONTAINER_A + ccountry_air_container::POOLS,
        POOL_TABLE_A,
        2,
        2,
    );
    qword(&mut bytes, POOL_TABLE_A + 8, POOL_B as u64);
    array(
        &mut bytes,
        POOL_A + cair_wing_pool::WINGS,
        WING_TABLE_A,
        3,
        3,
    );
    qword(&mut bytes, WING_TABLE_A + 8, WING_A as u64);
    qword(&mut bytes, WING_TABLE_A + 16, WING_B as u64);
    for (header, data) in [
        (HOST_BASE + COUNTRY_CONTAINERS, SHORT_CONTAINERS),
        (CONTAINER + ccountry_air_container::POOLS, SHORT_POOLS),
        (POOL + cair_wing_pool::WINGS, SHORT_WINGS),
    ] {
        array(&mut bytes, header, data, 1, 1);
    }
    for (at, value) in [
        (SHORT_CONTAINERS, CONTAINER),
        (SHORT_POOLS, POOL),
        (SHORT_WINGS, WING),
        (CONTAINER, IMAGE + ccountry_air_container::VTABLE_RVA),
        (POOL, IMAGE + cair_wing_pool::VTABLE_RVA),
    ] {
        qword(&mut bytes, at, value as u64);
    }
    wing(&mut bytes, WING, u64::MAX);
    let (walk, rows, wings) = registered(&bytes, 4, 8);
    assert_eq!(walk.outcome, Outcome::Rows { len: 3 });
    assert_eq!((walk.wings_len, walk.required_wings), (4, 4));
    let repeated = (11_u64 << 32) | u64::from(cair_wing::TYPE);
    assert_eq!(&wings[..4], [repeated, repeated, u64::MAX, u64::MAX]);
    assert_eq!(
        rows[2],
        RegistrationRow {
            base: (9_u64 << 32) | u64::from(TYPE),
            slot: 0,
            capacity_raw: 0,
            wings_start: 3,
            wings_len: 1,
        }
    );

    struct Traced<'a> {
        bytes: &'a [u8],
        blocked: Option<usize>,
        reads: RefCell<Vec<(usize, usize)>>,
    }
    impl Memory for Traced<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, destination.len()));
            if self
                .blocked
                .is_some_and(|blocked| (address..address + destination.len()).contains(&blocked))
            {
                destination[0] = 0xff;
                return false;
            }
            SliceMemory {
                base: GAME,
                bytes: self.bytes,
            }
            .copy(address, destination)
        }
    }
    for blocked in [
        None,
        Some(HOST_BASE + COUNTRY_CONTAINERS),
        Some(SHORT_CONTAINERS),
        Some(CONTAINER),
        Some(CONTAINER + ccountry_air_container::COUNTRY),
        Some(CONTAINER + ccountry_air_container::POOLS),
        Some(SHORT_POOLS),
        Some(POOL),
        Some(POOL + cair_wing_pool::WINGS),
        Some(SHORT_WINGS),
        Some(WING + WING_RECORD_BYTES - 1),
    ] {
        let run = |reset_each_base| {
            let memory = Traced {
                bytes: &bytes,
                blocked,
                reads: RefCell::new(Vec::new()),
            };
            let reader = Reader {
                memory: &memory,
                base: IMAGE,
            };
            let countries = ccountry::Reader::new(&memory, IMAGE, GAME).unwrap();
            let mut cells = [None; 16];
            let mut resolved = Resolver { cells: &mut cells };
            let mut state = RegistrationState::default();
            state.scratch.container_bytes.fill(0xa5);
            state.scratch.pool_pointers.fill(usize::MAX);
            state.scratch.pool_records.fill(0xa5);
            state.scratch.pool_pointer_bytes.fill(0xa5);
            state.scratch.wing_pointers.fill(usize::MAX);
            state.scratch.wing_records.fill(0xa5);
            state.scratch.wing_pointer_bytes.fill(0xa5);
            let mut rows = [RegistrationRow::default(); 4];
            let mut wings = [0; 8];
            let mut buffers = RegistrationBuffers {
                registrations: &mut rows,
                wings: &mut wings,
                resolved: &mut [],
            };
            let result = [(STATE_BASE, 7_u64), (HOST_BASE, 9)]
                .into_iter()
                .try_for_each(|(pointer, id)| {
                    if reset_each_base {
                        state.scratch = RegistrationScratch::default();
                    }
                    reader.registration(
                        (id << 32) | u64::from(TYPE),
                        pointer,
                        &countries,
                        &mut resolved,
                        &mut buffers,
                        &mut state,
                    )
                });
            (
                result,
                (
                    state.len,
                    state.unread_rows,
                    state.wings_len,
                    state.required_wings,
                ),
                rows,
                wings,
                memory.reads.into_inner(),
            )
        };
        let reused = run(false);
        assert_eq!(reused, run(true), "blocked={blocked:x?}");
        assert_eq!(reused.0.is_ok(), blocked.is_none());
    }
}

#[test]
fn registration_capacity_releases_staged_wings_and_reports_full_demand() {
    let (walk, rows, wings) = registered(&registration_fixture(), 4, 1);
    assert_eq!(
        walk.outcome,
        Outcome::Truncated {
            len: 1,
            unread_rows: 1
        }
    );
    // The oversized container is omitted whole; its first wing is released.
    assert_eq!((walk.wings_len, walk.required_wings), (0, 2));
    assert_eq!(
        (rows[0].slot, rows[0].wings_start, rows[0].wings_len),
        (2, 0, 0)
    );
    // The omitted container's staged wing is released, so nothing it wrote
    // can be published by the surviving row.
    assert!(wings[..walk.wings_len].is_empty());
    let (walk, _, _) = registered(&registration_fixture(), 0, 8);
    assert_eq!(
        walk.outcome,
        Outcome::Truncated {
            len: 0,
            unread_rows: 2
        }
    );
    assert_eq!(walk.required_wings, 2);
}

#[test]
fn registration_faults_never_become_absence() {
    for (address, value, reason) in [
        (
            CONTAINER_A,
            0x1234_u64,
            "air base or linked object vtable mismatch",
        ),
        (
            POOL_TABLE_A,
            0,
            "air container pool pointer unreadable or null",
        ),
        (WING_TABLE_A, 0, "air pool wing pointer unreadable or null"),
        (
            WING_A + cair_wing::REFERENCE_TYPE,
            0,
            "registered air wing has a null reference handle",
        ),
        (
            WING_A + cair_wing::SECONDARY,
            0x1234,
            "air pool wing vtable mismatch",
        ),
        (
            CONTAINER_B + ccountry_air_container::COUNTRY,
            5,
            "air base container country differs from its slot",
        ),
        (
            CONTAINER_A + ccountry_air_container::COUNTRY,
            5,
            "air base container at the reserved slot is linked to a country slot",
        ),
    ] {
        let mut bytes = registration_fixture();
        qword(&mut bytes, address, value);
        assert_eq!(
            registered(&bytes, 4, 8).0.outcome,
            Outcome::Unavailable { reason },
            "{address:#x}"
        );
    }
    let mut bytes = registration_fixture();
    // A pool count above its own capacity is a torn vector header.
    dword(&mut bytes, POOL_A + cair_wing_pool::WINGS_COUNT, 9);
    assert_eq!(
        registered(&bytes, 4, 8).0.outcome,
        Outcome::Unavailable {
            reason: "air pool wing vector unreadable or invalid"
        }
    );
}

#[test]
fn a_reserved_slot_accepts_an_unlinked_reference_and_keeps_its_capacity() {
    let mut bytes = registration_fixture();
    // Link entry seven is zero: an id the engine has not filed anywhere.
    dword(&mut bytes, CONTAINER_A + ccountry_air_container::COUNTRY, 7);
    qword(
        &mut bytes,
        CONTAINER_A + ccountry_air_container::CAPACITY_RAW,
        ccountry_air_container::INITIAL_CAPACITY_RAW as u64,
    );
    let (walk, rows, _) = registered(&bytes, 4, 8);
    assert_eq!(walk.outcome, Outcome::Rows { len: 2 });
    assert_eq!(
        (rows[0].slot, rows[0].capacity_raw),
        (0, ccountry_air_container::INITIAL_CAPACITY_RAW)
    );
    // A link entry naming a slot outside the country table is a torn read,
    // not an unlinked id, so it refuses instead of publishing the row.
    dword(&mut bytes, LINK_TABLE + 7 * 4, 4);
    assert_eq!(
        registered(&bytes, 4, 8).0.outcome,
        Outcome::Unavailable {
            reason: "country link unreadable or outside country table"
        }
    );
}

#[test]
fn a_base_without_containers_is_an_honest_empty_registration_table() {
    let mut bytes = registration_fixture();
    dword(&mut bytes, STATE_BASE + COUNTRY_CONTAINERS_COUNT, 0);
    let (walk, _, _) = registered(&bytes, 4, 8);
    assert_eq!(walk.outcome, Outcome::Rows { len: 0 });
    assert_eq!((walk.wings_len, walk.required_wings), (0, 0));
}

/// Guarded rounds: one per `copy` and one per `copy_records`, as in
/// `LiveMemory`. Batched element copies go through the trait default.
struct Rounds<'a> {
    inner: SliceMemory<'a>,
    rounds: std::cell::RefCell<Vec<usize>>,
}

impl Memory for Rounds<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.rounds.borrow_mut().push(address);
        self.inner.copy(address, destination)
    }

    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        self.rounds
            .borrow_mut()
            .push(sources.pointer(0).unwrap_or(0));
        struct Plain<'a, 'b>(&'a SliceMemory<'b>);
        impl Memory for Plain<'_, '_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                self.0.copy(address, destination)
            }
        }
        Plain(&self.inner).copy_records(sources, layout, destination)
    }
}

/// Well above the retired 1,024-cell resolver: every country slot of one
/// host base holds a container, and a second base shares every seventh.
/// Odd slots store an alias id, so the link table is twice the census.
#[test]
fn five_thousand_country_registrations_match_with_any_resolver_length() {
    const COUNTRIES: u32 = 5_001;
    const LINKS: u32 = COUNTRIES * 2;
    const HOSTS: usize = 0x2100;
    const BASE_A: usize = 0x2200;
    const BASE_B: usize = 0x2300;
    const TABLE: usize = 0x3000;
    const LINK: usize = 0x10000;
    const OBJECTS: usize = 0x20000;
    const SLOTS_A: usize = 0x40000;
    const SLOTS_B: usize = 0x50000;
    const CONTAINERS: usize = 0x60000;
    let mut bytes = vec![0; CONTAINERS + COUNTRIES as usize * 0x100];
    qword(
        &mut bytes,
        GAME + cgame_state::STRATEGIC_AIR_MANAGER,
        MANAGER as u64,
    );
    qword(
        &mut bytes,
        MANAGER,
        (IMAGE + cstrategic_air_manager::VTABLE_RVA) as u64,
    );
    array(
        &mut bytes,
        MANAGER + cstrategic_air_manager::STATELESS_AIR_BASES,
        HOSTS,
        2,
        2,
    );
    array(
        &mut bytes,
        GAME + cgame_state::COUNTRIES,
        TABLE,
        COUNTRIES,
        COUNTRIES,
    );
    array(
        &mut bytes,
        GAME + cgame_state::COUNTRY_LINKS,
        LINK,
        LINKS,
        LINKS,
    );
    for (index, (base, slots, id)) in [(BASE_A, SLOTS_A, 7_u64), (BASE_B, SLOTS_B, 9)]
        .into_iter()
        .enumerate()
    {
        qword(&mut bytes, HOSTS + index * 8, base as u64);
        qword(&mut bytes, base, (IMAGE + VTABLE_RVA) as u64);
        qword(&mut bytes, base + HANDLE, (id << 32) | u64::from(TYPE));
        array(
            &mut bytes,
            base + COUNTRY_CONTAINERS,
            slots,
            COUNTRIES,
            COUNTRIES,
        );
    }
    let mut expected = Vec::new();
    for slot in 1..COUNTRIES {
        let object = OBJECTS + slot as usize * 16;
        qword(&mut bytes, TABLE + slot as usize * 8, object as u64);
        qword(&mut bytes, object, (IMAGE + ccountry::VTABLE_RVA) as u64);
        dword(&mut bytes, object + ccountry::ID, slot);
        dword(&mut bytes, LINK + slot as usize * 4, slot);
        dword(&mut bytes, LINK + (slot + COUNTRIES) as usize * 4, slot);
        let raw = if slot % 2 == 1 {
            slot + COUNTRIES
        } else {
            slot
        };
        let container = CONTAINERS + slot as usize * 0x100;
        qword(
            &mut bytes,
            container,
            (IMAGE + ccountry_air_container::VTABLE_RVA) as u64,
        );
        dword(&mut bytes, container + ccountry_air_container::COUNTRY, raw);
        qword(
            &mut bytes,
            container + ccountry_air_container::CAPACITY_RAW,
            u64::from(raw),
        );
        qword(&mut bytes, SLOTS_A + slot as usize * 8, container as u64);
        expected.push(((7_u64 << 32) | u64::from(TYPE), slot, i64::from(raw)));
        if slot % 7 == 0 {
            qword(&mut bytes, SLOTS_B + slot as usize * 8, container as u64);
        }
    }
    expected.extend((1..COUNTRIES).filter(|slot| slot % 7 == 0).map(|slot| {
        let raw = if slot % 2 == 1 {
            slot + COUNTRIES
        } else {
            slot
        };
        ((9_u64 << 32) | u64::from(TYPE), slot, i64::from(raw))
    }));
    let run = |cache: usize| {
        let memory = Rounds {
            inner: SliceMemory {
                base: GAME,
                bytes: &bytes,
            },
            rounds: Default::default(),
        };
        let mut registrations = vec![RegistrationRow::default(); expected.len()];
        let mut wings = [0; 1];
        let mut resolved = vec![None; cache];
        let walk = walk_registrations(
            &memory,
            IMAGE,
            GAME,
            RegistrationBuffers {
                registrations: &mut registrations,
                wings: &mut wings,
                resolved: &mut resolved,
            },
        );
        (walk, registrations, memory.rounds.into_inner())
    };
    let (walk, rows, rounds) = run(LINKS as usize);
    assert_eq!(
        walk.outcome,
        Outcome::Rows {
            len: expected.len()
        }
    );
    assert_eq!(walk.required_links, LINKS as usize);
    assert_eq!(
        rows.iter()
            .map(|row| (row.base, row.slot, row.capacity_raw))
            .collect::<Vec<_>>(),
        expected
    );
    assert!(rows.iter().all(|row| row.wings_len == 0));
    // Each base's country-slot vector costs one guarded round per 64 slots.
    let chunk_rounds = rounds
        .iter()
        .filter(|&&at| {
            (SLOTS_A..SLOTS_A + COUNTRIES as usize * 8).contains(&at)
                || (SLOTS_B..SLOTS_B + COUNTRIES as usize * 8).contains(&at)
        })
        .count();
    assert_eq!(chunk_rounds, 2 * (COUNTRIES as usize).div_ceil(BATCH));
    // A short or absent cache resolves the extra ids again, never differently.
    for cache in [0, 1024, COUNTRIES as usize] {
        let (short, short_rows, short_rounds) = run(cache);
        assert_eq!((short, &short_rows), (walk, &rows), "cache {cache}");
        assert!(short_rounds.len() > rounds.len(), "cache {cache}");
    }
    // The second base reuses every resolution made for the first.
    let resolves = |rounds: &[usize]| {
        rounds
            .iter()
            .filter(|&&at| (LINK..LINK + LINKS as usize * 4).contains(&at))
            .count()
    };
    let (_, _, uncached) = run(0);
    assert!(resolves(&rounds) < resolves(&uncached));
}
