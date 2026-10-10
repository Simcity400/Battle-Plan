//! Shared projection of exact unit supply operands. No memory reads or names.

use engine::cunit_supply_inputs::{Demand, Failure, Inputs, Origin};
use replay::rows::{Recorded, UnitSupplyDemand, UnitSupplyInputs, UnitSupplyOrigin};

#[derive(Default)]
pub(super) struct Work {
    captured: u64,
    refused: u64,
    origins_captured: u64,
    origins_refused: [u64; 5],
}

impl Work {
    pub(super) fn observe(&mut self, inputs: Option<Inputs>, failure: Option<Failure>) {
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

    pub(super) fn status(&self) -> impl serde::Serialize + '_ {
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

fn demand(value: Demand) -> UnitSupplyDemand {
    UnitSupplyDemand {
        count: value.count,
        value: value.value,
    }
}

pub(super) fn matches(
    current: Option<Inputs>,
    origins: Option<&[Origin]>,
    previous: &Recorded<Option<UnitSupplyInputs>>,
) -> bool {
    let Recorded::Value(previous) = previous else {
        return false;
    };
    match (current, previous) {
        (None, None) => true,
        (Some(current), Some(previous)) => {
            current.asked == previous.asked
                && current.received == previous.received
                && current.from_state == previous.from_state
                && current.air_drops == previous.air_drops
                && current.disruption == previous.disruption
                && demand(current.division_demand) == previous.division_demand
                && demand(current.airplane_demand) == previous.airplane_demand
                && demand(current.ship_demand) == previous.ship_demand
                && match (origins, &previous.origins) {
                    (None, None) => true,
                    (Some(current), Some(previous)) => {
                        current.len() == previous.len()
                            && current.iter().zip(previous).all(|(a, b)| {
                                a.province == b.province
                                    && a.requested == b.requested
                                    && a.received == b.received
                            })
                    }
                    _ => false,
                }
        }
        _ => false,
    }
}

pub(super) fn project(
    current: Option<Inputs>,
    origins: Option<&[Origin]>,
) -> Option<UnitSupplyInputs> {
    current.map(|current| UnitSupplyInputs {
        asked: current.asked,
        received: current.received,
        from_state: current.from_state,
        air_drops: current.air_drops,
        disruption: current.disruption,
        division_demand: demand(current.division_demand),
        airplane_demand: demand(current.airplane_demand),
        ship_demand: demand(current.ship_demand),
        origins: origins.map(|origins| {
            engine::allocation::owned_vec(engine::allocation::collect_vec(origins.iter().map(
                |origin| UnitSupplyOrigin {
                    province: origin.province,
                    requested: origin.requested,
                    received: origin.received,
                },
            )))
        }),
    })
}
