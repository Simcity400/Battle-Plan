use super::*;
use engine_native::cgame_state;
use engine_native_memory::memory::SliceMemory;

fn i32_at(bytes: &mut [u8], at: usize, value: i32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn u64_at(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
fn array(bytes: &mut [u8], at: usize, data: usize, count: usize) {
    u64_at(bytes, at, data as u64);
    i32_at(bytes, at + 8, count as i32);
    i32_at(bytes, at + 12, count as i32);
}

struct Fault<'a> {
    bytes: &'a [u8],
    address: usize,
}
impl Memory for Fault<'_> {
    fn copy(&self, at: usize, output: &mut [u8]) -> bool {
        if at <= self.address && self.address < at.saturating_add(output.len()) {
            return false;
        }
        SliceMemory {
            base: 0,
            bytes: self.bytes,
        }
        .copy(at, output)
    }
}

#[test]
fn game_score_country_counts_preserve_signed_values_and_fleet_occurrences() {
    const N: usize = 4;
    const GS: usize = 0x1000;
    const POINTERS: usize = 0x3000;
    const LINKS: usize = POINTERS + (N + 1) * 8;
    const OBJECTS: usize = LINKS + (N + 1) * 4;
    const FLEET_POINTERS: usize = OBJECTS + N * 0x500;
    const FLEET_A: usize = FLEET_POINTERS + 0x100;
    const FLEET_B: usize = FLEET_A + 0x100;
    let mut bytes = vec![0; FLEET_B + 0x100];
    array(&mut bytes, GS + cgame_state::COUNTRIES, POINTERS, N + 1);
    array(&mut bytes, GS + cgame_state::COUNTRY_LINKS, LINKS, N + 1);
    for index in 0..N {
        let slot = index + 1;
        let pointer = OBJECTS + index * 0x500;
        u64_at(&mut bytes, POINTERS + slot * 8, pointer as u64);
        i32_at(&mut bytes, LINKS + slot * 4, slot as i32);
        u64_at(&mut bytes, pointer, ccountry::VTABLE_RVA.0 as u64);
        i32_at(&mut bytes, pointer + ccountry::ID, slot as i32);
        i32_at(
            &mut bytes,
            pointer + ccountry::DIVISIONS_COUNT,
            -(slot as i32),
        );
        i32_at(
            &mut bytes,
            pointer + ccountry::CONTROLLED_PROVINCES_COUNT,
            slot as i32,
        );
        array(&mut bytes, pointer + ccountry::FLEETS, FLEET_POINTERS, 3);
    }
    for (index, pointer) in [FLEET_A, FLEET_B, FLEET_A].into_iter().enumerate() {
        u64_at(&mut bytes, FLEET_POINTERS + index * 8, pointer as u64);
    }
    for (pointer, handle, count) in [(FLEET_A, 61_u64, i32::MAX), (FLEET_B, 62, 2)] {
        u64_at(&mut bytes, pointer, cfleet::VTABLE_RVA.0 as u64);
        u64_at(&mut bytes, pointer + cfleet::HANDLE, handle);
        i32_at(&mut bytes, pointer + cfleet::TASK_FORCE_COUNT, count);
    }
    let mut countries = vec![CountryRow::default(); N];
    let mut fleets = vec![FleetCount::default(); N * 3];
    let result = walk_country_inputs(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        GS,
        &mut countries,
        &mut fleets,
    );
    assert_eq!(
        result,
        CountryCapture {
            outcome: Outcome::Rows { len: N },
            country_len: N,
            fleet_len: N * 3,
            fleet_required: N * 3,
            slot_zero_null: true,
            slot_zero_refusal: None
        }
    );
    let mut growing_countries = Vec::new();
    let mut growing_fleets = Vec::new();
    let first = walk_country_inputs(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        GS,
        &mut growing_countries,
        &mut growing_fleets,
    );
    assert_eq!(first, result);
    assert_eq!(growing_countries[..N], countries[..N]);
    assert_eq!(growing_fleets[..N * 3], fleets[..N * 3]);
    for (index, row) in countries[..N].iter().enumerate() {
        assert_eq!(
            (row.country, row.army_count, row.navy_count, row.land_count),
            (
                (index + 1) as u32,
                Ok(-((index + 1) as i32)),
                Ok(0),
                Ok((index + 1) as i32)
            )
        );
        assert_eq!(row.fleet_len, 3);
        assert_eq!(
            fleets[row.fleet_start..row.fleet_start + 3]
                .iter()
                .map(|row| row.fleet)
                .collect::<Vec<_>>(),
            [61, 62, 61]
        );
    }
    let result = walk_country_inputs(
        &Fault {
            bytes: &bytes,
            address: OBJECTS + ccountry::DIVISIONS_COUNT,
        },
        0,
        GS,
        &mut countries,
        &mut fleets,
    );
    assert_eq!(result.outcome, Outcome::Rows { len: N });
    assert_eq!(
        countries[0].army_count,
        Err("game score army roster count unreadable")
    );
    assert_eq!(
        (countries[0].navy_count, countries[0].land_count),
        (Ok(0), Ok(1))
    );
    assert_eq!(countries[N - 1].army_count, Ok(-(N as i32)));
    let result = walk_country_inputs(
        &Fault {
            bytes: &bytes,
            address: FLEET_B + cfleet::TASK_FORCE_COUNT,
        },
        0,
        GS,
        &mut countries,
        &mut fleets,
    );
    assert_eq!(result.outcome, Outcome::Rows { len: N });
    assert_eq!(
        (countries[0].army_count, countries[0].land_count),
        (Ok(-1), Ok(1))
    );
    assert_eq!(
        countries[0].navy_count,
        Err("game score task force count unreadable")
    );
    assert_eq!(
        fleets[1],
        FleetCount {
            fleet: 62,
            task_forces: Err("game score task force count unreadable")
        }
    );
    let result = walk_country_inputs(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        GS,
        &mut countries,
        &mut Vec::new(),
    );
    assert_eq!(result.fleet_required, N * 3);
    assert_eq!(result.outcome, Outcome::Rows { len: N });
    assert!(countries[..N].iter().all(|row| row.navy_count.is_ok()));
}

#[test]
fn game_score_country_prefix_diagnostics_survive_later_identity_failure() {
    const GS: usize = 0x1000;
    const FIRST: usize = 0x2000;
    let mut bytes = vec![0; 0x4000];
    array(&mut bytes, GS + cgame_state::COUNTRIES, 0x1800, 3);
    array(&mut bytes, GS + cgame_state::COUNTRY_LINKS, 0x1900, 3);
    for slot in 1..=2 {
        let pointer = FIRST + (slot - 1) * 0x500;
        u64_at(&mut bytes, 0x1800 + slot * 8, pointer as u64);
        i32_at(&mut bytes, 0x1900 + slot * 4, slot as i32);
        u64_at(&mut bytes, pointer, ccountry::VTABLE_RVA.0 as u64);
        i32_at(&mut bytes, pointer + ccountry::ID, slot as i32);
    }
    u64_at(&mut bytes, FIRST + 0x500, 0);
    let mut rows = vec![CountryRow::default(); 2];
    let read = |bytes: &[u8], rows: &mut Vec<CountryRow>| {
        walk_country_inputs(
            &Fault {
                bytes,
                address: FIRST + ccountry::DIVISIONS_COUNT,
            },
            0,
            GS,
            rows,
            &mut Vec::new(),
        )
    };
    let capture = read(&bytes, &mut rows);
    assert_eq!(
        capture.outcome,
        Outcome::Unavailable {
            reason: "country vtable mismatch"
        }
    );
    assert_eq!(capture.country_len, 1);
    assert_eq!(
        rows[0].army_count,
        Err("game score army roster count unreadable")
    );
    assert_eq!(rows[0].land_count, Ok(0));
    u64_at(&mut bytes, FIRST, 0);
    let capture = read(&bytes, &mut rows);
    assert_eq!(
        capture.outcome,
        Outcome::Unavailable {
            reason: "country vtable mismatch"
        }
    );
    assert_eq!(
        capture.country_len, 0,
        "a failed read cannot reuse the previous prefix"
    );
}

#[test]
fn game_score_slot_zero_object_is_included_and_refusals_are_named() {
    const GS: usize = 0x1000;
    const OBJECT: usize = 0x2000;
    let mut bytes = vec![0; 0x3000];
    array(&mut bytes, GS + cgame_state::COUNTRIES, 0x1800, 2);
    array(&mut bytes, GS + cgame_state::COUNTRY_LINKS, 0x1900, 2);
    let mut rows = vec![CountryRow::default(); 2];
    let capture = walk_country_inputs(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        GS,
        &mut rows,
        &mut Vec::new(),
    );
    assert!(capture.slot_zero_null);
    assert_eq!(capture.slot_zero_refusal, None);
    u64_at(&mut bytes, 0x1800, OBJECT as u64);
    u64_at(&mut bytes, OBJECT, ccountry::VTABLE_RVA.0 as u64);
    i32_at(&mut bytes, OBJECT + ccountry::DIVISIONS_COUNT, -3);
    i32_at(&mut bytes, OBJECT + ccountry::CONTROLLED_PROVINCES_COUNT, 7);
    let capture = walk_country_inputs(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        GS,
        &mut rows,
        &mut Vec::new(),
    );
    assert_eq!(
        (
            capture.outcome,
            capture.slot_zero_null,
            capture.slot_zero_refusal
        ),
        (Outcome::Rows { len: 1 }, false, None)
    );
    assert_eq!(
        (
            rows[0].country,
            rows[0].canonical_slot,
            rows[0].army_count,
            rows[0].land_count
        ),
        (0, 0, Ok(-3), Ok(7))
    );
    u64_at(&mut bytes, OBJECT, 0);
    let capture = walk_country_inputs(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        GS,
        &mut rows,
        &mut Vec::new(),
    );
    assert_eq!(
        (capture.outcome, capture.slot_zero_refusal),
        (
            Outcome::Rows { len: 0 },
            Some("game score country slot zero vtable mismatch")
        )
    );
    let capture = walk_country_inputs(
        &Fault {
            bytes: &bytes,
            address: 0x1800,
        },
        0,
        GS,
        &mut rows,
        &mut Vec::new(),
    );
    assert_eq!(
        capture.slot_zero_refusal,
        Some("game score country slot zero pointer unreadable")
    );
}

#[test]
fn game_score_sizing_counts_failures_and_keeps_later_healthy_demand() {
    const GS: usize = 0x1000;
    const COUNTRY: usize = 0x3500;
    const AIR: usize = 0x6400;
    let mut bytes = vec![0; 0x9000];
    array(&mut bytes, GS + cgame_state::COUNTRIES, 0x2000, 4);
    array(&mut bytes, GS + cgame_state::COUNTRY_LINKS, 0x2100, 4);
    for (slot, pointer) in [(1, 0x3000), (2, COUNTRY), (3, 0x3a00)] {
        u64_at(&mut bytes, 0x2000 + slot * 8, pointer as u64);
        i32_at(&mut bytes, 0x2100 + slot * 4, slot as i32);
    }
    u64_at(&mut bytes, COUNTRY, ccountry::VTABLE_RVA.0 as u64);
    i32_at(&mut bytes, COUNTRY + ccountry::ID, 2);
    array(&mut bytes, COUNTRY + ccountry::FLEETS, 0x4800, 4);
    u64_at(&mut bytes, GS + cgame_state::STRATEGIC_AIR_MANAGER, 0x5000);
    u64_at(
        &mut bytes,
        0x5000,
        cstrategic_air_manager::VTABLE_RVA.0 as u64,
    );
    array(
        &mut bytes,
        0x5000 + cstrategic_air_manager::COUNTRY_AIR,
        0x6000,
        3,
    );
    u64_at(&mut bytes, 0x6008, 0x6200);
    u64_at(&mut bytes, 0x6010, AIR as u64);
    u64_at(&mut bytes, AIR, cstrategic_air::VTABLE_RVA.0 as u64);
    array(&mut bytes, AIR + cstrategic_air::FIRST_AIR_SITES, 0x7800, 5);
    array(&mut bytes, AIR + cstrategic_air::POOLS, 0x7000, 3);
    for (index, pointer) in [0x7100, 0x7200, 0x7100].into_iter().enumerate() {
        u64_at(&mut bytes, 0x7000 + index * 8, pointer);
    }
    u64_at(&mut bytes, 0x7200, cair_wing_pool::VTABLE_RVA.0 as u64);
    array(&mut bytes, 0x7200 + cair_wing_pool::WINGS, 0x8000, 7);
    let expected_failures = vec![
        ("country vtable mismatch", 2),
        ("game score sizing air-country vtable mismatch", 1),
        ("game score sizing air-pool vtable mismatch", 2),
    ];
    let observed = arena_demand(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        GS,
    )
    .unwrap();
    assert_eq!(
        observed,
        ArenaSizing {
            demand: ArenaDemand {
                countries: 4,
                fleets: 4,
                sites: 5,
                wings: 7
            },
            failures: expected_failures.clone()
        }
    );
    let observed = arena_demand(
        &Fault {
            bytes: &bytes,
            address: 0x2008,
        },
        0,
        GS,
    )
    .unwrap();
    assert_eq!(
        observed,
        ArenaSizing {
            demand: ArenaDemand {
                countries: 4,
                fleets: 4,
                sites: 5,
                wings: 7
            },
            failures: vec![
                ("country pointer unreadable or outside country table", 1),
                ("country vtable mismatch", 1),
                ("game score sizing air-country vtable mismatch", 1),
                ("game score sizing air-pool vtable mismatch", 2)
            ]
        }
    );
    let observed = arena_demand(
        &Fault {
            bytes: &bytes,
            address: 0x7000,
        },
        0,
        GS,
    )
    .unwrap();
    assert_eq!(
        observed,
        ArenaSizing {
            demand: ArenaDemand {
                countries: 4,
                fleets: 4,
                sites: 5,
                wings: 7
            },
            failures: vec![
                ("country vtable mismatch", 2),
                ("game score sizing air-country vtable mismatch", 1),
                ("game score sizing air-pool pointer unreadable", 1),
                ("game score sizing air-pool vtable mismatch", 1)
            ]
        }
    );
    for (offset, reason) in [
        (
            cgame_state::COUNTRIES,
            "country array unreadable or invalid",
        ),
        (
            cgame_state::COUNTRY_LINKS,
            "country links unreadable or invalid",
        ),
    ] {
        let observed = arena_demand(
            &Fault {
                bytes: &bytes,
                address: GS + offset,
            },
            0,
            GS,
        )
        .unwrap();
        assert_eq!(
            observed,
            ArenaSizing {
                demand: ArenaDemand {
                    countries: 0,
                    fleets: 0,
                    sites: 5,
                    wings: 7
                },
                failures: vec![
                    (reason, 1),
                    ("game score sizing air-country vtable mismatch", 1),
                    ("game score sizing air-pool vtable mismatch", 2)
                ]
            }
        );
    }
    let observed = arena_demand(
        &Fault {
            bytes: &bytes,
            address: AIR + cstrategic_air::FIRST_AIR_SITES,
        },
        0,
        GS,
    )
    .unwrap();
    assert_eq!(
        observed.demand,
        ArenaDemand {
            countries: 4,
            fleets: 4,
            sites: 0,
            wings: 7
        }
    );
    let mut expected_failures = expected_failures;
    expected_failures.push((
        "game score sizing first air-site roster unreadable or invalid",
        1,
    ));
    assert_eq!(observed.failures, expected_failures);
}

#[test]
fn game_score_first_sites_keep_repeats_access_false_and_read_failures_distinct() {
    let mut bytes = vec![0; 0x4000];
    array(
        &mut bytes,
        0x100 + cstrategic_air::FIRST_AIR_SITES,
        0x500,
        3,
    );
    for (index, pointer) in [0x1000_u64, 0x2000, 0x1000].into_iter().enumerate() {
        u64_at(&mut bytes, 0x500 + index * 8, pointer);
    }
    for (pointer, handle) in [(0x1000, 65_u64), (0x2000, 66)] {
        u64_at(&mut bytes, pointer, engine_native_parts::cair_base_layout::VTABLE_RVA.0 as u64);
        u64_at(&mut bytes, pointer + 8, handle);
    }
    i32_at(&mut bytes, 0x1000 + cair_base::COUNTRY_CONTAINERS_COUNT, 2);
    u64_at(&mut bytes, 0x1000 + cair_base::COUNTRY_CONTAINERS, 0x3000);
    u64_at(&mut bytes, 0x3008, 0xdead);
    i32_at(&mut bytes, 0x2000 + cair_base::COUNTRY_CONTAINERS_COUNT, 1);
    let mut rows = vec![AirSite::default(); 3];
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    assert_eq!(
        read_first_sites_buffer(&memory, 0, 1, Some(0x100), engine_native_memory::walk::Buffer::Growing(&mut rows)),
        AirRead {
            state: AirState::Present,
            error: "",
            len: 3,
            required: 3
        }
    );
    assert_eq!(
        rows,
        [
            AirSite {
                base: 65,
                access: Ok(true)
            },
            AirSite {
                base: 66,
                access: Ok(false)
            },
            AirSite {
                base: 65,
                access: Ok(true)
            }
        ]
    );
    assert_eq!(
        read_first_sites_buffer(&memory, 0, 1, None, engine_native_memory::walk::Buffer::Growing(&mut rows)).state,
        AirState::Absent
    );
    assert_eq!(
        read_first_sites_buffer(&memory, 0, 1, Some(0x100), engine_native_memory::walk::Buffer::Count),
        AirRead {
            state: AirState::Unknown,
            error: "game score first air-site arena exhausted",
            len: 0,
            required: 3
        }
    );
    assert_eq!(
        read_first_sites_buffer(&Fault {
                bytes: &bytes,
                address: 0x3008
            }, 0, 1, Some(0x100), engine_native_memory::walk::Buffer::Growing(&mut rows))
        .state,
        AirState::Present
    );
    assert_eq!(rows[0].access, Err("game score air-site access unreadable"));
    assert_eq!(rows[1].access, Ok(false));
    u64_at(&mut bytes, 0x3008, 0);
    assert_eq!(
        read_air_site(
            &SliceMemory {
                base: 0,
                bytes: &bytes
            },
            0,
            1,
            0x1000
        )
        .unwrap()
        .access,
        Ok(false)
    );
}

#[test]
fn game_score_wing_uses_stored_mission_one_record_and_shared_plane_read() {
    let mut bytes = vec![0; 0x3000];
    let wing = 0x100;
    bytes[wing + cair_wing::MISSION_STATS + cair_wing::MISSION_STAT_INDEXES] = 255;
    u64_at(&mut bytes, wing + cair_wing::DEFAULT_STATS + 0x33 * 8, 73);
    let row = cair_wing::ScoreLeaf {
        id: 69,
        planes: Ok(-4),
    };
    let read = |bytes: &[u8]| read_wing(&SliceMemory { base: 0, bytes }, wing, &row);
    assert_eq!(
        read(&bytes),
        WingRow {
            id: 69,
            planes: Ok(-4),
            stat_raw: Ok(73),
            mission_index: Ok(-1)
        }
    );
    bytes[wing + cair_wing::MISSION_STATS + cair_wing::MISSION_STAT_INDEXES] = 1;
    array(&mut bytes, wing + cair_wing::MISSION_STATS, 0x2000, 2);
    u64_at(
        &mut bytes,
        0x2000 + cair_wing::MISSION_STAT_STRIDE + 0x33 * 8,
        (-29_i64) as u64,
    );
    assert_eq!(read(&bytes).stat_raw, Ok(-29));
    bytes[wing + cair_wing::MISSION_STATS + cair_wing::MISSION_STAT_INDEXES] = 254;
    let invalid = read(&bytes);
    assert_eq!(invalid.mission_index, Ok(-2));
    assert_eq!(
        invalid.stat_raw,
        Err("air wing mission stat index negative")
    );
    assert_eq!(invalid.planes, Ok(-4));
    assert_eq!(
        cair_wing::mission_stat_index(
            &SliceMemory {
                base: 0,
                bytes: &[]
            },
            wing,
            0
        ),
        Ok(-1)
    );
    assert_eq!(
        cair_wing::mission_stat_index(
            &SliceMemory {
                base: 0,
                bytes: &bytes
            },
            wing,
            3
        ),
        Err(cair_wing::MissionStatError::MultipleMissionBits)
    );
}

#[test]
fn game_score_multiplier_failures_do_not_hide_other_native_globals() {
    const MODULE: usize = 0x1_0000;
    let rvas = [
        COUNTRY_SCORE_MULTIPLIER_RVA.0,
        ARMY_SCORE_MULTIPLIER_RVA.0,
        NAVY_SCORE_MULTIPLIER_RVA.0,
        AIR_SCORE_MULTIPLIER_RVA.0,
        INDUSTRY_SCORE_MULTIPLIER_RVA.0,
        PROVINCE_SCORE_MULTIPLIER_RVA.0,
    ];
    let first = rvas.into_iter().min().unwrap();
    let mut bytes = vec![0; rvas.into_iter().max().unwrap() + 8 - first];
    let values = [-3_i64, -2, -1, 1, 2, 3];
    for (rva, value) in rvas.into_iter().zip(values) {
        u64_at(&mut bytes, rva - first, value as u64);
    }
    let image = SliceMemory {
        base: MODULE + first,
        bytes: &bytes,
    };
    let all = read_multipliers(&image, MODULE);
    assert_eq!(
        [
            all.total_raw,
            all.army_raw,
            all.navy_raw,
            all.air_raw,
            all.industry_raw,
            all.land_raw
        ],
        values.map(Ok::<i64, &'static str>)
    );
    struct MissingArmy<'a>(SliceMemory<'a>);
    impl Memory for MissingArmy<'_> {
        fn copy(&self, at: usize, output: &mut [u8]) -> bool {
            at != MODULE + ARMY_SCORE_MULTIPLIER_RVA.0 && self.0.copy(at, output)
        }
    }
    let partial = read_multipliers(&MissingArmy(image), MODULE);
    assert!(partial.army_raw.is_err());
    assert_eq!((partial.total_raw, partial.land_raw), (Ok(-3), Ok(3)));
}

#[test]
fn first_score_air_sites_grow_from_original_headers_for_mod_sized_lists() {
    let mut rows = Vec::new();
    for count in [50_usize, 200] {
        let mut bytes = vec![0; 0x5000 + count * 8];
        array(
            &mut bytes,
            0x100 + cstrategic_air::FIRST_AIR_SITES,
            0x5000,
            count,
        );
        u64_at(&mut bytes, 0x1000, engine_native_parts::cair_base_layout::VTABLE_RVA.0 as u64);
        u64_at(&mut bytes, 0x1008, 65);
        i32_at(&mut bytes, 0x1000 + cair_base::COUNTRY_CONTAINERS_COUNT, 1);
        for index in 0..count {
            u64_at(&mut bytes, 0x5000 + index * 8, 0x1000);
        }
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(
            read_first_sites_buffer(&memory, 0, 1, Some(0x100), engine_native_memory::walk::Buffer::Count).state,
            AirState::Unknown
        );
        let captured = read_first_sites_buffer(
            &memory,
            0,
            1,
            Some(0x100),
            engine_native_memory::walk::Buffer::Growing(&mut rows),
        );
        assert_eq!(
            captured,
            AirRead {
                state: AirState::Present,
                error: "",
                len: count,
                required: count
            }
        );
        assert!(
            rows[..count]
                .iter()
                .all(|row| row.base == 65 && row.access == Ok(false))
        );
    }
}

struct Rounds<'a> {
    inner: Fault<'a>,
    rounds: std::cell::Cell<usize>,
}

struct Once<'a> {
    inner: Fault<'a>,
    spent: std::cell::Cell<bool>,
    attempts: std::cell::Cell<usize>,
}

impl<'a> Once<'a> {
    fn new(bytes: &'a [u8], address: usize) -> Self {
        Self {
            inner: Fault { bytes, address },
            spent: std::cell::Cell::new(false),
            attempts: std::cell::Cell::new(0),
        }
    }
}

impl Memory for Once<'_> {
    fn copy(&self, at: usize, output: &mut [u8]) -> bool {
        let address = self.inner.address;
        if at <= address && address < at.saturating_add(output.len()) {
            self.attempts.set(self.attempts.get() + 1);
        }
        if self.spent.get() {
            return SliceMemory {
                base: 0,
                bytes: self.inner.bytes,
            }
            .copy(at, output);
        }
        let copied = self.inner.copy(at, output);
        self.spent.set(!copied);
        copied
    }
}

impl Memory for Rounds<'_> {
    fn copy(&self, at: usize, output: &mut [u8]) -> bool {
        self.rounds.set(self.rounds.get() + 1);
        self.inner.copy(at, output)
    }

    fn copy_records(
        &self,
        sources: engine_native_memory::memory::RecordSources<'_>,
        layout: &engine_native_memory::memory::RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        self.rounds.set(self.rounds.get() + 1);
        struct Plain<'a, 'b>(&'a Fault<'b>);
        impl Memory for Plain<'_, '_> {
            fn copy(&self, at: usize, output: &mut [u8]) -> bool {
                self.0.copy(at, output)
            }
        }
        Plain(&self.inner).copy_records(sources, layout, destination)
    }
}

