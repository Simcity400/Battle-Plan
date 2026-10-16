//! Shared projection of unit supply operands for divisions, railway guns and
//! task forces, at the footer's display step ([`UnitSupplyInputs::STEP`]). No
//! memory reads or names.

use engine::cunit_supply_inputs::{Demand, Failure, Inputs, Origin};
use replay::rows::{
    DivisionSupplyFooter, Recorded, UnitSupplyDemand, UnitSupplyInputs, UnitSupplyOrigin,
};

#[derive(Default)]
pub struct Work {
    captured: u64,
    refused: u64,
    origins_captured: u64,
    origins_refused: [u64; 5],
}

impl Work {
    pub fn observe(&mut self, inputs: Option<Inputs>, failure: Option<Failure>) {
        let Some(inputs) = inputs else {
            self.refused = self.refused.saturating_add(1);
            return;
        };
        self.captured = self.captured.saturating_add(1);
        if inputs.origins_len.is_ok() {
            self.origins_captured = self.origins_captured.saturating_add(1);
        } else if let Some(failure) = failure {
            let index = match failure {
                Failure::Unreadable => 0,
                Failure::InvalidOrigins => 1,
                Failure::OriginsCapacity { .. } => 2,
                Failure::OriginsChanged => 3,
                Failure::Allocation => 4,
            };
            self.origins_refused[index] = self.origins_refused[index].saturating_add(1);
        }
    }

    pub fn status(&self) -> impl serde::Serialize + '_ {
        crate::status::status!({
            "captured": self.captured,
            "scope": "division, railway-gun and task-force supply-footer operand packs; origin fragments fail independently",
            "refused": self.refused,
            "refusal": Failure::Unreadable.reason(),
            "origins_captured": self.origins_captured,
            "origin_refusals": crate::status::map([
                (Failure::Unreadable.reason(), self.origins_refused[0]),
                (Failure::InvalidOrigins.reason(), self.origins_refused[1]),
                (Failure::OriginsCapacity { required: 0 }.reason(), self.origins_refused[2]),
                (Failure::OriginsChanged.reason(), self.origins_refused[3]),
                (Failure::Allocation.reason(), self.origins_refused[4]),
            ].into_iter()),
        })
    }
}

/// An amount at the footer's display step ([`UnitSupplyInputs::STEP`]).
fn displayed(value: i64) -> i64 {
    UnitSupplyInputs::STEP.round(value)
}

fn demand(value: Demand) -> UnitSupplyDemand {
    UnitSupplyDemand {
        count: value.count,
        value: displayed(value.value),
    }
}

/// A unit's footer as the counter rows record it: every amount rounded to
/// its display step, so a footer that drifted only below a step equals the
/// baseline.
pub fn project(current: Option<Inputs>, origins: Option<&[Origin]>) -> Option<UnitSupplyInputs> {
    current.map(|current| UnitSupplyInputs {
        asked: displayed(current.asked),
        received: displayed(current.received),
        from_state: displayed(current.from_state),
        air_drops: displayed(current.air_drops),
        disruption: displayed(current.disruption),
        division_demand: demand(current.division_demand),
        airplane_demand: demand(current.airplane_demand),
        ship_demand: demand(current.ship_demand),
        origins: origins.map(|origins| {
            engine::allocation::owned_vec(engine::allocation::collect_vec(origins.iter().map(
                |origin| UnitSupplyOrigin {
                    province: origin.province,
                    requested: displayed(origin.requested),
                    received: displayed(origin.received),
                },
            )))
        }),
    })
}

/// [`project`] as DivisionStatus records it.
pub fn project_division(
    current: Option<Inputs>,
    origins: Option<&[Origin]>,
) -> Option<DivisionSupplyFooter> {
    project(current, origins).map(Into::into)
}

/// Whether [`project_division`] would equal the held row's footer, without
/// building it.
pub fn matches_division(
    current: Option<Inputs>,
    origins: Option<&[Origin]>,
    previous: &Recorded<Option<DivisionSupplyFooter>>,
) -> bool {
    let Recorded::Value(previous) = previous else {
        return false;
    };
    match (current, previous) {
        (None, None) => true,
        (Some(current), Some(previous)) => {
            displayed(current.asked) == previous.asked
                && displayed(current.received) == previous.received
                && displayed(current.from_state) == previous.from_state
                && displayed(current.air_drops) == previous.air_drops
                && displayed(current.disruption) == previous.disruption
                && demand(current.division_demand) == previous.division_demand
                && demand(current.airplane_demand) == previous.airplane_demand
                && demand(current.ship_demand) == previous.ship_demand
                && match (origins, &previous.origins) {
                    (None, None) => true,
                    (Some(current), Some(previous)) => {
                        current.len() == previous.len()
                            && current.iter().zip(previous).all(|(a, b)| {
                                a.province == b.province
                                    && displayed(a.requested) == b.requested
                                    && displayed(a.received) == b.received
                            })
                    }
                    _ => false,
                }
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(origins_len: Result<usize, Failure>) -> Inputs {
        Inputs {
            asked: 123_456,
            received: 1_499,
            from_state: 1_500,
            air_drops: 3,
            disruption: -4,
            division_demand: Demand {
                count: 7,
                value: 2_500,
            },
            airplane_demand: Demand::default(),
            ship_demand: Demand {
                count: i32::MIN,
                value: i64::MAX,
            },
            origins_len,
        }
    }

    #[test]
    fn division_footers_project_and_compare_at_display_precision() {
        let origins = [Origin {
            province: -42,
            requested: 10_499,
            received: 4_999,
        }];
        let footer = project_division(Some(inputs(Ok(1))), Some(&origins)).unwrap();
        assert_eq!(
            footer,
            DivisionSupplyFooter {
                asked: 123_000,
                received: 1_000,
                from_state: 2_000,
                air_drops: 1_000,
                disruption: -1_000,
                division_demand: UnitSupplyDemand {
                    count: 7,
                    value: 3_000,
                },
                airplane_demand: UnitSupplyDemand::default(),
                ship_demand: UnitSupplyDemand {
                    count: i32::MIN,
                    value: i64::MAX,
                },
                origins: Some(vec![UnitSupplyOrigin {
                    province: -42,
                    requested: 10_000,
                    received: 5_000,
                }]),
            }
        );
        let held = Recorded::Value(Some(footer));
        assert!(matches_division(Some(inputs(Ok(1))), Some(&origins), &held));
        // Drift inside every printed hundredth equals the held footer.
        let drifted = Inputs {
            asked: 123_499,
            received: 501,
            ..inputs(Ok(1))
        };
        let drifted_origins = [Origin {
            requested: 9_500,
            received: 5_499,
            ..origins[0]
        }];
        assert!(matches_division(
            Some(drifted),
            Some(&drifted_origins),
            &held
        ));
        // A printed change, a lost origin fragment or an older recording's
        // absence does not.
        let moved = Inputs {
            received: 2_500,
            ..inputs(Ok(1))
        };
        assert!(!matches_division(Some(moved), Some(&origins), &held));
        assert!(!matches_division(Some(inputs(Ok(1))), None, &held));
        assert!(!matches_division(
            Some(inputs(Ok(1))),
            Some(&origins),
            &Recorded::NotRecorded
        ));
        assert!(matches_division(None, None, &Recorded::Value(None)));
        assert_eq!(project_division(None, None), None);
    }

    #[test]
    fn railway_gun_and_task_force_footers_project_at_display_precision() {
        let origins = [Origin {
            province: -42,
            requested: 10_499,
            received: 4_999,
        }];
        let unit = project(Some(inputs(Ok(1))), Some(&origins)).unwrap();
        assert_eq!(
            DivisionSupplyFooter::from(unit),
            project_division(Some(inputs(Ok(1))), Some(&origins)).unwrap()
        );
        let unit = project(Some(inputs(Ok(1))), None).unwrap();
        assert_eq!(
            (unit.asked, unit.received, unit.air_drops, unit.disruption),
            (123_000, 1_000, 1_000, -1_000)
        );
        assert_eq!(unit.division_demand.value, 3_000);
        assert_eq!(unit.ship_demand.value, i64::MAX);
        assert_eq!(unit.origins, None);
    }
}
