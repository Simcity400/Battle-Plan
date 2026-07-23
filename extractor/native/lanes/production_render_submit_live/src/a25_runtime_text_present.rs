//! A25 captured-runtime text bridge into the selected-Present UI carrier.
//!
//! This materializer is deliberately downstream of both owner-glyph admission
//! and the render-hour selector. It cannot construct a text command from a
//! stable font/sprite name: every descriptor and texture hash must resolve to
//! exactly one winner in the authenticated launch VFS.

use crate::a729_hook::{validate_selected_render_frame_receipt, RawSelectedRenderFrameCapture};
use crate::gui_paint_receipt::{GuiPaintReceipt, GuiPaintReceiptFrontier};
use crate::RawEffectiveVfsWinnerIndexBinding;
use portable_content_manifest::{ContentLayerKind, UsedContentKey};
use portable_render_stream::{
    ActiveMapmodeState, AuthenticatedUiPass, FontBinding, HudCommand, HudCommandExecution,
    HudCommandIdentity, HudPaintCommand, HudPlacement, InlineIconBinding, RenderTargetDesc,
    TextHorizontalAlignment, TextLayoutInputs, UiPassClip, UiPassOperation, UI_PASS_SCHEMA,
};
use runtime_text_inputs_live_lane::owner_glyph_probe::{
    OwnerGlyphPresentCandidate, SelectedOwnerGlyphCapture,
};
use runtime_text_inputs_live_lane::RuntimeTextLayout;
use std::collections::{BTreeMap, BTreeSet};

/// A captured-runtime text command cannot be scheduled until every producer
/// feeding the already-authenticated materializer has a production callback.
/// This is an integration gate, not a request for another visual fixture.
pub const A25_CAPTURED_RUNTIME_TEXT_PRODUCTION_ENABLED: bool = false;
/// d76 currently records the selected local UI catalog in one global scope.
/// Keep the A25 command binding identical instead of accepting caller entropy.
pub const A25_RUNTIME_TEXT_POV_SCOPE: &str = "global";
const A25_CAPTURED_RUNTIME_TEXT_CANONICAL_RGBA: [u8; 4] = [255; 4];

/// Exact production callbacks still absent from the A25 producer graph. Keep
/// these reasons typed so the collector cannot mistake an empty observation
/// set for measured-empty text paint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum A25RuntimeTextUnavailableReason {
    /// No live producer calls `begin_gui_paint_candidate_scope`, and the exact
    /// later final-draw callback can capture draw payload, selected target, and
    /// pipeline/scissor state only after `CInstantTextObject::Render` RVA
    /// `0x022D8350` has begun emitting the draw. It does not provide the missing
    /// pre-draw owner paint ordinal or destination. The owner-entry callback
    /// must therefore remain unavailable instead of borrowing later draw state
    /// or promoting the pointer-bearing diagnostic rectangle candidates.
    CommonGuiPaintOwnerEntryAuthorityUnavailable,
    /// The exact font and sprite pointers observed by the bitmap-font hooks do
    /// not yet have a mechanical pointer-to-authenticated-VFS receipt producer.
    AuthenticatedFontAndIconAssetBindingNotInstalled,
    /// The post-emit/cache hook owners are not yet installed by the early
    /// render lifecycle. Their static insertion points are exact, but dormant.
    OrdinaryGlyphHookLifecycleNotInstalled,
    /// The derived `CGameBitmapFont` callbacks are exact at RVAs
    /// `0x0162E880/0x0162EFE0/0x0162EB10`, but are not yet part of the installed
    /// render hook set.
    DerivedInlineIconHookLifecycleNotInstalled,
}

pub const A25_RUNTIME_TEXT_UNAVAILABLE_REASONS: &[A25RuntimeTextUnavailableReason] = &[
    A25RuntimeTextUnavailableReason::CommonGuiPaintOwnerEntryAuthorityUnavailable,
    A25RuntimeTextUnavailableReason::AuthenticatedFontAndIconAssetBindingNotInstalled,
    A25RuntimeTextUnavailableReason::OrdinaryGlyphHookLifecycleNotInstalled,
    A25RuntimeTextUnavailableReason::DerivedInlineIconHookLifecycleNotInstalled,
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum A25SelectedOwnerCaptureDrain {
    ProductionReady(Vec<SelectedOwnerGlyphCapture>),
    EvidenceOnly(Vec<SelectedOwnerGlyphCapture>),
    SourceUnavailable(Vec<A25RuntimeTextUnavailableReason>),
}

/// Drain the selected-Present owner queue without conflating a dormant
/// producer with an authenticated empty result. The legacy evidence switch is
/// still isolated and can never produce a production-ready drain.
pub fn drain_selected_a25_owner_captures(
    selected: OwnerGlyphPresentCandidate,
) -> Result<A25SelectedOwnerCaptureDrain, String> {
    if A25_CAPTURED_RUNTIME_TEXT_PRODUCTION_ENABLED {
        return Err(
            "A25 product latch is true without an installed production owner-glyph drain".into(),
        );
    }
    if runtime_text_inputs_live_lane::owner_glyph_probe::A25_OWNER_GLYPH_EVIDENCE_MODE_ENABLED {
        return runtime_text_inputs_live_lane::owner_glyph_probe::
            admit_selected_present_if_evidence_active(selected)
            .map(A25SelectedOwnerCaptureDrain::EvidenceOnly);
    }
    Ok(A25SelectedOwnerCaptureDrain::SourceUnavailable(
        A25_RUNTIME_TEXT_UNAVAILABLE_REASONS.to_vec(),
    ))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A25SelectedPresentTextPaint {
    pub gui_paint: GuiPaintReceipt,
    pub owner_capture: SelectedOwnerGlyphCapture,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A25SelectedPresentUiPassAssembly {
    pub update_index: u64,
    pub active_mapmode_state: Option<ActiveMapmodeState>,
    pub recorded_input_receipt_sha256: String,
    pub owner_captures: Vec<SelectedOwnerGlyphCapture>,
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
    if input.gui_paint.pre_materialized_command.is_some() {
        return Err("A25 GUI receipt also carries a non-A25 command".into());
    }
    let selected_ui_pass_target = input.gui_paint.selected_ui_pass_target.clone();
    let placement = owner_placement(&input.owner_capture.admitted)?;
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
        || identity.engine_instance_token
            != input.owner_capture.admitted.gui_engine_instance_token
        || identity.paint_ordinal != input.owner_capture.admitted.paint_ordinal
    {
        return Err("A25 text paint owner identity differs from its runtime-text row".into());
    }
    if input.gui_paint.draw_ordinal_span.draw_count == 0
        || input.gui_paint.draw_ordinal_span.first_ui_ordinal
            != input.gui_paint.draw_ordinal_span.last_ui_ordinal
        || identity.paint_ordinal != input.gui_paint.draw_ordinal_span.first_ui_ordinal
        || placement.surface_paint_ordinal != identity.paint_ordinal
    {
        return Err("A25 text paint does not own exactly one UI-pass operation".into());
    }
    if !matches!(
        &input.owner_capture.admitted.layout,
        RuntimeTextLayout::CapturedRuntime { .. }
    ) {
        return Err("A25 selected-Present owner capture has no admitted runtime layout".into());
    }
    validate_placement_target(&placement, &selected_ui_pass_target)?;

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
        engine_instance_token: identity.engine_instance_token,
        child_path: identity.child_path.clone(),
        element_type: identity.element_type.clone(),
        name: identity.name.clone(),
        paint_ordinal: identity.paint_ordinal,
    };
    let mut command = HudCommand {
        ordinal: identity.paint_ordinal,
        identity: command_identity,
        placement,
        execution: HudCommandExecution::InterpretedPaint {
            paint: HudPaintCommand::Text {
                runtime_text_context_id: input.owner_capture.context_id,
                runtime_text_frame_hour: i64::try_from(candidate.frame_hour)
                    .map_err(|_| "A25 frame hour exceeds replay i64".to_string())?,
                runtime_text_pov_country_or_global: A25_RUNTIME_TEXT_POV_SCOPE.into(),
                runtime_text_view_id: input.owner_capture.view_id,
                runtime_text_element_id: input.owner_capture.element_id,
                // Captured-runtime painting uses each admitted glyph's exact
                // RGBA/position and never reflows by horizontal alignment.
                // Canonical constants keep ignored fields out of caller control.
                rgba: A25_CAPTURED_RUNTIME_TEXT_CANONICAL_RGBA,
                horizontal_alignment: TextHorizontalAlignment::Left,
                layout_inputs: TextLayoutInputs::CapturedRuntime,
                font_bindings,
                inline_icon_bindings,
            },
        },
        ordered_effective_vfs_used_content: ordered_content,
        command_identity_sha256: String::new(),
    };
    command.command_identity_sha256 =
        command.computed_identity_sha256(&selected_ui_pass_target)?;
    Ok(command)
}

/// Assemble a complete selected-Present UI pass from the chronological GUI
/// frontier. A25 commands are derived here; every other GUI receipt must have
/// an independently authenticated command, so this function never invents a
/// placeholder operation merely to close the ordinal range.
pub fn assemble_selected_present_ui_pass(
    selected: &RawSelectedRenderFrameCapture,
    winners: &RawEffectiveVfsWinnerIndexBinding,
    frontier: GuiPaintReceiptFrontier,
    input: A25SelectedPresentUiPassAssembly,
) -> Result<AuthenticatedUiPass, String> {
    validate_selected_render_frame_receipt(selected)?;
    winners.validate()?;
    frontier.validate_for_ui_emission()?;
    let frame = &selected.selected.frame;
    if frontier.process_start_nonce != frame.process_start_nonce
        || frontier.hook_epoch_nonce != frame.hook_epoch_nonce
        || frontier.selected_submit_index != selected.receipt.selected_submit_index
        || frontier.selected_frame_sequence != selected.receipt.selected_frame_sequence
        || frontier.selected_present_global_order != selected.receipt.selected_global_order
        || frontier.selected_present_receipt_sha256
            != selected.receipt.selection_receipt_sha256
    {
        return Err("A25 UI frontier crosses its selected Present".into());
    }
    let first_sequence = frontier
        .ordered_receipts
        .first()
        .and_then(|receipt| receipt.ordered_draws.first())
        .map(|draw| draw.global_render_sequence)
        .ok_or("A25 UI frontier has no first draw sequence")?;
    let last_sequence = frontier
        .ordered_receipts
        .last()
        .and_then(|receipt| receipt.ordered_draws.last())
        .map(|draw| draw.global_render_sequence)
        .ok_or("A25 UI frontier has no last draw sequence")?;

    let mut owner_by_key = BTreeMap::new();
    for capture in input.owner_captures {
        let key = (
            capture.admitted.gui_engine_instance_token,
            capture.admitted.paint_ordinal,
        );
        if owner_by_key.insert(key, capture).is_some() {
            return Err("A25 selected owner capture identity is duplicated".into());
        }
    }
    let mut operations = Vec::with_capacity(frontier.complete_ui_operation_count as usize);
    for receipt in frontier.ordered_receipts {
        if receipt.draw_ordinal_span.draw_count == 0
            || receipt.draw_ordinal_span.first_ui_ordinal
                != receipt.draw_ordinal_span.last_ui_ordinal
            || receipt.identity.paint_ordinal != receipt.draw_ordinal_span.first_ui_ordinal
        {
            return Err("selected GUI receipt is not one exact UI operation".into());
        }
        let ordinal = receipt.identity.paint_ordinal;
        let owner_key = (receipt.identity.engine_instance_token, ordinal);
        let generic_command = receipt.pre_materialized_command.clone();
        let command = match (owner_by_key.remove(&owner_key), generic_command) {
            (Some(owner_capture), None) => materialize_a25_selected_present_text_command(
                selected,
                winners,
                A25SelectedPresentTextPaint {
                    gui_paint: receipt,
                    owner_capture,
                },
            )?,
            (None, Some(command)) => command,
            (Some(_), Some(_)) => {
                return Err("A25 and non-A25 UI commands claim the same operation".into())
            }
            (None, None) => {
                return Err("complete GUI frontier contains an unmaterialized operation".into())
            }
        };
        operations.push(UiPassOperation::HudCommand { command });
    }
    if !owner_by_key.is_empty() {
        return Err("selected UI assembly has commands outside the complete GUI frontier".into());
    }
    let selected_ordinal = selected
        .receipt
        .candidate_count
        .checked_sub(1)
        .ok_or("A25 selected Present has zero candidates")?;
    let portable_selected_present = portable_render_stream::SelectedPresentReceipt::sealed(
        selected.receipt.candidate_count,
        selected.receipt.candidate_set_sha256.clone(),
        selected.receipt.selected_submit_index,
        selected_ordinal,
        selected.receipt.target_game_hour_raw,
        selected.receipt.selected_presentation_qpc_ticks,
        selected.receipt.cutoff_qpc_ticks,
    );
    let mut pass = AuthenticatedUiPass {
        schema: UI_PASS_SCHEMA.into(),
        update_index: input.update_index,
        first_sequence,
        last_sequence,
        ui_pass_draw_count: frontier.complete_ui_operation_count,
        target: frontier.selected_ui_pass_target,
        selected_present_receipt_sha256: portable_selected_present.computed_sha256()?,
        producer_selection_receipt_sha256: selected.receipt.selection_receipt_sha256.clone(),
        source_frontier_sha256: frontier.frontier_sha256,
        recorded_input_receipt_sha256: input.recorded_input_receipt_sha256,
        active_mapmode_state: input.active_mapmode_state,
        ui_pass_operations: operations,
        pass_identity_sha256: String::new(),
    };
    pass.pass_identity_sha256 = pass.computed_identity_sha256()?;
    pass.validate()?;
    Ok(pass)
}

fn owner_placement(
    admitted: &runtime_text_inputs_live_lane::owner_glyph_capture::AdmittedOwnerGlyphCapture,
) -> Result<HudPlacement, String> {
    let [left_i32, top_i32, right_i32, bottom_i32] = admitted.destination_rect;
    if right_i32 <= left_i32 || bottom_i32 <= top_i32 {
        return Err("A25 owner-render rectangle has a non-positive extent".into());
    }
    let destination_rect = portable_render_stream::UiPassRect {
        left_i32,
        top_i32,
        right_i32,
        bottom_i32,
    };
    let clip_state = match admitted.ui_clip {
        None => UiPassClip::Disabled,
        Some([left_i32, top_i32, right_i32, bottom_i32]) => {
            if right_i32 <= left_i32 || bottom_i32 <= top_i32 {
                return Err("A25 owner-render scissor has a non-positive extent".into());
            }
            UiPassClip::Enabled {
                rect: portable_render_stream::UiPassRect {
                    left_i32,
                    top_i32,
                    right_i32,
                    bottom_i32,
                },
            }
        }
    };
    Ok(HudPlacement {
        destination_rect,
        clip_state,
        surface_paint_ordinal: admitted.paint_ordinal,
    })
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

#[cfg(test)]
mod production_reachability_tests {
    use super::*;

    #[test]
    fn dormant_product_returns_typed_unavailable_not_empty_capture() {
        let drain = drain_selected_a25_owner_captures(OwnerGlyphPresentCandidate {
            process_start_nonce: 1,
            hook_epoch_nonce: 1,
            submit_index: 1,
            frame_sequence: 1,
            frame_hour: 1,
        })
        .unwrap();
        assert_eq!(
            drain,
            A25SelectedOwnerCaptureDrain::SourceUnavailable(
                A25_RUNTIME_TEXT_UNAVAILABLE_REASONS.to_vec()
            )
        );
        assert!(!A25_CAPTURED_RUNTIME_TEXT_PRODUCTION_ENABLED);
    }

    #[test]
    fn production_gap_list_names_only_missing_callback_owners() {
        assert_eq!(A25_RUNTIME_TEXT_UNAVAILABLE_REASONS.len(), 4);
        assert!(!crate::gui_paint_receipt::GUI_PAINT_LIVE_RECEIPT_PROVEN);
        assert!(A25_RUNTIME_TEXT_UNAVAILABLE_REASONS.contains(
            &A25RuntimeTextUnavailableReason::CommonGuiPaintOwnerEntryAuthorityUnavailable
        ));
        assert!(A25_RUNTIME_TEXT_UNAVAILABLE_REASONS.contains(
            &A25RuntimeTextUnavailableReason::AuthenticatedFontAndIconAssetBindingNotInstalled
        ));
    }
}
