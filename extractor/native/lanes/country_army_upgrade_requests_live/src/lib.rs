use std::collections::BTreeSet;

pub const DOMAIN_ID: u16 = 93;
pub const DOMAIN_NAME: &str = "country_army_upgrade_requests_live";
pub const MAGIC: u32 = u32::from_le_bytes(*b"AUR1");
pub const SCHEMA_VERSION: u32 = 1;

pub const AUTHORITY_SOURCE_UNAVAILABLE: u32 = 0;
pub const AUTHORITY_EXACT_A729_RECEIPT: u32 = 1;

pub const COUNTRY_UPGRADE_STATUS_PTR_OFF: u64 = 0xF80;
pub const STATUS_VTABLE_RVA: u64 = 0x29BB408;
pub const STATUS_PERSISTENT_VTABLE_OFF: u64 = 0x18;
pub const STATUS_PERSISTENT_VTABLE_RVA: u64 = 0x29BB440;
pub const STATUS_GROUP_BEGIN_OFF: u64 = 0x20;
pub const STATUS_GROUP_CAPACITY_OFF: u64 = 0x28;
pub const STATUS_GROUP_COUNT_OFF: u64 = 0x2C;

pub const REQUESTS_VTABLE_RVA: u64 = 0x29BB380;
pub const REQUESTS_VERSION_OFF: u64 = 0x08;
pub const REQUESTS_VERSION: u32 = 1;
pub const REQUESTS_PERSISTENT_VTABLE_OFF: u64 = 0x18;
pub const REQUESTS_PERSISTENT_VTABLE_RVA: u64 = 0x29BB3B8;
pub const REQUESTS_ARMY_PTR_OFF: u64 = 0x20;
pub const REQUESTS_STATUS_PTR_OFF: u64 = 0x28;
pub const REQUEST_ROW_BEGIN_OFF: u64 = 0x30;
pub const REQUEST_ROW_CAPACITY_OFF: u64 = 0x38;
pub const REQUEST_ROW_COUNT_OFF: u64 = 0x3C;
pub const REQUEST_ROW_STRIDE: u64 = 0x18;
pub const REQUEST_ROW_EQUIPMENT_PTR_OFF: u64 = 0x00;
pub const REQUEST_ROW_REQUEST_OFF: u64 = 0x08;
pub const REQUEST_ROW_TOTAL_OFF: u64 = 0x10;
pub const DELIVERY_BEGIN_OFF: u64 = 0x48;
pub const DELIVERY_CAPACITY_OFF: u64 = 0x50;
pub const DELIVERY_COUNT_OFF: u64 = 0x54;

pub const ARMY_VTABLE_RVA: u64 = 0x2943AD0;
pub const ARMY_PERSISTENT_VTABLE_OFF: u64 = 0x10;
pub const ARMY_PERSISTENT_VTABLE_RVA: u64 = 0x2943CB0;
pub const ARMY_ID_OFF: u64 = 0x18;
pub const ARMY_ID_TYPE: u32 = 4_713;
pub const ARMY_ID_ALT_TYPE: u32 = 51;

pub fn is_army_identity_type(kind: u32) -> bool {
    kind == ARMY_ID_TYPE || kind == ARMY_ID_ALT_TYPE
}

pub const EQUIPMENT_VARIANT_VTABLE_RVA: u64 = 0x293AFB8;
pub const EQUIPMENT_VARIANT_ID_OFF: u64 = 0x08;
pub const EQUIPMENT_VARIANT_ID_TYPE: u32 = 70;

pub const DELIVERY_VTABLE_RVA: u64 = 0x29BB2E0;
pub const DELIVERY_STATUS_OFF: u64 = 0x08;
pub const DELIVERY_PROGRESS_OFF: u64 = 0x10;
pub const DELIVERY_TOTAL_PROGRESS_OFF: u64 = 0x18;
pub const DELIVERY_PRODUCED_POOL_OFF: u64 = 0x20;
pub const EQUIPMENT_VARIANT_POOL_VTABLE_RVA: u64 = 0x27330A0;
pub const PRODUCED_ROW_BEGIN_OFF: u64 = 0x40;
pub const PRODUCED_ROW_CAPACITY_OFF: u64 = 0x48;
pub const PRODUCED_ROW_COUNT_OFF: u64 = 0x4C;
pub const PRODUCED_ROW_ALLOCATOR_OFF: u64 = 0x50;
pub const PRODUCED_ALLOW_ZERO_OFF: u64 = 0x58;
pub const PRODUCED_ROW_STRIDE: u64 = 0x10;

pub const FIXED_POINT_SCALE: i64 = 100_000;
pub const MAX_COUNTRIES: usize = i32::MAX as usize;

const MIN_COUNTRY_ENTRY_LEN: usize = 8;
pub const MAX_GROUPS_PER_COUNTRY: usize = 4_096;
pub const MAX_REQUEST_ROWS_PER_GROUP: usize = 4_096;
pub const MAX_DELIVERIES_PER_GROUP: usize = 4_096;
pub const MAX_PRODUCED_ROWS_PER_DELIVERY: usize = 4_096;
pub const MAX_TOTAL_LEAF_ROWS: usize = 250_000;

pub const EXECUTABLE_SHA256: [u8; 32] =
    hex32(b"870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad");
pub const STATUS_REQUEST_PERSISTENCE_MANIFEST_SHA256: [u8; 32] =
    hex32(b"316b91f5773aa57d8688233330fe014ebed49e6efac3d4a6e14bcad8d3c97dd4");
pub const DELIVERY_PERSISTENCE_MANIFEST_SHA256: [u8; 32] =
    hex32(b"637f5a3e3a9c92a9d7243c93fc2e729c6b4fec01dc9172a4d001657d1eda95d4");
pub const PRODUCED_POOL_PERSISTENCE_MANIFEST_SHA256: [u8; 32] =
    hex32(b"702cae9cd3537e2864eb83a4b8c2915ed6c30473dabfcf7dc24af2f42a45b556");
pub const CARMY_IDENTITY_MANIFEST_SHA256: [u8; 32] =
    hex32(b"f14032b6a225a4d5ff908b2296cd36d510a89aaa9670bd8f3597d5947e7fb6c7");
pub const LIVE_AST_ROWS_SHA256: [u8; 32] =
    hex32(b"5fe6a7095288b5df235578ddd414fc774858a42cc69f6ec14117b4baaa08c352");
pub const LIVE_AST_ARMY_IDS_SHA256: [u8; 32] =
    hex32(b"06e8772cb26c2963ec3113825590b386a197a8589b833866fc2e3cefeaaf8cdd");

const fn hex32(input: &[u8; 64]) -> [u8; 32] {
    const fn nibble(value: u8) -> u8 {
        match value {
            b'0'..=b'9' => value - b'0',
            b'a'..=b'f' => value - b'a' + 10,
            b'A'..=b'F' => value - b'A' + 10,
            _ => 0xff,
        }
    }
    let mut out = [0; 32];
    let mut index = 0;
    while index < 32 {
        out[index] = (nibble(input[index * 2]) << 4) | nibble(input[index * 2 + 1]);
        index += 1;
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PersistentId {
    pub kind: u32,
    pub id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmyUpgradeRequestRow {
    pub equipment_variant: PersistentId,
    pub request_raw: i64,
    pub total_raw: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProducedEquipmentRow {
    pub equipment_variant: PersistentId,
    pub amount_raw: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpgradeDeliveryRow {
    pub status: u32,
    pub progress_raw: Option<i64>,
    pub total_progress_raw: i64,
    pub allow_zero_entries: bool,
    pub produced: Vec<ProducedEquipmentRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmyUpgradeGroup {
    pub army: PersistentId,
    pub requests: Vec<ArmyUpgradeRequestRow>,
    pub deliveries: Vec<UpgradeDeliveryRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountryArmyUpgradeRow {
    pub tag_db_id: u32,
    pub groups: Vec<ArmyUpgradeGroup>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmyUpgradeSnapshot {
    pub source_sequence: u64,
    pub countries: Vec<CountryArmyUpgradeRow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityState {
    SourceUnavailable,
    ExactA729Receipt,
}

impl AuthorityState {
    const fn raw(self) -> u32 {
        match self {
            Self::SourceUnavailable => AUTHORITY_SOURCE_UNAVAILABLE,
            Self::ExactA729Receipt => AUTHORITY_EXACT_A729_RECEIPT,
        }
    }

    fn from_raw(value: u32) -> Result<Self, CarrierError> {
        match value {
            AUTHORITY_SOURCE_UNAVAILABLE => Ok(Self::SourceUnavailable),
            AUTHORITY_EXACT_A729_RECEIPT => Ok(Self::ExactA729Receipt),
            _ => invalid("AUR1 authority is unknown"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmyUpgradeFrame {
    pub elapsed_wall_ms: u64,
    pub frame_hour: u64,
    pub status_flags: u32,
    pub authority: AuthorityState,
    pub snapshot: Option<ArmyUpgradeSnapshot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CountryRoot {
    pub tag_db_id: u32,
    pub object_ptr: u64,
}

pub trait ArmyUpgradeMemory {
    fn read_exact(&self, address: u64, out: &mut [u8]) -> Result<(), ()>;
}

pub trait ExactArmyUpgradeSource {
    fn is_exact_a729_country_army_upgrade_source(&self) -> bool;
    fn read_country_army_upgrades(&self) -> Result<ArmyUpgradeSnapshot, CarrierError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CarrierError {
    Source(String),
    CountryRead {
        tag_db_id: u32,
        object_ptr: u64,
        source: Box<CarrierError>,
    },
    Memory {
        address: u64,
        width: usize,
    },
    Invalid(String),
    Truncated,
    TrailingBytes,
}

/// The engine mutates the AUR1 object graph concurrently with this walk, so a
/// single pass pair can straddle a mutation even though the graph settles
/// again inside the same frame bracket (lane QZ: 18 of 442 d93 frames in the
/// sealed pw_gate_final capture failed the old fixed two-pass check, every
/// one of them while the graph was mid-mutation). The coherent-read contract
/// is therefore restartable: repeat the pass until two consecutive passes
/// agree or the bound is hit. Restarts are plain re-reads -- they never wait,
/// sleep, pause the engine, or widen the frame bracket -- and exhausting the
/// bound still refuses under its own name so a persistently torn graph is
/// never published as measured data.
pub const COHERENT_READ_MAX_PASSES: usize = 8;

pub fn read_snapshot_from_country_roots<M: ArmyUpgradeMemory>(
    memory: &M,
    module_base: u64,
    source_sequence: u64,
    roots: &[CountryRoot],
) -> Result<ArmyUpgradeSnapshot, CarrierError> {
    let mut previous = read_snapshot_once(memory, module_base, source_sequence, roots)?;
    for _pass in 1..COHERENT_READ_MAX_PASSES {
        let current = read_snapshot_once(memory, module_base, source_sequence, roots)?;
        if current == previous {
            return Ok(current);
        }
        previous = current;
    }
    invalid("AUR1 object graph kept changing across every coherent-read restart")
}

fn read_snapshot_once<M: ArmyUpgradeMemory>(
    memory: &M,
    module_base: u64,
    source_sequence: u64,
    roots: &[CountryRoot],
) -> Result<ArmyUpgradeSnapshot, CarrierError> {
    if module_base == 0 || source_sequence == 0 || roots.len() > MAX_COUNTRIES {
        return invalid("AUR1 country-root snapshot header is invalid");
    }
    let mut country_ids = BTreeSet::new();
    let mut countries = Vec::with_capacity(roots.len());
    let mut total_leaf_rows = 0usize;
    for root in roots {
        if root.tag_db_id == 0 || root.object_ptr == 0 || !country_ids.insert(root.tag_db_id) {
            return invalid("AUR1 country identity is zero or duplicated");
        }
        let groups_result = (|| -> Result<Vec<ArmyUpgradeGroup>, CarrierError> {
            let owner = read_u64(
                memory,
                add(root.object_ptr, COUNTRY_UPGRADE_STATUS_PTR_OFF)?,
            )?;
            let mut groups = Vec::new();
            if owner == 0 {
                return Ok(groups);
            }
            require_vtable(memory, owner, module_base, STATUS_VTABLE_RVA)?;
            require_vtable(
                memory,
                add(owner, STATUS_PERSISTENT_VTABLE_OFF)?,
                module_base,
                STATUS_PERSISTENT_VTABLE_RVA,
            )?;
            let (begin, count) = read_vector_header(
                memory,
                owner,
                STATUS_GROUP_BEGIN_OFF,
                STATUS_GROUP_CAPACITY_OFF,
                STATUS_GROUP_COUNT_OFF,
                MAX_GROUPS_PER_COUNTRY,
                "army-upgrade group",
            )?;
            let mut army_ids = BTreeSet::new();
            groups.reserve(count);
            for index in 0..count {
                let group_ptr = read_u64(memory, add(begin, (index as u64) * 8)?)?;
                if group_ptr == 0 {
                    return invalid("AUR1 group pointer is null");
                }
                require_vtable(memory, group_ptr, module_base, REQUESTS_VTABLE_RVA)?;
                require_vtable(
                    memory,
                    add(group_ptr, REQUESTS_PERSISTENT_VTABLE_OFF)?,
                    module_base,
                    REQUESTS_PERSISTENT_VTABLE_RVA,
                )?;
                if read_u32(memory, add(group_ptr, REQUESTS_VERSION_OFF)?)? != REQUESTS_VERSION {
                    return invalid("AUR1 request object version is not exact a729 version 1");
                }
                if read_u64(memory, add(group_ptr, REQUESTS_STATUS_PTR_OFF)?)? != owner {
                    return invalid("AUR1 request object status back-reference disagrees");
                }
                let army_ptr = read_u64(memory, add(group_ptr, REQUESTS_ARMY_PTR_OFF)?)?;
                if army_ptr == 0 {
                    return invalid("AUR1 request object has null CArmy identity");
                }
                require_vtable(memory, army_ptr, module_base, ARMY_VTABLE_RVA)?;
                require_vtable(
                    memory,
                    add(army_ptr, ARMY_PERSISTENT_VTABLE_OFF)?,
                    module_base,
                    ARMY_PERSISTENT_VTABLE_RVA,
                )?;
                let army = read_persistent_id(memory, add(army_ptr, ARMY_ID_OFF)?)?;
                if !is_army_identity_type(army.kind) || army.id == 0 || !army_ids.insert(army) {
                    return invalid("AUR1 CArmy persistent identity is invalid or duplicated");
                }
                let requests = read_request_rows(memory, module_base, group_ptr)?;
                let deliveries = read_deliveries(memory, module_base, group_ptr)?;
                total_leaf_rows = total_leaf_rows
                    .checked_add(requests.len())
                    .and_then(|value| value.checked_add(deliveries.len()))
                    .and_then(|value| {
                        deliveries
                            .iter()
                            .try_fold(value, |sum, row| sum.checked_add(row.produced.len()))
                    })
                    .ok_or_else(|| CarrierError::Invalid("AUR1 leaf count overflow".into()))?;
                if total_leaf_rows > MAX_TOTAL_LEAF_ROWS {
                    return invalid("AUR1 total leaf count exceeds bound");
                }
                groups.push(ArmyUpgradeGroup {
                    army,
                    requests,
                    deliveries,
                });
            }
            if read_vector_header(
                memory,
                owner,
                STATUS_GROUP_BEGIN_OFF,
                STATUS_GROUP_CAPACITY_OFF,
                STATUS_GROUP_COUNT_OFF,
                MAX_GROUPS_PER_COUNTRY,
                "army-upgrade group",
            )? != (begin, count)
                || read_u64(
                    memory,
                    add(root.object_ptr, COUNTRY_UPGRADE_STATUS_PTR_OFF)?,
                )? != owner
            {
                return invalid("AUR1 country/status graph changed during snapshot");
            }
            Ok(groups)
        })();
        let mut groups = groups_result.map_err(|source| CarrierError::CountryRead {
            tag_db_id: root.tag_db_id,
            object_ptr: root.object_ptr,
            source: Box::new(source),
        })?;
        groups.sort_by_key(|group| group.army);
        countries.push(CountryArmyUpgradeRow {
            tag_db_id: root.tag_db_id,
            groups,
        });
    }
    countries.sort_by_key(|country| country.tag_db_id);
    let snapshot = ArmyUpgradeSnapshot {
        source_sequence,
        countries,
    };
    validate_snapshot(&snapshot)?;
    Ok(snapshot)
}

fn read_request_rows<M: ArmyUpgradeMemory>(
    memory: &M,
    module_base: u64,
    group_ptr: u64,
) -> Result<Vec<ArmyUpgradeRequestRow>, CarrierError> {
    let (begin, count) = read_vector_header(
        memory,
        group_ptr,
        REQUEST_ROW_BEGIN_OFF,
        REQUEST_ROW_CAPACITY_OFF,
        REQUEST_ROW_COUNT_OFF,
        MAX_REQUEST_ROWS_PER_GROUP,
        "army-upgrade request",
    )?;
    let mut rows = Vec::with_capacity(count);
    for index in 0..count {
        let row = add(begin, (index as u64) * REQUEST_ROW_STRIDE)?;
        let equipment_ptr = read_u64(memory, add(row, REQUEST_ROW_EQUIPMENT_PTR_OFF)?)?;
        let equipment_variant = read_equipment_id(memory, module_base, equipment_ptr)?;
        rows.push(ArmyUpgradeRequestRow {
            equipment_variant,
            request_raw: read_i64(memory, add(row, REQUEST_ROW_REQUEST_OFF)?)?,
            total_raw: read_i64(memory, add(row, REQUEST_ROW_TOTAL_OFF)?)?,
        });
    }
    if read_vector_header(
        memory,
        group_ptr,
        REQUEST_ROW_BEGIN_OFF,
        REQUEST_ROW_CAPACITY_OFF,
        REQUEST_ROW_COUNT_OFF,
        MAX_REQUEST_ROWS_PER_GROUP,
        "army-upgrade request",
    )? != (begin, count)
    {
        return invalid("AUR1 request vector changed during snapshot");
    }
    Ok(rows)
}

fn read_deliveries<M: ArmyUpgradeMemory>(
    memory: &M,
    module_base: u64,
    group_ptr: u64,
) -> Result<Vec<UpgradeDeliveryRow>, CarrierError> {
    let (begin, count) = read_vector_header(
        memory,
        group_ptr,
        DELIVERY_BEGIN_OFF,
        DELIVERY_CAPACITY_OFF,
        DELIVERY_COUNT_OFF,
        MAX_DELIVERIES_PER_GROUP,
        "army-upgrade delivery",
    )?;
    let mut rows = Vec::with_capacity(count);
    for index in 0..count {
        let delivery = read_u64(memory, add(begin, (index as u64) * 8)?)?;
        if delivery == 0 {
            return invalid("AUR1 delivery pointer is null");
        }
        require_vtable(memory, delivery, module_base, DELIVERY_VTABLE_RVA)?;
        require_vtable(
            memory,
            add(delivery, DELIVERY_PRODUCED_POOL_OFF)?,
            module_base,
            EQUIPMENT_VARIANT_POOL_VTABLE_RVA,
        )?;
        let (produced_begin, produced_count) = read_vector_header(
            memory,
            delivery,
            PRODUCED_ROW_BEGIN_OFF,
            PRODUCED_ROW_CAPACITY_OFF,
            PRODUCED_ROW_COUNT_OFF,
            MAX_PRODUCED_ROWS_PER_DELIVERY,
            "army-upgrade produced equipment",
        )?;
        let mut produced = Vec::with_capacity(produced_count);
        for produced_index in 0..produced_count {
            let row = add(
                produced_begin,
                (produced_index as u64) * PRODUCED_ROW_STRIDE,
            )?;
            let equipment_ptr = read_u64(memory, row)?;
            produced.push(ProducedEquipmentRow {
                equipment_variant: read_equipment_id(memory, module_base, equipment_ptr)?,
                amount_raw: read_i64(memory, add(row, 8)?)?,
            });
        }
        if read_vector_header(
            memory,
            delivery,
            PRODUCED_ROW_BEGIN_OFF,
            PRODUCED_ROW_CAPACITY_OFF,
            PRODUCED_ROW_COUNT_OFF,
            MAX_PRODUCED_ROWS_PER_DELIVERY,
            "army-upgrade produced equipment",
        )? != (produced_begin, produced_count)
        {
            return invalid("AUR1 produced vector changed during snapshot");
        }
        let status = read_u32(memory, add(delivery, DELIVERY_STATUS_OFF)?)?;
        rows.push(UpgradeDeliveryRow {
            status,
            progress_raw: if status == 1 {
                Some(read_i64(memory, add(delivery, DELIVERY_PROGRESS_OFF)?)?)
            } else {
                None
            },
            total_progress_raw: read_i64(memory, add(delivery, DELIVERY_TOTAL_PROGRESS_OFF)?)?,
            allow_zero_entries: read_u8(memory, add(delivery, PRODUCED_ALLOW_ZERO_OFF)?)? != 0,
            produced,
        });
    }
    if read_vector_header(
        memory,
        group_ptr,
        DELIVERY_BEGIN_OFF,
        DELIVERY_CAPACITY_OFF,
        DELIVERY_COUNT_OFF,
        MAX_DELIVERIES_PER_GROUP,
        "army-upgrade delivery",
    )? != (begin, count)
    {
        return invalid("AUR1 delivery vector changed during snapshot");
    }
    Ok(rows)
}

fn read_equipment_id<M: ArmyUpgradeMemory>(
    memory: &M,
    module_base: u64,
    equipment_ptr: u64,
) -> Result<PersistentId, CarrierError> {
    if equipment_ptr == 0 {
        return invalid("AUR1 equipment variant pointer is null");
    }
    require_vtable(
        memory,
        equipment_ptr,
        module_base,
        EQUIPMENT_VARIANT_VTABLE_RVA,
    )?;
    let identity = read_persistent_id(memory, add(equipment_ptr, EQUIPMENT_VARIANT_ID_OFF)?)?;
    if identity.kind != EQUIPMENT_VARIANT_ID_TYPE || identity.id == 0 {
        return invalid("AUR1 equipment_variant_index identity is invalid");
    }
    Ok(identity)
}

fn read_vector_header<M: ArmyUpgradeMemory>(
    memory: &M,
    object: u64,
    begin_off: u64,
    capacity_off: u64,
    count_off: u64,
    maximum: usize,
    label: &str,
) -> Result<(u64, usize), CarrierError> {
    let begin = read_u64(memory, add(object, begin_off)?)?;
    let capacity = read_i32(memory, add(object, capacity_off)?)?;
    let count = read_i32(memory, add(object, count_off)?)?;
    if capacity < 0 || count < 0 || count > capacity || count as usize > maximum {
        return invalid(format!("AUR1 {label} vector header is invalid"));
    }
    if count != 0 && begin == 0 {
        return invalid(format!("AUR1 {label} vector begin is null"));
    }
    Ok((begin, count as usize))
}

fn require_vtable<M: ArmyUpgradeMemory>(
    memory: &M,
    object: u64,
    module_base: u64,
    expected_rva: u64,
) -> Result<(), CarrierError> {
    let expected = add(module_base, expected_rva)?;
    if read_u64(memory, object)? != expected {
        return invalid(format!("AUR1 vtable mismatch at 0x{object:x}"));
    }
    Ok(())
}

fn read_persistent_id<M: ArmyUpgradeMemory>(
    memory: &M,
    address: u64,
) -> Result<PersistentId, CarrierError> {
    Ok(PersistentId {
        kind: read_u32(memory, address)?,
        id: read_u32(memory, add(address, 4)?)?,
    })
}

pub fn collect_and_encode<S: ExactArmyUpgradeSource>(
    source: &S,
    frame_hour: u64,
    elapsed_wall_ms: u64,
) -> Result<Vec<u8>, CarrierError> {
    if !source.is_exact_a729_country_army_upgrade_source() {
        return Err(CarrierError::Source(
            "source did not authenticate exact a729 CCountry+0xF80 army upgrades".into(),
        ));
    }
    let snapshot = source.read_country_army_upgrades()?;
    encode_payload(&ArmyUpgradeFrame {
        elapsed_wall_ms,
        frame_hour,
        status_flags: 0,
        authority: AuthorityState::ExactA729Receipt,
        snapshot: Some(snapshot),
    })
}

pub fn encode_source_unavailable(
    frame_hour: u64,
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> Result<Vec<u8>, CarrierError> {
    encode_payload(&ArmyUpgradeFrame {
        elapsed_wall_ms,
        frame_hour,
        status_flags,
        authority: AuthorityState::SourceUnavailable,
        snapshot: None,
    })
}

pub fn validate_snapshot(snapshot: &ArmyUpgradeSnapshot) -> Result<(), CarrierError> {
    if snapshot.source_sequence == 0 || snapshot.countries.len() > MAX_COUNTRIES {
        return invalid("AUR1 snapshot header is invalid");
    }
    let mut countries = BTreeSet::new();
    let mut leaves = 0usize;
    let mut prior_country = None;
    for country in &snapshot.countries {
        if country.tag_db_id == 0
            || !countries.insert(country.tag_db_id)
            || prior_country.is_some_and(|prior| prior >= country.tag_db_id)
            || country.groups.len() > MAX_GROUPS_PER_COUNTRY
        {
            return invalid("AUR1 country order/identity/count is invalid");
        }
        prior_country = Some(country.tag_db_id);
        let mut armies = BTreeSet::new();
        let mut prior_army = None;
        for group in &country.groups {
            if !is_army_identity_type(group.army.kind)
                || group.army.id == 0
                || !armies.insert(group.army)
                || prior_army.is_some_and(|prior| prior >= group.army)
                || group.requests.len() > MAX_REQUEST_ROWS_PER_GROUP
                || group.deliveries.len() > MAX_DELIVERIES_PER_GROUP
            {
                return invalid("AUR1 army group order/identity/count is invalid");
            }
            prior_army = Some(group.army);
            for row in &group.requests {
                validate_equipment_id(row.equipment_variant)?;
            }
            for delivery in &group.deliveries {
                if delivery.produced.len() > MAX_PRODUCED_ROWS_PER_DELIVERY {
                    return invalid("AUR1 produced row count exceeds bound");
                }
                if (delivery.status == 1) != delivery.progress_raw.is_some() {
                    return invalid("AUR1 delivery progress presence disagrees with status");
                }
                for row in &delivery.produced {
                    validate_equipment_id(row.equipment_variant)?;
                }
            }
            leaves = leaves
                .checked_add(group.requests.len())
                .and_then(|value| value.checked_add(group.deliveries.len()))
                .and_then(|value| {
                    group
                        .deliveries
                        .iter()
                        .try_fold(value, |sum, row| sum.checked_add(row.produced.len()))
                })
                .ok_or_else(|| CarrierError::Invalid("AUR1 leaf count overflow".into()))?;
        }
    }
    if leaves > MAX_TOTAL_LEAF_ROWS {
        return invalid("AUR1 total leaf count exceeds bound");
    }
    Ok(())
}

fn validate_equipment_id(identity: PersistentId) -> Result<(), CarrierError> {
    if identity.kind != EQUIPMENT_VARIANT_ID_TYPE || identity.id == 0 {
        return invalid("AUR1 equipment_variant_index identity is invalid");
    }
    Ok(())
}

pub fn validate_frame(frame: &ArmyUpgradeFrame) -> Result<(), CarrierError> {
    match frame.authority {
        AuthorityState::SourceUnavailable => {
            if frame.status_flags == 0 || frame.snapshot.is_some() {
                return invalid("unavailable AUR1 frame must be nonclean and rowless");
            }
        }
        AuthorityState::ExactA729Receipt => {
            if frame.status_flags != 0 || frame.snapshot.is_none() {
                return invalid("receipted AUR1 frame must be clean and measured");
            }
            validate_snapshot(frame.snapshot.as_ref().unwrap())?;
        }
    }
    Ok(())
}

pub fn encode_payload(frame: &ArmyUpgradeFrame) -> Result<Vec<u8>, CarrierError> {
    validate_frame(frame)?;
    let mut out = Vec::new();
    u32v(&mut out, MAGIC);
    u32v(&mut out, SCHEMA_VERSION);
    u32v(&mut out, frame.authority.raw());
    u32v(&mut out, frame.status_flags);
    u64v(&mut out, frame.elapsed_wall_ms);
    u64v(&mut out, frame.frame_hour);
    out.extend_from_slice(&EXECUTABLE_SHA256);
    out.extend_from_slice(&STATUS_REQUEST_PERSISTENCE_MANIFEST_SHA256);
    out.extend_from_slice(&DELIVERY_PERSISTENCE_MANIFEST_SHA256);
    out.extend_from_slice(&PRODUCED_POOL_PERSISTENCE_MANIFEST_SHA256);
    out.extend_from_slice(&CARMY_IDENTITY_MANIFEST_SHA256);
    out.extend_from_slice(&LIVE_AST_ROWS_SHA256);
    out.extend_from_slice(&LIVE_AST_ARMY_IDS_SHA256);
    out.push(u8::from(frame.snapshot.is_some()));
    out.extend_from_slice(&[0; 3]);
    let Some(snapshot) = &frame.snapshot else {
        return Ok(out);
    };
    u64v(&mut out, snapshot.source_sequence);
    u32v(&mut out, snapshot.countries.len() as u32);
    for country in &snapshot.countries {
        u32v(&mut out, country.tag_db_id);
        u32v(&mut out, country.groups.len() as u32);
        for group in &country.groups {
            idv(&mut out, group.army);
            u32v(&mut out, group.requests.len() as u32);
            u32v(&mut out, group.deliveries.len() as u32);
            for row in &group.requests {
                idv(&mut out, row.equipment_variant);
                i64v(&mut out, row.request_raw);
                i64v(&mut out, row.total_raw);
            }
            for delivery in &group.deliveries {
                u32v(&mut out, delivery.status);
                u32v(&mut out, u32::from(delivery.progress_raw.is_some()));
                i64v(&mut out, delivery.progress_raw.unwrap_or(0));
                i64v(&mut out, delivery.total_progress_raw);
                out.push(u8::from(delivery.allow_zero_entries));
                out.extend_from_slice(&[0; 3]);
                u32v(&mut out, delivery.produced.len() as u32);
                for row in &delivery.produced {
                    idv(&mut out, row.equipment_variant);
                    i64v(&mut out, row.amount_raw);
                }
            }
        }
    }
    Ok(out)
}

pub fn decode_payload(payload: &[u8]) -> Result<ArmyUpgradeFrame, CarrierError> {
    let mut reader = Reader { payload, at: 0 };
    if reader.u32()? != MAGIC || reader.u32()? != SCHEMA_VERSION {
        return invalid("AUR1 magic/schema mismatch");
    }
    let authority = AuthorityState::from_raw(reader.u32()?)?;
    let status_flags = reader.u32()?;
    let elapsed_wall_ms = reader.u64()?;
    let frame_hour = reader.u64()?;
    for (expected, label) in [
        (EXECUTABLE_SHA256, "executable"),
        (
            STATUS_REQUEST_PERSISTENCE_MANIFEST_SHA256,
            "request persistence",
        ),
        (DELIVERY_PERSISTENCE_MANIFEST_SHA256, "delivery persistence"),
        (
            PRODUCED_POOL_PERSISTENCE_MANIFEST_SHA256,
            "produced pool persistence",
        ),
        (CARMY_IDENTITY_MANIFEST_SHA256, "CArmy identity"),
        (LIVE_AST_ROWS_SHA256, "live AST request rows"),
        (LIVE_AST_ARMY_IDS_SHA256, "live AST CArmy identities"),
    ] {
        if reader.bytes(32)? != expected {
            return invalid(format!("AUR1 {label} authority mismatch"));
        }
    }
    let has_snapshot = reader.bool()?;
    if reader.bytes(3)? != [0, 0, 0] {
        return invalid("AUR1 availability reserved bytes are nonzero");
    }
    let snapshot = if has_snapshot {
        let source_sequence = reader.u64()?;
        let country_count = reader.bounded_count(MAX_COUNTRIES, "country")?;
        if country_count > reader.remaining() / MIN_COUNTRY_ENTRY_LEN {
            return invalid("AUR1 country count exceeds the payload's capacity");
        }
        let mut countries = Vec::with_capacity(country_count);
        let mut leaves = 0usize;
        for _ in 0..country_count {
            let tag_db_id = reader.u32()?;
            let group_count = reader.bounded_count(MAX_GROUPS_PER_COUNTRY, "group")?;
            let mut groups = Vec::with_capacity(group_count);
            for _ in 0..group_count {
                let army = reader.id()?;
                let request_count = reader.bounded_count(MAX_REQUEST_ROWS_PER_GROUP, "request")?;
                let delivery_count = reader.bounded_count(MAX_DELIVERIES_PER_GROUP, "delivery")?;
                leaves = leaves
                    .checked_add(request_count)
                    .and_then(|value| value.checked_add(delivery_count))
                    .ok_or_else(|| CarrierError::Invalid("AUR1 leaf count overflow".into()))?;
                if leaves > MAX_TOTAL_LEAF_ROWS {
                    return invalid("AUR1 total leaf count exceeds bound");
                }
                let mut requests = Vec::with_capacity(request_count);
                for _ in 0..request_count {
                    requests.push(ArmyUpgradeRequestRow {
                        equipment_variant: reader.id()?,
                        request_raw: reader.i64()?,
                        total_raw: reader.i64()?,
                    });
                }
                let mut deliveries = Vec::with_capacity(delivery_count);
                for _ in 0..delivery_count {
                    let status = reader.u32()?;
                    let has_progress = match reader.u32()? {
                        0 => false,
                        1 => true,
                        _ => return invalid("AUR1 delivery progress marker is invalid"),
                    };
                    let progress_value = reader.i64()?;
                    if !has_progress && progress_value != 0 {
                        return invalid("AUR1 absent delivery progress has nonzero payload");
                    }
                    let progress_raw = has_progress.then_some(progress_value);
                    let total_progress_raw = reader.i64()?;
                    let allow_zero_entries = reader.bool()?;
                    if reader.bytes(3)? != [0, 0, 0] {
                        return invalid("AUR1 delivery reserved bytes are nonzero");
                    }
                    let produced_count =
                        reader.bounded_count(MAX_PRODUCED_ROWS_PER_DELIVERY, "produced")?;
                    leaves = leaves
                        .checked_add(produced_count)
                        .ok_or_else(|| CarrierError::Invalid("AUR1 leaf count overflow".into()))?;
                    if leaves > MAX_TOTAL_LEAF_ROWS {
                        return invalid("AUR1 total leaf count exceeds bound");
                    }
                    let mut produced = Vec::with_capacity(produced_count);
                    for _ in 0..produced_count {
                        produced.push(ProducedEquipmentRow {
                            equipment_variant: reader.id()?,
                            amount_raw: reader.i64()?,
                        });
                    }
                    deliveries.push(UpgradeDeliveryRow {
                        status,
                        progress_raw,
                        total_progress_raw,
                        allow_zero_entries,
                        produced,
                    });
                }
                groups.push(ArmyUpgradeGroup {
                    army,
                    requests,
                    deliveries,
                });
            }
            countries.push(CountryArmyUpgradeRow { tag_db_id, groups });
        }
        Some(ArmyUpgradeSnapshot {
            source_sequence,
            countries,
        })
    } else {
        None
    };
    if reader.at != payload.len() {
        return Err(CarrierError::TrailingBytes);
    }
    let frame = ArmyUpgradeFrame {
        elapsed_wall_ms,
        frame_hour,
        status_flags,
        authority,
        snapshot,
    };
    validate_frame(&frame)?;
    Ok(frame)
}

fn add(left: u64, right: u64) -> Result<u64, CarrierError> {
    left.checked_add(right)
        .ok_or_else(|| CarrierError::Invalid("AUR1 address overflow".into()))
}

fn read_bytes<const N: usize, M: ArmyUpgradeMemory>(
    memory: &M,
    address: u64,
) -> Result<[u8; N], CarrierError> {
    let mut out = [0; N];
    memory
        .read_exact(address, &mut out)
        .map_err(|_| CarrierError::Memory { address, width: N })?;
    Ok(out)
}

fn read_u8<M: ArmyUpgradeMemory>(memory: &M, address: u64) -> Result<u8, CarrierError> {
    Ok(read_bytes::<1, _>(memory, address)?[0])
}
fn read_u32<M: ArmyUpgradeMemory>(memory: &M, address: u64) -> Result<u32, CarrierError> {
    Ok(u32::from_le_bytes(read_bytes(memory, address)?))
}
fn read_i32<M: ArmyUpgradeMemory>(memory: &M, address: u64) -> Result<i32, CarrierError> {
    Ok(i32::from_le_bytes(read_bytes(memory, address)?))
}
fn read_u64<M: ArmyUpgradeMemory>(memory: &M, address: u64) -> Result<u64, CarrierError> {
    Ok(u64::from_le_bytes(read_bytes(memory, address)?))
}
fn read_i64<M: ArmyUpgradeMemory>(memory: &M, address: u64) -> Result<i64, CarrierError> {
    Ok(i64::from_le_bytes(read_bytes(memory, address)?))
}

fn invalid<T>(message: impl Into<String>) -> Result<T, CarrierError> {
    Err(CarrierError::Invalid(message.into()))
}
fn u32v(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn u64v(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn i64v(out: &mut Vec<u8>, value: i64) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn idv(out: &mut Vec<u8>, value: PersistentId) {
    u32v(out, value.kind);
    u32v(out, value.id);
}

struct Reader<'a> {
    payload: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, len: usize) -> Result<&'a [u8], CarrierError> {
        let end = self.at.checked_add(len).ok_or(CarrierError::Truncated)?;
        let value = self
            .payload
            .get(self.at..end)
            .ok_or(CarrierError::Truncated)?;
        self.at = end;
        Ok(value)
    }
    fn bool(&mut self) -> Result<bool, CarrierError> {
        match self.bytes(1)?[0] {
            0 => Ok(false),
            1 => Ok(true),
            _ => invalid("AUR1 bool byte is invalid"),
        }
    }
    fn u32(&mut self) -> Result<u32, CarrierError> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, CarrierError> {
        Ok(u64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64, CarrierError> {
        Ok(i64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
    }
    fn id(&mut self) -> Result<PersistentId, CarrierError> {
        Ok(PersistentId {
            kind: self.u32()?,
            id: self.u32()?,
        })
    }
    fn bounded_count(&mut self, maximum: usize, label: &str) -> Result<usize, CarrierError> {
        let value = self.u32()? as usize;
        if value > maximum {
            return invalid(format!("AUR1 {label} count exceeds bound"));
        }
        Ok(value)
    }

    fn remaining(&self) -> usize {
        self.payload.len().saturating_sub(self.at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MissingMemory;

    impl ArmyUpgradeMemory for MissingMemory {
        fn read_exact(&self, _address: u64, _out: &mut [u8]) -> Result<(), ()> {
            Err(())
        }
    }

    #[test]
    fn country_walk_failure_retains_country_address_and_width() {
        let error = read_snapshot_from_country_roots(
            &MissingMemory,
            0x1400_0000_0,
            1,
            &[CountryRoot {
                tag_db_id: 41,
                object_ptr: 0x2000_0000,
            }],
        )
        .unwrap_err();
        assert!(matches!(
            error,
            CarrierError::CountryRead {
                tag_db_id: 41,
                object_ptr: 0x2000_0000,
                source,
            } if matches!(
                *source,
                CarrierError::Memory {
                    address: 0x2000_0f80,
                    width: 8,
                }
            )
        ));
    }

    fn frame() -> ArmyUpgradeFrame {
        ArmyUpgradeFrame {
            elapsed_wall_ms: 7,
            frame_hour: 99,
            status_flags: 0,
            authority: AuthorityState::ExactA729Receipt,
            snapshot: Some(ArmyUpgradeSnapshot {
                source_sequence: 3,
                countries: vec![CountryArmyUpgradeRow {
                    tag_db_id: 67,
                    groups: vec![ArmyUpgradeGroup {
                        army: PersistentId {
                            kind: 4713,
                            id: 14646,
                        },
                        requests: vec![ArmyUpgradeRequestRow {
                            equipment_variant: PersistentId { kind: 70, id: 5149 },
                            request_raw: 700_000,
                            total_raw: 800_000,
                        }],
                        deliveries: vec![UpgradeDeliveryRow {
                            status: 1,
                            progress_raw: Some(25_000),
                            total_progress_raw: 100_000,
                            allow_zero_entries: false,
                            produced: vec![ProducedEquipmentRow {
                                equipment_variant: PersistentId { kind: 70, id: 5149 },
                                amount_raw: 3,
                            }],
                        }],
                    }],
                }],
            }),
        }
    }

    #[test]
    fn exact_raw_request_delivery_roundtrip() {
        let expected = frame();
        let encoded = encode_payload(&expected).unwrap();
        assert_eq!(decode_payload(&encoded).unwrap(), expected);
    }

    #[test]
    fn unavailable_cannot_masquerade_as_measured_empty() {
        let mut invalid_frame = frame();
        invalid_frame.authority = AuthorityState::SourceUnavailable;
        invalid_frame.status_flags = 1;
        assert!(encode_payload(&invalid_frame).is_err());
    }

    const FIXTURE_BASE: u64 = 0x1_4000_0000;
    const FIXTURE_COUNTRY: u64 = 0x2000_0000;
    const FIXTURE_STATUS: u64 = 0x2001_0000;
    const FIXTURE_GROUP_SLOTS: u64 = 0x2002_0000;
    const FIXTURE_GROUP: u64 = 0x2003_0000;
    const FIXTURE_ARMY: u64 = 0x2004_0000;

    #[derive(Default)]
    struct SparseMemory {
        bytes: std::collections::BTreeMap<u64, u8>,
    }

    impl SparseMemory {
        fn put(&mut self, address: u64, value: &[u8]) {
            for (index, byte) in value.iter().enumerate() {
                self.bytes.insert(address + index as u64, *byte);
            }
        }

        fn u32(&mut self, address: u64, value: u32) {
            self.put(address, &value.to_le_bytes());
        }

        fn u64(&mut self, address: u64, value: u64) {
            self.put(address, &value.to_le_bytes());
        }
    }

    impl ArmyUpgradeMemory for SparseMemory {
        fn read_exact(&self, address: u64, out: &mut [u8]) -> Result<(), ()> {
            for (index, slot) in out.iter_mut().enumerate() {
                *slot = *self.bytes.get(&(address + index as u64)).ok_or(())?;
            }
            Ok(())
        }
    }

    fn single_group_memory(army_kind: u32, army_id: u32) -> SparseMemory {
        let mut memory = SparseMemory::default();
        memory.u64(
            FIXTURE_COUNTRY + COUNTRY_UPGRADE_STATUS_PTR_OFF,
            FIXTURE_STATUS,
        );

        memory.u64(FIXTURE_STATUS, FIXTURE_BASE + STATUS_VTABLE_RVA);
        memory.u64(
            FIXTURE_STATUS + STATUS_PERSISTENT_VTABLE_OFF,
            FIXTURE_BASE + STATUS_PERSISTENT_VTABLE_RVA,
        );
        memory.u64(FIXTURE_STATUS + STATUS_GROUP_BEGIN_OFF, FIXTURE_GROUP_SLOTS);
        memory.u32(FIXTURE_STATUS + STATUS_GROUP_CAPACITY_OFF, 1);
        memory.u32(FIXTURE_STATUS + STATUS_GROUP_COUNT_OFF, 1);
        memory.u64(FIXTURE_GROUP_SLOTS, FIXTURE_GROUP);

        memory.u64(FIXTURE_GROUP, FIXTURE_BASE + REQUESTS_VTABLE_RVA);
        memory.u64(
            FIXTURE_GROUP + REQUESTS_PERSISTENT_VTABLE_OFF,
            FIXTURE_BASE + REQUESTS_PERSISTENT_VTABLE_RVA,
        );
        memory.u32(FIXTURE_GROUP + REQUESTS_VERSION_OFF, REQUESTS_VERSION);
        memory.u64(FIXTURE_GROUP + REQUESTS_ARMY_PTR_OFF, FIXTURE_ARMY);
        memory.u64(FIXTURE_GROUP + REQUESTS_STATUS_PTR_OFF, FIXTURE_STATUS);
        memory.u64(FIXTURE_GROUP + REQUEST_ROW_BEGIN_OFF, 0);
        memory.u32(FIXTURE_GROUP + REQUEST_ROW_CAPACITY_OFF, 0);
        memory.u32(FIXTURE_GROUP + REQUEST_ROW_COUNT_OFF, 0);
        memory.u64(FIXTURE_GROUP + DELIVERY_BEGIN_OFF, 0);
        memory.u32(FIXTURE_GROUP + DELIVERY_CAPACITY_OFF, 0);
        memory.u32(FIXTURE_GROUP + DELIVERY_COUNT_OFF, 0);

        memory.u64(FIXTURE_ARMY, FIXTURE_BASE + ARMY_VTABLE_RVA);
        memory.u64(
            FIXTURE_ARMY + ARMY_PERSISTENT_VTABLE_OFF,
            FIXTURE_BASE + ARMY_PERSISTENT_VTABLE_RVA,
        );
        memory.u32(FIXTURE_ARMY + ARMY_ID_OFF, army_kind);
        memory.u32(FIXTURE_ARMY + ARMY_ID_OFF + 4, army_id);
        memory
    }

    fn read_single_group(army_kind: u32) -> Result<ArmyUpgradeSnapshot, CarrierError> {
        read_snapshot_from_country_roots(
            &single_group_memory(army_kind, 14_646),
            FIXTURE_BASE,
            1,
            &[CountryRoot {
                tag_db_id: 1,
                object_ptr: FIXTURE_COUNTRY,
            }],
        )
    }

    #[test]
    fn native_read_admits_the_wrapper_observed_alt_unit_handle_type() {
        for kind in [ARMY_ID_TYPE, ARMY_ID_ALT_TYPE] {
            let snapshot = read_single_group(kind)
                .unwrap_or_else(|error| panic!("handle type {kind} refused: {error:?}"));
            assert_eq!(snapshot.countries.len(), 1);
            assert_eq!(snapshot.countries[0].groups.len(), 1);
            assert_eq!(snapshot.countries[0].groups[0].army.kind, kind);
        }
    }

    #[test]
    fn native_read_still_refuses_a_handle_type_outside_the_unit_body_set() {
        for kind in [0, 1, EQUIPMENT_VARIANT_ID_TYPE, 52, 4_712] {
            let error =
                read_single_group(kind).expect_err("a non-unit-body handle type must stay refused");
            assert!(
                matches!(&error, CarrierError::CountryRead { source, .. }
                    if matches!(&**source, CarrierError::Invalid(reason)
                        if reason == "AUR1 CArmy persistent identity is invalid or duplicated")),
                "handle type {kind} produced {error:?}"
            );
        }
    }

    #[test]
    fn native_read_still_refuses_a_zero_army_id() {
        let error = read_snapshot_from_country_roots(
            &single_group_memory(ARMY_ID_ALT_TYPE, 0),
            FIXTURE_BASE,
            1,
            &[CountryRoot {
                tag_db_id: 1,
                object_ptr: FIXTURE_COUNTRY,
            }],
        )
        .expect_err("a zero CArmy id must stay refused");
        assert!(matches!(&error, CarrierError::CountryRead { source, .. }
            if matches!(&**source, CarrierError::Invalid(reason)
                if reason == "AUR1 CArmy persistent identity is invalid or duplicated")));
    }

    #[test]
    fn snapshot_validation_matches_the_native_handle_type_set() {
        let mut snapshot = frame().snapshot.unwrap();
        snapshot.countries[0].groups[0].army.kind = ARMY_ID_ALT_TYPE;
        validate_snapshot(&snapshot).expect("alt unit handle type must validate");
        snapshot.countries[0].groups[0].army.kind = EQUIPMENT_VARIANT_ID_TYPE;
        assert!(validate_snapshot(&snapshot).is_err());
    }

    /// Serves `inner` verbatim except that reads covering
    /// `watch_address + 4` (the CArmy id u32) return values from `watch_ids`:
    /// successively while any remain (clamped to the final value), or cycling
    /// forever when `cycle` is set. Each individual pass therefore sees a
    /// structurally valid graph, and the schedule decides whether the graph
    /// ever stops changing between passes.
    struct MutatingMemory {
        inner: SparseMemory,
        watch_address: u64,
        watch_ids: Vec<u32>,
        cycle: bool,
        watch_reads: std::cell::Cell<usize>,
    }

    impl ArmyUpgradeMemory for MutatingMemory {
        fn read_exact(&self, address: u64, out: &mut [u8]) -> Result<(), ()> {
            if out.len() == 4 && address == self.watch_address + 4 {
                let index = self.watch_reads.get();
                let id = if self.cycle {
                    self.watch_ids[index % self.watch_ids.len()]
                } else {
                    self.watch_ids[index.min(self.watch_ids.len() - 1)]
                };
                self.watch_reads.set(index + 1);
                out.copy_from_slice(&id.to_le_bytes());
                return Ok(());
            }
            self.inner.read_exact(address, out)
        }
    }

    fn mutating_memory(ids: Vec<u32>, cycle: bool) -> MutatingMemory {
        MutatingMemory {
            inner: single_group_memory(ARMY_ID_TYPE, ids[0]),
            watch_address: FIXTURE_ARMY + ARMY_ID_OFF,
            watch_ids: ids,
            cycle,
            watch_reads: std::cell::Cell::new(0),
        }
    }

    fn read_mutating(memory: &MutatingMemory) -> Result<ArmyUpgradeSnapshot, CarrierError> {
        read_snapshot_from_country_roots(
            memory,
            FIXTURE_BASE,
            1,
            &[CountryRoot {
                tag_db_id: 1,
                object_ptr: FIXTURE_COUNTRY,
            }],
        )
    }

    // kills: the retired fixed two-pass check ("AUR1 object graph changed
    // across the coherent double-read"), which refused a frame whenever one
    // pass pair straddled an engine mutation even though the same bracket's
    // later passes agree -- the d93 BLOCKED cause on pw_gate_final (18 frames,
    // all d93_fail_reason_double_read_changed). The restartable contract must
    // settle such a frame as measured data.
    #[test]
    fn coherent_read_restarts_until_the_graph_settles() {
        // Pass 1 sees id 14646; the mutation lands; every later pass sees 14647.
        let memory = mutating_memory(vec![14_646, 14_647], false);
        let snapshot = read_mutating(&memory)
            .unwrap_or_else(|error| panic!("a settling graph must still seal: {error:?}"));
        assert_eq!(snapshot.countries[0].groups[0].army.id, 14_647);
        assert_eq!(memory.watch_reads.get(), 3);
    }

    // kills: any unbounded retry, and any collapse of a persistently torn
    // graph into measured data: a graph that changes on every pass must
    // exhaust exactly COHERENT_READ_MAX_PASSES passes and refuse under its own
    // restart-exhausted name. The elapsed-time bound kills retries that WAIT:
    // the eight fixture passes are memory-only and complete in microseconds,
    // so any per-pass sleep/yield of meaningful size blows the ceiling. That
    // no production path sleeps is additionally established by inspection --
    // the loop body is only `read_snapshot_once` calls.
    #[test]
    fn coherent_read_refuses_by_name_when_the_graph_never_settles() {
        let memory = mutating_memory(vec![14_646, 14_647], true);
        let started = std::time::Instant::now();
        let error = read_mutating(&memory)
            .err()
            .expect("a graph that changes on every pass must stay refused");
        let elapsed = started.elapsed();
        assert!(
            matches!(&error, CarrierError::Invalid(reason)
                if reason == "AUR1 object graph kept changing across every coherent-read restart"),
            "never-settling graph produced {error:?}"
        );
        assert_eq!(memory.watch_reads.get(), COHERENT_READ_MAX_PASSES);
        assert!(
            elapsed < std::time::Duration::from_millis(500),
            "the bounded refusal must not wait (took {elapsed:?})"
        );
    }
}
