use super::{
    inside_non_ui_render_phase, read_i32, read_u32, read_u32x16, read_usize,
    reject_render_order_event, RawS1DrawExecutionRef,
};
use core::cell::RefCell;
use core::ffi::c_void;
use std::collections::VecDeque;
use std::mem;
use std::sync::atomic::{AtomicUsize, Ordering};

pub const PRODUCER_RVA: usize = 0x0226_4CC0;
const OUTER_LISTS_RVA: usize = 0x0344_FEF0;
const PARTITION_SLOT_STRIDE: usize = 0x18;
const OUTER_ROW_STRIDE: usize = 0x50;
const DRAW_ROW_STRIDE: usize = 0x108;
const MATERIAL_ROW_STRIDE: usize = 0x60;
const MAX_OUTER_ROWS: u32 = 1_048_576;
const MAX_INNER_ROWS: u32 = 1_048_576;

pub const PRODUCER_SENTINEL: [u8; 16] = [
    0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x56, 0x48, 0x8d, 0xac, 0x24, 0x20, 0xfe, 0xff, 0xff, 0x48,
];

type ProducerFn = unsafe extern "system" fn(usize, usize, usize, usize, i32);

pub static ORIGINAL_PRODUCER: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawS1GenericPdxmeshCasterObservation {
    pub execution: RawS1DrawExecutionRef,
    pub partition_mask: u8,
    pub object_ordinal: u32,
    pub group_ordinal: u32,
    pub submesh_ordinal: u32,
    pub entity_object_ptr: usize,
    pub mesh_draw_ptr: usize,
    pub material_binding_ptr: usize,
    pub mesh_draw_skinned: bool,
    pub shader_program_ptr: usize,
    pub world_matrix_f32_bits: [u32; 16],
    pub vertex_count: u32,
    pub start_vertex: u32,
}

#[derive(Clone, Debug)]
struct PendingCaster {
    partition_mask: u8,
    object_ordinal: u32,
    group_ordinal: u32,
    submesh_ordinal: u32,
    entity_object_ptr: usize,
    mesh_draw_ptr: usize,
    material_binding_ptr: usize,
    mesh_draw_skinned: bool,
    world_matrix_f32_bits: [u32; 16],
}

#[derive(Default)]
struct PendingCall {
    rows: VecDeque<PendingCaster>,
    rejected: bool,
}

thread_local! {
    static CURRENT_CALL: RefCell<Option<PendingCall>> = const { RefCell::new(None) };
}

pub fn detour() -> *mut c_void {
    hook_producer as *mut c_void
}

pub fn reset() {
    CURRENT_CALL.with(|slot| *slot.borrow_mut() = None);
}

pub fn take_final_draw(
    execution: RawS1DrawExecutionRef,
    shader_program_ptr: usize,
    vertex_count: i32,
    start_vertex: u32,
) -> Result<Option<RawS1GenericPdxmeshCasterObservation>, ()> {
    CURRENT_CALL.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(call) = slot.as_mut() else {
            return Ok(None);
        };
        let Ok(vertex_count) = u32::try_from(vertex_count) else {
            call.rejected = true;
            return Err(());
        };
        let Some(row) = call.rows.pop_front() else {
            call.rejected = true;
            return Err(());
        };
        if shader_program_ptr == 0 {
            call.rejected = true;
            return Err(());
        }
        Ok(Some(RawS1GenericPdxmeshCasterObservation {
            execution,
            partition_mask: row.partition_mask,
            object_ordinal: row.object_ordinal,
            group_ordinal: row.group_ordinal,
            submesh_ordinal: row.submesh_ordinal,
            entity_object_ptr: row.entity_object_ptr,
            mesh_draw_ptr: row.mesh_draw_ptr,
            material_binding_ptr: row.material_binding_ptr,
            mesh_draw_skinned: row.mesh_draw_skinned,
            shader_program_ptr,
            world_matrix_f32_bits: row.world_matrix_f32_bits,
            vertex_count,
            start_vertex,
        }))
    })
}

unsafe extern "system" fn hook_producer(
    owner: usize,
    render_context: usize,
    view_projection: usize,
    pass_context: usize,
    partition_index: i32,
) {
    let observe = inside_non_ui_render_phase();
    if observe {
        let nested = CURRENT_CALL.with(|slot| slot.borrow().is_some());
        let pending = capture_pending(partition_index);
        if nested || pending.is_none() {
            reject_render_order_event();
        }
        CURRENT_CALL.with(|slot| {
            *slot.borrow_mut() = pending.or(Some(PendingCall {
                rows: VecDeque::new(),
                rejected: true,
            }))
        });
    }

    let original: ProducerFn = unsafe { mem::transmute(ORIGINAL_PRODUCER.load(Ordering::Acquire)) };
    unsafe {
        original(
            owner,
            render_context,
            view_projection,
            pass_context,
            partition_index,
        )
    };

    if observe {
        let complete = CURRENT_CALL.with(|slot| {
            slot.borrow_mut()
                .take()
                .is_some_and(|call| !call.rejected && call.rows.is_empty())
        });
        if !complete {
            reject_render_order_event();
        }
    }
}

fn capture_pending(partition_index: i32) -> Option<PendingCall> {
    let partition_index = u32::try_from(partition_index).ok()?;
    if partition_index > 2 {
        return None;
    }
    let partition_mask = 1u8.checked_shl(partition_index)?;
    let slot = super::MODULE_BASE
        .load(Ordering::Acquire)
        .checked_add(OUTER_LISTS_RVA)?
        .checked_add((partition_index as usize).checked_mul(PARTITION_SLOT_STRIDE)?)?;
    let outer_base = read_usize(slot)?;
    let outer_count = u32::try_from(read_i32(slot.checked_add(0x0c)?)?).ok()?;
    if outer_count > MAX_OUTER_ROWS || (outer_count != 0 && outer_base == 0) {
        return None;
    }

    let mut rows = VecDeque::new();
    for object_ordinal in 0..outer_count {
        let outer =
            outer_base.checked_add((object_ordinal as usize).checked_mul(OUTER_ROW_STRIDE)?)?;
        let group_ordinal = u32::try_from(read_i32(outer)?).ok()?;
        let entity_object_ptr = read_usize(outer.checked_add(8)?)?;
        let world_matrix_f32_bits = read_u32x16(outer.checked_add(0x10)?)?;
        if entity_object_ptr == 0
            || world_matrix_f32_bits
                .iter()
                .any(|bits| !f32::from_bits(*bits).is_finite())
        {
            return None;
        }
        let mesh_owner = read_usize(entity_object_ptr.checked_add(0x58)?)?;
        let binding_lists = read_usize(entity_object_ptr.checked_add(0xb8)?)?;
        let draw_lists = read_usize(mesh_owner.checked_add(0x48)?)?;
        let group_offset = (group_ordinal as usize).checked_mul(0x18)?;
        let draw_group = draw_lists.checked_add(group_offset)?;
        let binding_group = binding_lists.checked_add(group_offset)?;
        let draw_base = read_usize(draw_group)?;
        let draw_count = read_u32(draw_group.checked_add(0x0c)?)?;
        let binding_base = read_usize(binding_group)?;
        if draw_count > MAX_INNER_ROWS || (draw_count != 0 && (draw_base == 0 || binding_base == 0))
        {
            return None;
        }
        for submesh_ordinal in 0..draw_count {
            let mesh_draw_ptr =
                draw_base.checked_add((submesh_ordinal as usize).checked_mul(DRAW_ROW_STRIDE)?)?;
            let material_binding_ptr = binding_base
                .checked_add((submesh_ordinal as usize).checked_mul(MATERIAL_ROW_STRIDE)?)?;
            let mesh_draw_skinned = read_usize(mesh_draw_ptr.checked_add(0x40)?)? != 0;
            rows.push_back(PendingCaster {
                partition_mask,
                object_ordinal,
                group_ordinal,
                submesh_ordinal,
                entity_object_ptr,
                mesh_draw_ptr,
                material_binding_ptr,
                mesh_draw_skinned,
                world_matrix_f32_bits,
            });
        }
    }
    Some(PendingCall {
        rows,
        rejected: false,
    })
}
