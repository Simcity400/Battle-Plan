
use crate::{
    derive_xmap15_orders, FoundationFeatureContext, FoundationFeatureOutput, GameFileResolver,
    ProductionMapFeatureAdapter, ProvinceAnchors, Result, Xmap15PlanProvider,
};
use portable_render_stream::{
    AssignedStage, FamilyDisposition, FoundationGap, MapVisualFamily, UsedContentKey,
    Xmap15PlanKey, Xmap15PreparedFrame, Xmap15PreparedRow, Xmap15SemanticFrame,
    XMAP15_STATE_FIRST_SCHEMA,
};
use replay_core::{
    ArmyBattleplanGeometry, ArmyBattleplanOrderKey, ArmyBattleplanOwnerKind,
    ValidatedContentAdmission, ViewStatus, WorldFrame,
};
use serde::{Deserialize, Serialize};
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};

pub const XMAP15_MAPARROWS_PATH: &str = "gfx/maparrows/maparrows.txt";
pub const XMAP15_MAPARROW_SHADER_PATH: &str = "gfx/FX/maparrow.shader";
pub const XMAP15_MOVEMENT_DEFINITIONS_PATH: &str = "interface/arrows.gfx";
pub const XMAP15_MOVEMENT_EFFECT_PATH: &str = "gfx/FX/arrow.lua";
pub const XMAP15_MOVEMENT_SHADER_PATH: &str = "gfx/FX/arrow.shader";
pub const XMAP15_HEIGHTMAP_PATH: &str = "map/heightmap.bmp";
pub const XMAP15_MAP_DIMENSIONS_PATH: &str = "map/provinces.bmp";
pub const XMAP15_ASSIGNED_STAGE_NAME: &str = "orders_fronts_movement";
pub const XMAP15_ASSIGNED_OPERATION_NAME: &str = "orders.fronts-movement.draw";
const XMAP15_AUTHORITY_GAP_ID: &str = "orders.army_battleplans_semantic";
const XMAP15_REGISTRATION_EVIDENCE: &str =
    "exact-a729:c71e31a51db89ab76fe34ef07cd86285a72efdb0c7d21087cd324ec587b89241";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap15ViewerSelection {
    pub selected_plan: Option<Xmap15PlanKey>,
}

pub fn xmap15_viewer_selection(
    world: &WorldFrame,
    selected: Option<&ArmyBattleplanOrderKey>,
) -> Xmap15ViewerSelection {
    let selected_plan = selected.and_then(|selected| {
        let census = world.army_battleplans();
        if census.status != ViewStatus::Present { return None; }
        let mut matches = census.orders.iter().filter(|order|
            &order.key == selected && (1..=4).contains(&order.order_type));
        let order = matches.next()?;
        if matches.next().is_some() { return None; }
        Some(Xmap15PlanKey {
            owner_kind: match selected.owner.kind {
                ArmyBattleplanOwnerKind::Army => portable_render_stream::Xmap15OwnerKind::Army,
                ArmyBattleplanOwnerKind::ArmyGroup => portable_render_stream::Xmap15OwnerKind::ArmyGroup,
            },
            owner_id: selected.owner.id,
            engine_order_instance_id: selected.engine_order_instance_id,
            generation: selected.generation,
            slot_index: order.slot_index,
        })
    });
    Xmap15ViewerSelection { selected_plan }
}

pub trait Xmap15UnitIdentityProvider {
    fn hour_index(&self) -> i64;
    fn census_sha256(&self) -> &str;
    fn admission_sha256(&self) -> &str;
    fn formation_identity_match_count(
        &self,
        owner_is_army_group: bool,
        owner_id: u32,
        owner_country: &str,
    ) -> usize;
    fn division_identity_match_count(&self, stable_id: u32, owner_country: &str) -> usize;
}

pub struct PreparedXmap15Orders {
    pub plans: Xmap15PlanProvider,
    pub native: Option<PreparedXmap15Frame>,
}

#[derive(Clone)]
pub(crate) struct Xmap15OrdersAdapter {
    resolver: GameFileResolver,
    admission: ValidatedContentAdmission,
    /// The effective map's own province positions, decoded once per session:
    /// they are what a recorded route province id stands for, and re-decoding
    /// `provinces.bmp` per frame would put a bitmap walk on the hot path.
    anchors: std::sync::Arc<std::sync::OnceLock<std::result::Result<ProvinceAnchors, String>>>,
}

impl Xmap15OrdersAdapter {
    pub(crate) fn new(resolver: &GameFileResolver, admission: &ValidatedContentAdmission) -> Self {
        Self {
            resolver: resolver.clone(),
            admission: admission.clone(),
            anchors: std::sync::Arc::new(std::sync::OnceLock::new()),
        }
    }

    fn province_anchors(&self) -> std::result::Result<&ProvinceAnchors, String> {
        self.anchors
            .get_or_init(|| {
                crate::province_anchors_from_resolver(&self.resolver)
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)
    }
}

impl ProductionMapFeatureAdapter for Xmap15OrdersAdapter {
    fn family(&self) -> MapVisualFamily {
        MapVisualFamily::OrdersFrontsArrows
    }

    fn prepare(&self, context: &FoundationFeatureContext<'_>) -> Result<FoundationFeatureOutput> {
        let assigned_stage = context
            .require_assigned_stage(XMAP15_ASSIGNED_STAGE_NAME, XMAP15_ASSIGNED_OPERATION_NAME)?
            .clone();
        let authority_gap = context
            .map_authority_gaps
            .iter()
            .find(|gap| gap.input_id == XMAP15_AUTHORITY_GAP_ID)
            .ok_or("XMAP15 canonical map-authority gap is absent")?;
        let registration = xmap15_registration_sha256();
        let anchors = self.province_anchors();
        let order_derivation = derive_xmap15_orders(context.world_frame, anchors.as_ref().ok().copied());
        order_derivation.validate(context.world_frame.hour_index())?;
        let unavailable = |detail: &str| FoundationFeatureOutput {
            disposition: FamilyDisposition::Unavailable {
                gap: xmap15_unavailable_gap(
                    context.world_frame.hour_index(),
                    authority_gap,
                    detail,
                ),
            },
            prepared_terrain: None,
            prepared_political: None,
            prepared_thin_borders: None,
            prepared_lighting: None,
            prepared_weather: None,
            prepared_rivers_straits: None,
            prepared_water: None,
            prepared_units: None,
            prepared_combat: None,
            prepared_orders: Some(PreparedXmap15Orders {
                plans: order_derivation.clone(),
                native: None,
            }),
            prepared_rail_supply: None,
            prepared_map_modes: None,
            element_diagnostics: Vec::new(),
            consumed_gap_ids: vec![XMAP15_AUTHORITY_GAP_ID.into()],
        };

        if let Err(error) = anchors {
            return Ok(unavailable(&format!(
                "authenticated province anchors unavailable: {error}; {}",
                xmap15_order_derivation_account(context.world_frame, &order_derivation),
            )));
        }

        if semantic_value(context.world_frame).is_none() {
            // WHY the key is absent is the recorder's answer. A recording made
            // by a recorder that has the XMAP15 producer carries the gate
            // beside where the key would be, and that gate is the hour's own
            // account: how much of the plan census the producer minted, against
            // how many visual d54 orders, and which of its recorded inputs the
            // hour did not carry.
            return Ok(unavailable(&format!(
                "this recording carries no key {}: the unconditional current-hour XMAP15 \
                 semantic sidecar is absent; {}; {}",
                replay_core::XMAP15_ORDERS_FRONTS_MOVEMENT_SEMANTIC_PATH.join("."),
                xmap15_unavailable_account(context.world_frame),
                xmap15_order_derivation_account(context.world_frame, &order_derivation)
            )));
        }
        let Some(units) = context.xmap15_units else {
            return Ok(unavailable(
                "the exact current-hour XMAP13 division identity provider is absent",
            ));
        };

        let foundation_target_sha256 = Sha256::hex(
            format!(
                "{}\0{}\0{}",
                context.viewer.viewport_width,
                context.viewer.viewport_height,
                context
                    .viewer
                    .camera
                    .as_ref()
                    .map(|camera| camera.deterministic_sha256.as_str())
                    .unwrap_or("<missing>")
            )
            .as_bytes(),
        );
        let prepared = prepare_xmap15_frame(
            &self.resolver,
            &self.admission,
            context.world_frame,
            units,
            context.xmap15_selection,
            assigned_stage,
            foundation_target_sha256,
        )?;
        let disposition = if prepared.portable.ordered_rows.is_empty() {
            FamilyDisposition::AuthenticatedEmpty {
                producer_registration_sha256: registration,
                enumerator_witness_sha256: prepared.portable.enumerator_witness_sha256.clone(),
            }
        } else {
            let resources_sha256 =
                xmap15_used_content_sha256(&prepared.portable.ordered_used_content)?;
            FamilyDisposition::Present {
                payload_schema: XMAP15_STATE_FIRST_SCHEMA.into(),
                producer_registration_sha256: registration.clone(),
                payload_sha256: prepared.portable.deterministic_sha256.clone(),
                ordered_operations: vec![prepared
                    .assigned_stage
                    .foundation_operation_identity(
                        registration,
                        resources_sha256,
                        prepared.foundation_target_sha256.clone(),
                    )
                    .map_err(|error| format!("XMAP15 assigned-stage identity: {error}"))?],
                ordered_used_content: prepared.portable.ordered_used_content.clone(),
            }
        };
        Ok(FoundationFeatureOutput {
            disposition,
            prepared_terrain: None,
            prepared_political: None,
            prepared_thin_borders: None,
            prepared_lighting: None,
            prepared_weather: None,
            prepared_rivers_straits: None,
            prepared_water: None,
            prepared_units: None,
            prepared_combat: None,
            prepared_orders: Some(PreparedXmap15Orders {
                plans: order_derivation,
                native: Some(prepared),
            }),
            prepared_rail_supply: None,
            prepared_map_modes: None,
            element_diagnostics: Vec::new(),
            consumed_gap_ids: vec![XMAP15_AUTHORITY_GAP_ID.into()],
        })
    }
}

pub(crate) fn xmap15_used_content_sha256(content: &[UsedContentKey]) -> Result<String> {
    if content.is_empty() {
        return Err("XMAP15 effective-VFS content closure is empty".into());
    }
    Ok(Sha256::hex(&serde_json::to_vec(content)?))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedXmap15ParadropProfile {
    pub parameters: portable_render_stream::Xmap15ParadropProfile,
    pub ordered_used_content: Vec<UsedContentKey>,
}

fn paradrop_profile_from_effective(
    evaluated: &crate::xmap09_lighting_law::EffectiveDefines,
) -> Result<portable_render_stream::Xmap15ParadropProfile> {
    let profile = portable_render_stream::Xmap15ParadropProfile {
        height_to_length_f32_bits: evaluated.scalar("NGraphics", "ARROW_PARADROP_HEIGHT_TO_LEN")?.to_bits(),
        max_height_f32_bits: evaluated.scalar("NGraphics", "ARROW_PARADROP_HEIGHT_MAX")?.to_bits(),
    };
    profile.validate()?;
    Ok(profile)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedXmap15Frame {
    pub portable: Xmap15PreparedFrame,
    pub assigned_stage: AssignedStage,
    pub foundation_target_sha256: String,
    pub plan_definitions: Vec<PreparedXmap15PlanDefinition>,
    pub movement_definitions: Vec<PreparedXmap15MovementDefinition>,
    pub heightmap: Option<UsedContentKey>,
    pub paradrop_profile: Option<PreparedXmap15ParadropProfile>,
    pub map_dimensions_content: Option<UsedContentKey>,
    pub map_width: u32,
    pub map_height: u32,
}

impl PreparedXmap15Frame {
    pub fn validate(&self) -> Result<()> {
        self.portable
            .validate()
            .map_err(|error| format!("XMAP15 portable frame: {error}"))?;
        self.assigned_stage
            .validate()
            .map_err(|error| format!("XMAP15 assigned stage: {error}"))?;
        if self.assigned_stage.stage_name != XMAP15_ASSIGNED_STAGE_NAME
            || self.assigned_stage.operation_name != XMAP15_ASSIGNED_OPERATION_NAME
            || self.foundation_target_sha256.len() != 64
            || !self
                .foundation_target_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("XMAP15 assigned stage or foundation target identity drifted".into());
        }
        let has_rows = !self.portable.ordered_rows.is_empty();
        if self.map_dimensions_content.is_some() != has_rows
            || has_rows != (self.map_width > 0 && self.map_height > 0)
            || self.map_dimensions_content.as_ref().is_some_and(|content| {
                !content
                    .normalized_game_relative_path
                    .eq_ignore_ascii_case(XMAP15_MAP_DIMENSIONS_PATH)
            })
        {
            return Err("XMAP15 effective map-dimension identity is invalid".into());
        }
        let plan = self
            .plan_definitions
            .iter()
            .map(|definition| {
                (
                    definition.name.as_str(),
                    definition.deterministic_sha256.as_str(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let movement = self
            .movement_definitions
            .iter()
            .map(|definition| {
                (
                    definition.name.as_str(),
                    definition.deterministic_sha256.as_str(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        if plan.len() != self.plan_definitions.len()
            || movement.len() != self.movement_definitions.len()
            || self
                .plan_definitions
                .iter()
                .any(|value| value.validate().is_err())
            || self
                .movement_definitions
                .iter()
                .any(|value| value.validate().is_err())
        {
            return Err("XMAP15 effective definitions are invalid or duplicated".into());
        }
        let needs_paradrop = self.portable.ordered_rows.iter().any(|row| matches!(row,
            Xmap15PreparedRow::Plan { semantic, .. }
                if semantic.selector.order_type == 4 && semantic.route.explicit_height_f32_bits.is_none()));
        if self.paradrop_profile.is_some() != needs_paradrop {
            return Err("XMAP15 paradrop profile closure does not match native callback presence".into());
        }
        if let Some(profile) = &self.paradrop_profile {
            profile.parameters.validate()?;
            if profile.ordered_used_content.is_empty()
                || profile.ordered_used_content.iter().any(|key| {
                    let path = key.normalized_game_relative_path.to_ascii_lowercase();
                    !path.starts_with("common/defines/") || !path.ends_with(".lua")
                }) {
                return Err("XMAP15 paradrop profile lacks its authenticated defines closure".into());
            }
        }
        let exact_content = self
            .plan_definitions
            .iter()
            .flat_map(|definition| definition.ordered_used_content.iter().cloned())
            .chain(
                self.movement_definitions
                    .iter()
                    .flat_map(|definition| definition.ordered_used_content.iter().cloned()),
            )
            .chain(self.paradrop_profile.iter().flat_map(|profile| profile.ordered_used_content.iter().cloned()))
            .chain(self.heightmap.iter().cloned())
            .chain(self.map_dimensions_content.iter().cloned())
            .collect::<BTreeSet<_>>();
        let declared_content = self
            .portable
            .ordered_used_content
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        if exact_content != declared_content {
            return Err("XMAP15 frame content is not the exact definition closure".into());
        }
        if self.heightmap.is_some()
            != self
                .portable
                .ordered_rows
                .iter()
                .any(|row| matches!(row, Xmap15PreparedRow::Plan { .. }))
        {
            return Err("XMAP15 heightmap closure does not match plan presence".into());
        }
        for row in &self.portable.ordered_rows {
            let (name, digest, actual) = match row {
                Xmap15PreparedRow::Plan {
                    semantic,
                    definition_sha256,
                } => (
                    semantic.resolved_definition_name.as_str(),
                    definition_sha256,
                    plan.get(semantic.resolved_definition_name.as_str()),
                ),
                Xmap15PreparedRow::Movement {
                    semantic,
                    definition_sha256,
                } => (
                    semantic.definition.definition_name(),
                    definition_sha256,
                    movement.get(semantic.definition.definition_name()),
                ),
            };
            if actual.copied() != Some(digest.as_str()) {
                return Err(
                    format!("XMAP15 row definition {name} has no exact VFS profile").into(),
                );
            }
            if let Xmap15PreparedRow::Plan { semantic, .. } = row {
                let definition = self
                    .plan_definitions
                    .iter()
                    .find(|definition| definition.name == semantic.resolved_definition_name)
                    .expect("digest join proved definition presence");
                let variant = usize::from(semantic.texture_variant);
                if variant >= definition.ordered_texture_masks.len()
                    || variant >= definition.ordered_texture_patterns.len()
                {
                    return Err(format!(
                        "XMAP15 texture variant {} is outside effective definition {name}",
                        semantic.texture_variant
                    )
                    .into());
                }
                let headed = definition.head_length_f32_bits
                    .is_some_and(|bits| f32::from_bits(bits) > 0.0);
                let native_paradrop = semantic.selector.order_type == 4
                    && semantic.route.explicit_height_f32_bits.is_none()
                    && self.paradrop_profile.is_some();
                if semantic.route.explicit_height_f32_bits.is_none()
                    && !semantic.uses_native_terrain_height() && !native_paradrop {
                    return Err(format!("XMAP15 native height callback inputs are unavailable for {name}").into());
                }
                if headed && semantic.exact_head_vertices_f32_bits.is_none()
                    && !semantic.uses_native_terrain_height() && !native_paradrop {
                    return Err(format!("XMAP15 native height callback lacks exact head inputs for {name}").into());
                }
                if !headed && semantic.exact_head_vertices_f32_bits.is_some() {
                    return Err(format!("XMAP15 no-head definition carries head vertices for {name}").into());
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Xmap15LifecycleState {
    hour_index: Option<i64>,
    observation_sequence: Option<u64>,
    selected_plan: Option<Xmap15PlanKey>,
    retained_rows: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Xmap15LifecycleUpdate {
    pub hour_index: i64,
    pub observation_sequence: u64,
    pub created: Vec<String>,
    pub edited: Vec<String>,
    pub deleted: Vec<String>,
    pub selected_plan_changed: bool,
    pub authenticated_empty: bool,
    pub deterministic_sha256: String,
}

impl Xmap15LifecycleState {
    pub fn apply(&mut self, frame: &PreparedXmap15Frame) -> Result<Xmap15LifecycleUpdate> {
        frame.validate()?;
        let mut incoming = BTreeMap::new();
        for row in &frame.portable.ordered_rows {
            let key = prepared_row_key(row);
            let digest = Sha256::hex(&serde_json::to_vec(row)?);
            if incoming.insert(key, digest).is_some() {
                return Err("XMAP15 lifecycle input repeats a stable row".into());
            }
        }
        let created = incoming
            .keys()
            .filter(|key| !self.retained_rows.contains_key(*key))
            .cloned()
            .collect::<Vec<_>>();
        let edited = incoming
            .iter()
            .filter(|(key, digest)| {
                self.retained_rows
                    .get(*key)
                    .is_some_and(|old| old != *digest)
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        let deleted = self
            .retained_rows
            .keys()
            .filter(|key| !incoming.contains_key(*key))
            .cloned()
            .collect::<Vec<_>>();
        let selected_plan_changed = self.selected_plan != frame.portable.selected_plan;
        let mut update = Xmap15LifecycleUpdate {
            hour_index: frame.portable.hour_index,
            observation_sequence: frame.portable.observation_sequence,
            created,
            edited,
            deleted,
            selected_plan_changed,
            authenticated_empty: incoming.is_empty(),
            deterministic_sha256: String::new(),
        };
        update.deterministic_sha256 = lifecycle_update_sha256(&update)?;
        self.hour_index = Some(frame.portable.hour_index);
        self.observation_sequence = Some(frame.portable.observation_sequence);
        self.selected_plan = frame.portable.selected_plan.clone();
        self.retained_rows = incoming;
        Ok(update)
    }
}

pub(crate) fn prepared_row_key(row: &Xmap15PreparedRow) -> String {
    match row {
        Xmap15PreparedRow::Plan { semantic, .. } => format!(
            "plan:{:?}:{}:{}:{}:{}",
            semantic.key.owner_kind,
            semantic.key.owner_id,
            semantic.key.engine_order_instance_id,
            semantic.key.generation,
            semantic.key.slot_index
        ),
        Xmap15PreparedRow::Movement { semantic, .. } => format!(
            "movement:{}:{}:{}",
            semantic.key.owner_country, semantic.key.division_id, semantic.key.movement_generation
        ),
    }
}

fn lifecycle_update_sha256(update: &Xmap15LifecycleUpdate) -> Result<String> {
    let mut unsigned = update.clone();
    unsigned.deterministic_sha256.clear();
    Ok(Sha256::hex(&serde_json::to_vec(&unsigned)?))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedXmap15PlanDefinition {
    pub name: String,
    pub definition_index: u32,
    pub technique: String,
    pub body_width_f32_bits: [u32; 2],
    pub body_uv_scale_f32_bits: u32,
    pub body_uv_scale_2_f32_bits: Option<u32>,
    pub mask_inner_f32_bits: [u32; 3],
    pub mask_outer_f32_bits: [u32; 3],
    pub width_scale_f32_bits: u32,
    pub width_scale_max_f32_bits: Option<u32>,
    pub head_length_f32_bits: Option<u32>,
    pub texture_loop_f32_bits: Option<u32>,
    pub fade_in_f32_bits: u32,
    pub fade_out_f32_bits: u32,
    pub global_scale_min_size_f32_bits: u32,
    pub global_scale_max_points: u32,
    pub texture_anim_speed_f32_bits: Option<u32>,
    pub texture_variant_count: Option<u32>,
    pub culling_direction: Option<i32>,
    pub draw_last: bool,
    pub draw_shadows: bool,
    pub ordered_texture_masks: Vec<String>,
    pub ordered_texture_patterns: Vec<String>,
    pub ordered_shader_translation_unit_paths: Vec<String>,
    pub ordered_used_content: Vec<UsedContentKey>,
    pub deterministic_sha256: String,
}

impl PreparedXmap15PlanDefinition {
    fn validate(&self) -> Result<()> {
        let finite = |bits: u32| f32::from_bits(bits).is_finite();
        if self.name.is_empty()
            || self.technique.is_empty()
            || !xmap15_plan_technique_supported(&self.technique)
            || self
                .body_width_f32_bits
                .into_iter()
                .any(|bits| !finite(bits))
            || !finite(self.body_uv_scale_f32_bits)
            || self
                .body_uv_scale_2_f32_bits
                .is_some_and(|bits| !finite(bits))
            || self
                .mask_inner_f32_bits
                .into_iter()
                .any(|bits| !finite(bits))
            || self
                .mask_outer_f32_bits
                .into_iter()
                .any(|bits| !finite(bits))
            || !finite(self.width_scale_f32_bits)
            || self
                .width_scale_max_f32_bits
                .is_some_and(|bits| !finite(bits))
            || self.head_length_f32_bits.is_some_and(|bits| !finite(bits))
            || self.texture_loop_f32_bits.is_some_and(|bits| !finite(bits))
            || !finite(self.fade_in_f32_bits)
            || !finite(self.fade_out_f32_bits)
            || !finite(self.global_scale_min_size_f32_bits)
            || self
                .texture_anim_speed_f32_bits
                .is_some_and(|bits| !finite(bits))
            || self.texture_variant_count == Some(0)
            || self.ordered_texture_masks.len() != self.ordered_texture_patterns.len()
            || self.texture_variant_count.is_some_and(|count| {
                usize::try_from(count).ok() != Some(self.ordered_texture_masks.len())
            })
            || self.ordered_texture_masks.is_empty()
            || self.ordered_texture_patterns.is_empty()
            || self.ordered_shader_translation_unit_paths.is_empty()
            || self
                .ordered_shader_translation_unit_paths
                .last()
                .is_none_or(|path| !path.eq_ignore_ascii_case(XMAP15_MAPARROW_SHADER_PATH))
            || self.deterministic_sha256 != definition_sha256(self)?
        {
            return Err(format!("XMAP15 plan definition {} is invalid", self.name).into());
        }
        let expected = std::iter::once(XMAP15_MAPARROWS_PATH)
            .chain(
                self.ordered_shader_translation_unit_paths
                    .iter()
                    .map(String::as_str),
            )
            .chain(self.ordered_texture_masks.iter().map(String::as_str))
            .chain(self.ordered_texture_patterns.iter().map(String::as_str))
            .map(str::to_ascii_lowercase)
            .collect::<BTreeSet<_>>();
        let actual = self
            .ordered_used_content
            .iter()
            .map(|content| content.normalized_game_relative_path.to_ascii_lowercase())
            .collect::<BTreeSet<_>>();
        if expected != actual || actual.len() != self.ordered_used_content.len() {
            return Err(format!(
                "XMAP15 plan definition {} content closure is invalid",
                self.name
            )
            .into());
        }
        Ok(())
    }

    pub fn geometry_profile(&self) -> Result<portable_render_stream::Xmap15PlanGeometryProfile> {
        self.validate()?;
        let profile = portable_render_stream::Xmap15PlanGeometryProfile {
            body_start_width_f32_bits: self.body_width_f32_bits[0],
            body_end_width_f32_bits: self.body_width_f32_bits[1],
            runtime_width_scale_f32_bits: self.width_scale_f32_bits,
            body_uv_scale_f32_bits: self.body_uv_scale_f32_bits,
            width_class_zero_f32_bits: self.mask_inner_f32_bits[0],
            width_class_one_f32_bits: self.mask_inner_f32_bits[1],
            global_scale_min_size_f32_bits: self.global_scale_min_size_f32_bits,
            global_scale_max_points: self.global_scale_max_points,
            head_length_f32_bits: self.head_length_f32_bits.unwrap_or(0.0f32.to_bits()),
        };
        profile
            .validate()
            .map_err(|error| -> Box<dyn std::error::Error> {
                format!("XMAP15 geometry profile: {error}").into()
            })?;
        Ok(profile)
    }

    pub fn effective_texture_variant_count(&self) -> Result<u32> {
        self.validate()?;
        Ok(self
            .texture_variant_count
            .unwrap_or(u32::try_from(self.ordered_texture_masks.len())?))
    }
}

fn xmap15_plan_technique_supported(technique: &str) -> bool {
    matches!(
        technique,
        "MapArrowDefault"
            | "MapArrowDefaultWithDepth"
            | "MapArrowDefaultWithDepthTestOnly"
            | "MapArrowNoHeadWidthDepth"
            | "MapArrowNoHead"
    )
}

pub(crate) fn xmap15_scene_effects(resolver: &GameFileResolver, definitions: &[PreparedXmap15PlanDefinition]) -> Result<BTreeSet<String>> {
    if definitions.is_empty() { return Ok(BTreeSet::new()); }
    let files = crate::shader_compile::effect_translation_unit_files(resolver, XMAP15_MAPARROW_SHADER_PATH)?;
    let root = files.last().ok_or("XMAP15 shader translation unit is empty")?;
    let mut effects = BTreeSet::new();
    for definition in definitions {
        effects.insert(definition.technique.clone());
        let sibling = format!("{}Seethrough", definition.technique);
        if root.1.effect(&sibling).is_some() { effects.insert(sibling); }
    }
    Ok(effects)
}

#[derive(Clone, Copy)]
struct Xmap15SamplerContract {
    name: &'static str,
    index: u32,
    mag: &'static str,
    min: &'static str,
    mip: &'static str,
    address: &'static str,
    sampler_type: Option<&'static str>,
    lod_bias: Option<f32>,
    min_lod: Option<f32>,
    max_lod: Option<f32>,
}

const XMAP15_PLAN_SAMPLERS: [Xmap15SamplerContract; 11] = [
    Xmap15SamplerContract {
        name: "TexMask",
        index: 0,
        mag: "Point",
        min: "Point",
        mip: "Point",
        address: "Wrap",
        sampler_type: None,
        lod_bias: None,
        min_lod: None,
        max_lod: None,
    },
    Xmap15SamplerContract {
        name: "TexPattern",
        index: 1,
        mag: "Linear",
        min: "Linear",
        mip: "Linear",
        address: "Wrap",
        sampler_type: None,
        lod_bias: Some(-0.5),
        min_lod: Some(0.0),
        max_lod: Some(4.0),
    },
    Xmap15SamplerContract {
        name: "ShadowMap",
        index: 2,
        mag: "Linear",
        min: "Linear",
        mip: "Linear",
        address: "Wrap",
        sampler_type: Some("Shadow"),
        lod_bias: None,
        min_lod: None,
        max_lod: None,
    },
    Xmap15SamplerContract {
        name: "TerrainIDMap",
        index: 3,
        mag: "Point",
        min: "Point",
        mip: "None",
        address: "Clamp",
        sampler_type: None,
        lod_bias: None,
        min_lod: None,
        max_lod: None,
    },
    Xmap15SamplerContract {
        name: "HeightMap",
        index: 4,
        mag: "Linear",
        min: "Linear",
        mip: "Linear",
        address: "Clamp",
        sampler_type: None,
        lod_bias: None,
        min_lod: None,
        max_lod: None,
    },
    Xmap15SamplerContract {
        name: "TerrainNormal",
        index: 5,
        mag: "Linear",
        min: "Linear",
        mip: "Point",
        address: "Wrap",
        sampler_type: None,
        lod_bias: None,
        min_lod: None,
        max_lod: None,
    },
    Xmap15SamplerContract {
        name: "HeightNormal",
        index: 6,
        mag: "Linear",
        min: "Linear",
        mip: "Linear",
        address: "Wrap",
        sampler_type: None,
        lod_bias: None,
        min_lod: None,
        max_lod: None,
    },
    Xmap15SamplerContract {
        name: "LeanTexture1",
        index: 7,
        mag: "Linear",
        min: "Linear",
        mip: "Linear",
        address: "Wrap",
        sampler_type: None,
        lod_bias: None,
        min_lod: None,
        max_lod: None,
    },
    Xmap15SamplerContract {
        name: "LeanTexture2",
        index: 8,
        mag: "Linear",
        min: "Linear",
        mip: "Linear",
        address: "Wrap",
        sampler_type: None,
        lod_bias: None,
        min_lod: None,
        max_lod: None,
    },
    Xmap15SamplerContract {
        name: "LightDataMap",
        index: 9,
        mag: "Point",
        min: "Point",
        mip: "Point",
        address: "Clamp",
        sampler_type: None,
        lod_bias: None,
        min_lod: None,
        max_lod: None,
    },
    Xmap15SamplerContract {
        name: "LightIndexMap",
        index: 10,
        mag: "Point",
        min: "Point",
        mip: "Point",
        address: "Clamp",
        sampler_type: None,
        lod_bias: None,
        min_lod: None,
        max_lod: None,
    },
];

const XMAP15_MOVEMENT_SAMPLERS: [Xmap15SamplerContract; 3] = [
    Xmap15SamplerContract {
        name: "DiffuseTexture",
        index: 0,
        mag: "Linear",
        min: "Linear",
        mip: "Linear",
        address: "Clamp",
        sampler_type: None,
        lod_bias: None,
        min_lod: None,
        max_lod: None,
    },
    Xmap15SamplerContract {
        name: "NormalMap",
        index: 1,
        mag: "Linear",
        min: "Linear",
        mip: "Linear",
        address: "Clamp",
        sampler_type: None,
        lod_bias: None,
        min_lod: None,
        max_lod: None,
    },
    Xmap15SamplerContract {
        name: "SpecularMap",
        index: 2,
        mag: "Linear",
        min: "Linear",
        mip: "Linear",
        address: "Clamp",
        sampler_type: None,
        lod_bias: None,
        min_lod: None,
        max_lod: None,
    },
];

fn token_eq(actual: Option<&str>, expected: Option<&str>) -> bool {
    match (actual, expected) {
        (Some(actual), Some(expected)) => actual.eq_ignore_ascii_case(expected),
        (None, None) => true,
        _ => false,
    }
}

fn numeric_token_eq(actual: Option<&str>, expected: Option<f32>) -> bool {
    match (actual, expected) {
        (Some(actual), Some(expected)) => actual
            .parse::<f32>()
            .is_ok_and(|actual| actual.to_bits() == expected.to_bits()),
        (None, None) => true,
        _ => false,
    }
}

fn validate_xmap15_sampler_contract(
    role: &str,
    actual: &[crate::EffectSampler],
    expected: &[Xmap15SamplerContract],
) -> Result<()> {
    if actual.len() != expected.len() {
        return Err(format!("XMAP15 effective {role} sampler census drifted").into());
    }
    for contract in expected {
        let matches = actual
            .iter()
            .filter(|sampler| {
                sampler.name == contract.name
                    && sampler.index == contract.index
                    && token_eq(sampler.mag_filter.as_deref(), Some(contract.mag))
                    && token_eq(sampler.min_filter.as_deref(), Some(contract.min))
                    && token_eq(sampler.mip_filter.as_deref(), Some(contract.mip))
                    && token_eq(sampler.address_u.as_deref(), Some(contract.address))
                    && token_eq(sampler.address_v.as_deref(), Some(contract.address))
                    && token_eq(sampler.sampler_type.as_deref(), contract.sampler_type)
                    && numeric_token_eq(sampler.mip_map_lod_bias.as_deref(), contract.lod_bias)
                    && numeric_token_eq(sampler.mip_map_min_lod.as_deref(), contract.min_lod)
                    && numeric_token_eq(sampler.mip_map_max_lod.as_deref(), contract.max_lod)
                    && sampler.max_anisotropy.is_none()
            })
            .count();
        if matches != 1 {
            return Err(format!(
                "XMAP15 effective {role} sampler {} does not match the native descriptor",
                contract.name
            )
            .into());
        }
    }
    Ok(())
}

fn validate_xmap15_state(
    states: &BTreeMap<String, Vec<(String, String)>>,
    name: &str,
    expected: &[(&str, &str)],
    role: &str,
) -> Result<()> {
    let state = states
        .get(name)
        .ok_or_else(|| format!("XMAP15 effective {role} selects missing state {name}"))?;
    let actual = state
        .iter()
        .map(|(key, value)| (key.to_ascii_lowercase(), value.to_ascii_lowercase()))
        .collect::<BTreeMap<_, _>>();
    let expected = expected
        .iter()
        .map(|(key, value)| (key.to_ascii_lowercase(), value.to_ascii_lowercase()))
        .collect::<BTreeMap<_, _>>();
    if actual.len() != state.len() || actual != expected {
        return Err(format!("XMAP15 effective {role} state {name} drifted").into());
    }
    Ok(())
}

fn xmap15_depth_contract(name: &str) -> Option<&'static [(&'static str, &'static str)]> {
    match name {
        "DepthStencilState" => Some(&[
            ("DepthEnable", "no"),
            ("StencilEnable", "yes"),
            ("FrontStencilFailOp", "stencil_op_keep"),
            ("FrontStencilDepthFailOp", "stencil_op_keep"),
            ("FrontStencilPassOp", "stencil_op_replace"),
            ("FrontStencilFunc", "comparison_not_equal"),
            ("StencilRef", "1"),
            ("StencilReadMask", "1"),
            ("StencilWriteMask", "1"),
        ]),
        "DepthStencilStateSeethrough" => Some(&[
            ("DepthEnable", "yes"),
            ("DepthWriteMask", "depth_write_zero"),
            ("DepthFunction", "comparison_greater"),
            ("StencilEnable", "yes"),
            ("FrontStencilFailOp", "stencil_op_keep"),
            ("FrontStencilDepthFailOp", "stencil_op_keep"),
            ("FrontStencilPassOp", "stencil_op_keep"),
            ("FrontStencilFunc", "comparison_equal"),
            ("StencilRef", "8"),
            ("StencilReadMask", "8"),
        ]),
        "DefaultDepthNoStencil" => Some(&[
            ("DepthEnable", "yes"),
            ("DepthWriteMask", "depth_write_all"),
            ("DepthFunction", "comparison_less_equal"),
            ("StencilEnable", "no"),
        ]),
        "DefaultDepthNoStencilSeethrough" => Some(&[
            ("DepthEnable", "yes"),
            ("DepthWriteMask", "depth_write_zero"),
            ("DepthFunction", "comparison_greater"),
            ("StencilEnable", "yes"),
            ("FrontStencilFailOp", "stencil_op_keep"),
            ("FrontStencilDepthFailOp", "stencil_op_keep"),
            ("FrontStencilPassOp", "stencil_op_keep"),
            ("FrontStencilFunc", "comparison_equal"),
            ("StencilRef", "8"),
            ("StencilReadMask", "8"),
        ]),
        "DefaultDepthStencilNoDepthWrite" => Some(&[
            ("DepthEnable", "yes"),
            ("DepthWriteMask", "depth_write_zero"),
            ("DepthFunction", "comparison_less_equal"),
            ("StencilEnable", "yes"),
            ("FrontStencilFailOp", "stencil_op_keep"),
            ("FrontStencilDepthFailOp", "stencil_op_keep"),
            ("FrontStencilPassOp", "stencil_op_replace"),
            ("FrontStencilFunc", "comparison_not_equal"),
            ("StencilRef", "1"),
            ("StencilReadMask", "1"),
            ("StencilWriteMask", "1"),
        ]),
        "NoDepthStencilState" => Some(&[("DepthEnable", "no"), ("StencilEnable", "no")]),
        _ => None,
    }
}

pub(crate) fn validate_xmap15_plan_effect_contracts(
    resolver: &GameFileResolver,
    effects: &BTreeSet<String>,
) -> Result<()> {
    if effects.is_empty() {
        return Ok(());
    }
    let files = crate::shader_compile::effect_translation_unit_files(
        resolver,
        XMAP15_MAPARROW_SHADER_PATH,
    )?;
    let root = files
        .last()
        .map(|(_, file)| file)
        .ok_or("XMAP15 maparrow translation unit is empty")?;
    validate_xmap15_plan_effect_file_contracts(root, effects)
}

fn validate_xmap15_plan_effect_file_contracts(
    root: &crate::EffectFile,
    effects: &BTreeSet<String>,
) -> Result<()> {
    validate_xmap15_sampler_contract(
        "maparrow",
        &root.pixel_stage.samplers,
        &XMAP15_PLAN_SAMPLERS,
    )?;
    for name in effects {
        let (pixel, depth, blend, defines): (&str, &str, Option<&str>, &[&str]) = match name
            .as_str()
        {
            "MapArrowDefault" => ("ArrowPixelShader", "DepthStencilState", None, &[]),
            "MapArrowDefaultWithDepth" => ("ArrowPixelShader", "DefaultDepthNoStencil", None, &[]),
            "MapArrowDefaultWithDepthTestOnly" => (
                "ArrowPixelShader",
                "DefaultDepthStencilNoDepthWrite",
                None,
                &[],
            ),
            "MapArrowNoHeadWidthDepth" => (
                "ArrowPixelShaderNoHead",
                "DefaultDepthNoStencil",
                None,
                &["ANIM_TEXTURE"],
            ),
            "MapArrowNoHead" => (
                "ArrowPixelShaderNoHead",
                "DepthStencilState",
                None,
                &["ANIM_TEXTURE"],
            ),
            "MapArrowNoHeadWidthDepthSeethrough" => (
                "ArrowPixelShaderNoHead",
                "DefaultDepthNoStencilSeethrough",
                Some("BlendState"),
                &["ANIM_TEXTURE", "SEETHROUGH"],
            ),
            "MapArrowNoHeadSeethrough" => (
                "ArrowPixelShaderNoHead",
                "DepthStencilStateSeethrough",
                Some("BlendState"),
                &["ANIM_TEXTURE", "SEETHROUGH"],
            ),
            other => {
                return Err(
                    format!("XMAP15 native state has no effect contract for {other}").into(),
                )
            }
        };
        let effect = root
            .effect(name)
            .ok_or_else(|| format!("effective maparrow.shader omits {name}"))?;
        if root.effect_count(name) != 1
            || effect.vertex_shader.as_deref() != Some("ArrowVertexShader")
            || effect.pixel_shader.as_deref() != Some(pixel)
            || effect.depth_stencil_state.as_deref() != Some(depth)
            || effect.blend_state.as_deref() != blend
            || effect.rasterizer_state.is_some()
            || effect
                .defines
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                != defines
        {
            return Err(format!(
                "XMAP15 effective maparrow effect {name} drifted from native state"
            )
            .into());
        }
        validate_xmap15_state(
            &root.depth_stencil_states,
            depth,
            xmap15_depth_contract(depth).ok_or("XMAP15 depth contract is absent")?,
            "maparrow depth-stencil",
        )?;
        if let Some(blend) = blend {
            validate_xmap15_state(
                &root.blend_states,
                blend,
                &[
                    ("BlendEnable", "yes"),
                    ("AlphaTest", "no"),
                    ("SourceBlend", "src_alpha"),
                    ("DestBlend", "inv_src_alpha"),
                    ("WriteMask", "RED|GREEN|BLUE"),
                ],
                "maparrow blend",
            )?;
        }
    }
    Ok(())
}

pub(crate) fn validate_xmap15_movement_effect_contract(resolver: &GameFileResolver) -> Result<()> {
    let files = crate::shader_compile::effect_translation_unit_files(
        resolver,
        XMAP15_MOVEMENT_SHADER_PATH,
    )?;
    let root = files
        .last()
        .map(|(_, file)| file)
        .ok_or("XMAP15 movement translation unit is empty")?;
    validate_xmap15_movement_effect_file_contract(root)
}

fn validate_xmap15_movement_effect_file_contract(root: &crate::EffectFile) -> Result<()> {
    validate_xmap15_sampler_contract(
        "movement",
        &root.pixel_stage.samplers,
        &XMAP15_MOVEMENT_SAMPLERS,
    )?;
    let effect = root
        .effect("ArrowEffect")
        .ok_or("effective arrow.shader omits ArrowEffect")?;
    if root.effect_count("ArrowEffect") != 1
        || effect.vertex_shader.as_deref() != Some("VertexShader")
        || effect.pixel_shader.as_deref() != Some("PixelShader")
        || effect.depth_stencil_state.as_deref() != Some("NoDepthStencilState")
        || effect.blend_state.is_some()
        || effect.rasterizer_state.is_some()
        || !effect.defines.is_empty()
    {
        return Err("XMAP15 effective movement effect drifted from native state".into());
    }
    validate_xmap15_state(
        &root.depth_stencil_states,
        "NoDepthStencilState",
        xmap15_depth_contract("NoDepthStencilState").unwrap(),
        "movement depth-stencil",
    )
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedXmap15MovementDefinition {
    pub name: String,
    pub texture: String,
    pub normal: String,
    pub specular: String,
    pub effect: String,
    pub ordered_shader_translation_unit_paths: Vec<String>,
    pub ordered_used_content: Vec<UsedContentKey>,
    pub deterministic_sha256: String,
}

impl PreparedXmap15MovementDefinition {
    fn validate(&self) -> Result<()> {
        if self.name.is_empty()
            || self.texture.is_empty()
            || self.normal.is_empty()
            || self.specular.is_empty()
            || self.effect.is_empty()
            || self.ordered_shader_translation_unit_paths.is_empty()
            || self
                .ordered_shader_translation_unit_paths
                .last()
                .is_none_or(|path| !path.eq_ignore_ascii_case(XMAP15_MOVEMENT_SHADER_PATH))
            || self.deterministic_sha256 != definition_sha256(self)?
        {
            return Err(format!("XMAP15 movement definition {} is invalid", self.name).into());
        }
        let expected = [
            XMAP15_MOVEMENT_DEFINITIONS_PATH,
            self.texture.as_str(),
            self.normal.as_str(),
            self.specular.as_str(),
        ]
        .into_iter()
        .chain(
            self.ordered_shader_translation_unit_paths
                .iter()
                .map(String::as_str),
        )
        .map(str::to_ascii_lowercase)
        .collect::<BTreeSet<_>>();
        let actual = self
            .ordered_used_content
            .iter()
            .map(|content| content.normalized_game_relative_path.to_ascii_lowercase())
            .collect::<BTreeSet<_>>();
        if expected != actual || actual.len() != self.ordered_used_content.len() {
            return Err(format!(
                "XMAP15 movement definition {} content closure is invalid",
                self.name
            )
            .into());
        }
        Ok(())
    }
}

pub fn prepare_xmap15_frame(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
    world: &WorldFrame,
    units: &dyn Xmap15UnitIdentityProvider,
    selection: &Xmap15ViewerSelection,
    assigned_stage: AssignedStage,
    foundation_target_sha256: String,
) -> Result<PreparedXmap15Frame> {
    let semantic = semantic_frame(world)?;
    cross_check_d54_and_units(world, &semantic, units)?;

    let mut plan_names = semantic
        .plans
        .iter()
        .map(|row| row.resolved_definition_name.clone())
        .collect::<BTreeSet<_>>();
    let mut movement_names = semantic
        .movements
        .iter()
        .map(|row| row.definition.definition_name().to_string())
        .collect::<BTreeSet<_>>();
    let (plan_definitions, mut used_content) =
        prepare_plan_definitions(resolver, admission, &mut plan_names)?;
    let paradrop_profile = if semantic.plans.iter().any(|row|
        row.selector.order_type == 4 && row.route.explicit_height_f32_bits.is_none()) {
        let (sources, ordered_used_content) = crate::xmap09_lighting_law::admitted_effective_define_sources(
            resolver, "XMAP15 paradrop height defines",
        )?;
        let evaluated = crate::xmap09_lighting_law::EffectiveDefines::from_sources(&sources)?;
        let parameters = paradrop_profile_from_effective(&evaluated)?;
        used_content.extend(ordered_used_content.iter().cloned());
        Some(PreparedXmap15ParadropProfile { parameters, ordered_used_content })
    } else { None };
    let heightmap = if semantic.plans.is_empty() {
        None
    } else {
        let key = admitted(resolver, admission, XMAP15_HEIGHTMAP_PATH)?;
        used_content.push(key.clone());
        Some(key)
    };
    let (movement_definitions, movement_content) =
        prepare_movement_definitions(resolver, admission, &mut movement_names)?;
    used_content.extend(movement_content);
    let plan_effects = xmap15_scene_effects(resolver, &plan_definitions)?;
    validate_xmap15_plan_effect_contracts(resolver, &plan_effects)?;
    if !movement_definitions.is_empty() {
        validate_xmap15_movement_effect_contract(resolver)?;
    }
    let (map_dimensions_content, map_width, map_height) =
        if semantic.plans.is_empty() && semantic.movements.is_empty() {
            (None, 0, 0)
        } else {
            let key = admitted(resolver, admission, XMAP15_MAP_DIMENSIONS_PATH)?;
            let bytes = resolver.read_admitted_used_content(&key)?;
            let (width, height) = crate::production_foundation::bmp_dimensions(&bytes)?;
            used_content.push(key.clone());
            (Some(key), width, height)
        };
    used_content.sort();
    used_content.dedup();

    let plan_by_name = plan_definitions
        .iter()
        .map(|definition| {
            (
                definition.name.as_str(),
                definition.deterministic_sha256.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let movement_by_name = movement_definitions
        .iter()
        .map(|definition| {
            (
                definition.name.as_str(),
                definition.deterministic_sha256.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut ordered_rows = semantic
        .plans
        .iter()
        .cloned()
        .map(|semantic| {
            let definition_sha256 = plan_by_name
                .get(semantic.resolved_definition_name.as_str())
                .cloned()
                .ok_or("XMAP15 plan definition vanished after preparation")?;
            Ok(Xmap15PreparedRow::Plan {
                semantic,
                definition_sha256,
            })
        })
        .chain(semantic.movements.iter().cloned().map(|semantic| {
            let definition_sha256 = movement_by_name
                .get(semantic.definition.definition_name())
                .cloned()
                .ok_or("XMAP15 movement definition vanished after preparation")?;
            Ok(Xmap15PreparedRow::Movement {
                semantic,
                definition_sha256,
            })
        }))
        .collect::<Result<Vec<_>>>()?;
    ordered_rows.sort_by_key(|row| match row {
        Xmap15PreparedRow::Plan { semantic, .. } => (
            semantic.final_engine_draw_ordinal,
            semantic.definition_group_ordinal,
        ),
        Xmap15PreparedRow::Movement { semantic, .. } => (
            semantic.final_engine_draw_ordinal,
            semantic.definition_group_ordinal,
        ),
    });

    let semantic_bytes = semantic
        .encode_json()
        .map_err(|error| format!("XMAP15 semantic codec: {error}"))?;
    let portable = Xmap15PreparedFrame {
        schema: XMAP15_STATE_FIRST_SCHEMA.into(),
        hour_index: world.hour_index(),
        observation_sequence: semantic.observation_sequence,
        semantic_input_sha256: Sha256::hex(&semantic_bytes),
        enumerator_witness_sha256: semantic.enumerator_witness_sha256.clone(),
        xmap13_census_sha256: units.census_sha256().into(),
        xmap13_admission_sha256: units.admission_sha256().into(),
        selected_plan: selection.selected_plan.clone(),
        ordered_rows,
        ordered_used_content: used_content,
        deterministic_sha256: String::new(),
    }
    .seal()
    .map_err(|error| format!("seal XMAP15 frame: {error}"))?;
    let prepared = PreparedXmap15Frame {
        portable,
        assigned_stage,
        foundation_target_sha256,
        plan_definitions,
        movement_definitions,
        heightmap,
        paradrop_profile,
        map_dimensions_content,
        map_width,
        map_height,
    };
    prepared.validate()?;
    Ok(prepared)
}

pub(crate) fn xmap15_registration_sha256() -> String {
    Sha256::hex(
        format!("XMAP15\0{XMAP15_STATE_FIRST_SCHEMA}\0{XMAP15_REGISTRATION_EVIDENCE}").as_bytes(),
    )
}

fn xmap15_unavailable_gap(
    hour: i64,
    authority_gap: &crate::MapAuthorityGap,
    detail: &str,
) -> FoundationGap {
    let gap_id = "XMAP15.orders_fronts_arrows.unavailable".to_string();
    let reason = format!("{}; {detail}", authority_gap.reason);
    let witness_sha256 = Sha256::hex(format!("{gap_id}\n{hour}\n{hour}\n{reason}").as_bytes());
    FoundationGap {
        gap_id,
        first_unavailable_hour: hour,
        last_unavailable_hour: hour,
        witness_sha256,
        reason,
    }
}

fn semantic_value(world: &WorldFrame) -> Option<&serde_json::Value> {
    world.xmap15_orders_fronts_movement_semantic()
}

/// The gate text for a recording that carries one, and for one that does not.
///
/// Split out so it can be exercised without a `WorldFrame`: composing this
/// sentence is the only thing the absent arm does that a test can be wrong
/// about.
fn xmap15_account_from_gate(gate: Option<&serde_json::Value>, hour_index: i64) -> String {
    let usable = gate
        .and_then(|raw| {
            serde_json::from_value::<portable_render_stream::Xmap15ProducerGate>(raw.clone()).ok()
        })
        .filter(|gate| gate.validate().is_ok() && gate.hour_index == hour_index);
    match usable {
        Some(gate) => gate.unavailable_reason(),
        None => format!(
            "this recording carries no usable XMAP15 producer gate, so it says nothing about what \
             could have been minted this hour; {} mandatory leaves have no source in any capture",
            portable_render_stream::xmap15_contract_gate_leaves().len()
        ),
    }
}

/// The hour's own account of why XMAP15 is unavailable, from the recorder's
/// gate where the recording carries a usable one.
///
/// A gate for a DIFFERENT hour is refused rather than quoted: the fold writes
/// one per hour, and a mismatched index means the value was carried across an
/// hour boundary, so its counts describe an hour that is not this one.
/// What the recorded order census yielded this hour, for a gap that has to say
/// why the family cannot draw without pretending the orders were not there.
fn xmap15_order_derivation_account(
    world: &WorldFrame,
    derivation: &Xmap15PlanProvider,
) -> String {
    if let Xmap15PlanProvider::Present(derivation) = derivation {
        return derivation.refusal_account();
    }
    let view = world.army_battleplans();
    match view.diagnostic {
        Some(diagnostic) => format!(
            "the recording carries no admitted battle-plan order census this hour: {diagnostic}"
        ),
        None => format!(
            "the recording carries no admitted battle-plan order census this hour: the D54 view              is {:?}",
            view.status
        ),
    }
}

fn xmap15_unavailable_account(world: &WorldFrame) -> String {
    xmap15_account_from_gate(world.xmap15_producer_gate(), world.hour_index())
}

fn semantic_frame(world: &WorldFrame) -> Result<Xmap15SemanticFrame> {
    let value =
        semantic_value(world).ok_or("XMAP15 unconditional semantic sidecar is unavailable")?;
    let bytes = serde_json::to_vec(value)?;
    let semantic = Xmap15SemanticFrame::decode_json(&bytes)
        .map_err(|error| format!("XMAP15 semantic sidecar: {error}"))?;
    if semantic.hour_index != world.hour_index() {
        return Err("XMAP15 semantic sidecar belongs to a different replay hour".into());
    }
    Ok(semantic)
}

fn cross_check_d54_and_units(
    world: &WorldFrame,
    semantic: &Xmap15SemanticFrame,
    units: &dyn Xmap15UnitIdentityProvider,
) -> Result<()> {
    if units.hour_index() != world.hour_index() {
        return Err("XMAP15 XMAP13 identity provider belongs to a different hour".into());
    }
    validate_sha256(units.census_sha256(), "XMAP13 census")?;
    validate_sha256(units.admission_sha256(), "XMAP13 admission")?;
    let d54 = world.army_battleplans();
    if d54.status != ViewStatus::Present
        || !d54.baseline_seen
        || d54.omitted_non_exact_rows_this_hour != Some(0)
        || d54.path_geometry_omitted_rows_this_hour != Some(0)
        || d54.identity_or_active_omitted_rows_this_hour != Some(0)
    {
        return Err("XMAP15 requires a complete zero-omission D54 battleplan view".into());
    }
    let visual_d54 = d54
        .orders
        .iter()
        .filter(|order| {
            matches!(
                order.key.owner.kind,
                ArmyBattleplanOwnerKind::Army | ArmyBattleplanOwnerKind::ArmyGroup
            )
        })
        .filter(|order| (1..=4).contains(&order.order_type))
        .collect::<Vec<_>>();
    if visual_d54.len() != semantic.plans.len() {
        return Err("XMAP15 semantic plan census disagrees with visual D54 orders".into());
    }
    for row in &semantic.plans {
        let owner_kind = match row.key.owner_kind {
            portable_render_stream::Xmap15OwnerKind::Army => ArmyBattleplanOwnerKind::Army,
            portable_render_stream::Xmap15OwnerKind::ArmyGroup => {
                ArmyBattleplanOwnerKind::ArmyGroup
            }
        };
        let d54 = visual_d54
            .iter()
            .copied()
            .find(|order| {
                order.key.owner.kind == owner_kind
                    && order.key.owner.id == row.key.owner_id
                    && order.key.engine_order_instance_id == row.key.engine_order_instance_id
                    && order.key.generation == row.key.generation
            })
            .ok_or("XMAP15 semantic plan identity is absent from D54")?;
        let d54_provinces = match &d54.geometry {
            ArmyBattleplanGeometry::PathProvinces(values) => values.clone(),
            ArmyBattleplanGeometry::Paradrop {
                source_province_id,
                target_province_id,
            } => vec![*target_province_id, *source_province_id],
            _ => return Err("XMAP15 applicable D54 plan geometry is unavailable".into()),
        };
        if d54.slot_index != row.key.slot_index
            || d54.order_type != i32::from(row.selector.order_type)
            || d54.active != row.active
            || d54.assigned_division_ids != row.ordered_assigned_division_ids
            || d54_provinces != row.route.ordered_province_ids
        {
            return Err("XMAP15 semantic plan disagrees with exact D54 state".into());
        }
        if units.formation_identity_match_count(
            matches!(
                row.key.owner_kind,
                portable_render_stream::Xmap15OwnerKind::ArmyGroup
            ),
            row.key.owner_id,
            &row.owner_country,
        ) != 1
        {
            return Err("XMAP15 plan owner is absent from exact XMAP13 formation census".into());
        }
        for division_id in &row.ordered_assigned_division_ids {
            if units.division_identity_match_count(*division_id, &row.owner_country) != 1 {
                return Err("XMAP15 plan member is absent from exact XMAP13 census".into());
            }
        }
    }
    for row in &semantic.movements {
        if units.division_identity_match_count(row.key.division_id, &row.key.owner_country) != 1 {
            return Err("XMAP15 movement identity is absent from exact XMAP13 census".into());
        }
    }
    Ok(())
}

fn prepare_plan_definitions(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
    names: &mut BTreeSet<String>,
) -> Result<(Vec<PreparedXmap15PlanDefinition>, Vec<UsedContentKey>)> {
    if names.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let source_key = admitted(resolver, admission, XMAP15_MAPARROWS_PATH)?;
    let shader_content =
        admitted_effect_translation_unit(resolver, admission, XMAP15_MAPARROW_SHADER_PATH)?;
    let shader_paths = shader_content
        .iter()
        .map(|key| key.normalized_game_relative_path.clone())
        .collect::<Vec<_>>();
    let source_bytes = resolver.read_admitted_used_content(&source_key)?;
    let source =
        std::str::from_utf8(&source_bytes).map_err(|_| "effective maparrows.txt is not UTF-8")?;
    let blocks = parse_arrow_blocks(source)?;
    let definition_indices = arrow_definition_indices(source)?;
    let mut definitions = Vec::new();
    let mut all_content = vec![source_key.clone()];
    all_content.extend(shader_content.iter().cloned());
    for name in std::mem::take(names) {
        let fields = blocks
            .get(&name)
            .ok_or_else(|| format!("effective maparrows.txt omits definition {name}"))?;
        let masks = required_list(fields, "texture_masks", &name)?;
        let patterns = required_list(fields, "texture_patterns", &name)?;
        let mut content = vec![source_key.clone()];
        content.extend(shader_content.iter().cloned());
        for path in masks.iter().chain(&patterns) {
            let key = admitted(resolver, admission, path)?;
            content.push(key.clone());
            all_content.push(key);
        }
        content.sort();
        content.dedup();
        let mut definition = PreparedXmap15PlanDefinition {
            name: name.clone(),
            definition_index: definition_indices[&name],
            technique: required_atom(fields, "technique", &name)?,
            body_width_f32_bits: required_f32_pair(fields, "body_width", &name)?.map(f32::to_bits),
            body_uv_scale_f32_bits: required_f32(fields, "body_uv_scale", &name)?.to_bits(),
            body_uv_scale_2_f32_bits: optional_f32(fields, "body_uv_scale_2", &name)?
                .map(f32::to_bits),
            mask_inner_f32_bits: required_f32_array::<3>(fields, "mask_inner", &name)?
                .map(f32::to_bits),
            mask_outer_f32_bits: required_f32_array::<3>(fields, "mask_outer", &name)?
                .map(f32::to_bits),
            width_scale_f32_bits: optional_f32(fields, "width_scale", &name)?
                .unwrap_or(0.0)
                .to_bits(),
            width_scale_max_f32_bits: optional_f32(fields, "width_scale_max", &name)?
                .map(f32::to_bits),
            head_length_f32_bits: optional_f32(fields, "head_length", &name)?.map(f32::to_bits),
            texture_loop_f32_bits: optional_f32(fields, "texture_loop", &name)?.map(f32::to_bits),
            fade_in_f32_bits: optional_f32(fields, "fade_in", &name)?
                .unwrap_or(0.0)
                .to_bits(),
            fade_out_f32_bits: optional_f32(fields, "fade_out", &name)?
                .unwrap_or(0.0)
                .to_bits(),
            global_scale_min_size_f32_bits: required_f32(fields, "global_scale_min_size", &name)?
                .to_bits(),
            global_scale_max_points: required_u32(fields, "global_scale_max_points", &name)?,
            texture_anim_speed_f32_bits: optional_f32(fields, "texture_anim_speed", &name)?
                .map(f32::to_bits),
            texture_variant_count: optional_u32(fields, "texture_variant_count", &name)?,
            culling_direction: optional_i32(fields, "culling_direction", &name)?,
            draw_last: optional_bool(fields, "draw_last", &name)?.unwrap_or(false),
            draw_shadows: optional_bool(fields, "draw_shadows", &name)?.unwrap_or(true),
            ordered_texture_masks: masks,
            ordered_texture_patterns: patterns,
            ordered_shader_translation_unit_paths: shader_paths.clone(),
            ordered_used_content: content,
            deterministic_sha256: String::new(),
        };
        definition.deterministic_sha256 = definition_sha256(&definition)?;
        definition.validate()?;
        definitions.push(definition);
    }
    definitions.sort_by(|a, b| a.name.cmp(&b.name));
    Ok((definitions, all_content))
}

fn prepare_movement_definitions(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
    names: &mut BTreeSet<String>,
) -> Result<(Vec<PreparedXmap15MovementDefinition>, Vec<UsedContentKey>)> {
    if names.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let source_key = admitted(resolver, admission, XMAP15_MOVEMENT_DEFINITIONS_PATH)?;
    let shader_content =
        admitted_effect_translation_unit(resolver, admission, XMAP15_MOVEMENT_SHADER_PATH)?;
    let shader_paths = shader_content
        .iter()
        .map(|key| key.normalized_game_relative_path.clone())
        .collect::<Vec<_>>();
    let source_bytes = resolver.read_admitted_used_content(&source_key)?;
    let source =
        std::str::from_utf8(&source_bytes).map_err(|_| "effective arrows.gfx is not UTF-8")?;
    let blocks = parse_arrow_blocks(source)?;
    let mut definitions = Vec::new();
    let mut all_content = vec![source_key.clone()];
    all_content.extend(shader_content.iter().cloned());
    for name in std::mem::take(names) {
        let fields = blocks
            .get(&name)
            .ok_or_else(|| format!("effective arrows.gfx omits definition {name}"))?;
        let texture = required_atom(fields, "texture", &name)?;
        let normal = required_atom(fields, "normal", &name)?;
        let specular = required_atom(fields, "specular", &name)?;
        let effect = required_atom(fields, "effect", &name)?;
        if !effect.eq_ignore_ascii_case(XMAP15_MOVEMENT_EFFECT_PATH) {
            return Err(
                format!("movement definition {name} selects unsupported effect {effect}").into(),
            );
        }
        let mut content = vec![source_key.clone()];
        content.extend(shader_content.iter().cloned());
        for path in [&texture, &normal, &specular] {
            let key = admitted(resolver, admission, path)?;
            content.push(key.clone());
            all_content.push(key);
        }
        content.sort();
        content.dedup();
        let mut definition = PreparedXmap15MovementDefinition {
            name,
            texture,
            normal,
            specular,
            effect,
            ordered_shader_translation_unit_paths: shader_paths.clone(),
            ordered_used_content: content,
            deterministic_sha256: String::new(),
        };
        definition.deterministic_sha256 = definition_sha256(&definition)?;
        definition.validate()?;
        definitions.push(definition);
    }
    definitions.sort_by(|a, b| a.name.cmp(&b.name));
    Ok((definitions, all_content))
}

fn admitted(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
    path: &str,
) -> Result<UsedContentKey> {
    let key = admission
        .require_used_content(path)
        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?
        .clone();
    resolver.read_admitted_used_content(&key)?;
    Ok(key)
}

fn admitted_effect_translation_unit(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
    root: &str,
) -> Result<Vec<UsedContentKey>> {
    let files = crate::shader_compile::effect_translation_unit_files(resolver, root)?;
    let mut keys = Vec::with_capacity(files.len() + 1);
    keys.push(admitted(
        resolver,
        admission,
        crate::shader_compile::DX11_PLATFORM_HEADER,
    )?);
    for (path, _) in files {
        keys.push(admitted(resolver, admission, &path)?);
    }
    if keys
        .last()
        .is_none_or(|key| !key.normalized_game_relative_path.eq_ignore_ascii_case(root))
    {
        return Err(format!("XMAP15 shader closure for {root} omits its root").into());
    }
    Ok(keys)
}

#[derive(Clone, Debug)]
enum FieldValue {
    Atom(String),
    List(Vec<String>),
    Blocks(Vec<BTreeMap<String, FieldValue>>),
}

fn parse_arrow_blocks(source: &str) -> Result<BTreeMap<String, BTreeMap<String, FieldValue>>> {
    Ok(parse_arrow_entries(source)?.into_iter().collect())
}

pub(crate) fn arrow_definition_indices(source: &str) -> Result<BTreeMap<String, u32>> {
    parse_arrow_entries(source)?.into_iter().enumerate().map(|(i, (name, _))| {
        Ok((name, u32::try_from(i)?))
    }).collect()
}

fn parse_arrow_entries(source: &str) -> Result<Vec<(String, BTreeMap<String, FieldValue>)>> {
    let tokens = tokenize(source)?;
    let mut cursor = 0usize;
    let mut blocks = Vec::new();
    let mut names = BTreeSet::new();
    while cursor + 2 < tokens.len() {
        if tokens[cursor] != "arrowType" || tokens[cursor + 1] != "=" || tokens[cursor + 2] != "{" {
            cursor += 1;
            continue;
        }
        cursor += 3;
        let fields = parse_block(&tokens, &mut cursor, true)?;
        let name = match fields.get("name") {
            Some(FieldValue::Atom(value)) if !value.is_empty() => value.clone(),
            _ => return Err("arrowType omits a scalar name".into()),
        };
        if !names.insert(name.clone()) {
            return Err(format!("effective arrow definitions duplicate {name}").into());
        }
        blocks.push((name, fields));
    }
    Ok(blocks)
}

fn tokenize(source: &str) -> Result<Vec<String>> {
    let mut tokens = Vec::new();
    let mut chars = source.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '#' => {
                for next in chars.by_ref() {
                    if next == '\n' {
                        break;
                    }
                }
            }
            '"' => {
                let mut value = String::new();
                let mut closed = false;
                for next in chars.by_ref() {
                    if next == '"' {
                        closed = true;
                        break;
                    }
                    value.push(next);
                }
                if !closed {
                    return Err("arrow definition contains an unterminated string".into());
                }
                tokens.push(value);
            }
            '{' | '}' | '=' => tokens.push(ch.to_string()),
            value if value.is_whitespace() => {}
            _ => {
                let mut value = String::from(ch);
                while let Some(next) = chars.peek().copied() {
                    if next.is_whitespace() || matches!(next, '#' | '"' | '{' | '}' | '=') {
                        break;
                    }
                    value.push(next);
                    chars.next();
                }
                tokens.push(value);
            }
        }
    }
    Ok(tokens)
}

fn scalar_assignment_overwrites(key: &str) -> bool {
    matches!(key,
        "name" | "technique" | "fade_out" | "fade_in" | "draw_shadows"
        | "global_scale_min_size" | "global_scale_max_points" | "draw_last"
        | "body_uv_scale_2" | "texture_variant_count" | "head_length" | "texture_loop"
        | "body_uv_scale" | "text_size" | "text_offset" | "width_scale" | "width_scale_max"
    )
}

fn parse_block(
    tokens: &[String], cursor: &mut usize, arrow_definition: bool,
) -> Result<BTreeMap<String, FieldValue>> {
    let mut fields = BTreeMap::new();
    while *cursor < tokens.len() {
        if tokens[*cursor] == "}" {
            *cursor += 1;
            return Ok(fields);
        }
        if *cursor + 1 >= tokens.len() || tokens[*cursor + 1] != "=" {
            *cursor += 1;
            continue;
        }
        let key = tokens[*cursor].clone();
        *cursor += 2;
        if tokens.get(*cursor).is_some_and(|token| token == "{") {
            *cursor += 1;
            if key == "button" {
                let block = parse_block(tokens, cursor, false)?;
                match fields.entry(key).or_insert_with(|| FieldValue::Blocks(Vec::new())) {
                    FieldValue::Blocks(blocks) => blocks.push(block),
                    _ => return Err("arrow button must be a block".into()),
                }
                continue;
            }
            let mut depth = 1usize;
            let mut values = Vec::new();
            while *cursor < tokens.len() && depth != 0 {
                match tokens[*cursor].as_str() {
                    "{" => depth += 1,
                    "}" => depth -= 1,
                    "=" if depth == 1 => {}
                    _ if depth == 1 => values.push(tokens[*cursor].clone()),
                    _ => {}
                }
                *cursor += 1;
            }
            if depth != 0 {
                return Err("arrow definition has an unterminated block".into());
            }
            if fields
                .insert(key.clone(), FieldValue::List(values))
                .is_some()
            {
                return Err(format!("arrow definition repeats field {key}").into());
            }
        } else {
            if key == "button" {
                return Err("arrow button must be a block".into());
            }
            let value = tokens
                .get(*cursor)
                .ok_or("arrow definition field is truncated")?
                .clone();
            *cursor += 1;
            if let Some(previous) = fields.insert(key.clone(), FieldValue::Atom(value)) {
                if !arrow_definition || !scalar_assignment_overwrites(&key)
                    || !matches!(previous, FieldValue::Atom(_))
                {
                    return Err(format!("arrow definition repeats field {key}").into());
                }
            }
        }
    }
    Err("arrowType block is unterminated".into())
}

fn required_atom(fields: &BTreeMap<String, FieldValue>, key: &str, name: &str) -> Result<String> {
    match fields.get(key) {
        Some(FieldValue::Atom(value)) if !value.is_empty() => Ok(value.clone()),
        _ => Err(format!("arrow definition {name} omits scalar {key}").into()),
    }
}

fn optional_atom<'a>(fields: &'a BTreeMap<String, FieldValue>, key: &str) -> Option<&'a str> {
    match fields.get(key) {
        Some(FieldValue::Atom(value)) => Some(value),
        _ => None,
    }
}

fn optional_bool(
    fields: &BTreeMap<String, FieldValue>,
    key: &str,
    name: &str,
) -> Result<Option<bool>> {
    let Some(value) = optional_atom(fields, key) else {
        return Ok(None);
    };
    if value.eq_ignore_ascii_case("yes") {
        Ok(Some(true))
    } else if value.eq_ignore_ascii_case("no") {
        Ok(Some(false))
    } else {
        Err(format!("arrow definition {name} has invalid boolean {key}={value}").into())
    }
}

fn required_list(
    fields: &BTreeMap<String, FieldValue>,
    key: &str,
    name: &str,
) -> Result<Vec<String>> {
    match fields.get(key) {
        Some(FieldValue::List(values)) if !values.is_empty() => Ok(values.clone()),
        _ => Err(format!("arrow definition {name} omits list {key}").into()),
    }
}

fn required_f32(fields: &BTreeMap<String, FieldValue>, key: &str, name: &str) -> Result<f32> {
    optional_f32(fields, key, name)?
        .ok_or_else(|| format!("arrow definition {name} omits scalar {key}").into())
}

fn optional_f32(
    fields: &BTreeMap<String, FieldValue>,
    key: &str,
    name: &str,
) -> Result<Option<f32>> {
    let Some(value) = optional_atom(fields, key) else {
        return Ok(None);
    };
    let parsed = value
        .parse::<f32>()
        .map_err(|_| format!("arrow definition {name} has invalid {key}"))?;
    if !parsed.is_finite() {
        return Err(format!("arrow definition {name} has non-finite {key}").into());
    }
    Ok(Some(parsed))
}

fn required_f32_array<const N: usize>(
    fields: &BTreeMap<String, FieldValue>,
    key: &str,
    name: &str,
) -> Result<[f32; N]> {
    let values = required_list(fields, key, name)?;
    if values.len() != N {
        return Err(format!("arrow definition {name} has invalid {key} arity").into());
    }
    let parsed = values
        .into_iter()
        .map(|value| {
            value
                .parse::<f32>()
                .map_err(|_| format!("arrow definition {name} has invalid {key}"))
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if parsed.iter().any(|value| !value.is_finite()) {
        return Err(format!("arrow definition {name} has non-finite {key}").into());
    }
    parsed
        .try_into()
        .map_err(|_| format!("arrow definition {name} has invalid {key} arity").into())
}

fn required_f32_pair(
    fields: &BTreeMap<String, FieldValue>,
    key: &str,
    name: &str,
) -> Result<[f32; 2]> {
    required_f32_array::<2>(fields, key, name)
}

fn required_u32(fields: &BTreeMap<String, FieldValue>, key: &str, name: &str) -> Result<u32> {
    required_atom(fields, key, name)?
        .parse()
        .map_err(|_| format!("arrow definition {name} has invalid {key}").into())
}

fn optional_u32(
    fields: &BTreeMap<String, FieldValue>,
    key: &str,
    name: &str,
) -> Result<Option<u32>> {
    optional_atom(fields, key)
        .map(|value| {
            value
                .parse()
                .map_err(|_| format!("arrow definition {name} has invalid {key}").into())
        })
        .transpose()
}

fn optional_i32(
    fields: &BTreeMap<String, FieldValue>,
    key: &str,
    name: &str,
) -> Result<Option<i32>> {
    optional_atom(fields, key)
        .map(|value| {
            value
                .parse()
                .map_err(|_| format!("arrow definition {name} has invalid {key}").into())
        })
        .transpose()
}

fn definition_sha256<T: Serialize>(definition: &T) -> Result<String> {
    let mut value = serde_json::to_value(definition)?;
    value
        .as_object_mut()
        .ok_or("XMAP15 definition digest input is not an object")?
        .insert(
            "deterministic_sha256".into(),
            serde_json::Value::String(String::new()),
        );
    Ok(Sha256::hex(&serde_json::to_vec(&value)?))
}

fn validate_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{label} is not canonical lowercase SHA-256").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definition_parser_preserves_scalar_and_list_fields() {
        let source = r#"
            arrowType = {
                name = "offence_line"
                technique = "MapArrowNoHead"
                texture_masks = { "a.dds" "b.dds" }
                texture_patterns = { "p.dds" }
                body_width = { 1.1 1.0 }
                body_uv_scale = 1
            }
        "#;
        let blocks = parse_arrow_blocks(source).unwrap();
        let fields = blocks.get("offence_line").unwrap();
        assert_eq!(
            required_atom(fields, "technique", "offence_line").unwrap(),
            "MapArrowNoHead"
        );
        assert_eq!(
            required_list(fields, "texture_masks", "offence_line").unwrap(),
            vec!["a.dds", "b.dds"]
        );
        assert_eq!(
            required_f32_pair(fields, "body_width", "offence_line").unwrap(),
            [1.1, 1.0]
        );
    }

    #[test]
    fn definition_parser_admits_shipped_ordered_buttons() {
        let shipped = include_bytes!("../../tests/fixtures/maparrows_a729.txt");
        assert_eq!(shipped.len(), 41_644);
        assert_eq!(Sha256::hex(shipped),
            "ccdee05cd75e1e471ff5382e24ae590d7c1cc5738ca4db01e29314ea7a6289ff");
        let root = crate::test_support::temp_test_dir("xmap15_shipped_buttons");
        let path = root.join(XMAP15_MAPARROWS_PATH);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, shipped).unwrap();
        let resolver = crate::test_support::fixture_game_resolver(&root);
        let (resolver, admission) = crate::test_support::admitted_vanilla_resolver(
            &resolver, &BTreeSet::from([XMAP15_MAPARROWS_PATH.to_string()]), "shipped_arrow_buttons",
        );
        let key = admitted(&resolver, &admission, XMAP15_MAPARROWS_PATH).unwrap();
        let bytes = resolver.read_admitted_used_content(&key).unwrap();
        assert_eq!(bytes, shipped);
        let source = std::str::from_utf8(&bytes).unwrap();
        let blocks = parse_arrow_blocks(source).unwrap();
        assert_eq!(blocks.len(), 41);
        let fields = &blocks["virtual_line"];
        let FieldValue::Blocks(buttons) = &fields["button"] else { panic!("ordered buttons") };
        assert_eq!(buttons.len(), 2);
        assert_eq!(required_atom(&buttons[0], "name", "button").unwrap(), "expand_ahead");
        assert_eq!(required_atom(&buttons[1], "name", "button").unwrap(), "expand_behind");
        for button in buttons {
            assert_eq!(required_atom(button, "type", "button").unwrap(), "btn_drag");
        }
        assert_eq!(required_f32_pair(&buttons[0], "position", "button").unwrap(), [1.0, 0.0]);
        assert_eq!(required_f32_pair(&buttons[1], "position", "button").unwrap(), [0.0, 0.0]);
        assert_eq!(required_f32_pair(fields, "body_width", "virtual_line").unwrap(), [0.5, 0.5]);
        let indices = arrow_definition_indices(source).unwrap();
        assert_eq!(indices["virtual_line"], 1);
        assert_eq!(indices["supply_to_consumers"], 40);
        let engine = portable_engine_evidence::map_scene_residuals::DECOMPILES
            .iter().find(|row| row.0 == "141255760").unwrap().1;
        assert!(engine.contains("(longlong)(*(int *)(param_1 + 0xec) + -1) * 0x58"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn definition_parser_rejects_malformed_button_collections() {
        for source in [
            "arrowType = { name = line button = scalar }",
            "arrowType = { name = line button = { name = ahead } button = scalar }",
            "arrowType = { name = line button = { name = ahead } button = { name = behind }",
            "arrowType = { name = line button = { name = ahead name = behind } }",
        ] {
            assert!(parse_arrow_blocks(source).is_err(), "{source}");
        }
    }

    #[test]
    fn authored_shadow_flags_preserve_defaults_false_and_invalid() {
        for (value, expected) in [("yes", true), ("no", false)] {
            let source = format!("arrowType = {{ name = arrow draw_shadows = {value} }}");
            let blocks = parse_arrow_blocks(&source).unwrap();
            assert_eq!(optional_bool(&blocks["arrow"], "draw_shadows", "arrow").unwrap(), Some(expected));
        }
        let blocks = parse_arrow_blocks("arrowType = { name = arrow }").unwrap();
        assert_eq!(optional_bool(&blocks["arrow"], "draw_shadows", "arrow").unwrap().unwrap_or(true), true);
        let blocks = parse_arrow_blocks("arrowType = { name = arrow draw_shadows = perhaps }").unwrap();
        assert!(optional_bool(&blocks["arrow"], "draw_shadows", "arrow").is_err());
    }

    #[test]
    fn definition_parser_rejects_unterminated_blocks() {
        let source = r#"arrowType = { name = "move_thick" body_width = { 1 2 }"#;
        assert!(parse_arrow_blocks(source).is_err());
    }

    #[test]
    fn definition_parser_applies_verified_scalar_assignments_in_authored_order() {
        let callbacks = [
            ("name", "0x1b", "param_1 + 8", "before", "after"),
            ("technique", "0x2e55", "param_1 + 0x58", "MapArrow", "MapArrowNoHead"),
            ("fade_out", "0x2ab", "param_1 + 0xbc", "0.4", "0.7"),
            ("fade_in", "0x2ac", "param_1 + 0xb8", "0.3", "0.6"),
            ("draw_shadows", "0x2821", "param_1 + 0x101", "yes", "no"),
            ("global_scale_min_size", "0x3037", "param_1 + 0xd0", "2", "3"),
            ("global_scale_max_points", "0x3038", "param_1 + 0xd4", "6", "9"),
            ("draw_last", "0x3642", "param_1 + 0x100", "no", "yes"),
            ("body_uv_scale_2", "0x3a4c", "param_1 + 0xb4", "1.2", "2.3"),
            ("texture_variant_count", "0x3a4d", "param_1 + 0xfc", "4", "7"),
            ("head_length", "0x2e54", "param_1 + 0xa8", "3.1", "4.2"),
            ("texture_loop", "0x2e56", "param_1 + 0xac", "2.4", "3.5"),
            ("body_uv_scale", "0x2e59", "param_1 + 0xb0", "5.2", "6.3"),
            ("text_size", "0x2ef2", "param_1 + 0xc0", "12", "16"),
            ("text_offset", "0x2ef3", "param_1 + 0xc4", "0.8", "0.9"),
            ("width_scale", "0x2efb", "param_1 + 200", "1.4", "1.7"),
            ("width_scale_max", "0x2efc", "param_1 + 0xcc", "2.1", "2.8"),
        ];
        let engine = portable_engine_evidence::map_scene_residuals::DECOMPILES
            .iter().find(|row| row.0 == "141255760").unwrap().1;
        for (field, token, destination, first, last) in callbacks {
            assert!(engine.contains(token), "{field}");
            assert!(engine.contains(destination), "{field}");
            let source = format!("arrowType = {{ name = line {field} = {first} button = {{ name = ahead }} {field} = {last} button = {{ name = behind }} }}");
            let entries = parse_arrow_entries(&source).unwrap();
            let (name, fields) = &entries[0];
            assert_eq!(name, if field == "name" { last } else { "line" });
            assert_eq!(required_atom(fields, field, name).unwrap(), last, "{field}");
            let FieldValue::Blocks(buttons) = &fields["button"] else { panic!("buttons") };
            assert_eq!(buttons.len(), 2);
            assert_eq!(required_atom(&buttons[0], "name", "button").unwrap(), "ahead");
            assert_eq!(required_atom(&buttons[1], "name", "button").unwrap(), "behind");
        }
    }

    #[test]
    fn definition_parser_rejects_duplicates_without_assignment_law() {
        for fields in [
            "unknown = first unknown = second",
            "unknown = { first } unknown = { second }",
            "technique = { first } technique = second",
            "technique = first technique = { second }",
            "texture_masks = { a.dds } texture_masks = { b.dds }",
        ] {
            assert!(parse_arrow_blocks(&format!("arrowType = {{ name = line {fields} }}")).is_err(), "{fields}");
        }
    }

    #[test]
    fn plan_techniques_fail_closed_outside_the_exact_arrow_state_contract() {
        for technique in [
            "MapArrowDefault",
            "MapArrowDefaultWithDepth",
            "MapArrowDefaultWithDepthTestOnly",
            "MapArrowNoHeadWidthDepth",
            "MapArrowNoHead",
        ] {
            assert!(xmap15_plan_technique_supported(technique));
        }
        assert!(!xmap15_plan_technique_supported("MapSymbolDefault"));
        assert!(!xmap15_plan_technique_supported("modded_unknown_effect"));
    }

    #[test]
    fn production_world_join_admits_only_native_paradrop_endpoint_order() {
        struct Units { census: String, admission: String }
        impl Xmap15UnitIdentityProvider for Units {
            fn hour_index(&self) -> i64 { 0 }
            fn census_sha256(&self) -> &str { &self.census }
            fn admission_sha256(&self) -> &str { &self.admission }
            fn formation_identity_match_count(&self, group: bool, id: u32, country: &str) -> usize {
                usize::from(!group && id == 17 && country == "GER")
            }
            fn division_identity_match_count(&self, id: u32, country: &str) -> usize {
                usize::from(id == 301 && country == "GER")
            }
        }
        let world = crate::test_support::frame_with_rest(serde_json::json!({
            "tag_db_id_index": {"country_composition_version": 2},
            "order_of_battle": {"extra": {"army_battleplans_depth_live": {
                "known": true, "baseline_seen": true, "omitted_non_exact_rows_this_hour": 0,
                "path_geometry_omitted_rows_this_hour": 0, "identity_or_active_omitted_rows_this_hour": 0,
                "provenance_notes": [],
                "orders_group_manager": {"known": true, "complete": true,
                    "source": portable_render_stream::XMAP13_ORDERS_GROUP_MANAGER_AUTHORITY,
                    "registered_entry_count": 1, "army_group_count": 0, "army_group_ids": [],
                    "armies": [{"army_id": 17, "execution_type": 2, "cohesion_type": 3, "leader_proximity": 4}]},
                "orders": [{"owner_kind": "army", "owner_id": 17, "army_id": 17,
                    "engine_order_instance_id": 2025, "generation": 4, "slot_index": 1,
                    "order_type": 4, "target_states": [], "path_provinces": [101, 102],
                    "target_declared_count": null, "target_status": "unsupported",
                    "geometry_status": "exact", "execution_status": "exact",
                    "assigned_division_ids": [301], "assigned_units_status": "exact",
                    "order_linkage_status": "exact", "active": false, "active_status": "exact",
                    "style_flags": 0, "style_status": "exact",
                    "linked_orders": {"version": 2, "status": 1, "predecessors": [], "successors": []}}]
            }}}
        }));
        let route = portable_render_stream::Xmap15PreparedRoute::from_paradrop_province_anchors(
            [102, 101], [[2, 3], [1, 3]], 10, 0).unwrap();
        let row = portable_render_stream::Xmap15PlanSemanticRow {
            key: Xmap15PlanKey { owner_kind: portable_render_stream::Xmap15OwnerKind::Army,
                owner_id: 17, engine_order_instance_id: 2025, generation: 4, slot_index: 1 },
            owner_country: "GER".into(),
            selector: portable_render_stream::Xmap15PlanDefinitionSelector {
                order_type: 4, move_special_118: false, withdraw_119: false,
                fallback_298: false, virtual_299: false, blitz_118: false, army_group_style_39_and_378: false },
            resolved_definition_name: "paradrop".into(), ordered_assigned_division_ids: vec![301],
            assigned: true, active: false, route, manager_insertion_ordinal: 0, definition_group_ordinal: 0,
            object_width_multiplier_f32_bits: 0, comparator_tie_17c_f32_bits: 0,
            fade_in_f32_bits: 0, fade_out_f32_bits: 0, culling_offset_f32_bits: 0,
            inner_rgba_f32_bits: [1.0f32.to_bits(); 4], outer_rgba_f32_bits: [1.0f32.to_bits(); 4],
            texture_variant: 0, see_through: false, visible: true, final_engine_draw_ordinal: 0,
            exact_head_vertices_f32_bits: None, lifecycle_tombstone_sha256: "a".repeat(64),
        };
        let mut semantic = Xmap15SemanticFrame {
            schema: portable_render_stream::XMAP15_SEMANTIC_INPUT_SCHEMA.into(),
            executable_sha256: portable_render_stream::M03_A729_EXECUTABLE_SHA256.into(),
            producer_registration_sha256: portable_render_stream::xmap15_semantic_producer_registration_sha256(),
            hour_index: 0, observation_sequence: 0, baseline_complete: true,
            omitted_plan_rows: 0, omitted_movement_rows: 0, plan_object_census: 1, movement_object_census: 0,
            plans: vec![row], movements: vec![], enumerator_witness_sha256: "b".repeat(64),
        };
        let units = Units { census: "c".repeat(64), admission: "d".repeat(64) };
        assert!(cross_check_d54_and_units(&world, &semantic, &units).is_ok());
        semantic.plans[0].route.ordered_province_ids.reverse();
        assert!(cross_check_d54_and_units(&world, &semantic, &units).unwrap_err().to_string().contains("disagrees with exact D54 state"));
        semantic.plans[0].route.ordered_province_ids.reverse();
        semantic.plans[0].key.generation += 1;
        assert!(cross_check_d54_and_units(&world, &semantic, &units).is_err());
        semantic.plans[0].key.generation -= 1;
        semantic.plans[0].owner_country = "FRA".into();
        assert!(cross_check_d54_and_units(&world, &semantic, &units).is_err());
    }

    #[test]
    fn authenticated_empty_lifecycle_update_deletes_retained_rows() {
        let portable = Xmap15PreparedFrame {
            schema: XMAP15_STATE_FIRST_SCHEMA.into(),
            hour_index: 44,
            observation_sequence: 9,
            semantic_input_sha256: "a".repeat(64),
            enumerator_witness_sha256: "b".repeat(64),
            xmap13_census_sha256: "c".repeat(64),
            xmap13_admission_sha256: "d".repeat(64),
            selected_plan: None,
            ordered_rows: Vec::new(),
            ordered_used_content: Vec::new(),
            deterministic_sha256: String::new(),
        }
        .seal()
        .unwrap();
        let frame = PreparedXmap15Frame {
            portable,
            assigned_stage: AssignedStage::seal(
                "f".repeat(64),
                XMAP15_ASSIGNED_STAGE_NAME,
                XMAP15_ASSIGNED_OPERATION_NAME,
                4,
            )
            .unwrap(),
            foundation_target_sha256: "e".repeat(64),
            plan_definitions: Vec::new(),
            movement_definitions: Vec::new(),
            heightmap: None,
            paradrop_profile: None,
            map_dimensions_content: None,
            map_width: 0,
            map_height: 0,
        };
        let mut state = Xmap15LifecycleState::default();
        state
            .retained_rows
            .insert("plan:old".into(), "e".repeat(64));
        let update = state.apply(&frame).unwrap();
        assert!(update.authenticated_empty);
        assert_eq!(update.deleted, vec!["plan:old"]);
        assert!(state.retained_rows.is_empty());
    }

    #[test]
    fn paradrop_profile_uses_effective_lua_overrides_and_refuses_missing_operands() {
        let sources = [
            ("common/defines/00_graphics.lua".into(),
                "NDefines = { NGraphics = { ARROW_PARADROP_HEIGHT_TO_LEN = 0.5, ARROW_PARADROP_HEIGHT_MAX = 20 } }".into()),
            ("common/defines/01_mod.lua".into(),
                "NDefines.NGraphics.ARROW_PARADROP_HEIGHT_TO_LEN = NDefines.NGraphics.ARROW_PARADROP_HEIGHT_TO_LEN * 3; NDefines.NGraphics.ARROW_PARADROP_HEIGHT_MAX = -0.0".into()),
        ];
        let evaluated = crate::xmap09_lighting_law::EffectiveDefines::from_sources(&sources).unwrap();
        let profile = paradrop_profile_from_effective(&evaluated).unwrap();
        assert_eq!(profile.height_to_length_f32_bits, 1.5f32.to_bits());
        assert_eq!(profile.max_height_f32_bits, (-0.0f32).to_bits());
        for mutation in ["nil", "'wrong'", "0/0", "1/0"] {
            let mut broken = sources.to_vec();
            broken.push(("common/defines/02_bad.lua".into(),
                format!("NDefines.NGraphics.ARROW_PARADROP_HEIGHT_MAX = {mutation}")));
            let evaluated = crate::xmap09_lighting_law::EffectiveDefines::from_sources(&broken).unwrap();
            assert!(paradrop_profile_from_effective(&evaluated).is_err(), "{mutation}");
        }
    }

    fn sampler_from_contract(contract: Xmap15SamplerContract) -> crate::EffectSampler {
        crate::EffectSampler {
            name: contract.name.into(),
            index: contract.index,
            mag_filter: Some(contract.mag.into()),
            min_filter: Some(contract.min.into()),
            mip_filter: Some(contract.mip.into()),
            address_u: Some(contract.address.into()),
            address_v: Some(contract.address.into()),
            sampler_type: contract.sampler_type.map(str::to_string),
            mip_map_lod_bias: contract.lod_bias.map(|value| value.to_string()),
            mip_map_min_lod: contract.min_lod.map(|value| value.to_string()),
            mip_map_max_lod: contract.max_lod.map(|value| value.to_string()),
            max_anisotropy: None,
        }
    }

    fn state(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).into(), (*value).into()))
            .collect()
    }

    fn exact_plan_effect_file() -> crate::EffectFile {
        let mut file = crate::EffectFile::default();
        file.pixel_stage.samplers = XMAP15_PLAN_SAMPLERS
            .into_iter()
            .map(sampler_from_contract)
            .collect();
        file.blend_states.insert(
            "BlendState".into(),
            state(&[
                ("BlendEnable", "yes"),
                ("AlphaTest", "no"),
                ("SourceBlend", "src_alpha"),
                ("DestBlend", "inv_src_alpha"),
                ("WriteMask", "RED|GREEN|BLUE"),
            ]),
        );
        for name in ["DepthStencilState", "DepthStencilStateSeethrough"] {
            file.depth_stencil_states
                .insert(name.into(), state(xmap15_depth_contract(name).unwrap()));
        }
        file.effects = vec![
            crate::shader_compile::EffectDef {
                name: "MapArrowNoHead".into(),
                vertex_shader: Some("ArrowVertexShader".into()),
                pixel_shader: Some("ArrowPixelShaderNoHead".into()),
                defines: vec!["ANIM_TEXTURE".into()],
                blend_state: None,
                depth_stencil_state: Some("DepthStencilState".into()),
                rasterizer_state: None,
            },
            crate::shader_compile::EffectDef {
                name: "MapArrowNoHeadSeethrough".into(),
                vertex_shader: Some("ArrowVertexShader".into()),
                pixel_shader: Some("ArrowPixelShaderNoHead".into()),
                defines: vec!["ANIM_TEXTURE".into(), "SEETHROUGH".into()],
                blend_state: Some("BlendState".into()),
                depth_stencil_state: Some("DepthStencilStateSeethrough".into()),
                rasterizer_state: None,
            },
        ];
        file
    }

    #[test]
    fn effective_plan_state_and_sampler_contract_fails_closed_on_mod_drift() {
        let effects = BTreeSet::from([
            "MapArrowNoHead".to_string(),
            "MapArrowNoHeadSeethrough".to_string(),
        ]);
        let exact = exact_plan_effect_file();
        validate_xmap15_plan_effect_file_contracts(&exact, &effects).unwrap();

        let mut sampler_drift = exact.clone();
        sampler_drift.pixel_stage.samplers[0].address_u = Some("Clamp".into());
        assert!(validate_xmap15_plan_effect_file_contracts(&sampler_drift, &effects).is_err());

        let mut state_drift = exact.clone();
        state_drift
            .depth_stencil_states
            .get_mut("DepthStencilState")
            .unwrap()[0] = ("DepthEnable".into(), "yes".into());
        assert!(validate_xmap15_plan_effect_file_contracts(&state_drift, &effects).is_err());

        let mut effect_drift = exact;
        effect_drift.effects[0].rasterizer_state = Some("ModRaster".into());
        assert!(validate_xmap15_plan_effect_file_contracts(&effect_drift, &effects).is_err());
    }

    #[test]
    fn effective_movement_state_and_sampler_contract_fails_closed_on_mod_drift() {
        let mut file = crate::EffectFile::default();
        file.pixel_stage.samplers = XMAP15_MOVEMENT_SAMPLERS
            .into_iter()
            .map(sampler_from_contract)
            .collect();
        file.depth_stencil_states.insert(
            "NoDepthStencilState".into(),
            state(xmap15_depth_contract("NoDepthStencilState").unwrap()),
        );
        file.effects.push(crate::shader_compile::EffectDef {
            name: "ArrowEffect".into(),
            vertex_shader: Some("VertexShader".into()),
            pixel_shader: Some("PixelShader".into()),
            defines: Vec::new(),
            blend_state: None,
            depth_stencil_state: Some("NoDepthStencilState".into()),
            rasterizer_state: None,
        });
        validate_xmap15_movement_effect_file_contract(&file).unwrap();

        file.pixel_stage.samplers[2].mip_filter = Some("Point".into());
        assert!(validate_xmap15_movement_effect_file_contract(&file).is_err());
    }

    /// kills: the absent-carrier arm saying only that the key is missing.
    ///
    /// "This recording carries no key X" is true of a recorder that never ran
    /// the producer AND of an hour whose inputs it could not read, and the two
    /// send a reader at different work. The recording's own gate separates
    /// them, and a gate stamped with another hour is refused rather than
    /// quoted, because its counts describe an hour that is not this one.
    #[test]
    fn the_absent_carrier_arm_quotes_the_recordings_own_gate_and_refuses_another_hours() {
        let gate = portable_render_stream::Xmap15ProducerGate {
            schema: portable_render_stream::XMAP15_PRODUCER_GATE_SCHEMA.into(),
            hour_index: 17,
            minted_plan_rows: 3,
            visual_d54_orders: 3,
            hour_gates: vec![portable_render_stream::Xmap15HourGate {
                recorded_input: "units.divisions".into(),
                recorded_state: "the recording carries an explicit null for this section".into(),
            }],
        };
        let value = serde_json::to_value(&gate).expect("the gate serializes");
        let quoted = xmap15_account_from_gate(Some(&value), 17);
        assert!(quoted.contains("3 plan rows against 3 visual"), "{quoted}");
        assert!(quoted.contains("units.divisions"), "{quoted}");
        assert!(quoted.contains("false claim"), "{quoted}");

        let wrong_hour = xmap15_account_from_gate(Some(&value), 18);
        assert!(wrong_hour.contains("no usable XMAP15 producer gate"), "{wrong_hour}");
        assert_ne!(wrong_hour, quoted);

        let malformed = serde_json::json!({"schema": "something else"});
        assert!(xmap15_account_from_gate(Some(&malformed), 17)
            .contains("no usable XMAP15 producer gate"));

        // With no gate at all the constant half still reaches the reader.
        let absent = xmap15_account_from_gate(None, 17);
        assert!(
            absent.contains(&format!(
                "{} mandatory leaves",
                portable_render_stream::xmap15_contract_gate_leaves().len()
            )),
            "{absent}"
        );
    }
}
