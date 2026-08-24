use super::{
    check_sha256, collect_static_draw_used_content, decimal_u64, validate_static_map_draw,
    validate_static_target, RenderTargetDesc, StaticMapDraw, UsedContentKey,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const UI_PASS_SCHEMA: &str = "hoi4.production-selected-present-ui-pass.v2";
pub const UI_A729_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticatedUiPass {
    pub schema: String,
    #[serde(with = "decimal_u64")]
    pub update_index: u64,
    #[serde(with = "decimal_u64")]
    pub first_sequence: u64,
    #[serde(with = "decimal_u64")]
    pub last_sequence: u64,
    pub ui_pass_draw_count: u32,
    pub target: RenderTargetDesc,
    pub selected_present_receipt_sha256: String,
    pub producer_selection_receipt_sha256: String,
    pub source_frontier_sha256: String,
    pub recorded_input_receipt_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_mapmode_state: Option<ActiveMapmodeState>,
    pub ui_pass_operations: Vec<UiPassOperation>,
    pub pass_identity_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum UiPassOperation {
    HudCommand {
        command: HudCommand,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HudCommand {
    pub ordinal: u32,
    pub identity: HudCommandIdentity,
    pub placement: HudPlacement,
    pub execution: HudCommandExecution,
    pub ordered_effective_vfs_used_content: Vec<UsedContentKey>,
    pub command_identity_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HudCommandIdentity {
    pub authored_surface_identity: String,
    pub runtime_root: String,
    pub selected_gui_definition: UsedContentKey,
    #[serde(with = "decimal_u64")]
    pub engine_instance_token: u64,
    pub child_path: Vec<u32>,
    pub element_type: String,
    pub name: String,
    pub paint_ordinal: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HudPlacement {
    pub destination_rect: UiPassRect,
    pub clip_state: UiPassClip,
    pub surface_paint_ordinal: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HudCommandExecution {
    ExactPipeline { draw: UiExactPipelineDraw },
    InterpretedPaint { paint: HudPaintCommand },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiExactPipelineDraw {
    pub exact_draw: StaticMapDraw,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HudPaintCommand {
    Sprite {
        sprite_id: String,
        resolved_frame: u32,
        law: SpriteLaw,
        sprite_definition_content_index: u32,
        texture_content_index: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        back_texture_content_index: Option<u32>,
    },
    Text {
        runtime_text_context_id: String,
        runtime_text_frame_hour: i64,
        runtime_text_pov_country_or_global: String,
        runtime_text_view_id: String,
        runtime_text_element_id: String,
        rgba: [u8; 4],
        horizontal_alignment: TextHorizontalAlignment,
        layout_inputs: TextLayoutInputs,
        font_bindings: Vec<FontBinding>,
        inline_icon_bindings: Vec<InlineIconBinding>,
    },
    MaskedSprite {
        sprite_id: String,
        mask_slot: u8,
        overlay_slot: u8,
        sprite_definition_content_index: u32,
        texture_content_index: u32,
        mask_texture_content_index: u32,
        overlay_texture_content_index: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextHorizontalAlignment {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TextLayoutInputs {
    AuthoredReproducible {
        authored_rect_offsets: TextRectOffsets,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_width: Option<u32>,
        wrap: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        clip_rect_offsets: Option<TextRectOffsets>,
    },
    CapturedRuntime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextRectOffsets {
    pub left_i32: i32,
    pub top_i32: i32,
    pub right_i32: i32,
    pub bottom_i32: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FontBinding {
    pub font_identity: String,
    pub definition_content_index: u32,
    pub atlas_texture_content_indices: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InlineIconBinding {
    pub sprite_identity: String,
    pub definition_content_index: u32,
    pub texture_content_indices: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SpriteLaw {
    Plain,
    Scaled,
    NineSlice { border_x: u32, border_y: u32 },
    ProgressLeftCrop { state_bits: u32 },
    ProgressMinMax { state_bits: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveMapmodeState {
    pub mode_token: String,
    pub button_ordinal: u32,
    pub authored_button_identity: String,
    pub layout_root_token: String,
    pub provenance: MapmodeProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MapmodeProvenance {
    EffectiveVfs {
        content: UsedContentKey,
    },
    EngineBuiltin {
        executable_sha256: String,
        registration_token: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum UiPassClip {
    Disabled,
    Enabled { rect: UiPassRect },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiPassRect {
    pub left_i32: i32,
    pub top_i32: i32,
    pub right_i32: i32,
    pub bottom_i32: i32,
}

impl UiPassOperation {
    pub fn ui_pass_ordinal(&self) -> u32 {
        match self {
            Self::HudCommand { command } => command.ordinal,
        }
    }
}

impl AuthenticatedUiPass {
    pub fn computed_identity_sha256(&self) -> Result<String, String> {
        #[derive(Serialize)]
        struct Payload<'a> {
            schema: &'a str,
            update_index: u64,
            first_sequence: u64,
            last_sequence: u64,
            ui_pass_draw_count: u32,
            target: &'a RenderTargetDesc,
            selected_present_receipt_sha256: &'a str,
            producer_selection_receipt_sha256: &'a str,
            source_frontier_sha256: &'a str,
            recorded_input_receipt_sha256: &'a str,
            active_mapmode_state: &'a Option<ActiveMapmodeState>,
            ui_pass_operations: &'a [UiPassOperation],
        }
        let bytes = serde_json::to_vec(&Payload {
            schema: &self.schema,
            update_index: self.update_index,
            first_sequence: self.first_sequence,
            last_sequence: self.last_sequence,
            ui_pass_draw_count: self.ui_pass_draw_count,
            target: &self.target,
            selected_present_receipt_sha256: &self.selected_present_receipt_sha256,
            producer_selection_receipt_sha256: &self.producer_selection_receipt_sha256,
            source_frontier_sha256: &self.source_frontier_sha256,
            recorded_input_receipt_sha256: &self.recorded_input_receipt_sha256,
            active_mapmode_state: &self.active_mapmode_state,
            ui_pass_operations: &self.ui_pass_operations,
        })
        .map_err(|error| error.to_string())?;
        let mut hasher = Sha256::new();
        hasher.update(b"hoi4.production-selected-present-ui-pass-identity.v2\0");
        hasher.update(bytes);
        Ok(format!("{:x}", hasher.finalize()))
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != UI_PASS_SCHEMA {
            return Err("unsupported selected-Present UI-pass schema".into());
        }
        if self.first_sequence > self.last_sequence {
            return Err("selected-Present UI-pass sequence range is reversed".into());
        }
        validate_static_target(&self.target, false)?;
        check_sha256(&self.selected_present_receipt_sha256, None)?;
        check_sha256(&self.producer_selection_receipt_sha256, None)?;
        check_sha256(&self.source_frontier_sha256, None)?;
        check_sha256(&self.recorded_input_receipt_sha256, None)?;
        if let Some(state) = &self.active_mapmode_state {
            state.validate()?;
        }
        if self.ui_pass_draw_count == 0
            || self.ui_pass_operations.len() != self.ui_pass_draw_count as usize
        {
            return Err("UI-pass operations do not exactly cover the draw frontier".into());
        }
        check_sha256(&self.pass_identity_sha256, None)?;

        let mut hashes = BTreeSet::new();
        let mut identities = BTreeSet::new();
        let mut root_by_owner = BTreeMap::new();
        let mut definition_by_owner = BTreeMap::new();
        let mut last_paint_by_owner = BTreeMap::<(String, u64, String), u32>::new();
        for (ordinal, operation) in self.ui_pass_operations.iter().enumerate() {
            if operation.ui_pass_ordinal() != ordinal as u32 {
                return Err("UI-pass operation ordinals are not exact 0..N-1 order".into());
            }
            match operation {
                UiPassOperation::HudCommand { command } => {
                    command.validate(&self.target)?;
                    if !hashes.insert(command.command_identity_sha256.clone())
                        || !identities.insert(command.identity.clone())
                    {
                        return Err("UI-pass HUD command identity is duplicated".into());
                    }
                    let surface = command.identity.authored_surface_identity.clone();
                    let owner = (surface.clone(), command.identity.engine_instance_token);
                    let root = command.identity.runtime_root.clone();
                    if root_by_owner
                        .insert(owner.clone(), root.clone())
                        .is_some_and(|prior| prior != root)
                    {
                        return Err(
                            "UI-pass HUD owner changes authenticated runtime root between segments"
                                .into(),
                        );
                    }
                    if definition_by_owner
                        .insert(owner, command.identity.selected_gui_definition.clone())
                        .is_some_and(|prior| prior != command.identity.selected_gui_definition)
                    {
                        return Err(
                            "UI-pass HUD owner changes selected GUI definition between segments"
                                .into(),
                        );
                    }
                    let paint_key = (surface, command.identity.engine_instance_token, root);
                    if last_paint_by_owner
                        .insert(paint_key, command.identity.paint_ordinal)
                        .is_some_and(|prior| command.identity.paint_ordinal <= prior)
                    {
                        return Err("UI-pass HUD surface paint order is not monotonic".into());
                    }
                }
            }
        }
        if self.computed_identity_sha256()? != self.pass_identity_sha256 {
            return Err("selected-Present UI-pass identity SHA-256 mismatch".into());
        }
        Ok(())
    }
}

impl HudCommand {
    pub fn computed_identity_sha256(&self, target: &RenderTargetDesc) -> Result<String, String> {
        #[derive(Serialize)]
        struct Payload<'a> {
            identity: &'a HudCommandIdentity,
            placement: &'a HudPlacement,
            execution: &'a HudCommandExecution,
            ordered_effective_vfs_used_content: &'a [UsedContentKey],
            target: &'a RenderTargetDesc,
        }
        let bytes = serde_json::to_vec(&Payload {
            identity: &self.identity,
            placement: &self.placement,
            execution: &self.execution,
            ordered_effective_vfs_used_content: &self.ordered_effective_vfs_used_content,
            target,
        })
        .map_err(|error| error.to_string())?;
        let mut hasher = Sha256::new();
        hasher.update(b"hoi4.production-hud-command-identity.v1\0");
        hasher.update(bytes);
        Ok(format!("{:x}", hasher.finalize()))
    }

    pub fn validate(&self, target: &RenderTargetDesc) -> Result<(), String> {
        self.identity.validate()?;
        self.placement.validate(target)?;
        if self.placement.surface_paint_ordinal != self.identity.paint_ordinal {
            return Err("HUD placement/identity paint ordinal mismatch".into());
        }
        check_sha256(&self.command_identity_sha256, None)?;
        if self.computed_identity_sha256(target)? != self.command_identity_sha256 {
            return Err("HUD command identity SHA-256 mismatch".into());
        }

        let mut paths = BTreeSet::new();
        for content in &self.ordered_effective_vfs_used_content {
            content
                .validate()
                .map_err(|error| format!("invalid UI-pass HUD UsedContentKey: {error}"))?;
            if !paths.insert(content.normalized_game_relative_path.as_str()) {
                return Err("UI-pass HUD used-content path is duplicated".into());
            }
        }
        if self.ordered_effective_vfs_used_content.first()
            != Some(&self.identity.selected_gui_definition)
        {
            return Err(
                "UI-pass HUD content order does not start with selected GUI definition".into(),
            );
        }

        match &self.execution {
            HudCommandExecution::ExactPipeline { draw } => {
                validate_static_map_draw(&draw.exact_draw)?;
                if draw.exact_draw.output_state.ordered_color_targets.len() != 1
                    || draw.exact_draw.output_state.ordered_color_targets.first() != Some(target)
                    || draw.exact_draw.output_state.depth_target.is_some()
                {
                    return Err("exact HUD pipeline does not join pass target".into());
                }
                self.placement.validate_against_draw(&draw.exact_draw)?;
                let mut expected = vec![self.identity.selected_gui_definition.clone()];
                for content in collect_static_draw_used_content(&draw.exact_draw) {
                    if !expected.contains(&content) {
                        expected.push(content);
                    }
                }
                if self.ordered_effective_vfs_used_content != expected {
                    return Err("exact HUD pipeline content order/cover is not exact".into());
                }
            }
            HudCommandExecution::InterpretedPaint { paint } => {
                if let HudPaintCommand::Text {
                    runtime_text_view_id,
                    runtime_text_element_id,
                    ..
                } = paint
                {
                    if runtime_text_view_id != &self.identity.runtime_root
                        || runtime_text_element_id != &self.identity.name
                    {
                        return Err(
                            "HUD text A25 view/element identity differs from dynamic command identity"
                                .into(),
                        );
                    }
                }
                paint.validate(&self.ordered_effective_vfs_used_content)?
            }
        }
        Ok(())
    }
}

impl HudCommandIdentity {
    fn validate(&self) -> Result<(), String> {
        if self.authored_surface_identity.trim().is_empty()
            || self.runtime_root.trim().is_empty()
            || self.engine_instance_token == 0
            || self.element_type.trim().is_empty()
            || self.name.trim().is_empty()
        {
            return Err("UI-pass HUD dynamic command identity is blank".into());
        }
        self.selected_gui_definition
            .validate()
            .map_err(|error| format!("invalid HUD GUI definition: {error}"))
    }
}

impl HudPlacement {
    fn validate(&self, target: &RenderTargetDesc) -> Result<(), String> {
        self.destination_rect
            .validate_within(target.width, target.height, "HUD destination")?;
        if let UiPassClip::Enabled { rect } = self.clip_state {
            rect.validate_within(target.width, target.height, "HUD clip")?;
        }
        Ok(())
    }

    fn validate_against_draw(&self, draw: &StaticMapDraw) -> Result<(), String> {
        let rect = self.destination_rect;
        let viewport = &draw.output_state.viewport;
        if viewport.top_left_x_f32_bits != (rect.left_i32 as f32).to_bits()
            || viewport.top_left_y_f32_bits != (rect.top_i32 as f32).to_bits()
            || viewport.width_f32_bits != ((rect.right_i32 - rect.left_i32) as f32).to_bits()
            || viewport.height_f32_bits != ((rect.bottom_i32 - rect.top_i32) as f32).to_bits()
        {
            return Err("exact HUD pipeline viewport disagrees with axis-aligned placement".into());
        }
        match (self.clip_state, &draw.output_state.scissor) {
            (UiPassClip::Disabled, None) => Ok(()),
            (UiPassClip::Enabled { rect }, Some(scissor))
                if rect.left_i32 == scissor.left_i32
                    && rect.top_i32 == scissor.top_i32
                    && rect.right_i32 == scissor.right_i32
                    && rect.bottom_i32 == scissor.bottom_i32 =>
            {
                Ok(())
            }
            _ => Err("exact HUD pipeline scissor disagrees with clip placement".into()),
        }
    }
}

impl HudPaintCommand {
    fn validate(&self, ordered_content: &[UsedContentKey]) -> Result<(), String> {
        let required_indices = match self {
            Self::Sprite { sprite_id, .. } if sprite_id.trim().is_empty() => {
                return Err("HUD sprite paint has blank sprite identity".into())
            }
            Self::Text {
                runtime_text_context_id,
                runtime_text_pov_country_or_global,
                runtime_text_view_id,
                runtime_text_element_id,
                ..
            } if runtime_text_context_id.trim().is_empty()
                || runtime_text_pov_country_or_global.trim().is_empty()
                || runtime_text_view_id.trim().is_empty()
                || runtime_text_element_id.trim().is_empty() =>
            {
                return Err("HUD text paint has blank runtime-text identity token".into())
            }
            Self::MaskedSprite {
                sprite_id,
                mask_slot,
                overlay_slot,
                ..
            } if sprite_id.trim().is_empty() || mask_slot == overlay_slot => {
                return Err("HUD masked-sprite paint identity/slots are invalid".into())
            }
            Self::Sprite {
                law,
                sprite_definition_content_index,
                texture_content_index,
                back_texture_content_index,
                ..
            } => {
                let mut indices = vec![*sprite_definition_content_index, *texture_content_index];
                match (law, back_texture_content_index) {
                    (SpriteLaw::ProgressMinMax { .. }, Some(index)) => indices.push(*index),
                    (SpriteLaw::ProgressMinMax { .. }, None) => {
                        return Err(
                            "HUD progress-minmax paint lacks exact back-texture content index"
                                .into(),
                        )
                    }
                    (_, Some(_)) => {
                        return Err(
                            "HUD non-progress-minmax paint carries an unproven back texture".into(),
                        )
                    }
                    (_, None) => {}
                }
                if indices.iter().copied().collect::<BTreeSet<_>>().len() != indices.len() {
                    return Err("HUD sprite paint content indices are not distinct".into());
                }
                indices
            }
            Self::Text {
                layout_inputs,
                font_bindings,
                inline_icon_bindings,
                ..
            } => {
                layout_inputs.validate()?;
                if font_bindings.is_empty() {
                    return Err("HUD text paint has no exact font binding".into());
                }

                let mut identities = BTreeSet::new();
                let mut indices = Vec::new();
                for binding in font_bindings {
                    if binding.font_identity.trim().is_empty()
                        || !identities.insert(binding.font_identity.as_str())
                        || binding.atlas_texture_content_indices.is_empty()
                    {
                        return Err("HUD text font binding identity/atlas list is invalid".into());
                    }
                    indices.push(binding.definition_content_index);
                    indices.extend(binding.atlas_texture_content_indices.iter().copied());
                }

                identities.clear();
                for binding in inline_icon_bindings {
                    if binding.sprite_identity.trim().is_empty()
                        || !identities.insert(binding.sprite_identity.as_str())
                        || binding.texture_content_indices.is_empty()
                    {
                        return Err(
                            "HUD text inline-icon binding identity/texture list is invalid".into(),
                        );
                    }
                    indices.push(binding.definition_content_index);
                    indices.extend(binding.texture_content_indices.iter().copied());
                }

                if indices.iter().copied().collect::<BTreeSet<_>>().len() != indices.len() {
                    return Err("HUD text binding content indices are not distinct".into());
                }
                if indices
                    .iter()
                    .any(|index| *index == 0 || (*index as usize) >= ordered_content.len())
                {
                    return Err("HUD interpreted-paint content index is invalid".into());
                }
                let exact_order = (1..ordered_content.len() as u32).collect::<Vec<_>>();
                if indices != exact_order {
                    return Err("HUD text binding content order/cover is not exact".into());
                }
                indices
            }
            Self::MaskedSprite {
                sprite_definition_content_index,
                texture_content_index,
                mask_texture_content_index,
                overlay_texture_content_index,
                ..
            } => vec![
                *sprite_definition_content_index,
                *texture_content_index,
                *mask_texture_content_index,
                *overlay_texture_content_index,
            ],
        };
        let referenced = required_indices.iter().copied().collect::<BTreeSet<_>>();
        if referenced.contains(&0)
            || referenced
                .iter()
                .any(|index| (*index as usize) >= ordered_content.len())
        {
            return Err("HUD interpreted-paint content index is invalid".into());
        }
        let exact = (1..ordered_content.len() as u32).collect::<BTreeSet<_>>();
        if referenced != exact {
            return Err("HUD interpreted-paint content join is not exact".into());
        }
        Ok(())
    }
}

impl TextLayoutInputs {
    fn validate(&self) -> Result<(), String> {
        match self {
            Self::AuthoredReproducible {
                authored_rect_offsets,
                max_width,
                clip_rect_offsets,
                ..
            } => {
                authored_rect_offsets.validate("authored text")?;
                if max_width == &Some(0) {
                    return Err("HUD authored text max width is zero".into());
                }
                if let Some(offsets) = clip_rect_offsets {
                    offsets.validate("authored text clip")?;
                }
                Ok(())
            }
            Self::CapturedRuntime => Ok(()),
        }
    }
}

impl TextRectOffsets {
    fn validate(self, name: &str) -> Result<(), String> {
        if self.right_i32 <= self.left_i32 || self.bottom_i32 <= self.top_i32 {
            return Err(format!("HUD {name} offsets are invalid"));
        }
        Ok(())
    }
}

impl ActiveMapmodeState {
    fn validate(&self) -> Result<(), String> {
        if self.mode_token.trim().is_empty()
            || self.authored_button_identity.trim().is_empty()
            || self.layout_root_token.trim().is_empty()
        {
            return Err("active mapmode identity/layout root is blank".into());
        }
        match &self.provenance {
            MapmodeProvenance::EffectiveVfs { content } => content
                .validate()
                .map_err(|error| format!("invalid active mapmode UsedContentKey: {error}")),
            MapmodeProvenance::EngineBuiltin {
                executable_sha256,
                registration_token,
            } => {
                check_sha256(executable_sha256, None)?;
                if executable_sha256 != UI_A729_EXECUTABLE_SHA256
                    || registration_token.trim().is_empty()
                {
                    return Err("active mapmode engine-builtin provenance is not exact a729".into());
                }
                Ok(())
            }
        }
    }
}

impl UiPassRect {
    fn validate_within(self, width: u32, height: u32, name: &str) -> Result<(), String> {
        if self.left_i32 < 0
            || self.top_i32 < 0
            || self.right_i32 <= self.left_i32
            || self.bottom_i32 <= self.top_i32
            || u32::try_from(self.right_i32)
                .ok()
                .is_none_or(|right| right > width)
            || u32::try_from(self.bottom_i32)
                .ok()
                .is_none_or(|bottom| bottom > height)
        {
            return Err(format!("{name} rect is invalid"));
        }
        Ok(())
    }
}
