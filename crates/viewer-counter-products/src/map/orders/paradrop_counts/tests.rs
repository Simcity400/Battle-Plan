use super::*;
use content_reader_map::province_geometry::ProvinceGeometry;
use std::collections::BTreeMap;

use replay_data::{
    rows::AirBaseRegistrationRow, rows::AirBaseRow, rows::AirReferenceWitnessRow,
    rows::AirWingRaidReference, rows::CountryLinkRow, rows::ProvinceRow, rows::Recorded,
    rows::ShipRow, rows::TableId, rows::TableSchema,
};
use replay_frame::{frame::Metrics, frame::TableView};

pub(in crate::map::orders) struct Fixture {
    pub(in crate::map::orders) frame: DecodedFrame,
    geometry: ProvinceGeometry,
    states: BTreeMap<u32, Vec<u32>>,
    region_by_province: BTreeMap<u32, u32>,
    regions: BTreeMap<u32, Vec<u32>>,
    units: content_reader_script::sub_units::SubUnits,
}

impl Fixture {
    pub(in crate::map::orders) fn new() -> Self {
        let mut out = Self::without_unit_content();
        out.units.read(
            b"sub_units={Cargo={type=air_transport} Fighter={type=fighter}}",
            "fixture.txt",
        );
        out
    }

    fn without_unit_content() -> Self {
        let mut out = Self {
            frame: DecodedFrame {
                branch: 1,
                hour: 2,
                metrics: Metrics::default(),
                tables: std::array::from_fn(|_| {
                    TableView::Unavailable {
                        reason: "fixture did not provide this table".into(),
                    }
                    .into()
                }),
            },
            geometry: ProvinceGeometry {
                width: 100,
                height: 30,
                centroids: BTreeMap::from([(1, [10, 10]), (2, [14, 10]), (3, [70, 10])]),
                bounds: BTreeMap::from([(1, [9, 9, 2, 2]), (2, [13, 9, 2, 2]), (3, [69, 9, 2, 2])]),
                ..Default::default()
            },
            states: BTreeMap::from([(10, vec![1]), (20, vec![3])]),
            region_by_province: BTreeMap::new(),
            regions: BTreeMap::new(),
            units: Default::default(),
        };
        out.put::<tables::Provinces>(vec![ProvinceRow {
            id: 1,
            state: 10,
            ..Default::default()
        }]);
        out.put::<tables::CountryLinks>(vec![CountryLinkRow {
            id: 77,
            canonical_slot: 4,
        }]);
        out.put::<tables::AirBases>(vec![AirBaseRow {
            id: 100,
            state: Some(10),
            province: Some(1),
            kind: 0,
            ..Default::default()
        }]);
        out.put::<tables::AirBaseRegistrations>(vec![AirBaseRegistrationRow {
            base: 100,
            slot: 4,
            capacity_raw: 0,
            wings: vec![200],
            carrier: Default::default(),
            transport_planes: Default::default(),
            fuel_demand_raw: Default::default(),
            fuel_allocated_raw: Default::default(),
        }]);
        out.put::<tables::AirWings>(vec![AirWingRow {
            id: 200,
            owner: 77,
            air_base: 100,
            planes: 9,
            base_range_raw: 600_000,
            pool_subunit: Recorded::Value(Some(1)),
            raid_reference: Recorded::Value(Some(AirWingRaidReference {
                raw: u64::MAX,
                resolves: false,
            })),
            ..Default::default()
        }]);
        out.put::<tables::RaidCountryInputs>(vec![RaidCountryInputRow {
            canonical_slot: 4,
            air_range_factor: Some(0),
            ..Default::default()
        }]);
        out
    }

    fn put<T: TableSchema>(&mut self, mut rows: Vec<T::Row>) {
        use replay_data::rows::RowCodec;
        rows.sort_by_key(RowCodec::key);
        self.frame.tables[T::ID.index()] = TableView::Complete(T::wrap(rows)).into();
    }

    pub(in crate::map::orders) fn context(&self) -> Context<'_> {
        Context {
            frame: &self.frame,
            registry: OnceCell::new(),
            content: Some(AirTransportContent {
                geometry: &self.geometry,
                state_provinces: &self.states,
                region_by_province: &self.region_by_province,
                region_provinces: &self.regions,
                sub_units: Ok(&self.units),
                map_scale: 100_000,
                ace_wing_size: 0,
                ace_wing_size_max_bonus: 0,
            }),
        }
    }

    fn region(&mut self) {
        self.region_by_province.insert(2, 30);
        self.regions.insert(30, vec![2]);
        self.put::<tables::AirReferenceWitnesses>(vec![AirReferenceWitnessRow {
            handle: 100,
            ..Default::default()
        }]);
    }

    fn wing(&mut self, change: impl FnOnce(&mut AirWingRow)) {
        let mut wings = self.frame.complete::<tables::AirWings>().unwrap().to_vec();
        change(&mut wings[0]);
        self.put::<tables::AirWings>(wings);
    }
}

fn order() -> ArmyOrderRow {
    ArmyOrderRow {
        owner_group: 10,
        order_type: 4,
        country: 999,
        paradrop_target: 1,
        path: vec![2],
        ..Default::default()
    }
}
fn group() -> OrdersGroupRow {
    OrdersGroupRow {
        handle: 10,
        country: 77,
        ..Default::default()
    }
}

#[test]
fn no_source_state_base_or_container_returns_zero_without_later_tables() {
    let mut fixture = Fixture::without_unit_content();
    let mut order = order();
    let empty = Context::new(&fixture.frame, None);
    for source in [0, u32::MAX] {
        order.paradrop_target = source;
        assert_eq!(empty.count(&order, None), Ok(0));
    }
    order.paradrop_target = 1;
    fixture.put::<tables::Provinces>(vec![ProvinceRow {
        id: 1,
        state: 0,
        ..Default::default()
    }]);
    assert_eq!(
        Context::new(&fixture.frame, None).count(&order, None),
        Ok(0)
    );
    fixture.put::<tables::Provinces>(vec![ProvinceRow {
        id: 1,
        state: 10,
        ..Default::default()
    }]);
    fixture.put::<tables::AirBases>(vec![]);
    assert_eq!(fixture.context().count(&order, None), Ok(0));
    fixture = Fixture::without_unit_content();
    fixture.put::<tables::AirBaseRegistrations>(vec![]);
    assert_eq!(
        Context::new(&fixture.frame, None).count(&order, Some(&group())),
        Ok(0)
    );
}

#[test]
fn owner_group_container_preserves_repeated_signed_planes_and_ignores_capacity() {
    content_reader_script::test_game::enter();
    let mut fixture = Fixture::new();
    fixture.put::<tables::AirBaseRegistrations>(vec![AirBaseRegistrationRow {
        base: 100,
        slot: 4,
        wings: vec![200, 200],
        capacity_raw: -1,
        carrier: Default::default(),
        transport_planes: Default::default(),
        fuel_demand_raw: Default::default(),
        fuel_allocated_raw: Default::default(),
    }]);
    fixture.wing(|wing| wing.planes = i32::MAX);
    assert_eq!(fixture.context().count(&order(), Some(&group())), Ok(-2));
}

#[test]
fn nontransport_pool_and_earlier_idle_gates_do_not_demand_later_operands() {
    content_reader_script::test_game::enter();
    let mut fixture = Fixture::new();
    fixture.wing(|wing| wing.pool_subunit = Recorded::Value(Some(0)));
    assert_eq!(
        Context::new(&fixture.frame, None).count(&order(), Some(&group())),
        Ok(0)
    );
    fixture.wing(|wing| wing.pool_subunit = Recorded::Value(Some(2)));
    fixture.frame.tables[TableId::RaidCountryInputs.index()] = TableView::Unavailable {
        reason: "unreached range".into(),
    }
    .into();
    assert_eq!(fixture.context().count(&order(), Some(&group())), Ok(0));
    let mut fixture = Fixture::new();
    fixture.region();
    fixture.wing(|wing| {
        wing.mission_mask = 1;
        wing.raid_reference = Recorded::NotRecorded;
    });
    fixture.frame.tables[TableId::AirReferenceWitnesses.index()] = TableView::Unavailable {
        reason: "unreached registry".into(),
    }
    .into();
    assert_eq!(fixture.context().count(&order(), Some(&group())), Ok(0));
}

#[test]
fn missing_pool_identity_and_uncertain_content_refuse_instead_of_guessing() {
    content_reader_script::test_game::enter();
    let mut fixture = Fixture::new();
    fixture.wing(|wing| wing.pool_subunit = Recorded::NotRecorded);
    assert!(
        fixture
            .context()
            .count(&order(), Some(&group()))
            .unwrap_err()
            .contains("AirWings.pool_subunit not recorded")
    );
    fixture.wing(|wing| wing.pool_subunit = Recorded::Value(None));
    assert!(
        fixture
            .context()
            .count(&order(), Some(&group()))
            .unwrap_err()
            .contains("pool sub-unit identity")
    );
    fixture.wing(|wing| wing.pool_subunit = Recorded::Value(Some(1)));
    fixture.units.read(
        b"sub_units={Cargo={weights={x @variable=1}}}",
        "unsupported.txt",
    );
    assert!(fixture.context().count(&order(), Some(&group())).is_err());
}

#[test]
fn transfer_and_raid_liveness_use_their_witnesses_without_raid_roster_inference() {
    content_reader_script::test_game::enter();
    let mut fixture = Fixture::new();
    fixture.region();
    assert_eq!(fixture.context().count(&order(), Some(&group())), Ok(9));
    fixture.wing(|wing| {
        wing.raid_reference = Recorded::Value(Some(AirWingRaidReference {
            raw: 123,
            resolves: true,
        }))
    });
    assert_eq!(fixture.context().count(&order(), Some(&group())), Ok(0));
    fixture.wing(|wing| wing.raid_reference = Recorded::NotRecorded);
    assert!(
        fixture
            .context()
            .count(&order(), Some(&group()))
            .unwrap_err()
            .contains("AirWings.raid_reference not recorded")
    );
    fixture.wing(|wing| wing.base_override = 100);
    assert_eq!(fixture.context().count(&order(), Some(&group())), Ok(0));
    fixture.wing(|wing| wing.base_override = 999);
    assert!(fixture.context().count(&order(), Some(&group())).is_err());
}

#[test]
fn region_reach_uses_the_wings_pool_base_not_the_orders_source_base() {
    content_reader_script::test_game::enter();
    let mut fixture = Fixture::new();
    fixture.region();
    fixture.wing(|wing| wing.air_base = 101);
    fixture.put::<tables::AirReferenceWitnesses>(vec![AirReferenceWitnessRow {
        handle: 101,
        base_kind: Some(0),
        state: Some(20),
        province: Some(3),
        ..Default::default()
    }]);
    assert_eq!(fixture.context().count(&order(), Some(&group())), Ok(0));
    fixture.put::<tables::AirReferenceWitnesses>(vec![]);
    fixture.wing(|wing| wing.raid_reference = Recorded::NotRecorded);
    assert_eq!(fixture.context().count(&order(), Some(&group())), Ok(0));
}

#[test]
fn resolved_carrier_requires_its_task_force_mission_and_uses_its_province_anchor() {
    content_reader_script::test_game::enter();
    let mut fixture = Fixture::new();
    fixture.region();
    fixture.wing(|wing| wing.air_base = 101);
    fixture.put::<tables::AirReferenceWitnesses>(vec![AirReferenceWitnessRow {
        handle: 101,
        base_kind: Some(0),
        host: Some(400),
        host_owner: Some(77),
        ..Default::default()
    }]);
    assert!(
        fixture
            .context()
            .count(&order(), Some(&group()))
            .unwrap_err()
            .contains("Ships")
    );
    fixture.put::<tables::Ships>(vec![ShipRow {
        id: 400,
        task_force: 500,
        ..Default::default()
    }]);
    fixture.put::<tables::TaskForces>(vec![TaskForceRow {
        id: 500,
        province: 2,
        mission: 0,
        ..Default::default()
    }]);
    assert_eq!(fixture.context().count(&order(), Some(&group())), Ok(9));
    fixture.put::<tables::TaskForces>(vec![TaskForceRow {
        id: 500,
        province: 3,
        mission: 0,
        ..Default::default()
    }]);
    assert_eq!(fixture.context().count(&order(), Some(&group())), Ok(0));
    fixture.wing(|wing| wing.raid_reference = Recorded::NotRecorded);
    fixture.put::<tables::TaskForces>(vec![TaskForceRow {
        id: 500,
        province: 2,
        mission: 1,
        ..Default::default()
    }]);
    assert_eq!(fixture.context().count(&order(), Some(&group())), Ok(0));
    fixture.put::<tables::AirReferenceWitnesses>(vec![AirReferenceWitnessRow {
        handle: 101,
        base_kind: Some(0),
        host: Some(400),
        host_without_task_force: true,
        ..Default::default()
    }]);
    assert!(
        fixture
            .context()
            .count(&order(), Some(&group()))
            .unwrap_err()
            .contains("null task force")
    );
}

#[test]
fn nonzero_stale_state_host_uses_state_geometry_without_ship_tables() {
    content_reader_script::test_game::enter();
    let mut fixture = Fixture::new();
    fixture.region();
    fixture.put::<tables::AirBases>(vec![AirBaseRow {
        id: 100,
        state: Some(10),
        province: Some(1),
        host: u64::MAX,
        kind: 0,
        ..Default::default()
    }]);
    assert_eq!(fixture.context().count(&order(), Some(&group())), Ok(9));
}

#[test]
fn a_live_host_takes_precedence_over_the_state_for_mixed_base_geometry() {
    content_reader_script::test_game::enter();
    let mut fixture = Fixture::new();
    fixture.region();
    fixture.wing(|wing| wing.air_base = 101);
    fixture.put::<tables::Ships>(vec![ShipRow {
        id: 400,
        task_force: 500,
        ..Default::default()
    }]);
    fixture.put::<tables::AirReferenceWitnesses>(vec![AirReferenceWitnessRow {
        handle: 101,
        base_kind: Some(0),
        state: Some(20),
        province: Some(3),
        host: Some(400),
        host_owner: Some(77),
        ..Default::default()
    }]);
    fixture.put::<tables::TaskForces>(vec![TaskForceRow {
        id: 500,
        province: 2,
        mission: 0,
        ..Default::default()
    }]);
    assert_eq!(fixture.context().count(&order(), Some(&group())), Ok(9));
    fixture.put::<tables::AirReferenceWitnesses>(vec![AirReferenceWitnessRow {
        handle: 101,
        base_kind: Some(0),
        state: Some(10),
        province: Some(1),
        host: Some(400),
        host_owner: Some(77),
        ..Default::default()
    }]);
    fixture.put::<tables::TaskForces>(vec![TaskForceRow {
        id: 500,
        province: 3,
        mission: 0,
        ..Default::default()
    }]);
    assert_eq!(fixture.context().count(&order(), Some(&group())), Ok(0));
}

#[test]
fn missing_ace_range_is_reached_before_idle_mission_gates() {
    content_reader_script::test_game::enter();
    let mut fixture = Fixture::new();
    fixture.wing(|wing| {
        wing.ace = 700;
        wing.mission_mask = 1;
    });
    assert!(
        fixture
            .context()
            .count(&order(), Some(&group()))
            .unwrap_err()
            .contains("AceRangeInputs")
    );
}
