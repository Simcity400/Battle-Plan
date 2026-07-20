//! A25 captured-runtime text bridge into the selected-Present UI carrier.
//!
//! This materializer is deliberately downstream of both owner-glyph admission
//! and the render-hour selector. It cannot construct a text command from a
//! stable font/sprite name: every descriptor and texture hash must resolve to
//! exactly one winner in the authenticated launch VFS.

use crate::a729_hook::{validate_selected_render_frame_receipt, RawSelectedRenderFrameCapture};
use crate::gui_paint_receipt::GuiPaintReceipt;
use crate::RawEffectiveVfsWinnerIndexBinding;
use portable_content_manifest::{ContentLayerKind, UsedContentKey};
use portable_render_stream::{
    FontBinding, HudCommand, HudCommandExecution, HudCommandIdentity, HudPaintCommand,
    HudPlacement, InlineIconBinding, RenderTargetDesc, TextHorizontalAlignment, TextLayoutInputs,
};
use runtime_text_inputs_live_lane::owner_glyph_probe::{
    OwnerGlyphPresentCandidate, SelectedOwnerGlyphCapture,
};
use runtime_text_inputs_live_lane::RuntimeTextLayout;
use std::collections::BTreeSet;

/// A captured-runtime text command cannot be scheduled while the upstream
/// live owner/path fixtures remain unpromoted. Evidence mode may exercise the
/// same materializer and retain its failures without enabling replay output.
pub const A25_CAPTURED_RUNTIME_TEXT_PRODUCTION_ENABLED: bool = false;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A25SelectedPresentTextPaint {
    pub gui_paint: GuiPaintReceipt,
    pub owner_capture: SelectedOwnerGlyphCapture,
    pub pov_country_or_global: String,
    pub placement: HudPlacement,
    pub selected_ui_pass_target: RenderTargetDesc,
    pub rgba: [u8; 4],
    pub horizontal_alignment: TextHorizontalAlignment,
    pub layout_inputs: TextLayoutInputs,
}

/// Convert the exact extractor-local A25 asset receipts into the existing
/// pointer-free `HudPaintCommand::Text` recorder/replayer contract.
pub fn materialize_a25_selected_present_text_command(
    selected: &RawSelectedRenderFrameCapture,
    winners: &RawEffectiveVfsWinnerIndexBinding,
    input: A25SelectedPresentTextPaint,
) -> Result<HudCommand, String> {
    validate_selected_render_frame_receipt(selected)?;
    winners.validate()?;
    input.gui_paint.validate_for_emission()?;
    let frame = &selected.selected.frame;
    let catalog = &selected.selected.runtime.catalog;
    catalog.validate_launch_binding()?;
    let receipt = &selected.receipt;
    let candidate = input.owner_capture.candidate;
    let expected_candidate = selected_a25_owner_glyph_candidate(selected)?;
    if candidate != expected_candidate
        || input.gui_paint.process_start_nonce != frame.process_start_nonce
        || input.gui_paint.hook_epoch_nonce != frame.hook_epoch_nonce
        || input.gui_paint.selected_submit_index != frame.submit_index
        || input.gui_paint.selected_frame_sequence != frame.frame_sequence
        || input.gui_paint.selected_present_global_order != frame.global_order
        || input.gui_paint.selected_present_receipt_sha256
            != receipt.selection_receipt_sha256
        || catalog.process.process_start_nonce != frame.process_start_nonce
        || catalog.launch_epoch.hook_epoch_nonce != frame.hook_epoch_nonce
        || catalog.submit_index != frame.submit_index
    {
        return Err("A25 text paint crosses its selected Present or capture epoch".into());
    }
    if winners.process_start_nonce != frame.process_start_nonce {
        return Err("A25 text asset winners differ from the selected launch VFS".into());
    }
    let identity = &input.gui_paint.identity;
    authorize_used_content(winners, &identity.selected_gui_definition, "selected GUI definition")?;
    if identity.runtime_root != input.owner_capture.view_id
        || identity.name != input.owner_capture.element_id
        || input.owner_capture.context_id.trim().is_empty()
        || input.pov_country_or_global.trim().is_empty()
    {
        return Err("A25 text paint owner identity differs from its runtime-text row".into());
    }
    if input.gui_paint.draw_ordinal_span.draw_count != 1
        || input.gui_paint.draw_ordinal_span.first_ui_ordinal
            != input.gui_paint.draw_ordinal_span.last_ui_ordinal
        || identity.paint_ordinal != input.gui_paint.draw_ordinal_span.first_ui_ordinal
        || input.placement.surface_paint_ordinal != identity.paint_ordinal
    {
        return Err("A25 text paint does not own exactly one UI-pass operation".into());
    }
    if input.layout_inputs != TextLayoutInputs::CapturedRuntime {
        return Err("A25 owner-glyph observation may emit only CapturedRuntime layout".into());
    }
    if !matches!(
        &input.owner_capture.admitted.layout,
        RuntimeTextLayout::CapturedRuntime { .. }
    ) {
        return Err("A25 selected-Present owner capture has no admitted runtime layout".into());
    }
    validate_placement_target(&input.placement, &input.selected_ui_pass_target)?;

    let mut ordered_content = vec![identity.selected_gui_definition.clone()];
    let mut used_paths = BTreeSet::from([
        identity
            .selected_gui_definition
            .normalized_game_relative_path
            .clone(),
    ]);
    let mut used_content = BTreeSet::from([identity.selected_gui_definition.clone()]);
    let mut font_bindings = Vec::new();
    for font in &input.owner_capture.admitted.fonts {
        let definition = resolve_unique_payload(winners, &font.descriptor_sha256, "font definition")?;
        let definition_content_index = push_exact_content(
            &mut ordered_content,
            &mut used_paths,
            &mut used_content,
            definition,
        )?;
        let mut atlas_texture_content_indices = Vec::new();
        for digest in &font.atlas_sha256 {
            let atlas = resolve_unique_payload(winners, digest, "font atlas")?;
            atlas_texture_content_indices.push(push_exact_content(
                &mut ordered_content,
                &mut used_paths,
                &mut used_content,
                atlas,
            )?);
        }
        font_bindings.push(FontBinding {
            font_identity: font.identity.clone(),
            definition_content_index,
            atlas_texture_content_indices,
        });
    }
    let mut inline_icon_bindings = Vec::new();
    for icon in &input.owner_capture.admitted.icon_assets {
        let definition =
            resolve_unique_payload(winners, &icon.definition_sha256, "inline-icon definition")?;
        let definition_content_index = push_exact_content(
            &mut ordered_content,
            &mut used_paths,
            &mut used_content,
            definition,
        )?;
        let mut texture_content_indices = Vec::new();
        for digest in &icon.texture_sha256 {
            let texture = resolve_unique_payload(winners, digest, "inline-icon texture")?;
            texture_content_indices.push(push_exact_content(
                &mut ordered_content,
                &mut used_paths,
                &mut used_content,
                texture,
            )?);
        }
        inline_icon_bindings.push(InlineIconBinding {
            sprite_identity: icon.identity.clone(),
            definition_content_index,
            texture_content_indices,
        });
    }
    if font_bindings.is_empty() {
        return Err("A25 selected-Present text has no authenticated font binding".into());
    }

    let command_identity = HudCommandIdentity {
        authored_surface_identity: identity.authored_surface_identity.clone(),
        runtime_root: identity.runtime_root.clone(),
        selected_gui_definition: identity.selected_gui_definition.clone(),
        child_path: identity.child_path.clone(),
        element_type: identity.element_type.clone(),
        name: identity.name.clone(),
        paint_ordinal: identity.paint_ordinal,
    };
    let mut command = HudCommand {
        ordinal: identity.paint_ordinal,
        identity: command_identity,
        placement: input.placement,
        execution: HudCommandExecution::InterpretedPaint {
            paint: HudPaintCommand::Text {
                runtime_text_context_id: input.owner_capture.context_id,
                runtime_text_frame_hour: i64::try_from(candidate.frame_hour)
                    .map_err(|_| "A25 frame hour exceeds replay i64".to_string())?,
                runtime_text_pov_country_or_global: input.pov_country_or_global,
                runtime_text_view_id: input.owner_capture.view_id,
                runtime_text_element_id: input.owner_capture.element_id,
                rgba: input.rgba,
                horizontal_alignment: input.horizontal_alignment,
                layout_inputs: input.layout_inputs,
                font_bindings,
                inline_icon_bindings,
            },
        },
        ordered_effective_vfs_used_content: ordered_content,
        command_identity_sha256: String::new(),
    };
    command.command_identity_sha256 =
        command.computed_identity_sha256(&input.selected_ui_pass_target)?;
    Ok(command)
}

/// Pointer-free selected candidate key consumed by the A25 evidence-mode
/// queue before d76 collection and UI-pass materialization.
pub fn selected_a25_owner_glyph_candidate(
    selected: &RawSelectedRenderFrameCapture,
) -> Result<OwnerGlyphPresentCandidate, String> {
    validate_selected_render_frame_receipt(selected)?;
    let frame = &selected.selected.frame;
    Ok(OwnerGlyphPresentCandidate {
        process_start_nonce: frame.process_start_nonce,
        hook_epoch_nonce: frame.hook_epoch_nonce,
        submit_index: frame.submit_index,
        frame_sequence: frame.frame_sequence,
        frame_hour: u64::try_from(
            compact_contract::calendar::hour_index_from_raw(frame.game_hour_raw)
                .ok_or("A25 selected Present has a noncanonical game hour")?,
        )
        .map_err(|_| "A25 selected Present has a negative canonical hour")?,
    })
}

fn resolve_unique_payload(
    winners: &RawEffectiveVfsWinnerIndexBinding,
    payload_sha256: &str,
    label: &str,
) -> Result<UsedContentKey, String> {
    let mut matches = winners
        .ordered_rows
        .iter()
        .filter(|row| row.content.payload_sha256 == payload_sha256);
    let row = matches
        .next()
        .ok_or_else(|| format!("A25 {label} hash is absent from the authenticated VFS winners"))?;
    if matches.next().is_some() {
        return Err(format!(
            "A25 {label} hash is ambiguous across authenticated VFS winners"
        ));
    }
    used_content_from_raw(winners, &row.content)
}

fn used_content_from_raw(
    winners: &RawEffectiveVfsWinnerIndexBinding,
    raw: &crate::RawVfsContentIdentity,
) -> Result<UsedContentKey, String> {
    winners.authorize_content(raw)?;
    let layer_kind = match raw.layer_kind.as_str() {
        "base" => ContentLayerKind::Base,
        "integrated_dlc" => ContentLayerKind::IntegratedDlc,
        "dlc" => ContentLayerKind::Dlc,
        "mod" => ContentLayerKind::Mod,
        _ => return Err("A25 asset winner has an unknown layer kind".into()),
    };
    let content = UsedContentKey {
        effective_vfs_manifest_sha256: winners.authority_manifest_sha256.clone(),
        normalized_game_relative_path: raw.logical_path.clone(),
        winning_layer_ordinal: raw.winning_layer_ordinal,
        layer_kind,
        layer_id: raw.layer_id.clone(),
        source_sha256: raw.source_sha256.clone(),
        payload_sha256: raw.payload_sha256.clone(),
        payload_bytes: raw.payload_bytes,
    };
    content.validate()?;
    Ok(content)
}

fn authorize_used_content(
    winners: &RawEffectiveVfsWinnerIndexBinding,
    content: &UsedContentKey,
    label: &str,
) -> Result<(), String> {
    content
        .validate()
        .map_err(|error| format!("A25 {label} is invalid: {error}"))?;
    let row = winners
        .ordered_rows
        .iter()
        .find(|row| row.content.logical_path == content.normalized_game_relative_path)
        .ok_or_else(|| format!("A25 {label} is absent from authenticated VFS winners"))?;
    if used_content_from_raw(winners, &row.content)? != *content {
        return Err(format!("A25 {label} differs from the authenticated VFS winner"));
    }
    Ok(())
}

fn validate_placement_target(
    placement: &HudPlacement,
    target: &RenderTargetDesc,
) -> Result<(), String> {
    let rect = placement.destination_rect;
    if rect.left_i32 < 0
        || rect.top_i32 < 0
        || rect.right_i32 <= rect.left_i32
        || rect.bottom_i32 <= rect.top_i32
        || u32::try_from(rect.right_i32)
            .ok()
            .map_or(true, |right| right > target.width)
        || u32::try_from(rect.bottom_i32)
            .ok()
            .map_or(true, |bottom| bottom > target.height)
    {
        return Err("A25 text placement escapes the selected UI-pass target".into());
    }
    Ok(())
}

fn push_exact_content(
    ordered: &mut Vec<UsedContentKey>,
    used_paths: &mut BTreeSet<String>,
    used_content: &mut BTreeSet<UsedContentKey>,
    content: UsedContentKey,
) -> Result<u32, String> {
    if !used_paths.insert(content.normalized_game_relative_path.clone())
        || !used_content.insert(content.clone())
    {
        return Err("A25 font/icon roles alias one VFS content identity".into());
    }
    let index = u32::try_from(ordered.len())
        .map_err(|_| "A25 text content index exceeds u32".to_string())?;
    ordered.push(content);
    Ok(index)
}
