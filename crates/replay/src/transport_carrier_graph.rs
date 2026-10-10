//! The original ordered D5AF10 -> C23D90 -> C3E1D0 -> F47350 graph.
//!
//! Inputs borrow the current hour's original task-force, model and air passes.
//! An ordinal is an occurrence, not a set member: the native dense containers,
//! pools and wings may repeat. Implementors must authenticate every borrowed
//! owner/backlink and preserve the original signed counts and complete prefixes.
//! A missing original prefix is an error; this evaluator never performs a census
//! or retries a failed native observation.

pub type Fact<T> = Result<T, &'static str>;

/// Independently admitted leaves of one original SHIPS occurrence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ship {
    pub formation_flags: Fact<u64>,
    pub positive_equipment_flags: Fact<u64>,
    pub carrier_air_base: Fact<Option<u64>>,
}

/// Borrowed, authenticated original occurrences, addressed by native ordinal.
///
/// Base keys are opaque recorded identities. Neither a base nor any of its
/// repeated registrations is inferred from country tags or asset names.
pub trait Inputs {
    fn original_ship_count(&mut self) -> Fact<i32>;
    fn original_ship(&mut self, ordinal: u32) -> Fact<Ship>;
    fn carrier_capacity(&mut self, base: u64) -> Fact<i32>;
    fn registered_container_count(&mut self, base: u64) -> Fact<i32>;
    fn pool_count(&mut self, base: u64, container_ordinal: u32) -> Fact<i32>;
    fn wing_count(&mut self, base: u64, container_ordinal: u32, pool_ordinal: u32) -> Fact<i32>;
    fn wing_planes(
        &mut self,
        base: u64,
        container_ordinal: u32,
        pool_ordinal: u32,
        wing_ordinal: u32,
    ) -> Fact<i32>;
    /// Production borrows the original F47350 sum, sharing +7C attempts with
    /// the existing fuel observer. Default fixtures fold original occurrences.
    fn pool_planes(&mut self, base: u64, container: u32, pool: u32) -> Fact<i32> {
        let wings = count(self.wing_count(base, container, pool))?;
        let mut total = 0_i32;
        for wing in 0..wings {
            total = total.wrapping_add(self.wing_planes(base, container, pool, wing)?);
        }
        Ok(total)
    }
}

fn count(value: Fact<i32>) -> Fact<u32> {
    u32::try_from(value?).map_err(|_| "negative original carrier graph count")
}

/// C2B7E0's exact short-circuit predicate over an already observed model loan.
pub fn is_carrier(ship: Ship) -> Fact<bool> {
    let flags = ship.formation_flags?;
    if flags & (1_u64 << 22) != 0 {
        return Ok(true);
    }
    let positive = ship.positive_equipment_flags?;
    if positive & (1_u64 << 22) != 0 {
        return Ok(true);
    }
    Ok((flags | positive) & (1_u64 << 37) != 0)
}

/// F47350 and C3E1D0 sum signed wing+7C values, with i32 native wrapping.
fn planes<I: Inputs>(inputs: &mut I, base: u64) -> Fact<i32> {
    let containers = count(inputs.registered_container_count(base))?;
    let mut total = 0_i32;
    for container in 0..containers {
        let pools = count(inputs.pool_count(base, container))?;
        for pool in 0..pools {
            total = total.wrapping_add(inputs.pool_planes(base, container, pool)?);
        }
    }
    Ok(total)
}

/// D5AF10's wrapping sum of every original SHIPS C23D90 fill.
pub fn fill_sum<I: Inputs>(inputs: &mut I) -> Fact<i64> {
    let ships = count(inputs.original_ship_count())?;
    let mut sum = 0_i64;
    for ordinal in 0..ships {
        let ship = inputs.original_ship(ordinal)?;
        if !is_carrier(ship)? {
            continue;
        }
        let Some(base) = ship.carrier_air_base? else {
            continue;
        };
        let capacity = inputs.carrier_capacity(base)?;
        let planes = planes(inputs, base)?;
        sum = sum.wrapping_add(crate::transport_damage_graph::carrier_fill(
            capacity, planes,
        ));
    }
    Ok(sum)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Original {
        ships: Vec<Ship>,
        containers: Vec<Vec<Vec<i32>>>,
        capacity: i32,
        visited: Vec<(u32, u32, u32)>,
    }

    impl Inputs for Original {
        fn original_ship_count(&mut self) -> Fact<i32> {
            Ok(self.ships.len() as i32)
        }
        fn original_ship(&mut self, ordinal: u32) -> Fact<Ship> {
            self.ships
                .get(ordinal as usize)
                .copied()
                .ok_or("missing original ship")
        }
        fn carrier_capacity(&mut self, _base: u64) -> Fact<i32> {
            Ok(self.capacity)
        }
        fn registered_container_count(&mut self, _base: u64) -> Fact<i32> {
            Ok(self.containers.len() as i32)
        }
        fn pool_count(&mut self, _base: u64, container: u32) -> Fact<i32> {
            Ok(self.containers[container as usize].len() as i32)
        }
        fn wing_count(&mut self, _base: u64, container: u32, pool: u32) -> Fact<i32> {
            Ok(self.containers[container as usize][pool as usize].len() as i32)
        }
        fn wing_planes(&mut self, _base: u64, container: u32, pool: u32, wing: u32) -> Fact<i32> {
            self.visited.push((container, pool, wing));
            Ok(self.containers[container as usize][pool as usize][wing as usize])
        }
    }

    fn carrier() -> Ship {
        Ship {
            formation_flags: Ok(1_u64 << 22),
            positive_equipment_flags: Err("unreached equipment pool"),
            carrier_air_base: Ok(Some(1)),
        }
    }

    #[test]
    fn repeated_ships_containers_pools_and_wings_are_all_occurrences() {
        let mut original = Original {
            ships: vec![carrier(), carrier()],
            containers: vec![vec![vec![10, 10], vec![10]], vec![vec![10]]],
            capacity: 40,
            visited: Vec::new(),
        };
        assert_eq!(fill_sum(&mut original), Ok(200_000));
        assert_eq!(
            original.visited,
            vec![
                (0, 0, 0),
                (0, 0, 1),
                (0, 1, 0),
                (1, 0, 0),
                (0, 0, 0),
                (0, 0, 1),
                (0, 1, 0),
                (1, 0, 0),
            ]
        );
    }

    #[test]
    fn native_wing_sum_wraps_before_fill_division() {
        let mut original = Original {
            ships: vec![carrier()],
            containers: vec![vec![vec![i32::MAX, 1]]],
            capacity: 1,
            visited: Vec::new(),
        };
        assert_eq!(fill_sum(&mut original), Ok(0));
    }

    #[test]
    fn noncarrier_and_absent_base_do_not_demand_nested_air_inputs() {
        let mut original = Original {
            ships: vec![
                Ship {
                    formation_flags: Ok(0),
                    positive_equipment_flags: Ok(0),
                    carrier_air_base: Err("unreached base"),
                },
                Ship {
                    carrier_air_base: Ok(None),
                    ..carrier()
                },
            ],
            containers: Vec::new(),
            capacity: 1,
            visited: Vec::new(),
        };
        assert_eq!(fill_sum(&mut original), Ok(0));
        assert!(original.visited.is_empty());
    }

    #[test]
    fn first_pool_predicate_failure_is_not_bypassed_by_later_stored_flag() {
        let mut original = Original {
            ships: vec![Ship {
                formation_flags: Ok(1_u64 << 37),
                positive_equipment_flags: Err("original pool failed"),
                carrier_air_base: Ok(Some(1)),
            }],
            containers: Vec::new(),
            capacity: 1,
            visited: Vec::new(),
        };
        assert_eq!(fill_sum(&mut original), Err("original pool failed"));
    }
}
