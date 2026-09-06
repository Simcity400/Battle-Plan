use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const PROGRAM_SHA256: &str = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
const GAME_STATE_RVA: u64 = 0x3316e90;
const RUNTIME_DLC_MASK_RVA: u64 = 0x3316e78;
const REFERENCE_TABLES_RVA: u64 = 0x3438e60;
const DATE_OFFSET: u64 = 0x468;
pub const DATE_EPOCH: u32 = 43_799_999;
const MAX_READ_BYTES: usize = 64 * 1024 * 1024;
const MAX_ROWS: usize = 100_000;
const VARIANT_VTABLE: u64 = 0x293afb8;
const TEMPLATE_VTABLE: u64 = 0x2938f70;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    Unreadable,
    InvalidRoot,
    InvalidIdentity,
    InvalidString,
    InvalidVector,
    RegistryCountMismatch,
    Torn,
    HourLabelMismatch,
    BoundExceeded,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CatalogFailure {
    pub kind: FailureKind,
    pub field: String,
}
type Result<T> = std::result::Result<T, CatalogFailure>;
fn failure(kind: FailureKind, field: impl Into<String>) -> CatalogFailure {
    CatalogFailure {
        kind,
        field: field.into(),
    }
}

pub trait Memory {
    fn read(&self, address: u64, length: usize) -> std::result::Result<Vec<u8>, String>;
}
impl<F: Fn(u64, usize) -> std::result::Result<Vec<u8>, String>> Memory for F {
    fn read(&self, address: u64, length: usize) -> std::result::Result<Vec<u8>, String> {
        self(address, length)
    }
}

struct ReadSet<'a, M: Memory> {
    memory: &'a M,
    base: u64,
    reads: BTreeMap<(u64, usize), Vec<u8>>,
    bytes: usize,
}
impl<'a, M: Memory> ReadSet<'a, M> {
    fn new(memory: &'a M, base: u64) -> Self {
        Self {
            memory,
            base,
            reads: BTreeMap::new(),
            bytes: 0,
        }
    }
    fn bytes(&mut self, address: u64, length: usize) -> Result<Vec<u8>> {
        if length == 0 {
            return Ok(Vec::new());
        }
        if address == 0 || address.checked_add(length as u64).is_none() {
            return Err(failure(FailureKind::InvalidRoot, "read address"));
        }
        let key = (address, length);
        if !self.reads.contains_key(&key) && self.bytes.saturating_add(length) > MAX_READ_BYTES {
            return Err(failure(FailureKind::BoundExceeded, "census read set"));
        }
        let bytes = self
            .memory
            .read(address, length)
            .map_err(|_| failure(FailureKind::Unreadable, format!("read {length} bytes")))?;
        if bytes.len() != length {
            return Err(failure(FailureKind::Unreadable, "short read"));
        }
        if let Some(previous) = self.reads.get(&key) {
            if previous != &bytes {
                return Err(failure(FailureKind::Torn, "repeated field"));
            }
        } else {
            self.bytes += length;
            self.reads.insert(key, bytes.clone());
        }
        Ok(bytes)
    }
    fn u8(&mut self, address: u64) -> Result<u8> {
        Ok(self.bytes(address, 1)?[0])
    }
    fn u32(&mut self, address: u64) -> Result<u32> {
        Ok(u32::from_le_bytes(
            self.bytes(address, 4)?.try_into().unwrap(),
        ))
    }
    fn u64(&mut self, address: u64) -> Result<u64> {
        Ok(u64::from_le_bytes(
            self.bytes(address, 8)?.try_into().unwrap(),
        ))
    }
    fn pointer(&mut self, address: u64) -> Result<u64> {
        let value = self.u64(address)?;
        if !object_pointer(value) {
            return Err(failure(FailureKind::InvalidRoot, "object pointer"));
        }
        Ok(value)
    }
    fn string(&mut self, address: u64, limit: usize) -> Result<String> {
        let header = self.bytes(address, 32)?;
        let length = u64_at(&header, 16);
        let capacity = u64_at(&header, 24);
        if length > limit as u64
            || capacity < length
            || capacity > 1024 * 1024
            || (capacity < 16 && length > 15)
        {
            return Err(failure(FailureKind::InvalidString, "MSVC length/capacity"));
        }
        let bytes = if capacity < 16 {
            header[..length as usize].to_vec()
        } else {
            let data = u64_at(&header, 0);
            if !object_pointer(data) {
                return Err(failure(FailureKind::InvalidString, "MSVC data"));
            }
            self.bytes(data, length as usize)?
        };
        String::from_utf8(bytes).map_err(|_| failure(FailureKind::InvalidString, "UTF-8"))
    }
    fn vector(
        &mut self,
        owner: u64,
        data_offset: u64,
        count_offset: u64,
        stride: usize,
        limit: usize,
    ) -> Result<Vec<u8>> {
        let data = self.u64(owner + data_offset)?;
        let capacity = self.u32(owner + count_offset - 4)? as usize;
        let count = self.u32(owner + count_offset)? as usize;
        if count > capacity || count > limit {
            return Err(failure(FailureKind::InvalidVector, "count/capacity"));
        }
        if count == 0 {
            return Ok(Vec::new());
        }
        if !object_pointer(data) {
            return Err(failure(FailureKind::InvalidVector, "data pointer"));
        }
        self.bytes(
            data,
            count
                .checked_mul(stride)
                .ok_or_else(|| failure(FailureKind::BoundExceeded, "vector bytes"))?,
        )
    }
    fn seal(&self) -> Result<()> {
        for (&(address, length), expected) in &self.reads {
            let actual = self
                .memory
                .read(address, length)
                .map_err(|_| failure(FailureKind::Unreadable, "seal read"))?;
            if &actual != expected {
                return Err(failure(FailureKind::Torn, "roots/fields/membership seal"));
            }
        }
        Ok(())
    }
    fn registry(&mut self, kind: u32, vtable_rva: u64) -> Result<Vec<(u32, u64)>> {
        let registry = self.pointer(self.base + REFERENCE_TABLES_RVA + u64::from(kind) * 8)?;
        let header = self.bytes(registry + 8, 24)?;
        let data = u64_at(&header, 0);
        let count = u32_at(&header, 8) as usize;
        let mask = u32_at(&header, 12) as usize;
        let max_probe = header[16] as usize;
        if count > MAX_ROWS || mask >= (1 << 20) || !(mask + 1).is_power_of_two() || max_probe > 64
        {
            return Err(failure(
                FailureKind::BoundExceeded,
                "reference registry header",
            ));
        }
        if count == 0 {
            return Ok(Vec::new());
        }
        if !object_pointer(data) || max_probe == 0 {
            return Err(failure(
                FailureKind::InvalidRoot,
                "reference registry table",
            ));
        }
        // Rehash allocates the bucket count, the probe tail, and one FF sentinel.
        let slots = mask + 1 + max_probe;
        let table = self.bytes(data, (slots + 1) * 24)?;
        if table[slots * 24 + 4] != 255 {
            return Err(failure(
                FailureKind::InvalidRoot,
                "reference registry sentinel",
            ));
        }
        let mut ids = BTreeSet::new();
        let mut pointers = BTreeSet::new();
        let mut rows = Vec::new();
        for (index, slot) in table[..slots * 24].chunks_exact(24).enumerate() {
            let probe = slot[4] as usize;
            if probe == 0 {
                continue;
            }
            let id = u32_at(slot, 12);
            let pointer = u64_at(slot, 16);
            let mixed = ((u64::from(kind) << 32) | u64::from(id)).wrapping_mul(0x49631a45);
            let hash = mixed as u32 ^ (mixed >> 32) as u32;
            if probe > max_probe
                || u32_at(slot, 8) != kind
                || u32_at(slot, 0) != hash
                || index != (hash as usize & mask) + probe - 1
                || id == 0
                || !object_pointer(pointer)
                || !ids.insert(id)
                || !pointers.insert(pointer)
            {
                return Err(failure(
                    FailureKind::InvalidIdentity,
                    "reference registry slot",
                ));
            }
            if self.u64(pointer)? != self.base + vtable_rva
                || self.u32(pointer + 8)? != kind
                || self.u32(pointer + 12)? != id
                || self.u8(pointer + 16)? != 1
            {
                return Err(failure(FailureKind::InvalidIdentity, "registered object"));
            }
            rows.push((id, pointer));
        }
        if rows.len() != count {
            return Err(failure(
                FailureKind::RegistryCountMismatch,
                "reference registry population",
            ));
        }
        rows.sort_by_key(|row| row.1);
        Ok(rows)
    }
}
fn object_pointer(value: u64) -> bool {
    (0x10000000000..0x7ff000000000).contains(&value) && value % 8 == 0
}
fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}
fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn nullable_string(value: &str) -> Value {
    if value.is_empty() {
        Value::Null
    } else {
        Value::String(value.into())
    }
}

fn item_icon_override<M: Memory>(read: &mut ReadSet<'_, M>, pointer: u64) -> Result<Value> {
    let header = (|| {
        let active = read.u8(pointer + 0x450)?;
        let kind = read.u32(pointer + 0x454)?;
        if active > 1 || kind > 1 {
            return Err(failure(
                FailureKind::InvalidIdentity,
                format!("item icon active {active} kind {kind}"),
            ));
        }
        Ok((active != 0, kind))
    })();
    let (active, kind) = match header {
        Ok(header) => header,
        Err(error)
            if matches!(
                error.kind,
                FailureKind::Unreadable | FailureKind::InvalidIdentity
            ) =>
        {
            return Ok(json!({"status":"unavailable", "reason":error}));
        }
        Err(error) => return Err(error),
    };
    let token = match read.string(pointer + 0x458, 4096) {
        Ok(value) => json!({"status":"measured", "value":value, "utf8_hex":hex(value.as_bytes())}),
        Err(error)
            if matches!(
                error.kind,
                FailureKind::Unreadable | FailureKind::InvalidString
            ) =>
        {
            json!({"status":"unavailable", "reason":error})
        }
        Err(error) => return Err(error),
    };
    Ok(json!({"status":"measured", "active":active, "kind":kind, "token":token}))
}

fn serialized_variant_flag_2b9d<M: Memory>(read: &mut ReadSet<'_, M>, pointer: u64) -> Result<Value> {
    match read.u8(pointer + 0x41e) {
        Ok(raw_u8) => Ok(json!({"status":"measured", "raw_u8":raw_u8})),
        Err(error) if error.kind == FailureKind::Unreadable => {
            Ok(json!({"status":"unavailable", "reason":error}))
        }
        Err(error) => Err(error),
    }
}

const MAX_MISSION_KEY_PREFIX: u32 = 4096;

fn mission_count<M: Memory>(read: &mut ReadSet<'_, M>, address: u64, field: &str) -> Result<u32> {
    let count = read.u32(address)? as i32;
    if count < 0 {
        return Err(failure(FailureKind::InvalidVector, format!("{field}: negative count {count}")));
    }
    Ok(count as u32)
}

fn mission_key_prefix<M: Memory>(
    read: &mut ReadSet<'_, M>, data: u64, count: u32, stride: u64, target: u32,
) -> Result<(Vec<u32>, Option<u64>)> {
    if count > 0 && !object_pointer(data) {
        return Err(failure(FailureKind::InvalidVector, "mission key data pointer"));
    }
    let mut keys = Vec::new();
    for ordinal in 0..count {
        if ordinal == MAX_MISSION_KEY_PREFIX {
            return Err(failure(FailureKind::BoundExceeded, "mission key prefix"));
        }
        let address = data.checked_add(u64::from(ordinal) * stride)
            .ok_or_else(|| failure(FailureKind::InvalidVector, "mission key address overflow"))?;
        let key = read.u32(address)?;
        keys.push(key);
        if key == target { return Ok((keys, Some(address))); }
    }
    Ok((keys, None))
}

fn mission_override<M: Memory>(
    read: &mut ReadSet<'_, M>, pointer: u64, group_key: u32, stat_key: u32,
) -> Result<Value> {
    let observation: Result<Value> = (|| {
        let group_count = mission_count(read, pointer + 0x37c, "mission group count")?;
        let group_data = if group_count == 0 { 0 } else { read.u64(pointer + 0x370)? };
        let (group_key_prefix, group) = mission_key_prefix(read, group_data, group_count, 0x28, group_key)?;
        let Some(group) = group else {
            return Ok(json!({"status":"group_absent","group_count":group_count,"group_key_prefix":group_key_prefix}));
        };
        let stat_count = mission_count(read, group + 0x1c, "mission stat count")?;
        let stat_data = if stat_count == 0 { 0 } else { read.u64(group + 0x10)? };
        let (stat_key_prefix, stat) = mission_key_prefix(read, stat_data, stat_count, 0x10, stat_key)?;
        match stat {
            Some(stat) => Ok(json!({"status":"measured","raw_i64":read.u64(stat + 8)? as i64,
                "group_count":group_count,"group_key_prefix":group_key_prefix,
                "stat_count":stat_count,"stat_key_prefix":stat_key_prefix})),
            None => Ok(json!({"status":"stat_absent","group_count":group_count,
                "group_key_prefix":group_key_prefix,"stat_count":stat_count,"stat_key_prefix":stat_key_prefix})),
        }
    })();
    match observation {
        Ok(value) => Ok(value),
        Err(mut error) if matches!(error.kind, FailureKind::Unreadable | FailureKind::InvalidVector) => {
            error.field = format!("mission {group_key}/{stat_key}: {}", error.field);
            Ok(json!({"status":"unavailable","reason":error}))
        }
        Err(error) => Err(error),
    }
}

fn mission_stat_operands<M: Memory>(read: &mut ReadSet<'_, M>, pointer: u64) -> Result<Value> {
    let mut rows = Vec::new();
    for (group_key, stat_key, offset) in [(0x1000, 0x23, 0x218), (0x2000, 0x24, 0x220)] {
        let fallback = match read.u64(pointer + offset) {
            Ok(raw) => json!({"status":"measured","raw_i64":raw as i64}),
            Err(mut error) if error.kind == FailureKind::Unreadable => {
                error.field = format!("mission fallback {stat_key}: {}", error.field);
                json!({"status":"unavailable","reason":error})
            }
            Err(error) => return Err(error),
        };
        rows.push(json!({"group_key":group_key,"stat_key":stat_key,"fallback":fallback,
            "override_lookup":mission_override(read, pointer, group_key, stat_key)?}));
    }
    Ok(Value::Array(rows))
}

fn variant_definition<M: Memory>(read: &mut ReadSet<'_, M>, pointer: u64) -> Result<Value> {
    let equipment_token = read.u32(pointer + 0x18)?;
    let definition = read.pointer(pointer + 0x3f0)?;
    if read.u64(definition)? != read.base + 0x29218a0
        || read.u32(definition + 8)? != equipment_token
    {
        return Err(failure(
            FailureKind::InvalidIdentity,
            "equipment definition",
        ));
    }
    let archetype = read.string(definition + 0x18, 160)?;
    if archetype.is_empty() {
        return Err(failure(FailureKind::InvalidString, "equipment archetype"));
    }
    let display = read.string(pointer + 0x28, 256)?;
    let name = read.string(pointer + 0x48, 256)?;
    let item_icon_override = item_icon_override(read, pointer)?;
    let serialized_variant_flag_2b9d = serialized_variant_flag_2b9d(read, pointer)?;
    let mission_stat_operands = mission_stat_operands(read, pointer)?;
    let explicit_entity = match read.string(pointer + 0x430, 4096) {
        Ok(value) => json!({"status":"measured", "value":value, "utf8_hex":hex(value.as_bytes())}),
        Err(error)
            if matches!(
                error.kind,
                FailureKind::Unreadable | FailureKind::InvalidString
            ) =>
        {
            json!({"status":"unavailable", "reason":error})
        }
        Err(error) => return Err(error),
    };
    if read.u64(pointer + 0xb0)? != read.base + 0x306d170
        || read.u64(pointer + 0x68)? != read.base + 0x27ab628
    {
        return Err(failure(
            FailureKind::InvalidIdentity,
            "variant module/upgrade container",
        ));
    }
    let module_bytes = read.vector(pointer, 0xb8, 0xc4, 16, 256)?;
    let mut modules = Vec::new();
    for (ordinal, row) in module_bytes.chunks_exact(16).enumerate() {
        let module = u64_at(row, 8);
        if !object_pointer(module) || read.u64(module)? != read.base + 0x2921850 {
            return Err(failure(FailureKind::InvalidIdentity, "module definition"));
        }
        let token = read.string(module + 0x18, 160)?;
        if token.is_empty() {
            return Err(failure(FailureKind::InvalidString, "module token"));
        }
        modules.push(json!({"source_ordinal":ordinal,"slot_handle":u32_at(row,0),"module_definition_handle":read.u32(module+8)?,"module_token":token,"module_token_utf8_hex":hex(token.as_bytes())}));
    }
    let upgrade_bytes = read.vector(pointer, 0x70, 0x7c, 16, 256)?;
    let upgrades: Vec<_> = upgrade_bytes.chunks_exact(16).enumerate().map(|(ordinal,row)| {
        let raw = u64_at(row,8);
        json!({"source_ordinal":ordinal,"upgrade_handle":u32_at(row,0),"level":raw&255,"level_raw_u64":raw})
    }).collect();
    let fields = [
        ("speed_cache", 0x270, 100_000),
        ("range", 0x278, 1_000),
        ("air_attack", 0x280, 100_000),
        ("agility", 0x288, 100_000),
        ("max_speed", 0x2f0, 100_000),
        ("reliability", 0x308, 100_000),
        ("air_defence", 0x318, 100_000),
        ("thrust", 0x320, 100_000),
        ("fuel_consumption", 0x328, 100_000),
        ("build_cost_ic", 0x368, 100_000),
    ];
    let mut stats = Vec::new();
    for (ordinal, (name, offset, scale)) in fields.iter().enumerate() {
        stats.push(json!({"source_ordinal":ordinal,"stat_id":name,"raw_i64":read.u64(pointer+offset)? as i64,"scale":scale}));
    }
    Ok(
        json!({"equipment_token_raw":equipment_token,"archetype_token":archetype,"archetype_token_utf8_hex":hex(archetype.as_bytes()),"display_name":nullable_string(&display),"display_name_utf8_hex":hex(display.as_bytes()),"variant_name":nullable_string(&name),"variant_name_utf8_hex":hex(name.as_bytes()),"explicit_entity":explicit_entity,"item_icon_override":item_icon_override,"serialized_variant_flag_2b9d":serialized_variant_flag_2b9d,"mission_stat_operands":mission_stat_operands,"modules_complete":true,"modules":modules,"upgrades_complete":true,"upgrades":upgrades,"cached_stats_complete":true,"cached_stats":stats}),
    )
}

fn subunits<M: Memory>(
    read: &mut ReadSet<'_, M>,
    pointer: u64,
    data_offset: u64,
    count_offset: u64,
    rows_per_column: u32,
) -> Result<(Vec<Value>, Vec<usize>, usize)> {
    let data = read.vector(pointer, data_offset, count_offset, 8, 64)?;
    let count = data.len() / 8;
    let mut rows = Vec::new();
    let mut placeholders = Vec::new();
    for (ordinal, raw) in data.chunks_exact(8).enumerate() {
        let definition = u64_at(raw, 0);
        if definition == 0 {
            placeholders.push(ordinal);
            continue;
        }
        if !object_pointer(definition) {
            return Err(failure(
                FailureKind::InvalidIdentity,
                "subunit definition address",
            ));
        }
        let vtable = read.u64(definition)?;
        let is_real = read.u8(definition + 0x10)?;
        if vtable
            == read.base + portable_engine_evidence::native_design_catalog::NULL_SUBUNIT_VTABLE_RVA
            && is_real == 0
        {
            placeholders.push(ordinal);
            continue;
        }
        if vtable != read.base + 0x2773ba8 || is_real != 1 {
            return Err(failure(
                FailureKind::InvalidIdentity,
                "subunit definition discriminator",
            ));
        }
        let token = read.string(definition + 0x18, 160)?;
        if token.is_empty() {
            return Err(failure(FailureKind::InvalidString, "subunit token"));
        }
        rows.push(json!({"source_ordinal":ordinal,"x":ordinal as u32 / rows_per_column,"y":ordinal as u32 % rows_per_column,"subunit_definition_handle":read.u32(definition+8)?,"type":token,"type_utf8_hex":hex(token.as_bytes())}));
    }
    Ok((rows, placeholders, count))
}
fn template_definition<M: Memory>(read: &mut ReadSet<'_, M>, pointer: u64) -> Result<Value> {
    let name = read.string(pointer + 0x20, 256)?;
    let dimensions = read.u32(pointer + 0x60)?;
    let columns = dimensions & 65535;
    let rows = dimensions >> 16;
    if !(1..=16).contains(&columns) || !(1..=16).contains(&rows) {
        return Err(failure(FailureKind::InvalidVector, "template dimensions"));
    }
    let (regiments, regiment_placeholders, regiment_count) =
        subunits(read, pointer, 0x68, 0x74, rows)?;
    let support_rows = read.u32(pointer + 0x94)?.max(1);
    let (support, support_placeholders, support_count) =
        subunits(read, pointer, 0x88, 0x94, support_rows)?;
    let (regimental_support, regimental_support_placeholders, regimental_support_count) =
        subunits(read, pointer, 0xa8, 0xb4, 1)?;
    Ok(
        json!({"name":nullable_string(&name),"name_utf8_hex":hex(name.as_bytes()),"regiment_grid_columns":columns,"regiment_grid_rows":rows,"regiment_pointer_count":regiment_count,"regiment_placeholder_ordinals":regiment_placeholders,"regiments":regiments,"support_pointer_count":support_count,"support_placeholder_ordinals":support_placeholders,"support":support,"regimental_support_pointer_count":regimental_support_count,"regimental_support_placeholder_ordinals":regimental_support_placeholders,"regimental_support":regimental_support,"priority":read.u32(pointer+0x1a4)?,"lock_regime_snapshot_raw":read.u8(pointer+0x1cc)?,"is_locked":read.u8(pointer+0x235)?!=0,"obsolete":read.u8(pointer+0x236)?&1!=0}),
    )
}

pub fn observe_date<M: Memory>(memory: &M, base: u64) -> Result<u32> {
    let mut read = ReadSet::new(memory, base);
    let state = read.pointer(base + GAME_STATE_RVA)?;
    read.u32(state + DATE_OFFSET)
}

fn campaign_pair<M: Memory>(read: &mut ReadSet<'_, M>, state: u64) -> Result<Value> {
    use compact_contract::build_profile as profile;
    let seed = read.u32(state + profile::DOMAIN_CAMPAIGN_ID_GAME_UNIQUE_SEED_OFF as u64)? as i32;
    let uuid = read.string(
        state + profile::DOMAIN_CAMPAIGN_ID_GAME_UNIQUE_ID_OFF as u64,
        compact_contract::campaign_identity::CID1_UUID_LEN,
    )?;
    if crate::campaign_identity::canonical_uuid(&uuid).as_deref() != Some(uuid.as_str()) {
        return Err(failure(FailureKind::InvalidIdentity, "campaign UUID"));
    }
    Ok(json!({"game_unique_id":uuid,"game_unique_seed":seed}))
}

pub fn observe_campaign<M: Memory>(memory: &M, base: u64) -> Result<(u32, Value)> {
    let mut read = ReadSet::new(memory, base);
    let state = read.pointer(base + GAME_STATE_RVA)?;
    let date = read.u32(state + DATE_OFFSET)?;
    let pair = campaign_pair(&mut read, state)?;
    read.seal()?;
    Ok((date, pair))
}

fn runtime_dlc_mask<M: Memory>(read: &mut ReadSet<'_, M>) -> Result<Value> {
    match read.u32(read.base + RUNTIME_DLC_MASK_RVA) {
        Ok(raw_u32) => Ok(json!({"status":"measured", "raw_u32":raw_u32})),
        Err(error) if error.kind == FailureKind::Unreadable => {
            Ok(json!({"status":"unavailable", "reason":error}))
        }
        Err(error) => Err(error),
    }
}

pub fn capture<M: Memory>(memory: &M, base: u64, expected_date: u32) -> Result<Value> {
    let hour = expected_date
        .checked_sub(DATE_EPOCH)
        .ok_or_else(|| failure(FailureKind::HourLabelMismatch, "portable epoch"))?;
    let mut read = ReadSet::new(memory, base);
    let state = read.pointer(base + GAME_STATE_RVA)?;
    if read.u32(state + DATE_OFFSET)? != expected_date {
        return Err(failure(FailureKind::HourLabelMismatch, "opening date"));
    }
    let campaign = campaign_pair(&mut read, state)?;
    let runtime_dlc_mask = runtime_dlc_mask(&mut read)?;
    let tag_db = read.pointer(base + 0x33189c8)?;
    let tag_data = read.pointer(tag_db)?;
    let tag_cap = read.u32(tag_db + 8)?;
    let tag_count = read.u32(tag_db + 12)?;
    let dynamic_boundary = read.u32(tag_db + 0x88)?;
    if !compact_contract::country_tag_db::tag_db_size_structurally_valid(tag_count, tag_cap)
        || dynamic_boundary > tag_count
        || tag_count as usize > MAX_ROWS
    {
        return Err(failure(FailureKind::InvalidVector, "country tag database"));
    }
    let mut tags = Vec::new();
    let mut seen_tags = BTreeSet::new();
    for index in 0..tag_count {
        let tag = read.string(tag_data + u64::from(index) * 32, 3)?;
        if tag.len() != 3 || tag.as_bytes().contains(&0) || !seen_tags.insert(tag.clone()) {
            return Err(failure(FailureKind::InvalidString, "country tag"));
        }
        tags.push(tag);
    }
    let country_bytes = read.vector(state, 0x310, 0x31c, 8, MAX_ROWS)?;
    if country_bytes.is_empty() {
        return Err(failure(
            FailureKind::InvalidRoot,
            "country registry is not initialized",
        ));
    }
    let mut countries = Vec::new();
    let mut country_pointers = BTreeSet::new();
    for (index, slot) in country_bytes.chunks_exact(8).enumerate() {
        let pointer = u64_at(slot, 0);
        if !object_pointer(pointer)
            || !country_pointers.insert(pointer)
            || read.u64(pointer)? != base + 0x27d0e90
        {
            return Err(failure(
                FailureKind::InvalidIdentity,
                "country registry member",
            ));
        }
        let tag = tags
            .get(read.u32(pointer + 8)? as usize)
            .ok_or_else(|| failure(FailureKind::InvalidIdentity, "country tag index"))?
            .clone();
        countries.push((index, tag, pointer));
    }
    let country_pointer_ordinals: BTreeMap<_, _> = country_pointers
        .into_iter()
        .enumerate()
        .map(|(ordinal, pointer)| (pointer, ordinal))
        .collect();
    let variants = read.registry(70, VARIANT_VTABLE)?;
    let templates = read.registry(52, TEMPLATE_VTABLE)?;
    let variants_by_pointer: BTreeMap<_, _> = variants
        .iter()
        .map(|&(id, pointer)| (pointer, id))
        .collect();
    let templates_by_pointer: BTreeMap<_, _> = templates
        .iter()
        .map(|&(id, pointer)| (pointer, id))
        .collect();
    let mut occurrences = BTreeMap::<u64, Vec<Value>>::new();
    let mut developed = BTreeMap::<u64, Vec<Value>>::new();
    let mut first_discovery = BTreeMap::<u64, usize>::new();
    let mut template_sources = BTreeMap::<u64, Vec<Value>>::new();
    for (index, tag, country) in &countries {
        let manager = read.pointer(country + 0xf68)?;
        for (list, data_offset, count_offset) in [
            ("A", 0xa0, 0xac),
            ("B", 0xe8, 0xf4),
            ("developed", 0xb8, 0xc4),
        ] {
            let vector = read.vector(manager, data_offset, count_offset, 8, MAX_ROWS)?;
            for (ordinal, slot) in vector.chunks_exact(8).enumerate() {
                let pointer = u64_at(slot, 0);
                if !variants_by_pointer.contains_key(&pointer) {
                    return Err(failure(
                        FailureKind::InvalidIdentity,
                        format!("{list} variant absent from registry"),
                    ));
                }
                let occurrence = json!({"source_country_tag":tag,"source_country_registry_index":index,"source_list":list,"source_list_ordinal":ordinal});
                if list == "developed" {
                    developed.entry(pointer).or_default().push(occurrence);
                } else {
                    let next = first_discovery.len();
                    first_discovery.entry(pointer).or_insert(next);
                    occurrences.entry(pointer).or_default().push(occurrence);
                }
            }
        }
        let vector = read.vector(*country, 0x1b8, 0x1c4, 8, MAX_ROWS)?;
        for (ordinal, slot) in vector.chunks_exact(8).enumerate() {
            let pointer = u64_at(slot, 0);
            if !templates_by_pointer.contains_key(&pointer) {
                return Err(failure(
                    FailureKind::InvalidIdentity,
                    "country template is absent from the reference registry",
                ));
            }
            template_sources.entry(pointer).or_default().push(json!({
                "source_country_tag":tag,"source_country_registry_index":index,
                "country_vector_ordinal":ordinal,
            }));
        }
    }
    let mut variant_rows = Vec::new();
    for (ordinal, (id, pointer)) in variants.iter().enumerate() {
        let creator = tags
            .get(read.u32(pointer + 0x1c)? as usize)
            .ok_or_else(|| failure(FailureKind::InvalidIdentity, "variant creator tag"))?;
        let origin_id = read.u32(pointer + 0x20)?;
        let origin = if origin_id == 0 {
            None
        } else {
            Some(
                tags.get(origin_id as usize)
                    .ok_or_else(|| failure(FailureKind::InvalidIdentity, "variant origin tag"))?,
            )
        };
        let definition = variant_definition(&mut read, *pointer)?;
        variant_rows.push(json!({"id":id,"type":70,"owner_tag":creator,"creator_tag":creator,"origin_override_tag":origin,"selected_country_tag":origin.unwrap_or(creator),"obsolete":read.u8(pointer+0x424)?!=0,"definition_complete":true,"catalog_scan_ordinal":ordinal,"first_ui_discovery_ordinal":first_discovery.get(pointer),"source_occurrences":occurrences.remove(pointer).unwrap_or_default(),"developed_source_occurrences":developed.remove(pointer).unwrap_or_default(),"definition":definition}));
    }
    let mut template_rows = Vec::new();
    for (ordinal, (id, pointer)) in templates.iter().enumerate() {
        let sources = template_sources.remove(pointer).unwrap_or_default();
        let sole_source = if sources.len() == 1 {
            sources.first()
        } else {
            None
        };
        let mut row = json!({
            "owner_tag":sole_source.map(|source|&source["source_country_tag"]),
            "source_country_tag":sole_source.map(|source|&source["source_country_tag"]),
            "source_country_registry_index":sole_source.map(|source|&source["source_country_registry_index"]),
            "country_vector_ordinal":sole_source.map(|source|&source["country_vector_ordinal"]),
            "country_source_occurrences":sources,
        });
        let definition = template_definition(&mut read, *pointer)?;
        let object = row.as_object_mut().unwrap();
        object.insert("id".into(), json!(id));
        object.insert("type".into(), json!(52));
        object.insert("obsolete".into(), definition["obsolete"].clone());
        object.insert("definition_complete".into(), json!(true));
        object.insert("catalog_scan_ordinal".into(), json!(ordinal));
        object.insert("definition".into(), definition);
        template_rows.push(row);
    }
    read.seal()?;
    if observe_date(memory, base)? != expected_date {
        return Err(failure(FailureKind::HourLabelMismatch, "closing date"));
    }
    let value = json!({"schema":snapshot::design_catalog::A27_LIVE_CATALOG_SCHEMA,"runtime_dlc_mask":runtime_dlc_mask,"authority":{"program":"hoi4.exe 1.19.2.0.a729","program_sha256":PROGRAM_SHA256,"campaign":campaign,"current_date_raw":expected_date,"capture_hour_index":hour,"coherence":"double_read_all_roots_fields_and_membership"},"full_replacement":true,"completeness":{"membership":true,"owner":template_rows.iter().all(|row|row["owner_tag"].is_string()),"definitions":true,"lifecycle":true,"ui_occurrences":true},"catalog_roots":{"variants":"CReferenceObject/type70","division_templates":"CReferenceObject/type52","template_owner_order":"CCountry+0x1B8/+0x1C4","equipment_ui_occurrences":"CCountry+0xF68 manager A(+0xA0/+0xAC) then B(+0xE8/+0xF4)","equipment_developed_occurrences":"CCountry+0xF68 manager developed(+0xB8/+0xC4)"},"country_count":countries.len(),"countries":countries.iter().map(|(index,tag,pointer)|json!({"registry_index":index,"tag":tag,"designer_pointer_order_ordinal":country_pointer_ordinals[pointer]})).collect::<Vec<_>>(),"variants":variant_rows,"division_templates":template_rows,"ui_occurrence_counts":{"unique_variants":first_discovery.len(),"catalog_variants_without_occurrence":variants.len()-first_discovery.len()}});
    let bytes = serde_json::to_vec(&value)
        .map_err(|_| failure(FailureKind::InvalidIdentity, "catalog serialization"))?;
    snapshot::design_catalog::PortableDesignCatalog::decode_a27_live_json(&bytes).map_err(
        |error| {
            failure(
                FailureKind::InvalidIdentity,
                format!("portable catalog admission: {error}"),
            )
        },
    )?;
    Ok(value)
}
pub mod custody;
#[cfg(test)]
mod tests;
pub mod timeline;
