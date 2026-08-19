use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::{
    VirtualQuery, MEMORY_BASIC_INFORMATION, MEM_COMMIT, MEM_PRIVATE, PAGE_GUARD, PAGE_NOACCESS,
};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

#[cfg(test)]
use std::{cell::RefCell, collections::BTreeMap};

#[cfg(test)]
struct TestMemory {
    bytes: BTreeMap<usize, u8>,
    mutation: Option<TestMutation>,
}

#[cfg(test)]
struct TestMutation {
    trigger_address: usize,
    remaining_reads: usize,
    write_address: usize,
    bytes: Vec<u8>,
}

#[cfg(test)]
impl TestMemory {
    fn read(&mut self, address: usize, len: usize) -> Option<Vec<u8>> {
        let result = (0..len)
            .map(|offset| self.bytes.get(&address.checked_add(offset)?).copied())
            .collect::<Option<Vec<_>>>()?;
        if let Some(mutation) = self.mutation.as_mut() {
            if mutation.trigger_address == address {
                mutation.remaining_reads = mutation.remaining_reads.saturating_sub(1);
                if mutation.remaining_reads == 0 {
                    for (offset, byte) in mutation.bytes.iter().copied().enumerate() {
                        self.bytes
                            .insert(mutation.write_address.checked_add(offset)?, byte);
                    }
                    self.mutation = None;
                }
            }
        }
        Some(result)
    }
}

#[cfg(test)]
thread_local! {
    static TEST_MEMORY: RefCell<Option<TestMemory>> = const { RefCell::new(None) };
}

pub const ALERT_PRODUCER_RVA: usize = 0x00af_e8a0;
pub const ARCHETYPE_STABLE_ID_OFF: usize = 0x418;
pub const FIXED_POINT_DIVISOR: i64 = 100_000;

const GAME_STATE_ROOT_RVA: usize = 0x0331_6e90;
const COUNTRY_VTABLE_RVA: usize = 0x027d_0e90;
const OCCUPATION_STATUS_VTABLE_RVA: usize = 0x0296_dd50;
const OCCUPATION_STATUS_SECONDARY_VTABLE_RVA: usize = 0x0296_dd88;
const OCCUPATION_DATA_VTABLE_RVA: usize = 0x0296_dd00;
const GARRISON_VTABLE_RVA: usize = 0x0296_dcb0;

const GAME_STATE_HUMAN: usize = 0xa8;
const GAME_STATE_COUNTRY_TABLE: usize = 0x310;
const GAME_STATE_COUNTRY_COUNT: usize = 0x31c;
const GAME_STATE_TAG_TO_COUNTRY_INDEX: usize = 0x340;
const GAME_STATE_COUNTRY_RUNTIME_OWNER: usize = 0x3d8;
const HUMAN_PLAYER_TAG: usize = 0x70;

const COUNTRY_TAG_DB_ID: usize = 0x08;
const COUNTRY_MANPOWER_TAG_HANDLE: usize = 0x330;
const COUNTRY_MANPOWER_DIRECT: usize = 0x334;
const COUNTRY_MANPOWER_OBJECTS: usize = 0x460;
const COUNTRY_MANPOWER_OBJECT_COUNT: usize = 0x46c;
const COUNTRY_STOCKPILE: usize = 0xf98;
const COUNTRY_OCCUPATION_STATUS: usize = 0xfd0;

const STATUS_SECONDARY_VTABLE: usize = 0x18;
const STATUS_OCCUPATION_DATA_ROWS: usize = 0x60;
const STATUS_OCCUPATION_DATA_COUNT: usize = 0x6c;
const STATUS_ASSIGNED_MANPOWER: usize = 0x90;

const OCCUPATION_DATA_PARENT_STATUS: usize = 0x08;
const GARRISON_TABLE_DATA: usize = 0xd8;
const GARRISON_TABLE_COUNT: usize = 0xe0;
const GARRISON_TABLE_MASK: usize = 0xe4;
const GARRISON_TABLE_MAX_DISTANCE: usize = 0xe8;
const ROBIN_HOOD_ROW_LEN: usize = 0x18;
const ROBIN_HOOD_ROW_DISTANCE: usize = 0x04;
const ROBIN_HOOD_ROW_KEY: usize = 0x08;
const ROBIN_HOOD_ROW_VALUE: usize = 0x10;

const GARRISON_CHILDREN: usize = 0x30;
const GARRISON_CHILD_COUNT: usize = 0x3c;
const GARRISON_OCCUPATION_DATA: usize = 0x58;
const GARRISON_ASSIGNED_MANPOWER: usize = 0x68;
const GARRISON_LOCAL_EQUIPMENT: usize = 0x90;
const GARRISON_REQUIRED_MANPOWER: usize = 0x88;
const GARRISON_REQUIRED_EQUIPMENT: usize = 0xd0;
const GARRISON_WINDOW_LEN: usize = 0xe9;
const CHILD_EQUIPMENT_POOL: usize = 0x38;

const EQUIPMENT_POOL_DATA: usize = 0x08;
const EQUIPMENT_POOL_CAPACITY: usize = 0x10;
const EQUIPMENT_POOL_COUNT: usize = 0x14;
const REQUIRED_EQUIPMENT_ROW_LEN: usize = 0x10;
const ALLOCATION_EQUIPMENT_ROW_LEN: usize = 0x18;
const ALLOCATION_EQUIPMENT_AMOUNT: usize = 0x10;
const EQUIPMENT_TYPE_ARCHETYPE: usize = 0x4d8;
const EQUIPMENT_ARCHETYPE_ACTIVE: usize = 0x10;
const EQUIPMENT_TYPE_STOCKPILE_INDEX: usize = 0x538;
const EQUIPMENT_TYPE_PRIMARY_FLAGS: usize = 0x540;
const EQUIPMENT_TYPE_SECONDARY_FLAGS: usize = 0x548;
const EQUIPMENT_TYPE_SPECIAL_COUNTRY: usize = 0x3eb;
const EQUIPMENT_ARCHETYPE_CATEGORY_FLAGS: usize = 0x540;
const EQUIPMENT_ARCHETYPE_POOL_FLAGS: usize = 0x408;

const STOCKPILE_CATEGORY_ARRAY: usize = 0x08;
const STOCKPILE_CONTEXT: usize = 0x20;
const STOCKPILE_DEFAULT_CATEGORY: usize = 0x0f;
const STOCKPILE_CATEGORY_LOOKUP_OWNER: usize = 0x10;
const STOCKPILE_CATEGORY_LOOKUP_ROOT: usize = 0x30;
const STOCKPILE_CONTEXT_SPECIAL_POOL: usize = 0x1210;
const STOCKPILE_CONTEXT_SPECIAL_USED: usize = 0x126c;
const STOCKPILE_CONTEXT_CACHE_VALID: usize = 0x1278;
const STOCKPILE_CONTEXT_CACHE_RAW: usize = 0x1280;
const SPECIAL_POOL_DATA: usize = 0x20;
const SPECIAL_POOL_COUNT: usize = 0x2c;

const COUNTRY_RUNTIME_OWNER_TABLE: usize = 0x108;
const COUNTRY_RUNTIME_ADJUSTMENT_CONTAINER: usize = 0x80;
const COUNTRY_RUNTIME_SUBTRACT_A: usize = 0xc8;
const COUNTRY_RUNTIME_ADD_A: usize = 0x138;
const COUNTRY_RUNTIME_SUBTRACT_B: usize = 0x140;
const MANPOWER_OBJECT_VALUE: usize = 0x848;

const EQUIPMENT_POOL_LIMIT: i64 = 0x53e2_d620_fb40;
#[cfg(test)]
const HEAP_LO: usize = 0x100_0000_0000;
#[cfg(test)]
const HEAP_HI: usize = 0x7ff0_0000_0000;

const ALERT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57,
];

static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static LIFECYCLE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlertEquipmentShortage {
    pub archetype_id: u32,
    pub whole_units: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlertObservation {
    pub game_hour_raw: i32,
    pub country_ptr: u64,
    pub predicate_returned: bool,
    pub missing_manpower: i64,
    pub missing_equipment: Vec<AlertEquipmentShortage>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AlertDrain {
    pub observations: Vec<AlertObservation>,
    pub poisoned_hours: Vec<i32>,
    pub publish_lock_dropped: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Roots {
    game_state: usize,
    country: usize,
    status: usize,
}

#[derive(Clone, Copy)]
struct RequiredEquipment {
    pointer: usize,
    archetype_id: u32,
    raw: i64,
}

type EquipmentPool = HashMap<u32, (usize, i64)>;

pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

pub fn install() -> Result<(), String> {
    let _lifecycle = LIFECYCLE_LOCK
        .lock()
        .map_err(|_| "a729 garrison-alert lifecycle lock poisoned".to_string())?;
    if installed() {
        return Ok(());
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("a729 garrison-alert reader has no main-module base".into());
    }
    let observed = read_stable_bytes(
        base.checked_add(ALERT_PRODUCER_RVA)
            .ok_or("a729 garrison-alert sentinel address overflow")?,
        ALERT_SENTINEL.len(),
    )
    .ok_or("a729 garrison-alert sentinel is unreadable or unstable")?;
    if observed.as_slice() != ALERT_SENTINEL {
        return Err("a729 garrison-alert sentinel mismatch".into());
    }
    MODULE_BASE.store(base, Ordering::Release);
    INSTALLED.store(true, Ordering::Release);
    Ok(())
}

pub fn take_for_hour(game_hour_raw: i32) -> Result<Option<AlertObservation>, String> {
    let _lifecycle = LIFECYCLE_LOCK
        .lock()
        .map_err(|_| "a729 garrison-alert lifecycle lock poisoned".to_string())?;
    if !installed() {
        return Err("a729 garrison-alert passive reader is not installed".into());
    }
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return Err("a729 garrison-alert passive reader lost module custody".into());
    }
    if capture_game_hour_raw(base) != Some(game_hour_raw) {
        return Ok(None);
    }
    let roots = resolve_active_country(base)?;
    let mut observation = reconstruct_alert(base, roots, game_hour_raw)?;
    observation
        .missing_equipment
        .sort_unstable_by_key(|row| row.archetype_id);
    if capture_game_hour_raw(base) != Some(game_hour_raw) {
        return Ok(None);
    }
    let confirmation_roots = resolve_active_country(base)?;
    if confirmation_roots != roots {
        return Err("a729 garrison-alert roots changed during capture".into());
    }
    let mut confirmation = reconstruct_alert(base, confirmation_roots, game_hour_raw)?;
    confirmation
        .missing_equipment
        .sort_unstable_by_key(|row| row.archetype_id);
    if confirmation != observation {
        return Err("a729 garrison-alert snapshot changed during capture".into());
    }
    if capture_game_hour_raw(base) != Some(game_hour_raw) {
        return Ok(None);
    }
    Ok(Some(observation))
}

pub fn seal_drain_and_uninstall() -> Result<AlertDrain, String> {
    let _lifecycle = LIFECYCLE_LOCK
        .lock()
        .map_err(|_| "a729 garrison-alert lifecycle lock poisoned".to_string())?;
    INSTALLED.store(false, Ordering::Release);
    MODULE_BASE.store(0, Ordering::Release);
    Ok(AlertDrain::default())
}

fn reconstruct_alert(
    base: usize,
    roots: Roots,
    game_hour_raw: i32,
) -> Result<AlertObservation, String> {
    let garrisons = read_garrisons(base, roots.status)?;
    let mut required_manpower = 0i32;
    let mut required_equipment: HashMap<u32, RequiredEquipment> = HashMap::new();

    for (garrison, occupation_data) in garrisons {
        let window = read_stable_bytes(garrison, GARRISON_WINDOW_LEN)
            .ok_or("a729 garrison row is unreadable or unstable")?;
        if blob_usize(&window, 0) != Some(base + GARRISON_VTABLE_RVA)
            || blob_usize(&window, GARRISON_OCCUPATION_DATA) != Some(occupation_data)
        {
            return Err("a729 garrison row identity mismatch".into());
        }
        let required = blob_i32(&window, GARRISON_REQUIRED_MANPOWER)
            .ok_or("a729 garrison required manpower is missing")?;
        let assigned = sum_i32_container(garrison + GARRISON_ASSIGNED_MANPOWER)?;
        required_manpower = required_manpower.wrapping_add(required.wrapping_sub(assigned));

        let required_rows = read_required_equipment_pool(garrison + GARRISON_REQUIRED_EQUIPMENT)?;
        let local_rows = read_allocation_equipment_pool(garrison + GARRISON_LOCAL_EQUIPMENT)?;
        let children = read_pointer_vector(
            blob_usize(&window, GARRISON_CHILDREN)
                .ok_or("a729 garrison child vector is missing")?,
            blob_i32(&window, GARRISON_CHILD_COUNT)
                .ok_or("a729 garrison child count is missing")?,
            "garrison children",
        )?;
        let mut child_pools = Vec::new();
        child_pools
            .try_reserve_exact(children.len())
            .map_err(|_| "a729 child equipment-pool result allocation failed".to_string())?;
        for child in children {
            require_heap(child, "garrison child")?;
            child_pools.push(read_allocation_equipment_pool(
                child
                    .checked_add(CHILD_EQUIPMENT_POOL)
                    .ok_or("a729 child equipment-pool address overflow")?,
            )?);
        }

        for (required_ptr, required_raw) in required_rows {
            let Some((archetype, archetype_id)) = canonical_equipment_archetype(required_ptr)?
            else {
                continue;
            };
            let mut raw = required_raw;
            raw = raw.wrapping_sub(pool_amount_by_id(&local_rows, archetype_id));
            for child_pool in &child_pools {
                raw = raw.wrapping_sub(pool_amount_by_id(child_pool, archetype_id));
            }
            if let Some(entry) = required_equipment.get_mut(&archetype_id) {
                if entry.pointer != archetype {
                    return Err("a729 equipment stable ID aliases multiple archetypes".into());
                }
                entry.raw = accumulate_pool(entry.raw, raw);
            } else {
                required_equipment
                    .try_reserve(1)
                    .map_err(|_| "a729 required-equipment map allocation failed".to_string())?;
                required_equipment.insert(
                    archetype_id,
                    RequiredEquipment {
                        pointer: archetype,
                        archetype_id,
                        raw: clamp_pool(raw),
                    },
                );
            }
        }
    }

    let country_available = read_country_manpower_available(roots.game_state, roots.country)?;
    let occupation_assigned = sum_i32_container(roots.status + STATUS_ASSIGNED_MANPOWER)?;
    let manpower_raw = required_manpower
        .wrapping_sub(country_available)
        .wrapping_sub(occupation_assigned);
    let missing_manpower = i64::from(manpower_raw.max(0));

    let stockpile = read_stable_usize(roots.country + COUNTRY_STOCKPILE)
        .ok_or("a729 country stockpile root is unreadable or unstable")?;
    require_heap(stockpile, "country stockpile")?;
    let mut missing_equipment = Vec::new();
    for required in required_equipment.into_values() {
        let stockpile_raw =
            read_stockpile_amount(roots.game_state, stockpile, required.pointer)?.max(0);
        let shortage_raw = required.raw.wrapping_sub(stockpile_raw);
        let whole_units = shortage_raw / FIXED_POINT_DIVISOR;
        if whole_units > 0 {
            missing_equipment
                .try_reserve(1)
                .map_err(|_| "a729 missing-equipment result allocation failed".to_string())?;
            missing_equipment.push(AlertEquipmentShortage {
                archetype_id: required.archetype_id,
                whole_units,
            });
        }
    }

    Ok(AlertObservation {
        game_hour_raw,
        country_ptr: roots.country as u64,
        predicate_returned: missing_manpower == 0 && missing_equipment.is_empty(),
        missing_manpower,
        missing_equipment,
    })
}

fn resolve_active_country(base: usize) -> Result<Roots, String> {
    let game_state = read_stable_usize(base + GAME_STATE_ROOT_RVA)
        .ok_or("a729 game-state root is unreadable or unstable")?;
    require_heap(game_state, "game state")?;
    let human = read_stable_usize(game_state + GAME_STATE_HUMAN)
        .ok_or("a729 active CHuman root is unreadable or unstable")?;
    require_heap(human, "active CHuman")?;
    let tag = read_stable_u32(human + HUMAN_PLAYER_TAG)
        .ok_or("a729 active CHuman tag is unreadable or unstable")? as usize;
    if tag == 0 {
        return Err("a729 active CHuman tag is zero".into());
    }
    let index_map = read_stable_usize(game_state + GAME_STATE_TAG_TO_COUNTRY_INDEX)
        .ok_or("a729 active-country index map is unreadable or unstable")?;
    require_heap(index_map, "active-country index map")?;
    let country_index = read_stable_u32(
        index_map
            .checked_add(
                tag.checked_mul(4)
                    .ok_or("a729 active-country tag overflow")?,
            )
            .ok_or("a729 active-country index address overflow")?,
    )
    .ok_or("a729 active-country index is unreadable or unstable")? as usize;
    let count = read_stable_u32(game_state + GAME_STATE_COUNTRY_COUNT)
        .ok_or("a729 country count is unreadable or unstable")? as usize;
    if count == 0 || country_index >= count {
        return Err("a729 active CHuman mapping is outside the country table".into());
    }
    let table = read_stable_usize(game_state + GAME_STATE_COUNTRY_TABLE)
        .ok_or("a729 country table is unreadable or unstable")?;
    require_heap(table, "country table")?;
    let country = read_stable_usize(
        table
            .checked_add(
                country_index
                    .checked_mul(8)
                    .ok_or("a729 country index overflow")?,
            )
            .ok_or("a729 country table address overflow")?,
    )
    .ok_or("a729 active country root is unreadable or unstable")?;
    require_heap(country, "active country")?;
    if read_stable_usize(country) != Some(base + COUNTRY_VTABLE_RVA)
        || read_stable_u32(country + COUNTRY_TAG_DB_ID) != Some(tag as u32)
    {
        return Err("a729 active country identity mismatch".into());
    }
    let status = read_stable_usize(country + COUNTRY_OCCUPATION_STATUS)
        .ok_or("a729 occupation-status root is unreadable or unstable")?;
    require_heap(status, "occupation status")?;
    if read_stable_usize(status) != Some(base + OCCUPATION_STATUS_VTABLE_RVA)
        || read_stable_usize(status + STATUS_SECONDARY_VTABLE)
            != Some(base + OCCUPATION_STATUS_SECONDARY_VTABLE_RVA)
    {
        return Err("a729 occupation-status identity mismatch".into());
    }
    Ok(Roots {
        game_state,
        country,
        status,
    })
}

fn read_garrisons(base: usize, status: usize) -> Result<Vec<(usize, usize)>, String> {
    let occupation_data = read_pointer_vector(
        read_stable_usize(status + STATUS_OCCUPATION_DATA_ROWS)
            .ok_or("a729 occupation-data vector is unreadable or unstable")?,
        read_stable_i32(status + STATUS_OCCUPATION_DATA_COUNT)
            .ok_or("a729 occupation-data count is unreadable or unstable")?,
        "occupation data",
    )?;
    let mut rows = Vec::new();
    let mut seen_garrisons = HashSet::new();
    for data in occupation_data {
        require_heap(data, "occupation data")?;
        if read_stable_usize(data) != Some(base + OCCUPATION_DATA_VTABLE_RVA)
            || read_stable_usize(data + OCCUPATION_DATA_PARENT_STATUS) != Some(status)
        {
            return Err("a729 occupation-data identity mismatch".into());
        }
        for garrison in read_robin_hood_garrisons(data)? {
            require_heap(garrison, "garrison")?;
            let identity = read_stable_bytes(garrison, GARRISON_OCCUPATION_DATA + 8)
                .ok_or("a729 garrison identity is unreadable or unstable")?;
            if blob_usize(&identity, 0) != Some(base + GARRISON_VTABLE_RVA)
                || blob_usize(&identity, GARRISON_OCCUPATION_DATA) != Some(data)
            {
                return Err("a729 garrison identity/back-reference mismatch".into());
            }
            seen_garrisons
                .try_reserve(1)
                .map_err(|_| "a729 garrison identity-set allocation failed".to_string())?;
            if !seen_garrisons.insert(garrison) {
                return Err("a729 garrison occurs more than once in exact containers".into());
            }
            rows.try_reserve(1)
                .map_err(|_| "a729 garrison result allocation failed".to_string())?;
            rows.push((garrison, data));
        }
    }
    Ok(rows)
}

fn read_robin_hood_garrisons(data: usize) -> Result<Vec<usize>, String> {
    let count = read_stable_u32(data + GARRISON_TABLE_COUNT)
        .ok_or("a729 garrison-table count is unreadable or unstable")? as usize;
    if count == 0 {
        return Ok(Vec::new());
    }
    let mask = read_stable_u32(data + GARRISON_TABLE_MASK)
        .ok_or("a729 garrison-table mask is unreadable or unstable")? as usize;
    let max_distance = read_stable_u8(data + GARRISON_TABLE_MAX_DISTANCE)
        .ok_or("a729 garrison-table distance is unreadable or unstable")?
        as usize;
    let slots = mask
        .checked_add(max_distance)
        .and_then(|value| value.checked_add(1))
        .ok_or("a729 garrison-table span overflow")?;
    if slots < count {
        return Err("a729 garrison-table span is shorter than its live-row count".into());
    }
    let begin = read_stable_usize(data + GARRISON_TABLE_DATA)
        .ok_or("a729 garrison-table data is unreadable or unstable")?;
    require_heap(begin, "garrison table data")?;
    let bytes = read_stable_bytes(
        begin,
        slots
            .checked_mul(ROBIN_HOOD_ROW_LEN)
            .ok_or("a729 garrison-table byte length overflow")?,
    )
    .ok_or("a729 garrison table is unreadable or unstable")?;
    let mut keyed_rows = Vec::new();
    keyed_rows
        .try_reserve_exact(count)
        .map_err(|_| "a729 garrison-table live-row allocation failed".to_string())?;
    let mut seen_keys = HashSet::new();
    seen_keys
        .try_reserve(count)
        .map_err(|_| "a729 garrison-table key-set allocation failed".to_string())?;
    for slot in 0..slots {
        let offset = slot * ROBIN_HOOD_ROW_LEN;
        if bytes[offset + ROBIN_HOOD_ROW_DISTANCE] != 0 {
            let key = blob_u32(&bytes, offset + ROBIN_HOOD_ROW_KEY)
                .ok_or("a729 garrison-table key is truncated")?;
            if !seen_keys.insert(key) {
                return Err("a729 garrison-table contains a duplicate key".into());
            }
            keyed_rows.push((
                key,
                blob_usize(&bytes, offset + ROBIN_HOOD_ROW_VALUE)
                    .ok_or("a729 garrison-table value is truncated")?,
            ));
        }
    }
    if keyed_rows.len() != count {
        return Err("a729 garrison-table live row count is inconsistent".into());
    }
    keyed_rows.sort_unstable_by_key(|(key, _)| *key);
    let mut values = Vec::new();
    values
        .try_reserve_exact(keyed_rows.len())
        .map_err(|_| "a729 garrison-table result allocation failed".to_string())?;
    for (_, value) in keyed_rows {
        values.push(value);
    }
    Ok(values)
}

fn sum_i32_container(container: usize) -> Result<i32, String> {
    let count = read_stable_i32(container + EQUIPMENT_POOL_COUNT)
        .ok_or("a729 manpower-container count is unreadable or unstable")?;
    let rows = read_pointer_sized_rows(
        read_stable_usize(container + EQUIPMENT_POOL_DATA)
            .ok_or("a729 manpower-container data is unreadable or unstable")?,
        count,
        8,
        "manpower container",
    )?;
    let mut sum = 0i32;
    for offset in (0..rows.len()).step_by(8) {
        sum =
            sum.wrapping_add(blob_i32(&rows, offset + 4).ok_or("a729 manpower row is truncated")?);
    }
    Ok(sum)
}

fn read_required_equipment_pool(pool: usize) -> Result<Vec<(usize, i64)>, String> {
    let (count, bytes) = read_equipment_pool_bytes(pool, REQUIRED_EQUIPMENT_ROW_LEN)?;
    let mut rows = Vec::new();
    rows.try_reserve_exact(count)
        .map_err(|_| "a729 required-equipment result allocation failed".to_string())?;
    for offset in (0..bytes.len()).step_by(REQUIRED_EQUIPMENT_ROW_LEN) {
        let equipment_type =
            blob_usize(&bytes, offset).ok_or("a729 required-equipment row is truncated")?;
        require_heap(equipment_type, "required equipment type")?;
        let amount =
            blob_i64(&bytes, offset + 8).ok_or("a729 required-equipment amount is truncated")?;
        rows.push((equipment_type, amount));
    }
    Ok(rows)
}

fn read_allocation_equipment_pool(pool: usize) -> Result<EquipmentPool, String> {
    let (_, bytes) = read_equipment_pool_bytes(pool, ALLOCATION_EQUIPMENT_ROW_LEN)?;
    let mut rows = HashMap::new();
    rows.try_reserve(bytes.len() / ALLOCATION_EQUIPMENT_ROW_LEN)
        .map_err(|_| "a729 allocation-pool map allocation failed".to_string())?;
    for offset in (0..bytes.len()).step_by(ALLOCATION_EQUIPMENT_ROW_LEN) {
        let archetype =
            blob_usize(&bytes, offset).ok_or("a729 allocation-pool row is truncated")?;
        require_heap(archetype, "allocation-pool archetype")?;
        let id = read_stable_u32(archetype + ARCHETYPE_STABLE_ID_OFF)
            .filter(|id| *id != 0)
            .ok_or("a729 allocation-pool archetype ID is unreadable, unstable, or zero")?;
        let amount = blob_i64(&bytes, offset + ALLOCATION_EQUIPMENT_AMOUNT)
            .ok_or("a729 allocation-pool amount is truncated")?;
        if rows.insert(id, (archetype, amount)).is_some() {
            return Err("a729 allocation pool contains a duplicate stable ID".into());
        }
    }
    Ok(rows)
}

fn read_equipment_pool_bytes(pool: usize, row_len: usize) -> Result<(usize, Vec<u8>), String> {
    let count = read_stable_i32(pool + EQUIPMENT_POOL_COUNT)
        .ok_or("a729 equipment-pool count is unreadable or unstable")?;
    let capacity = read_stable_i32(pool + EQUIPMENT_POOL_CAPACITY)
        .ok_or("a729 equipment-pool capacity is unreadable or unstable")?;
    if count < 0 || capacity < count {
        return Err("a729 equipment-pool count/capacity is invalid".into());
    }
    let bytes = read_pointer_sized_rows(
        read_stable_usize(pool + EQUIPMENT_POOL_DATA)
            .ok_or("a729 equipment-pool data is unreadable or unstable")?,
        count,
        row_len,
        "equipment pool",
    )?;
    Ok((count as usize, bytes))
}

fn pool_amount_by_id(rows: &EquipmentPool, id: u32) -> i64 {
    rows.get(&id).map(|(_, amount)| *amount).unwrap_or(0)
}

fn canonical_equipment_archetype(equipment_type: usize) -> Result<Option<(usize, u32)>, String> {
    require_heap(equipment_type, "required equipment type")?;
    let archetype = read_stable_usize(equipment_type + EQUIPMENT_TYPE_ARCHETYPE)
        .ok_or("a729 required equipment archetype root is unreadable or unstable")?;
    require_heap(archetype, "required equipment archetype root")?;
    if read_stable_u8(archetype + EQUIPMENT_ARCHETYPE_ACTIVE)
        .ok_or("a729 required base-archetype activity is unreadable or unstable")?
        == 0
    {
        return Ok(None);
    }
    let archetype_id = read_stable_u32(archetype + ARCHETYPE_STABLE_ID_OFF)
        .filter(|id| *id != 0)
        .ok_or("a729 required base-archetype ID is unreadable, unstable, or zero")?;
    Ok(Some((archetype, archetype_id)))
}

fn read_country_manpower_available(game_state: usize, country: usize) -> Result<i32, String> {
    let direct = read_stable_i32(country + COUNTRY_MANPOWER_DIRECT)
        .ok_or("a729 direct manpower reserve is unreadable or unstable")?;
    let resolved = resolve_country_handle(
        game_state,
        country + COUNTRY_MANPOWER_TAG_HANDLE,
        GAME_STATE_COUNTRY_TABLE,
        0,
    )?;
    let count = read_stable_i32(resolved + COUNTRY_MANPOWER_OBJECT_COUNT)
        .ok_or("a729 manpower-object count is unreadable or unstable")?;
    let objects = read_pointer_vector(
        read_stable_usize(resolved + COUNTRY_MANPOWER_OBJECTS)
            .ok_or("a729 manpower-object vector is unreadable or unstable")?,
        count,
        "manpower objects",
    )?;
    let mut sum = direct;
    for object in objects {
        require_heap(object, "manpower object")?;
        sum = sum.wrapping_add(
            read_stable_i32(object + MANPOWER_OBJECT_VALUE)
                .ok_or("a729 manpower-object value is unreadable or unstable")?,
        );
    }
    Ok(sum)
}

fn read_stockpile_amount(
    game_state: usize,
    stockpile: usize,
    equipment_type: usize,
) -> Result<i64, String> {
    let mask = effective_equipment_mask(equipment_type)?;
    let context = read_stable_usize(stockpile + STOCKPILE_CONTEXT)
        .ok_or("a729 stockpile context is unreadable or unstable")?;
    require_heap(context, "stockpile context")?;
    let mut raw = if mask & 1 == 0 {
        read_default_stockpile_amount(stockpile, equipment_type)?
    } else {
        read_special_stockpile_amount(context)?
    };

    if read_stable_u8(equipment_type + EQUIPMENT_TYPE_SPECIAL_COUNTRY)
        .ok_or("a729 equipment special-country flag is unreadable or unstable")?
        != 0
    {
        let runtime_country = resolve_country_handle(
            game_state,
            context + 8,
            GAME_STATE_COUNTRY_RUNTIME_OWNER,
            COUNTRY_RUNTIME_OWNER_TABLE,
        )?;
        let add = read_stable_i32(runtime_country + COUNTRY_RUNTIME_ADD_A)
            .ok_or("a729 stockpile country addend is unreadable or unstable")?;
        let subtract = read_stable_i32(runtime_country + COUNTRY_RUNTIME_SUBTRACT_A)
            .ok_or("a729 stockpile country subtractor is unreadable or unstable")?;
        raw = raw
            .wrapping_add(i64::from(add) * FIXED_POINT_DIVISOR)
            .wrapping_sub(i64::from(subtract) * FIXED_POINT_DIVISOR);
    } else if (mask >> 31) & 1 != 0 {
        let runtime_country = resolve_country_handle(
            game_state,
            context + 8,
            GAME_STATE_COUNTRY_RUNTIME_OWNER,
            COUNTRY_RUNTIME_OWNER_TABLE,
        )?;
        let adjustment = read_stable_usize(runtime_country + COUNTRY_RUNTIME_ADJUSTMENT_CONTAINER)
            .ok_or("a729 stockpile adjustment container is unreadable or unstable")?;
        require_heap(adjustment, "stockpile adjustment container")?;
        let subtract = read_stable_i32(adjustment + COUNTRY_RUNTIME_SUBTRACT_B)
            .ok_or("a729 stockpile adjustment is unreadable or unstable")?;
        raw = raw.wrapping_sub(i64::from(subtract) * FIXED_POINT_DIVISOR);
    }
    Ok(raw.max(0))
}

fn read_default_stockpile_amount(stockpile: usize, equipment_type: usize) -> Result<i64, String> {
    let categories = read_stable_usize(stockpile + STOCKPILE_CATEGORY_ARRAY)
        .ok_or("a729 stockpile category array is unreadable or unstable")?;
    require_heap(categories, "stockpile category array")?;
    let category = read_stable_usize(categories + STOCKPILE_DEFAULT_CATEGORY * 8)
        .ok_or("a729 default stockpile category is unreadable or unstable")?;
    require_heap(category, "default stockpile category")?;
    let lookup_owner = read_stable_usize(category + STOCKPILE_CATEGORY_LOOKUP_OWNER)
        .ok_or("a729 stockpile lookup owner is unreadable or unstable")?;
    require_heap(lookup_owner, "stockpile lookup owner")?;
    let lookup_ref = read_stable_usize(lookup_owner + STOCKPILE_CATEGORY_LOOKUP_ROOT)
        .ok_or("a729 stockpile lookup root is unreadable or unstable")?;
    require_heap(lookup_ref, "stockpile lookup root")?;
    let values = read_stable_usize(lookup_ref)
        .ok_or("a729 stockpile values table is unreadable or unstable")?;
    require_heap(values, "stockpile values table")?;
    let index = read_stable_u32(equipment_type + EQUIPMENT_TYPE_STOCKPILE_INDEX)
        .ok_or("a729 equipment stockpile index is unreadable or unstable")?
        as usize;
    read_stable_i64(
        values
            .checked_add(
                index
                    .checked_mul(8)
                    .ok_or("a729 stockpile index overflow")?,
            )
            .ok_or("a729 stockpile value address overflow")?,
    )
    .ok_or_else(|| "a729 stockpile value is unreadable or unstable".into())
}

fn read_special_stockpile_amount(context: usize) -> Result<i64, String> {
    let cached = read_stable_u8(context + STOCKPILE_CONTEXT_CACHE_VALID)
        .ok_or("a729 special-stockpile cache flag is unreadable or unstable")?;
    let total = if cached != 0 {
        read_stable_i64(context + STOCKPILE_CONTEXT_CACHE_RAW)
            .ok_or("a729 special-stockpile cache is unreadable or unstable")?
    } else {
        let pool = context + STOCKPILE_CONTEXT_SPECIAL_POOL;
        let count = read_stable_i32(pool + SPECIAL_POOL_COUNT)
            .ok_or("a729 special-stockpile row count is unreadable or unstable")?;
        let bytes = read_pointer_sized_rows(
            read_stable_usize(pool + SPECIAL_POOL_DATA)
                .ok_or("a729 special-stockpile rows are unreadable or unstable")?,
            count,
            REQUIRED_EQUIPMENT_ROW_LEN,
            "special stockpile",
        )?;
        let mut total = 0i64;
        for offset in (0..bytes.len()).step_by(REQUIRED_EQUIPMENT_ROW_LEN) {
            let archetype =
                blob_usize(&bytes, offset).ok_or("a729 special-stockpile row is truncated")?;
            require_heap(archetype, "special-stockpile archetype")?;
            let flags = read_stable_u64(archetype + EQUIPMENT_ARCHETYPE_POOL_FLAGS)
                .ok_or("a729 special-stockpile archetype flags are unreadable or unstable")?;
            if flags & 1 != 0 {
                total = accumulate_pool(
                    total,
                    blob_i64(&bytes, offset + 8)
                        .ok_or("a729 special-stockpile amount is truncated")?,
                );
            }
        }
        total
    };
    let used = read_stable_i32(context + STOCKPILE_CONTEXT_SPECIAL_USED)
        .ok_or("a729 special-stockpile used count is unreadable or unstable")?;
    let whole = (total / FIXED_POINT_DIVISOR) as i32;
    Ok(i64::from(whole.wrapping_sub(used)) * FIXED_POINT_DIVISOR)
}

fn effective_equipment_mask(equipment_type: usize) -> Result<u64, String> {
    let mut primary = read_stable_u64(equipment_type + EQUIPMENT_TYPE_PRIMARY_FLAGS)
        .ok_or("a729 equipment primary flags are unreadable or unstable")?;
    if primary == 0 {
        let archetype = read_stable_usize(equipment_type + EQUIPMENT_TYPE_ARCHETYPE)
            .ok_or("a729 equipment archetype is unreadable or unstable")?;
        require_heap(archetype, "equipment archetype")?;
        primary = read_stable_u64(archetype + EQUIPMENT_ARCHETYPE_CATEGORY_FLAGS)
            .ok_or("a729 archetype category flags are unreadable or unstable")?;
    }
    let secondary = read_stable_u64(equipment_type + EQUIPMENT_TYPE_SECONDARY_FLAGS)
        .ok_or("a729 equipment secondary flags are unreadable or unstable")?;
    let mut mask = primary | secondary;
    if mask & 0x8000_4003_c1 != 0 {
        if mask & 0x40 != 0 {
            mask &= !0x181;
        } else if (mask as i8) < 0 {
            mask &= !0x101;
        } else if mask & 0x100 != 0 {
            mask &= !1;
        }
    }
    Ok(mask)
}

fn resolve_country_handle(
    game_state: usize,
    handle: usize,
    table_owner_offset: usize,
    table_offset: usize,
) -> Result<usize, String> {
    let raw =
        read_stable_u32(handle).ok_or("a729 country handle is unreadable or unstable")? as usize;
    let index = if raw == 0 {
        0
    } else {
        let map = read_stable_usize(game_state + GAME_STATE_TAG_TO_COUNTRY_INDEX)
            .ok_or("a729 country-index map is unreadable or unstable")?;
        require_heap(map, "country-index map")?;
        read_stable_u32(
            map.checked_add(raw.checked_mul(4).ok_or("a729 country-handle overflow")?)
                .ok_or("a729 country-index address overflow")?,
        )
        .ok_or("a729 mapped country index is unreadable or unstable")? as usize
    };
    let country_count = read_stable_u32(game_state + GAME_STATE_COUNTRY_COUNT)
        .ok_or("a729 resolved-country count is unreadable or unstable")?
        as usize;
    if country_count == 0 || index >= country_count {
        return Err("a729 resolved country index is outside the country table".into());
    }
    let owner = read_stable_usize(game_state + table_owner_offset)
        .ok_or("a729 country-table owner is unreadable or unstable")?;
    require_heap(owner, "country-table owner")?;
    let table = if table_offset == 0 {
        owner
    } else {
        read_stable_usize(owner + table_offset)
            .ok_or("a729 runtime-country table is unreadable or unstable")?
    };
    require_heap(table, "resolved country table")?;
    let country = read_stable_usize(
        table
            .checked_add(
                index
                    .checked_mul(8)
                    .ok_or("a729 resolved-country overflow")?,
            )
            .ok_or("a729 resolved-country address overflow")?,
    )
    .ok_or("a729 resolved country is unreadable or unstable")?;
    require_heap(country, "resolved country")?;
    Ok(country)
}

fn read_pointer_vector(data: usize, count: i32, label: &str) -> Result<Vec<usize>, String> {
    let bytes = read_pointer_sized_rows(data, count, mem::size_of::<usize>(), label)?;
    let mut rows = Vec::new();
    rows.try_reserve_exact(count.max(0) as usize)
        .map_err(|_| format!("a729 {label} result allocation failed"))?;
    for offset in (0..bytes.len()).step_by(mem::size_of::<usize>()) {
        rows.push(blob_usize(&bytes, offset).ok_or_else(|| format!("a729 {label} row truncated"))?);
    }
    Ok(rows)
}

fn read_pointer_sized_rows(
    data: usize,
    count: i32,
    row_len: usize,
    label: &str,
) -> Result<Vec<u8>, String> {
    if count < 0 {
        return Err(format!("a729 {label} count is negative"));
    }
    if count == 0 {
        return Ok(Vec::new());
    }
    require_heap(data, label)?;
    let len = (count as usize)
        .checked_mul(row_len)
        .ok_or_else(|| format!("a729 {label} byte length overflow"))?;
    read_stable_bytes(data, len)
        .ok_or_else(|| format!("a729 {label} rows are unreadable or unstable"))
}

fn clamp_pool(value: i64) -> i64 {
    value.clamp(-EQUIPMENT_POOL_LIMIT, EQUIPMENT_POOL_LIMIT)
}

fn accumulate_pool(current: i64, delta: i64) -> i64 {
    clamp_pool(current.wrapping_add(delta))
}

fn capture_game_hour_raw(base: usize) -> Option<i32> {
    let game_state = read_stable_usize(base.checked_add(GAME_STATE_ROOT_RVA)?)?;
    require_heap(game_state, "game state").ok()?;
    let address =
        game_state.checked_add(compact_contract::build_profile::CGAMESTATE_CURRENT_HOUR)?;
    read_stable_i32(address)
        .filter(|hour| compact_contract::calendar::hour_index_from_raw(*hour).is_some())
}

fn require_heap(value: usize, label: &str) -> Result<(), String> {
    if value == 0 || value & 7 != 0 {
        return Err(format!("a729 {label} pointer is invalid"));
    }
    #[cfg(test)]
    {
        return if (HEAP_LO..HEAP_HI).contains(&value) {
            Ok(())
        } else {
            Err(format!("a729 {label} pointer is outside test heap"))
        };
    }
    #[cfg(not(test))]
    {
        let mut info: MEMORY_BASIC_INFORMATION = unsafe { mem::zeroed() };
        let observed = unsafe {
            VirtualQuery(
                value as *const c_void,
                &mut info,
                mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        if observed != mem::size_of::<MEMORY_BASIC_INFORMATION>()
            || info.State != MEM_COMMIT
            || info.Type != MEM_PRIVATE
            || info.Protect & (PAGE_GUARD | PAGE_NOACCESS) != 0
        {
            return Err(format!(
                "a729 {label} pointer is not readable private memory"
            ));
        }
        let start = info.BaseAddress as usize;
        let end = start
            .checked_add(info.RegionSize)
            .ok_or_else(|| format!("a729 {label} memory region overflows"))?;
        if !(start..end).contains(&value) {
            return Err(format!("a729 {label} pointer is outside its memory region"));
        }
        Ok(())
    }
}

fn read_stable_bytes(address: usize, len: usize) -> Option<Vec<u8>> {
    let first = read_bytes(address, len)?;
    let second = read_bytes(address, len)?;
    (first == second).then_some(first)
}

fn read_bytes(address: usize, len: usize) -> Option<Vec<u8>> {
    if address == 0 || len == 0 {
        return None;
    }
    #[cfg(test)]
    {
        let test_result =
            TEST_MEMORY.with(|memory| memory.borrow_mut().as_mut().map(|m| m.read(address, len)));
        if let Some(result) = test_result {
            return result;
        }
    }
    if !readable_committed_span(address, len) {
        return None;
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(len).ok()?;
    bytes.resize(len, 0);
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            bytes.as_mut_ptr() as *mut c_void,
            len,
            &mut read,
        )
    };
    (ok != 0 && read == len).then_some(bytes)
}

fn readable_committed_span(address: usize, len: usize) -> bool {
    let Some(end) = address.checked_add(len) else {
        return false;
    };
    let mut cursor = address;
    while cursor < end {
        let mut info: MEMORY_BASIC_INFORMATION = unsafe { mem::zeroed() };
        let observed = unsafe {
            VirtualQuery(
                cursor as *const c_void,
                &mut info,
                mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        if observed != mem::size_of::<MEMORY_BASIC_INFORMATION>()
            || info.State != MEM_COMMIT
            || info.Protect & (PAGE_GUARD | PAGE_NOACCESS) != 0
        {
            return false;
        }
        let region_start = info.BaseAddress as usize;
        let Some(region_end) = region_start.checked_add(info.RegionSize) else {
            return false;
        };
        if cursor < region_start || region_end <= cursor {
            return false;
        }
        cursor = region_end.min(end);
    }
    true
}

fn read_stable_u8(address: usize) -> Option<u8> {
    read_stable_bytes(address, 1).map(|bytes| bytes[0])
}

fn read_stable_u32(address: usize) -> Option<u32> {
    let bytes = read_stable_bytes(address, 4)?;
    Some(u32::from_le_bytes(bytes.try_into().ok()?))
}

fn read_stable_i32(address: usize) -> Option<i32> {
    let bytes = read_stable_bytes(address, 4)?;
    Some(i32::from_le_bytes(bytes.try_into().ok()?))
}

fn read_stable_u64(address: usize) -> Option<u64> {
    let bytes = read_stable_bytes(address, 8)?;
    Some(u64::from_le_bytes(bytes.try_into().ok()?))
}

fn read_stable_i64(address: usize) -> Option<i64> {
    let bytes = read_stable_bytes(address, 8)?;
    Some(i64::from_le_bytes(bytes.try_into().ok()?))
}

fn read_stable_usize(address: usize) -> Option<usize> {
    let bytes = read_stable_bytes(address, mem::size_of::<usize>())?;
    Some(usize::from_le_bytes(bytes.try_into().ok()?))
}

fn blob_usize(bytes: &[u8], offset: usize) -> Option<usize> {
    let end = offset.checked_add(mem::size_of::<usize>())?;
    Some(usize::from_le_bytes(
        bytes.get(offset..end)?.try_into().ok()?,
    ))
}

fn blob_i32(bytes: &[u8], offset: usize) -> Option<i32> {
    let end = offset.checked_add(4)?;
    Some(i32::from_le_bytes(bytes.get(offset..end)?.try_into().ok()?))
}

fn blob_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    Some(u32::from_le_bytes(bytes.get(offset..end)?.try_into().ok()?))
}

fn blob_i64(bytes: &[u8], offset: usize) -> Option<i64> {
    let end = offset.checked_add(8)?;
    Some(i64::from_le_bytes(bytes.get(offset..end)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_bytes(memory: &mut TestMemory, address: usize, bytes: &[u8]) {
        for (offset, byte) in bytes.iter().copied().enumerate() {
            memory.bytes.insert(address + offset, byte);
        }
    }

    fn write_zeroes(memory: &mut TestMemory, address: usize, len: usize) {
        write_bytes(memory, address, &vec![0; len]);
    }

    fn write_u8(memory: &mut TestMemory, address: usize, value: u8) {
        write_bytes(memory, address, &[value]);
    }

    fn write_u32(memory: &mut TestMemory, address: usize, value: u32) {
        write_bytes(memory, address, &value.to_le_bytes());
    }

    fn write_i32(memory: &mut TestMemory, address: usize, value: i32) {
        write_bytes(memory, address, &value.to_le_bytes());
    }

    fn write_i64(memory: &mut TestMemory, address: usize, value: i64) {
        write_bytes(memory, address, &value.to_le_bytes());
    }

    fn write_u64(memory: &mut TestMemory, address: usize, value: u64) {
        write_bytes(memory, address, &value.to_le_bytes());
    }

    fn write_usize(memory: &mut TestMemory, address: usize, value: usize) {
        write_bytes(memory, address, &value.to_le_bytes());
    }

    fn blank_memory() -> TestMemory {
        TestMemory {
            bytes: BTreeMap::new(),
            mutation: None,
        }
    }

    fn minimal_world(hour: i32) -> (TestMemory, usize, usize, usize) {
        let base = 0x1400_0000_0usize;
        let game_state = HEAP_LO + 0x10_000;
        let human = HEAP_LO + 0x20_000;
        let index_map = HEAP_LO + 0x30_000;
        let country_table = HEAP_LO + 0x40_000;
        let country = HEAP_LO + 0x50_000;
        let status = HEAP_LO + 0x60_000;
        let stockpile = HEAP_LO + 0x70_000;
        let raw_tag = 1_500usize;
        let country_index = 2usize;
        let mut memory = blank_memory();

        write_usize(&mut memory, base + GAME_STATE_ROOT_RVA, game_state);
        write_i32(
            &mut memory,
            game_state + compact_contract::build_profile::CGAMESTATE_CURRENT_HOUR,
            hour,
        );
        write_usize(&mut memory, game_state + GAME_STATE_HUMAN, human);
        write_u32(&mut memory, human + HUMAN_PLAYER_TAG, raw_tag as u32);
        write_usize(
            &mut memory,
            game_state + GAME_STATE_TAG_TO_COUNTRY_INDEX,
            index_map,
        );
        write_u32(&mut memory, index_map + raw_tag * 4, country_index as u32);
        write_usize(
            &mut memory,
            game_state + GAME_STATE_COUNTRY_TABLE,
            country_table,
        );
        write_u32(&mut memory, game_state + GAME_STATE_COUNTRY_COUNT, 5_000);
        write_usize(&mut memory, country_table + country_index * 8, country);

        write_usize(&mut memory, country, base + COUNTRY_VTABLE_RVA);
        write_u32(&mut memory, country + COUNTRY_TAG_DB_ID, raw_tag as u32);
        write_usize(&mut memory, country + COUNTRY_OCCUPATION_STATUS, status);
        write_usize(&mut memory, country + COUNTRY_STOCKPILE, stockpile);
        write_u32(
            &mut memory,
            country + COUNTRY_MANPOWER_TAG_HANDLE,
            raw_tag as u32,
        );
        write_i32(&mut memory, country + COUNTRY_MANPOWER_DIRECT, 0);
        write_usize(&mut memory, country + COUNTRY_MANPOWER_OBJECTS, 0);
        write_i32(&mut memory, country + COUNTRY_MANPOWER_OBJECT_COUNT, 0);

        write_usize(&mut memory, status, base + OCCUPATION_STATUS_VTABLE_RVA);
        write_usize(
            &mut memory,
            status + STATUS_SECONDARY_VTABLE,
            base + OCCUPATION_STATUS_SECONDARY_VTABLE_RVA,
        );
        write_usize(&mut memory, status + STATUS_OCCUPATION_DATA_ROWS, 0);
        write_i32(&mut memory, status + STATUS_OCCUPATION_DATA_COUNT, 0);
        write_usize(
            &mut memory,
            status + STATUS_ASSIGNED_MANPOWER + EQUIPMENT_POOL_DATA,
            0,
        );
        write_i32(
            &mut memory,
            status + STATUS_ASSIGNED_MANPOWER + EQUIPMENT_POOL_COUNT,
            0,
        );
        (memory, base, game_state, country)
    }

    struct TestStateGuard;

    impl Drop for TestStateGuard {
        fn drop(&mut self) {
            let _lifecycle = LIFECYCLE_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            TEST_MEMORY.with(|slot| {
                *slot.borrow_mut() = None;
            });
            INSTALLED.store(false, Ordering::Release);
            MODULE_BASE.store(0, Ordering::Release);
        }
    }

    /// released only after `TestStateGuard` has restored the globals, because
    static TEST_GLOBAL_STATE: Mutex<()> = Mutex::new(());

    fn with_test_memory<T>(memory: TestMemory, f: impl FnOnce() -> T) -> T {
        let _serialize = TEST_GLOBAL_STATE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        TEST_MEMORY.with(|slot| {
            *slot.borrow_mut() = Some(memory);
        });
        let _guard = TestStateGuard;
        f()
    }

    #[test]
    fn exact_equipment_mask_normalization_matches_a729_leaf() {
        fn normalize(mut mask: u64) -> u64 {
            if mask & 0x8000_4003_c1 != 0 {
                if mask & 0x40 != 0 {
                    mask &= !0x181;
                } else if (mask as i8) < 0 {
                    mask &= !0x101;
                } else if mask & 0x100 != 0 {
                    mask &= !1;
                }
            }
            mask
        }
        assert_eq!(normalize(0x1c1), 0x40);
        assert_eq!(normalize(0x181), 0x80);
        assert_eq!(normalize(0x101), 0x100);
        assert_eq!(normalize(0x8000_0000_00), 0x8000_0000_00);
    }

    #[test]
    fn equipment_pool_clamp_matches_a729_limits() {
        assert_eq!(clamp_pool(EQUIPMENT_POOL_LIMIT + 1), EQUIPMENT_POOL_LIMIT);
        assert_eq!(clamp_pool(-EQUIPMENT_POOL_LIMIT - 1), -EQUIPMENT_POOL_LIMIT);
        assert_eq!(
            accumulate_pool(i64::MAX, 1),
            -EQUIPMENT_POOL_LIMIT,
            "a729 wraps before applying the signed lower clamp"
        );
        assert_eq!(
            accumulate_pool(i64::MIN, -1),
            EQUIPMENT_POOL_LIMIT,
            "a729 wraps before applying the signed upper clamp"
        );
    }

    #[test]
    fn byte_image_exercises_real_required_and_allocation_layouts() {
        let required_pool = HEAP_LO + 0x100_000;
        let required_rows = HEAP_LO + 0x110_000;
        let allocation_pool = HEAP_LO + 0x120_000;
        let allocation_rows = HEAP_LO + 0x130_000;
        let derived_type = HEAP_LO + 0x140_000;
        let base_archetype = HEAP_LO + 0x150_000;
        let mut memory = blank_memory();

        write_usize(
            &mut memory,
            required_pool + EQUIPMENT_POOL_DATA,
            required_rows,
        );
        write_i32(&mut memory, required_pool + EQUIPMENT_POOL_CAPACITY, 1);
        write_i32(&mut memory, required_pool + EQUIPMENT_POOL_COUNT, 1);
        write_usize(&mut memory, required_rows, derived_type);
        write_i64(&mut memory, required_rows + 8, 300_000);
        write_usize(
            &mut memory,
            derived_type + EQUIPMENT_TYPE_ARCHETYPE,
            base_archetype,
        );
        write_u8(&mut memory, base_archetype + EQUIPMENT_ARCHETYPE_ACTIVE, 1);
        write_u32(&mut memory, base_archetype + ARCHETYPE_STABLE_ID_OFF, 7);

        write_usize(
            &mut memory,
            allocation_pool + EQUIPMENT_POOL_DATA,
            allocation_rows,
        );
        write_i32(&mut memory, allocation_pool + EQUIPMENT_POOL_CAPACITY, 1);
        write_i32(&mut memory, allocation_pool + EQUIPMENT_POOL_COUNT, 1);
        write_usize(&mut memory, allocation_rows, base_archetype);
        write_i64(&mut memory, allocation_rows + 8, 999_999);
        write_i64(
            &mut memory,
            allocation_rows + ALLOCATION_EQUIPMENT_AMOUNT,
            100_000,
        );

        with_test_memory(memory, || {
            assert_eq!(
                read_required_equipment_pool(required_pool).unwrap(),
                vec![(derived_type, 300_000)]
            );
            let allocations = read_allocation_equipment_pool(allocation_pool).unwrap();
            assert_eq!(pool_amount_by_id(&allocations, 7), 100_000);
            assert_eq!(
                canonical_equipment_archetype(derived_type).unwrap(),
                Some((base_archetype, 7))
            );
            TEST_MEMORY.with(|slot| {
                write_u8(
                    slot.borrow_mut().as_mut().unwrap(),
                    base_archetype + EQUIPMENT_ARCHETYPE_ACTIVE,
                    0,
                );
            });
            assert_eq!(canonical_equipment_archetype(derived_type).unwrap(), None);
        });
    }

    #[test]
    fn dense_required_pool_is_not_artificially_capped() {
        const ROWS: usize = 4_097;
        let pool = HEAP_LO + 0x180_000;
        let data = HEAP_LO + 0x190_000;
        let mut memory = blank_memory();
        write_usize(&mut memory, pool + EQUIPMENT_POOL_DATA, data);
        write_i32(&mut memory, pool + EQUIPMENT_POOL_CAPACITY, ROWS as i32);
        write_i32(&mut memory, pool + EQUIPMENT_POOL_COUNT, ROWS as i32);
        write_zeroes(&mut memory, data, ROWS * REQUIRED_EQUIPMENT_ROW_LEN);
        for row in 0..ROWS {
            write_usize(
                &mut memory,
                data + row * REQUIRED_EQUIPMENT_ROW_LEN,
                HEAP_LO + 0x400_000 + row * 8,
            );
            write_i64(
                &mut memory,
                data + row * REQUIRED_EQUIPMENT_ROW_LEN + 8,
                row as i64,
            );
        }
        with_test_memory(memory, || {
            assert_eq!(read_required_equipment_pool(pool).unwrap().len(), ROWS);
        });
    }

    #[test]
    fn byte_image_exercises_stockpile_branches() {
        let (mut memory, _base, game_state, _country) = minimal_world(100);
        let stockpile = HEAP_LO + 0x200_000;
        let context = HEAP_LO + 0x210_000;
        let archetype = HEAP_LO + 0x220_000;
        let categories = HEAP_LO + 0x230_000;
        let category = HEAP_LO + 0x240_000;
        let lookup_owner = HEAP_LO + 0x250_000;
        let lookup_ref = HEAP_LO + 0x260_000;
        let values = HEAP_LO + 0x270_000;
        let special_rows = HEAP_LO + 0x280_000;
        let runtime_owner = HEAP_LO + 0x290_000;
        let runtime_table = HEAP_LO + 0x2a0_000;
        let runtime_country = HEAP_LO + 0x2b0_000;
        let adjustment = HEAP_LO + 0x2c0_000;
        let raw_tag = 1_500usize;

        write_usize(&mut memory, stockpile + STOCKPILE_CONTEXT, context);
        write_usize(
            &mut memory,
            stockpile + STOCKPILE_CATEGORY_ARRAY,
            categories,
        );
        write_usize(
            &mut memory,
            categories + STOCKPILE_DEFAULT_CATEGORY * 8,
            category,
        );
        write_usize(
            &mut memory,
            category + STOCKPILE_CATEGORY_LOOKUP_OWNER,
            lookup_owner,
        );
        write_usize(
            &mut memory,
            lookup_owner + STOCKPILE_CATEGORY_LOOKUP_ROOT,
            lookup_ref,
        );
        write_usize(&mut memory, lookup_ref, values);
        write_u32(&mut memory, archetype + EQUIPMENT_TYPE_STOCKPILE_INDEX, 3);
        write_usize(&mut memory, archetype + EQUIPMENT_TYPE_ARCHETYPE, archetype);
        write_u64(&mut memory, archetype + EQUIPMENT_TYPE_SECONDARY_FLAGS, 0);
        write_u8(&mut memory, archetype + EQUIPMENT_TYPE_SPECIAL_COUNTRY, 0);
        write_i64(&mut memory, values + 3 * 8, 500_000);

        write_u32(&mut memory, context + 8, raw_tag as u32);
        write_usize(
            &mut memory,
            game_state + GAME_STATE_COUNTRY_RUNTIME_OWNER,
            runtime_owner,
        );
        write_usize(
            &mut memory,
            runtime_owner + COUNTRY_RUNTIME_OWNER_TABLE,
            runtime_table,
        );
        write_usize(&mut memory, runtime_table + 2 * 8, runtime_country);
        write_usize(
            &mut memory,
            runtime_country + COUNTRY_RUNTIME_ADJUSTMENT_CONTAINER,
            adjustment,
        );
        write_i32(&mut memory, adjustment + COUNTRY_RUNTIME_SUBTRACT_B, 1);
        write_i32(&mut memory, runtime_country + COUNTRY_RUNTIME_ADD_A, 3);
        write_i32(&mut memory, runtime_country + COUNTRY_RUNTIME_SUBTRACT_A, 1);

        write_u8(&mut memory, context + STOCKPILE_CONTEXT_CACHE_VALID, 0);
        write_usize(
            &mut memory,
            context + STOCKPILE_CONTEXT_SPECIAL_POOL + SPECIAL_POOL_DATA,
            special_rows,
        );
        write_i32(
            &mut memory,
            context + STOCKPILE_CONTEXT_SPECIAL_POOL + SPECIAL_POOL_COUNT,
            1,
        );
        write_usize(&mut memory, special_rows, archetype);
        write_i64(&mut memory, special_rows + 8, 700_000);
        write_u64(&mut memory, archetype + EQUIPMENT_ARCHETYPE_POOL_FLAGS, 1);
        write_i32(&mut memory, context + STOCKPILE_CONTEXT_SPECIAL_USED, 1);

        with_test_memory(memory, || {
            let set_flags = |flags: u64| {
                TEST_MEMORY.with(|slot| {
                    write_u64(
                        slot.borrow_mut().as_mut().unwrap(),
                        archetype + EQUIPMENT_TYPE_PRIMARY_FLAGS,
                        flags,
                    );
                });
            };
            let set_special_country = |value: u8| {
                TEST_MEMORY.with(|slot| {
                    write_u8(
                        slot.borrow_mut().as_mut().unwrap(),
                        archetype + EQUIPMENT_TYPE_SPECIAL_COUNTRY,
                        value,
                    );
                });
            };

            set_flags(2);
            assert_eq!(
                read_stockpile_amount(game_state, stockpile, archetype).unwrap(),
                500_000
            );
            let formerly_capped_index = 1u32 << 20;
            TEST_MEMORY.with(|slot| {
                let mut memory = slot.borrow_mut();
                let memory = memory.as_mut().expect("test memory installed");
                write_u32(
                    memory,
                    archetype + EQUIPMENT_TYPE_STOCKPILE_INDEX,
                    formerly_capped_index,
                );
                write_i64(memory, values + formerly_capped_index as usize * 8, 550_000);
            });
            assert_eq!(
                read_stockpile_amount(game_state, stockpile, archetype).unwrap(),
                550_000
            );
            TEST_MEMORY.with(|slot| {
                write_u32(
                    slot.borrow_mut().as_mut().expect("test memory installed"),
                    archetype + EQUIPMENT_TYPE_STOCKPILE_INDEX,
                    3,
                );
            });

            set_flags(1);
            assert_eq!(
                read_stockpile_amount(game_state, stockpile, archetype).unwrap(),
                600_000
            );

            set_flags(0x8000_0000);
            assert_eq!(
                read_stockpile_amount(game_state, stockpile, archetype).unwrap(),
                400_000
            );

            set_flags(2);
            set_special_country(1);
            assert_eq!(
                read_stockpile_amount(game_state, stockpile, archetype).unwrap(),
                700_000
            );
        });
    }

    #[test]
    fn byte_image_preserves_exact_container_and_robin_key_order() {
        let hour = 122;
        let (mut memory, base, game_state, country) = minimal_world(hour);
        let status = HEAP_LO + 0x60_000;
        let occupation_vector = HEAP_LO + 0x300_000;
        let occupation_a = HEAP_LO + 0x310_000;
        let occupation_b = HEAP_LO + 0x320_000;
        let table_a = HEAP_LO + 0x330_000;
        let table_b = HEAP_LO + 0x340_000;
        let garrison_a = HEAP_LO + 0x370_000;
        let garrison_b = HEAP_LO + 0x360_000;
        let garrison_c = HEAP_LO + 0x350_000;
        let required_a = HEAP_LO + 0x380_000;
        let required_b = HEAP_LO + 0x390_000;
        let required_c = HEAP_LO + 0x3a0_000;
        let derived_type = HEAP_LO + 0x3b0_000;
        let archetype = HEAP_LO + 0x3c0_000;
        let stockpile = HEAP_LO + 0x70_000;
        let stockpile_context = HEAP_LO + 0x3d0_000;

        write_usize(
            &mut memory,
            status + STATUS_OCCUPATION_DATA_ROWS,
            occupation_vector,
        );
        write_i32(&mut memory, status + STATUS_OCCUPATION_DATA_COUNT, 2);
        write_usize(&mut memory, occupation_vector, occupation_a);
        write_usize(&mut memory, occupation_vector + 8, occupation_b);

        for occupation in [occupation_a, occupation_b] {
            write_usize(&mut memory, occupation, base + OCCUPATION_DATA_VTABLE_RVA);
            write_usize(
                &mut memory,
                occupation + OCCUPATION_DATA_PARENT_STATUS,
                status,
            );
        }
        write_usize(&mut memory, occupation_a + GARRISON_TABLE_DATA, table_a);
        write_u32(&mut memory, occupation_a + GARRISON_TABLE_COUNT, 2);
        write_u32(&mut memory, occupation_a + GARRISON_TABLE_MASK, 1);
        write_u8(&mut memory, occupation_a + GARRISON_TABLE_MAX_DISTANCE, 0);
        write_zeroes(&mut memory, table_a, ROBIN_HOOD_ROW_LEN * 2);
        write_u8(&mut memory, table_a + ROBIN_HOOD_ROW_DISTANCE, 1);
        write_u32(&mut memory, table_a + ROBIN_HOOD_ROW_KEY, 2);
        write_usize(&mut memory, table_a + ROBIN_HOOD_ROW_VALUE, garrison_b);
        write_u8(
            &mut memory,
            table_a + ROBIN_HOOD_ROW_LEN + ROBIN_HOOD_ROW_DISTANCE,
            1,
        );
        write_u32(
            &mut memory,
            table_a + ROBIN_HOOD_ROW_LEN + ROBIN_HOOD_ROW_KEY,
            1,
        );
        write_usize(
            &mut memory,
            table_a + ROBIN_HOOD_ROW_LEN + ROBIN_HOOD_ROW_VALUE,
            garrison_a,
        );

        write_usize(&mut memory, occupation_b + GARRISON_TABLE_DATA, table_b);
        write_u32(&mut memory, occupation_b + GARRISON_TABLE_COUNT, 1);
        write_u32(&mut memory, occupation_b + GARRISON_TABLE_MASK, 0);
        write_u8(&mut memory, occupation_b + GARRISON_TABLE_MAX_DISTANCE, 0);
        write_zeroes(&mut memory, table_b, ROBIN_HOOD_ROW_LEN);
        write_u8(&mut memory, table_b + ROBIN_HOOD_ROW_DISTANCE, 1);
        write_u32(&mut memory, table_b + ROBIN_HOOD_ROW_KEY, 1);
        write_usize(&mut memory, table_b + ROBIN_HOOD_ROW_VALUE, garrison_c);

        for (garrison, occupation, required_rows, amount) in [
            (
                garrison_a,
                occupation_a,
                required_a,
                -2 * EQUIPMENT_POOL_LIMIT,
            ),
            (garrison_b, occupation_a, required_b, EQUIPMENT_POOL_LIMIT),
            (garrison_c, occupation_b, required_c, EQUIPMENT_POOL_LIMIT),
        ] {
            write_zeroes(&mut memory, garrison, GARRISON_WINDOW_LEN);
            write_usize(&mut memory, garrison, base + GARRISON_VTABLE_RVA);
            write_usize(&mut memory, garrison + GARRISON_OCCUPATION_DATA, occupation);
            write_usize(
                &mut memory,
                garrison + GARRISON_REQUIRED_EQUIPMENT + EQUIPMENT_POOL_DATA,
                required_rows,
            );
            write_i32(
                &mut memory,
                garrison + GARRISON_REQUIRED_EQUIPMENT + EQUIPMENT_POOL_CAPACITY,
                1,
            );
            write_i32(
                &mut memory,
                garrison + GARRISON_REQUIRED_EQUIPMENT + EQUIPMENT_POOL_COUNT,
                1,
            );
            write_zeroes(&mut memory, required_rows, REQUIRED_EQUIPMENT_ROW_LEN);
            write_usize(&mut memory, required_rows, derived_type);
            write_i64(&mut memory, required_rows + 8, amount);
        }

        write_usize(
            &mut memory,
            derived_type + EQUIPMENT_TYPE_ARCHETYPE,
            archetype,
        );
        write_u8(&mut memory, archetype + EQUIPMENT_ARCHETYPE_ACTIVE, 1);
        write_u32(&mut memory, archetype + ARCHETYPE_STABLE_ID_OFF, 7);
        write_usize(&mut memory, archetype + EQUIPMENT_TYPE_ARCHETYPE, archetype);
        write_u64(&mut memory, archetype + EQUIPMENT_TYPE_PRIMARY_FLAGS, 1);
        write_u64(&mut memory, archetype + EQUIPMENT_TYPE_SECONDARY_FLAGS, 0);
        write_u8(&mut memory, archetype + EQUIPMENT_TYPE_SPECIAL_COUNTRY, 0);
        write_usize(
            &mut memory,
            stockpile + STOCKPILE_CONTEXT,
            stockpile_context,
        );
        write_u8(
            &mut memory,
            stockpile_context + STOCKPILE_CONTEXT_CACHE_VALID,
            1,
        );
        write_i64(
            &mut memory,
            stockpile_context + STOCKPILE_CONTEXT_CACHE_RAW,
            0,
        );
        write_i32(
            &mut memory,
            stockpile_context + STOCKPILE_CONTEXT_SPECIAL_USED,
            0,
        );

        with_test_memory(memory, || {
            let roots = resolve_active_country(base).unwrap();
            assert_eq!(roots.game_state, game_state);
            assert_eq!(roots.country, country);
            let observation = reconstruct_alert(base, roots, hour).unwrap();
            assert_eq!(
                observation.missing_equipment,
                vec![AlertEquipmentShortage {
                    archetype_id: 7,
                    whole_units: EQUIPMENT_POOL_LIMIT / FIXED_POINT_DIVISOR,
                }]
            );
        });
    }

    #[test]
    fn byte_image_cadence_remaps_mod_tag_and_rejects_torn_same_hour_snapshot() {
        let hour = compact_contract::calendar::DATE_EPOCH_CONST + 1936 * 365 * 24;
        let (memory, base, _game_state, country) = minimal_world(hour);
        with_test_memory(memory, || {
            MODULE_BASE.store(base, Ordering::Release);
            INSTALLED.store(true, Ordering::Release);
            for _ in 0..5 {
                let observation = take_for_hour(hour).unwrap().unwrap();
                assert_eq!(observation.country_ptr, country as u64);
                assert!(observation.predicate_returned);
            }
            assert_eq!(seal_drain_and_uninstall().unwrap(), AlertDrain::default());
            assert!(!installed());
        });

        let (mut torn, base, _game_state, country) = minimal_world(hour);
        torn.mutation = Some(TestMutation {
            trigger_address: country + COUNTRY_MANPOWER_DIRECT,
            remaining_reads: 2,
            write_address: country + COUNTRY_MANPOWER_DIRECT,
            bytes: (-100i32).to_le_bytes().to_vec(),
        });
        with_test_memory(torn, || {
            MODULE_BASE.store(base, Ordering::Release);
            INSTALLED.store(true, Ordering::Release);
            let error = take_for_hour(hour).unwrap_err();
            assert_eq!(error, "a729 garrison-alert snapshot changed during capture");
            seal_drain_and_uninstall().unwrap();
        });

        for remaining_reads in [1, 2, 4] {
            let (mut rollover, base, game_state, _country) = minimal_world(hour);
            let hour_address =
                game_state + compact_contract::build_profile::CGAMESTATE_CURRENT_HOUR;
            rollover.mutation = Some(TestMutation {
                trigger_address: hour_address,
                remaining_reads,
                write_address: hour_address,
                bytes: (hour + 1).to_le_bytes().to_vec(),
            });
            with_test_memory(rollover, || {
                MODULE_BASE.store(base, Ordering::Release);
                INSTALLED.store(true, Ordering::Release);
                assert_eq!(take_for_hour(hour).unwrap(), None);
                seal_drain_and_uninstall().unwrap();
            });
        }
    }
}
