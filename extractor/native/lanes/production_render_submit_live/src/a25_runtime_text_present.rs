//! A25 captured-runtime text bridge into the selected-Present UI carrier.
//!
//! This materializer is deliberately downstream of both owner-glyph admission
//! and the render-hour selector. It cannot construct a text command from a
//! stable font/sprite name: every descriptor and texture hash must resolve to
//! exactly one winner in the authenticated launch VFS.

use crate::a729_hook::{
    validate_selected_render_frame_receipt, RawSelectedRenderFrameCapture,
    SelectedPresentGuiDrawCapture,
};
use crate::gui_paint_receipt::{GuiPaintReceipt, GuiPaintReceiptFrontier};
use crate::RawEffectiveVfsWinnerIndexBinding;
use portable_content_manifest::{ContentLayerKind, UsedContentKey};
use portable_render_stream::{
    ActiveMapmodeState, AuthenticatedUiPass, BufferSource, FontBinding, HudCommand,
    HudCommandExecution, HudCommandIdentity, HudPaintCommand, HudPlacement, InlineIconBinding,
    RenderTargetDesc, RenderTargetInitialSource, StaticGeometrySource, StaticMapDraw,
    StaticShaderResourceSource, TextHorizontalAlignment, TextLayoutInputs, TextureResourceDesc,
    UiExactPipelineDraw, UiPassClip, UiPassOperation, UI_PASS_SCHEMA,
};
use runtime_text_inputs_live_lane::owner_glyph_probe::{
    OwnerGlyphPresentCandidate, SelectedOwnerGlyphCapture,
};
use runtime_text_inputs_live_lane::RuntimeTextLayout;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// A captured-runtime text command cannot be scheduled until every producer
/// feeding the already-authenticated materializer has a production callback.
/// This is an integration gate, not a request for another visual fixture.
pub const A25_CAPTURED_RUNTIME_TEXT_PRODUCTION_ENABLED: bool = true;
/// Keep the shared A25 command binding in one global scope instead of
/// accepting caller entropy.
pub const A25_RUNTIME_TEXT_POV_SCOPE: &str = "global";
const A25_CAPTURED_RUNTIME_TEXT_CANONICAL_RGBA: [u8; 4] = [255; 4];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommonGuiOperationIdentity {
    pub engine_instance_token: u64,
    pub paint_ordinal: u32,
}

/// Pointer-free owner identity shared by every paint within one exact GUI-list
/// invocation. Node and paint ordinals remain separate operation identity;
/// putting either into this token would incorrectly create a different replay
/// owner for each command below the same registered GUI root.
pub fn common_gui_operation_identity(
    process_start_nonce: u64,
    hook_epoch_nonce: u64,
    submit_index: u64,
    frame_sequence: u64,
    list_invocation_ordinal: u64,
    _node_ordinal: u32,
    paint_ordinal: u32,
) -> Result<CommonGuiOperationIdentity, String> {
    if process_start_nonce == 0
        || hook_epoch_nonce == 0
        || submit_index == 0
        || frame_sequence == 0
        || list_invocation_ordinal == 0
    {
        return Err("A25 common-GUI operation identity is incomplete".into());
    }
    let mut identity = Sha256::new();
    identity.update(b"hoi4-a25-common-gui-owner-scope-v2\0");
    identity.update(process_start_nonce.to_le_bytes());
    identity.update(hook_epoch_nonce.to_le_bytes());
    identity.update(submit_index.to_le_bytes());
    identity.update(frame_sequence.to_le_bytes());
    identity.update(list_invocation_ordinal.to_le_bytes());
    let digest = identity.finalize();
    let mut engine_instance_token = u64::from_le_bytes(digest[..8].try_into().unwrap());
    if engine_instance_token == 0 {
        engine_instance_token = 1;
    }
    Ok(CommonGuiOperationIdentity {
        engine_instance_token,
        paint_ordinal,
    })
}

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

pub const A25_RUNTIME_TEXT_UNAVAILABLE_REASONS: &[A25RuntimeTextUnavailableReason] = &[];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum A25SelectedOwnerCaptureDrain {
    ProductionReady(Vec<SelectedOwnerGlyphCapture>),
    EvidenceOnly(Vec<SelectedOwnerGlyphCapture>),
    SourceUnavailable(Vec<A25RuntimeTextUnavailableReason>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A25SelectedOwnerCaptureReservation {
    pub reservation_id: Option<u64>,
    pub drain: A25SelectedOwnerCaptureDrain,
}

/// Reserve the exact selected/nonselected A25 owner frontier. A completed
/// reservation remains lane-owned until the enclosing compact item reaches
/// its authoritative sink; rollback makes the identical selected result
/// retriable without admitting or discarding any owner twice.
pub fn reserve_selected_a25_owner_captures(
    selected: OwnerGlyphPresentCandidate,
) -> Result<A25SelectedOwnerCaptureReservation, String> {
    if A25_CAPTURED_RUNTIME_TEXT_PRODUCTION_ENABLED {
        let reservation = runtime_text_inputs_live_lane::owner_glyph_probe::
            reserve_selected_present_if_production_active(selected)?
            .ok_or_else(|| {
                "A25 product latch is true without an installed production owner-glyph automation"
                    .to_string()
            })?;
        return Ok(A25SelectedOwnerCaptureReservation {
            reservation_id: Some(reservation.reservation_id),
            drain: A25SelectedOwnerCaptureDrain::ProductionReady(reservation.captures),
        });
    }
    if runtime_text_inputs_live_lane::owner_glyph_probe::A25_OWNER_GLYPH_EVIDENCE_MODE_ENABLED {
        let reservation = runtime_text_inputs_live_lane::owner_glyph_probe::
            reserve_selected_present_if_evidence_active(selected)?;
        return Ok(match reservation {
            Some(reservation) => A25SelectedOwnerCaptureReservation {
                reservation_id: Some(reservation.reservation_id),
                drain: A25SelectedOwnerCaptureDrain::EvidenceOnly(reservation.captures),
            },
            None => A25SelectedOwnerCaptureReservation {
                reservation_id: None,
                drain: A25SelectedOwnerCaptureDrain::EvidenceOnly(Vec::new()),
            },
        });
    }
    Ok(A25SelectedOwnerCaptureReservation {
        reservation_id: None,
        drain: A25SelectedOwnerCaptureDrain::SourceUnavailable(
            A25_RUNTIME_TEXT_UNAVAILABLE_REASONS.to_vec(),
        ),
    })
}

pub fn commit_selected_a25_owner_captures(reservation_id: u64) -> Result<(), String> {
    runtime_text_inputs_live_lane::owner_glyph_probe::commit_selected_present(reservation_id)
}

pub fn rollback_selected_a25_owner_captures(reservation_id: u64) -> Result<(), String> {
    runtime_text_inputs_live_lane::owner_glyph_probe::rollback_selected_present(reservation_id)
}

/// Drain the selected-Present owner queue without conflating a dormant
/// producer with an authenticated empty result. The legacy evidence switch is
/// still isolated and can never produce a production-ready drain.
pub fn drain_selected_a25_owner_captures(
    selected: OwnerGlyphPresentCandidate,
) -> Result<A25SelectedOwnerCaptureDrain, String> {
    let reservation = reserve_selected_a25_owner_captures(selected)?;
    if let Some(reservation_id) = reservation.reservation_id {
        commit_selected_a25_owner_captures(reservation_id)?;
    }
    Ok(reservation.drain)
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

/// Materialize the production selected-Present UI carrier directly from the
/// exact GUI-list rows. This path deliberately does not construct or consume a
/// `GuiPaintReceiptFrontier`: list/node traversal, owner joins, and promoted
/// D3D capture-sequence joins are checked at their native producer boundary.
///
/// The target is taken from the first promoted generic GUI draw and every
/// other generic row must name the same target. A text-only frontier is
/// rejected because these direct inputs contain no authenticated render-target
/// descriptor from which the pass target could be recovered.
#[allow(clippy::too_many_arguments)]
pub fn materialize_direct_selected_present_ui_pass(
    selected: &RawSelectedRenderFrameCapture,
    winners: &RawEffectiveVfsWinnerIndexBinding,
    gui_draws: &[SelectedPresentGuiDrawCapture],
    owner_captures: Vec<SelectedOwnerGlyphCapture>,
    mut promoted_exact_draws_by_capture_sequence: BTreeMap<u64, StaticMapDraw>,
    update_index: u64,
    recorded_input_receipt_sha256: String,
    active_mapmode_state: Option<ActiveMapmodeState>,
) -> Result<AuthenticatedUiPass, String> {
    validate_selected_render_frame_receipt(selected)?;
    winners.validate()?;
    if gui_draws.is_empty() {
        return Err("direct selected-Present GUI frontier is empty".into());
    }
    let target = promoted_exact_draws_by_capture_sequence
        .values()
        .next()
        .and_then(exact_gui_draw_target)
        .cloned()
        .ok_or(
            "direct selected-Present GUI frontier has no exact generic draw target",
        )?;
    let expected_candidate = selected_a25_owner_glyph_candidate(selected)?;
    let frame = &selected.selected.frame;
    let mut owner_by_key = BTreeMap::new();
    for owner in owner_captures {
        if owner.candidate != expected_candidate {
            return Err("direct selected-Present text owner crosses its selected Present".into());
        }
        let key = (
            owner.admitted.gui_engine_instance_token,
            owner.admitted.paint_ordinal,
        );
        if owner_by_key.insert(key, owner).is_some() {
            return Err("direct selected-Present text owner identity is duplicated".into());
        }
    }

    let mut operations = Vec::with_capacity(gui_draws.len());
    let mut source_frontier = Sha256::new();
    source_frontier.update(b"hoi4-a729-direct-selected-present-gui-frontier-v1\0");
    source_frontier.update(frame.process_start_nonce.to_le_bytes());
    source_frontier.update(frame.hook_epoch_nonce.to_le_bytes());
    source_frontier.update(frame.submit_index.to_le_bytes());
    source_frontier.update(frame.frame_sequence.to_le_bytes());
    source_frontier.update((gui_draws.len() as u64).to_le_bytes());
    let mut prior_list_node = None;
    let mut first_sequence = None;
    let mut last_sequence = None;

    for (ordinal, capture) in gui_draws.iter().enumerate() {
        let ordinal = u32::try_from(ordinal)
            .map_err(|_| "direct selected-Present GUI frontier exceeds u32".to_string())?;
        if capture.candidate_submit_index != frame.submit_index
            || capture.candidate_frame_sequence != frame.frame_sequence
        {
            return Err("direct GUI row crosses its selected Present".into());
        }
        if capture
            .root
            .as_ref()
            .is_some_and(|root| root.game_hour_raw != frame.game_hour_raw)
        {
            return Err("direct GUI root game hour differs from its selected Present".into());
        }
        if capture.operation.paint_ordinal != ordinal {
            return Err("direct GUI operation ordinals are not exact 0..N-1 order".into());
        }
        let list_node = (capture.list_invocation_ordinal, capture.node_ordinal);
        if capture.list_invocation_ordinal == 0
            || prior_list_node.is_some_and(|prior| list_node <= prior)
        {
            return Err("direct GUI rows are not in exact list/node traversal order".into());
        }
        prior_list_node = Some(list_node);
        let expected_operation = common_gui_operation_identity(
            frame.process_start_nonce,
            frame.hook_epoch_nonce,
            frame.submit_index,
            frame.frame_sequence,
            capture.list_invocation_ordinal,
            capture.node_ordinal,
            ordinal,
        )?;
        if capture.operation != expected_operation {
            return Err("direct GUI operation token does not authenticate its list/node row".into());
        }
        let identity = direct_gui_command_identity(capture, winners)?;
        if identity.engine_instance_token != capture.operation.engine_instance_token
            || identity.paint_ordinal != ordinal
        {
            return Err("direct GUI pointer-free identity differs from operation identity".into());
        }
        let owner_key = (capture.operation.engine_instance_token, ordinal);
        let owner = owner_by_key.remove(&owner_key);
        let command = match owner {
            Some(owner) => {
                if !capture.static_d3d_captures.is_empty() {
                    return Err(
                        "direct text GUI row also owns promoted generic D3D captures".into(),
                    );
                }
                materialize_direct_a25_text_command(
                    selected,
                    winners,
                    identity,
                    owner,
                    &target,
                )?
            }
            None => {
                if capture.static_d3d_captures.len() != 1 {
                    return Err(
                        "direct generic GUI row does not own exactly one D3D capture".into(),
                    );
                }
                let capture_sequence = capture.static_d3d_captures[0].state.capture_sequence;
                if capture_sequence == 0 {
                    return Err("direct generic GUI capture sequence is zero".into());
                }
                let exact_draw = promoted_exact_draws_by_capture_sequence
                    .remove(&capture_sequence)
                    .ok_or("direct generic GUI capture has no promoted exact draw")?;
                first_sequence.get_or_insert(capture_sequence);
                if last_sequence.is_some_and(|previous| capture_sequence <= previous) {
                    return Err(
                        "direct generic GUI capture sequences are not strictly increasing".into(),
                    );
                }
                last_sequence = Some(capture_sequence);
                materialize_direct_exact_gui_command(
                    winners,
                    identity,
                    exact_draw,
                    &target,
                )?
            }
        };
        source_frontier.update(capture.list_invocation_ordinal.to_le_bytes());
        source_frontier.update(capture.node_ordinal.to_le_bytes());
        source_frontier.update(capture.operation.engine_instance_token.to_le_bytes());
        source_frontier.update(capture.operation.paint_ordinal.to_le_bytes());
        let root = capture
            .root
            .as_ref()
            .ok_or("direct GUI row lost its authenticated runtime root")?;
        let authored = capture
            .authored_node
            .as_ref()
            .ok_or("direct GUI row lost its authenticated authored node")?;
        hash_framed_text(&mut source_frontier, &root.authored_surface_identity);
        hash_framed_text(&mut source_frontier, &root.runtime_root_identity);
        hash_framed_text(&mut source_frontier, &root.effective_interface_winner);
        source_frontier.update(root.callback_qpc_ticks.to_le_bytes());
        source_frontier.update(root.game_hour_raw.to_le_bytes());
        hash_framed_text(
            &mut source_frontier,
            &authored.runtime_widget_vtable_rva_hex,
        );
        hash_framed_text(
            &mut source_frontier,
            &authored.authored_type_vtable_rva_hex,
        );
        hash_framed_text(&mut source_frontier, &authored.authored_source_path);
        hash_framed_text(&mut source_frontier, &authored.authored_symbol);
        source_frontier.update(authored.authored_source_line.to_le_bytes());
        hash_u32_path(
            &mut source_frontier,
            capture
                .authored_child_path
                .as_ref()
                .ok_or("direct GUI row lost its authored child path")?,
        );
        hash_u32_path(
            &mut source_frontier,
            capture
                .graphical_child_path
                .as_ref()
                .ok_or("direct GUI row lost its graphical child path")?,
        );
        source_frontier.update((capture.static_d3d_captures.len() as u64).to_le_bytes());
        for raw_draw in &capture.static_d3d_captures {
            source_frontier.update(raw_draw.state.capture_sequence.to_le_bytes());
        }
        hash_framed_text(&mut source_frontier, &command.command_identity_sha256);
        operations.push(UiPassOperation::HudCommand { command });
    }
    if !owner_by_key.is_empty() {
        return Err("direct text owners fall outside the exact GUI frontier".into());
    }
    if !promoted_exact_draws_by_capture_sequence.is_empty() {
        return Err("promoted exact GUI draws fall outside the exact GUI frontier".into());
    }
    let first_sequence =
        first_sequence.ok_or("direct GUI frontier has no authenticated draw sequence")?;
    let last_sequence =
        last_sequence.ok_or("direct GUI frontier has no authenticated draw sequence")?;
    let selected_ordinal = selected
        .receipt
        .candidate_count
        .checked_sub(1)
        .ok_or("direct selected Present has zero candidates")?;
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
        update_index,
        first_sequence,
        last_sequence,
        ui_pass_draw_count: u32::try_from(operations.len())
            .map_err(|_| "direct GUI operation count exceeds u32".to_string())?,
        target,
        selected_present_receipt_sha256: portable_selected_present.computed_sha256()?,
        producer_selection_receipt_sha256: selected.receipt.selection_receipt_sha256.clone(),
        source_frontier_sha256: format!("{:x}", source_frontier.finalize()),
        recorded_input_receipt_sha256,
        active_mapmode_state,
        ui_pass_operations: operations,
        pass_identity_sha256: String::new(),
    };
    pass.pass_identity_sha256 = pass.computed_identity_sha256()?;
    pass.validate()?;
    Ok(pass)
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

fn direct_gui_command_identity(
    capture: &SelectedPresentGuiDrawCapture,
    winners: &RawEffectiveVfsWinnerIndexBinding,
) -> Result<HudCommandIdentity, String> {
    let root = capture
        .root
        .as_ref()
        .ok_or("direct GUI row has no authenticated runtime root")?;
    let authored = capture
        .authored_node
        .as_ref()
        .ok_or("direct GUI row has no authenticated authored node")?;
    let child_path = capture
        .authored_child_path
        .as_ref()
        .ok_or("direct GUI row has no authored child path")?;
    capture
        .graphical_child_path
        .as_ref()
        .ok_or("direct GUI row has no graphical child path")?;
    if root.authored_surface_identity.trim().is_empty()
        || root.runtime_root_identity.trim().is_empty()
        || root.effective_interface_winner.trim().is_empty()
        || root.callback_qpc_ticks == 0
        || authored.runtime_widget_vtable_rva_hex.trim().is_empty()
        || authored.authored_type_vtable_rva_hex.trim().is_empty()
        || authored.authored_source_path.trim().is_empty()
        || authored.authored_symbol.trim().is_empty()
        || authored.authored_source_line == 0
    {
        return Err("direct GUI authored/root identity is incomplete".into());
    }
    if root.candidate_submit_index != capture.candidate_submit_index
        || root.candidate_frame_sequence != capture.candidate_frame_sequence
        || root.effective_interface_winner != authored.authored_source_path
        || authored.selected_gui_definition.normalized_game_relative_path
            != root.effective_interface_winner
    {
        return Err("direct GUI authored node disagrees with its registered runtime root".into());
    }
    authorize_used_content(
        winners,
        &authored.selected_gui_definition,
        "direct selected GUI definition",
    )?;
    Ok(HudCommandIdentity {
        authored_surface_identity: root.authored_surface_identity.clone().into(),
        runtime_root: root.runtime_root_identity.clone().into(),
        selected_gui_definition: authored.selected_gui_definition.clone(),
        engine_instance_token: capture.operation.engine_instance_token,
        child_path: child_path.clone(),
        element_type: format!(
            "{}:{}:{}:{}",
            authored.authored_source_path,
            authored.authored_source_line,
            authored.authored_type_vtable_rva_hex,
            authored.runtime_widget_vtable_rva_hex,
        ),
        name: authored.authored_symbol.clone(),
        paint_ordinal: capture.operation.paint_ordinal,
    })
}

fn materialize_direct_a25_text_command(
    selected: &RawSelectedRenderFrameCapture,
    winners: &RawEffectiveVfsWinnerIndexBinding,
    identity: HudCommandIdentity,
    owner: SelectedOwnerGlyphCapture,
    target: &RenderTargetDesc,
) -> Result<HudCommand, String> {
    if owner.admitted.gui_engine_instance_token != identity.engine_instance_token
        || owner.admitted.paint_ordinal != identity.paint_ordinal
        || owner.view_id != identity.runtime_root
        || owner.element_id != identity.name
        || owner.context_id.trim().is_empty()
    {
        return Err("direct text owner does not exactly join its authored GUI row".into());
    }
    if !matches!(
        &owner.admitted.layout,
        RuntimeTextLayout::CapturedRuntime { .. }
    ) {
        return Err("direct selected-Present text owner has no captured runtime layout".into());
    }
    let placement = owner_placement(&owner.admitted)?;
    validate_placement_target(&placement, target)?;
    let mut ordered_content = vec![identity.selected_gui_definition.clone()];
    let mut used_paths = BTreeSet::from([
        identity
            .selected_gui_definition
            .normalized_game_relative_path
            .clone(),
    ]);
    let mut used_content = BTreeSet::from([identity.selected_gui_definition.clone()]);
    let mut font_bindings = Vec::new();
    for font in &owner.admitted.fonts {
        let definition = resolve_unique_payload(winners, &font.descriptor_sha256, "font definition")?;
        let definition_content_index = push_exact_content(
            &mut ordered_content,
            &mut used_paths,
            &mut used_content,
            definition,
        )?;
        let mut atlas_texture_content_indices = Vec::new();
        for digest in &font.atlas_sha256 {
            atlas_texture_content_indices.push(push_exact_content(
                &mut ordered_content,
                &mut used_paths,
                &mut used_content,
                resolve_unique_payload(winners, digest, "font atlas")?,
            )?);
        }
        font_bindings.push(FontBinding {
            font_identity: font.identity.clone(),
            definition_content_index,
            atlas_texture_content_indices,
        });
    }
    if font_bindings.is_empty() {
        return Err("direct selected-Present text has no authenticated font binding".into());
    }
    let mut inline_icon_bindings = Vec::new();
    for icon in &owner.admitted.icon_assets {
        let definition_content_index = push_exact_content(
            &mut ordered_content,
            &mut used_paths,
            &mut used_content,
            resolve_unique_payload(
                winners,
                &icon.definition_sha256,
                "inline-icon definition",
            )?,
        )?;
        let mut texture_content_indices = Vec::new();
        for digest in &icon.texture_sha256 {
            texture_content_indices.push(push_exact_content(
                &mut ordered_content,
                &mut used_paths,
                &mut used_content,
                resolve_unique_payload(winners, digest, "inline-icon texture")?,
            )?);
        }
        inline_icon_bindings.push(InlineIconBinding {
            sprite_identity: icon.identity.clone(),
            definition_content_index,
            texture_content_indices,
        });
    }
    let frame_hour = i64::try_from(owner.candidate.frame_hour)
        .map_err(|_| "direct text frame hour exceeds replay i64".to_string())?;
    let mut command = HudCommand {
        ordinal: identity.paint_ordinal,
        identity,
        placement,
        execution: HudCommandExecution::InterpretedPaint {
            paint: HudPaintCommand::Text {
                runtime_text_context_id: owner.context_id,
                runtime_text_frame_hour: frame_hour,
                runtime_text_pov_country_or_global: A25_RUNTIME_TEXT_POV_SCOPE.into(),
                runtime_text_view_id: owner.view_id,
                runtime_text_element_id: owner.element_id,
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
    if owner.candidate != selected_a25_owner_glyph_candidate(selected)? {
        return Err("direct text owner candidate changed during materialization".into());
    }
    command.command_identity_sha256 = command.computed_identity_sha256(target)?;
    command.validate(target)?;
    Ok(command)
}

fn materialize_direct_exact_gui_command(
    winners: &RawEffectiveVfsWinnerIndexBinding,
    identity: HudCommandIdentity,
    exact_draw: StaticMapDraw,
    target: &RenderTargetDesc,
) -> Result<HudCommand, String> {
    let draw_target = exact_gui_draw_target(&exact_draw)
        .ok_or("direct exact GUI draw does not own one color target without depth")?;
    if draw_target != target {
        return Err("direct exact GUI draw target disagrees with the selected UI target".into());
    }
    for binding in &exact_draw.ordered_shader_resources {
        if !matches!(
            &binding.source,
            StaticShaderResourceSource::UsedContent { .. }
        ) {
            return Err(
                "direct exact GUI draw uses a non-UsedContent shader resource source".into(),
            );
        }
    }
    if !matches!(&exact_draw.geometry, StaticGeometrySource::Exact { .. }) {
        return Err("direct exact GUI draw does not carry exact captured geometry".into());
    }
    let mut placement = placement_from_exact_gui_draw(&exact_draw)?;
    placement.surface_paint_ordinal = identity.paint_ordinal;
    validate_placement_target(&placement, target)?;
    let mut ordered_content = vec![identity.selected_gui_definition.clone()];
    for content in collect_direct_exact_draw_content(&exact_draw) {
        authorize_used_content(winners, &content, "direct exact GUI draw content")?;
        if !ordered_content.contains(&content) {
            ordered_content.push(content);
        }
    }
    let mut command = HudCommand {
        ordinal: identity.paint_ordinal,
        identity,
        placement,
        execution: HudCommandExecution::ExactPipeline {
            draw: UiExactPipelineDraw { exact_draw },
        },
        ordered_effective_vfs_used_content: ordered_content,
        command_identity_sha256: String::new(),
    };
    command.command_identity_sha256 = command.computed_identity_sha256(target)?;
    command.validate(target)?;
    Ok(command)
}

fn exact_gui_draw_target(draw: &StaticMapDraw) -> Option<&RenderTargetDesc> {
    (draw.output_state.ordered_color_targets.len() == 1
        && draw.output_state.depth_target.is_none())
    .then(|| &draw.output_state.ordered_color_targets[0])
}

fn placement_from_exact_gui_draw(draw: &StaticMapDraw) -> Result<HudPlacement, String> {
    let viewport = &draw.output_state.viewport;
    let left_i32 = exact_viewport_i32(
        viewport.top_left_x_f32_bits,
        "direct GUI viewport left",
        false,
    )?;
    let top_i32 = exact_viewport_i32(
        viewport.top_left_y_f32_bits,
        "direct GUI viewport top",
        false,
    )?;
    let width_i32 = exact_viewport_i32(
        viewport.width_f32_bits,
        "direct GUI viewport width",
        true,
    )?;
    let height_i32 = exact_viewport_i32(
        viewport.height_f32_bits,
        "direct GUI viewport height",
        true,
    )?;
    let destination_rect = portable_render_stream::UiPassRect {
        left_i32,
        top_i32,
        right_i32: left_i32
            .checked_add(width_i32)
            .ok_or("direct GUI viewport right edge overflows i32")?,
        bottom_i32: top_i32
            .checked_add(height_i32)
            .ok_or("direct GUI viewport bottom edge overflows i32")?,
    };
    let clip_state = match &draw.output_state.scissor {
        None => UiPassClip::Disabled,
        Some(scissor) => {
            if scissor.right_i32 <= scissor.left_i32
                || scissor.bottom_i32 <= scissor.top_i32
            {
                return Err("direct GUI draw scissor has non-positive extent".into());
            }
            UiPassClip::Enabled {
                rect: portable_render_stream::UiPassRect {
                    left_i32: scissor.left_i32,
                    top_i32: scissor.top_i32,
                    right_i32: scissor.right_i32,
                    bottom_i32: scissor.bottom_i32,
                },
            }
        }
    };
    Ok(HudPlacement {
        destination_rect,
        clip_state,
        surface_paint_ordinal: 0,
    })
}

fn exact_viewport_i32(bits: u32, label: &str, positive: bool) -> Result<i32, String> {
    let value = f32::from_bits(bits);
    let widened = f64::from(value);
    if !value.is_finite()
        || value.fract() != 0.0
        || widened < f64::from(i32::MIN)
        || widened > f64::from(i32::MAX)
        || (positive && value <= 0.0)
    {
        return Err(format!("{label} is not an exact admissible integer"));
    }
    let integer = value as i32;
    if (integer as f32).to_bits() != bits {
        return Err(format!("{label} is not an exact admissible integer"));
    }
    Ok(integer)
}

fn collect_direct_exact_draw_content(draw: &StaticMapDraw) -> Vec<UsedContentKey> {
    fn push_unique(out: &mut Vec<UsedContentKey>, content: &UsedContentKey) {
        if !out.contains(content) {
            out.push(content.clone());
        }
    }
    fn collect_source(out: &mut Vec<UsedContentKey>, source: &BufferSource) {
        if let BufferSource::UsedContentRange { content, .. } = source {
            push_unique(out, content);
        }
    }
    fn collect_resource(out: &mut Vec<UsedContentKey>, resource: &TextureResourceDesc) {
        for subresource in &resource.ordered_subresources {
            collect_source(out, &subresource.source);
        }
    }

    let mut content = Vec::new();
    push_unique(&mut content, &draw.shader_program.source);
    for binding in &draw.ordered_shader_resources {
        if let StaticShaderResourceSource::UsedContent {
            content: used,
            resource,
            ..
        } = &binding.source
        {
            push_unique(&mut content, used);
            collect_resource(&mut content, resource);
        }
    }
    if let StaticGeometrySource::Exact { geometry } = &draw.geometry {
        for binding in &geometry.ordered_vertex_buffers {
            collect_source(&mut content, &binding.source);
        }
        if let Some(binding) = &geometry.index_buffer {
            collect_source(&mut content, &binding.source);
        }
    }
    for target in draw
        .output_state
        .ordered_color_targets
        .iter()
        .chain(draw.output_state.depth_target.iter())
    {
        if let Some(used) = &target.persistent_content {
            push_unique(&mut content, used);
        }
        if let Some(exact) = &target.exact_resource {
            for initial in &exact.ordered_initial_subresources {
                if let RenderTargetInitialSource::UsedContentRange { content: used, .. } =
                    &initial.source
                {
                    push_unique(&mut content, used);
                }
            }
        }
    }
    content
}

fn hash_framed_text(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value.as_bytes());
}

fn hash_u32_path(hasher: &mut Sha256, path: &[u32]) {
    hasher.update((path.len() as u64).to_le_bytes());
    for component in path {
        hasher.update(component.to_le_bytes());
    }
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
/// queue before UI-pass materialization.
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
        let error = drain_selected_a25_owner_captures(OwnerGlyphPresentCandidate {
            process_start_nonce: 1,
            hook_epoch_nonce: 1,
            submit_index: 1,
            frame_sequence: 1,
            frame_hour: 1,
        })
        .unwrap_err();
        assert!(error.contains("without an installed production owner-glyph automation"));
        assert!(A25_CAPTURED_RUNTIME_TEXT_PRODUCTION_ENABLED);
    }

    #[test]
    fn production_gap_list_names_only_missing_callback_owners() {
        assert!(A25_RUNTIME_TEXT_UNAVAILABLE_REASONS.is_empty());
        assert!(A25_CAPTURED_RUNTIME_TEXT_PRODUCTION_ENABLED);
    }
}
