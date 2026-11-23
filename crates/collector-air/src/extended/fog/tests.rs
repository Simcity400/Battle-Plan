use super::*;
use engine_native::{carmy, cfleet, cgame_state, cprovince, cstate, ctask_force, cunit};
use engine_native_memory::memory::SliceMemory;
use replay_data::rows::TableSchema;

const BASE: usize = 0x10000;
const COUNTRY: usize = 0x11000;
const RADAR_GROUP: usize = 0x12000;
const RADAR_PROVINCES: usize = 0x12100;
const STATE_OBJECT: usize = 0x12200;
const HOSTS: usize = 0x12300;
const TALLIES: usize = 0x12340;
const THEATRE: usize = 0x13000;
const AREA: usize = 0x13400;
const PROVINCE_A: usize = 0x13600;
const PROVINCE_B: usize = 0x13700;
const DIVISION: usize = 0x14000;
const FLEET: usize = 0x14400;
const TASK_FORCE: usize = 0x14800;
const VIEWERS: usize = 0x14c00;
const MANAGER: usize = 0x15000;
const AIR_BASE: usize = 0x15400;
const CONTAINER: usize = 0x15800;
const POOL: usize = 0x15c00;
const WING: usize = 0x16000;

fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
    bytes[address - BASE..address - BASE + value.len()].copy_from_slice(value);
}

fn array(bytes: &mut [u8], address: usize, data: usize, count: u32) {
    put(bytes, address, &(data as u64).to_le_bytes());
    put(bytes, address + 8, &count.to_le_bytes());
    put(bytes, address + 12, &count.to_le_bytes());
}

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x8000];
    array(&mut bytes, BASE + cgame_state::COUNTRIES, 0x10800, 2);
    put(&mut bytes, 0x10808, &(COUNTRY as u64).to_le_bytes());
    array(&mut bytes, BASE + cgame_state::COUNTRY_LINKS, 0x10900, 3);
    put(&mut bytes, 0x10904, &1_u32.to_le_bytes());
    put(&mut bytes, 0x10908, &1_u32.to_le_bytes());
    put(
        &mut bytes,
        COUNTRY,
        &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(&mut bytes, COUNTRY + ccountry::ID, &2_u32.to_le_bytes());
    put(
        &mut bytes,
        COUNTRY + ccountry::RADARS_POOL,
        &(engine_native_parts::cradars_pool::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        COUNTRY + ccountry::RADARS_POOL_COUNTRY,
        &2_u32.to_le_bytes(),
    );
    array(&mut bytes, COUNTRY + ccountry::RADAR_GROUPS, 0x10a00, 1);
    put(&mut bytes, 0x10a00, &(RADAR_GROUP as u64).to_le_bytes());
    put(
        &mut bytes,
        RADAR_GROUP + radar_group::PARENT,
        &((COUNTRY + ccountry::RADARS_POOL) as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        RADAR_GROUP + radar_group::STATE,
        &(STATE_OBJECT as u64).to_le_bytes(),
    );
    array(
        &mut bytes,
        RADAR_GROUP + radar_group::PROVINCES,
        RADAR_PROVINCES,
        2,
    );
    put(&mut bytes, RADAR_PROVINCES, &11_u32.to_le_bytes());
    put(&mut bytes, RADAR_PROVINCES + 4, &12_u32.to_le_bytes());
    put(
        &mut bytes,
        STATE_OBJECT,
        &(cstate::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(&mut bytes, STATE_OBJECT + cstate::ID, &33_u32.to_le_bytes());
    array(&mut bytes, COUNTRY + ccountry::VOLUNTEER_HOSTS, HOSTS, 1);
    array(&mut bytes, COUNTRY + ccountry::SENT_VOLUNTEERS, 0x12380, 2);
    put(&mut bytes, 0x12380, &0_u64.to_le_bytes());
    put(
        &mut bytes,
        0x12388,
        &0xf000_0001_0000_0013_u64.to_le_bytes(),
    );
    array(
        &mut bytes,
        COUNTRY + ccountry::VOLUNTEER_TALLIES,
        TALLIES,
        1,
    );
    put(&mut bytes, HOSTS, &9_u32.to_le_bytes());
    put(&mut bytes, TALLIES, &(-3_i32).to_le_bytes());
    array(&mut bytes, COUNTRY + ccountry::THEATRES, 0x10b00, 1);
    put(&mut bytes, 0x10b00, &(THEATRE as u64).to_le_bytes());
    put(
        &mut bytes,
        THEATRE,
        &(ctheatre::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        THEATRE + ctheatre::HANDLE,
        &((5_u64 << 32) | u64::from(ctheatre::REFERENCE_TYPE)).to_le_bytes(),
    );
    put(
        &mut bytes,
        THEATRE + ctheatre::VOLUNTEERS_THEATRE,
        &4_i32.to_le_bytes(),
    );
    array(
        &mut bytes,
        THEATRE + ctheatre::CONTROLLER_AREA_DATA,
        0x10c00,
        1,
    );
    put(&mut bytes, 0x10c00, &(AREA as u64).to_le_bytes());
    put(
        &mut bytes,
        AREA,
        &(ccontroller_area::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        AREA + ccontroller_area::REPRESENTATIVE_PROVINCE,
        &(PROVINCE_A as u64).to_le_bytes(),
    );
    array(
        &mut bytes,
        AREA + ccontroller_area::PROVINCE_DATA,
        0x10d00,
        2,
    );
    put(&mut bytes, 0x10d00, &(PROVINCE_A as u64).to_le_bytes());
    put(&mut bytes, 0x10d08, &(PROVINCE_B as u64).to_le_bytes());
    for (province, id) in [(PROVINCE_A, 21_u32), (PROVINCE_B, 22)] {
        put(
            &mut bytes,
            province,
            &(cprovince::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut bytes, province + cprovince::ID, &id.to_le_bytes());
    }
    array(
        &mut bytes,
        THEATRE + ctheatre::STATIONED_UNIT_DATA,
        0x10e00,
        1,
    );
    put(&mut bytes, 0x10e00, &(DIVISION as u64).to_le_bytes());
    array(&mut bytes, COUNTRY + ccountry::DIVISIONS, 0x10f00, 1);
    put(&mut bytes, 0x10f00, &(DIVISION as u64).to_le_bytes());
    put(
        &mut bytes,
        DIVISION,
        &(carmy::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        DIVISION + cunit::HANDLE,
        &((6_u64 << 32) | 1).to_le_bytes(),
    );
    array(&mut bytes, DIVISION + cunit::VIEWER_DATA, VIEWERS, 2);
    for (index, (viewer, level)) in [(1_u32, 3_u8), (2, 255)].into_iter().enumerate() {
        put(
            &mut bytes,
            VIEWERS + index * unit_viewer_entry::STRIDE,
            &viewer.to_le_bytes(),
        );
        put(
            &mut bytes,
            VIEWERS + index * unit_viewer_entry::STRIDE + unit_viewer_entry::LEVEL,
            &[level],
        );
    }
    array(&mut bytes, COUNTRY + ccountry::FLEETS, 0x11400, 1);
    put(&mut bytes, 0x11400, &(FLEET as u64).to_le_bytes());
    put(
        &mut bytes,
        FLEET,
        &(cfleet::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    array(&mut bytes, FLEET + cfleet::TASK_FORCES, 0x11500, 1);
    put(&mut bytes, 0x11500, &(TASK_FORCE as u64).to_le_bytes());
    put(
        &mut bytes,
        TASK_FORCE,
        &(ctask_force::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        TASK_FORCE + cunit::HANDLE,
        &((8_u64 << 32) | 61).to_le_bytes(),
    );
    put(
        &mut bytes,
        BASE + cgame_state::STRATEGIC_AIR_MANAGER,
        &(MANAGER as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        MANAGER,
        &(engine_native_air::cstrategic_air_manager::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    array(
        &mut bytes,
        MANAGER + engine_native_air::cstrategic_air_manager::STATE_AIR_BASES,
        0x11600,
        1,
    );
    put(&mut bytes, 0x11600, &(AIR_BASE as u64).to_le_bytes());
    put(
        &mut bytes,
        AIR_BASE,
        &(engine_native_parts::cair_base_layout::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        AIR_BASE + engine_native_parts::cair_base_layout::HANDLE,
        &((4_u64 << 32) | u64::from(engine_native_parts::cair_base_layout::TYPE)).to_le_bytes(),
    );
    array(
        &mut bytes,
        AIR_BASE + cair_base::COUNTRY_CONTAINERS,
        0x11700,
        2,
    );
    put(&mut bytes, 0x11708, &(CONTAINER as u64).to_le_bytes());
    put(
        &mut bytes,
        CONTAINER,
        &(engine_native_air::ccountry_air_container::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        CONTAINER + engine_native_air::ccountry_air_container::COUNTRY,
        &2_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        CONTAINER + engine_native_air::ccountry_air_container::CAPACITY_RAW,
        &engine_native_air::ccountry_air_container::INITIAL_CAPACITY_RAW.to_le_bytes(),
    );
    array(
        &mut bytes,
        CONTAINER + engine_native_air::ccountry_air_container::POOLS,
        0x11800,
        1,
    );
    put(&mut bytes, 0x11800, &(POOL as u64).to_le_bytes());
    put(
        &mut bytes,
        POOL,
        &(engine_native_parts::cair_wing_pool::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    array(
        &mut bytes,
        POOL + engine_native_parts::cair_wing_pool::WINGS,
        0x11900,
        1,
    );
    put(&mut bytes, 0x11900, &(WING as u64).to_le_bytes());
    put(
        &mut bytes,
        WING,
        &(engine_native_parts::cair_wing_layout::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        WING + engine_native_parts::cair_wing_layout::SECONDARY,
        &(engine_native_parts::cair_wing_layout::SECONDARY_VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        WING + engine_native_parts::cair_wing_layout::REFERENCE_TYPE,
        &((77_u64 << 32) | u64::from(engine_native_parts::cair_wing_layout::TYPE)).to_le_bytes(),
    );
    put(
        &mut bytes,
        WING + engine_native_parts::cair_wing_layout::POOL,
        &(POOL as u64).to_le_bytes(),
    );
    bytes
}

fn small() -> (Capacities, Captured) {
    let capacities = Capacities {
        rows: [4; N],
        radar_provinces: 8,
        area_provinces: 8,
        wings: 8,
    };
    let captured = Captured::with_capacities(&capacities);
    (capacities, captured)
}

fn complete<S: TableSchema>(view: &TableView) -> &[S::Row] {
    let TableView::Complete(rows) = view else {
        panic!("expected complete table, found {view:?}")
    };
    rows.rows::<S>().unwrap()
}

#[test]
fn every_family_keeps_native_order_widths_and_absent_values() {
    let (_, mut captured) = small();
    let bytes = fixture();
    let memory = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let _ticks = captured.capture(&memory, 0, BASE, 0, true);
    let views = captured.tables();
    assert_eq!(
        complete::<tables::RadarCoverage>(&views[0]),
        &[rows::RadarCoverageRow {
            country: 2,
            ordinal: 0,
            state: Some(33),
            provinces: vec![11, 12],
            radius: None,
        }]
    );
    assert_eq!(
        complete::<tables::VolunteerHosts>(&views[1]),
        &[rows::VolunteerHostRow {
            country: 2,
            position: 0,
            host: 9,
            deployed_divisions: -3,
            air_capacity_modifier: None.into(),
        }]
    );
    assert_eq!(
        complete::<tables::TheatreAreas>(&views[2]),
        &[rows::TheatreAreaRow {
            theatre: (5 << 32) | u64::from(ctheatre::REFERENCE_TYPE),
            ordinal: 0,
            representative: Some(21),
            provinces: vec![21, 22],
        }]
    );
    assert_eq!(
        complete::<tables::StationedUnits>(&views[3]),
        &[rows::StationedUnitRow {
            theatre: (5 << 32) | u64::from(ctheatre::REFERENCE_TYPE),
            ordinal: 0,
            unit: (6 << 32) | 1,
        }]
    );
    assert_eq!(
        complete::<tables::SentVolunteers>(&views[6]),
        &[
            rows::SentVolunteerRow {
                country: 2,
                ordinal: 0,
                division: 0,
            },
            rows::SentVolunteerRow {
                country: 2,
                ordinal: 1,
                division: 0xf000_0001_0000_0013,
            },
        ]
    );
    let intel = complete::<tables::UnitIntel>(&views[4]);
    assert_eq!(intel.len(), 2);
    assert_eq!(
        (
            intel[0].viewer,
            intel[0].level,
            intel[1].viewer,
            intel[1].level
        ),
        (1, 3, 2, -1)
    );
    assert_eq!(
        complete::<tables::AirBaseRegistrations>(&views[5]),
        &[rows::AirBaseRegistrationRow {
            base: (4 << 32) | u64::from(engine_native_parts::cair_base_layout::TYPE),
            slot: 1,
            capacity_raw: engine_native_air::ccountry_air_container::INITIAL_CAPACITY_RAW,
            wings: vec![(77 << 32) | u64::from(engine_native_parts::cair_wing_layout::TYPE)],
            carrier: rows::Recorded::Value(None),
            transport_planes: rows::Recorded::Value(rows::Retired),
            fuel_demand_raw: rows::Recorded::Value(None),
            fuel_allocated_raw: rows::Recorded::Value(None),
        }]
    );
}

#[test]
fn short_arenas_grow_to_the_full_census_in_one_capture() {
    let capacities = Capacities {
        rows: [4; N],
        radar_provinces: 1,
        area_provinces: 8,
        wings: 8,
    };
    let mut captured = Captured::with_capacities(&capacities);
    let bytes = fixture();
    let memory = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let _ticks = captured.capture(&memory, 0, BASE, 0, true);
    assert_eq!(captured.outcomes()[0], Outcome::Rows { len: 1 });
}

#[test]
fn torn_sources_stay_unavailable_and_never_become_empty_tables() {
    let (_, mut captured) = small();
    let mut bytes = fixture();
    put(&mut bytes, 0x10e00, &0_u64.to_le_bytes());
    put(
        &mut bytes,
        RADAR_GROUP + radar_group::PARENT,
        &0x1234_u64.to_le_bytes(),
    );
    let memory = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let _ticks = captured.capture(&memory, 0, BASE, 0, true);
    let outcomes = captured.outcomes();
    assert!(matches!(outcomes[0], Outcome::Unavailable { .. }));
    assert!(matches!(outcomes[3], Outcome::Unavailable { .. }));
    assert_eq!(outcomes[2], Outcome::Rows { len: 1 });
    let views = captured.tables();
    assert!(matches!(views[0], TableView::Unavailable { .. }));
    assert!(matches!(views[3], TableView::Unavailable { .. }));
}

#[test]
fn an_empty_world_produces_honest_empty_tables() {
    let (_, mut captured) = small();
    let mut bytes = fixture();
    for (address, count) in [
        (COUNTRY + ccountry::RADAR_GROUPS_COUNT, 0_u32),
        (COUNTRY + ccountry::VOLUNTEER_HOSTS_COUNT, 0),
        (COUNTRY + ccountry::VOLUNTEER_TALLY_COUNT, 0),
        (COUNTRY + ccountry::SENT_VOLUNTEERS + 12, 0),
        (COUNTRY + ccountry::THEATRES_COUNT, 0),
        (DIVISION + cunit::VIEWER_COUNT, 0),
        (AIR_BASE + cair_base::COUNTRY_CONTAINERS_COUNT, 0),
    ] {
        put(&mut bytes, address, &count.to_le_bytes());
    }
    let memory = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let _ticks = captured.capture(&memory, 0, BASE, 0, true);
    assert_eq!(captured.outcomes(), [Outcome::Rows { len: 0 }; N]);
    for view in captured.tables() {
        assert!(matches!(&view, TableView::Complete(rows) if rows.is_empty()));
    }
}

#[test]
fn free_fog_acquisition_preserves_frame_state_until_owned_decode() {
    let (_, mut captured) = small();
    let bytes = fixture();
    let memory = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let before = captured.outcomes();
    let radar_ctx = captured.radar_copy_context(0, BASE, 0, true);
    let intel_ctx = captured.intel_copy_context(0, BASE);
    assert_eq!(
        (
            radar_ctx.row_limit,
            radar_ctx.province_limit,
            intel_ctx.row_limit
        ),
        (4, 8, 4)
    );
    let mut radar_first = RadarJob::with_storage(4, 8);
    let mut intel_first = IntelJob::with_storage(4, 0);
    let radar = copy_radar_refresh(&memory, radar_ctx, &mut radar_first);
    let intel = copy_intel_refresh_with_divisions(&memory, intel_ctx, &mut intel_first, None);
    assert!(radar.raw_ok && intel.raw_ok);
    assert_eq!(captured.outcomes(), before);
    captured.decode_radar_payload(&radar_first, &mut [0]);
    captured.decode_intel_payload(&intel_first, &mut [0]);
    assert!(matches!(captured.outcomes()[0], Outcome::Rows { len: 1 }));
    assert!(matches!(captured.outcomes()[4], Outcome::Rows { len: 2 }));
}

#[test]
fn accepted_fog_failures_are_terminal_and_leave_other_tables_independent() {
    struct Reserved<'a>(SliceMemory<'a>);
    impl Memory for Reserved<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            assert_ne!(
                address,
                COUNTRY + ccountry::RADARS_POOL,
                "reserved radar retried"
            );
            assert_ne!(
                address,
                DIVISION + cunit::VIEWER_DATA,
                "reserved division intel retried"
            );
            assert_ne!(
                address,
                TASK_FORCE + cunit::VIEWER_DATA,
                "reserved naval intel retried"
            );
            assert_ne!(
                address,
                BASE + cgame_state::STRATEGIC_AIR_MANAGER,
                "reserved registrations retried"
            );
            self.0.copy(address, output)
        }
    }
    let (_, mut captured) = small();
    let bytes = fixture();
    let memory = Reserved(SliceMemory {
        base: BASE,
        bytes: &bytes,
    });
    captured.radar_raw_mode = true;
    captured.intel_raw_mode = true;
    captured.registrations_raw_mode = true;
    let ticks = captured.capture(&memory, 0, BASE, 0, true);
    assert_eq!((ticks[0], ticks[4], ticks[5]), (0, 0, 0));
    let before = captured.outcomes();
    captured.fail_radar_capture("radar worker panicked");
    assert_eq!(
        captured.radar.outcome,
        Outcome::Unavailable {
            reason: "radar worker panicked"
        }
    );
    assert_eq!(captured.intel.outcome, before[4]);
    assert!(!captured.radar_raw_mode && captured.intel_raw_mode);
    assert_eq!(
        (
            captured.radar_provinces.len,
            captured.radar_provinces.required
        ),
        (0, 0)
    );
    captured.fail_intel_capture("intel worker stopped");
    assert_eq!(
        captured.intel.outcome,
        Outcome::Unavailable {
            reason: "intel worker stopped"
        }
    );
    assert!(!captured.intel_raw_mode);
    let after = captured.outcomes();
    for index in [1, 2, 3, 5] {
        assert_eq!(after[index], before[index]);
    }
    assert!(matches!(
        captured.tables()[0],
        TableView::Unavailable { .. }
    ));
    assert!(matches!(
        captured.tables()[4],
        TableView::Unavailable { .. }
    ));
    assert!(captured.registrations_raw_mode);
    captured.fail_registrations_capture("shared bases worker panicked");
    assert_eq!(
        captured.registrations.outcome,
        Outcome::Unavailable {
            reason: "shared bases worker panicked"
        }
    );
    assert!(!captured.registrations_raw_mode);
    assert_eq!((captured.wings.len, captured.wings.required), (0, 0));
    assert_eq!(&captured.outcomes()[..5], &after[..5]);
    assert!(matches!(
        captured.tables()[5],
        TableView::Unavailable { .. }
    ));
    captured.fail_registrations_capture("shared bases worker stopped");
    assert!(matches!(
        captured.tables()[5],
        TableView::Unavailable { .. }
    ));
}

#[test]
fn areas_and_stationed_units_share_one_traversal_every_hour() {
    struct Counted<'a> {
        inner: SliceMemory<'a>,
        unit_roots: std::cell::Cell<usize>,
        area_roots: std::cell::Cell<usize>,
    }
    impl Memory for Counted<'_> {
        fn copy(&self, address: usize, bytes: &mut [u8]) -> bool {
            if address == THEATRE + ctheatre::STATIONED_UNIT_DATA {
                self.unit_roots.set(self.unit_roots.get() + 1);
            }
            if address == THEATRE + ctheatre::CONTROLLER_AREA_DATA {
                self.area_roots.set(self.area_roots.get() + 1);
            }
            self.inner.copy(address, bytes)
        }
    }
    let bytes = fixture();
    let mut captured = Captured::with_capacities(&Capacities {
        rows: [4; N],
        radar_provinces: 8,
        area_provinces: 8,
        wings: 8,
    });
    for hour in 0..24 {
        assert!(replay_data::cadence::should_walk(
            TableId::TheatreAreas,
            hour,
            false
        ));
        let memory = Counted {
            inner: SliceMemory {
                base: BASE,
                bytes: &bytes,
            },
            unit_roots: std::cell::Cell::new(0),
            area_roots: std::cell::Cell::new(0),
        };
        let _ticks = captured.capture(&memory, 0, BASE, hour, false);
        assert_eq!(captured.areas.outcome, Outcome::Rows { len: 1 }, "{hour}");
        assert_eq!(captured.units.outcome, Outcome::Rows { len: 1 }, "{hour}");
        assert_eq!(memory.unit_roots.get(), 1);
        assert!(memory.area_roots.get() >= 1);
    }
    let _ = captured.capture(
        &SliceMemory {
            base: BASE,
            bytes: &[],
        },
        0,
        BASE,
        0,
        false,
    );
    assert!(matches!(
        captured.units.outcome,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn radar_split_copy_matches_direct_walk() {
    use super::radar_test_kit as kit;
    use engine_native_memory::memory::{CountedMemory, ReadCounts};
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let direct = kit::direct(&memory);
    let memory = CountedMemory::new(memory);
    let capture = read_accounting::Capture::new();
    let (split, report, _) = kit::split(&memory);
    let counts = capture.finish();
    assert!(report.raw_ok);
    kit::assert_matches_direct(&split, &direct);
    assert_eq!(split.outcome, Outcome::Rows { len: 1 });
    assert_eq!(
        (
            split.rows[0].country,
            split.rows[0].ordinal,
            split.rows[0].state
        ),
        (1, 0, Some(77))
    );
    let anchor = TableId::RadarCoverage;
    let mut expected = read_accounting::ZERO;
    expected[anchor.index()] = ReadCounts {
        copy_calls: 17,
        scattered_calls: 4,
        record_calls: 0,
        plan_calls: 0,
        spans: 27,
        bytes: 204,
    };
    assert_eq!(counts, expected);
}

#[test]
fn radar_truncation_matrix_matches_direct_with_same_caps() {
    use super::radar_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    for (rows, provinces) in [(0, 0), (1, 1), (1, 2), (64, 64)] {
        let direct = kit::direct_with(&memory, rows, provinces);
        let (split, report, _) = kit::split_with(&memory, rows, provinces);
        assert!(report.raw_ok, "rows={rows} provinces={provinces}");
        kit::assert_matches_direct(&split, &direct);
    }
    let direct = kit::direct_with(&memory, 1, 2);
    assert_eq!(direct.outcome, Outcome::Rows { len: 1 });
}

#[test]
fn radar_split_copy_matches_direct_walk_on_fault_matrix() {
    use super::radar_test_kit as kit;
    use engine_native_memory::memory::SliceMemory;
    struct Fault<'a> {
        memory: SliceMemory<'a>,
        fault: std::ops::Range<usize>,
    }
    impl engine_native_memory::Memory for Fault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let Some(end) = address.checked_add(output.len()) else {
                return false;
            };
            if address < self.fault.end && end > self.fault.start {
                return false;
            }
            self.memory.copy(address, output)
        }
    }
    for fault in [
        kit::GAME_STATE + engine_native::cgame_state::COUNTRIES,
        kit::C0,
        kit::C0 + engine_native::ccountry::RADARS_POOL,
        kit::GROUP,
        kit::GROUP + engine_map_air::radar_group::STATE,
    ] {
        let bytes = kit::fixture();
        let memory = Fault {
            memory: SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            fault: fault..fault + 1,
        };
        let direct = kit::direct(&memory);
        let (split, report, _) = kit::split(&memory);
        assert!(report.raw_ok, "fault={fault:x}");
        kit::assert_matches_direct(&split, &direct);
    }
}

#[test]
fn intel_split_copy_matches_direct_walk() {
    use super::intel_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let direct = kit::direct(&memory);
    let (split, report, _) = kit::split(&memory);
    assert!(report.raw_ok);
    kit::assert_matches_direct(&split, &direct);
    assert_eq!(split.outcome, Outcome::Rows { len: 2 });
    assert_eq!((split.rows[0].viewer, split.rows[0].level), (1, 3));
    assert_eq!((split.rows[1].viewer, split.rows[1].level), (2, -1));
    kit::assert_paths_match_direct(&split, &direct);
}

#[test]
fn intel_truncation_matrix_matches_direct_with_same_caps() {
    use super::intel_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    for cap in [0, 1, 2, 64] {
        let direct = kit::direct_with(&memory, cap);
        let (split, report, _) = kit::split_with(&memory, cap);
        assert!(report.raw_ok, "cap={cap}");
        kit::assert_matches_direct(&split, &direct);
        kit::assert_paths_match_direct(&split, &direct);
    }
    let direct = kit::direct_with(&memory, 2);
    assert_eq!(direct.outcome, Outcome::Rows { len: 2 });
}

#[test]
fn intel_split_copy_matches_direct_walk_on_fault_matrix() {
    use super::intel_test_kit as kit;
    use engine_native_memory::memory::SliceMemory;
    struct Fault<'a> {
        memory: SliceMemory<'a>,
        fault: std::ops::Range<usize>,
    }
    impl engine_native_memory::Memory for Fault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let Some(end) = address.checked_add(output.len()) else {
                return false;
            };
            if address < self.fault.end && end > self.fault.start {
                return false;
            }
            self.memory.copy(address, output)
        }
    }
    for fault in [
        kit::GAME_STATE + engine_native::cgame_state::COUNTRIES,
        kit::C0,
        kit::C0 + engine_native::ccountry::DIVISIONS,
        kit::DIVISION,
        kit::DIVISION + engine_native::cunit::HANDLE,
        kit::VIEWERS,
        kit::C0 + engine_native::ccountry::FLEETS,
    ] {
        let bytes = kit::fixture();
        let memory = Fault {
            memory: SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            fault: fault..fault + 1,
        };
        let direct = kit::direct(&memory);
        let (split, report, _) = kit::split(&memory);
        assert!(report.raw_ok, "fault={fault:x}");
        kit::assert_matches_direct(&split, &direct);
        kit::assert_paths_match_direct(&split, &direct);
    }
}
