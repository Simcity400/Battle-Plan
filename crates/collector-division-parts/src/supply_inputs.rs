
use engine_native_parts::cunit_supply_inputs::{Demand, Failure, Inputs, Origin};
use replay_data::{
    rows::DivisionSupplyFooter, rows::Recorded, rows::UnitSupplyDemand, rows::UnitSupplyInputs,
    rows::UnitSupplyOrigin,
};

#[derive(Default)]
pub struct Sample {
    pub inputs: Option<Inputs>,
    pub failure: Option<Failure>,
    origins: Option<Vec<Origin>>,
    charge: collector_primitives::raw_memory::Charge,
}
impl Sample {
    pub fn owned(
        inputs: Option<Inputs>,
        failure: Option<Failure>,
        origins: Option<&[Origin]>,
    ) -> Result<std::sync::Arc<Self>, &'static str> {
        let origins = origins
            .map(|rows| {
                let mut owned = Vec::new();
                engine_allocation::reserve(&mut owned, rows.len())
                    .map_err(|_| "daily division supply allocation failed")?;
                owned.extend_from_slice(rows);
                Ok(owned)
            })
            .transpose()?;
        let mut sample = Self {
            inputs,
            failure,
            origins,
            charge: Default::default(),
        };
        sample
            .charge
            .set(collector_primitives::raw_memory::Capacity {
                auxiliary_vectors: sample.allocated_bytes() as u64,
                ..Default::default()
            });
        Ok(std::sync::Arc::new(sample))
    }
    pub fn origins(&self) -> Option<&[Origin]> {
        self.origins.as_deref()
    }
    pub fn allocated_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + 2 * std::mem::size_of::<usize>()
            + self
                .origins
                .as_ref()
                .map_or(0, |rows| rows.capacity() * std::mem::size_of::<Origin>())
    }
}

fn displayed(value: i64) -> i64 {
    UnitSupplyInputs::STEP.round(value)
}

fn demand(value: Demand) -> UnitSupplyDemand {
    UnitSupplyDemand {
        count: value.count,
        value: displayed(value.value),
    }
}

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
            engine_allocation::owned_vec(engine_allocation::collect_vec(origins.iter().map(
                |origin| UnitSupplyOrigin {
                    province: origin.province,
                    requested: displayed(origin.requested),
                    received: displayed(origin.received),
                },
            )))
        }),
    })
}

pub fn project_division(
    current: Option<Inputs>,
    origins: Option<&[Origin]>,
) -> Option<DivisionSupplyFooter> {
    project(current, origins).map(Into::into)
}

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
