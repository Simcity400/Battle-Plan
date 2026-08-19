use super::{read_i32, read_i64, read_u16, read_u32, read_usize, reject};
use crate::{RawStaticDrawCall, RawVertexBufferObject};
use core::cell::RefCell;
use core::ffi::c_void;
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::mem;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

pub const ROW_PRODUCER_RVA: usize = 0x0164_AA70;
pub const SLOT_SELECTOR_RVA: usize = 0x0164_CCB0;
pub const GAME_STATE_ROOT_RVA: usize = 0x0331_6E90;

pub const ROW_PRODUCER_SENTINEL: [u8; 16] = [
    0x88, 0x54, 0x24, 0x10, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41,
];
pub const SLOT_SELECTOR_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57,
];

const ROW_STRIDE: usize = 0x30;
const MAX_ROWS_PER_CALL: usize = 65_536;
const MAX_PROVINCES_PER_ROW: usize = 65_536;
const MAX_PROVINCES_PER_CALL: usize = 1_048_576;
const ARROW_WRAPPER_BUFFER_OFFSETS: [usize; 4] = [0x48, 0xc8, 0x148, 0x1c8];
const NORMAL_EFFECT_TOKEN: &str = "map_no_head";
const SEE_THROUGH_EFFECT_TOKEN: &str = "map_no_head_see_through";
const INVALID_FIXED5: i64 = 0xffff_ffff;

type RowProducerFn = unsafe extern "system" fn(usize, u8);
type SlotSelectorFn = unsafe extern "system" fn(usize, usize, i32, u8) -> usize;

pub static ORIGINAL_ROW_PRODUCER: AtomicUsize = AtomicUsize::new(0);
pub static ORIGINAL_SLOT_SELECTOR: AtomicUsize = AtomicUsize::new(0);
static NEXT_SELECTOR_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RawRailSupplyArrowPass {
    Normal,
    SeeThrough,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawRailSupplyArrowObservation {
    pub selector_sequence: u64,
    pub row_producer_rva: u32,
    pub slot_selector_rva: u32,
    pub row_ordinal: u32,
    pub construction_mode_u8: u8,
    pub ordered_province_ids: Vec<u32>,
    pub effective_level_i32: i32,
    pub target_level_i32: i32,
    pub cooldown_selector_i32: i32,
    pub flags_u32: u32,
    pub max_damage_fixed5: i64,
    pub width_code_i32: i32,
    pub damaged_u8: u8,
    pub selected_rgba_f32_bits: [u32; 4],
    pub texture_slot_u32: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawRailSupplyArrowDrawJoin {
    pub selector_sequence: u64,
    pub capture_sequence: u64,
    pub pass: RawRailSupplyArrowPass,
    pub first_vertex: u32,
    pub vertex_count: u32,
    pub vertex_buffer_snapshot_sequence: u64,
    pub vertex_buffer_byte_width: u32,
    pub wrapper_buffer_ordinal: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawRailSupplyArrowFrame {
    pub frame_sequence: u64,
    pub submit_index: u64,
    pub rows: Vec<RawRailSupplyArrowObservation>,
    pub draw_joins: Vec<RawRailSupplyArrowDrawJoin>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SelectorInput {
    row_ordinal: u32,
    ordered_province_ids: Vec<u32>,
    effective_level_i32: i32,
    target_level_i32: i32,
    cooldown_selector_i32: i32,
    flags_u32: u32,
    max_damage_fixed5: i64,
}

#[derive(Clone, Debug)]
struct SlotResult {
    arrow_ptr: usize,
    width_code_i32: i32,
    damaged_u8: u8,
    selected_rgba_f32_bits: [u32; 4],
    texture_slot_u32: u32,
}

struct PendingSelector {
    frame_sequence: u64,
    submit_index: u64,
    rows_owner: usize,
    construction_mode_u8: u8,
    rows: Vec<SelectorInput>,
    slots: Vec<SlotResult>,
    rejected: bool,
}

thread_local! {
    static PENDING_SELECTOR: RefCell<Option<PendingSelector>> = const { RefCell::new(None) };
}

#[derive(Clone)]
struct ActiveArrow {
    row_index: usize,
    candidate_vertex_buffers: [Option<usize>; 4],
}

struct FrameState {
    frame_sequence: u64,
    submit_index: u64,
    rejected: bool,
    rows: Vec<RawRailSupplyArrowObservation>,
    arrows: HashMap<usize, ActiveArrow>,
    draw_joins: Vec<RawRailSupplyArrowDrawJoin>,
}

#[derive(Default)]
struct State {
    frame: Option<FrameState>,
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();

fn state() -> &'static Mutex<State> {
    STATE.get_or_init(|| Mutex::new(State::default()))
}

pub fn row_producer_detour() -> *mut c_void {
    hook_row_producer as *mut c_void
}

pub fn slot_selector_detour() -> *mut c_void {
    hook_slot_selector as *mut c_void
}

pub fn begin_frame(frame_sequence: u64, submit_index: u64) -> Result<(), String> {
    if frame_sequence == 0 || submit_index == 0 {
        return Err("rail-supply frame identity is null".into());
    }
    let mut state = state()
        .lock()
        .map_err(|_| "rail-supply state mutex is poisoned")?;
    if state.frame.is_some() {
        return Err("rail-supply frame began while another frame was active".into());
    }
    state.frame = Some(FrameState {
        frame_sequence,
        submit_index,
        rejected: false,
        rows: Vec::new(),
        arrows: HashMap::new(),
        draw_joins: Vec::new(),
    });
    Ok(())
}

pub fn take_frame(
    frame_sequence: u64,
    submit_index: u64,
) -> Result<RawRailSupplyArrowFrame, String> {
    if PENDING_SELECTOR.with(|pending| pending.borrow().is_some()) {
        return Err("rail-supply frame ended inside its selector detour".into());
    }
    let frame = state()
        .lock()
        .map_err(|_| "rail-supply state mutex is poisoned")?
        .frame
        .take()
        .ok_or("rail-supply frame is absent")?;
    if frame.frame_sequence != frame_sequence
        || frame.submit_index != submit_index
        || frame.rejected
    {
        return Err("rail-supply frame is foreign or contains a rejected observation".into());
    }
    let mut raw = RawRailSupplyArrowFrame {
        frame_sequence,
        submit_index,
        rows: frame.rows,
        draw_joins: frame.draw_joins,
    };
    raw.rows.sort_by_key(|row| row.selector_sequence);
    raw.draw_joins.sort_by_key(|join| join.capture_sequence);
    validate_frame(&raw)?;
    Ok(raw)
}

pub fn abort_frame() {
    if let Ok(mut state) = state().lock() {
        state.frame = None;
    } else {
        reject();
    }
    PENDING_SELECTOR.with(|pending| *pending.borrow_mut() = None);
}

pub fn reset() {
    abort_frame();
    ORIGINAL_ROW_PRODUCER.store(0, Ordering::Release);
    ORIGINAL_SLOT_SELECTOR.store(0, Ordering::Release);
    NEXT_SELECTOR_SEQUENCE.store(1, Ordering::Release);
}

pub fn validate_frame(frame: &RawRailSupplyArrowFrame) -> Result<(), String> {
    if frame.frame_sequence == 0 || frame.submit_index == 0 {
        return Err("rail-supply frame identity is null".into());
    }
    let mut sequences = BTreeSet::new();
    for row in &frame.rows {
        let expected_damaged = u8::from(row.flags_u32 & 8 != 0 && row.max_damage_fixed5 > 0);
        let expected_slot = texture_slot(row.width_code_i32, row.damaged_u8)
            .ok_or("rail-supply row has an invalid width/damage selector")?;
        if row.selector_sequence == 0
            || row.row_producer_rva != ROW_PRODUCER_RVA as u32
            || row.slot_selector_rva != SLOT_SELECTOR_RVA as u32
            || row.construction_mode_u8 > 1
            || row.ordered_province_ids.len() < 2
            || row.ordered_province_ids.len() > MAX_PROVINCES_PER_ROW
            || row.ordered_province_ids.iter().any(|id| *id == 0)
            || row.damaged_u8 != expected_damaged
            || row.texture_slot_u32 != expected_slot
            || !sequences.insert(row.selector_sequence)
        {
            return Err(
                "rail-supply row is null, duplicated, or violates the exact selector law".into(),
            );
        }
    }
    let mut pass_keys = BTreeSet::new();
    let mut capture_sequences = BTreeSet::new();
    for join in &frame.draw_joins {
        if !sequences.contains(&join.selector_sequence)
            || join.capture_sequence == 0
            || join.vertex_count == 0
            || join.vertex_buffer_snapshot_sequence != join.capture_sequence
            || join.vertex_buffer_byte_width == 0
            || join.wrapper_buffer_ordinal >= 4
            || !capture_sequences.insert(join.capture_sequence)
            || !pass_keys.insert((join.selector_sequence, join.pass))
        {
            return Err(
                "rail-supply static draw join is incomplete, duplicated, or foreign".into(),
            );
        }
    }
    for sequence in sequences {
        if !pass_keys.contains(&(sequence, RawRailSupplyArrowPass::Normal))
            || !pass_keys.contains(&(sequence, RawRailSupplyArrowPass::SeeThrough))
        {
            return Err("rail-supply row lacks its exact normal/see-through draw pair".into());
        }
    }
    Ok(())
}

pub fn hash_frame(hasher: &mut Sha256, frame: &RawRailSupplyArrowFrame) -> Result<(), String> {
    validate_frame(frame)?;
    hasher.update(frame.frame_sequence.to_le_bytes());
    hasher.update(frame.submit_index.to_le_bytes());
    hasher.update(
        u64::try_from(frame.rows.len())
            .map_err(|_| "rail-supply row count exceeds u64")?
            .to_le_bytes(),
    );
    for row in &frame.rows {
        hasher.update(row.selector_sequence.to_le_bytes());
        hasher.update(row.row_producer_rva.to_le_bytes());
        hasher.update(row.slot_selector_rva.to_le_bytes());
        hasher.update(row.row_ordinal.to_le_bytes());
        hasher.update([row.construction_mode_u8]);
        hasher.update(
            u64::try_from(row.ordered_province_ids.len())
                .map_err(|_| "rail-supply province count exceeds u64")?
                .to_le_bytes(),
        );
        for id in &row.ordered_province_ids {
            hasher.update(id.to_le_bytes());
        }
        hasher.update(row.effective_level_i32.to_le_bytes());
        hasher.update(row.target_level_i32.to_le_bytes());
        hasher.update(row.cooldown_selector_i32.to_le_bytes());
        hasher.update(row.flags_u32.to_le_bytes());
        hasher.update(row.max_damage_fixed5.to_le_bytes());
        hasher.update(row.width_code_i32.to_le_bytes());
        hasher.update([row.damaged_u8]);
        for bits in row.selected_rgba_f32_bits {
            hasher.update(bits.to_le_bytes());
        }
        hasher.update(row.texture_slot_u32.to_le_bytes());
    }
    hasher.update(
        u64::try_from(frame.draw_joins.len())
            .map_err(|_| "rail-supply draw-join count exceeds u64")?
            .to_le_bytes(),
    );
    for join in &frame.draw_joins {
        hasher.update(join.selector_sequence.to_le_bytes());
        hasher.update(join.capture_sequence.to_le_bytes());
        hasher.update([match join.pass {
            RawRailSupplyArrowPass::Normal => 0,
            RawRailSupplyArrowPass::SeeThrough => 1,
        }]);
        hasher.update(join.first_vertex.to_le_bytes());
        hasher.update(join.vertex_count.to_le_bytes());
        hasher.update(join.vertex_buffer_snapshot_sequence.to_le_bytes());
        hasher.update(join.vertex_buffer_byte_width.to_le_bytes());
        hasher.update([join.wrapper_buffer_ordinal]);
    }
    Ok(())
}

pub fn observe_static_draw(
    capture_sequence: u64,
    draw_call: RawStaticDrawCall,
    ordered_vertex_buffers: &[RawVertexBufferObject],
    buffer_snapshots: &[crate::RawBufferSnapshot],
    effect_token: Option<&str>,
) -> Result<(), String> {
    let Some(slot_zero) = exactly_one_slot_zero(ordered_vertex_buffers) else {
        return Ok(());
    };
    let mut state = state()
        .lock()
        .map_err(|_| "rail-supply state mutex is poisoned")?;
    let Some(frame) = state.frame.as_mut() else {
        return Ok(());
    };
    let mut matches = Vec::new();
    for (arrow_ptr, arrow) in &frame.arrows {
        for (ordinal, candidate) in arrow.candidate_vertex_buffers.iter().enumerate() {
            if *candidate == Some(slot_zero.buffer_ptr) {
                matches.push((*arrow_ptr, arrow.row_index, ordinal as u8));
            }
        }
    }
    if matches.is_empty() {
        return Ok(());
    }
    if matches.len() != 1 {
        frame.rejected = true;
        return Err("rail-supply draw matches multiple arrow wrapper buffers".into());
    }
    let (_, row_index, wrapper_buffer_ordinal) = matches[0];
    let pass = match effect_token {
        Some(NORMAL_EFFECT_TOKEN) => RawRailSupplyArrowPass::Normal,
        Some(SEE_THROUGH_EFFECT_TOKEN) => RawRailSupplyArrowPass::SeeThrough,
        _ => {
            frame.rejected = true;
            return Err(
                "rail-supply draw has no authenticated normal/see-through effect token".into(),
            );
        }
    };
    let (vertex_count, first_vertex) = match draw_call {
        RawStaticDrawCall::Draw {
            vertex_count,
            start_vertex,
        } if vertex_count != 0 => (vertex_count, start_vertex),
        _ => {
            frame.rejected = true;
            return Err("rail-supply arrow used a non-Draw or empty final submission".into());
        }
    };
    let snapshots = buffer_snapshots
        .iter()
        .filter(|snapshot| snapshot.buffer_ptr == slot_zero.buffer_ptr)
        .collect::<Vec<_>>();
    if snapshots.len() != 1 || snapshots[0].snapshot_sequence != capture_sequence {
        frame.rejected = true;
        return Err("rail-supply vertex buffer lacks one same-draw authenticated snapshot".into());
    }
    let selector_sequence = frame
        .rows
        .get(row_index)
        .ok_or("rail-supply draw points outside the active row table")?
        .selector_sequence;
    if frame.draw_joins.iter().any(|join| {
        join.capture_sequence == capture_sequence
            || (join.selector_sequence == selector_sequence && join.pass == pass)
    }) {
        frame.rejected = true;
        return Err("rail-supply row has a duplicate pass or capture sequence".into());
    }
    frame.draw_joins.push(RawRailSupplyArrowDrawJoin {
        selector_sequence,
        capture_sequence,
        pass,
        first_vertex,
        vertex_count,
        vertex_buffer_snapshot_sequence: snapshots[0].snapshot_sequence,
        vertex_buffer_byte_width: snapshots[0].byte_width,
        wrapper_buffer_ordinal,
    });
    Ok(())
}

fn exactly_one_slot_zero(rows: &[RawVertexBufferObject]) -> Option<&RawVertexBufferObject> {
    let mut matches = rows
        .iter()
        .filter(|row| row.input_slot == 0 && row.buffer_ptr != 0);
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

unsafe extern "system" fn hook_row_producer(rows_owner: usize, construction_mode_u8: u8) {
    let original_address = ORIGINAL_ROW_PRODUCER.load(Ordering::Acquire);
    if original_address == 0 {
        reject();
        return;
    }
    let original: RowProducerFn = unsafe { mem::transmute(original_address) };
    let frame_identity = current_frame_identity();
    let mut armed = false;
    let mut local_rejection = false;
    if let Some((frame_sequence, submit_index)) = frame_identity {
        match snapshot_rows(rows_owner) {
            Some(rows) if construction_mode_u8 <= 1 => {
                PENDING_SELECTOR.with(|pending| {
                    let mut pending = pending.borrow_mut();
                    if pending.is_some() {
                        local_rejection = true;
                    } else {
                        *pending = Some(PendingSelector {
                            frame_sequence,
                            submit_index,
                            rows_owner,
                            construction_mode_u8,
                            rows,
                            slots: Vec::new(),
                            rejected: false,
                        });
                        armed = true;
                    }
                });
            }
            _ => local_rejection = true,
        }
    }
    unsafe { original(rows_owner, construction_mode_u8) };
    if !armed {
        if local_rejection {
            reject_current_frame();
            reject();
        }
        return;
    }
    let pending = PENDING_SELECTOR.with(|pending| pending.borrow_mut().take());
    let result = pending
        .ok_or(())
        .and_then(finalize_selector)
        .and_then(commit_selector);
    if result.is_err() {
        reject_current_frame();
        reject();
    }
}

unsafe extern "system" fn hook_slot_selector(
    pool: usize,
    ccolor: usize,
    width_code_i32: i32,
    damaged_u8: u8,
) -> usize {
    let original_address = ORIGINAL_SLOT_SELECTOR.load(Ordering::Acquire);
    if original_address == 0 {
        reject();
        return 0;
    }
    let rgba = read_rgba(ccolor);
    let original: SlotSelectorFn = unsafe { mem::transmute(original_address) };
    let arrow_ptr = unsafe { original(pool, ccolor, width_code_i32, damaged_u8) };
    PENDING_SELECTOR.with(|pending| {
        let mut pending = pending.borrow_mut();
        let Some(pending) = pending.as_mut() else {
            return;
        };
        let observed_texture_slot = arrow_ptr
            .checked_add(0x204)
            .and_then(read_i32)
            .and_then(|value| u32::try_from(value).ok());
        match (rgba, observed_texture_slot) {
            (Some(selected_rgba_f32_bits), Some(texture_slot_u32))
                if arrow_ptr != 0
                    && matches!(width_code_i32, 1..=3)
                    && damaged_u8 <= 1
                    && texture_slot(width_code_i32, damaged_u8) == Some(texture_slot_u32)
                    && pending.slots.len() < MAX_ROWS_PER_CALL =>
            {
                pending.slots.push(SlotResult {
                    arrow_ptr,
                    width_code_i32,
                    damaged_u8,
                    selected_rgba_f32_bits,
                    texture_slot_u32,
                });
            }
            _ => pending.rejected = true,
        }
    });
    arrow_ptr
}

fn current_frame_identity() -> Option<(u64, u64)> {
    state()
        .lock()
        .ok()?
        .frame
        .as_ref()
        .and_then(|frame| (!frame.rejected).then_some((frame.frame_sequence, frame.submit_index)))
}

fn reject_current_frame() {
    if let Ok(mut state) = state().lock() {
        if let Some(frame) = state.frame.as_mut() {
            frame.rejected = true;
        }
    }
}

fn finalize_selector(pending: PendingSelector) -> Result<(PendingSelector, Vec<ActiveArrow>), ()> {
    if pending.rejected || pending.rows.len() != pending.slots.len() {
        return Err(());
    }
    let after = snapshot_rows(pending.rows_owner).ok_or(())?;
    if after != pending.rows {
        return Err(());
    }
    let data = read_usize(pending.rows_owner).ok_or(())?;
    let mut arrows = Vec::with_capacity(pending.rows.len());
    let mut unique_arrows = BTreeSet::new();
    for (index, (row, slot)) in pending.rows.iter().zip(&pending.slots).enumerate() {
        let row_address = data
            .checked_add(index.checked_mul(ROW_STRIDE).ok_or(())?)
            .ok_or(())?;
        if read_usize(row_address) != Some(slot.arrow_ptr)
            || !unique_arrows.insert(slot.arrow_ptr)
            || slot.damaged_u8 != u8::from(row.flags_u32 & 8 != 0 && row.max_damage_fixed5 > 0)
        {
            return Err(());
        }
        let wrapper = read_usize(slot.arrow_ptr.checked_add(0x48).ok_or(())?).ok_or(())?;
        if wrapper == 0 {
            return Err(());
        }
        let mut candidate_vertex_buffers = [None; 4];
        let mut unique_buffers = BTreeSet::new();
        for (ordinal, offset) in ARROW_WRAPPER_BUFFER_OFFSETS.iter().copied().enumerate() {
            let pointer = read_usize(wrapper.checked_add(offset).ok_or(())?).ok_or(())?;
            if pointer != 0 {
                if !unique_buffers.insert(pointer) {
                    return Err(());
                }
                candidate_vertex_buffers[ordinal] = Some(pointer);
            }
        }
        if unique_buffers.is_empty() {
            return Err(());
        }
        arrows.push(ActiveArrow {
            row_index: index,
            candidate_vertex_buffers,
        });
    }
    Ok((pending, arrows))
}

fn commit_selector((pending, arrows): (PendingSelector, Vec<ActiveArrow>)) -> Result<(), ()> {
    let mut state = state().lock().map_err(|_| ())?;
    let frame = state.frame.as_mut().ok_or(())?;
    if frame.rejected
        || frame.frame_sequence != pending.frame_sequence
        || frame.submit_index != pending.submit_index
        || frame.rows.len().checked_add(pending.rows.len()).ok_or(())? > MAX_ROWS_PER_CALL
    {
        return Err(());
    }
    for ((input, slot), active) in pending.rows.into_iter().zip(pending.slots).zip(arrows) {
        let selector_sequence = next_selector_sequence().ok_or(())?;
        let row_index = frame.rows.len();
        let raw = RawRailSupplyArrowObservation {
            selector_sequence,
            row_producer_rva: ROW_PRODUCER_RVA as u32,
            slot_selector_rva: SLOT_SELECTOR_RVA as u32,
            row_ordinal: input.row_ordinal,
            construction_mode_u8: pending.construction_mode_u8,
            ordered_province_ids: input.ordered_province_ids,
            effective_level_i32: input.effective_level_i32,
            target_level_i32: input.target_level_i32,
            cooldown_selector_i32: input.cooldown_selector_i32,
            flags_u32: input.flags_u32,
            max_damage_fixed5: input.max_damage_fixed5,
            width_code_i32: slot.width_code_i32,
            damaged_u8: slot.damaged_u8,
            selected_rgba_f32_bits: slot.selected_rgba_f32_bits,
            texture_slot_u32: slot.texture_slot_u32,
        };
        let mut active = active;
        active.row_index = row_index;
        if frame.arrows.insert(slot.arrow_ptr, active).is_some() {
            frame.rejected = true;
            return Err(());
        }
        frame.rows.push(raw);
    }
    Ok(())
}

fn next_selector_sequence() -> Option<u64> {
    NEXT_SELECTOR_SEQUENCE
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
            value.checked_add(1)
        })
        .ok()
        .filter(|value| *value != 0)
}

fn snapshot_rows(rows_owner: usize) -> Option<Vec<SelectorInput>> {
    if rows_owner == 0 {
        return None;
    }
    let data = read_usize(rows_owner)?;
    let count = usize::try_from(read_i32(rows_owner.checked_add(0x0c)?)?).ok()?;
    if count > MAX_ROWS_PER_CALL || (count != 0 && data == 0) {
        return None;
    }
    let province_vector = province_vector_base()?;
    let mut total_provinces = 0usize;
    let mut rows = Vec::with_capacity(count);
    for ordinal in 0..count {
        let row = data.checked_add(ordinal.checked_mul(ROW_STRIDE)?)?;
        let province_ids_ptr = read_usize(row.checked_add(0x08)?)?;
        let province_count = usize::try_from(read_i32(row.checked_add(0x14)?)?).ok()?;
        if !(2..=MAX_PROVINCES_PER_ROW).contains(&province_count) || province_ids_ptr == 0 {
            return None;
        }
        total_provinces = total_provinces.checked_add(province_count)?;
        if total_provinces > MAX_PROVINCES_PER_CALL {
            return None;
        }
        let mut ordered_province_ids = Vec::with_capacity(province_count);
        let mut max_damage_fixed5 = 0i64;
        for index in 0..province_count {
            let id = read_u32(province_ids_ptr.checked_add(index.checked_mul(4)?)?)?;
            if id == 0 {
                return None;
            }
            ordered_province_ids.push(id);
            max_damage_fixed5 = max_damage_fixed5.max(province_damage_fixed5(province_vector, id)?);
        }
        rows.push(SelectorInput {
            row_ordinal: u32::try_from(ordinal).ok()?,
            ordered_province_ids,
            effective_level_i32: read_i32(row.checked_add(0x20)?)?,
            target_level_i32: read_i32(row.checked_add(0x24)?)?,
            cooldown_selector_i32: read_i32(row.checked_add(0x28)?)?,
            flags_u32: read_u32(row.checked_add(0x2c)?)?,
            max_damage_fixed5,
        });
    }
    Some(rows)
}

fn province_vector_base() -> Option<usize> {
    let base = super::MODULE_BASE.load(Ordering::Acquire);
    let game_state = read_usize(base.checked_add(GAME_STATE_ROOT_RVA)?)?;
    let manager = read_usize(game_state.checked_add(0x3e0)?)?;
    let vector = read_usize(manager.checked_add(0x08)?)?;
    (base != 0 && game_state != 0 && manager != 0 && vector != 0).then_some(vector)
}

fn province_damage_fixed5(province_vector: usize, province_id: u32) -> Option<i64> {
    let province =
        read_usize(province_vector.checked_add((province_id as usize).checked_mul(8)?)?)?;
    if province == 0 || read_i32(province.checked_add(0x68)?)? > 0 {
        return Some(0);
    }
    let health = read_usize(province.checked_add(0x18)?)?;
    let health_fixed5 = if health == 0 {
        0
    } else {
        exact_health_fixed5(health)?
    };
    Some(100_000i64.wrapping_sub(health_fixed5))
}

fn exact_health_fixed5(health: usize) -> Option<i64> {
    let current = read_u16(health.checked_add(0x40)?)? as i16 as i64;
    if current == 0 {
        return Some(0);
    }
    let completed = read_u16(health.checked_add(0x42)?)? as i16 as i64;
    if current == completed {
        return Some(100_000);
    }
    let backing = read_i64(health.checked_add(0x48)?)?;
    let normalized = if fixed5_value_is_finite(backing) {
        let scaled = backing.wrapping_mul(100_000);
        let magic = -0x2940_6b2a_1a85_bd43i64;
        let high = (((magic as i128) * (scaled as i128)) >> 64) as i64;
        let folded = high.wrapping_add(scaled);
        (folded >> 23).wrapping_sub(folded >> 63)
    } else {
        INVALID_FIXED5
    };
    let denominator = current.wrapping_mul(100_000);
    if denominator == 0 {
        return Some(INVALID_FIXED5);
    }
    let numerator = normalized.wrapping_add(completed.wrapping_mul(100_000));
    if !fixed5_value_is_finite(numerator) {
        return Some(INVALID_FIXED5);
    }
    Some(numerator.wrapping_mul(100_000) / denominator)
}

fn fixed5_value_is_finite(value: i64) -> bool {
    value != i64::MAX && value != i64::MIN && value != i64::MIN + 1
}

fn read_rgba(ccolor: usize) -> Option<[u32; 4]> {
    Some([
        read_u32(ccolor.checked_add(0x10)?)?,
        read_u32(ccolor.checked_add(0x14)?)?,
        read_u32(ccolor.checked_add(0x18)?)?,
        read_u32(ccolor.checked_add(0x1c)?)?,
    ])
}

fn texture_slot(width_code_i32: i32, damaged_u8: u8) -> Option<u32> {
    if damaged_u8 > 1 {
        return None;
    }
    let base = match width_code_i32 {
        1 => 1,
        2 => 0,
        3 => 2,
        _ => return None,
    };
    Some(base + if damaged_u8 == 0 { 0 } else { 3 })
}
