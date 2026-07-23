//! Typed, fail-closed deployment-queue presentation state.
//!
//! X4 records an ordered hierarchy, not an aggregate production number. This
//! module therefore preserves conveyor, line and in-transit-equipment order
//! verbatim. It does not localise serializer strings, invent names for missing
//! archetypes, or turn an absent/unobtained queue into an empty queue.

use replay_core::{
    DeploymentConveyorRow, DeploymentEquipmentRow, DeploymentLineRow, Leaf, ProductionView,
    ViewStatus, WorldFrame,
};
use std::collections::BTreeSet;

const MAX_CARRIER_STRING_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeploymentQueueLevel {
    Conveyor,
    Line,
    Equipment,
}

/// Why no deployment-queue presentation may be exposed for this country.
///
/// `NotRecorded` and `CapturedButUnobtained` intentionally remain distinct:
/// neither is equivalent to the measured empty queue represented by
/// `Available { conveyors: vec![] }`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeploymentQueueUnavailable {
    DomainAbsent,
    DomainUnavailable,
    CountryAbsent,
    NotRecorded,
    CapturedButUnobtained,
    InvalidOrder {
        level: DeploymentQueueLevel,
        expected: u32,
        actual: u32,
    },
    InvalidIdentity {
        level: DeploymentQueueLevel,
        id: u32,
    },
    InvalidValue {
        level: DeploymentQueueLevel,
        ordinal: u32,
        field: &'static str,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeploymentEquipmentPresentation {
    pub ordinal: u32,
    pub equipment_id: u32,
    pub archetype: Leaf<String>,
    pub amount: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeploymentLinePresentation {
    pub ordinal: u32,
    pub line_id: u32,
    pub name_order: Leaf<i32>,
    pub amount: i32,
    pub training: Leaf<f64>,
    pub max_training: Leaf<f64>,
    pub manpower_have: Leaf<i64>,
    pub manpower_need: Leaf<i64>,
    pub max_manpower: Leaf<i64>,
    pub equipment_in_transit: Vec<DeploymentEquipmentPresentation>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeploymentConveyorPresentation {
    pub ordinal: u32,
    pub conveyor_id: u32,
    pub name: String,
    pub template_id: u32,
    pub amount: i32,
    pub location: u32,
    pub priority: u32,
    pub role: String,
    pub closed: bool,
    pub lines: Vec<DeploymentLinePresentation>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeploymentQueuePresentation {
    pub country_tag: String,
    pub conveyors: Vec<DeploymentConveyorPresentation>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AdmittedDeploymentQueue {
    Unavailable(DeploymentQueueUnavailable),
    Available(DeploymentQueuePresentation),
}

/// Select and admit the deployment queue directly from a world frame.
pub fn deployment_queue_state(
    frame: &WorldFrame,
    country_tag: &str,
) -> AdmittedDeploymentQueue {
    admit_deployment_queue(
        frame.production_status(),
        country_tag,
        frame.production(country_tag),
    )
}

/// Admit one country's exact X4 deployment queue.
///
/// A measured empty vector is the only empty-state success. Any malformed
/// identity, ordering witness or numeric body rejects the whole hierarchy so a
/// renderer cannot display a plausible-looking partial queue.
pub fn admit_deployment_queue(
    status: ViewStatus,
    country_tag: &str,
    production: Option<&ProductionView>,
) -> AdmittedDeploymentQueue {
    match status {
        ViewStatus::DomainAbsent => {
            return unavailable(DeploymentQueueUnavailable::DomainAbsent)
        }
        ViewStatus::DomainUnavailable => {
            return unavailable(DeploymentQueueUnavailable::DomainUnavailable)
        }
        ViewStatus::Present => {}
    }
    let Some(production) = production else {
        return unavailable(DeploymentQueueUnavailable::CountryAbsent);
    };
    let rows = match &production.deployment_queue {
        Leaf::Absent => return unavailable(DeploymentQueueUnavailable::NotRecorded),
        Leaf::Unobtained => {
            return unavailable(DeploymentQueueUnavailable::CapturedButUnobtained)
        }
        Leaf::Measured(rows) => rows,
    };
    match admit_rows(rows) {
        Ok(conveyors) => AdmittedDeploymentQueue::Available(DeploymentQueuePresentation {
            country_tag: country_tag.to_owned(),
            conveyors,
        }),
        Err(reason) => unavailable(reason),
    }
}

fn unavailable(reason: DeploymentQueueUnavailable) -> AdmittedDeploymentQueue {
    AdmittedDeploymentQueue::Unavailable(reason)
}

fn admit_rows(
    rows: &[DeploymentConveyorRow],
) -> Result<Vec<DeploymentConveyorPresentation>, DeploymentQueueUnavailable> {
    let mut conveyor_ids = BTreeSet::new();
    let mut line_ids = BTreeSet::new();
    let mut conveyors = Vec::with_capacity(rows.len());
    for (expected_ordinal, conveyor) in rows.iter().enumerate() {
        check_order(
            DeploymentQueueLevel::Conveyor,
            expected_ordinal,
            conveyor.ordinal,
        )?;
        if conveyor.conveyor_id == 0 || !conveyor_ids.insert(conveyor.conveyor_id) {
            return Err(DeploymentQueueUnavailable::InvalidIdentity {
                level: DeploymentQueueLevel::Conveyor,
                id: conveyor.conveyor_id,
            });
        }
        if conveyor.template_id == 0 {
            return Err(invalid_value(
                DeploymentQueueLevel::Conveyor,
                conveyor.ordinal,
                "template_id",
            ));
        }
        if conveyor.amount < 0 {
            return Err(invalid_value(
                DeploymentQueueLevel::Conveyor,
                conveyor.ordinal,
                "amount",
            ));
        }
        if !valid_carrier_string(&conveyor.name) {
            return Err(invalid_value(
                DeploymentQueueLevel::Conveyor,
                conveyor.ordinal,
                "name",
            ));
        }
        if !valid_carrier_string(&conveyor.role) {
            return Err(invalid_value(
                DeploymentQueueLevel::Conveyor,
                conveyor.ordinal,
                "role",
            ));
        }

        let mut lines = Vec::with_capacity(conveyor.lines.len());
        for (expected_ordinal, line) in conveyor.lines.iter().enumerate() {
            lines.push(admit_line(expected_ordinal, line, &mut line_ids)?);
        }
        conveyors.push(DeploymentConveyorPresentation {
            ordinal: conveyor.ordinal,
            conveyor_id: conveyor.conveyor_id,
            name: conveyor.name.clone(),
            template_id: conveyor.template_id,
            amount: conveyor.amount,
            location: conveyor.location,
            priority: conveyor.priority,
            role: conveyor.role.clone(),
            closed: conveyor.closed,
            lines,
        });
    }
    Ok(conveyors)
}

fn admit_line(
    expected_ordinal: usize,
    line: &DeploymentLineRow,
    line_ids: &mut BTreeSet<u32>,
) -> Result<DeploymentLinePresentation, DeploymentQueueUnavailable> {
    check_order(
        DeploymentQueueLevel::Line,
        expected_ordinal,
        line.ordinal,
    )?;
    if line.line_id == 0 || !line_ids.insert(line.line_id) {
        return Err(DeploymentQueueUnavailable::InvalidIdentity {
            level: DeploymentQueueLevel::Line,
            id: line.line_id,
        });
    }
    if line.amount < 0 {
        return Err(invalid_value(
            DeploymentQueueLevel::Line,
            line.ordinal,
            "amount",
        ));
    }
    for (field, value) in [
        ("training", &line.training),
        ("max_training", &line.max_training),
    ] {
        if !valid_optional_nonnegative_f64(value) {
            return Err(invalid_value(
                DeploymentQueueLevel::Line,
                line.ordinal,
                field,
            ));
        }
    }
    for (field, value) in [
        ("manpower_have", &line.manpower_have),
        ("manpower_need", &line.manpower_need),
        ("max_manpower", &line.max_manpower),
    ] {
        if !valid_optional_nonnegative_i64(value) {
            return Err(invalid_value(
                DeploymentQueueLevel::Line,
                line.ordinal,
                field,
            ));
        }
    }

    let mut equipment_in_transit = Vec::with_capacity(line.equipment_in_transit.len());
    for (expected_ordinal, equipment) in line.equipment_in_transit.iter().enumerate() {
        equipment_in_transit.push(admit_equipment(expected_ordinal, equipment)?);
    }
    Ok(DeploymentLinePresentation {
        ordinal: line.ordinal,
        line_id: line.line_id,
        name_order: line.name_order.clone(),
        amount: line.amount,
        training: line.training.clone(),
        max_training: line.max_training.clone(),
        manpower_have: line.manpower_have.clone(),
        manpower_need: line.manpower_need.clone(),
        max_manpower: line.max_manpower.clone(),
        equipment_in_transit,
    })
}

fn admit_equipment(
    expected_ordinal: usize,
    equipment: &DeploymentEquipmentRow,
) -> Result<DeploymentEquipmentPresentation, DeploymentQueueUnavailable> {
    check_order(
        DeploymentQueueLevel::Equipment,
        expected_ordinal,
        equipment.ordinal,
    )?;
    if equipment.equipment_id == 0 {
        return Err(DeploymentQueueUnavailable::InvalidIdentity {
            level: DeploymentQueueLevel::Equipment,
            id: equipment.equipment_id,
        });
    }
    if !equipment.amount.is_finite() || equipment.amount < 0.0 {
        return Err(invalid_value(
            DeploymentQueueLevel::Equipment,
            equipment.ordinal,
            "amount",
        ));
    }
    if matches!(&equipment.archetype, Leaf::Measured(value) if !valid_carrier_string(value)) {
        return Err(invalid_value(
            DeploymentQueueLevel::Equipment,
            equipment.ordinal,
            "archetype",
        ));
    }
    Ok(DeploymentEquipmentPresentation {
        ordinal: equipment.ordinal,
        equipment_id: equipment.equipment_id,
        archetype: equipment.archetype.clone(),
        amount: equipment.amount,
    })
}

fn check_order(
    level: DeploymentQueueLevel,
    expected: usize,
    actual: u32,
) -> Result<(), DeploymentQueueUnavailable> {
    let Ok(expected) = u32::try_from(expected) else {
        return Err(DeploymentQueueUnavailable::InvalidOrder {
            level,
            expected: u32::MAX,
            actual,
        });
    };
    if expected != actual {
        return Err(DeploymentQueueUnavailable::InvalidOrder {
            level,
            expected,
            actual,
        });
    }
    Ok(())
}

fn invalid_value(
    level: DeploymentQueueLevel,
    ordinal: u32,
    field: &'static str,
) -> DeploymentQueueUnavailable {
    DeploymentQueueUnavailable::InvalidValue {
        level,
        ordinal,
        field,
    }
}

fn valid_optional_nonnegative_f64(value: &Leaf<f64>) -> bool {
    match value {
        Leaf::Measured(value) => value.is_finite() && *value >= 0.0,
        Leaf::Absent | Leaf::Unobtained => true,
    }
}

fn valid_optional_nonnegative_i64(value: &Leaf<i64>) -> bool {
    match value {
        Leaf::Measured(value) => *value >= 0,
        Leaf::Absent | Leaf::Unobtained => true,
    }
}

fn valid_carrier_string(value: &str) -> bool {
    value.len() <= MAX_CARRIER_STRING_BYTES
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exact_conveyor() -> DeploymentConveyorRow {
        DeploymentConveyorRow {
            ordinal: 0,
            conveyor_id: 44,
            name: "First Army".to_string(),
            template_id: 90,
            amount: 3,
            location: 321,
            priority: 2,
            role: "infantry".to_string(),
            closed: false,
            lines: vec![DeploymentLineRow {
                ordinal: 0,
                line_id: 45,
                name_order: Leaf::Absent,
                amount: 1,
                training: Leaf::Measured(0.25),
                max_training: Leaf::Unobtained,
                manpower_have: Leaf::Measured(8_000),
                manpower_need: Leaf::Absent,
                max_manpower: Leaf::Unobtained,
                equipment_in_transit: vec![DeploymentEquipmentRow {
                    ordinal: 0,
                    equipment_id: 46,
                    archetype: Leaf::Absent,
                    amount: 500.0,
                }],
            }],
        }
    }

    #[test]
    fn measured_hierarchy_preserves_order_and_nullable_leaf_states() {
        let admitted = admit_rows(&[exact_conveyor()]).expect("exact queue");
        assert_eq!(admitted.len(), 1);
        assert_eq!(admitted[0].ordinal, 0);
        assert_eq!(admitted[0].lines[0].ordinal, 0);
        assert_eq!(admitted[0].lines[0].equipment_in_transit[0].ordinal, 0);
        assert_eq!(admitted[0].lines[0].name_order, Leaf::Absent);
        assert_eq!(admitted[0].lines[0].max_training, Leaf::Unobtained);
        assert_eq!(
            admitted[0].lines[0].equipment_in_transit[0].archetype,
            Leaf::Absent
        );
    }

    #[test]
    fn measured_empty_queue_is_available_data() {
        assert_eq!(admit_rows(&[]), Ok(Vec::new()));
    }

    #[test]
    fn noncanonical_nested_order_rejects_whole_queue() {
        let mut conveyor = exact_conveyor();
        conveyor.lines[0].equipment_in_transit[0].ordinal = 1;
        assert_eq!(
            admit_rows(&[conveyor]),
            Err(DeploymentQueueUnavailable::InvalidOrder {
                level: DeploymentQueueLevel::Equipment,
                expected: 0,
                actual: 1,
            })
        );
    }

    #[test]
    fn invalid_optional_measurement_is_not_rendered_as_missing() {
        let mut conveyor = exact_conveyor();
        conveyor.lines[0].training = Leaf::Measured(f64::NAN);
        assert_eq!(
            admit_rows(&[conveyor]),
            Err(DeploymentQueueUnavailable::InvalidValue {
                level: DeploymentQueueLevel::Line,
                ordinal: 0,
                field: "training",
            })
        );
    }
}
