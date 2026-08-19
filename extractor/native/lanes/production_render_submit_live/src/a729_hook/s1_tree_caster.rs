use super::{
    active_s1_tree_identity, inside_non_ui_render_phase, read_i32, read_u16, read_u32, read_u8,
    read_usize, reject_render_order_event, retain_s1_tree_rows,
};
use std::cell::RefCell;
use std::cmp::Ordering;
use std::ffi::c_void;
use std::mem;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

pub const PRODUCER_RVA: usize = 0x0126_5C10;
pub const PRODUCER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57,
];

type ProducerFn = unsafe extern "system" fn(usize, usize, u8);

pub static ORIGINAL_PRODUCER: AtomicUsize = AtomicUsize::new(0);

const MAX_SOURCE_ROWS: usize = 4_096;
const MAX_LOD_ROWS: usize = 64;
const MAX_NESTED_CALLS: usize = 4;
const EXCLUDED_DISTANCE_F32_BITS: u32 = 0x7f7f_ffff;
const VALID_PARTITION_MASK: u8 = 0x01 | 0x02 | 0x04;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawS1TreeCasterObservation {
    pub frame_sequence: u64,
    pub submit_index: u64,
    pub partition_mask: u8,
    pub source_row_ordinal: u32,
    pub sort_byte: u8,
    pub sort_distance_f32_bits: u32,
    pub selected_lod_index: u32,
    pub source_instance_count: u32,
    pub source_row_ptr: usize,
    pub selected_model_ptr: usize,
    pub selected_mesh_draw_ptr: usize,
    pub selected_material_effect_key_u32: u32,
    pub final_draw_sequence: u64,
}

#[derive(Clone, Copy)]
struct PendingTreeCaster {
    row: RawS1TreeCasterObservation,
}

struct ActiveTreeCall {
    frame_sequence: u64,
    submit_index: u64,
    render_context_ptr: usize,
    rows: Vec<PendingTreeCaster>,
    next_draw: usize,
    invalid: bool,
}

thread_local! {
    static ACTIVE_TREE_CALLS: RefCell<Vec<ActiveTreeCall>> = const { RefCell::new(Vec::new()) };
}

fn checked_address(base: usize, offset: usize) -> Result<usize, String> {
    base.checked_add(offset)
        .ok_or_else(|| "S1 tree caster address overflow".into())
}

fn selected_lod_and_draw(
    owner_ptr: usize,
    category: u8,
    distance: f32,
) -> Result<Option<(u32, usize, usize, u32)>, String> {
    let model_slot = checked_address(
        owner_ptr,
        0x4b38usize
            .checked_add(usize::from(category) * 8)
            .ok_or("S1 tree model-slot offset overflow")?,
    )?;
    let model_ptr = read_usize(model_slot).ok_or("S1 tree model pointer is unreadable")?;
    if model_ptr == 0 {
        return Ok(None);
    }

    let threshold_count = read_i32(checked_address(model_ptr, 0x144)?)
        .ok_or("S1 tree LOD threshold count is unreadable")?;
    if threshold_count < 0 || threshold_count as usize > MAX_LOD_ROWS {
        return Err("S1 tree LOD threshold count is out of bounds".into());
    }
    let thresholds_ptr = read_usize(checked_address(model_ptr, 0x138)?)
        .ok_or("S1 tree LOD threshold pointer is unreadable")?;
    if threshold_count != 0 && thresholds_ptr == 0 {
        return Err("S1 tree LOD threshold vector is null".into());
    }

    let mut selected_lod = 0u32;
    let mut prior_threshold = None;
    for ordinal in 0..threshold_count as usize {
        let bits = read_u32(checked_address(thresholds_ptr, ordinal * 4)?)
            .ok_or("S1 tree LOD threshold is unreadable")?;
        let threshold = f32::from_bits(bits);
        if !threshold.is_finite() || prior_threshold.is_some_and(|prior: f32| threshold < prior) {
            return Err("S1 tree LOD thresholds are non-finite or reordered".into());
        }
        prior_threshold = Some(threshold);
        if distance < threshold {
            break;
        }
        selected_lod = ordinal as u32;
    }

    let lod_count = read_i32(checked_address(model_ptr, 0x54)?)
        .ok_or("S1 tree model LOD count is unreadable")?;
    if lod_count <= 0 || lod_count as usize > MAX_LOD_ROWS {
        return Err("S1 tree model LOD count is out of bounds".into());
    }
    selected_lod = selected_lod.min((lod_count - 1) as u32);

    let draw_lists_ptr = read_usize(checked_address(model_ptr, 0x48)?)
        .ok_or("S1 tree model draw-list pointer is unreadable")?;
    if draw_lists_ptr == 0 {
        return Err("S1 tree model draw-list vector is null".into());
    }
    let lod_header = checked_address(draw_lists_ptr, selected_lod as usize * 0x18)?;
    let draw_count = read_i32(checked_address(lod_header, 0x0c)?)
        .ok_or("S1 tree selected LOD draw count is unreadable")?;
    if draw_count <= 0 {
        return Ok(None);
    }
    let draw_base = read_usize(lod_header).ok_or("S1 tree selected LOD draw base is unreadable")?;
    if draw_base == 0 {
        return Err("S1 tree selected LOD draw base is null".into());
    }
    let mesh_draw_ptr = draw_base;
    let material_effect_key = read_u32(checked_address(mesh_draw_ptr, 0x24)?)
        .ok_or("S1 tree material effect key is unreadable")?;
    Ok(Some((
        selected_lod,
        model_ptr,
        mesh_draw_ptr,
        material_effect_key,
    )))
}

fn precompute_rows(
    frame_sequence: u64,
    submit_index: u64,
    owner_ptr: usize,
    partition_mask: u8,
) -> Result<Vec<PendingTreeCaster>, String> {
    if frame_sequence == 0
        || submit_index == 0
        || owner_ptr == 0
        || partition_mask == 0
        || partition_mask.count_ones() != 1
        || partition_mask & !VALID_PARTITION_MASK != 0
    {
        return Err("S1 tree caster identity or partition mask is invalid".into());
    }
    let source_base = read_usize(checked_address(owner_ptr, 0x5330)?)
        .ok_or("S1 tree source base is unreadable")?;
    let source_count = read_i32(checked_address(owner_ptr, 0x533c)?)
        .ok_or("S1 tree source count is unreadable")?;
    if source_count < 0 || source_count as usize > MAX_SOURCE_ROWS {
        return Err("S1 tree source count is out of bounds".into());
    }
    if source_count != 0 && source_base == 0 {
        return Err("S1 tree source vector is null".into());
    }

    let mut rows = Vec::new();
    for source_row_ordinal in 0..source_count as usize {
        let source_row_ptr = checked_address(source_base, source_row_ordinal * 0x10)?;
        let category = read_u8(checked_address(source_row_ptr, 0x0a)?)
            .ok_or("S1 tree source category is unreadable")?;
        let state_index = read_u8(checked_address(source_row_ptr, 0x0b)?)
            .ok_or("S1 tree source state index is unreadable")?;
        let state_offset = usize::from(state_index) * 8;
        let state_mask = read_u8(checked_address(owner_ptr, 0x5348 + state_offset)?)
            .ok_or("S1 tree source state mask is unreadable")?;
        let distance_bits = read_u32(checked_address(owner_ptr, 0x534c + state_offset)?)
            .ok_or("S1 tree source distance is unreadable")?;
        if distance_bits == EXCLUDED_DISTANCE_F32_BITS || state_mask & partition_mask == 0 {
            continue;
        }
        let distance = f32::from_bits(distance_bits);
        if !distance.is_finite() {
            return Err("S1 tree included distance is non-finite".into());
        }
        let Some((selected_lod, model_ptr, mesh_draw_ptr, material_effect_key)) =
            selected_lod_and_draw(owner_ptr, category, distance)?
        else {
            continue;
        };
        let source_instance_count = read_u16(checked_address(source_row_ptr, 0x08)?)
            .ok_or("S1 tree source instance count is unreadable")?;
        if source_instance_count == 0 {
            return Err("S1 tree submitting row has zero instances".into());
        }
        rows.push(PendingTreeCaster {
            row: RawS1TreeCasterObservation {
                frame_sequence,
                submit_index,
                partition_mask,
                source_row_ordinal: source_row_ordinal as u32,
                sort_byte: category,
                sort_distance_f32_bits: distance_bits,
                selected_lod_index: selected_lod,
                source_instance_count: u32::from(source_instance_count),
                source_row_ptr,
                selected_model_ptr: model_ptr,
                selected_mesh_draw_ptr: mesh_draw_ptr,
                selected_material_effect_key_u32: material_effect_key,
                final_draw_sequence: 0,
            },
        });
    }

    rows.sort_by(|left, right| {
        left.row.sort_byte.cmp(&right.row.sort_byte).then_with(|| {
            f32::from_bits(left.row.sort_distance_f32_bits)
                .partial_cmp(&f32::from_bits(right.row.sort_distance_f32_bits))
                .expect("included S1 tree distances were checked finite")
        })
    });
    if rows.len() >= 33
        && rows.windows(2).any(|pair| {
            pair[0].row.sort_byte == pair[1].row.sort_byte
                && f32::from_bits(pair[0].row.sort_distance_f32_bits)
                    .partial_cmp(&f32::from_bits(pair[1].row.sort_distance_f32_bits))
                    == Some(Ordering::Equal)
        })
    {
        return Err(
            "S1 tree sort has an equivalent-key pair whose engine order is not portable".into(),
        );
    }
    Ok(rows)
}

pub fn begin_tree_caster_call(
    frame_sequence: u64,
    submit_index: u64,
    owner_ptr: usize,
    render_context_ptr: usize,
    partition_mask: u8,
) -> Result<(), String> {
    if render_context_ptr == 0 {
        return Err("S1 tree render context is null".into());
    }
    let rows = precompute_rows(frame_sequence, submit_index, owner_ptr, partition_mask)?;
    ACTIVE_TREE_CALLS.with(|active| {
        let mut active = active.borrow_mut();
        if active.len() >= MAX_NESTED_CALLS {
            return Err("S1 tree caster call nesting exceeds its bound".into());
        }
        active.push(ActiveTreeCall {
            frame_sequence,
            submit_index,
            render_context_ptr,
            rows,
            next_draw: 0,
            invalid: false,
        });
        Ok(())
    })
}

pub fn observe_tree_final_draw(sequence: u64, render_context_ptr: usize, instance_count: u32) {
    ACTIVE_TREE_CALLS.with(|active| {
        let mut active = active.borrow_mut();
        let Some(call) = active.last_mut() else {
            return;
        };
        if sequence == 0
            || render_context_ptr != call.render_context_ptr
            || call.next_draw >= call.rows.len()
        {
            call.invalid = true;
            return;
        }
        let pending = &mut call.rows[call.next_draw].row;
        if pending.source_instance_count != instance_count {
            call.invalid = true;
            return;
        }
        pending.final_draw_sequence = sequence;
        call.next_draw += 1;
    });
}

pub fn finish_tree_caster_call(
    frame_sequence: u64,
    submit_index: u64,
) -> Result<Vec<RawS1TreeCasterObservation>, String> {
    ACTIVE_TREE_CALLS.with(|active| {
        let mut active = active.borrow_mut();
        let call = active.pop().ok_or("S1 tree caster call stack underflow")?;
        if call.frame_sequence != frame_sequence || call.submit_index != submit_index {
            return Err("S1 tree caster call crosses its frame/submit identity".into());
        }
        if call.invalid
            || call.next_draw != call.rows.len()
            || call.rows.iter().any(|row| row.row.final_draw_sequence == 0)
        {
            return Err("S1 tree caster rows do not match the exact final-draw sequence".into());
        }
        Ok(call.rows.into_iter().map(|row| row.row).collect())
    })
}

pub fn reset_tree_caster_calls() {
    ACTIVE_TREE_CALLS.with(|active| active.borrow_mut().clear());
}

pub fn detour() -> *mut c_void {
    hook_tree_caster_producer as *mut c_void
}

unsafe extern "system" fn hook_tree_caster_producer(
    owner: usize,
    render_context: usize,
    partition_mask: u8,
) {
    let observe = inside_non_ui_render_phase();
    let identity = observe.then(active_s1_tree_identity).flatten();
    let began = if observe {
        let began = identity.is_some_and(|(frame_sequence, submit_index)| {
            begin_tree_caster_call(
                frame_sequence,
                submit_index,
                owner,
                render_context,
                partition_mask,
            )
            .is_ok()
        });
        if !began {
            reject_render_order_event();
        }
        began
    } else {
        false
    };

    let original: ProducerFn =
        unsafe { mem::transmute(ORIGINAL_PRODUCER.load(AtomicOrdering::Acquire)) };
    unsafe { original(owner, render_context, partition_mask) };

    if began {
        let (frame_sequence, submit_index) = identity.expect("a begun S1 tree call has identity");
        match finish_tree_caster_call(frame_sequence, submit_index) {
            Ok(rows) => {
                if !retain_s1_tree_rows(rows) {
                    reject_render_order_event();
                }
            }
            Err(_) => reject_render_order_event(),
        }
    }
}
