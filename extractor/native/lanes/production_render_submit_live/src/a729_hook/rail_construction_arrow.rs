use super::{read_u32, read_u8, read_usize, reject};
use core::ffi::c_void;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::mem;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

pub const PRODUCER_RVA: usize = 0x0159_B650;
pub const PRODUCER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24, 0x20, 0x55,
];
const BUCKET_STRIDE: usize = 0x30;

type ProducerFn = unsafe extern "system" fn(usize, u32, u32, u8);

pub static ORIGINAL_PRODUCER: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawRailConstructionArrowVisualObservation {
    pub producer_rva: u32,
    pub province_lo: u32,
    pub province_hi: u32,
    pub edge_type_u8: u8,
    pub entity_instance_id: u64,
    pub entity_registration_token: Option<String>,
}

#[derive(Clone)]
struct ObservedEntity {
    entity_instance_id: u64,
    definition_ptr: usize,
    registration_token: Option<String>,
}

#[derive(Clone)]
struct ActiveArrow {
    entity_ptr: usize,
    raw: RawRailConstructionArrowVisualObservation,
}

#[derive(Default)]
struct State {
    entities: HashMap<usize, ObservedEntity>,
    active: BTreeMap<(u32, u32), ActiveArrow>,
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();

fn state() -> &'static Mutex<State> {
    STATE.get_or_init(|| Mutex::new(State::default()))
}

pub fn detour() -> *mut c_void {
    hook_producer as *mut c_void
}

pub fn observe_entity_created(
    entity_ptr: usize,
    entity_instance_id: u64,
    definition_ptr: usize,
    registration_token: Option<String>,
) {
    if entity_ptr == 0 || entity_instance_id == 0 || definition_ptr == 0 {
        reject();
        return;
    }
    let Ok(mut state) = state().lock() else {
        reject();
        return;
    };
    let row = ObservedEntity {
        entity_instance_id,
        definition_ptr,
        registration_token,
    };
    let duplicate = state.entities.insert(entity_ptr, row).is_some();
    drop(state);
    if duplicate {
        reject();
    }
}

pub fn observe_entity_registration_token(entity_ptr: usize, token: String) {
    let Ok(mut state) = state().lock() else {
        reject();
        return;
    };
    let Some(entity) = state.entities.get_mut(&entity_ptr) else {
        return;
    };
    if entity
        .registration_token
        .as_ref()
        .is_some_and(|existing| existing != &token)
    {
        drop(state);
        reject();
        return;
    }
    entity.registration_token = Some(token);
}

pub fn observe_entity_destroy(entity_ptr: usize) {
    let Ok(mut state) = state().lock() else {
        reject();
        return;
    };
    state.entities.remove(&entity_ptr);
    state.active.retain(|_, row| row.entity_ptr != entity_ptr);
}

pub fn snapshot_active() -> Option<Vec<RawRailConstructionArrowVisualObservation>> {
    state()
        .lock()
        .ok()
        .map(|state| state.active.values().map(|row| row.raw.clone()).collect())
}

pub fn validate_snapshot(rows: &[RawRailConstructionArrowVisualObservation]) -> Result<(), String> {
    let mut prior_key = None;
    let mut entity_ids = BTreeSet::new();
    for row in rows {
        let key = (row.province_lo, row.province_hi);
        if row.producer_rva != PRODUCER_RVA as u32
            || row.province_lo == 0
            || row.province_lo >= row.province_hi
            || !matches!(row.edge_type_u8, 2 | 3)
            || row.entity_instance_id == 0
            || row
                .entity_registration_token
                .as_ref()
                .is_some_and(|token| token.is_empty())
            || prior_key.is_some_and(|prior| prior >= key)
            || !entity_ids.insert(row.entity_instance_id)
        {
            return Err(
                "rail construction-arrow snapshot is null, reordered, duplicated, or outside the exact a729 producer law"
                    .into(),
            );
        }
        prior_key = Some(key);
    }
    Ok(())
}

pub fn reset() {
    if let Some(state) = STATE.get() {
        if let Ok(mut state) = state.lock() {
            *state = State::default();
        } else {
            reject();
        }
    }
}

fn canonical_pair(a: u32, b: u32) -> Option<(u32, u32)> {
    (a != 0 && b != 0 && a != b).then(|| if a < b { (a, b) } else { (b, a) })
}

fn endpoint_hash(province_lo: u32, province_hi: u32) -> u32 {
    province_lo
        .to_le_bytes()
        .into_iter()
        .chain(province_hi.to_le_bytes())
        .fold(0x811c_9dc5u32, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
        })
}

fn find_entity_ptr(
    manager: usize,
    province_lo: u32,
    province_hi: u32,
) -> Result<Option<usize>, ()> {
    let base = read_usize(manager.checked_add(0x90).ok_or(())?).ok_or(())?;
    let mask = read_u32(manager.checked_add(0x9c).ok_or(())?).ok_or(())?;
    let bucket_count = mask.checked_add(1).ok_or(())?;
    if base == 0 || bucket_count == 0 || !bucket_count.is_power_of_two() {
        return Err(());
    }
    let bucket = endpoint_hash(province_lo, province_hi) & mask;
    let entry_offset = (bucket as usize).checked_mul(BUCKET_STRIDE).ok_or(())?;
    let mut entry = base.checked_add(entry_offset).ok_or(())?;
    if read_u8(entry.checked_add(4).ok_or(())?).ok_or(())? as i8 == -1 {
        return Ok(None);
    }
    let mut ordinal = 1u16;
    loop {
        if read_u32(entry.checked_add(8).ok_or(())?).ok_or(())? == province_lo
            && read_u32(entry.checked_add(0x0c).ok_or(())?).ok_or(())? == province_hi
        {
            let entity_ptr = read_usize(entry.checked_add(0x10).ok_or(())?).ok_or(())?;
            return if entity_ptr == 0 {
                Err(())
            } else {
                Ok(Some(entity_ptr))
            };
        }
        ordinal = ordinal.checked_add(1).ok_or(())?;
        let chain_length = u16::from(read_u8(entry.checked_add(0x34).ok_or(())?).ok_or(())?);
        if ordinal > chain_length {
            return Ok(None);
        }
        entry = entry.checked_add(BUCKET_STRIDE).ok_or(())?;
    }
}

unsafe extern "system" fn hook_producer(
    manager: usize,
    province_a: u32,
    province_b: u32,
    enabled: u8,
) {
    let original_address = ORIGINAL_PRODUCER.load(Ordering::Acquire);
    if original_address == 0 {
        reject();
        return;
    }
    let original: ProducerFn = unsafe { mem::transmute(original_address) };
    unsafe { original(manager, province_a, province_b, enabled) };

    let Some((province_lo, province_hi)) = canonical_pair(province_a, province_b) else {
        reject();
        return;
    };
    if manager == 0 || enabled > 1 {
        reject();
        return;
    }
    let entity_ptr = match find_entity_ptr(manager, province_lo, province_hi) {
        Ok(value) => value,
        Err(()) => {
            reject();
            return;
        }
    };
    let Ok(mut state) = state().lock() else {
        reject();
        return;
    };
    if enabled == 0 {
        if entity_ptr.is_some() {
            drop(state);
            reject();
            return;
        }
        state.active.remove(&(province_lo, province_hi));
        return;
    }

    let Some(entity_ptr) = entity_ptr else {
        drop(state);
        reject();
        return;
    };
    let Some(entity) = state.entities.get(&entity_ptr).cloned() else {
        drop(state);
        reject();
        return;
    };
    let prototype_type_2 = manager.checked_add(0x20).and_then(read_usize);
    let prototype_type_3 = manager.checked_add(0x28).and_then(read_usize);
    let edge_type_u8 = match (
        prototype_type_2 == Some(entity.definition_ptr),
        prototype_type_3 == Some(entity.definition_ptr),
    ) {
        (true, false) => 2,
        (false, true) => 3,
        _ => {
            drop(state);
            reject();
            return;
        }
    };
    state.active.insert(
        (province_lo, province_hi),
        ActiveArrow {
            entity_ptr,
            raw: RawRailConstructionArrowVisualObservation {
                producer_rva: PRODUCER_RVA as u32,
                province_lo,
                province_hi,
                edge_type_u8,
                entity_instance_id: entity.entity_instance_id,
                entity_registration_token: entity.registration_token,
            },
        },
    );
}
