//! Ownership is process-local and mechanical: each GUI draw's runtime widget
//! Focus names and rectangle overlap are never ownership inputs.

use core::ffi::c_void;
use focus_research_decisions_depth_live::a729_focus_item_hook::{
    FocusSelectedPresentOperations, NationalFocusCanvasRuntimeGeometry,
    PendingNationalFocusGuiJoin, SelectedPresentCanvasBinding,
};
use portable_render_stream::{AuthenticatedUiPass, UiPassClip, UiPassOperation, UiPassRect};
use production_render_submit_live_lane::a729_hook::SelectedPresentGuiDrawCapture;
use std::collections::{BTreeMap, BTreeSet};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

const RUNTIME_WIDGET_PARENT_OFFSET: usize = 0x108;
const MAX_RUNTIME_PARENT_DEPTH: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NationalFocusCanvasSelectedMetadata {
    pub selected_present_candidate_id: u64,
    pub selected_present_ordinal: u32,
    pub selected_present_qpc_ticks: u64,
    pub selected_present_cutoff_qpc_ticks: u64,
    pub qpc_frequency_hz: u64,
    pub client_extent_pixels: [i32; 2],
}

pub fn national_focus_selected_metadata(
    selected: &production_render_submit_live_lane::a729_hook::RawSelectedRenderFrameCapture,
) -> Result<NationalFocusCanvasSelectedMetadata, String> {
    production_render_submit_live_lane::a729_hook::validate_selected_render_frame_receipt(
        selected,
    )?;
    let selected_present_ordinal = selected
        .receipt
        .candidate_count
        .checked_sub(1)
        .ok_or("A31 selected-Present candidate set is empty")?;
    let client = selected
        .selected
        .frame
        .foreground_pointer_sample
        .as_ref()
        .ok_or("A31 selected Present has no authenticated client-extent sample")?;
    let client_extent_pixels = [
        i32::try_from(client.client_extent[0])
            .map_err(|_| "A31 selected client width exceeds i32")?,
        i32::try_from(client.client_extent[1])
            .map_err(|_| "A31 selected client height exceeds i32")?,
    ];
    Ok(NationalFocusCanvasSelectedMetadata {
        selected_present_candidate_id: selected.receipt.selected_submit_index,
        selected_present_ordinal,
        selected_present_qpc_ticks: selected.receipt.selected_presentation_qpc_ticks,
        selected_present_cutoff_qpc_ticks: selected.receipt.cutoff_qpc_ticks,
        qpc_frequency_hz: selected.receipt.cutoff_qpc_frequency_hz,
        client_extent_pixels,
    })
}

pub fn register_national_focus_canvas_root(
    join: &PendingNationalFocusGuiJoin,
    selected: &production_render_submit_live_lane::a729_hook::RawSelectedRenderFrameCapture,
) -> Result<(), String> {
    production_render_submit_live_lane::a729_hook::validate_selected_render_frame_receipt(
        selected,
    )?;
    let frame_hour =
        compact_contract::calendar::hour_index_from_raw(selected.receipt.target_game_hour_raw)
            .and_then(|value| u64::try_from(value).ok())
            .ok_or("A31 selected-Present game hour is invalid")?;
    if join.root.frame_hour != frame_hour
        || selected.receipt.selected_submit_index != selected.selected.frame.submit_index
        || selected.receipt.selected_frame_sequence != selected.selected.frame.frame_sequence
    {
        return Err("A31 runtime root and selected-Present identities differ".into());
    }
    production_render_submit_live_lane::a729_hook::register_selected_present_gui_root(
        selected.receipt.selected_submit_index,
        selected.receipt.selected_frame_sequence,
        join.root.authored_surface_identity,
        join.root.runtime_root_identity,
        join.root.effective_interface_winner,
        join.root.callback_qpc_ticks,
        selected.receipt.target_game_hour_raw,
        join.root.focus_view_ptr,
        join.root.runtime_root_ptr,
        join.root.runtime_graphical_root_ptr,
    )
}

pub fn bind_national_focus_canvas(
    join: &PendingNationalFocusGuiJoin,
    gui_draws: &[SelectedPresentGuiDrawCapture],
    ui_pass: &AuthenticatedUiPass,
    selected: &NationalFocusCanvasSelectedMetadata,
    geometry: &NationalFocusCanvasRuntimeGeometry,
) -> Result<SelectedPresentCanvasBinding, String> {
    ui_pass.validate()?;
    if selected.qpc_frequency_hz == 0
        || selected.qpc_frequency_hz != geometry.qpc_frequency_hz
        || geometry.sample_started_qpc_ticks == 0
        || geometry.sample_completed_qpc_ticks < geometry.sample_started_qpc_ticks
        || geometry.sample_completed_qpc_ticks > selected.selected_present_qpc_ticks
        || selected.selected_present_qpc_ticks > selected.selected_present_cutoff_qpc_ticks
    {
        return Err("A31 runtime geometry does not precede the selected Present".into());
    }
    let mut item_roots = BTreeSet::new();
    let mut focus_operations = BTreeMap::<String, Vec<(u32, UiPassRect)>>::new();
    for item in &join.items {
        if item.item_ptr == 0
            || item.runtime_gui_root_ptr == 0
            || item.focus_id.is_empty()
            || !item_roots.insert(item.runtime_gui_root_ptr)
            || focus_operations
                .insert(item.focus_id.clone(), Vec::new())
                .is_some()
        {
            return Err(
                "A31 focus GUI item identity/root frontier is invalid or duplicated".into(),
            );
        }
    }

    let operations_by_ordinal = ui_pass
        .ui_pass_operations
        .iter()
        .map(|operation| (operation.ui_pass_ordinal(), operation))
        .collect::<BTreeMap<_, _>>();
    if operations_by_ordinal.len() != ui_pass.ui_pass_operations.len() {
        return Err("A31 selected-Present UI pass contains duplicate ordinals".into());
    }

    let mut canvas_operations = Vec::new();
    let mut owned_operations = BTreeSet::new();
    let mut exact_clip = None;
    let mut root_engine_instance_token = None;
    for draw in gui_draws {
        let Some(root) = draw.root.as_ref() else {
            continue;
        };
        if root.controller_ptr != join.root.focus_view_ptr
            || root.runtime_graphical_subobject_ptr != join.root.runtime_graphical_root_ptr
            || root.authored_surface_identity != join.root.authored_surface_identity
            || root.runtime_root_identity != join.root.runtime_root_identity
            || root.effective_interface_winner != join.root.effective_interface_winner
        {
            continue;
        }
        let ordinal = draw.operation.paint_ordinal;
        if !owned_operations.insert(ordinal) {
            return Err("A31 focus canvas owns a duplicate selected-Present ordinal".into());
        }
        let operation = operations_by_ordinal
            .get(&ordinal)
            .ok_or("A31 GUI draw ordinal is absent from the authenticated UI pass")?;
        let (identity_token, placement) = match operation {
            UiPassOperation::HudCommand { command } => {
                (command.identity.engine_instance_token, &command.placement)
            }
            UiPassOperation::M02 { .. } => {
                return Err("A31 common-GUI draw resolved to an M02 operation".into())
            }
        };
        if identity_token != draw.operation.engine_instance_token
            || placement.surface_paint_ordinal != ordinal
        {
            return Err("A31 GUI draw identity differs from its authenticated UI operation".into());
        }
        if root_engine_instance_token
            .replace(identity_token)
            .is_some_and(|prior| prior != identity_token)
        {
            return Err("A31 focus-root operations do not share one engine instance token".into());
        }
        if let UiPassClip::Enabled { rect } = placement.clip_state {
            if exact_clip.replace(rect).is_some_and(|prior| prior != rect) {
                return Err("A31 focus canvas operations disagree on the exact clip".into());
            }
        } else {
            return Err("A31 focus canvas operation omitted its exact clip".into());
        }
        let visible_rect = intersect_rect(
            placement.destination_rect,
            exact_clip.expect("set by enabled clip"),
        )
        .ok_or("A31 focus canvas operation lies wholly outside its exact clip")?;

        let mut matching_focus = None;
        for item in &join.items {
            if runtime_widget_descends_from(draw.runtime_widget_ptr, item.runtime_gui_root_ptr)? {
                if matching_focus.replace(item.focus_id.as_str()).is_some() {
                    return Err("A31 GUI draw descends from multiple focus-item roots".into());
                }
            }
        }
        if let Some(focus_id) = matching_focus {
            focus_operations
                .get_mut(focus_id)
                .expect("focus map created from join")
                .push((ordinal, visible_rect));
        } else {
            canvas_operations.push(ordinal);
        }
    }
    if owned_operations.is_empty() {
        return Err("A31 registered focus root owns no authenticated GUI operations".into());
    }

    let mut bound_focus_operations = Vec::with_capacity(join.items.len());
    for item in &join.items {
        let mut rows = focus_operations
            .remove(&item.focus_id)
            .ok_or("A31 focus item disappeared during GUI binding")?;
        if rows.is_empty() {
            return Err(format!(
                "A31 visible focus {} owns no selected-Present GUI operation",
                item.focus_id
            ));
        }
        rows.sort_unstable_by_key(|(ordinal, _)| *ordinal);
        let mut ordinals = Vec::with_capacity(rows.len());
        let mut screen_rect = None;
        for (ordinal, rect) in rows {
            ordinals.push(ordinal);
            screen_rect = Some(match screen_rect {
                Some(existing) => union_rect(existing, rect)?,
                None => rect,
            });
        }
        bound_focus_operations.push(FocusSelectedPresentOperations {
            focus_id: item.focus_id.clone(),
            operation_ordinals: ordinals,
            selected_present_screen_rect: to_focus_rect(
                screen_rect.expect("nonempty focus operation rows"),
            )?,
        });
    }
    canvas_operations.sort_unstable();
    let selected_present_ui_operation_order = owned_operations.iter().copied().collect::<Vec<_>>();
    let clip_rect =
        to_focus_rect(exact_clip.ok_or("A31 focus canvas has no exact selected-Present clip")?)?;
    let viewport_extent_pixels = [clip_rect.width, clip_rect.height];
    let render_target_extent_pixels = [
        i32::try_from(ui_pass.target.width)
            .map_err(|_| "A31 selected UI target width exceeds i32")?,
        i32::try_from(ui_pass.target.height)
            .map_err(|_| "A31 selected UI target height exceeds i32")?,
    ];

    Ok(SelectedPresentCanvasBinding {
        selected_present_candidate_id: selected.selected_present_candidate_id,
        selected_present_ordinal: selected.selected_present_ordinal,
        selected_present_cutoff_qpc_ticks: selected.selected_present_cutoff_qpc_ticks,
        selected_present_pass_identity_sha256: ui_pass.pass_identity_sha256.clone(),
        selected_present_engine_instance_token: root_engine_instance_token
            .ok_or("A31 focus canvas has no engine instance token")?,
        authored_surface_identity: join.root.authored_surface_identity.into(),
        runtime_root_identity: join.root.runtime_root_identity.into(),
        effective_interface_winner: join.root.effective_interface_winner.into(),
        render_target_extent_pixels,
        client_extent_pixels: selected.client_extent_pixels,
        clip_rect,
        content_extent_pixels: geometry.content_extent_pixels,
        viewport_extent_pixels,
        canvas_origin_pixels_fixed5: geometry.canvas_origin_pixels_fixed5,
        pan_pixels_fixed5: geometry.pan_pixels_fixed5,
        canvas_scale_bits: geometry.canvas_scale_bits,
        focus_operations: bound_focus_operations,
        canvas_operation_ordinals: canvas_operations,
        selected_present_ui_operation_order,
    })
}

fn runtime_widget_descends_from(mut widget: usize, root: usize) -> Result<bool, String> {
    if widget == 0 || root == 0 {
        return Err("A31 runtime GUI ancestry contains a null identity".into());
    }
    for _ in 0..=MAX_RUNTIME_PARENT_DEPTH {
        if widget == root {
            return Ok(true);
        }
        let parent_address = widget
            .checked_add(RUNTIME_WIDGET_PARENT_OFFSET)
            .ok_or("A31 runtime GUI parent address overflowed")?;
        let parent =
            read_usize(parent_address).ok_or("A31 runtime GUI parent link was unreadable")?;
        if parent == 0 {
            return Ok(false);
        }
        if parent == widget {
            return Err("A31 runtime GUI parent chain is self-referential".into());
        }
        widget = parent;
    }
    Err("A31 runtime GUI parent chain exceeded the exact depth bound".into())
}

fn read_usize(address: usize) -> Option<usize> {
    let mut value = 0usize;
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            (&mut value as *mut usize).cast(),
            core::mem::size_of::<usize>(),
            &mut read,
        )
    };
    (ok != 0 && read == core::mem::size_of::<usize>()).then_some(value)
}

fn intersect_rect(left: UiPassRect, right: UiPassRect) -> Option<UiPassRect> {
    let rect = UiPassRect {
        left_i32: left.left_i32.max(right.left_i32),
        top_i32: left.top_i32.max(right.top_i32),
        right_i32: left.right_i32.min(right.right_i32),
        bottom_i32: left.bottom_i32.min(right.bottom_i32),
    };
    (rect.right_i32 > rect.left_i32 && rect.bottom_i32 > rect.top_i32).then_some(rect)
}

fn union_rect(left: UiPassRect, right: UiPassRect) -> Result<UiPassRect, String> {
    let rect = UiPassRect {
        left_i32: left.left_i32.min(right.left_i32),
        top_i32: left.top_i32.min(right.top_i32),
        right_i32: left.right_i32.max(right.right_i32),
        bottom_i32: left.bottom_i32.max(right.bottom_i32),
    };
    if rect.right_i32 <= rect.left_i32 || rect.bottom_i32 <= rect.top_i32 {
        return Err("A31 selected-Present rectangle union is empty".into());
    }
    Ok(rect)
}

fn to_focus_rect(
    rect: UiPassRect,
) -> Result<compact_contract::national_focus_presentation::RectI32, String> {
    Ok(compact_contract::national_focus_presentation::RectI32 {
        x: rect.left_i32,
        y: rect.top_i32,
        width: rect
            .right_i32
            .checked_sub(rect.left_i32)
            .ok_or("A31 selected-Present rectangle width overflowed")?,
        height: rect
            .bottom_i32
            .checked_sub(rect.top_i32)
            .ok_or("A31 selected-Present rectangle height overflowed")?,
    })
}
