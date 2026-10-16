use super::*;
use std::{cell::RefCell, collections::BTreeMap};

#[test]
fn source_to_id_distance_uses_native_wrap_sqrt_and_invalid_signed_slot_zero() {
    let map = ProvinceGeometry {
        width: 100,
        centroids: BTreeMap::from([(1, [98, 4]), (2, [2, 1])]),
        ..Default::default()
    };
    // Newton iteration stops at its fixed5 tolerance.
    assert_eq!(distance(&map, 1, 2), 500_002);
    assert_eq!(distance(&map, 1, 1), 0);
    let zero = installed_content::province_geometry::native_fixed_sqrt(20);
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
    use replay::rows::Recorded;
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

fn map(bounds: &[(u32, Bounds)], centroids: &[(u32, [i32; 2])]) -> ProvinceGeometry {
    ProvinceGeometry {
        width: 100,
        height: 100,
        bounds: bounds.iter().copied().collect(),
        centroids: centroids.iter().copied().collect(),
        ..Default::default()
    }
}

#[test]
fn first_painted_province_and_strict_half_width_choose_split_boxes() {
    let geometry = map(
        &[(1, [10, 0, 2, 4]), (2, [80, 10, 2, 4]), (3, [49, 30, 2, 4])],
        &[],
    );
    let forward = RegionGeometry::new(&geometry, &[99, 1, 2, 3]);
    assert_eq!(forward.rectangles, [[10, 0, 41, 34], [80, 10, 2, 4]]);
    assert_eq!(forward.centre, [1, 17]);
    let reverse = RegionGeometry::new(&geometry, &[2, 1, 3]);
    assert_eq!(reverse.rectangles, [[80, 10, 2, 4], [10, 0, 41, 34]]);
    assert_eq!(reverse.centre, [1, 12]);
    let tied = map(&[(1, [10, 0, 2, 2]), (2, [60, 0, 2, 2])], &[]);
    assert_eq!(RegionGeometry::new(&tied, &[1, 2]).count, 1);
}

#[test]
fn native_prefilter_can_discard_a_province_at_the_range_boundary() {
    let geometry = map(&[(1, [40, 0, 2, 2])], &[(1, [40, 1])]);
    let region = RegionGeometry::new(&geometry, &[1]);
    assert_eq!(wrapped_squared([40, 1], [0, 1], 100), 1600);
    assert!(!region.reaches(&geometry, &[1], Origin::State([0, 1]), 4_000_000));
    assert!(region.reaches(&geometry, &[1], Origin::State([1, 1]), 4_000_000));
    assert!(region.reaches(&geometry, &[1], Origin::Host([1, 1]), 4_000_000));
}

#[test]
fn disjoint_split_y_spans_refuse_reach_before_the_province_scan() {
    let geometry = map(
        &[(1, [10, 0, 2, 2]), (2, [80, 20, 2, 2])],
        &[(1, [10, 0]), (2, [80, 20])],
    );
    let region = RegionGeometry::new(&geometry, &[1, 2]);
    assert!(!region.reaches(&geometry, &[1, 2], Origin::State([10, 0]), 10_000_000));
    assert!(!RegionGeometry::new(&geometry, &[]).reaches(
        &geometry,
        &[],
        Origin::State([0, 0]),
        10_000_000
    ));
}

#[test]
fn distance_wraps_once_and_preserves_i32_squared_overflow() {
    assert_eq!(wrapped_squared([0, 0], [50, 0], 100), 2500);
    assert_eq!(wrapped_squared([0, 0], [51, 0], 100), 2401);
    assert_eq!(wrapped_squared([0, 0], [251, 0], 100), 22801);
    assert_eq!(wrapped_squared([0, 50_000], [0, 0], 100), -1_794_967_296);
    assert_eq!(wrapped_squared([i32::MIN, 0], [0, 0], 100), 10_000);
    assert_eq!(province_squared([i32::MIN, 0], [0, 0], 100), 0);
}
