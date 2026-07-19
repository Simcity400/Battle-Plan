//! Typed selected-Present UI-pass consumer.
//!
//! The portable carrier owns the complete draw frontier and therefore the
//! only legal M02/HUD interleave.  This module prepares that stream without
//! concatenating families, inventing a default root, or borrowing a vanilla
//! asset.  Exact-pipeline commands remain D3D commands for the app adapter;
//! proven interpreted sprite commands become one transparent pixel layer at
//! the same ordinal. Text commands stay explicitly unsupported until their
//! typed runtime catalog and layout inputs reach this product boundary.

use crate::assets::{normalize_rel_path, AssetResolver};
use crate::compose::{
    draw_image, draw_nine_slice, draw_progress_minmax, draw_scaled, draw_scaled_crop,
};
use crate::dds::RgbaImage;
use crate::font::{blank_image, BitmapFont, TextAlign};
use crate::gfx::SpriteRegistry;
use crate::topbar::apply_mask;
use crate::{GameUiError, Result};
use replay_core::{
    AuthenticatedUiPass, FontBinding, HudCommand, HudCommandExecution, HudPaintCommand,
    InlineIconBinding, M02CombatDrawRef, M02CombatFrame, RuntimeTextCatalog, RuntimeTextLayout,
    RuntimeTextRecord, SpriteLaw, TextHorizontalAlignment, TextLayoutInputs, TextRectOffsets,
    UiPassClip, UiPassOperation,
};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub enum PreparedUiPassOperation {
    M02 {
        ui_pass_ordinal: u32,
        output: M02CombatDrawRef,
    },
    ExactPipeline {
        command: HudCommand,
    },
    InterpretedPaint {
        command: HudCommand,
        /// The preflight-rendered full-frame layer for this command. Kept so
        /// execution uploads the already-proven pixels instead of repeating
        /// the decode + rasterization. Memory note: every interpreted layer
        /// (width*height*4 bytes) in the pass is held simultaneously for the
        /// lifetime of the prepared pass — acceptable at HUD-scale command
        /// counts.
        image: RgbaImage,
    },
}

impl PreparedUiPassOperation {
    pub fn ordinal(&self) -> u32 {
        match self {
            Self::M02 {
                ui_pass_ordinal, ..
            } => *ui_pass_ordinal,
            Self::ExactPipeline { command } | Self::InterpretedPaint { command, .. } => {
                command.ordinal
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreparedUiPass {
    pub update_index: u64,
    pub width: u32,
    pub height: u32,
    pub operations: Vec<PreparedUiPassOperation>,
}

/// Playback identity that scopes A25 text rows to the exact displayed frame.
/// A context id is only a compact join token; it is never sufficient by
/// itself because ids may be reused by another hour, POV, view, or element.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectedPresentPlaybackContext<'a> {
    pub frame_hour: i64,
    pub pov_country_or_global: &'a str,
}

/// Build the selected-Present decoder capability from exactly the content
/// rows authenticated by HUD commands in this pass. This is deliberately
/// narrower than the replay-wide content admission.
pub fn admitted_asset_resolver(
    resolver: &map_viewer::GameFileResolver,
    admission: &replay_core::ValidatedContentAdmission,
    pass: &AuthenticatedUiPass,
) -> Result<AssetResolver> {
    pass.validate().map_err(|error| {
        GameUiError::parse(format!(
            "invalid authenticated selected-Present UI pass: {error}"
        ))
    })?;
    AssetResolver::from_used_content_rows(
        resolver,
        admission,
        pass.ui_pass_operations
            .iter()
            .flat_map(|operation| match operation {
                UiPassOperation::HudCommand { command } => {
                    command.ordered_effective_vfs_used_content.clone()
                }
                UiPassOperation::M02 { .. } => Vec::new(),
            }),
    )
}

/// Prepare every operation before the first pixel is written. A missing or
/// unsupported command rejects the feature locally; it never permits M02 to
/// jump over an unexecuted HUD ordinal.
pub fn prepare_selected_present_ui_pass(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    pass: &AuthenticatedUiPass,
    m02: Option<&M02CombatFrame>,
    runtime_text: Option<&RuntimeTextCatalog>,
    playback_context: SelectedPresentPlaybackContext<'_>,
) -> Result<PreparedUiPass> {
    pass.validate().map_err(|error| {
        GameUiError::parse(format!(
            "invalid authenticated selected-Present UI pass: {error}"
        ))
    })?;
    validate_m02_interleave(pass, m02)?;

    let width = pass.target.width;
    let height = pass.target.height;
    let mut operations = Vec::with_capacity(pass.ui_pass_operations.len());
    for operation in &pass.ui_pass_operations {
        operations.push(match operation {
            UiPassOperation::M02 {
                ui_pass_ordinal,
                output,
            } => PreparedUiPassOperation::M02 {
                ui_pass_ordinal: *ui_pass_ordinal,
                output: output.clone(),
            },
            UiPassOperation::HudCommand { command } => match &command.execution {
                HudCommandExecution::ExactPipeline { .. } => {
                    PreparedUiPassOperation::ExactPipeline {
                        command: command.clone(),
                    }
                }
                HudCommandExecution::InterpretedPaint { paint } => {
                    // Preflight the complete interpreter and every referenced
                    // VFS payload before the caller writes the first UI pixel,
                    // keeping the rendered layer so execution does not repeat
                    // the decode + rasterization.
                    let image = render_interpreted_paint(
                        resolver,
                        registry,
                        runtime_text,
                        playback_context,
                        command,
                        paint,
                        width,
                        height,
                    )?;
                    PreparedUiPassOperation::InterpretedPaint {
                        command: command.clone(),
                        image,
                    }
                }
            },
        });
    }
    if operations
        .iter()
        .enumerate()
        .any(|(ordinal, operation)| operation.ordinal() != ordinal as u32)
    {
        return Err(GameUiError::parse(
            "prepared UI-pass operation order changed after authentication",
        ));
    }
    Ok(PreparedUiPass {
        update_index: pass.update_index,
        width,
        height,
        operations,
    })
}

fn validate_m02_interleave(pass: &AuthenticatedUiPass, m02: Option<&M02CombatFrame>) -> Result<()> {
    let pass_refs = pass
        .ui_pass_operations
        .iter()
        .filter_map(|operation| match operation {
            UiPassOperation::M02 { output, .. } => Some(output),
            UiPassOperation::HudCommand { .. } => None,
        })
        .collect::<Vec<_>>();
    if pass_refs.is_empty() {
        if m02.is_some_and(|frame| !frame.ordered_draws.is_empty()) {
            return Err(GameUiError::parse(
                "M02 carrier exists but selected-Present UI pass omits its draws",
            ));
        }
        return Ok(());
    }
    let m02 = m02.ok_or_else(|| {
        GameUiError::missing(
            "selected-Present UI pass references M02 but the authenticated M02 carrier is absent",
        )
    })?;
    m02.validate()
        .map_err(|error| GameUiError::parse(format!("invalid M02 carrier: {error}")))?;
    if m02.update_index != pass.update_index {
        return Err(GameUiError::parse(
            "selected-Present UI pass and M02 carrier belong to different updates",
        ));
    }
    if pass_refs.len() != m02.ordered_draws.len()
        || pass_refs
            .iter()
            .zip(&m02.ordered_draws)
            .any(|(from_pass, from_m02)| *from_pass != from_m02)
    {
        return Err(GameUiError::parse(
            "selected-Present UI pass does not exactly cover M02 draw order",
        ));
    }
    Ok(())
}

fn render_interpreted_paint(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    runtime_text: Option<&RuntimeTextCatalog>,
    playback_context: SelectedPresentPlaybackContext<'_>,
    command: &HudCommand,
    paint: &HudPaintCommand,
    width: u32,
    height: u32,
) -> Result<RgbaImage> {
    match paint {
        HudPaintCommand::Sprite {
            sprite_id,
            resolved_frame,
            law,
            sprite_definition_content_index,
            texture_content_index,
            back_texture_content_index,
        } => render_sprite(
            resolver,
            registry,
            command,
            sprite_id,
            *resolved_frame,
            law,
            *sprite_definition_content_index,
            *texture_content_index,
            *back_texture_content_index,
            width,
            height,
        ),
        HudPaintCommand::Text {
            runtime_text_context_id,
            runtime_text_frame_hour,
            runtime_text_pov_country_or_global,
            runtime_text_view_id,
            runtime_text_element_id,
            rgba,
            horizontal_alignment,
            layout_inputs,
            font_bindings,
            inline_icon_bindings,
        } => {
            let record = require_runtime_text_record(
                runtime_text,
                runtime_text_context_id,
                *runtime_text_frame_hour,
                runtime_text_pov_country_or_global,
                runtime_text_view_id,
                runtime_text_element_id,
                playback_context,
                command,
            )?;
            render_text(
                resolver,
                registry,
                command,
                record,
                *rgba,
                *horizontal_alignment,
                layout_inputs,
                font_bindings,
                inline_icon_bindings,
                width,
                height,
            )
        }
        HudPaintCommand::MaskedSprite {
            sprite_id,
            mask_slot,
            overlay_slot,
            sprite_definition_content_index,
            texture_content_index,
            mask_texture_content_index,
            overlay_texture_content_index,
        } => render_masked_sprite(
            resolver,
            registry,
            command,
            sprite_id,
            *mask_slot,
            *overlay_slot,
            *sprite_definition_content_index,
            *texture_content_index,
            *mask_texture_content_index,
            *overlay_texture_content_index,
            width,
            height,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn render_text(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    command: &HudCommand,
    record: &RuntimeTextRecord,
    rgba: [u8; 4],
    horizontal_alignment: TextHorizontalAlignment,
    layout_inputs: &TextLayoutInputs,
    font_bindings: &[FontBinding],
    inline_icon_bindings: &[InlineIconBinding],
    width: u32,
    height: u32,
) -> Result<RgbaImage> {
    let fonts = load_bound_fonts(resolver, command, font_bindings)?;
    validate_inline_icon_bindings(registry, command, inline_icon_bindings)?;
    let destination = command.placement.destination_rect;
    let mut image = blank_image(width as usize, height as usize);
    match (&record.layout, layout_inputs) {
        (
            RuntimeTextLayout::AuthoredReproducible,
            TextLayoutInputs::AuthoredReproducible {
                authored_rect_offsets,
                max_width,
                wrap,
                clip_rect_offsets,
            },
        ) => {
            if !inline_icon_bindings.is_empty() {
                return Err(GameUiError::parse(format!(
                    "authored-reproducible HUD text {} carries captured inline-icon bindings",
                    command.ordinal
                )));
            }
            let primary = font_bindings.first().ok_or_else(|| {
                GameUiError::parse("authenticated HUD text has no primary font binding")
            })?;
            let font = fonts.get(&primary.font_identity).ok_or_else(|| {
                GameUiError::parse("authenticated HUD primary font binding was not loaded")
            })?;
            let rect = offset_rect(destination, *authored_rect_offsets)?;
            let layout_width = max_width
                .map(|value| value as i32)
                .unwrap_or(rect.right_i32 - rect.left_i32);
            let text = if *wrap {
                crate::gui_scene_util::wrap_text_greedy(
                    &record.final_text_utf8,
                    layout_width,
                    |candidate| font.measure_text(candidate),
                )
            } else {
                record.final_text_utf8.clone()
            };
            let report = font.draw_text(
                &mut image,
                rect.left_i32,
                rect.top_i32,
                Some(layout_width),
                match horizontal_alignment {
                    TextHorizontalAlignment::Left => TextAlign::Left,
                    TextHorizontalAlignment::Center => TextAlign::Center,
                    TextHorizontalAlignment::Right => TextAlign::Right,
                },
                &text,
                rgba,
            );
            if !report.missing_glyphs.is_empty() {
                return Err(GameUiError::missing(format!(
                    "authenticated HUD text {} primary font {} lacks glyphs {:?}",
                    command.ordinal, primary.font_identity, report.missing_glyphs
                )));
            }
            if let Some(clip) = clip_rect_offsets {
                apply_rect_clip(&mut image, offset_rect(destination, *clip)?);
            }
        }
        (RuntimeTextLayout::CapturedRuntime(captured), TextLayoutInputs::CapturedRuntime) => {
            let effective_clip = captured_effective_clip(
                destination,
                captured.clip,
                command.placement.clip_state,
                width,
                height,
            )?;
            let glyph_clip = Some([
                effective_clip.left_i32,
                effective_clip.top_i32,
                effective_clip.right_i32 - effective_clip.left_i32,
                effective_clip.bottom_i32 - effective_clip.top_i32,
            ]);
            for glyph in &captured.glyphs {
                let font = fonts.get(&glyph.font).ok_or_else(|| {
                    GameUiError::missing(format!(
                        "captured HUD text {} lacks exact font binding {}",
                        command.ordinal, glyph.font
                    ))
                })?;
                let (x, y) = captured_owner_relative_point(destination, glyph.x, glyph.y, "glyph")?;
                if !font.draw_runtime_glyph(
                    &mut image,
                    glyph.glyph_id,
                    x,
                    y,
                    glyph.rgba,
                    glyph_clip,
                ) {
                    return Err(GameUiError::missing(format!(
                        "captured HUD text {} font {} lacks glyph {}",
                        command.ordinal, glyph.font, glyph.glyph_id
                    )));
                }
            }
            for icon in &captured.inline_icons {
                let binding = inline_icon_bindings
                    .iter()
                    .find(|binding| binding.sprite_identity == icon.sprite)
                    .ok_or_else(|| {
                        GameUiError::missing(format!(
                            "captured HUD text {} lacks inline-icon binding {}",
                            command.ordinal, icon.sprite
                        ))
                    })?;
                let definition = registry.get(&binding.sprite_identity).ok_or_else(|| {
                    GameUiError::missing(format!(
                        "captured HUD text {} references missing inline sprite {}",
                        command.ordinal, binding.sprite_identity
                    ))
                })?;
                let decoded = resolver.decode_sprite(definition)?;
                let frame = decoded.frames.get(icon.frame as usize).ok_or_else(|| {
                    GameUiError::parse(format!(
                        "captured HUD inline sprite {} frame {} is outside 0..{}",
                        icon.sprite,
                        icon.frame,
                        decoded.frames.len()
                    ))
                })?;
                let (x, y) =
                    captured_owner_relative_point(destination, icon.x, icon.y, "inline icon")?;
                draw_image(&mut image, frame, x, y);
            }
            apply_rect_clip(&mut image, effective_clip);
        }
        _ => {
            return Err(GameUiError::parse(format!(
                "authenticated HUD text {} layout kind disagrees with A25 context {}",
                command.ordinal, record.context_id
            )))
        }
    }
    apply_clip(&mut image, command.placement.clip_state);
    Ok(image)
}

fn load_bound_fonts(
    resolver: &AssetResolver,
    command: &HudCommand,
    bindings: &[FontBinding],
) -> Result<BTreeMap<String, BitmapFont>> {
    let mut fonts = BTreeMap::new();
    for binding in bindings {
        let descriptor = indexed_command_content_path(
            command,
            binding.definition_content_index,
            "font definition",
        )?;
        let font = BitmapFont::load_from_descriptor(resolver, &binding.font_identity, descriptor)?;
        require_indexed_command_content(
            command,
            binding.definition_content_index,
            &font.source_file,
            "font definition",
        )?;
        if font.pages.len() != binding.atlas_texture_content_indices.len() {
            return Err(GameUiError::missing(format!(
                "authenticated HUD font {} has {} pages but {} atlas bindings",
                binding.font_identity,
                font.pages.len(),
                binding.atlas_texture_content_indices.len()
            )));
        }
        for (page, index) in font
            .pages
            .iter()
            .zip(&binding.atlas_texture_content_indices)
        {
            require_indexed_command_content(command, *index, &page.file, "font atlas")?;
        }
        fonts.insert(binding.font_identity.clone(), font);
    }
    Ok(fonts)
}

fn validate_inline_icon_bindings(
    registry: &SpriteRegistry,
    command: &HudCommand,
    bindings: &[InlineIconBinding],
) -> Result<()> {
    for binding in bindings {
        let definition = registry.get(&binding.sprite_identity).ok_or_else(|| {
            GameUiError::missing(format!(
                "authenticated HUD text references missing inline sprite {}",
                binding.sprite_identity
            ))
        })?;
        require_indexed_command_content(
            command,
            binding.definition_content_index,
            &definition.source_file,
            "inline-sprite definition",
        )?;
        if definition.texture_files.len() != binding.texture_content_indices.len() {
            return Err(GameUiError::missing(format!(
                "authenticated HUD inline sprite {} has {} texture slots but {} bindings",
                binding.sprite_identity,
                definition.texture_files.len(),
                binding.texture_content_indices.len()
            )));
        }
        for (texture, index) in definition
            .texture_files
            .iter()
            .zip(&binding.texture_content_indices)
        {
            require_indexed_command_content(command, *index, &texture.path, "inline texture")?;
        }
    }
    Ok(())
}

/// Frozen A25 law: glyph/icon run coordinates are signed owner-top-left
/// offsets. Playback applies no baseline translation, DPI transform, GUI
/// scaling, or destination-size normalization.
fn captured_owner_relative_point(
    destination: replay_core::UiPassRect,
    local_x: i32,
    local_y: i32,
    role: &str,
) -> Result<(i32, i32)> {
    let x = destination
        .left_i32
        .checked_add(local_x)
        .ok_or_else(|| GameUiError::parse(format!("captured HUD {role} x coordinate overflow")))?;
    let y = destination
        .top_i32
        .checked_add(local_y)
        .ok_or_else(|| GameUiError::parse(format!("captured HUD {role} y coordinate overflow")))?;
    Ok((x, y))
}

fn captured_effective_clip(
    destination: replay_core::UiPassRect,
    local_clip: Option<[i32; 4]>,
    command_clip: UiPassClip,
    width: u32,
    height: u32,
) -> Result<replay_core::UiPassRect> {
    let target = replay_core::UiPassRect {
        left_i32: 0,
        top_i32: 0,
        right_i32: i32::try_from(width)
            .map_err(|_| GameUiError::parse("selected-Present target width exceeds i32"))?,
        bottom_i32: i32::try_from(height)
            .map_err(|_| GameUiError::parse("selected-Present target height exceeds i32"))?,
    };
    let mut effective = match command_clip {
        UiPassClip::Disabled => target,
        UiPassClip::Enabled { rect } => intersect_rect(target, rect),
    };
    if let Some([x, y, w, h]) = local_clip {
        if w <= 0 || h <= 0 {
            return Err(GameUiError::parse(
                "captured HUD local clip has non-positive extent",
            ));
        }
        let left = destination
            .left_i32
            .checked_add(x)
            .ok_or_else(|| GameUiError::parse("captured HUD clip left coordinate overflow"))?;
        let top = destination
            .top_i32
            .checked_add(y)
            .ok_or_else(|| GameUiError::parse("captured HUD clip top coordinate overflow"))?;
        let right = left
            .checked_add(w)
            .ok_or_else(|| GameUiError::parse("captured HUD clip right coordinate overflow"))?;
        let bottom = top
            .checked_add(h)
            .ok_or_else(|| GameUiError::parse("captured HUD clip bottom coordinate overflow"))?;
        effective = intersect_rect(
            effective,
            replay_core::UiPassRect {
                left_i32: left,
                top_i32: top,
                right_i32: right,
                bottom_i32: bottom,
            },
        );
    }
    Ok(effective)
}

fn intersect_rect(
    left: replay_core::UiPassRect,
    right: replay_core::UiPassRect,
) -> replay_core::UiPassRect {
    let left_i32 = left.left_i32.max(right.left_i32);
    let top_i32 = left.top_i32.max(right.top_i32);
    replay_core::UiPassRect {
        left_i32,
        top_i32,
        right_i32: left.right_i32.min(right.right_i32).max(left_i32),
        bottom_i32: left.bottom_i32.min(right.bottom_i32).max(top_i32),
    }
}

fn offset_rect(
    base: replay_core::UiPassRect,
    offsets: TextRectOffsets,
) -> Result<replay_core::UiPassRect> {
    Ok(replay_core::UiPassRect {
        left_i32: base
            .left_i32
            .checked_add(offsets.left_i32)
            .ok_or_else(|| GameUiError::parse("authored HUD text left coordinate overflow"))?,
        top_i32: base
            .top_i32
            .checked_add(offsets.top_i32)
            .ok_or_else(|| GameUiError::parse("authored HUD text top coordinate overflow"))?,
        right_i32: base
            .left_i32
            .checked_add(offsets.right_i32)
            .ok_or_else(|| GameUiError::parse("authored HUD text right coordinate overflow"))?,
        bottom_i32: base
            .top_i32
            .checked_add(offsets.bottom_i32)
            .ok_or_else(|| GameUiError::parse("authored HUD text bottom coordinate overflow"))?,
    })
}

fn apply_rect_clip(image: &mut RgbaImage, rect: replay_core::UiPassRect) {
    apply_clip(image, UiPassClip::Enabled { rect });
}

fn require_runtime_text_record<'a>(
    catalog: Option<&'a RuntimeTextCatalog>,
    context_id: &str,
    carried_frame_hour: i64,
    carried_pov_country_or_global: &str,
    carried_view_id: &str,
    carried_element_id: &str,
    playback_context: SelectedPresentPlaybackContext<'_>,
    command: &HudCommand,
) -> Result<&'a RuntimeTextRecord> {
    let catalog = catalog.ok_or_else(|| {
        GameUiError::missing(format!(
            "authenticated HUD text context {context_id} requires the A25 runtime-text catalog"
        ))
    })?;
    if !catalog.contexts_complete {
        return Err(GameUiError::missing(format!(
            "A25 runtime-text catalog is incomplete for HUD context {context_id}"
        )));
    }
    let record = catalog.contexts.get(context_id).ok_or_else(|| {
        GameUiError::missing(format!(
            "complete A25 runtime-text catalog lacks HUD context {context_id}"
        ))
    })?;
    let expected_view = command.identity.runtime_root.as_str();
    let expected_element = command.identity.name.as_str();
    if carried_frame_hour != playback_context.frame_hour
        || carried_pov_country_or_global != playback_context.pov_country_or_global
        || carried_view_id != expected_view
        || carried_element_id != expected_element
        || record.frame_hour != carried_frame_hour
        || record.pov_country_or_global != carried_pov_country_or_global
        || record.view_id != carried_view_id
        || record.element_id != carried_element_id
    {
        return Err(GameUiError::missing(format!(
            "A25 runtime-text context {context_id} identity mismatch: recorded=({}, {:?}, {:?}, {:?}) current=({}, {:?}, {:?}, {:?})",
            record.frame_hour,
            record.pov_country_or_global,
            record.view_id,
            record.element_id,
            playback_context.frame_hour,
            playback_context.pov_country_or_global,
            expected_view,
            expected_element,
        )));
    }
    Ok(record)
}

#[allow(clippy::too_many_arguments)]
fn render_masked_sprite(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    command: &HudCommand,
    sprite_id: &str,
    mask_slot: u8,
    overlay_slot: u8,
    sprite_definition_content_index: u32,
    texture_content_index: u32,
    mask_texture_content_index: u32,
    overlay_texture_content_index: u32,
    width: u32,
    height: u32,
) -> Result<RgbaImage> {
    let sprite = registry.get(sprite_id).ok_or_else(|| {
        GameUiError::missing(format!(
            "authenticated HUD command {} references missing masked sprite {}",
            command.ordinal, sprite_id
        ))
    })?;
    require_indexed_command_content(
        command,
        sprite_definition_content_index,
        &sprite.source_file,
        "masked-sprite definition",
    )?;

    let base_path = indexed_command_content_path(command, texture_content_index, "base texture")?;
    let mask = sprite
        .texture_files
        .get(mask_slot as usize)
        .ok_or_else(|| {
            GameUiError::missing(format!(
                "authenticated HUD masked sprite {} lacks mask slot {}",
                sprite_id, mask_slot
            ))
        })?;
    let overlay = sprite
        .texture_files
        .get(overlay_slot as usize)
        .ok_or_else(|| {
            GameUiError::missing(format!(
                "authenticated HUD masked sprite {} lacks overlay slot {}",
                sprite_id, overlay_slot
            ))
        })?;
    require_indexed_command_content(
        command,
        mask_texture_content_index,
        &mask.path,
        "mask texture",
    )?;
    require_indexed_command_content(
        command,
        overlay_texture_content_index,
        &overlay.path,
        "overlay texture",
    )?;

    let base = resolver.decode_texture(base_path)?.image;
    let mask = resolver.decode_texture(&mask.path)?.image;
    let overlay = resolver.decode_texture(&overlay.path)?.image;
    let composed = compose_masked_layers(&base, &mask, &overlay)?;

    let rect = command.placement.destination_rect;
    let target_w = usize::try_from(rect.right_i32 - rect.left_i32)
        .map_err(|_| GameUiError::parse("HUD masked-sprite destination width is invalid"))?;
    let target_h = usize::try_from(rect.bottom_i32 - rect.top_i32)
        .map_err(|_| GameUiError::parse("HUD masked-sprite destination height is invalid"))?;
    let mut image = blank_image(width as usize, height as usize);
    if target_w == composed.width && target_h == composed.height {
        draw_image(&mut image, &composed, rect.left_i32, rect.top_i32);
    } else {
        draw_scaled(
            &mut image,
            &composed,
            rect.left_i32,
            rect.top_i32,
            target_w,
            target_h,
        );
    }
    apply_clip(&mut image, command.placement.clip_state);
    Ok(image)
}

fn compose_masked_layers(
    base: &RgbaImage,
    mask: &RgbaImage,
    overlay: &RgbaImage,
) -> Result<RgbaImage> {
    if overlay.width == 0 || overlay.height == 0 {
        return Err(GameUiError::parse(
            "authenticated HUD masked sprite has an empty overlay",
        ));
    }
    let mut composed = blank_image(overlay.width, overlay.height);
    draw_scaled(&mut composed, base, 0, 0, overlay.width, overlay.height);
    apply_mask(&mut composed, mask);
    draw_image(&mut composed, overlay, 0, 0);
    Ok(composed)
}

fn render_sprite(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    command: &HudCommand,
    sprite_id: &str,
    resolved_frame: u32,
    law: &SpriteLaw,
    sprite_definition_content_index: u32,
    texture_content_index: u32,
    back_texture_content_index: Option<u32>,
    width: u32,
    height: u32,
) -> Result<RgbaImage> {
    let sprite = registry.get(sprite_id).ok_or_else(|| {
        GameUiError::missing(format!(
            "authenticated HUD command {} references missing sprite {}",
            command.ordinal, sprite_id
        ))
    })?;
    require_indexed_command_content(
        command,
        sprite_definition_content_index,
        &sprite.source_file,
        "sprite definition",
    )?;
    if sprite.texture_files.is_empty() {
        return Err(GameUiError::missing(format!(
            "authenticated HUD sprite {} lacks its required texture slot",
            sprite_id
        )));
    }
    require_indexed_command_content(
        command,
        texture_content_index,
        &sprite.texture_files[0].path,
        "sprite texture",
    )?;
    if matches!(law, SpriteLaw::ProgressMinMax { .. }) {
        let back_index = back_texture_content_index.ok_or_else(|| {
            GameUiError::missing(format!(
                "authenticated HUD progress_minmax command {} lacks its back-texture index",
                command.ordinal
            ))
        })?;
        let back_slot = sprite.texture_files.get(1).ok_or_else(|| {
            GameUiError::missing(format!(
                "authenticated HUD progress_minmax sprite {} lacks its back texture slot",
                sprite_id
            ))
        })?;
        require_indexed_command_content(command, back_index, &back_slot.path, "back texture")?;
    }

    let decoded = resolver.decode_sprite(sprite)?;
    let frame = decoded.frames.get(resolved_frame as usize).ok_or_else(|| {
        GameUiError::parse(format!(
            "authenticated HUD sprite {} frame {} is outside 0..{}",
            sprite_id,
            resolved_frame,
            decoded.frames.len()
        ))
    })?;
    let rect = command.placement.destination_rect;
    let target_w = usize::try_from(rect.right_i32 - rect.left_i32)
        .map_err(|_| GameUiError::parse("HUD sprite destination width is invalid"))?;
    let target_h = usize::try_from(rect.bottom_i32 - rect.top_i32)
        .map_err(|_| GameUiError::parse("HUD sprite destination height is invalid"))?;
    let mut image = blank_image(width as usize, height as usize);
    match law {
        SpriteLaw::Plain => {
            if frame.width != target_w || frame.height != target_h {
                return Err(GameUiError::parse(format!(
                    "plain HUD sprite {} dimensions {}x{} do not equal destination {}x{}",
                    sprite_id, frame.width, frame.height, target_w, target_h
                )));
            }
            draw_image(&mut image, frame, rect.left_i32, rect.top_i32);
        }
        SpriteLaw::Scaled => draw_scaled(
            &mut image,
            frame,
            rect.left_i32,
            rect.top_i32,
            target_w,
            target_h,
        ),
        SpriteLaw::NineSlice { border_x, border_y } => draw_nine_slice(
            &mut image,
            frame,
            rect.left_i32,
            rect.top_i32,
            target_w,
            target_h,
            *border_x as usize,
            *border_y as usize,
        ),
        SpriteLaw::ProgressLeftCrop { state_bits } => {
            let state = exact_unit_state(*state_bits, command.ordinal)?;
            let fill_w = ((target_w as f32) * state).round() as usize;
            if fill_w > 0 {
                draw_scaled_crop(
                    &mut image,
                    frame,
                    rect.left_i32,
                    rect.top_i32,
                    target_w,
                    target_h,
                    fill_w,
                );
            }
        }
        SpriteLaw::ProgressMinMax { state_bits } => {
            let state = exact_unit_state(*state_bits, command.ordinal)?;
            let back = resolver
                .decode_texture(&sprite.texture_files[1].path)?
                .image;
            draw_progress_minmax(
                &mut image,
                frame,
                &back,
                rect.left_i32,
                rect.top_i32,
                target_w,
                target_h,
                state,
            );
        }
    }
    apply_clip(&mut image, command.placement.clip_state);
    Ok(image)
}

fn require_indexed_command_content(
    command: &HudCommand,
    index: u32,
    path: &str,
    role: &str,
) -> Result<()> {
    let normalized = normalize_rel_path(path).to_ascii_lowercase();
    let Some(content) = command
        .ordered_effective_vfs_used_content
        .get(index as usize)
    else {
        return Err(GameUiError::parse(format!(
            "authenticated HUD command {} has out-of-range {role} content index {index}",
            command.ordinal
        )));
    };
    if normalize_rel_path(&content.normalized_game_relative_path).to_ascii_lowercase() == normalized
    {
        Ok(())
    } else {
        Err(GameUiError::missing(format!(
            "authenticated HUD command {} {role} content index {index} resolves to {}, expected {}",
            command.ordinal, content.normalized_game_relative_path, normalized
        )))
    }
}

fn indexed_command_content_path<'a>(
    command: &'a HudCommand,
    index: u32,
    role: &str,
) -> Result<&'a str> {
    command
        .ordered_effective_vfs_used_content
        .get(index as usize)
        .map(|content| content.normalized_game_relative_path.as_str())
        .ok_or_else(|| {
            GameUiError::parse(format!(
                "authenticated HUD command {} has out-of-range {role} content index {index}",
                command.ordinal
            ))
        })
}

fn exact_unit_state(bits: u32, ordinal: u32) -> Result<f32> {
    let state = f32::from_bits(bits);
    if !state.is_finite() || !(0.0..=1.0).contains(&state) {
        return Err(GameUiError::parse(format!(
            "authenticated HUD command {ordinal} has invalid progress state bits 0x{bits:08x}"
        )));
    }
    Ok(state)
}

fn apply_clip(image: &mut RgbaImage, clip: UiPassClip) {
    let UiPassClip::Enabled { rect } = clip else {
        return;
    };
    for y in 0..image.height {
        for x in 0..image.width {
            if (x as i32) < rect.left_i32
                || (x as i32) >= rect.right_i32
                || (y as i32) < rect.top_i32
                || (y as i32) >= rect.bottom_i32
            {
                let offset = (y * image.width + x) * 4;
                image.pixels[offset..offset + 4].fill(0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn used(path: &str) -> replay_core::UsedContentKey {
        replay_core::UsedContentKey {
            effective_vfs_manifest_sha256: "11".repeat(32),
            normalized_game_relative_path: path.into(),
            winning_layer_ordinal: 0,
            layer_kind: replay_core::ContentLayerKind::Base,
            layer_id: "base".into(),
            source_sha256: "22".repeat(32),
            payload_sha256: "22".repeat(32),
            payload_bytes: 1,
        }
    }

    fn command_with_content(paths: &[&str]) -> HudCommand {
        let content = paths.iter().map(|path| used(path)).collect::<Vec<_>>();
        HudCommand {
            ordinal: 0,
            identity: replay_core::HudCommandIdentity {
                authored_surface_identity: "topbar".into(),
                runtime_root: "topbar".into(),
                selected_gui_definition: content[0].clone(),
                child_path: vec![0],
                element_type: "iconType".into(),
                name: "DateText".into(),
                paint_ordinal: 0,
            },
            placement: replay_core::HudPlacement {
                destination_rect: replay_core::UiPassRect {
                    left_i32: 0,
                    top_i32: 0,
                    right_i32: 1,
                    bottom_i32: 1,
                },
                clip_state: UiPassClip::Disabled,
                surface_paint_ordinal: 0,
            },
            execution: HudCommandExecution::InterpretedPaint {
                paint: HudPaintCommand::Text {
                    runtime_text_context_id: "test".into(),
                    runtime_text_frame_hour: 1,
                    runtime_text_pov_country_or_global: "global".into(),
                    runtime_text_view_id: "topbar".into(),
                    runtime_text_element_id: "DateText".into(),
                    rgba: [255, 255, 255, 255],
                    horizontal_alignment: TextHorizontalAlignment::Left,
                    layout_inputs: TextLayoutInputs::AuthoredReproducible {
                        authored_rect_offsets: TextRectOffsets {
                            left_i32: 0,
                            top_i32: 0,
                            right_i32: 1,
                            bottom_i32: 1,
                        },
                        max_width: Some(1),
                        wrap: false,
                        clip_rect_offsets: None,
                    },
                    font_bindings: vec![FontBinding {
                        font_identity: "test".into(),
                        definition_content_index: 1,
                        atlas_texture_content_indices: vec![2],
                    }],
                    inline_icon_bindings: Vec::new(),
                },
            },
            ordered_effective_vfs_used_content: content,
            command_identity_sha256: "33".repeat(32),
        }
    }

    #[test]
    fn progress_state_is_exact_and_fail_closed() {
        assert_eq!(exact_unit_state(0.5f32.to_bits(), 7).unwrap(), 0.5);
        assert!(exact_unit_state(f32::NAN.to_bits(), 7).is_err());
        assert!(exact_unit_state((-0.01f32).to_bits(), 7).is_err());
        assert!(exact_unit_state(1.01f32.to_bits(), 7).is_err());
    }

    #[test]
    fn clip_clears_every_pixel_outside_half_open_rect() {
        let mut image = RgbaImage {
            width: 3,
            height: 2,
            pixels: vec![255; 3 * 2 * 4],
        };
        apply_clip(
            &mut image,
            UiPassClip::Enabled {
                rect: replay_core::UiPassRect {
                    left_i32: 1,
                    top_i32: 0,
                    right_i32: 3,
                    bottom_i32: 1,
                },
            },
        );
        for y in 0..2 {
            for x in 0..3 {
                let offset = (y * 3 + x) * 4;
                let expected = if y == 0 && x >= 1 { 255 } else { 0 };
                assert_eq!(image.pixels[offset], expected);
                assert_eq!(image.pixels[offset + 3], expected);
            }
        }
    }

    #[test]
    fn indexed_content_join_never_falls_back_to_a_matching_path_elsewhere() {
        let command = command_with_content(&[
            "interface/root.gui",
            "interface/sprites.gfx",
            "gfx/interface/sprite.dds",
        ]);
        require_indexed_command_content(&command, 1, "interface/sprites.gfx", "definition")
            .unwrap();
        require_indexed_command_content(&command, 2, "gfx/interface/sprite.dds", "texture")
            .unwrap();
        assert!(require_indexed_command_content(
            &command,
            1,
            "gfx/interface/sprite.dds",
            "texture"
        )
        .is_err());
        assert!(require_indexed_command_content(
            &command,
            9,
            "gfx/interface/sprite.dds",
            "texture"
        )
        .is_err());
    }

    #[test]
    fn masked_layers_scale_mask_then_overlay_in_proven_order() {
        let base = RgbaImage {
            width: 1,
            height: 1,
            pixels: vec![200, 100, 50, 255],
        };
        let mask = RgbaImage {
            width: 2,
            height: 1,
            pixels: vec![0, 0, 0, 0, 128, 0, 0, 0],
        };
        let overlay = RgbaImage {
            width: 2,
            height: 1,
            pixels: vec![10, 20, 30, 255, 0, 0, 0, 0],
        };
        let composed = compose_masked_layers(&base, &mask, &overlay).unwrap();
        assert_eq!(&composed.pixels[0..4], &[10, 20, 30, 255]);
        assert_eq!(&composed.pixels[4..8], &[200, 100, 50, 128]);
    }

    #[test]
    fn runtime_text_join_requires_complete_exact_context() {
        let record = replay_core::RuntimeTextRecord {
            context_id: "text:1".into(),
            frame_hour: 1,
            pov_country_or_global: "global".into(),
            view_id: "topbar".into(),
            element_id: "DateText".into(),
            source: replay_core::RuntimeTextSource::Literal("date".into()),
            final_text_bytes: b"date".to_vec(),
            final_text_utf8: "date".into(),
            final_text_sha256: "00".repeat(32),
            variables: Default::default(),
            data_functions: Default::default(),
            dynamic_colors: Default::default(),
            layout: replay_core::RuntimeTextLayout::AuthoredReproducible,
            producer: replay_core::RuntimeTextProducer {
                setter_entry_rva: replay_core::A729_TEXT_SETTER_RVA,
                setter_caller_rva: None,
                formatter_entry_rva: None,
                identity_source: replay_core::RuntimeTextIdentitySource::AuthoredWidgetRegistration,
            },
        };
        let mut contexts = std::collections::BTreeMap::new();
        contexts.insert(record.context_id.clone(), record);
        let complete = RuntimeTextCatalog {
            contexts_complete: true,
            contexts: contexts.clone(),
        };
        let context = SelectedPresentPlaybackContext {
            frame_hour: 1,
            pov_country_or_global: "global",
        };
        let command = command_with_content(&[
            "interface/root.gui",
            "interface/font.fnt",
            "gfx/interface/font.dds",
        ]);
        assert_eq!(
            require_runtime_text_record(
                Some(&complete),
                "text:1",
                1,
                "global",
                "topbar",
                "DateText",
                context,
                &command,
            )
            .unwrap()
            .final_text_utf8,
            "date"
        );
        assert!(require_runtime_text_record(
            None, "text:1", 1, "global", "topbar", "DateText", context, &command
        )
        .is_err());
        assert!(require_runtime_text_record(
            Some(&RuntimeTextCatalog {
                contexts_complete: false,
                contexts,
            }),
            "text:1",
            1,
            "global",
            "topbar",
            "DateText",
            context,
            &command,
        )
        .is_err());
        assert!(require_runtime_text_record(
            Some(&complete),
            "missing",
            1,
            "global",
            "topbar",
            "DateText",
            context,
            &command,
        )
        .is_err());

        for mismatch in 0..4 {
            let mut collided = complete.clone();
            let record = collided.contexts.get_mut("text:1").unwrap();
            match mismatch {
                0 => record.frame_hour += 1,
                1 => record.pov_country_or_global = "GER".into(),
                2 => record.view_id = "diplomacy".into(),
                _ => record.element_id = "OtherText".into(),
            }
            assert!(
                require_runtime_text_record(
                    Some(&collided),
                    "text:1",
                    1,
                    "global",
                    "topbar",
                    "DateText",
                    context,
                    &command,
                )
                .is_err(),
                "runtime-text context collision {mismatch} was admitted"
            );
        }

        for (hour, pov, view, element) in [
            (2, "global", "topbar", "DateText"),
            (1, "GER", "topbar", "DateText"),
            (1, "global", "diplomacy", "DateText"),
            (1, "global", "topbar", "OtherText"),
        ] {
            assert!(require_runtime_text_record(
                Some(&complete),
                "text:1",
                hour,
                pov,
                view,
                element,
                context,
                &command,
            )
            .is_err());
        }
    }

    #[test]
    fn captured_runtime_coordinates_are_owner_top_left_relative_only() {
        let destination = replay_core::UiPassRect {
            left_i32: 100,
            top_i32: 200,
            right_i32: 300,
            bottom_i32: 400,
        };
        assert_eq!(
            captured_owner_relative_point(destination, -3, 4, "glyph").unwrap(),
            (97, 204)
        );
        assert_eq!(
            captured_owner_relative_point(destination, 9, 10, "inline icon").unwrap(),
            (109, 210)
        );
        // Captured baseline/advance are metrics, not a second translation or
        // replay-side DPI/GUI scale.
        let (_baseline, _advance) = (777, 333);
        assert_eq!(
            captured_owner_relative_point(destination, 9, 10, "inline icon").unwrap(),
            (109, 210)
        );
    }

    #[test]
    fn captured_runtime_clip_translates_intersects_and_is_half_open() {
        let destination = replay_core::UiPassRect {
            left_i32: 100,
            top_i32: 200,
            right_i32: 300,
            bottom_i32: 400,
        };
        let clip = captured_effective_clip(
            destination,
            Some([1, 2, 30, 40]),
            UiPassClip::Enabled {
                rect: replay_core::UiPassRect {
                    left_i32: 110,
                    top_i32: 190,
                    right_i32: 120,
                    bottom_i32: 220,
                },
            },
            500,
            500,
        )
        .unwrap();
        assert_eq!(
            clip,
            replay_core::UiPassRect {
                left_i32: 110,
                top_i32: 202,
                right_i32: 120,
                bottom_i32: 220,
            }
        );
        let mut image = RgbaImage {
            width: 121,
            height: 221,
            pixels: vec![255; 121 * 221 * 4],
        };
        apply_rect_clip(&mut image, clip);
        assert_eq!(image.pixels[(202 * 121 + 110) * 4], 255);
        assert_eq!(image.pixels[(202 * 121 + 120) * 4], 0);
        assert_eq!(image.pixels[(220 * 121 + 110) * 4], 0);
    }

    #[test]
    fn captured_runtime_clip_absence_negative_offsets_and_overflow_fail_closed() {
        let destination = replay_core::UiPassRect {
            left_i32: 100,
            top_i32: 200,
            right_i32: 300,
            bottom_i32: 400,
        };
        assert_eq!(
            captured_effective_clip(destination, None, UiPassClip::Disabled, 500, 600).unwrap(),
            replay_core::UiPassRect {
                left_i32: 0,
                top_i32: 0,
                right_i32: 500,
                bottom_i32: 600,
            }
        );
        assert_eq!(
            captured_effective_clip(
                destination,
                Some([-120, -220, 30, 40]),
                UiPassClip::Disabled,
                500,
                600,
            )
            .unwrap(),
            replay_core::UiPassRect {
                left_i32: 0,
                top_i32: 0,
                right_i32: 10,
                bottom_i32: 20,
            }
        );
        let overflowing = replay_core::UiPassRect {
            left_i32: i32::MAX,
            top_i32: 0,
            right_i32: i32::MAX,
            bottom_i32: 1,
        };
        assert!(captured_owner_relative_point(overflowing, 1, 0, "glyph").is_err());
        assert!(captured_effective_clip(
            overflowing,
            Some([1, 0, 1, 1]),
            UiPassClip::Disabled,
            1,
            1,
        )
        .is_err());
    }

    #[test]
    fn authored_wrap_preserves_explicit_lines_and_uses_measured_width() {
        let wrapped =
            crate::gui_scene_util::wrap_text_greedy("aa bb\ncc", 4, |value| value.len() as i32);
        assert_eq!(wrapped, "aa\nbb\ncc");
    }
}
