use super::*;
use std::{cell::RefCell, collections::BTreeMap};

#[test]
fn source_to_id_distance_uses_native_wrap_sqrt_and_invalid_signed_slot_zero() {
    let map = ProvinceGeometry {
        width: 100,
        centroids: BTreeMap::from([(1, [98, 4]), (2, [2, 1])]),
        ..Default::default()
    };
    assert_eq!(distance(&map, 1, 2), 500_002);
    assert_eq!(distance(&map, 1, 1), 0);
    let zero = content_reader_map::province_geometry::native_fixed_sqrt(20);
    for invalid in [0, 300, u32::MAX] {
        assert_eq!(distance(&map, 1, invalid), zero);
    }
    let overflowing = ProvinceGeometry {
        width: 100,
        centroids: BTreeMap::from([(1, [i32::MIN, 0])]),
        ..Default::default()
    };
    assert_eq!(distance(&overflowing, 1, 0), 0);
}

#[test]
fn invasion_counts_distinguish_old_absence_refusal_zero_and_signed_values() {
    use replay_data::rows::Recorded;
    let mut plan = ArmyOrderPlanRow {
        invasion_transports: Recorded::NotRecorded,
        ..Default::default()
    };
    assert_eq!(
        invasion(&plan),
        Err("ArmyOrderPlans.invasion_transports not recorded".to_owned())
    );
    plan.invasion_transports = Recorded::Value(None);
    assert_eq!(
        invasion(&plan),
        Err("the invasion order's allocated convoy count is unavailable".to_owned())
    );
    for value in [0, -3, i32::MAX] {
        plan.invasion_transports = Recorded::Value(Some(value));
        assert_eq!(invasion(&plan), Ok(value));
    }
}

#[derive(Default)]
struct Fixture {
    wings: BTreeMap<u64, AirWingRow>,
    calls: RefCell<Vec<(&'static str, u64)>>,
    transports: bool,
    range: i64,
    transfer: Option<bool>,
    region: Option<bool>,
}

impl Fixture {
    fn insert_wing(&mut self, id: u64, planes: i32) -> &mut AirWingRow {
        self.wings.entry(id).or_insert(AirWingRow {
            id,
            planes,
            ..Default::default()
        })
    }
    fn note(&self, field: &'static str, id: u64) {
        self.calls.borrow_mut().push((field, id));
    }
}

impl Sources for Fixture {
    fn wing(&self, id: u64) -> Result<&AirWingRow, String> {
        self.note("wing", id);
        self.wings.get(&id).ok_or_else(|| "missing wing".to_owned())
    }
    fn is_transport_pool(&self, wing: &AirWingRow) -> Result<bool, String> {
        self.note("pool", wing.id);
        Ok(self.transports)
    }
    fn range(&self, wing: &AirWingRow) -> Result<i64, String> {
        self.note("range", wing.id);
        Ok(self.range)
    }
    fn transfer_resolves(&self, handle: u64) -> Result<bool, String> {
        self.note("transfer", handle);
        self.transfer
            .ok_or_else(|| "missing transfer witness".to_owned())
    }
    fn region_admits(&self, wing: &AirWingRow) -> Result<bool, String> {
        self.note("region", wing.id);
        self.region
            .ok_or_else(|| "missing region inputs".to_owned())
    }
}

#[test]
fn repeats_and_signed_planes_keep_the_native_wrapping_sum() {
    let mut source = Fixture {
        transports: true,
        range: 300_000,
        ..Default::default()
    };
    source.insert_wing(1, i32::MAX);
    source.insert_wing(2, -3);
    assert_eq!(count(&[1, 1, 2], 300_000, false, &source), Ok(-5));
    assert!(
        !source
            .calls
            .borrow()
            .iter()
            .any(|&(field, _)| field == "region")
    );
    assert_eq!(count(&[], 0, true, &Fixture::default()), Ok(0));
}

#[test]
fn unreached_transfer_and_region_failures_do_not_poison_earlier_gates() {
    let mut source = Fixture {
        transports: true,
        range: 300_000,
        ..Default::default()
    };
    let wing = source.insert_wing(1, 7);
    wing.base_override = 9;
    wing.mission_mask = 1;
    assert_eq!(count(&[1], 0, true, &source), Ok(0));
    assert_eq!(
        source.calls.take(),
        vec![("wing", 1), ("pool", 1), ("range", 1)]
    );
    source.wings.get_mut(&1).unwrap().mission_mask = 0;
    assert_eq!(
        count(&[1], 0, true, &source),
        Err("missing transfer witness".to_owned())
    );
    source.transfer = Some(true);
    assert_eq!(count(&[1], 0, true, &source), Ok(0));
    source.transfer = Some(false);
    assert_eq!(
        count(&[1], 0, true, &source),
        Err("missing region inputs".to_owned())
    );
    source.region = Some(true);
    assert_eq!(count(&[1], 0, true, &source), Ok(7));
    source.transports = false;
    source.calls.take();
    assert_eq!(count(&[1], 0, true, &source), Ok(0));
    assert_eq!(source.calls.take(), vec![("wing", 1), ("pool", 1)]);
}

#[test]
fn veto_counter_and_exact_range_boundary_match_native_admission() {
    let mut source = Fixture {
        transports: true,
        range: 300_000,
        ..Default::default()
    };
    let wing = source.insert_wing(1, 7);
    wing.eligibility_counter = -2;
    wing.eligibility_limit = -2;
    assert_eq!(count(&[1], 300_000, false, &source), Ok(7));
    assert_eq!(count(&[1], 300_001, false, &source), Ok(0));
    source.wings.get_mut(&1).unwrap().eligibility_counter = -1;
    assert_eq!(count(&[1], 300_000, false, &source), Ok(0));
    source.wings.get_mut(&1).unwrap().eligibility_counter = -2;
    source.wings.get_mut(&1).unwrap().eligibility_veto = 1;
    assert_eq!(count(&[1], 300_000, false, &source), Ok(0));
}
