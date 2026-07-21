//! Exact-a729 acquisition hooks for the C07/O05 generic map-building carrier.
//!
//! This module is deliberately process-local. It snapshots complete
//! `CGraphicalMap::UpdateProvinces` building scopes, consumes transient record
//! and entity pointers only for joins, and exposes a pointer-free selected-
//! Present observation to the collector.

use super::{
    capture_present_game_hour_raw, query_performance_counter, query_performance_frequency,
    read_bytes, read_u32, read_usize, verify_sentinel, STATE,
};
use core::cell::RefCell;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::mem;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

pub(super) const UPDATE_PROVINCES_RVA: usize = 0x00b4_c120;
pub(super) const BUILDING_CONSTRUCTOR_RVA: usize = 0x0165_d5c0;
pub(super) const BUILDING_VECTOR_UPDATE_RVA: usize = 0x0127_b2e0;
pub(super) const ENTITY_TRANSFORM_SETTER_RVA: usize = 0x0227_ba20;

const RECORD_BYTES: usize = 0x48;
const MAX_RECORDS_PER_SCOPE: usize = 1_048_576;
const MAX_COMPLETED_SCOPES: usize = 4_096;

const UPDATE_PROVINCES_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x18, 0x48, 0x89, 0x50, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54,
];
const BUILDING_CONSTRUCTOR_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x4c, 0x89, 0x4c, 0x24, 0x20, 0x89, 0x54, 0x24, 0x10, 0x55, 0x56,
];
const BUILDING_VECTOR_UPDATE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57,
];
const ENTITY_TRANSFORM_SETTER_SENTINEL: [u8; 16] = [
    0x40, 0x55, 0x53, 0x57, 0x48, 0x8d, 0x6c, 0x24, 0xd0, 0x48, 0x81, 0xec, 0x30, 0x01, 0x00, 0x00,
];

type UpdateProvincesFn = unsafe extern "system" fn(usize, usize, u32);
type BuildingConstructorFn =
    unsafe extern "system" fn(usize, i32, usize, usize, i32, usize) -> usize;
type BuildingVectorUpdateFn = unsafe extern "system" fn(usize, u8, u8, usize, u32);
type EntityTransformSetterFn = unsafe extern "system" fn(usize, usize);

pub(super) static ORIGINAL_UPDATE_PROVINCES: AtomicUsize = AtomicUsize::new(0);
pub(super) static ORIGINAL_BUILDING_CONSTRUCTOR: AtomicUsize = AtomicUsize::new(0);
pub(super) static ORIGINAL_BUILDING_VECTOR_UPDATE: AtomicUsize = AtomicUsize::new(0);
pub(super) static ORIGINAL_ENTITY_TRANSFORM_SETTER: AtomicUsize = AtomicUsize::new(0);
static NEXT_RUNTIME_OBJECT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawO05RuntimeRecordObservation {
    pub runtime_object_id: u64,
    pub record_bytes: [u8; RECORD_BYTES],
    pub selected_entity_instance_id: Option<u64>,
    pub world_transform_4x4_f32_bits: Option<[u32; 16]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawO05RuntimeScopeObservation {
    pub game_hour_raw: i32,
    pub first_qpc_ticks: u64,
    pub completed_qpc_ticks: u64,
    pub qpc_frequency_hz: u64,
    pub records: Vec<RawO05RuntimeRecordObservation>,
}

#[derive(Default)]
struct O05State {
    runtime_ids: HashMap<usize, u64>,
    entity_matrices: HashMap<usize, [u32; 16]>,
    completed: VecDeque<RawO05RuntimeScopeObservation>,
    lost_or_rejected: bool,
}

struct PendingScope {
    game_hour_raw: i32,
    first_qpc_ticks: u64,
    qpc_frequency_hz: u64,
    seen_record_ptrs: BTreeSet<usize>,
    records: Vec<RawO05RuntimeRecordObservation>,
    rejected: bool,
}

static O05_STATE: OnceLock<Mutex<O05State>> = OnceLock::new();
thread_local! {
    static CURRENT_SCOPE: RefCell<Option<PendingScope>> = const { RefCell::new(None) };
}

fn state() -> &'static Mutex<O05State> {
    O05_STATE.get_or_init(|| Mutex::new(O05State::default()))
}

pub(super) fn verify_sentinels(base: usize) -> Result<(), String> {
    for (address, sentinel, label) in [
        (
            base + UPDATE_PROVINCES_RVA,
            &UPDATE_PROVINCES_SENTINEL,
            "O05 UpdateProvinces owner",
        ),
        (
            base + BUILDING_CONSTRUCTOR_RVA,
            &BUILDING_CONSTRUCTOR_SENTINEL,
            "O05 runtime-record constructor",
        ),
        (
            base + BUILDING_VECTOR_UPDATE_RVA,
            &BUILDING_VECTOR_UPDATE_SENTINEL,
            "O05 per-province building vector update",
        ),
        (
            base + ENTITY_TRANSFORM_SETTER_RVA,
            &ENTITY_TRANSFORM_SETTER_SENTINEL,
            "O05 entity transform setter",
        ),
    ] {
        verify_sentinel(address, sentinel, label)?;
    }
    Ok(())
}

pub(super) unsafe extern "system" fn hook_update_provinces(
    owner: usize,
    camera_or_update: usize,
    flags: u32,
) {
    let original: UpdateProvincesFn =
        unsafe { mem::transmute(ORIGINAL_UPDATE_PROVINCES.load(Ordering::Acquire)) };
    let start = (
        capture_present_game_hour_raw(),
        query_performance_counter(),
        query_performance_frequency(),
    );
    CURRENT_SCOPE.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_some() {
            if let Some(active) = slot.as_mut() {
                active.rejected = true;
            }
            return;
        }
        if let (Ok(game_hour_raw), Some(first_qpc_ticks), Some(qpc_frequency_hz)) = start {
            *slot = Some(PendingScope {
                game_hour_raw,
                first_qpc_ticks,
                qpc_frequency_hz,
                seen_record_ptrs: BTreeSet::new(),
                records: Vec::new(),
                rejected: false,
            });
        }
    });
    unsafe { original(owner, camera_or_update, flags) };
    let completed_qpc_ticks = query_performance_counter();
    let completed_hour = capture_present_game_hour_raw();
    CURRENT_SCOPE.with(|slot| {
        let Some(pending) = slot.borrow_mut().take() else {
            return;
        };
        if pending.records.is_empty() && !pending.rejected {
            return;
        }
        let accepted = completed_qpc_ticks
            .zip(completed_hour.ok())
            .filter(|(ticks, hour)| {
                !pending.rejected
                    && *ticks >= pending.first_qpc_ticks
                    && *hour == pending.game_hour_raw
                    && !pending.records.is_empty()
            })
            .map(|(ticks, _)| RawO05RuntimeScopeObservation {
                game_hour_raw: pending.game_hour_raw,
                first_qpc_ticks: pending.first_qpc_ticks,
                completed_qpc_ticks: ticks,
                qpc_frequency_hz: pending.qpc_frequency_hz,
                records: pending.records,
            });
        let Ok(mut state) = state().lock() else {
            return;
        };
        match accepted {
            Some(scope) => {
                if let Some(index) = state
                    .completed
                    .iter()
                    .position(|row| row.game_hour_raw == scope.game_hour_raw)
                {
                    if state.completed[index].completed_qpc_ticks >= scope.completed_qpc_ticks {
                        state.lost_or_rejected = true;
                    } else {
                        state.completed[index] = scope;
                    }
                } else if state.completed.len() < MAX_COMPLETED_SCOPES {
                    state.completed.push_back(scope);
                } else {
                    state.lost_or_rejected = true;
                }
            }
            None => state.lost_or_rejected = true,
        }
    });
}

pub(super) unsafe extern "system" fn hook_building_constructor(
    owner: usize,
    building_type: i32,
    source: usize,
    spawn_point: usize,
    kind: i32,
    extra: usize,
) -> usize {
    let original: BuildingConstructorFn =
        unsafe { mem::transmute(ORIGINAL_BUILDING_CONSTRUCTOR.load(Ordering::Acquire)) };
    let record =
        unsafe { original(owner, building_type, source, spawn_point, kind, extra) };
    if record != 0 {
        let stable_id = NEXT_RUNTIME_OBJECT_ID.fetch_add(1, Ordering::AcqRel);
        if stable_id == 0 {
            if let Ok(mut state) = state().lock() {
                state.lost_or_rejected = true;
            }
        } else if let Ok(mut state) = state().lock() {
            state.runtime_ids.insert(record, stable_id);
        }
    }
    record
}

pub(super) unsafe extern "system" fn hook_building_vector_update(
    province_graphics: usize,
    visible: u8,
    update: u8,
    camera: usize,
    flags: u32,
) {
    let original: BuildingVectorUpdateFn =
        unsafe { mem::transmute(ORIGINAL_BUILDING_VECTOR_UPDATE.load(Ordering::Acquire)) };
    unsafe { original(province_graphics, visible, update, camera, flags) };
    CURRENT_SCOPE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(pending) = slot.as_mut() else {
            return;
        };
        let Some(vector) = read_usize(province_graphics.saturating_add(0x1548)) else {
            pending.rejected = true;
            return;
        };
        let Some(count) = read_u32(province_graphics.saturating_add(0x1554))
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| *value <= MAX_RECORDS_PER_SCOPE)
        else {
            pending.rejected = true;
            return;
        };
        if (count != 0 && vector == 0)
            || pending.records.len().saturating_add(count) > MAX_RECORDS_PER_SCOPE
        {
            pending.rejected = true;
            return;
        }
        let Ok(mut state) = state().lock() else {
            pending.rejected = true;
            return;
        };
        let entity_state = match STATE.get().and_then(|value| value.lock().ok()) {
            Some(value) => value,
            None => {
                pending.rejected = true;
                return;
            }
        };
        for ordinal in 0..count {
            let Some(pointer_slot) = vector.checked_add(ordinal.saturating_mul(mem::size_of::<usize>()))
            else {
                pending.rejected = true;
                return;
            };
            let Some(record_ptr) = read_usize(pointer_slot).filter(|value| *value != 0) else {
                pending.rejected = true;
                return;
            };
            if !pending.seen_record_ptrs.insert(record_ptr) {
                pending.rejected = true;
                return;
            }
            let Some(bytes) = read_bytes(record_ptr, RECORD_BYTES) else {
                pending.rejected = true;
                return;
            };
            let Ok(record_bytes): Result<[u8; RECORD_BYTES], _> = bytes.try_into() else {
                pending.rejected = true;
                return;
            };
            let runtime_object_id = match state.runtime_ids.get(&record_ptr).copied() {
                Some(value) => value,
                None => {
                    let value = NEXT_RUNTIME_OBJECT_ID.fetch_add(1, Ordering::AcqRel);
                    if value == 0 {
                        pending.rejected = true;
                        state.lost_or_rejected = true;
                        return;
                    }
                    state.runtime_ids.insert(record_ptr, value);
                    value
                }
            };
            let entity_ptr = usize::try_from(u64::from_le_bytes(
                record_bytes[0x10..0x18].try_into().expect("bounded O05 pointer"),
            ))
            .ok()
            .unwrap_or(0);
            let (selected_entity_instance_id, world_transform_4x4_f32_bits) =
                if entity_ptr == 0 {
                    (None, None)
                } else {
                    (
                        entity_state.entities.get(&entity_ptr).map(|row| row.stable_id),
                        state.entity_matrices.get(&entity_ptr).copied(),
                    )
                };
            if entity_ptr != 0
                && (selected_entity_instance_id.is_none()
                    || world_transform_4x4_f32_bits.is_none())
            {
                pending.rejected = true;
                return;
            }
            pending.records.push(RawO05RuntimeRecordObservation {
                runtime_object_id,
                record_bytes,
                selected_entity_instance_id,
                world_transform_4x4_f32_bits,
            });
        }
    });
}

pub(super) unsafe extern "system" fn hook_entity_transform_setter(
    entity: usize,
    matrix: usize,
) {
    let original: EntityTransformSetterFn =
        unsafe { mem::transmute(ORIGINAL_ENTITY_TRANSFORM_SETTER.load(Ordering::Acquire)) };
    if entity != 0 && matrix != 0 {
        if let Some(bytes) = read_bytes(matrix, 64) {
            let mut values = [0u32; 16];
            for (index, chunk) in bytes.chunks_exact(4).enumerate() {
                values[index] = u32::from_le_bytes(chunk.try_into().expect("four matrix bytes"));
            }
            if let Ok(mut state) = state().lock() {
                state.entity_matrices.insert(entity, values);
            }
        } else if let Ok(mut state) = state().lock() {
            state.lost_or_rejected = true;
        }
    }
    unsafe { original(entity, matrix) };
}

pub(super) fn observe_entity_destroy(entity: usize) {
    if let Ok(mut state) = state().lock() {
        state.entity_matrices.remove(&entity);
    }
}

pub fn take_scope_for_selected_present(
    selected_game_hour_raw: i32,
    selected_qpc_ticks: u64,
    selected_qpc_frequency_hz: u64,
) -> Result<Option<RawO05RuntimeScopeObservation>, String> {
    let mut state = state()
        .lock()
        .map_err(|_| "O05 acquisition state poisoned")?;
    if state.lost_or_rejected {
        return Err("O05 acquisition lost or rejected a runtime scope".into());
    }
    if selected_qpc_ticks == 0 || selected_qpc_frequency_hz == 0 {
        return Err("O05 selected-Present QPC authority is incomplete".into());
    }
    let eligible = state
        .completed
        .iter()
        .enumerate()
        .filter(|(_, scope)| {
            scope.game_hour_raw == selected_game_hour_raw
                && scope.qpc_frequency_hz == selected_qpc_frequency_hz
                && scope.completed_qpc_ticks <= selected_qpc_ticks
        })
        .max_by_key(|(_, scope)| scope.completed_qpc_ticks)
        .map(|(index, _)| index);
    let Some(index) = eligible else {
        state
            .completed
            .retain(|scope| scope.game_hour_raw >= selected_game_hour_raw);
        return Ok(None);
    };
    let selected = state
        .completed
        .remove(index)
        .ok_or("O05 selected scope disappeared")?;
    state.completed.retain(|scope| {
        scope.game_hour_raw > selected_game_hour_raw
            || (scope.game_hour_raw == selected_game_hour_raw
                && scope.completed_qpc_ticks > selected_qpc_ticks)
    });
    Ok(Some(selected))
}

pub(super) fn reset() {
    NEXT_RUNTIME_OBJECT_ID.store(1, Ordering::Release);
    if let Ok(mut state) = state().lock() {
        *state = O05State::default();
    }
    CURRENT_SCOPE.with(|slot| {
        *slot.borrow_mut() = None;
    });
}
