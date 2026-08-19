use std::env;
use std::fs;
use std::process;

pub const PROVISIONAL_DOMAIN_NAVAL_OOB_EXT: u16 = 29_001;
pub const NAVAL_OOB_EXT_SCHEMA_VERSION: u32 = 1;
pub const NAVAL_OOB_EXT_HEADER_LEN: usize = 32;
pub const NAVAL_OOB_EXT_FLEET_ROW_LEN: usize = 40;
pub const NAVAL_OOB_EXT_TASK_FORCE_ROW_LEN: usize = 40;
pub const NAVAL_OOB_EXT_REGION_BUCKET_ROW_LEN: usize = 24;

pub const FLAG_HAS_LEADER: u32 = 1 << 0;
pub const FLAG_HAS_LEADER_SKILL: u32 = 1 << 1;
pub const FLAG_HAS_HOME_BASE: u32 = 1 << 2;
pub const FLAG_HAS_MISSION: u32 = 1 << 3;
pub const FLAG_HAS_THEATRE: u32 = 1 << 4;
pub const FLAG_HAS_REGION: u32 = 1 << 5;
pub const FLAG_SHIP_NAME_PRESENT: u32 = 1 << 6;
pub const FLAG_FIELD_REJECTED: u32 = 1 << 7;
pub const FLAG_FIXTURE_REQUIRED: u32 = 1 << 8;
pub const FLAG_DERIVED_REGION: u32 = 1 << 9;
pub const FLAG_VALIDATED_DIRECT: u32 = 1 << 10;

pub const KNOWN_FLAGS: u32 = FLAG_HAS_LEADER
    | FLAG_HAS_LEADER_SKILL
    | FLAG_HAS_HOME_BASE
    | FLAG_HAS_MISSION
    | FLAG_HAS_THEATRE
    | FLAG_HAS_REGION
    | FLAG_SHIP_NAME_PRESENT
    | FLAG_FIELD_REJECTED
    | FLAG_FIXTURE_REQUIRED
    | FLAG_DERIVED_REGION
    | FLAG_VALIDATED_DIRECT;

pub const MAX_OOB_ID: u32 = 4_000_000;
pub const MAX_OWNER_TAG_DB_ID: u32 =
    compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID;
/// The province id space's STRUCTURAL width (lane IA). Was a guess --
/// `65_535` or `1_000_000`, depending on which copy you read; the tree carried
/// four different ceilings for one id space. Proven from the engine's signed-int
/// subscript law at `ai_area.cpp:0xcc` (`claim_347234c20a542ca79032`).
pub const MAX_PROVINCE_ID: u32 = (i32::MAX as u32) - 1;
pub const MAX_REGION_ID: u32 = 4_096;
pub const MAX_LEADER_SKILL: u32 = 15;
pub const MAX_MISSION_ENUM: u32 = 64;
pub const MAX_SHIP_NAME_LEN: usize = 128;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FleetExtensionRow {
    pub fleet_id: u32,
    pub flags: u32,
    pub leader_id: u32,
    pub leader_skill: u32,
    pub leader_trait_count: u32,
    pub home_base_province_id: u32,
    pub mission_enum: u32,
    pub theatre_id: u32,
    pub region_bucket_first: u32,
    pub region_bucket_count: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskForceExtensionRow {
    pub task_force_id: u32,
    pub fleet_id: u32,
    pub flags: u32,
    pub mission_enum: u32,
    pub home_base_province_id: u32,
    pub location_province_id: u32,
    pub strategic_region_id: u32,
    pub naval_base_state_id: u32,
    pub leader_id: u32,
    pub reserved0: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShipNameRow {
    pub instance_id: u32,
    pub flags: u32,
    pub name: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RegionBucketRow {
    pub strategic_region_id: u32,
    pub owner_tag_db_id: u32,
    pub fleet_id: u32,
    pub task_force_count: u32,
    pub ship_count: u32,
    pub flags: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NavalOobExtensionPayload {
    pub status_flags: u32,
    pub fleets: Vec<FleetExtensionRow>,
    pub task_forces: Vec<TaskForceExtensionRow>,
    pub ship_names: Vec<ShipNameRow>,
    pub region_buckets: Vec<RegionBucketRow>,
}

pub fn encode_payload(payload: &NavalOobExtensionPayload) -> Vec<u8> {
    let mut out = Vec::new();
    push_u32(&mut out, NAVAL_OOB_EXT_SCHEMA_VERSION);
    push_u32(&mut out, payload.status_flags);
    push_u32(&mut out, payload.fleets.len() as u32);
    push_u32(&mut out, payload.task_forces.len() as u32);
    push_u32(&mut out, payload.ship_names.len() as u32);
    push_u32(&mut out, payload.region_buckets.len() as u32);
    push_u32(&mut out, 0);
    push_u32(&mut out, 0);

    for row in &payload.fleets {
        push_u32(&mut out, row.fleet_id);
        push_u32(&mut out, row.flags);
        push_u32(&mut out, row.leader_id);
        push_u32(&mut out, row.leader_skill);
        push_u32(&mut out, row.leader_trait_count);
        push_u32(&mut out, row.home_base_province_id);
        push_u32(&mut out, row.mission_enum);
        push_u32(&mut out, row.theatre_id);
        push_u32(&mut out, row.region_bucket_first);
        push_u32(&mut out, row.region_bucket_count);
    }

    for row in &payload.task_forces {
        push_u32(&mut out, row.task_force_id);
        push_u32(&mut out, row.fleet_id);
        push_u32(&mut out, row.flags);
        push_u32(&mut out, row.mission_enum);
        push_u32(&mut out, row.home_base_province_id);
        push_u32(&mut out, row.location_province_id);
        push_u32(&mut out, row.strategic_region_id);
        push_u32(&mut out, row.naval_base_state_id);
        push_u32(&mut out, row.leader_id);
        push_u32(&mut out, row.reserved0);
    }

    for row in &payload.ship_names {
        push_u32(&mut out, row.instance_id);
        push_u32(&mut out, row.flags);
        match &row.name {
            Some(name) => {
                push_u16(&mut out, name.len() as u16);
                out.extend_from_slice(name.as_bytes());
            }
            None => push_u16(&mut out, 0),
        }
    }

    for row in &payload.region_buckets {
        push_u32(&mut out, row.strategic_region_id);
        push_u32(&mut out, row.owner_tag_db_id);
        push_u32(&mut out, row.fleet_id);
        push_u32(&mut out, row.task_force_count);
        push_u32(&mut out, row.ship_count);
        push_u32(&mut out, row.flags);
    }
    out
}

pub fn decode_payload(raw: &[u8]) -> Result<NavalOobExtensionPayload, String> {
    if raw.len() < NAVAL_OOB_EXT_HEADER_LEN {
        return Err(format!(
            "payload is {} byte(s), expected at least {NAVAL_OOB_EXT_HEADER_LEN}",
            raw.len()
        ));
    }
    let schema_version = read_u32(raw, 0)?;
    if schema_version != NAVAL_OOB_EXT_SCHEMA_VERSION {
        return Err(format!("unsupported schema_version {schema_version}"));
    }
    let status_flags = read_u32(raw, 4)?;
    validate_flags(status_flags, "payload status_flags")?;
    let fleet_count = read_u32(raw, 8)? as usize;
    let task_force_count = read_u32(raw, 12)? as usize;
    let ship_name_count = read_u32(raw, 16)? as usize;
    let region_bucket_count = read_u32(raw, 20)? as usize;
    if read_u32(raw, 24)? != 0 || read_u32(raw, 28)? != 0 {
        return Err("header reserved fields must be zero".to_string());
    }

    let fixed_len = NAVAL_OOB_EXT_HEADER_LEN
        .saturating_add(fleet_count.saturating_mul(NAVAL_OOB_EXT_FLEET_ROW_LEN))
        .saturating_add(task_force_count.saturating_mul(NAVAL_OOB_EXT_TASK_FORCE_ROW_LEN));
    if raw.len() < fixed_len {
        return Err(format!("payload is {} byte(s), fixed rows need {fixed_len}", raw.len()));
    }

    let mut cursor = NAVAL_OOB_EXT_HEADER_LEN;
    let mut fleets = Vec::with_capacity(fleet_count);
    for index in 0..fleet_count {
        let row = FleetExtensionRow {
            fleet_id: read_u32(raw, cursor)?,
            flags: read_u32(raw, cursor + 4)?,
            leader_id: read_u32(raw, cursor + 8)?,
            leader_skill: read_u32(raw, cursor + 12)?,
            leader_trait_count: read_u32(raw, cursor + 16)?,
            home_base_province_id: read_u32(raw, cursor + 20)?,
            mission_enum: read_u32(raw, cursor + 24)?,
            theatre_id: read_u32(raw, cursor + 28)?,
            region_bucket_first: read_u32(raw, cursor + 32)?,
            region_bucket_count: read_u32(raw, cursor + 36)?,
        };
        validate_fleet_row(index, &row, region_bucket_count)?;
        fleets.push(row);
        cursor += NAVAL_OOB_EXT_FLEET_ROW_LEN;
    }

    let mut task_forces = Vec::with_capacity(task_force_count);
    for index in 0..task_force_count {
        let row = TaskForceExtensionRow {
            task_force_id: read_u32(raw, cursor)?,
            fleet_id: read_u32(raw, cursor + 4)?,
            flags: read_u32(raw, cursor + 8)?,
            mission_enum: read_u32(raw, cursor + 12)?,
            home_base_province_id: read_u32(raw, cursor + 16)?,
            location_province_id: read_u32(raw, cursor + 20)?,
            strategic_region_id: read_u32(raw, cursor + 24)?,
            naval_base_state_id: read_u32(raw, cursor + 28)?,
            leader_id: read_u32(raw, cursor + 32)?,
            reserved0: read_u32(raw, cursor + 36)?,
        };
        validate_task_force_row(index, &row)?;
        task_forces.push(row);
        cursor += NAVAL_OOB_EXT_TASK_FORCE_ROW_LEN;
    }

    let mut ship_names = Vec::with_capacity(ship_name_count);
    for index in 0..ship_name_count {
        if cursor + 10 > raw.len() {
            return Err(format!("ship name row {index} header is truncated"));
        }
        let instance_id = read_u32(raw, cursor)?;
        let flags = read_u32(raw, cursor + 4)?;
        validate_flags(flags, "ship name flags")?;
        let name_len = read_u16(raw, cursor + 8)? as usize;
        cursor += 10;
        if name_len > MAX_SHIP_NAME_LEN {
            return Err(format!("ship name row {index} name_len {name_len} exceeds {MAX_SHIP_NAME_LEN}"));
        }
        if cursor + name_len > raw.len() {
            return Err(format!("ship name row {index} text is truncated"));
        }
        let name = if name_len == 0 {
            None
        } else {
            Some(String::from_utf8(raw[cursor..cursor + name_len].to_vec()).map_err(|_| {
                format!("ship name row {index} is not valid utf-8")
            })?)
        };
        cursor += name_len;
        let row = ShipNameRow {
            instance_id,
            flags,
            name,
        };
        validate_ship_name_row(index, &row)?;
        ship_names.push(row);
    }

    let region_len = region_bucket_count.saturating_mul(NAVAL_OOB_EXT_REGION_BUCKET_ROW_LEN);
    if cursor + region_len != raw.len() {
        return Err(format!(
            "payload length {} does not match decoded length {}",
            raw.len(),
            cursor + region_len
        ));
    }
    let mut region_buckets = Vec::with_capacity(region_bucket_count);
    for index in 0..region_bucket_count {
        let row = RegionBucketRow {
            strategic_region_id: read_u32(raw, cursor)?,
            owner_tag_db_id: read_u32(raw, cursor + 4)?,
            fleet_id: read_u32(raw, cursor + 8)?,
            task_force_count: read_u32(raw, cursor + 12)?,
            ship_count: read_u32(raw, cursor + 16)?,
            flags: read_u32(raw, cursor + 20)?,
        };
        validate_region_bucket_row(index, &row)?;
        region_buckets.push(row);
        cursor += NAVAL_OOB_EXT_REGION_BUCKET_ROW_LEN;
    }

    Ok(NavalOobExtensionPayload {
        status_flags,
        fleets,
        task_forces,
        ship_names,
        region_buckets,
    })
}

fn validate_fleet_row(
    index: usize,
    row: &FleetExtensionRow,
    region_bucket_count: usize,
) -> Result<(), String> {
    validate_oob_id(row.fleet_id, "fleet", index)?;
    validate_flags(row.flags, "fleet flags")?;
    validate_optional_id(row.flags & FLAG_HAS_LEADER != 0, row.leader_id, "fleet leader", index)?;
    if row.flags & FLAG_HAS_LEADER_SKILL != 0 && row.leader_skill > MAX_LEADER_SKILL {
        return Err(format!("fleet row {index} leader_skill {} exceeds {MAX_LEADER_SKILL}", row.leader_skill));
    }
    validate_optional_province(
        row.flags & FLAG_HAS_HOME_BASE != 0,
        row.home_base_province_id,
        "fleet home_base",
        index,
    )?;
    if row.flags & FLAG_HAS_MISSION != 0 && row.mission_enum > MAX_MISSION_ENUM {
        return Err(format!("fleet row {index} mission_enum {} exceeds {MAX_MISSION_ENUM}", row.mission_enum));
    }
    validate_optional_id(row.flags & FLAG_HAS_THEATRE != 0, row.theatre_id, "fleet theatre", index)?;
    if row.region_bucket_count > 0 {
        let start = row.region_bucket_first as usize;
        let count = row.region_bucket_count as usize;
        if start.checked_add(count).filter(|end| *end <= region_bucket_count).is_none() {
            return Err(format!("fleet row {index} region bucket span is out of range"));
        }
    }
    Ok(())
}

fn validate_task_force_row(index: usize, row: &TaskForceExtensionRow) -> Result<(), String> {
    validate_oob_id(row.task_force_id, "task_force", index)?;
    validate_oob_id(row.fleet_id, "task_force fleet", index)?;
    validate_flags(row.flags, "task_force flags")?;
    if row.flags & FLAG_HAS_MISSION != 0 && row.mission_enum > MAX_MISSION_ENUM {
        return Err(format!("task-force row {index} mission_enum {} exceeds {MAX_MISSION_ENUM}", row.mission_enum));
    }
    validate_optional_province(
        row.flags & FLAG_HAS_HOME_BASE != 0,
        row.home_base_province_id,
        "task-force home_base",
        index,
    )?;
    if row.location_province_id == 0 || row.location_province_id > MAX_PROVINCE_ID {
        return Err(format!("task-force row {index} has invalid location_province_id {}", row.location_province_id));
    }
    if row.flags & FLAG_HAS_REGION != 0
        && (row.strategic_region_id == 0 || row.strategic_region_id > MAX_REGION_ID)
    {
        return Err(format!("task-force row {index} has invalid strategic_region_id {}", row.strategic_region_id));
    }
    validate_optional_id(row.flags & FLAG_HAS_LEADER != 0, row.leader_id, "task-force leader", index)?;
    if row.reserved0 != 0 {
        return Err(format!("task-force row {index} reserved0 must be zero"));
    }
    Ok(())
}

fn validate_ship_name_row(index: usize, row: &ShipNameRow) -> Result<(), String> {
    validate_oob_id(row.instance_id, "ship instance", index)?;
    let has_name = row.flags & FLAG_SHIP_NAME_PRESENT != 0;
    match (&row.name, has_name) {
        (Some(name), true) if !name.is_empty() && name.len() <= MAX_SHIP_NAME_LEN => Ok(()),
        (None, false) => Ok(()),
        (Some(_), false) => Err(format!("ship name row {index} has text without present flag")),
        (None, true) => Err(format!("ship name row {index} has present flag without text")),
        _ => Err(format!("ship name row {index} has invalid name")),
    }
}

fn validate_region_bucket_row(index: usize, row: &RegionBucketRow) -> Result<(), String> {
    if row.strategic_region_id == 0 || row.strategic_region_id > MAX_REGION_ID {
        return Err(format!(
            "region bucket row {index} has invalid strategic_region_id {}",
            row.strategic_region_id
        ));
    }
    if row.owner_tag_db_id > MAX_OWNER_TAG_DB_ID {
        return Err(format!("region bucket row {index} owner_tag_db_id {} exceeds {MAX_OWNER_TAG_DB_ID}", row.owner_tag_db_id));
    }
    validate_oob_id(row.fleet_id, "region bucket fleet", index)?;
    validate_flags(row.flags, "region bucket flags")?;
    Ok(())
}

fn validate_flags(flags: u32, label: &str) -> Result<(), String> {
    if flags & !KNOWN_FLAGS != 0 {
        return Err(format!("{label} has unknown bits 0x{:X}", flags & !KNOWN_FLAGS));
    }
    Ok(())
}

fn validate_oob_id(value: u32, label: &str, index: usize) -> Result<(), String> {
    if value == 0 || value > MAX_OOB_ID {
        return Err(format!("{label} row {index} has invalid id {value}"));
    }
    Ok(())
}

fn validate_optional_id(
    present: bool,
    value: u32,
    label: &str,
    index: usize,
) -> Result<(), String> {
    if present {
        validate_oob_id(value, label, index)
    } else if value != 0 {
        Err(format!("{label} row {index} has value without flag"))
    } else {
        Ok(())
    }
}

fn validate_optional_province(
    present: bool,
    value: u32,
    label: &str,
    index: usize,
) -> Result<(), String> {
    if present {
        if value == 0 || value > MAX_PROVINCE_ID {
            Err(format!("{label} row {index} has invalid province {value}"))
        } else {
            Ok(())
        }
    } else if value != 0 {
        Err(format!("{label} row {index} has province without flag"))
    } else {
        Ok(())
    }
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn read_u16(raw: &[u8], offset: usize) -> Result<u16, String> {
    raw.get(offset..offset + 2)
        .and_then(|data| data.try_into().ok())
        .map(u16::from_le_bytes)
        .ok_or_else(|| format!("read_u16 at {offset} is out of bounds"))
}

fn read_u32(raw: &[u8], offset: usize) -> Result<u32, String> {
    raw.get(offset..offset + 4)
        .and_then(|data| data.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| format!("read_u32 at {offset} is out of bounds"))
}

fn summary_json(payload: &NavalOobExtensionPayload) -> String {
    format!(
        "{{\"provisional_domain_id\":{},\"schema_version\":{},\"status_flags\":{},\"fleet_rows\":{},\"task_force_rows\":{},\"ship_name_rows\":{},\"region_bucket_rows\":{}}}\n",
        PROVISIONAL_DOMAIN_NAVAL_OOB_EXT,
        NAVAL_OOB_EXT_SCHEMA_VERSION,
        payload.status_flags,
        payload.fleets.len(),
        payload.task_forces.len(),
        payload.ship_names.len(),
        payload.region_buckets.len()
    )
}

fn main() {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: gap_extension_compact <payload.bin>");
        process::exit(2);
    };
    let raw = match fs::read(&path) {
        Ok(raw) => raw,
        Err(err) => {
            eprintln!("read {path}: {err}");
            process::exit(2);
        }
    };
    match decode_payload(&raw) {
        Ok(payload) => print!("{}", summary_json(&payload)),
        Err(err) => {
            eprintln!("decode failed: {err}");
            process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_and_decodes_badged_extension_rows() {
        let payload = NavalOobExtensionPayload {
            status_flags: FLAG_FIXTURE_REQUIRED | FLAG_DERIVED_REGION,
            fleets: vec![FleetExtensionRow {
                fleet_id: 615,
                flags: FLAG_HAS_LEADER | FLAG_HAS_LEADER_SKILL | FLAG_VALIDATED_DIRECT,
                leader_id: 77,
                leader_skill: 4,
                leader_trait_count: 2,
                ..FleetExtensionRow::default()
            }],
            task_forces: vec![TaskForceExtensionRow {
                task_force_id: 617,
                fleet_id: 615,
                flags: FLAG_HAS_REGION | FLAG_DERIVED_REGION | FLAG_FIXTURE_REQUIRED,
                location_province_id: 12389,
                strategic_region_id: 31,
                ..TaskForceExtensionRow::default()
            }],
            ship_names: vec![ShipNameRow {
                instance_id: 1001,
                flags: FLAG_FIELD_REJECTED,
                name: None,
            }],
            region_buckets: vec![RegionBucketRow {
                strategic_region_id: 31,
                owner_tag_db_id: 47,
                fleet_id: 615,
                task_force_count: 1,
                ship_count: 2,
                flags: FLAG_DERIVED_REGION,
            }],
        };

        let raw = encode_payload(&payload);
        let decoded = decode_payload(&raw).expect("payload should decode");

        assert_eq!(decoded, payload);
        assert_eq!(raw.len(), 32 + 40 + 40 + 10 + 24);
    }

    #[test]
    fn decodes_present_ship_name_when_fixture_later_validates_it() {
        let payload = NavalOobExtensionPayload {
            ship_names: vec![ShipNameRow {
                instance_id: 1001,
                flags: FLAG_SHIP_NAME_PRESENT | FLAG_VALIDATED_DIRECT,
                name: Some("Test Ship".to_string()),
            }],
            ..NavalOobExtensionPayload::default()
        };

        let decoded = decode_payload(&encode_payload(&payload)).expect("payload should decode");

        assert_eq!(decoded.ship_names[0].name.as_deref(), Some("Test Ship"));
    }

    #[test]
    fn rejects_invalid_region_bucket_id() {
        let payload = NavalOobExtensionPayload {
            region_buckets: vec![RegionBucketRow {
                strategic_region_id: MAX_REGION_ID + 1,
                owner_tag_db_id: 47,
                fleet_id: 615,
                task_force_count: 1,
                ship_count: 2,
                flags: FLAG_DERIVED_REGION,
            }],
            ..NavalOobExtensionPayload::default()
        };

        let err = decode_payload(&encode_payload(&payload)).unwrap_err();

        assert!(err.contains("invalid strategic_region_id"));
    }

    #[test]
    fn rejects_ship_name_without_present_flag() {
        let payload = NavalOobExtensionPayload {
            ship_names: vec![ShipNameRow {
                instance_id: 1001,
                flags: 0,
                name: Some("Bad".to_string()),
            }],
            ..NavalOobExtensionPayload::default()
        };

        let err = decode_payload(&encode_payload(&payload)).unwrap_err();

        assert!(err.contains("text without present flag"));
    }

    #[test]
    fn rejects_home_base_value_without_flag() {
        let payload = NavalOobExtensionPayload {
            task_forces: vec![TaskForceExtensionRow {
                task_force_id: 617,
                fleet_id: 615,
                location_province_id: 12389,
                home_base_province_id: 99,
                ..TaskForceExtensionRow::default()
            }],
            ..NavalOobExtensionPayload::default()
        };

        let err = decode_payload(&encode_payload(&payload)).unwrap_err();

        assert!(err.contains("home_base"));
    }
}
