use super::*;
use std::collections::BTreeMap;

struct World {
    route: Option<Vec<u32>>,
    regions: BTreeMap<u32, u32>,
    enemies: Vec<u32>,
    ledgers: BTreeMap<(u32, u32), Vec<u64>>,
    chance: Fact<i64>,
    distance: i64,
    speed: i64,
    visits: Vec<(Option<u32>, u32)>,
    speed_regions: Vec<Option<u32>>,
    missions: Vec<u64>,
}

impl Default for World {
    fn default() -> Self {
        Self {
            route: Some(vec![2, 3]),
            regions: [(1, 10), (2, 20), (3, 20)].into(),
            enemies: vec![7],
            ledgers: [((7, 20), vec![11])].into(),
            chance: Ok(Q15 / 2),
            distance: ONE / 2,
            speed: ONE,
            visits: Vec::new(),
            speed_regions: Vec::new(),
            missions: Vec::new(),
        }
    }
}

impl Inputs for World {
    fn route(&mut self, _: Unit, _: u32, _: u32) -> Fact<Option<Vec<u32>>> {
        Ok(self.route.clone())
    }
    fn region(&mut self, province: u32) -> Fact<Option<u32>> {
        Ok(self.regions.get(&province).copied())
    }
    fn province_present(&mut self, province: u32) -> Fact<bool> {
        Ok(self.regions.contains_key(&province))
    }
    fn enemies(&mut self, _: Unit) -> Fact<Vec<u32>> {
        Ok(self.enemies.clone())
    }
    fn ledger(&mut self, enemy: u32, region: u32) -> Fact<Vec<u64>> {
        Ok(self
            .ledgers
            .get(&(enemy, region))
            .cloned()
            .unwrap_or_default())
    }
    fn mission_chance(&mut self, receiver: u64, _: Unit, _: i32) -> Fact<i64> {
        self.missions.push(receiver);
        self.chance.clone()
    }
    fn distance(&mut self, _: Unit, previous: Option<u32>, current: u32) -> Fact<i64> {
        self.visits.push((previous, current));
        Ok(self.distance)
    }
    fn speed(&mut self, _: Unit, region: Option<u32>) -> Fact<i64> {
        self.speed_regions.push(region);
        Ok(self.speed)
    }
}

fn calculate(world: &mut World) -> Fact<i64> {
    query(Unit::Army(1), 1, 3, 9, world)
}

#[test]
fn false_or_empty_query_route_returns_before_region_enemy_and_mission_inputs() {
    for route in [None, Some(Vec::new())] {
        let mut world = World {
            route,
            regions: BTreeMap::new(),
            chance: Err("unavailable".into()),
            ..Default::default()
        };
        assert_eq!(calculate(&mut world), Ok(0));
        assert!(world.visits.is_empty() && world.missions.is_empty());
    }
}

#[test]
fn repeated_enemies_and_nullable_missions_fold_in_order_and_preserve_fractional_exposure() {
    let mut world = World {
        enemies: vec![7, 7],
        ledgers: [((7, 20), vec![0, 11, 11, 0])].into(),
        ..Default::default()
    };
    // Four half-chance occurrences give 1/16 survival, applied once after
    // two half-hour edges. A set or per-edge rounding produces a different risk.
    assert_eq!(calculate(&mut world), Ok(93_750));
    assert_eq!(world.missions, vec![11, 11, 11, 11]);
    assert_eq!(world.visits, vec![(Some(1), 2), (Some(2), 3)]);
    assert_eq!(world.speed_regions, vec![Some(10), Some(20)]);
}

#[test]
fn exposure_carries_between_risky_regions_but_a_unity_region_clears_it() {
    for (middle, expected) in [(20, 50_000), (30, 0)] {
        let mut world = World {
            route: Some(vec![2, 4, 3]),
            distance: ONE / 3 + 1,
            ..Default::default()
        };
        world.regions.insert(4, middle);
        assert_eq!(calculate(&mut world), Ok(expected));
    }
    let mut world = World::default();
    world.regions.insert(3, 30);
    world.ledgers.insert((7, 30), vec![12]);
    assert_eq!(calculate(&mut world), Ok(50_000));
    assert_eq!(world.missions, vec![11, 12]);
}

#[test]
fn null_province_preserves_exposure_but_clears_the_next_distance_source() {
    let mut world = World {
        route: Some(vec![2, 99, 3]),
        ..Default::default()
    };
    assert_eq!(calculate(&mut world), Ok(50_000));
    assert_eq!(world.visits, vec![(Some(1), 2), (None, 3)]);
    assert_eq!(world.speed_regions, vec![Some(10), Some(20)]);
}

#[test]
fn zero_speed_and_zero_survival_return_native_full_risk() {
    let mut world = World {
        speed: 0,
        ..Default::default()
    };
    assert_eq!(calculate(&mut world), Ok(ONE));
    assert_eq!(world.visits.len(), 1);
    let mut world = World {
        chance: Ok(Q15),
        distance: ONE,
        ..Default::default()
    };
    assert_eq!(calculate(&mut world), Ok(ONE));
    assert_eq!(world.visits.len(), 1);
}

#[test]
fn reached_mission_failures_refuse_but_empty_ledgers_do_not_demand_damage_or_speed() {
    let mut world = World {
        chance: Err("damage unavailable".into()),
        ..Default::default()
    };
    assert_eq!(calculate(&mut world), Err("damage unavailable".into()));
    world.ledgers.clear();
    assert_eq!(calculate(&mut world), Ok(0));
    assert!(world.visits.is_empty());
}

#[test]
fn emitted_chance_conversion_wraps_and_clamps() {
    assert_eq!(fixed5_to_q15(ONE), Q15);
    assert_eq!(fixed5_to_q15(-ONE), -Q15);
    assert_eq!(fixed5_to_q15(ONE / 2), Q15 / 2);
    assert_eq!(mission_chance(100 * ONE, 0, ONE, 0, 100 * ONE), Q15);
    assert_eq!(mission_chance(50 * ONE, 0, ONE, 0, 100 * ONE), Q15 / 2);
    assert_eq!(mission_chance(-50 * ONE, 0, ONE, 0, 100 * ONE), 0);
    assert_eq!(mission_chance(200 * ONE, 0, ONE, 0, 100 * ONE), Q15);
}
