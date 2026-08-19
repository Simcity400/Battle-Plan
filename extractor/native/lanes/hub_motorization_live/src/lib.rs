use std::collections::BTreeSet;

pub const DOMAIN_ID: u16 = 62;
pub const DOMAIN_NAME: &str = "hub_motorization_live";

pub const CGAMESTATE_RVA: u64 = 0x3316E90;
pub const COUNTRY_TABLE_OFF: u64 = 0x310;
pub const COUNTRY_COUNT_OFF: u64 = 0x31C;
pub const COUNTRY_VTABLE_RVA: u64 = 0x27D0E90;
pub const COUNTRY_TAG_DB_ID_OFF: u64 = 0x08;

pub const CSUPPLY_SYSTEM_OFF: u64 = 0x3D8;
pub const CSS_VECTOR_DATA_OFF: u64 = 0x108;
pub const CSS_VECTOR_COUNT_OFF: u64 = 0x110;
pub const CSS_VECTOR_CAP_OFF: u64 = 0x114;

pub const CCSS_VTABLE0_RVA: u64 = 0x298BBF8;
pub const CCSS_VTABLE1_RVA: u64 = 0x298BC48;
pub const CCSS_COUNTRY_OFF: u64 = 0x78;
pub const CCSS_NODE_ARRAY_OFF: u64 = 0x188;
pub const CCSS_NODE_COUNT_OFF: u64 = 0x194;
pub const NODE_RECORD_STRIDE: usize = 0x28;
pub const NODE_PROVINCE_OFF: usize = 0x00;
pub const NODE_STATE_OFF: usize = 0x04;
pub const NODE_MOTOR_MAP_PTR_OFF: usize = 0x10;
pub const NODE_MOTOR_MAP_SIZE_OFF: usize = 0x1c;
pub const MOTOR_MAP_ENTRY_STRIDE: usize = 0x08;
pub const MOTOR_MAP_ACTOR_TAG_DB_ID_OFF: usize = 0x00;
pub const MOTOR_MAP_LEVEL_OFF: usize = 0x04;

pub const MAX_COUNTRY_COUNT: u32 = 65_536;

pub const MAX_COUNTRY_TAG_DB_ID: u32 = (i32::MAX as u32) - 1;
pub const MAX_NODE_COUNT: u32 = 16_384;
pub const MAX_PROVINCE_ID: u32 = (i32::MAX as u32) - 1;
pub const MAX_STATE_ID: u32 = (i32::MAX as u32) - 1;
pub const MAX_MOTOR_MAP_SIZE: u32 = 2_048;

pub const SCHEMA_VERSION: u32 = 2;
pub const PAYLOAD_HEADER_LEN: usize = 32;
pub const ROW_LEN: usize = 32;

pub const ROW_FLAG_COUNTRY_VERIFIED: u32 = 1 << 0;
pub const ROW_FLAG_STRUCT_VERIFIED: u32 = 1 << 1;
pub const ROW_FLAG_NODE_MOTORIZED: u32 = 1 << 2;
pub const ROW_FLAG_ACTOR_VERIFIED: u32 = 1 << 3;
pub const ROW_FLAG_LEVEL_VERIFIED: u32 = 1 << 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HubMotorizationRow {
    pub owner_tag_db_id: u32,
    pub actor_tag_db_id: u32,
    pub province_id: u32,
    pub state_id: u32,
    pub record_index: u32,
    pub map_entry_index: u32,
    pub motorization_level: u32,
    pub flags: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HubMotorizationHeader {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub schema_version: u32,
    pub total_settings_nodes: u32,
    pub total_map_entries: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HubMotorizationGateSummary {
    pub status_flags: u32,
    pub schema_version: u32,
    pub total_settings_nodes: u32,
    pub total_map_entries: u32,
    pub row_len: u32,
    pub row_count: u64,
    pub verified_rows: u64,
    pub motorized_nodes: u64,
    pub map_entries: u64,
    pub level_one_entries: u64,
    pub level_two_entries: u64,
    pub parse_errors: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HubMotorizationCollection {
    pub observed_objects: u32,
    pub settings_objects: u32,
    pub total_settings_nodes: u32,
    pub total_map_entries: u32,
    pub rows: Vec<HubMotorizationRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HubMotorizationError {
    ModuleBaseZero,
    GameStateUnavailable,
    CountryTableUnavailable,
    SupplySystemUnavailable,
    CcssVectorIncoherent,
    CcssVectorRaced,
    CcssScanFailed,
    CcssDuplicateCountry { tag_db_id: u32 },
    CcssMalformed { object: u64, reason: &'static str },
}

pub trait Mem {
    fn u32(&self, addr: u64) -> Option<u32>;
    fn u64(&self, addr: u64) -> Option<u64>;
    fn read_bytes(&self, addr: u64, len: usize) -> Option<Vec<u8>>;
    fn readable_ranges(&self, lo: u64, hi: u64) -> Vec<(u64, usize)>;
}

fn ok_ptr(value: u64, lo: u64, hi: u64, align: u64) -> bool {
    align != 0 && value >= lo && value < hi && value % align == 0
}

pub fn encode_payload(header: &HubMotorizationHeader, rows: &[HubMotorizationRow]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(PAYLOAD_HEADER_LEN + rows.len() * ROW_LEN);
    payload.extend_from_slice(&header.elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&(rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(&header.status_flags.to_le_bytes());
    payload.extend_from_slice(&header.schema_version.to_le_bytes());
    payload.extend_from_slice(&header.total_settings_nodes.to_le_bytes());
    payload.extend_from_slice(&(ROW_LEN as u32).to_le_bytes());
    payload.extend_from_slice(&header.total_map_entries.to_le_bytes());
    debug_assert_eq!(payload.len(), PAYLOAD_HEADER_LEN);
    for row in rows {
        payload.extend_from_slice(&row.owner_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.actor_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.province_id.to_le_bytes());
        payload.extend_from_slice(&row.state_id.to_le_bytes());
        payload.extend_from_slice(&row.record_index.to_le_bytes());
        payload.extend_from_slice(&row.map_entry_index.to_le_bytes());
        payload.extend_from_slice(&row.motorization_level.to_le_bytes());
        payload.extend_from_slice(&row.flags.to_le_bytes());
    }
    payload
}

pub fn decode_payload(payload: &[u8]) -> HubMotorizationGateSummary {
    let mut summary = HubMotorizationGateSummary::default();
    if payload.len() < PAYLOAD_HEADER_LEN {
        summary.parse_errors.push(format!(
            "payload {} < header {}",
            payload.len(),
            PAYLOAD_HEADER_LEN
        ));
        return summary;
    }
    let declared_rows = u32::from_le_bytes(payload[8..12].try_into().unwrap()) as usize;
    summary.status_flags = u32::from_le_bytes(payload[12..16].try_into().unwrap());
    summary.schema_version = u32::from_le_bytes(payload[16..20].try_into().unwrap());
    summary.total_settings_nodes = u32::from_le_bytes(payload[20..24].try_into().unwrap());
    summary.row_len = u32::from_le_bytes(payload[24..28].try_into().unwrap());
    summary.total_map_entries = u32::from_le_bytes(payload[28..32].try_into().unwrap());
    if summary.schema_version != SCHEMA_VERSION {
        summary.parse_errors.push(format!(
            "schema_version {} != {}",
            summary.schema_version, SCHEMA_VERSION
        ));
    }
    if summary.row_len as usize != ROW_LEN {
        summary
            .parse_errors
            .push(format!("row_len {} != {}", summary.row_len, ROW_LEN));
        return summary;
    }
    let expected_len = PAYLOAD_HEADER_LEN.saturating_add(declared_rows.saturating_mul(ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "payload len {} != expected {}",
            payload.len(),
            expected_len
        ));
        return summary;
    }
    for i in 0..declared_rows {
        let offset = PAYLOAD_HEADER_LEN + i * ROW_LEN;
        let row = read_row(payload, offset);
        validate_row(i, &row, &mut summary.parse_errors);
        if row.flags & (ROW_FLAG_COUNTRY_VERIFIED | ROW_FLAG_STRUCT_VERIFIED)
            == (ROW_FLAG_COUNTRY_VERIFIED | ROW_FLAG_STRUCT_VERIFIED)
        {
            summary.verified_rows += 1;
        }
        if row.flags & ROW_FLAG_NODE_MOTORIZED != 0 {
            summary.motorized_nodes += 1;
        }
        summary.map_entries = summary.map_entries.saturating_add(1);
        if row.motorization_level == 1 {
            summary.level_one_entries = summary.level_one_entries.saturating_add(1);
        } else if row.motorization_level == 2 {
            summary.level_two_entries = summary.level_two_entries.saturating_add(1);
        }
        summary.row_count += 1;
    }
    if summary.map_entries != u64::from(summary.total_map_entries) {
        summary.parse_errors.push(format!(
            "decoded map_entries {} != header total_map_entries {}",
            summary.map_entries, summary.total_map_entries
        ));
    }
    summary
}

pub fn decode_payload_rows(
    payload: &[u8],
) -> Result<(HubMotorizationHeader, Vec<HubMotorizationRow>), Vec<String>> {
    let mut errors = Vec::new();
    if payload.len() < PAYLOAD_HEADER_LEN {
        return Err(vec![format!(
            "payload {} < header {}",
            payload.len(),
            PAYLOAD_HEADER_LEN
        )]);
    }
    let declared_rows = u32::from_le_bytes(payload[8..12].try_into().unwrap()) as usize;
    let header = HubMotorizationHeader {
        elapsed_wall_ms: u64::from_le_bytes(payload[0..8].try_into().unwrap()),
        status_flags: u32::from_le_bytes(payload[12..16].try_into().unwrap()),
        schema_version: u32::from_le_bytes(payload[16..20].try_into().unwrap()),
        total_settings_nodes: u32::from_le_bytes(payload[20..24].try_into().unwrap()),
        total_map_entries: u32::from_le_bytes(payload[28..32].try_into().unwrap()),
    };
    let row_len = u32::from_le_bytes(payload[24..28].try_into().unwrap()) as usize;
    if header.schema_version != SCHEMA_VERSION {
        errors.push(format!(
            "schema_version {} != {}",
            header.schema_version, SCHEMA_VERSION
        ));
    }
    if row_len != ROW_LEN {
        errors.push(format!("row_len {} != {}", row_len, ROW_LEN));
    }
    let expected_len = PAYLOAD_HEADER_LEN.saturating_add(declared_rows.saturating_mul(ROW_LEN));
    if payload.len() != expected_len {
        errors.push(format!(
            "payload len {} != expected {}",
            payload.len(),
            expected_len
        ));
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let mut rows = Vec::with_capacity(declared_rows);
    let mut map_entries = 0u64;
    for i in 0..declared_rows {
        let row = read_row(payload, PAYLOAD_HEADER_LEN + i * ROW_LEN);
        validate_row(i, &row, &mut errors);
        map_entries = map_entries.saturating_add(1);
        rows.push(row);
    }
    if map_entries != u64::from(header.total_map_entries) {
        errors.push(format!(
            "decoded map_entries {} != header total_map_entries {}",
            map_entries, header.total_map_entries
        ));
    }
    if errors.is_empty() {
        Ok((header, rows))
    } else {
        Err(errors)
    }
}

fn read_row(payload: &[u8], offset: usize) -> HubMotorizationRow {
    HubMotorizationRow {
        owner_tag_db_id: u32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap()),
        actor_tag_db_id: u32::from_le_bytes(payload[offset + 4..offset + 8].try_into().unwrap()),
        province_id: u32::from_le_bytes(payload[offset + 8..offset + 12].try_into().unwrap()),
        state_id: u32::from_le_bytes(payload[offset + 12..offset + 16].try_into().unwrap()),
        record_index: u32::from_le_bytes(payload[offset + 16..offset + 20].try_into().unwrap()),
        map_entry_index: u32::from_le_bytes(payload[offset + 20..offset + 24].try_into().unwrap()),
        motorization_level: u32::from_le_bytes(
            payload[offset + 24..offset + 28].try_into().unwrap(),
        ),
        flags: u32::from_le_bytes(payload[offset + 28..offset + 32].try_into().unwrap()),
    }
}

fn validate_row(index: usize, row: &HubMotorizationRow, errors: &mut Vec<String>) {
    if row.owner_tag_db_id == 0 || row.owner_tag_db_id > MAX_COUNTRY_TAG_DB_ID {
        errors.push(format!(
            "row {index} owner_tag_db_id {} out of range",
            row.owner_tag_db_id
        ));
    }
    if row.actor_tag_db_id == 0 || row.actor_tag_db_id > MAX_COUNTRY_TAG_DB_ID {
        errors.push(format!(
            "row {index} actor_tag_db_id {} out of range",
            row.actor_tag_db_id
        ));
    }
    if row.province_id == 0 || row.province_id > MAX_PROVINCE_ID {
        errors.push(format!(
            "row {index} province_id {} out of range",
            row.province_id
        ));
    }
    if row.state_id == 0 || row.state_id > MAX_STATE_ID {
        errors.push(format!(
            "row {index} state_id {} out of range",
            row.state_id
        ));
    }
    if row.motorization_level == 0 || row.motorization_level > 2 {
        errors.push(format!(
            "row {index} motorization_level {} out of range",
            row.motorization_level
        ));
    }
    let known_flags = ROW_FLAG_COUNTRY_VERIFIED
        | ROW_FLAG_STRUCT_VERIFIED
        | ROW_FLAG_NODE_MOTORIZED
        | ROW_FLAG_ACTOR_VERIFIED
        | ROW_FLAG_LEVEL_VERIFIED;
    if row.flags & !known_flags != 0 {
        errors.push(format!("row {index} unknown flags {:#x}", row.flags));
    }
    if row.flags & known_flags != known_flags {
        errors.push(format!("row {index} missing required validation flags"));
    }
}

pub fn collect_rows<M: Mem>(
    mem: &M,
    module_base: u64,
    heap_lo: u64,
    heap_hi: u64,
) -> Result<HubMotorizationCollection, HubMotorizationError> {
    if module_base == 0 {
        return Err(HubMotorizationError::ModuleBaseZero);
    }
    let cgs = mem
        .u64(module_base + CGAMESTATE_RVA)
        .filter(|ptr| ok_ptr(*ptr, heap_lo, heap_hi, 8))
        .ok_or(HubMotorizationError::GameStateUnavailable)?;
    let country_table = mem
        .u64(cgs + COUNTRY_TABLE_OFF)
        .filter(|ptr| ok_ptr(*ptr, heap_lo, heap_hi, 8))
        .ok_or(HubMotorizationError::CountryTableUnavailable)?;
    let country_count = mem
        .u32(cgs + COUNTRY_COUNT_OFF)
        .filter(|count| *count <= MAX_COUNTRY_COUNT)
        .ok_or(HubMotorizationError::CountryTableUnavailable)?;
    let objects = anchored_ccss_objects(mem, module_base, heap_lo, heap_hi, cgs)?;
    let mut collection = HubMotorizationCollection {
        observed_objects: objects.len().min(u32::MAX as usize) as u32,
        ..HubMotorizationCollection::default()
    };
    let mut seen_settings_country = BTreeSet::new();
    for object in objects {
        let Some(count) = mem.u32(object + CCSS_NODE_COUNT_OFF) else {
            continue;
        };
        if count == 0 {
            continue;
        }
        if count > MAX_NODE_COUNT {
            continue;
        }
        if !object_passes_candidate_guard(
            mem,
            module_base,
            heap_lo,
            heap_hi,
            object,
            count,
            country_table,
            country_count,
        ) {
            continue;
        }
        let object_rows = derive_object_rows(
            mem,
            module_base,
            heap_lo,
            heap_hi,
            object,
            country_table,
            country_count,
        )?;
        if !seen_settings_country.insert(object_rows.tag_db_id) {
            return Err(HubMotorizationError::CcssDuplicateCountry {
                tag_db_id: object_rows.tag_db_id,
            });
        }
        collection.settings_objects = collection.settings_objects.saturating_add(1);
        collection.total_settings_nodes = collection
            .total_settings_nodes
            .saturating_add(object_rows.node_count);
        collection.total_map_entries = collection
            .total_map_entries
            .saturating_add(object_rows.map_entries);
        collection.rows.extend(object_rows.rows);
    }
    Ok(collection)
}

fn object_passes_candidate_guard<M: Mem>(
    mem: &M,
    module_base: u64,
    heap_lo: u64,
    heap_hi: u64,
    object: u64,
    count: u32,
    country_table: u64,
    country_count: u32,
) -> bool {
    let Some(country) = mem
        .u64(object + CCSS_COUNTRY_OFF)
        .filter(|ptr| ok_ptr(*ptr, heap_lo, heap_hi, 8))
    else {
        return false;
    };
    if mem.u64(country) != Some(module_base + COUNTRY_VTABLE_RVA) {
        return false;
    }
    let Some(tag_db_id) = mem.u32(country + COUNTRY_TAG_DB_ID_OFF) else {
        return false;
    };
    if tag_db_id >= country_count {
        return false;
    }
    if mem.u64(country_table + u64::from(tag_db_id) * 8) != Some(country) {
        return false;
    }
    mem.u64(object + CCSS_NODE_ARRAY_OFF)
        .filter(|ptr| ok_ptr(*ptr, heap_lo, heap_hi, 8))
        .and_then(|arr| {
            (count as usize)
                .checked_mul(NODE_RECORD_STRIDE)
                .map(|len| (arr, len))
        })
        .and_then(|(arr, len)| mem.read_bytes(arr, len))
        .is_some()
}

struct ObjectRows {
    tag_db_id: u32,
    node_count: u32,
    map_entries: u32,
    rows: Vec<HubMotorizationRow>,
}

fn derive_object_rows<M: Mem>(
    mem: &M,
    module_base: u64,
    heap_lo: u64,
    heap_hi: u64,
    object: u64,
    country_table: u64,
    country_count: u32,
) -> Result<ObjectRows, HubMotorizationError> {
    let malformed = |reason| HubMotorizationError::CcssMalformed { object, reason };
    if mem.u64(object) != Some(module_base + CCSS_VTABLE0_RVA)
        || mem.u64(object + 8) != Some(module_base + CCSS_VTABLE1_RVA)
    {
        return Err(malformed("vtable_mismatch"));
    }
    let country = mem
        .u64(object + CCSS_COUNTRY_OFF)
        .filter(|ptr| ok_ptr(*ptr, heap_lo, heap_hi, 8))
        .ok_or_else(|| malformed("country_backptr_unreadable"))?;
    if mem.u64(country) != Some(module_base + COUNTRY_VTABLE_RVA) {
        return Err(malformed("country_vtable_mismatch"));
    }
    let tag_db_id = mem
        .u32(country + COUNTRY_TAG_DB_ID_OFF)
        .filter(|tag| *tag > 0 && *tag < country_count && *tag <= MAX_COUNTRY_TAG_DB_ID)
        .ok_or_else(|| malformed("tag_db_id_out_of_range"))?;
    if mem.u64(country_table + u64::from(tag_db_id) * 8) != Some(country) {
        return Err(malformed("country_table_self_ref_mismatch"));
    }
    let node_count = mem
        .u32(object + CCSS_NODE_COUNT_OFF)
        .filter(|count| *count > 0 && *count <= MAX_NODE_COUNT)
        .ok_or_else(|| malformed("node_count_out_of_range"))?;
    let arr = mem
        .u64(object + CCSS_NODE_ARRAY_OFF)
        .filter(|ptr| ok_ptr(*ptr, heap_lo, heap_hi, 8))
        .ok_or_else(|| malformed("node_array_unreadable"))?;
    let byte_len = (node_count as usize)
        .checked_mul(NODE_RECORD_STRIDE)
        .ok_or_else(|| malformed("node_array_len_overflow"))?;
    let blob = mem
        .read_bytes(arr, byte_len)
        .ok_or_else(|| malformed("node_array_short_read"))?;
    if blob.len() != byte_len {
        return Err(malformed("node_array_short_read"));
    }

    let mut rows = Vec::new();
    let mut map_entries = 0u32;
    for i in 0..node_count as usize {
        let offset = i * NODE_RECORD_STRIDE;
        let province_id = u32::from_le_bytes(
            blob[offset + NODE_PROVINCE_OFF..offset + NODE_PROVINCE_OFF + 4]
                .try_into()
                .unwrap(),
        );
        let state_id = u32::from_le_bytes(
            blob[offset + NODE_STATE_OFF..offset + NODE_STATE_OFF + 4]
                .try_into()
                .unwrap(),
        );
        if province_id == 0 || province_id > MAX_PROVINCE_ID {
            return Err(malformed("province_id_out_of_range"));
        }
        if state_id == 0 || state_id > MAX_STATE_ID {
            return Err(malformed("state_id_out_of_range"));
        }
        let map_ptr = u64::from_le_bytes(
            blob[offset + NODE_MOTOR_MAP_PTR_OFF..offset + NODE_MOTOR_MAP_PTR_OFF + 8]
                .try_into()
                .unwrap(),
        );
        let map_size = u32::from_le_bytes(
            blob[offset + NODE_MOTOR_MAP_SIZE_OFF..offset + NODE_MOTOR_MAP_SIZE_OFF + 4]
                .try_into()
                .unwrap(),
        );
        if map_size > MAX_MOTOR_MAP_SIZE {
            return Err(malformed("motor_map_size_out_of_range"));
        }
        if map_size == 0 {
            continue;
        }
        if !ok_ptr(map_ptr, heap_lo, heap_hi, 8) {
            return Err(malformed("motor_map_ptr_invalid"));
        }
        let map_byte_len = (map_size as usize)
            .checked_mul(MOTOR_MAP_ENTRY_STRIDE)
            .ok_or_else(|| malformed("motor_map_len_overflow"))?;
        let map_blob = mem
            .read_bytes(map_ptr, map_byte_len)
            .filter(|bytes| bytes.len() == map_byte_len)
            .ok_or_else(|| malformed("motor_map_short_read"))?;
        for entry_index in 0..map_size as usize {
            let entry_off = entry_index * MOTOR_MAP_ENTRY_STRIDE;
            let actor_tag_db_id = u32::from_le_bytes(
                map_blob[entry_off + MOTOR_MAP_ACTOR_TAG_DB_ID_OFF
                    ..entry_off + MOTOR_MAP_ACTOR_TAG_DB_ID_OFF + 4]
                    .try_into()
                    .unwrap(),
            );
            if actor_tag_db_id == 0
                || actor_tag_db_id >= country_count
                || actor_tag_db_id > MAX_COUNTRY_TAG_DB_ID
            {
                return Err(malformed("motor_map_country_ref_out_of_range"));
            }
            let actor_country = mem
                .u64(country_table + u64::from(actor_tag_db_id) * 8)
                .filter(|ptr| ok_ptr(*ptr, heap_lo, heap_hi, 8))
                .ok_or_else(|| malformed("motor_map_actor_country_unreadable"))?;
            if mem.u64(actor_country) != Some(module_base + COUNTRY_VTABLE_RVA)
                || mem.u32(actor_country + COUNTRY_TAG_DB_ID_OFF) != Some(actor_tag_db_id)
            {
                return Err(malformed("motor_map_actor_country_mismatch"));
            }
            let motorization_level = u32::from(map_blob[entry_off + MOTOR_MAP_LEVEL_OFF]);
            if !(1..=2).contains(&motorization_level) {
                return Err(malformed("motor_map_level_out_of_range"));
            }
            rows.push(HubMotorizationRow {
                owner_tag_db_id: tag_db_id,
                actor_tag_db_id,
                province_id,
                state_id,
                record_index: i as u32,
                map_entry_index: entry_index as u32,
                motorization_level,
                flags: ROW_FLAG_COUNTRY_VERIFIED
                    | ROW_FLAG_STRUCT_VERIFIED
                    | ROW_FLAG_NODE_MOTORIZED
                    | ROW_FLAG_ACTOR_VERIFIED
                    | ROW_FLAG_LEVEL_VERIFIED,
            });
        }
        let live_header = mem
            .read_bytes(
                arr + (i as u64) * NODE_RECORD_STRIDE as u64 + NODE_MOTOR_MAP_PTR_OFF as u64,
                0x10,
            )
            .filter(|bytes| bytes.len() == 0x10)
            .ok_or_else(|| malformed("motor_map_header_reread_failed"))?;
        let live_ptr = u64::from_le_bytes(live_header[0..8].try_into().unwrap());
        let live_size = u32::from_le_bytes(live_header[12..16].try_into().unwrap());
        if live_ptr != map_ptr || live_size != map_size {
            return Err(malformed("motor_map_raced"));
        }
        map_entries = map_entries.saturating_add(map_size);
    }
    Ok(ObjectRows {
        tag_db_id,
        node_count,
        map_entries,
        rows,
    })
}

fn read_ccss_vector_header<M: Mem>(
    mem: &M,
    supply_system: u64,
    heap_lo: u64,
    heap_hi: u64,
) -> Option<(u64, u32, u32)> {
    let blob = mem.read_bytes(supply_system + CSS_VECTOR_DATA_OFF, 0x10)?;
    if blob.len() != 0x10 {
        return None;
    }
    let data_ptr = u64::from_le_bytes(blob[0..8].try_into().unwrap());
    let count = u32::from_le_bytes(blob[8..12].try_into().unwrap());
    let cap = u32::from_le_bytes(blob[12..16].try_into().unwrap());
    if count > 0 && !ok_ptr(data_ptr, heap_lo, heap_hi, 8) {
        return None;
    }
    Some((data_ptr, count, cap))
}

fn anchored_ccss_objects<M: Mem>(
    mem: &M,
    module_base: u64,
    heap_lo: u64,
    heap_hi: u64,
    cgs: u64,
) -> Result<Vec<u64>, HubMotorizationError> {
    let supply_system = mem
        .u64(cgs + CSUPPLY_SYSTEM_OFF)
        .filter(|ptr| ok_ptr(*ptr, heap_lo, heap_hi, 8))
        .ok_or(HubMotorizationError::SupplySystemUnavailable)?;

    let (data_ptr, count, cap) = read_ccss_vector_header(mem, supply_system, heap_lo, heap_hi)
        .ok_or(HubMotorizationError::CcssVectorIncoherent)?;
    if count > cap || cap > MAX_COUNTRY_COUNT || count > MAX_COUNTRY_COUNT {
        return Err(HubMotorizationError::CcssVectorIncoherent);
    }

    let vtable0 = module_base + CCSS_VTABLE0_RVA;
    let vtable1 = module_base + CCSS_VTABLE1_RVA;
    let country_vtable = module_base + COUNTRY_VTABLE_RVA;
    let mut objects = BTreeSet::new();
    if count > 0 {
        let byte_len = (count as usize)
            .checked_mul(8)
            .ok_or(HubMotorizationError::CcssVectorIncoherent)?;
        let ptrs = mem
            .read_bytes(data_ptr, byte_len)
            .filter(|blob| blob.len() == byte_len)
            .ok_or(HubMotorizationError::CcssScanFailed)?;
        for chunk in ptrs.chunks_exact(8) {
            let ccss = u64::from_le_bytes(chunk.try_into().unwrap());
            if !ok_ptr(ccss, heap_lo, heap_hi, 8) {
                continue;
            }
            if mem.u64(ccss) != Some(vtable0) || mem.u64(ccss + 8) != Some(vtable1) {
                continue;
            }
            let Some(country) = mem
                .u64(ccss + CCSS_COUNTRY_OFF)
                .filter(|ptr| ok_ptr(*ptr, heap_lo, heap_hi, 8))
            else {
                continue;
            };
            if mem.u64(country) != Some(country_vtable) {
                continue;
            }
            objects.insert(ccss);
        }
    }

    let (data_ptr_after, count_after, _cap_after) =
        read_ccss_vector_header(mem, supply_system, heap_lo, heap_hi)
            .ok_or(HubMotorizationError::CcssVectorRaced)?;
    if data_ptr_after != data_ptr || count_after != count {
        return Err(HubMotorizationError::CcssVectorRaced);
    }

    Ok(objects.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const BASE: u64 = 0x1_4000_0000;
    const LO: u64 = 0x1000_0000;
    const HI: u64 = 0x7fff_ffff;

    use std::cell::RefCell;

    #[derive(Default)]
    struct MockMem {
        u64s: HashMap<u64, u64>,
        u32s: HashMap<u64, u32>,
        bytes: Vec<(u64, Vec<u8>)>,
        ranges: Vec<(u64, usize)>,
        race_header: Option<(u64, Vec<u8>)>,
        race_hits: RefCell<usize>,
    }

    impl MockMem {
        fn su64(&mut self, addr: u64, value: u64) {
            self.u64s.insert(addr, value);
        }
        fn su32(&mut self, addr: u64, value: u32) {
            self.u32s.insert(addr, value);
        }
        fn sbytes(&mut self, addr: u64, bytes: Vec<u8>) {
            self.bytes.retain(|(base, _)| *base != addr);
            self.bytes.push((addr, bytes));
        }
        fn set_ccss_vector(
            &mut self,
            cgs: u64,
            supply_system: u64,
            data_ptr: u64,
            ccss_ptrs: &[u64],
            declared_count: u32,
            cap: u32,
        ) {
            self.su64(cgs + CSUPPLY_SYSTEM_OFF, supply_system);
            let mut header = vec![0u8; 0x10];
            header[0..8].copy_from_slice(&data_ptr.to_le_bytes());
            header[8..12].copy_from_slice(&declared_count.to_le_bytes());
            header[12..16].copy_from_slice(&cap.to_le_bytes());
            self.sbytes(supply_system + CSS_VECTOR_DATA_OFF, header);
            let mut arr = vec![0u8; ccss_ptrs.len() * 8];
            for (i, ptr) in ccss_ptrs.iter().enumerate() {
                arr[i * 8..i * 8 + 8].copy_from_slice(&ptr.to_le_bytes());
            }
            self.sbytes(data_ptr, arr);
        }
    }

    impl Mem for MockMem {
        fn u32(&self, addr: u64) -> Option<u32> {
            self.u32s.get(&addr).copied()
        }
        fn u64(&self, addr: u64) -> Option<u64> {
            self.u64s.get(&addr).copied()
        }
        fn read_bytes(&self, addr: u64, len: usize) -> Option<Vec<u8>> {
            if let Some((race_addr, second)) = &self.race_header {
                if addr == *race_addr {
                    let mut hits = self.race_hits.borrow_mut();
                    *hits += 1;
                    if *hits >= 2 && second.len() >= len {
                        return Some(second[..len].to_vec());
                    }
                }
            }
            for (base, bytes) in &self.bytes {
                if addr >= *base {
                    let offset = (addr - *base) as usize;
                    if offset <= bytes.len() && offset.saturating_add(len) <= bytes.len() {
                        return Some(bytes[offset..offset + len].to_vec());
                    }
                }
            }
            None
        }
        fn readable_ranges(&self, _lo: u64, _hi: u64) -> Vec<(u64, usize)> {
            self.ranges.clone()
        }
    }

    fn put_record(
        blob: &mut [u8],
        index: usize,
        province: u32,
        state: u32,
        map_ptr: u64,
        map_size: u32,
    ) {
        let offset = index * NODE_RECORD_STRIDE;
        blob[offset..offset + 4].copy_from_slice(&province.to_le_bytes());
        blob[offset + 4..offset + 8].copy_from_slice(&state.to_le_bytes());
        blob[offset + 0x10..offset + 0x18].copy_from_slice(&map_ptr.to_le_bytes());
        blob[offset + 0x1c..offset + 0x20].copy_from_slice(&map_size.to_le_bytes());
    }

    fn put_motor_entry(blob: &mut [u8], index: usize, country_ref_id: u32, level: u8) {
        let offset = index * MOTOR_MAP_ENTRY_STRIDE;
        blob[offset..offset + 4].copy_from_slice(&country_ref_id.to_le_bytes());
        blob[offset + MOTOR_MAP_LEVEL_OFF] = level;
    }

    fn good_image() -> MockMem {
        let mut mem = MockMem::default();
        let cgs = 0x2000_0000;
        let country_table = 0x2100_0000;
        let country = 0x2200_0000;
        let ccss = 0x2300_0000;
        let node_array = 0x2400_0000;
        let motor_map = 0x2500_0000;
        mem.su64(BASE + CGAMESTATE_RVA, cgs);
        mem.su64(cgs + COUNTRY_TABLE_OFF, country_table);
        mem.su32(cgs + COUNTRY_COUNT_OFF, 440);
        mem.su64(country_table + 16 * 8, country);
        mem.su64(country, BASE + COUNTRY_VTABLE_RVA);
        mem.su32(country + COUNTRY_TAG_DB_ID_OFF, 16);
        mem.su64(ccss, BASE + CCSS_VTABLE0_RVA);
        mem.su64(ccss + 8, BASE + CCSS_VTABLE1_RVA);
        mem.su64(ccss + CCSS_COUNTRY_OFF, country);
        mem.su64(ccss + CCSS_NODE_ARRAY_OFF, node_array);
        mem.su32(ccss + CCSS_NODE_COUNT_OFF, 3);
        let mut scan = vec![0u8; 0x100];
        scan[0..8].copy_from_slice(&(BASE + CCSS_VTABLE0_RVA).to_le_bytes());
        scan[8..16].copy_from_slice(&(BASE + CCSS_VTABLE1_RVA).to_le_bytes());
        mem.sbytes(ccss, scan);
        mem.ranges.push((ccss, 0x100));
        let supply_system = 0x2600_0000;
        let ccss_array = 0x2700_0000;
        mem.set_ccss_vector(cgs, supply_system, ccss_array, &[ccss], 1, 1);
        let mut nodes = vec![0u8; 3 * NODE_RECORD_STRIDE];
        put_record(&mut nodes, 0, 9288, 2, 0, 0);
        put_record(&mut nodes, 1, 13071, 5, motor_map, 2);
        put_record(&mut nodes, 2, 7384, 5, motor_map + 0x40, 1);
        mem.sbytes(node_array, nodes);
        let mut map_a = vec![0u8; 2 * MOTOR_MAP_ENTRY_STRIDE];
        put_motor_entry(&mut map_a, 0, 17, 1);
        put_motor_entry(&mut map_a, 1, 18, 2);
        mem.sbytes(motor_map, map_a);
        let mut map_b = vec![0u8; MOTOR_MAP_ENTRY_STRIDE];
        put_motor_entry(&mut map_b, 0, 19, 2);
        mem.sbytes(motor_map + 0x40, map_b);
        for actor_tag in 17..=19u32 {
            let actor = 0x2200_0000 + u64::from(actor_tag) * 0x1000;
            mem.su64(country_table + u64::from(actor_tag) * 8, actor);
            mem.su64(actor, BASE + COUNTRY_VTABLE_RVA);
            mem.su32(actor + COUNTRY_TAG_DB_ID_OFF, actor_tag);
        }
        mem
    }

    #[test]
    fn collect_rows_emits_exact_actor_levels() {
        let mem = good_image();
        let got = collect_rows(&mem, BASE, LO, HI).expect("collection");
        assert_eq!(got.observed_objects, 1);
        assert_eq!(got.settings_objects, 1);
        assert_eq!(got.total_settings_nodes, 3);
        assert_eq!(got.total_map_entries, 3);
        assert_eq!(
            got.rows,
            vec![
                HubMotorizationRow {
                    owner_tag_db_id: 16,
                    actor_tag_db_id: 17,
                    province_id: 13071,
                    state_id: 5,
                    record_index: 1,
                    map_entry_index: 0,
                    motorization_level: 1,
                    flags: ROW_FLAG_COUNTRY_VERIFIED
                        | ROW_FLAG_STRUCT_VERIFIED
                        | ROW_FLAG_NODE_MOTORIZED
                        | ROW_FLAG_ACTOR_VERIFIED
                        | ROW_FLAG_LEVEL_VERIFIED,
                },
                HubMotorizationRow {
                    owner_tag_db_id: 16,
                    actor_tag_db_id: 18,
                    province_id: 13071,
                    state_id: 5,
                    record_index: 1,
                    map_entry_index: 1,
                    motorization_level: 2,
                    flags: ROW_FLAG_COUNTRY_VERIFIED
                        | ROW_FLAG_STRUCT_VERIFIED
                        | ROW_FLAG_NODE_MOTORIZED
                        | ROW_FLAG_ACTOR_VERIFIED
                        | ROW_FLAG_LEVEL_VERIFIED,
                },
                HubMotorizationRow {
                    owner_tag_db_id: 16,
                    actor_tag_db_id: 19,
                    province_id: 7384,
                    state_id: 5,
                    record_index: 2,
                    map_entry_index: 0,
                    motorization_level: 2,
                    flags: ROW_FLAG_COUNTRY_VERIFIED
                        | ROW_FLAG_STRUCT_VERIFIED
                        | ROW_FLAG_NODE_MOTORIZED
                        | ROW_FLAG_ACTOR_VERIFIED
                        | ROW_FLAG_LEVEL_VERIFIED,
                },
            ]
        );
    }

    #[test]
    fn skips_candidate_on_country_self_ref_mismatch() {
        let mut mem = good_image();
        mem.su64(0x2100_0000 + 16 * 8, 0x2200_0100);
        let got = collect_rows(&mem, BASE, LO, HI).expect("collection");
        assert_eq!(got.observed_objects, 1);
        assert_eq!(got.settings_objects, 0);
        assert!(got.rows.is_empty());
    }

    #[test]
    fn an_authenticated_object_with_an_out_of_envelope_node_fails_closed() {
        for (province, state, reason) in [
            (MAX_PROVINCE_ID + 1, 5u32, "province_id_out_of_range"),
            (13_071, MAX_STATE_ID + 1, "state_id_out_of_range"),
        ] {
            let mut mem = good_image();
            let mut nodes = vec![0u8; 3 * NODE_RECORD_STRIDE];
            put_record(&mut nodes, 0, 9288, 2, 0, 0);
            put_record(&mut nodes, 1, province, state, 0x2500_0000, 2);
            put_record(&mut nodes, 2, 42, 7, 0, 0);
            mem.sbytes(0x2400_0000, nodes);
            assert_eq!(
                collect_rows(&mem, BASE, LO, HI),
                Err(HubMotorizationError::CcssMalformed {
                    object: 0x2300_0000,
                    reason,
                }),
                "province {province} state {state} must be refused by name"
            );
        }
    }

    #[test]
    fn rejects_unknown_inner_level() {
        let mut mem = good_image();
        let mut bad = vec![0u8; 2 * MOTOR_MAP_ENTRY_STRIDE];
        put_motor_entry(&mut bad, 0, 17, 3);
        put_motor_entry(&mut bad, 1, 18, 2);
        mem.sbytes(0x2500_0000, bad);
        assert_eq!(
            collect_rows(&mem, BASE, LO, HI),
            Err(HubMotorizationError::CcssMalformed {
                object: 0x2300_0000,
                reason: "motor_map_level_out_of_range",
            })
        );
    }

    #[test]
    fn rejects_unresolved_inner_country_reference() {
        let mut mem = good_image();
        mem.u64s.remove(&(0x2100_0000 + 17 * 8));
        assert_eq!(
            collect_rows(&mem, BASE, LO, HI),
            Err(HubMotorizationError::CcssMalformed {
                object: 0x2300_0000,
                reason: "motor_map_actor_country_unreadable",
            })
        );
    }

    #[test]
    fn encode_decode_roundtrip() {
        let rows = collect_rows(&good_image(), BASE, LO, HI).unwrap().rows;
        let payload = encode_payload(
            &HubMotorizationHeader {
                elapsed_wall_ms: 5,
                status_flags: 0,
                schema_version: SCHEMA_VERSION,
                total_settings_nodes: 3,
                total_map_entries: 3,
            },
            &rows,
        );
        let summary = decode_payload(&payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(summary.row_count, 3);
        assert_eq!(summary.verified_rows, 3);
        assert_eq!(summary.motorized_nodes, 3);
        assert_eq!(summary.map_entries, 3);
        assert_eq!(summary.level_one_entries, 1);
        assert_eq!(summary.level_two_entries, 2);
        let (header, decoded) = decode_payload_rows(&payload).expect("strict rows");
        assert_eq!(header.total_settings_nodes, 3);
        assert_eq!(header.total_map_entries, 3);
        assert_eq!(decoded, rows);
    }

    struct NoHeapScanMem(MockMem);
    impl Mem for NoHeapScanMem {
        fn u32(&self, addr: u64) -> Option<u32> {
            self.0.u32(addr)
        }
        fn u64(&self, addr: u64) -> Option<u64> {
            self.0.u64(addr)
        }
        fn read_bytes(&self, addr: u64, len: usize) -> Option<Vec<u8>> {
            self.0.read_bytes(addr, len)
        }
        fn readable_ranges(&self, _lo: u64, _hi: u64) -> Vec<(u64, usize)> {
            panic!("hub_motorization_live must not full-heap scan: readable_ranges called");
        }
    }

    #[test]
    fn collect_rows_does_not_full_heap_scan() {
        let anchored =
            collect_rows(&NoHeapScanMem(good_image()), BASE, LO, HI).expect("anchored collection");
        let baseline = collect_rows(&good_image(), BASE, LO, HI).expect("baseline");
        assert_eq!(
            anchored, baseline,
            "no-heap-scan path yields identical rows"
        );
        assert_eq!(anchored.settings_objects, 1);
        assert_eq!(anchored.rows.len(), 3);
    }

    #[test]
    fn anchored_walk_validates_by_vtable_pair_and_country_backref() {
        let cgs = 0x2000_0000u64;
        let objects = anchored_ccss_objects(&good_image(), BASE, LO, HI, cgs).expect("walk");
        assert_eq!(objects, vec![0x2300_0000]);

        let mut mem = good_image();
        mem.su64(0x2300_0000 + 8, BASE + CCSS_VTABLE1_RVA + 0x8);
        let objects = anchored_ccss_objects(&mem, BASE, LO, HI, cgs).expect("walk");
        assert!(objects.is_empty(), "vtable-pair mismatch must reject entry");

        let mut mem = good_image();
        mem.su64(0x2200_0000, BASE + COUNTRY_VTABLE_RVA + 0x10);
        let objects = anchored_ccss_objects(&mem, BASE, LO, HI, cgs).expect("walk");
        assert!(
            objects.is_empty(),
            "country backref mismatch must reject entry"
        );

        let mut mem = good_image();
        mem.set_ccss_vector(cgs, 0x2600_0000, 0x2700_0000, &[0x2300_0000, 0x1], 2, 2);
        let objects = anchored_ccss_objects(&mem, BASE, LO, HI, cgs).expect("walk");
        assert_eq!(
            objects,
            vec![0x2300_0000],
            "garbage entry skipped, valid kept"
        );
    }

    #[test]
    fn anchored_walk_returns_objects_sorted_by_address() {
        let cgs = 0x2000_0000u64;
        let mut mem = good_image();
        let ccss_lo = 0x2300_0000u64;
        let country_hi = 0x2a00_0000u64;
        let ccss_hi = 0x2b00_0000u64;
        let node_array_hi = 0x2c00_0000u64;
        let motor_map_hi = 0x2d00_0000u64;
        mem.su64(0x2100_0000 + 17 * 8, country_hi);
        mem.su64(country_hi, BASE + COUNTRY_VTABLE_RVA);
        mem.su32(country_hi + COUNTRY_TAG_DB_ID_OFF, 17);
        mem.su64(ccss_hi, BASE + CCSS_VTABLE0_RVA);
        mem.su64(ccss_hi + 8, BASE + CCSS_VTABLE1_RVA);
        mem.su64(ccss_hi + CCSS_COUNTRY_OFF, country_hi);
        mem.su64(ccss_hi + CCSS_NODE_ARRAY_OFF, node_array_hi);
        mem.su32(ccss_hi + CCSS_NODE_COUNT_OFF, 1);
        let mut nodes = vec![0u8; NODE_RECORD_STRIDE];
        put_record(&mut nodes, 0, 100, 3, motor_map_hi, 4);
        mem.sbytes(node_array_hi, nodes);
        mem.set_ccss_vector(cgs, 0x2600_0000, 0x2700_0000, &[ccss_hi, ccss_lo], 2, 2);

        let objects = anchored_ccss_objects(&mem, BASE, LO, HI, cgs).expect("walk");
        assert_eq!(objects, vec![ccss_lo, ccss_hi], "ascending-by-address");
    }

    #[test]
    fn anchored_walk_rejects_incoherent_and_raced_vectors() {
        let cgs = 0x2000_0000u64;

        let mut mem = good_image();
        mem.set_ccss_vector(cgs, 0x2600_0000, 0x2700_0000, &[0x2300_0000], 5, 1);
        assert_eq!(
            anchored_ccss_objects(&mem, BASE, LO, HI, cgs),
            Err(HubMotorizationError::CcssVectorIncoherent)
        );

        let mut mem = good_image();
        mem.set_ccss_vector(
            cgs,
            0x2600_0000,
            0x2700_0000,
            &[0x2300_0000],
            MAX_COUNTRY_COUNT + 1,
            MAX_COUNTRY_COUNT + 1,
        );
        assert_eq!(
            anchored_ccss_objects(&mem, BASE, LO, HI, cgs),
            Err(HubMotorizationError::CcssVectorIncoherent)
        );

        let mut mem = good_image();
        mem.set_ccss_vector(cgs, 0x2600_0000, 0x2700_0000, &[0x2300_0000], 1, 1);
        let mut moved = vec![0u8; 0x10];
        moved[0..8].copy_from_slice(&0x2800_0000u64.to_le_bytes());
        moved[8..12].copy_from_slice(&1u32.to_le_bytes());
        moved[12..16].copy_from_slice(&1u32.to_le_bytes());
        mem.race_header = Some((0x2600_0000 + CSS_VECTOR_DATA_OFF, moved));
        assert_eq!(
            anchored_ccss_objects(&mem, BASE, LO, HI, cgs),
            Err(HubMotorizationError::CcssVectorRaced)
        );

        let mut mem = good_image();
        mem.u64s.remove(&(cgs + CSUPPLY_SYSTEM_OFF));
        assert_eq!(
            anchored_ccss_objects(&mem, BASE, LO, HI, cgs),
            Err(HubMotorizationError::SupplySystemUnavailable)
        );
    }

    #[test]
    fn decode_rejects_bad_row_len() {
        let mut payload = encode_payload(
            &HubMotorizationHeader {
                elapsed_wall_ms: 0,
                status_flags: 0,
                schema_version: SCHEMA_VERSION,
                total_settings_nodes: 0,
                total_map_entries: 0,
            },
            &[],
        );
        payload[24..28].copy_from_slice(&16u32.to_le_bytes());
        let summary = decode_payload(&payload);
        assert!(summary.parse_errors.iter().any(|e| e.contains("row_len")));
    }
}
