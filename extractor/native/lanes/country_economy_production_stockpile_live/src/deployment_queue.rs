use std::collections::BTreeSet;

pub const DEPLOYMENT_QUEUE_SCHEMA_VERSION: u32 = 1;
pub const DEPLOYMENT_QUEUE_HEADER_LEN: usize = 40;
pub const DEPLOYMENT_CONVEYOR_ID_TYPE: u32 = 64;
pub const DEPLOYMENT_LINE_ID_TYPE: u32 = 63;
pub const DEPLOYMENT_TEMPLATE_ID_TYPE: u32 = 52;
pub const DEPLOYMENT_EQUIPMENT_ID_TYPE: u32 = 70;

const COUNTRY_FLAG_QUEUE_COMPLETE: u32 = 1;
const CONVEYOR_FLAG_CLOSED: u32 = 1;
const LINE_FLAG_TRAINING: u32 = 1;
const LINE_FLAG_MAX_TRAINING: u32 = 1 << 1;
const LINE_FLAG_MANPOWER_HAVE: u32 = 1 << 2;
const LINE_FLAG_MANPOWER_NEED: u32 = 1 << 3;
const LINE_FLAG_MAX_MANPOWER: u32 = 1 << 4;
const LINE_KNOWN_FLAGS: u32 = LINE_FLAG_TRAINING
    | LINE_FLAG_MAX_TRAINING
    | LINE_FLAG_MANPOWER_HAVE
    | LINE_FLAG_MANPOWER_NEED
    | LINE_FLAG_MAX_MANPOWER;
const MAX_STRING_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct DeploymentEquipmentRow {
    pub ordinal: u32,
    pub equipment_id: u32,
    pub archetype: String,
    pub amount: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeploymentLineRow {
    pub ordinal: u32,
    pub line_id: u32,
    pub name_order: i32,
    pub amount: i32,
    pub training: Option<f64>,
    pub max_training: Option<f64>,
    pub manpower_have: Option<i64>,
    pub manpower_need: Option<i64>,
    pub max_manpower: Option<i64>,
    pub equipment_in_transit: Vec<DeploymentEquipmentRow>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeploymentConveyorRow {
    pub ordinal: u32,
    pub conveyor_id: u32,
    pub name: String,
    pub template_id: u32,
    pub amount: i32,
    pub location: u32,
    pub priority: u32,
    pub role: String,
    pub closed: bool,
    pub lines: Vec<DeploymentLineRow>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeploymentCountryRow {
    pub tag_db_id: u32,
    /// `true` is a positive full-vector witness, including `conveyors=[]`.
    /// `false` means unreadable/unknown and therefore requires an empty carrier.
    pub queue_complete: bool,
    pub conveyors: Vec<DeploymentConveyorRow>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeploymentQueueSummary {
    pub country_rows: u32,
    pub complete_country_rows: u32,
    pub unknown_country_rows: u32,
    pub conveyor_rows: u32,
    pub line_rows: u32,
    pub equipment_rows: u32,
    pub changed_country_rows: u32,
    pub status_flags: u32,
}

/// Encode the exact save/live-neutral X4 deployment carrier.
///
/// The carrier deliberately does not claim a live producer. It preserves the
/// authenticated save identities and nested ordering so either a save oracle or
/// a future validated memory reader can feed the same pointer-free contract.
pub fn encode_deployment_queue_payload(
    elapsed_wall_ms: u64,
    changed_country_rows: u32,
    status_flags: u32,
    countries: &[DeploymentCountryRow],
) -> Result<Vec<u8>, String> {
    let country_count = u32::try_from(countries.len())
        .map_err(|_| "deployment country count exceeds u32".to_string())?;
    if changed_country_rows > country_count {
        return Err(format!(
            "deployment changed_country_rows {changed_country_rows} > country_count {country_count}"
        ));
    }
    let conveyor_count = countries.iter().try_fold(0u32, |total, country| {
        total
            .checked_add(
                u32::try_from(country.conveyors.len())
                    .map_err(|_| "deployment country conveyor count exceeds u32".to_string())?,
            )
            .ok_or_else(|| "deployment conveyor count overflow".to_string())
    })?;
    let line_count = countries.iter().try_fold(0u32, |total, country| {
        country.conveyors.iter().try_fold(total, |total, conveyor| {
            total
                .checked_add(
                    u32::try_from(conveyor.lines.len())
                        .map_err(|_| "deployment conveyor line count exceeds u32".to_string())?,
                )
                .ok_or_else(|| "deployment line count overflow".to_string())
        })
    })?;
    let equipment_count = countries.iter().try_fold(0u32, |total, country| {
        country.conveyors.iter().try_fold(total, |total, conveyor| {
            conveyor.lines.iter().try_fold(total, |total, line| {
                total
                    .checked_add(
                        u32::try_from(line.equipment_in_transit.len())
                            .map_err(|_| "deployment equipment count exceeds u32".to_string())?,
                    )
                    .ok_or_else(|| "deployment equipment count overflow".to_string())
            })
        })
    })?;

    let mut out = Vec::new();
    out.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    out.extend_from_slice(&country_count.to_le_bytes());
    out.extend_from_slice(&changed_country_rows.to_le_bytes());
    out.extend_from_slice(&status_flags.to_le_bytes());
    out.extend_from_slice(&DEPLOYMENT_QUEUE_SCHEMA_VERSION.to_le_bytes());
    out.extend_from_slice(&conveyor_count.to_le_bytes());
    out.extend_from_slice(&line_count.to_le_bytes());
    out.extend_from_slice(&equipment_count.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());

    for country in countries {
        if country.tag_db_id == 0 {
            return Err("deployment country has zero tag_db_id".into());
        }
        if !country.queue_complete && !country.conveyors.is_empty() {
            return Err(format!(
                "deployment country {} is unknown but carries conveyor rows",
                country.tag_db_id
            ));
        }
        out.extend_from_slice(&country.tag_db_id.to_le_bytes());
        out.extend_from_slice(
            &(if country.queue_complete {
                COUNTRY_FLAG_QUEUE_COMPLETE
            } else {
                0
            })
            .to_le_bytes(),
        );
        out.extend_from_slice(&(country.conveyors.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        for conveyor in &country.conveyors {
            out.extend_from_slice(&conveyor.ordinal.to_le_bytes());
            out.extend_from_slice(&conveyor.conveyor_id.to_le_bytes());
            out.extend_from_slice(&DEPLOYMENT_CONVEYOR_ID_TYPE.to_le_bytes());
            out.extend_from_slice(&conveyor.template_id.to_le_bytes());
            out.extend_from_slice(&DEPLOYMENT_TEMPLATE_ID_TYPE.to_le_bytes());
            out.extend_from_slice(&conveyor.amount.to_le_bytes());
            out.extend_from_slice(&conveyor.location.to_le_bytes());
            out.extend_from_slice(&conveyor.priority.to_le_bytes());
            out.extend_from_slice(
                &(if conveyor.closed {
                    CONVEYOR_FLAG_CLOSED
                } else {
                    0
                })
                .to_le_bytes(),
            );
            out.extend_from_slice(&(conveyor.lines.len() as u32).to_le_bytes());
            append_string(&mut out, &conveyor.name)?;
            append_string(&mut out, &conveyor.role)?;
            for line in &conveyor.lines {
                let mut flags = 0u32;
                flags |= u32::from(line.training.is_some()) * LINE_FLAG_TRAINING;
                flags |= u32::from(line.max_training.is_some()) * LINE_FLAG_MAX_TRAINING;
                flags |= u32::from(line.manpower_have.is_some()) * LINE_FLAG_MANPOWER_HAVE;
                flags |= u32::from(line.manpower_need.is_some()) * LINE_FLAG_MANPOWER_NEED;
                flags |= u32::from(line.max_manpower.is_some()) * LINE_FLAG_MAX_MANPOWER;
                out.extend_from_slice(&line.ordinal.to_le_bytes());
                out.extend_from_slice(&line.line_id.to_le_bytes());
                out.extend_from_slice(&DEPLOYMENT_LINE_ID_TYPE.to_le_bytes());
                out.extend_from_slice(&line.name_order.to_le_bytes());
                out.extend_from_slice(&line.amount.to_le_bytes());
                out.extend_from_slice(&flags.to_le_bytes());
                out.extend_from_slice(&line.training.unwrap_or(0.0).to_bits().to_le_bytes());
                out.extend_from_slice(&line.max_training.unwrap_or(0.0).to_bits().to_le_bytes());
                out.extend_from_slice(&line.manpower_have.unwrap_or(0).to_le_bytes());
                out.extend_from_slice(&line.manpower_need.unwrap_or(0).to_le_bytes());
                out.extend_from_slice(&line.max_manpower.unwrap_or(0).to_le_bytes());
                out.extend_from_slice(&(line.equipment_in_transit.len() as u32).to_le_bytes());
                out.extend_from_slice(&0u32.to_le_bytes());
                for equipment in &line.equipment_in_transit {
                    out.extend_from_slice(&equipment.ordinal.to_le_bytes());
                    out.extend_from_slice(&equipment.equipment_id.to_le_bytes());
                    out.extend_from_slice(&DEPLOYMENT_EQUIPMENT_ID_TYPE.to_le_bytes());
                    out.extend_from_slice(&0u32.to_le_bytes());
                    out.extend_from_slice(&equipment.amount.to_bits().to_le_bytes());
                    append_string(&mut out, &equipment.archetype)?;
                }
            }
        }
    }
    Ok(out)
}

pub fn decode_deployment_queue_payload(payload: &[u8]) -> Result<DeploymentQueueSummary, String> {
    need(payload, DEPLOYMENT_QUEUE_HEADER_LEN, "deployment header")?;
    let country_count = read_u32(payload, 8);
    let changed_country_rows = read_u32(payload, 12);
    let status_flags = read_u32(payload, 16);
    let schema_version = read_u32(payload, 20);
    let declared_conveyors = read_u32(payload, 24);
    let declared_lines = read_u32(payload, 28);
    let declared_equipment = read_u32(payload, 32);
    let reserved = read_u32(payload, 36);
    if schema_version != DEPLOYMENT_QUEUE_SCHEMA_VERSION || reserved != 0 {
        return Err(format!(
            "deployment invalid schema/reserved {schema_version}/{reserved}"
        ));
    }
    if changed_country_rows > country_count {
        return Err("deployment changed country count exceeds country count".into());
    }

    let mut cursor = DEPLOYMENT_QUEUE_HEADER_LEN;
    let mut tags = BTreeSet::new();
    let mut conveyor_ids = BTreeSet::new();
    let mut line_ids = BTreeSet::new();
    let mut complete_country_rows = 0u32;
    let mut unknown_country_rows = 0u32;
    let mut observed_conveyors = 0u32;
    let mut observed_lines = 0u32;
    let mut observed_equipment = 0u32;
    for country_index in 0..country_count {
        need(payload, cursor + 16, "deployment country row")?;
        let tag_db_id = read_u32(payload, cursor);
        let flags = read_u32(payload, cursor + 4);
        let conveyor_count = read_u32(payload, cursor + 8);
        let country_reserved = read_u32(payload, cursor + 12);
        cursor += 16;
        if tag_db_id == 0 || !tags.insert(tag_db_id) {
            return Err(format!(
                "deployment country {country_index} invalid/duplicate tag {tag_db_id}"
            ));
        }
        if flags & !COUNTRY_FLAG_QUEUE_COMPLETE != 0 || country_reserved != 0 {
            return Err(format!(
                "deployment country {tag_db_id} invalid flags/reserved"
            ));
        }
        if flags & COUNTRY_FLAG_QUEUE_COMPLETE == 0 {
            unknown_country_rows += 1;
            if conveyor_count != 0 {
                return Err(format!(
                    "deployment unknown country {tag_db_id} carries conveyors"
                ));
            }
        } else {
            complete_country_rows += 1;
        }
        for conveyor_ordinal in 0..conveyor_count {
            need(payload, cursor + 40, "deployment conveyor row")?;
            let ordinal = read_u32(payload, cursor);
            let conveyor_id = read_u32(payload, cursor + 4);
            let id_type = read_u32(payload, cursor + 8);
            let template_id = read_u32(payload, cursor + 12);
            let template_type = read_u32(payload, cursor + 16);
            let amount = read_i32(payload, cursor + 20);
            let _location = read_u32(payload, cursor + 24);
            let _priority = read_u32(payload, cursor + 28);
            let conveyor_flags = read_u32(payload, cursor + 32);
            let line_count = read_u32(payload, cursor + 36);
            cursor += 40;
            if ordinal != conveyor_ordinal
                || conveyor_id == 0
                || id_type != DEPLOYMENT_CONVEYOR_ID_TYPE
                || !conveyor_ids.insert(conveyor_id)
                || template_id == 0
                || template_type != DEPLOYMENT_TEMPLATE_ID_TYPE
                || amount < 0
                || conveyor_flags & !CONVEYOR_FLAG_CLOSED != 0
            {
                return Err(format!(
                    "deployment invalid conveyor identity/body {conveyor_id}"
                ));
            }
            let _name = read_string(payload, &mut cursor, "deployment conveyor name")?;
            let _role = read_string(payload, &mut cursor, "deployment conveyor role")?;
            observed_conveyors = observed_conveyors
                .checked_add(1)
                .ok_or_else(|| "deployment conveyor count overflow".to_string())?;
            for line_ordinal in 0..line_count {
                need(payload, cursor + 72, "deployment line row")?;
                let ordinal = read_u32(payload, cursor);
                let line_id = read_u32(payload, cursor + 4);
                let id_type = read_u32(payload, cursor + 8);
                let _name_order = read_i32(payload, cursor + 12);
                let amount = read_i32(payload, cursor + 16);
                let line_flags = read_u32(payload, cursor + 20);
                let training = f64::from_bits(read_u64(payload, cursor + 24));
                let max_training = f64::from_bits(read_u64(payload, cursor + 32));
                let manpower_have = read_i64(payload, cursor + 40);
                let manpower_need = read_i64(payload, cursor + 48);
                let max_manpower = read_i64(payload, cursor + 56);
                let equipment_count = read_u32(payload, cursor + 64);
                let line_reserved = read_u32(payload, cursor + 68);
                cursor += 72;
                if ordinal != line_ordinal
                    || line_id == 0
                    || id_type != DEPLOYMENT_LINE_ID_TYPE
                    || !line_ids.insert(line_id)
                    || amount < 0
                    || line_flags & !LINE_KNOWN_FLAGS != 0
                    || line_reserved != 0
                {
                    return Err(format!("deployment invalid line identity/body {line_id}"));
                }
                validate_optional_float(line_flags, LINE_FLAG_TRAINING, training, "training")?;
                validate_optional_float(
                    line_flags,
                    LINE_FLAG_MAX_TRAINING,
                    max_training,
                    "max_training",
                )?;
                validate_optional_nonnegative_i64(
                    line_flags,
                    LINE_FLAG_MANPOWER_HAVE,
                    manpower_have,
                    "manpower_have",
                )?;
                validate_optional_nonnegative_i64(
                    line_flags,
                    LINE_FLAG_MANPOWER_NEED,
                    manpower_need,
                    "manpower_need",
                )?;
                validate_optional_nonnegative_i64(
                    line_flags,
                    LINE_FLAG_MAX_MANPOWER,
                    max_manpower,
                    "max_manpower",
                )?;
                observed_lines = observed_lines
                    .checked_add(1)
                    .ok_or_else(|| "deployment line count overflow".to_string())?;
                for equipment_ordinal in 0..equipment_count {
                    need(payload, cursor + 24, "deployment equipment row")?;
                    let ordinal = read_u32(payload, cursor);
                    let equipment_id = read_u32(payload, cursor + 4);
                    let equipment_type = read_u32(payload, cursor + 8);
                    let equipment_reserved = read_u32(payload, cursor + 12);
                    let amount = f64::from_bits(read_u64(payload, cursor + 16));
                    cursor += 24;
                    let _archetype = read_string(payload, &mut cursor, "deployment archetype")?;
                    if ordinal != equipment_ordinal
                        || equipment_id == 0
                        || equipment_type != DEPLOYMENT_EQUIPMENT_ID_TYPE
                        || equipment_reserved != 0
                        || !amount.is_finite()
                        || amount < 0.0
                    {
                        return Err(format!(
                            "deployment invalid equipment row {equipment_id}:{equipment_type}"
                        ));
                    }
                    observed_equipment = observed_equipment
                        .checked_add(1)
                        .ok_or_else(|| "deployment equipment count overflow".to_string())?;
                }
            }
        }
    }
    if cursor != payload.len() {
        return Err(format!(
            "deployment trailing bytes: consumed {cursor} of {}",
            payload.len()
        ));
    }
    if (observed_conveyors, observed_lines, observed_equipment)
        != (declared_conveyors, declared_lines, declared_equipment)
    {
        return Err(format!(
            "deployment declared/observed count mismatch: declared {declared_conveyors}/{declared_lines}/{declared_equipment}, observed {observed_conveyors}/{observed_lines}/{observed_equipment}"
        ));
    }
    Ok(DeploymentQueueSummary {
        country_rows: country_count,
        complete_country_rows,
        unknown_country_rows,
        conveyor_rows: observed_conveyors,
        line_rows: observed_lines,
        equipment_rows: observed_equipment,
        changed_country_rows,
        status_flags,
    })
}

fn append_string(out: &mut Vec<u8>, value: &str) -> Result<(), String> {
    let bytes = value.as_bytes();
    if bytes.len() > MAX_STRING_BYTES {
        return Err(format!(
            "deployment string length {} exceeds {MAX_STRING_BYTES}",
            bytes.len()
        ));
    }
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

fn read_string(payload: &[u8], cursor: &mut usize, label: &str) -> Result<String, String> {
    need(payload, cursor.saturating_add(4), label)?;
    let len = read_u32(payload, *cursor) as usize;
    *cursor += 4;
    if len > MAX_STRING_BYTES {
        return Err(format!("{label} length {len} exceeds {MAX_STRING_BYTES}"));
    }
    need(payload, cursor.saturating_add(len), label)?;
    let value = std::str::from_utf8(&payload[*cursor..*cursor + len])
        .map_err(|_| format!("{label} is not UTF-8"))?
        .to_string();
    *cursor += len;
    Ok(value)
}

fn validate_optional_float(flags: u32, flag: u32, value: f64, label: &str) -> Result<(), String> {
    if flags & flag != 0 {
        if !value.is_finite() || value < 0.0 {
            return Err(format!("deployment {label} is invalid"));
        }
    } else if value.to_bits() != 0 {
        return Err(format!("deployment {label} payload present without flag"));
    }
    Ok(())
}

fn validate_optional_nonnegative_i64(
    flags: u32,
    flag: u32,
    value: i64,
    label: &str,
) -> Result<(), String> {
    if flags & flag != 0 {
        if value < 0 {
            return Err(format!("deployment {label} is negative"));
        }
    } else if value != 0 {
        return Err(format!("deployment {label} payload present without flag"));
    }
    Ok(())
}

fn need(payload: &[u8], required: usize, label: &str) -> Result<(), String> {
    if payload.len() < required {
        Err(format!(
            "{label} truncated: need {required}, have {}",
            payload.len()
        ))
    } else {
        Ok(())
    }
}

fn read_u32(payload: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap())
}

fn read_i32(payload: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap())
}

fn read_u64(payload: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(payload[offset..offset + 8].try_into().unwrap())
}

fn read_i64(payload: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes(payload[offset..offset + 8].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exact_country() -> DeploymentCountryRow {
        DeploymentCountryRow {
            tag_db_id: 7,
            queue_complete: true,
            conveyors: vec![DeploymentConveyorRow {
                ordinal: 0,
                conveyor_id: 254,
                name: "Infantry 1939 A".into(),
                template_id: 1546,
                amount: 1,
                location: 1537,
                priority: 1,
                role: "infantry".into(),
                closed: false,
                lines: vec![DeploymentLineRow {
                    ordinal: 0,
                    line_id: 6483,
                    name_order: 21,
                    amount: 1,
                    training: Some(0.09841),
                    max_training: Some(1.0),
                    manpower_have: Some(12_160),
                    manpower_need: Some(12_160),
                    max_manpower: Some(12_160),
                    equipment_in_transit: vec![DeploymentEquipmentRow {
                        ordinal: 0,
                        equipment_id: 4730,
                        archetype: "anti_tank_equipment_1".into(),
                        amount: 16.0,
                    }],
                }],
            }],
        }
    }

    #[test]
    fn exact_nested_carrier_round_trips_and_counts() {
        let payload = encode_deployment_queue_payload(9, 1, 0, &[exact_country()]).unwrap();
        let summary = decode_deployment_queue_payload(&payload).unwrap();
        assert_eq!(summary.country_rows, 1);
        assert_eq!(summary.complete_country_rows, 1);
        assert_eq!(summary.conveyor_rows, 1);
        assert_eq!(summary.line_rows, 1);
        assert_eq!(summary.equipment_rows, 1);
    }

    #[test]
    fn proven_empty_and_unknown_are_distinct() {
        let countries = [
            DeploymentCountryRow {
                tag_db_id: 1,
                queue_complete: true,
                conveyors: vec![],
            },
            DeploymentCountryRow {
                tag_db_id: 2,
                queue_complete: false,
                conveyors: vec![],
            },
        ];
        let summary = decode_deployment_queue_payload(
            &encode_deployment_queue_payload(9, 2, 0, &countries).unwrap(),
        )
        .unwrap();
        assert_eq!(summary.complete_country_rows, 1);
        assert_eq!(summary.unknown_country_rows, 1);
    }

    #[test]
    fn unknown_country_cannot_smuggle_rows() {
        let mut country = exact_country();
        country.queue_complete = false;
        assert!(encode_deployment_queue_payload(9, 1, 0, &[country])
            .unwrap_err()
            .contains("unknown but carries"));
    }

    #[test]
    fn decoder_rejects_wrong_identity_type_and_duplicate_ids() {
        let mut payload = encode_deployment_queue_payload(9, 1, 0, &[exact_country()]).unwrap();
        // Header 40 + country 16 + conveyor ordinal/id => type at +64.
        payload[64..68].copy_from_slice(&63u32.to_le_bytes());
        assert!(decode_deployment_queue_payload(&payload)
            .unwrap_err()
            .contains("invalid conveyor"));

        let mut country = exact_country();
        let mut second = country.conveyors[0].clone();
        second.ordinal = 1;
        country.conveyors.push(second);
        let payload = encode_deployment_queue_payload(9, 1, 0, &[country]).unwrap();
        assert!(decode_deployment_queue_payload(&payload)
            .unwrap_err()
            .contains("invalid conveyor"));
    }

    #[test]
    fn decoder_rejects_trailing_bytes_and_count_drift() {
        let mut trailing = encode_deployment_queue_payload(9, 1, 0, &[exact_country()]).unwrap();
        trailing.push(0);
        assert!(decode_deployment_queue_payload(&trailing)
            .unwrap_err()
            .contains("trailing bytes"));

        let mut drift = encode_deployment_queue_payload(9, 1, 0, &[exact_country()]).unwrap();
        drift[32..36].copy_from_slice(&2u32.to_le_bytes());
        assert!(decode_deployment_queue_payload(&drift)
            .unwrap_err()
            .contains("count mismatch"));
    }
}
