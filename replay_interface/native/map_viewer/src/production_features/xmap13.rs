
use crate::{
    parse_building_entity_bindings, parse_building_pdxmesh_bindings, ExactUnitDisclosure,
    FoundationFeatureContext, FoundationFeatureOutput, FoundationGap, GameFileResolver,
    GuiContainerLayout, GuiControlLayout, L03DrawArguments, L03PdxMeshGroupInput,
    L03SemanticInputRow, L03TypedOwner, ProductionMapFeatureAdapter, Result,
    UnitCounterAssetManifest, Xmap05VisibilityReason, UNIT_COUNTER_CORE_GFX_SOURCE,
};
use portable_content_manifest::UsedContentKey;
use portable_render_stream::{
    AssignedStage, FamilyDisposition, MapVisualFamily, ParentAnchor, Xmap20ActiveEffect,
    Xmap20DependencyReceipt, XMAP20_ENTITY_IDENTITY_OWNER,
};
use replay_core::{Leaf, ValidatedContentAdmission, ViewStatus};
use serde::{Deserialize, Serialize};
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};

pub const XMAP13_INPUT_SCHEMA: &str = "hoi4.xmap13.unit-semantic.a729.v5";
pub const XMAP13_PREPARED_SCHEMA: &str = "hoi4.xmap13.unit-prepared.a729.v1";
pub const XMAP13_NATIVE_RECEIPT_SCHEMA: &str = "hoi4.xmap13.native-component-receipt.a729.v1";
pub const XMAP13_EFFECT_IDENTITY_RECEIPT_SCHEMA: &str =
    "hoi4.xmap13.effect-identity-receipt.a729.v1";
pub const XMAP13_ENGINE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const XMAP13_DOMAIN: &str = "map_unit_presentation";
pub const XMAP13_UNIT_ENTITY_STAGE_NAME: &str = "xmap13.unit_entities.execute";
pub const XMAP13_UNIT_ENTITY_OPERATION_NAME: &str = "units.entities.draw";
pub const XMAP13_COUNTER_QUAD_STAGE_NAME: &str = "xmap13.counter_quads.execute";
pub const XMAP13_COUNTER_QUAD_OPERATION_NAME: &str = "units.counter-quads.draw";
const GRAPHICS_DEFINES: &str = "common/defines/00_graphics.lua";

const OWNED_GAPS: [&str; 5] = [
    "orders.theatre_army_hierarchy",
    "units.air_base_location",
    "units.divisions",
    "units.taskforces_fleets",
    "units.wings",
];

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Xmap13UnitIdentity {
    Division { type_id: u32, stable_id: u32 },
    TaskForce { type_id: u32, stable_id: u32 },
    AirWing { stable_id: String },
}

impl Xmap13UnitIdentity {
    pub fn canonical_id(&self) -> String {
        match self {
            Self::Division { type_id, stable_id } => {
                format!("xmap13:division:{type_id}:{stable_id}")
            }
            Self::TaskForce { type_id, stable_id } => {
                format!("xmap13:task_force:{type_id}:{stable_id}")
            }
            Self::AirWing { stable_id } => format!("xmap13:air_wing:{stable_id}"),
        }
    }

    fn validate(&self) -> Result<()> {
        match self {
            Self::Division { type_id, stable_id } | Self::TaskForce { type_id, stable_id } => {
                if *type_id == 0 || *stable_id == 0 {
                    return Err("XMAP13 typed unit identity contains zero".into());
                }
            }
            Self::AirWing { stable_id } if stable_id.trim().is_empty() => {
                return Err("XMAP13 air-wing identity is blank".into());
            }
            Self::AirWing { .. } => {}
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap13AuthoredStateEvent {
    pub authored_event_index: u32,
    pub authored_effect_index: u32,
    pub effect_kind: Xmap13AuthoredEffectKind,
    pub effect_token: String,
    pub node: Option<String>,
    pub event_time_f32_bits: Option<u32>,
    pub keep_particle: Option<bool>,
    pub trigger_once: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Xmap13AuthoredEffectKind {
    Particle,
    Sound,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap13TaskForceMember {
    pub ship_instance_id: i64,
    pub semantic_ship_uid: String,
    pub definition_id: String,
    pub definition_content: UsedContentKey,
    pub hull_id: String,
    pub hull_content: UsedContentKey,
    pub ordered_category_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap13FormationIdentity {
    pub owner_is_army_group: bool,
    pub owner_id: u32,
    pub owner_country: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap13CounterGroupMembership {
    pub grouping_tier: u8,
    pub allegiance: Xmap13CounterAllegiance,
    pub stable_group_id: String,
    pub member_world_anchor_f32_bits: [u32; 3],
    pub member_ordinal: u32,
    pub member_count: u32,
    pub counter_item_id: String,
    pub counter_item_kind: Xmap13CounterItemKind,
    pub counter_item_ordinal: u32,
    pub counter_item_count: u32,
    pub counter_item_member_ordinal: u32,
    pub counter_item_member_count: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Xmap13CounterItemKind {
    Land,
    Naval,
    RailwayGun,
    Air,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Xmap13CounterHorizontalAlignment {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Xmap13CounterVerticalAlignment {
    Top,
    Center,
    Bottom,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum Xmap13CounterTextValue {
    ItemMemberCount,
    Literal { value: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum Xmap13CounterSpriteBinding {
    AuthoredGui,
    RuntimeRebound { authored_gui_sprite_name: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Xmap13CounterFrameSelection {
    AuthoredFixed,
    AuthoredDefaultFirst,
    RuntimeSelected,
    RuntimeSelectedOverAuthoredInitial,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "primitive", rename_all = "snake_case", deny_unknown_fields)]
pub enum Xmap13CounterPrimitive {
    Sprite {
        stable_layer_id: String,
        authored_control_name: String,
        authored_sprite_name: String,
        binding: Xmap13CounterSpriteBinding,
        authored_frame_index: Option<u32>,
        frame_selection: Xmap13CounterFrameSelection,
        runtime_collection_instance_ordinal: Option<u32>,
        texture_content: UsedContentKey,
        source_rect_px: [u32; 4],
        destination_rect_f32_bits: [u32; 4],
        rotation_f32_bits: u32,
        visible_fraction_f32_bits: Option<u32>,
        modulation_rgba_f32_bits: [u32; 4],
    },
    Text {
        stable_layer_id: String,
        authored_control_name: String,
        font_name: String,
        ordered_font_content: Vec<UsedContentKey>,
        value: Xmap13CounterTextValue,
        control_rect_f32_bits: [u32; 4],
        horizontal_alignment: Xmap13CounterHorizontalAlignment,
        vertical_alignment: Xmap13CounterVerticalAlignment,
        color_argb: u32,
    },
    Hidden {
        stable_layer_id: String,
        authored_control_name: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap13CounterPresentation {
    pub grouping_tier: u8,
    pub counter_item_id: String,
    pub allegiance: Xmap13CounterAllegiance,
    pub selected: bool,
    pub anchor_offset_px_f32_bits: [u32; 2],
    pub stack_advance_px_f32_bits: u32,
    pub active_ability_instance_count: u32,
    pub ordered_primitives: Vec<Xmap13CounterPrimitive>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap13CounterSemantic {
    pub counter_kind: String,
    pub ordered_interface_content: Vec<UsedContentKey>,
    pub ordered_group_memberships: Vec<Xmap13CounterGroupMembership>,
    pub ordered_presentations: Vec<Xmap13CounterPresentation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap13SemanticRow {
    pub identity: Xmap13UnitIdentity,
    pub semantic_source_id: String,
    pub entity_instance_id: u64,
    pub owner_country: String,
    pub anchor_province_id: u32,
    pub entity_asset: UsedContentKey,
    pub model_registry: UsedContentKey,
    pub mesh: UsedContentKey,
    pub shader_source: UsedContentKey,
    pub animation_asset: Option<UsedContentKey>,
    pub ordered_texture_content: Vec<UsedContentKey>,
    pub entity_name: String,
    pub model_name: String,
    pub state_name: String,
    pub material_variant: String,
    pub animation_name: Option<String>,
    pub animation_phase_f32_bits: u32,
    pub animation_speed_f32_bits: u32,
    pub selected_lod_index: u32,
    pub selected_shape_indices: Vec<u32>,
    pub groups: Vec<L03PdxMeshGroupInput>,
    pub world_matrix_f32_bits: [u32; 16],
    pub skinned: bool,
    pub skin_palette_sha256: Option<String>,
    pub skin_palette_3200_bytes: Option<Vec<u8>>,
    pub casts_light_depth: bool,
    pub authored_event_block_count: u32,
    pub authored_state_events: Vec<Xmap13AuthoredStateEvent>,
    pub counter: Option<Xmap13CounterSemantic>,
    pub task_force_members: Option<Vec<Xmap13TaskForceMember>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap13SemanticEnvelope {
    pub schema: String,
    pub hour_index: i64,
    pub cadence: String,
    pub identity_order: String,
    pub complete_division_census: bool,
    pub complete_task_force_census: bool,
    pub complete_air_wing_census: bool,
    pub complete_formation_identity_census: bool,
    pub producer_executable_sha256: String,
    pub producer_evidence_manifest_sha256: String,
    pub ordered_identity_sha256: String,
    pub ordered_formation_identity_sha256: String,
    pub rows: Vec<Xmap13SemanticRow>,
    pub ordered_formations: Vec<Xmap13FormationIdentity>,
    pub deterministic_sha256: String,
}

impl Xmap13SemanticEnvelope {
    pub fn seal(mut self) -> Result<Self> {
        self.ordered_identity_sha256 =
            ordered_identity_digest(self.rows.iter().map(|row| &row.identity))?;
        self.ordered_formation_identity_sha256 =
            ordered_formation_identity_digest(&self.ordered_formations)?;
        self.deterministic_sha256.clear();
        self.deterministic_sha256 = Sha256::hex(&serde_json::to_vec(&self)?);
        self.validate_for_hour(self.hour_index)?;
        Ok(self)
    }

    pub fn validate_for_hour(&self, hour_index: i64) -> Result<()> {
        if self.schema != XMAP13_INPUT_SCHEMA
            || self.hour_index != hour_index
            || self.cadence != "every_world_frame"
            || self.identity_order != "engine_enumerator_order"
            || !self.complete_division_census
            || !self.complete_task_force_census
            || !self.complete_air_wing_census
            || !self.complete_formation_identity_census
            || self.producer_executable_sha256 != XMAP13_ENGINE_SHA256
        {
            return Err("XMAP13 semantic envelope header/completeness is invalid".into());
        }
        validate_sha256(
            &self.producer_evidence_manifest_sha256,
            "XMAP13 producer evidence manifest",
        )?;
        validate_sha256(&self.ordered_identity_sha256, "XMAP13 ordered identity")?;
        validate_sha256(
            &self.ordered_formation_identity_sha256,
            "XMAP13 ordered formation identity",
        )?;
        validate_sha256(&self.deterministic_sha256, "XMAP13 semantic envelope")?;
        let expected_order = ordered_identity_digest(self.rows.iter().map(|row| &row.identity))?;
        if self.ordered_identity_sha256 != expected_order {
            return Err("XMAP13 semantic envelope ordered identity digest mismatch".into());
        }
        if self.ordered_formation_identity_sha256
            != ordered_formation_identity_digest(&self.ordered_formations)?
        {
            return Err("XMAP13 semantic envelope ordered formation digest mismatch".into());
        }
        validate_formation_rows(&self.ordered_formations)?;
        validate_complete_counter_memberships(&self.rows)?;
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        if self.deterministic_sha256 != Sha256::hex(&serde_json::to_vec(&unsigned)?) {
            return Err("XMAP13 semantic envelope digest mismatch".into());
        }
        Ok(())
    }
}

/// particle ownership only needs the complete engine-order entity census and
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap13EffectIdentityReceipt {
    pub schema: String,
    pub hour_index: i64,
    pub semantic_envelope_sha256: String,
    pub producer_evidence_manifest_sha256: String,
    pub ordered_identity_sha256: String,
    pub ordered_semantic_source_ids: Vec<String>,
    pub ordered_semantic_source_ids_sha256: String,
    pub admitted_content_sha256: String,
    pub deterministic_sha256: String,
}

impl Xmap13EffectIdentityReceipt {
    fn seal(mut self) -> Result<Self> {
        self.deterministic_sha256.clear();
        self.validate_body()?;
        self.deterministic_sha256 = Sha256::hex(&serde_json::to_vec(&self)?);
        self.validate()?;
        Ok(self)
    }

    pub fn validate(&self) -> Result<()> {
        self.validate_body()?;
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        if self.deterministic_sha256 != Sha256::hex(&serde_json::to_vec(&unsigned)?) {
            return Err("XMAP13 effect-identity receipt digest mismatch".into());
        }
        Ok(())
    }

    fn validate_body(&self) -> Result<()> {
        if self.schema != XMAP13_EFFECT_IDENTITY_RECEIPT_SCHEMA || self.hour_index < 0 {
            return Err("XMAP13 effect-identity receipt header is invalid".into());
        }
        for (label, value) in [
            ("semantic envelope", &self.semantic_envelope_sha256),
            ("producer evidence", &self.producer_evidence_manifest_sha256),
            ("ordered identity", &self.ordered_identity_sha256),
            (
                "ordered semantic-source census",
                &self.ordered_semantic_source_ids_sha256,
            ),
            ("admitted content", &self.admitted_content_sha256),
        ] {
            validate_sha256(value, &format!("XMAP13 {label}"))?;
        }
        let mut unique = BTreeSet::new();
        if self
            .ordered_semantic_source_ids
            .iter()
            .any(|value| value.trim().is_empty() || !unique.insert(value))
            || self.ordered_semantic_source_ids_sha256
                != semantic_source_id_digest(&self.ordered_semantic_source_ids)
        {
            return Err("XMAP13 effect semantic-source census is invalid".into());
        }
        Ok(())
    }

    pub fn xmap20_dependency_receipt(&self) -> Result<Xmap20DependencyReceipt> {
        self.validate()?;
        Ok(Xmap20DependencyReceipt {
            owner_packet: XMAP20_ENTITY_IDENTITY_OWNER.into(),
            provider_schema: XMAP13_EFFECT_IDENTITY_RECEIPT_SCHEMA.into(),
            payload_sha256: self.semantic_envelope_sha256.clone(),
            descriptor_sha256: self.ordered_semantic_source_ids_sha256.clone(),
            execution_sha256: self.deterministic_sha256.clone(),
        })
    }
}

#[derive(Clone, Debug)]
pub struct Xmap13EffectIdentityProvider {
    receipt: Xmap13EffectIdentityReceipt,
    ordered_rows: Vec<Xmap13SemanticRow>,
}

impl Xmap13EffectIdentityProvider {
    pub fn receipt(&self) -> &Xmap13EffectIdentityReceipt {
        &self.receipt
    }

    pub fn ordered_semantic_source_ids(&self) -> &[String] {
        &self.receipt.ordered_semantic_source_ids
    }

    pub fn trigger_for_effect(
        &self,
        effect: &Xmap20ActiveEffect,
    ) -> Result<crate::Xmap20TriggerReceipt> {
        self.receipt.validate()?;
        let (entity_instance_id, node_token) = match &effect.parent_anchor {
            ParentAnchor::Entity {
                entity_instance_id,
                node_token,
            } => {
                if effect.producer_kind != "unit" {
                    return Err("XMAP20 entity effect has a non-unit producer kind".into());
                }
                (entity_instance_id, node_token)
            }
            ParentAnchor::World { .. } => {
                return Err("XMAP13 cannot authenticate a world-anchored XMAP20 effect".into())
            }
        };
        let row = self
            .ordered_rows
            .iter()
            .find(|row| row.semantic_source_id == effect.producer_instance_id)
            .ok_or("XMAP20 entity source is outside the authenticated XMAP13 census")?;
        let event = row
            .authored_state_events
            .iter()
            .find(|event| {
                event.authored_event_index == effect.authored_event_index
                    && event.authored_effect_index == effect.authored_effect_index
            })
            .ok_or("XMAP20 effect has no exact authored XMAP13 event row")?;
        if row.entity_instance_id != *entity_instance_id
            || event.effect_kind != Xmap13AuthoredEffectKind::Particle
            || event.effect_token != effect.registration_token
            || event.node.as_deref() != Some(node_token.as_str())
        {
            return Err("XMAP20 effect differs from its authored XMAP13 particle row".into());
        }
        crate::Xmap20TriggerReceipt {
            schema: crate::XMAP20_TRIGGER_RECEIPT_SCHEMA.into(),
            semantic_owner_packet: "XMAP13".into(),
            semantic_source_id: row.semantic_source_id.clone(),
            entity_instance_id: Some(row.entity_instance_id),
            source_content: row.entity_asset.clone(),
            entity_name: Some(row.entity_name.clone()),
            state_name: Some(row.state_name.clone()),
            authored_event_index: event.authored_event_index,
            authored_effect_index: event.authored_effect_index,
            particle_token: event.effect_token.clone(),
            node_token: event.node.clone(),
            time_f32_bits: event.event_time_f32_bits,
            keep_particle: event.keep_particle,
            trigger_once: event.trigger_once,
            owner_admission_sha256: self.receipt.deterministic_sha256.clone(),
            deterministic_sha256: String::new(),
        }
        .seal()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Xmap13CounterAllegiance {
    WorldObserver,
    Ours,
    Enemy,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedXmap13CounterItem {
    pub stable_item_id: String,
    pub kind: Xmap13CounterItemKind,
    pub item_ordinal: u32,
    pub ordered_member_identities: Vec<Xmap13UnitIdentity>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedXmap13CounterGroup {
    pub stable_group_id: String,
    pub grouping_tier: u8,
    pub group_world_anchor_f32_bits: [u32; 3],
    pub allegiance: Xmap13CounterAllegiance,
    pub ordered_member_identities: Vec<Xmap13UnitIdentity>,
    pub ordered_items: Vec<PreparedXmap13CounterItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedXmap13HitTarget {
    pub identity: Xmap13UnitIdentity,
    pub min_screen_f32_bits: [u32; 2],
    pub max_screen_f32_bits: [u32; 2],
    pub depth_f32_bits: u32,
}

impl PreparedXmap13HitTarget {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        let min = self.min_screen_f32_bits.map(f32::from_bits);
        let max = self.max_screen_f32_bits.map(f32::from_bits);
        x.is_finite() && y.is_finite() && x >= min[0] && x <= max[0] && y >= min[1] && y <= max[1]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedXmap13Unit {
    pub identity: Xmap13UnitIdentity,
    pub semantic_source_id: String,
    pub entity_instance_id: u64,
    pub owner_country: String,
    pub anchor_province_id: u32,
    pub entity_asset: UsedContentKey,
    pub model_registry: UsedContentKey,
    pub mesh: UsedContentKey,
    pub shader_source: UsedContentKey,
    pub animation_asset: Option<UsedContentKey>,
    pub ordered_texture_content: Vec<UsedContentKey>,
    pub entity_name: String,
    pub model_name: String,
    pub state_name: String,
    pub material_variant: String,
    pub animation_name: Option<String>,
    pub animation_phase_f32_bits: u32,
    pub animation_speed_f32_bits: u32,
    pub authored_event_block_count: u32,
    pub authored_state_events: Vec<Xmap13AuthoredStateEvent>,
    pub task_force_members: Option<Vec<Xmap13TaskForceMember>>,
    pub counter: Option<Xmap13CounterSemantic>,
    pub world_matrix_f32_bits: [u32; 16],
    pub disclosure: ExactUnitDisclosure,
    pub visibility_reason: Xmap05VisibilityReason,
    pub selected: bool,
    pub selected_lod_index: u32,
    pub selected_shape_indices: Vec<u32>,
    pub selected_mesh: String,
    pub groups: Vec<L03PdxMeshGroupInput>,
    pub skinned: bool,
    pub skin_palette_sha256: Option<String>,
    pub skin_palette_3200_bytes: Option<Vec<u8>>,
    pub partition_mask: u8,
    pub casts_light_depth: bool,
    pub local_aabb_min_f32_bits: [u32; 3],
    pub local_aabb_max_f32_bits: [u32; 3],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap13IdentityProvider {
    hour_index: i64,
    ordered_units: Vec<PreparedXmap13Unit>,
    ordered_formations: Vec<Xmap13FormationIdentity>,
    census_sha256: String,
    formation_census_sha256: String,
    admission_sha256: String,
}

impl Xmap13IdentityProvider {
    pub fn hour_index(&self) -> i64 {
        self.hour_index
    }

    pub fn ordered_units(&self) -> &[PreparedXmap13Unit] {
        &self.ordered_units
    }

    pub fn lookup(&self, identity: &Xmap13UnitIdentity) -> Option<&PreparedXmap13Unit> {
        self.ordered_units
            .iter()
            .find(|row| &row.identity == identity)
    }

    pub fn lookup_semantic_source_id(&self, source: &str) -> Option<&PreparedXmap13Unit> {
        self.ordered_units
            .iter()
            .find(|row| row.semantic_source_id == source)
    }

    pub fn division_identity_match_count(&self, stable_id: u32, owner_country: &str) -> usize {
        self.ordered_units
            .iter()
            .filter(|row| {
                matches!(
                    row.identity,
                    Xmap13UnitIdentity::Division {
                        stable_id: candidate,
                        ..
                    } if candidate == stable_id
                ) && row.owner_country == owner_country
            })
            .count()
    }

    pub fn formation_identity_match_count(
        &self,
        owner_is_army_group: bool,
        owner_id: u32,
        owner_country: &str,
    ) -> usize {
        self.ordered_formations
            .iter()
            .filter(|row| {
                row.owner_is_army_group == owner_is_army_group
                    && row.owner_id == owner_id
                    && row.owner_country == owner_country
            })
            .count()
    }

    pub fn ordered_formations(&self) -> &[Xmap13FormationIdentity] {
        &self.ordered_formations
    }

    pub fn census_sha256(&self) -> &str {
        &self.census_sha256
    }

    pub fn formation_census_sha256(&self) -> &str {
        &self.formation_census_sha256
    }

    pub fn admission_sha256(&self) -> &str {
        &self.admission_sha256
    }
}

impl crate::production_features::xmap15::Xmap15UnitIdentityProvider for Xmap13IdentityProvider {
    fn hour_index(&self) -> i64 {
        self.hour_index()
    }

    fn census_sha256(&self) -> &str {
        self.census_sha256()
    }

    fn admission_sha256(&self) -> &str {
        self.admission_sha256()
    }

    fn formation_identity_match_count(
        &self,
        owner_is_army_group: bool,
        owner_id: u32,
        owner_country: &str,
    ) -> usize {
        self.formation_identity_match_count(owner_is_army_group, owner_id, owner_country)
    }

    fn division_identity_match_count(&self, stable_id: u32, owner_country: &str) -> usize {
        self.division_identity_match_count(stable_id, owner_country)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedXmap13Frame {
    pub schema: String,
    pub hour_index: i64,
    pub ordered_used_content: Vec<UsedContentKey>,
    pub unit_entity_assignment: AssignedStage,
    pub counter_quad_assignment: AssignedStage,
    pub identity_provider: Xmap13IdentityProvider,
    pub visible_units: Vec<PreparedXmap13Unit>,
    pub native_entity_draws: Vec<crate::Xmap12PdxMeshMaterialization>,
    pub displayed_counter_groups: Vec<PreparedXmap13CounterGroup>,
    pub hit_targets: Vec<PreparedXmap13HitTarget>,
    pub light_depth_casters: Vec<L03SemanticInputRow>,
    pub semantic_input_sha256: String,
    pub deterministic_sha256: String,
}

impl PreparedXmap13Frame {
    pub fn identity_provider(&self) -> &Xmap13IdentityProvider {
        &self.identity_provider
    }

    pub fn light_depth_casters(&self) -> &[L03SemanticInputRow] {
        &self.light_depth_casters
    }

    pub fn hit_test(&self, screen_x: f32, screen_y: f32) -> Option<&Xmap13UnitIdentity> {
        self.hit_targets
            .iter()
            .filter(|row| row.contains(screen_x, screen_y))
            .min_by(|left, right| {
                f32::from_bits(left.depth_f32_bits).total_cmp(&f32::from_bits(right.depth_f32_bits))
            })
            .map(|row| &row.identity)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP13_PREPARED_SCHEMA {
            return Err("XMAP13 prepared schema mismatch".into());
        }
        validate_sha256(&self.semantic_input_sha256, "XMAP13 semantic input")?;
        validate_sha256(&self.deterministic_sha256, "XMAP13 prepared frame")?;
        validate_sha256(&self.identity_provider.census_sha256, "XMAP13 census")?;
        validate_sha256(
            &self.identity_provider.formation_census_sha256,
            "XMAP13 formation census",
        )?;
        validate_sha256(&self.identity_provider.admission_sha256, "XMAP13 admission")?;
        self.unit_entity_assignment
            .validate()
            .map_err(|error| -> Box<dyn std::error::Error> {
                format!("XMAP13 unit-entity assignment: {error}").into()
            })?;
        self.counter_quad_assignment
            .validate()
            .map_err(|error| -> Box<dyn std::error::Error> {
                format!("XMAP13 counter-quad assignment: {error}").into()
            })?;
        if self.unit_entity_assignment.stage_name != XMAP13_UNIT_ENTITY_STAGE_NAME
            || self.unit_entity_assignment.operation_name != XMAP13_UNIT_ENTITY_OPERATION_NAME
            || self.counter_quad_assignment.stage_name != XMAP13_COUNTER_QUAD_STAGE_NAME
            || self.counter_quad_assignment.operation_name != XMAP13_COUNTER_QUAD_OPERATION_NAME
            || self.unit_entity_assignment.topology_sha256
                != self.counter_quad_assignment.topology_sha256
            || self.unit_entity_assignment.assigned_ordinal
                >= self.counter_quad_assignment.assigned_ordinal
        {
            return Err("XMAP13 prepared assigned-stage closure is invalid".into());
        }
        let expected_census = ordered_identity_digest(
            self.identity_provider
                .ordered_units
                .iter()
                .map(|row| &row.identity),
        )?;
        let expected_formation_census =
            ordered_formation_identity_digest(&self.identity_provider.ordered_formations)?;
        let expected_admission = Sha256::hex(
            format!(
                "{}\0{}\0{}\0{}",
                self.semantic_input_sha256,
                expected_census,
                expected_formation_census,
                used_content_digest(&self.ordered_used_content)?
            )
            .as_bytes(),
        );
        if self.identity_provider.census_sha256 != expected_census
            || self.identity_provider.formation_census_sha256 != expected_formation_census
            || self.identity_provider.admission_sha256 != expected_admission
        {
            return Err("XMAP13 prepared census/admission digest mismatch".into());
        }
        let mut previous_content_path = None;
        for content in &self.ordered_used_content {
            content
                .validate()
                .map_err(|error| format!("XMAP13 prepared content identity: {error}"))?;
            if previous_content_path
                .replace(content.normalized_game_relative_path.as_str())
                .is_some_and(|previous| previous >= content.normalized_game_relative_path.as_str())
            {
                return Err("XMAP13 prepared content order is not canonical/unique".into());
            }
        }
        let all = self
            .identity_provider
            .ordered_units
            .iter()
            .map(|row| &row.identity)
            .collect::<BTreeSet<_>>();
        let expected_visible = self
            .identity_provider
            .ordered_units
            .iter()
            .filter(|row| {
                matches!(
                    row.disclosure,
                    ExactUnitDisclosure::MapVisible | ExactUnitDisclosure::ExactDetails
                )
            })
            .cloned()
            .collect::<Vec<_>>();
        let visible = expected_visible
            .iter()
            .map(|row| &row.identity)
            .collect::<BTreeSet<_>>();
        if self.identity_provider.hour_index != self.hour_index
            || all.len() != self.identity_provider.ordered_units.len()
            || self.visible_units != expected_visible
            || self
                .hit_targets
                .iter()
                .any(|row| !visible.contains(&row.identity))
        {
            return Err("XMAP13 prepared identity closure is invalid".into());
        }
        let mut native_ids = BTreeSet::new();
        for (order, draw) in self.native_entity_draws.iter().enumerate() {
            draw.validate()?;
            let unit = self
                .visible_units
                .iter()
                .find(|unit| unit.identity.canonical_id() == draw.stable_typed_id)
                .ok_or("XMAP13 native entity materialization has no disclosed unit")?;
            // `draw.selected_lod` is NOT compared with `unit.selected_lod_index`
            // here. The draw's LOD is the one this viewer's camera selected;
            // the unit's is the one the recorder's camera selected and is kept
            // as provenance of what the recording declared. Two cameras
            // disagreeing is the normal case and must not fail a frame.
            if draw.stable_typed_id != unit.identity.canonical_id()
                || draw.owner != L03TypedOwner::M01Unit
                || draw.partition_mask != unit.partition_mask
                || draw.world_matrix_f32_bits != unit.world_matrix_f32_bits
                || draw.object_draw.visible_order as usize != order
                || draw.object_draw.render.skinned != unit.skinned
                || draw.skin_palette_3200_bytes != unit.skin_palette_3200_bytes
                || !native_ids.insert(draw.stable_typed_id.as_str())
            {
                return Err(
                    "XMAP13 native entity materialization differs from prepared unit".into(),
                );
            }
        }
        let expected_counter_identities = self
            .visible_units
            .iter()
            .filter(|row| {
                row.counter.is_some() && native_ids.contains(row.identity.canonical_id().as_str())
            })
            .map(|row| &row.identity)
            .collect::<BTreeSet<_>>();
        let counter_identities =
            validate_prepared_counter_groups(&self.visible_units, &self.displayed_counter_groups)?;
        if counter_identities != expected_counter_identities {
            return Err("XMAP13 prepared counter census is incomplete".into());
        }
        let mut hit_identities = BTreeSet::new();
        for hit in &self.hit_targets {
            let min = hit.min_screen_f32_bits.map(f32::from_bits);
            let max = hit.max_screen_f32_bits.map(f32::from_bits);
            let depth = f32::from_bits(hit.depth_f32_bits);
            if !hit_identities.insert(&hit.identity)
                || min
                    .into_iter()
                    .chain(max)
                    .chain([depth])
                    .any(|value| !value.is_finite())
                || min[0] > max[0]
                || min[1] > max[1]
            {
                return Err("XMAP13 prepared hit target is invalid or duplicated".into());
            }
        }
        let expected_casters = self
            .native_entity_draws
            .iter()
            .filter(|draw| {
                self.visible_units.iter().any(|row| {
                    row.casts_light_depth && row.identity.canonical_id() == draw.stable_typed_id
                })
            })
            .map(crate::Xmap12PdxMeshMaterialization::l03_caster_row)
            .collect::<Vec<_>>();
        if self.light_depth_casters != expected_casters {
            return Err("XMAP13 prepared light/depth caster adapter drifted".into());
        }
        let expected = prepared_digest(self)?;
        if self.deterministic_sha256 != expected {
            return Err("XMAP13 prepared frame digest mismatch".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap13TargetExecutionIdentity {
    pub hour_index: i64,
    pub content_manifest_sha256: String,
    pub foundation_content_operation_sha256: String,
    pub viewer_state_sha256: String,
    pub main_scene_target_sha256: String,
    pub counter_target_sha256: Option<String>,
    pub map_copy_receipt_set_sha256: String,
}

impl Xmap13TargetExecutionIdentity {
    pub fn derive(
        foundation: &crate::PreparedFoundationFrame,
        viewer: &crate::ViewerOwnedMapState,
        target: &crate::ProductionMainSceneTargetDesc,
        counter_target_sha256: Option<&str>,
        map_copy_receipt_set_sha256: &str,
    ) -> Result<Self> {
        viewer.validate()?;
        target.validate()?;
        validate_sha256(map_copy_receipt_set_sha256, "XMAP13 map-copy receipt set")?;
        if let Some(counter_target_sha256) = counter_target_sha256 {
            validate_sha256(counter_target_sha256, "XMAP13 counter target")?;
        }
        let units = foundation
            .units
            .as_ref()
            .ok_or("XMAP13 target execution identity requires a Present prepared unit frame")?;
        units.validate()?;
        if units.hour_index != foundation.envelope.hour_index {
            return Err("XMAP13 target execution identity hour disagrees with foundation".into());
        }
        Ok(Self {
            hour_index: foundation.envelope.hour_index,
            content_manifest_sha256: foundation.envelope.content_manifest_sha256.clone(),
            foundation_content_operation_sha256: foundation
                .envelope
                .foundation_content_operation_sha256
                .clone(),
            viewer_state_sha256: Sha256::hex(&serde_json::to_vec(viewer)?),
            main_scene_target_sha256: Sha256::hex(&serde_json::to_vec(target)?),
            counter_target_sha256: counter_target_sha256.map(str::to_string),
            map_copy_receipt_set_sha256: map_copy_receipt_set_sha256.to_string(),
        })
    }

    pub fn validate_against(
        &self,
        foundation: &crate::PreparedFoundationFrame,
        viewer: &crate::ViewerOwnedMapState,
        target: &crate::ProductionMainSceneTargetDesc,
        counter_target_sha256: Option<&str>,
        map_copy_receipt_set_sha256: &str,
    ) -> Result<()> {
        let expected = Self::derive(
            foundation,
            viewer,
            target,
            counter_target_sha256,
            map_copy_receipt_set_sha256,
        )?;
        if self != &expected {
            return Err(
                "XMAP13 target execution identity differs from live shared identities".into(),
            );
        }
        Ok(())
    }

    fn validate(&self) -> Result<()> {
        for (label, value) in [
            ("content manifest", self.content_manifest_sha256.as_str()),
            (
                "foundation content operation",
                self.foundation_content_operation_sha256.as_str(),
            ),
            ("viewer state", self.viewer_state_sha256.as_str()),
            ("main-scene target", self.main_scene_target_sha256.as_str()),
            (
                "map-copy receipt set",
                self.map_copy_receipt_set_sha256.as_str(),
            ),
        ] {
            validate_sha256(value, &format!("XMAP13 {label}"))?;
        }
        if let Some(counter_target_sha256) = self.counter_target_sha256.as_deref() {
            validate_sha256(counter_target_sha256, "XMAP13 counter target")?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Xmap13NativeComponent {
    UnitEntities,
    CounterQuads,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "closure", rename_all = "snake_case", deny_unknown_fields)]
pub enum Xmap13NativeDrawClosure {
    Present {
        executor_target_sha256: String,
        actual_draw_count: u32,
        ordered_draw_receipt_sha256s: Vec<String>,
    },
    AuthenticatedEmpty {
        enumerator_witness_sha256: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap13NativeComponentReceipt {
    pub schema: String,
    pub component: Xmap13NativeComponent,
    pub target_execution_identity: Xmap13TargetExecutionIdentity,
    pub assigned_topology_sha256: String,
    pub assigned_stage_id: String,
    pub assigned_operation_id: String,
    pub assigned_ordinal: u32,
    pub assigned_assignment_sha256: String,
    pub prepared_frame_sha256: String,
    pub draw: Xmap13NativeDrawClosure,
    pub packet_draw_receipt_sha256: String,
}

impl Xmap13NativeComponentReceipt {
    pub fn seal(mut self) -> Result<Self> {
        self.packet_draw_receipt_sha256.clear();
        self.packet_draw_receipt_sha256 = Sha256::hex(&serde_json::to_vec(&self)?);
        self.validate()?;
        Ok(self)
    }

    pub fn validate_for(
        &self,
        identity: &Xmap13TargetExecutionIdentity,
        prepared: &PreparedXmap13Frame,
    ) -> Result<()> {
        self.validate()?;
        prepared.validate()?;
        if &self.target_execution_identity != identity
            || self.prepared_frame_sha256 != prepared.deterministic_sha256
            || self.target_execution_identity.hour_index != prepared.hour_index
        {
            return Err(
                "XMAP13 native component receipt differs from prepared target frame".into(),
            );
        }
        let assignment = match self.component {
            Xmap13NativeComponent::UnitEntities => &prepared.unit_entity_assignment,
            Xmap13NativeComponent::CounterQuads => &prepared.counter_quad_assignment,
        };
        if self.assigned_topology_sha256 != assignment.topology_sha256
            || self.assigned_stage_id != assignment.stage_name
            || self.assigned_operation_id != assignment.operation_name
            || self.assigned_ordinal != assignment.assigned_ordinal
            || self.assigned_assignment_sha256 != assignment.assignment_sha256
        {
            return Err(
                "XMAP13 native component receipt differs from authenticated assignment".into(),
            );
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP13_NATIVE_RECEIPT_SCHEMA
            || self.assigned_stage_id.trim().is_empty()
            || self.assigned_operation_id.trim().is_empty()
        {
            return Err("XMAP13 native component receipt header/assignment is invalid".into());
        }
        self.target_execution_identity.validate()?;
        for (label, value) in [
            ("assigned topology", self.assigned_topology_sha256.as_str()),
            (
                "assigned assignment",
                self.assigned_assignment_sha256.as_str(),
            ),
            ("prepared frame", self.prepared_frame_sha256.as_str()),
            (
                "packet draw receipt",
                self.packet_draw_receipt_sha256.as_str(),
            ),
        ] {
            validate_sha256(value, &format!("XMAP13 {label}"))?;
        }
        match &self.draw {
            Xmap13NativeDrawClosure::Present {
                executor_target_sha256,
                actual_draw_count,
                ordered_draw_receipt_sha256s,
            } => {
                validate_sha256(executor_target_sha256, "XMAP13 executor target")?;
                let expected_target = match self.component {
                    Xmap13NativeComponent::UnitEntities => {
                        Some(&self.target_execution_identity.main_scene_target_sha256)
                    }
                    Xmap13NativeComponent::CounterQuads => self
                        .target_execution_identity
                        .counter_target_sha256
                        .as_ref(),
                };
                if expected_target != Some(executor_target_sha256)
                    || *actual_draw_count == 0
                    || usize::try_from(*actual_draw_count)? != ordered_draw_receipt_sha256s.len()
                    || ordered_draw_receipt_sha256s
                        .iter()
                        .collect::<BTreeSet<_>>()
                        .len()
                        != ordered_draw_receipt_sha256s.len()
                {
                    return Err(
                        "XMAP13 Present native draw closure is empty, inconsistent, duplicated, or mistargeted"
                            .into(),
                    );
                }
                for receipt in ordered_draw_receipt_sha256s {
                    validate_sha256(receipt, "XMAP13 ordered native draw receipt")?;
                }
            }
            Xmap13NativeDrawClosure::AuthenticatedEmpty {
                enumerator_witness_sha256,
            } => validate_sha256(
                enumerator_witness_sha256,
                "XMAP13 authenticated-empty enumerator witness",
            )?,
        }
        let mut unsigned = self.clone();
        unsigned.packet_draw_receipt_sha256.clear();
        if self.packet_draw_receipt_sha256 != Sha256::hex(&serde_json::to_vec(&unsigned)?) {
            return Err("XMAP13 native component receipt seal mismatch".into());
        }
        Ok(())
    }
}

#[derive(Clone)]
struct Xmap13Content {
    resolver: GameFileResolver,
    admission: ValidatedContentAdmission,
    counter_assets: UnitCounterAssetManifest,
    graphics_defines: UsedContentKey,
    normalized_grouping_thresholds: [f32; 3],
}

pub struct Xmap13Adapter {
    content: Xmap13Content,
}

impl Xmap13Adapter {
    pub fn new(resolver: &GameFileResolver, admission: &ValidatedContentAdmission) -> Result<Self> {
        if !resolver.has_content_admission() {
            return Err("XMAP13 requires an admission-bound effective VFS".into());
        }
        let graphics_defines = admission
            .require_used_content(GRAPHICS_DEFINES)
            .map_err(|error| format!("XMAP13 requires admitted graphics defines: {error}"))?
            .clone();
        let source = latin1(&resolver.read_admitted_used_content(&graphics_defines)?);
        let raw_grouping_thresholds = [
            crate::parse_define_number(&source, "MAP_ICONS_GROUP_CAM_DISTANCE")?,
            crate::parse_define_number(&source, "MAP_ICONS_STATE_GROUP_CAM_DISTANCE")?,
            crate::parse_define_number(&source, "MAP_ICONS_STRATEGIC_GROUP_CAM_DISTANCE")?,
        ];
        if !raw_grouping_thresholds
            .into_iter()
            .all(|value| value.is_finite() && value > 0.0)
            || !(raw_grouping_thresholds[0] < raw_grouping_thresholds[1]
                && raw_grouping_thresholds[1] < raw_grouping_thresholds[2])
        {
            return Err("XMAP13 map-icon grouping thresholds are invalid".into());
        }
        let normalized_grouping_thresholds = raw_grouping_thresholds
            .map(|value| value / crate::xmap13_counter_grouping_law::GROUPING_NORMALIZATION_F32);
        Ok(Self {
            content: Xmap13Content {
                resolver: resolver.clone(),
                admission: admission.clone(),
                counter_assets: crate::unit_counter_assets_from_resolver(resolver)?,
                graphics_defines,
                normalized_grouping_thresholds,
            },
        })
    }
}

impl ProductionMapFeatureAdapter for Xmap13Adapter {
    fn family(&self) -> MapVisualFamily {
        MapVisualFamily::UnitsCounters
    }

    fn prepare(&self, context: &FoundationFeatureContext<'_>) -> Result<FoundationFeatureOutput> {
        // Owned-input accounting first, and identically in both states: the four
        // `units.*` rows are owner-proven map-authority inputs, so a frame that
        // carries the complete typed censuses reports them ADMITTED rather than
        // gapped. Treating "not a gap" as a hard error — as the previous
        // gap-only form did — would have turned proving them into an abort of
        // the whole foundation graph.
        let consumed_gap_ids = crate::account_owned_map_authority_inputs(
            "XMAP13",
            &OWNED_GAPS,
            context.map_authority_gaps,
            context.map_authority_admitted,
        )?;
        // The refusal names what is actually missing rather than the whole
        // domain, but it must not overstate what has been solved either: this
        // adapter still requires and consumes the recorded grouping census, so
        // the census is not something the viewer supplies today. The law for it
        // exists in `xmap13_counter_grouping_law.rs` and is waiting on two
        // identity residuals; until they close, the recorded bytes are still
        // load-bearing and their absence is still a real gap.
        const MISSING_RESIDUE: &str = "this adapter still requires the recorded counter-grouping \
                                       census, whose viewer-side law is implemented but not yet \
                                       wired; and the engine's per-unit model and pose selection \
                                       - world transform, animation phase, and the 3,200-byte \
                                       bone palette - has no recorded source at all";
        let Some(raw) = context.world_frame.domain(XMAP13_DOMAIN) else {
            return Ok(unavailable_output(
                context.world_frame.hour_index(),
                consumed_gap_ids,
                &format!(
                    "recording has no map_unit_presentation semantic census; {MISSING_RESIDUE}"
                ),
            ));
        };
        if raw.is_null() {
            return Ok(unavailable_output(
                context.world_frame.hour_index(),
                consumed_gap_ids,
                &format!(
                    "map_unit_presentation is explicitly unavailable at this hour; \
                     {MISSING_RESIDUE}"
                ),
            ));
        }
        let envelope: Xmap13SemanticEnvelope = serde_json::from_value(raw.clone())
            .map_err(|error| format!("decode XMAP13 semantic envelope: {error}"))?;
        envelope.validate_for_hour(context.world_frame.hour_index())?;
        let political = context
            .prepared_political
            .ok_or("XMAP13 requires the earlier XMAP05 political/visibility payload")?;
        if context.world_frame.division_units_status() != ViewStatus::Present
            || context.world_frame.fleets_status() != ViewStatus::Present
            || context.world_frame.wings_status() != ViewStatus::Present
        {
            return Ok(unavailable_output(
                context.world_frame.hour_index(),
                consumed_gap_ids,
                "typed unit collections are not all complete/Present",
            ));
        }
        if context.viewer.observer_country_id.is_some() && !context.world_frame.wings().is_empty() {
            return Ok(unavailable_output(
                context.world_frame.hour_index(),
                consumed_gap_ids,
                "XMAP05 explicitly has no authoritative air-wing spotting producer for country POV",
            ));
        }
        if let Some(reason) = counter_allegiance_unavailable(context, &envelope, political)? {
            return Ok(unavailable_output(
                context.world_frame.hour_index(),
                consumed_gap_ids,
                &reason,
            ));
        }
        let Some(grouping_tier) = self.content.exact_grouping_tier(context)? else {
            return Ok(unavailable_output(
                context.world_frame.hour_index(),
                consumed_gap_ids,
                "viewer-owned XMAP03 camera state is unavailable",
            ));
        };
        let unit_entity_assignment = context
            .require_assigned_stage(
                XMAP13_UNIT_ENTITY_STAGE_NAME,
                XMAP13_UNIT_ENTITY_OPERATION_NAME,
            )?
            .clone();
        let counter_quad_assignment = context
            .require_assigned_stage(
                XMAP13_COUNTER_QUAD_STAGE_NAME,
                XMAP13_COUNTER_QUAD_OPERATION_NAME,
            )?
            .clone();
        let prepared = self.content.prepare_exact(
            context,
            &envelope,
            political,
            grouping_tier,
            unit_entity_assignment,
            counter_quad_assignment,
        )?;
        let resources_sha256 = used_content_digest(&prepared.ordered_used_content)?;
        let target_sha256 = Sha256::hex(
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
        let registration = Sha256::hex(
            format!(
                "{XMAP13_PREPARED_SCHEMA}\0{XMAP13_ENGINE_SHA256}\0{}",
                envelope.producer_evidence_manifest_sha256
            )
            .as_bytes(),
        );
        Ok(FoundationFeatureOutput {
            disposition: FamilyDisposition::Present {
                payload_schema: XMAP13_PREPARED_SCHEMA.into(),
                producer_registration_sha256: registration.clone(),
                payload_sha256: prepared.deterministic_sha256.clone(),
                ordered_operations: vec![
                    prepared
                        .unit_entity_assignment
                        .foundation_operation_identity(
                            registration.clone(),
                            resources_sha256.clone(),
                            target_sha256.clone(),
                        )?,
                    prepared
                        .counter_quad_assignment
                        .foundation_operation_identity(
                            registration,
                            resources_sha256,
                            target_sha256,
                        )?,
                ],
                ordered_used_content: prepared.ordered_used_content.clone(),
            },
            prepared_terrain: None,
            prepared_political: None,
            prepared_thin_borders: None,
            prepared_rivers_straits: None,
            prepared_water: None,
            prepared_lighting: None,
            prepared_weather: None,
            prepared_effects: None,
            prepared_units: Some(prepared),
            prepared_combat: None,
            prepared_orders: None,
            prepared_rail_supply: None,
            prepared_map_modes: None,
            prepared_map_furniture: None,
            consumed_gap_ids,
        })
    }
}

pub fn prepare_xmap13_effect_identity(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
    world_frame: &replay_core::WorldFrame,
) -> Result<Xmap13EffectIdentityProvider> {
    let raw = world_frame
        .domain(XMAP13_DOMAIN)
        .ok_or("recording has no complete map_unit_presentation semantic census")?;
    if raw.is_null() {
        return Err("map_unit_presentation is explicitly unavailable at this hour".into());
    }
    let envelope: Xmap13SemanticEnvelope = serde_json::from_value(raw.clone())
        .map_err(|error| format!("decode XMAP13 semantic envelope: {error}"))?;
    envelope.validate_for_hour(world_frame.hour_index())?;
    let adapter = Xmap13Adapter::new(resolver, admission)?;
    adapter
        .content
        .prepare_effect_identity(world_frame, &envelope)
}

impl Xmap13Content {
    fn exact_grouping_tier(&self, context: &FoundationFeatureContext<'_>) -> Result<Option<u8>> {
        let Some(camera) = context.viewer.camera.as_ref() else {
            return Ok(None);
        };
        Ok(Some(
            crate::xmap13_counter_grouping_law::grouping_tier(
                camera.current_eye()[1],
                self.normalized_grouping_thresholds,
            )
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?,
        ))
    }

    fn prepare_effect_identity(
        &self,
        world_frame: &replay_core::WorldFrame,
        envelope: &Xmap13SemanticEnvelope,
    ) -> Result<Xmap13EffectIdentityProvider> {
        if world_frame.division_units_status() != ViewStatus::Present
            || world_frame.fleets_status() != ViewStatus::Present
            || world_frame.wings_status() != ViewStatus::Present
        {
            return Err("XMAP13 effect identity requires complete typed unit collections".into());
        }
        let mut seen_identity = BTreeSet::new();
        let mut seen_source = BTreeSet::new();
        let mut seen_entity_instance = BTreeSet::new();
        let mut seen_ship_instance = BTreeSet::new();
        let mut used_content = BTreeMap::<String, UsedContentKey>::new();
        for row in &envelope.rows {
            row.identity.validate()?;
            if row.semantic_source_id.trim().is_empty()
                || row.entity_instance_id == 0
                || row.owner_country.trim().is_empty()
                || row.anchor_province_id == 0
                || row.entity_name.trim().is_empty()
                || row.model_name.trim().is_empty()
                || row.state_name.trim().is_empty()
                || row.material_variant.trim().is_empty()
                || !seen_identity.insert(row.identity.clone())
                || !seen_source.insert(row.semantic_source_id.clone())
                || !seen_entity_instance.insert(row.entity_instance_id)
            {
                return Err("XMAP13 effect semantic identity is incomplete or duplicated".into());
            }
            validate_world_matrix(row.world_matrix_f32_bits)?;
            validate_animation(row)?;
            validate_events(row.authored_event_block_count, &row.authored_state_events)?;
            validate_counter(row.counter.as_ref())?;
            validate_counter_kind(row)?;
            validate_task_force_members(row, &mut seen_ship_instance)?;
            validate_typed_join(world_frame, row)?;
            for content in row_content(row) {
                self.admit_content(content)?;
                match used_content.get(&content.normalized_game_relative_path) {
                    Some(previous) if previous != content => {
                        return Err(
                            "XMAP13 effect content path has conflicting admitted identity".into(),
                        )
                    }
                    Some(_) => {}
                    None => {
                        used_content.insert(
                            content.normalized_game_relative_path.clone(),
                            content.clone(),
                        );
                    }
                }
            }
            self.validate_counter_content(row.counter.as_ref())?;
            self.validate_task_force_member_content(row)?;
            self.validate_entity_model_binding(row)?;
            self.validate_animation_asset(row)?;
            self.validate_mesh_selection(row)?;
        }
        let division_rows = envelope
            .rows
            .iter()
            .filter(|row| matches!(row.identity, Xmap13UnitIdentity::Division { .. }))
            .count();
        let task_force_rows = envelope
            .rows
            .iter()
            .filter(|row| matches!(row.identity, Xmap13UnitIdentity::TaskForce { .. }))
            .count();
        let air_wing_rows = envelope
            .rows
            .iter()
            .filter(|row| matches!(row.identity, Xmap13UnitIdentity::AirWing { .. }))
            .count();
        if division_rows != world_frame.division_units().len()
            || task_force_rows != world_frame.fleets().len()
            || air_wing_rows != world_frame.wings().len()
        {
            return Err("XMAP13 effect identities do not exactly cover typed unit censuses".into());
        }
        validate_formation_census(world_frame, &envelope.ordered_formations)?;
        let ordered_semantic_source_ids = envelope
            .rows
            .iter()
            .map(|row| row.semantic_source_id.clone())
            .collect::<Vec<_>>();
        let receipt = Xmap13EffectIdentityReceipt {
            schema: XMAP13_EFFECT_IDENTITY_RECEIPT_SCHEMA.into(),
            hour_index: envelope.hour_index,
            semantic_envelope_sha256: envelope.deterministic_sha256.clone(),
            producer_evidence_manifest_sha256: envelope.producer_evidence_manifest_sha256.clone(),
            ordered_identity_sha256: envelope.ordered_identity_sha256.clone(),
            ordered_semantic_source_ids_sha256: semantic_source_id_digest(
                &ordered_semantic_source_ids,
            ),
            ordered_semantic_source_ids,
            admitted_content_sha256: used_content_digest(
                &used_content.into_values().collect::<Vec<_>>(),
            )?,
            deterministic_sha256: String::new(),
        }
        .seal()?;
        Ok(Xmap13EffectIdentityProvider {
            receipt,
            ordered_rows: envelope.rows.clone(),
        })
    }

    fn prepare_exact(
        &self,
        context: &FoundationFeatureContext<'_>,
        envelope: &Xmap13SemanticEnvelope,
        political: &crate::PreparedPoliticalFrame,
        grouping_tier: u8,
        unit_entity_assignment: AssignedStage,
        counter_quad_assignment: AssignedStage,
    ) -> Result<PreparedXmap13Frame> {
        if context.world_frame.division_units_status() != ViewStatus::Present
            || context.world_frame.fleets_status() != ViewStatus::Present
            || context.world_frame.wings_status() != ViewStatus::Present
        {
            return Err("typed unit collections are not all complete/Present".into());
        }
        let camera = context
            .viewer
            .camera
            .as_ref()
            .ok_or("XMAP13 requires viewer-owned XMAP03 camera state")?;
        let production_camera = camera.production_camera()?;
        let mut used_content = BTreeMap::<String, UsedContentKey>::new();
        used_content.insert(
            self.graphics_defines.normalized_game_relative_path.clone(),
            self.graphics_defines.clone(),
        );
        let mut units = Vec::with_capacity(envelope.rows.len());
        // Per unit, what the admitted mesh proves about its LODs. The LOD count
        // bounds the LOD the viewer's camera selects during materialization, and
        // it is the only thing that legitimately bounds it: the producer's own
        // declared LOD came from the RECORDER's camera and says nothing about
        // ours. The per-LOD bounds are what the hit target is rebuilt from once
        // the viewer's camera has chosen.
        let mut mesh_selections = BTreeMap::<String, Xmap13MeshSelection>::new();
        let mut seen_identity = BTreeSet::new();
        let mut seen_source = BTreeSet::new();
        let mut seen_entity_instance = BTreeSet::new();
        let mut seen_ship_instance = BTreeSet::new();
        for row in &envelope.rows {
            row.identity.validate()?;
            if row.semantic_source_id.trim().is_empty()
                || row.entity_instance_id == 0
                || row.owner_country.trim().is_empty()
                || row.anchor_province_id == 0
                || row.entity_name.trim().is_empty()
                || row.model_name.trim().is_empty()
                || row.state_name.trim().is_empty()
                || row.material_variant.trim().is_empty()
                || row.selected_shape_indices.is_empty()
                || row
                    .selected_shape_indices
                    .iter()
                    .collect::<BTreeSet<_>>()
                    .len()
                    != row.selected_shape_indices.len()
                || row.groups.is_empty()
                || row.selected_shape_indices.len() != row.groups.len()
                || !seen_identity.insert(row.identity.clone())
                || !seen_source.insert(row.semantic_source_id.clone())
                || !seen_entity_instance.insert(row.entity_instance_id)
            {
                return Err(
                    "XMAP13 semantic row identity/content is incomplete or duplicated".into(),
                );
            }
            validate_world_matrix(row.world_matrix_f32_bits)?;
            validate_animation(row)?;
            validate_events(row.authored_event_block_count, &row.authored_state_events)?;
            validate_counter(row.counter.as_ref())?;
            validate_counter_kind(row)?;
            validate_task_force_members(row, &mut seen_ship_instance)?;
            let content_rows = row_content(row);
            for content in content_rows {
                self.admit_content(content)?;
                match used_content.get(&content.normalized_game_relative_path) {
                    Some(previous) if previous != content => {
                        return Err("XMAP13 content path has conflicting admitted identity".into())
                    }
                    Some(_) => {}
                    None => {
                        used_content.insert(
                            content.normalized_game_relative_path.clone(),
                            content.clone(),
                        );
                    }
                }
            }
            self.validate_counter_content(row.counter.as_ref())?;
            self.validate_task_force_member_content(row)?;
            self.validate_entity_model_binding(row)?;
            self.validate_animation_asset(row)?;
            let mesh_selection = self.validate_mesh_selection(row)?;
            let (aabb_min, aabb_max) = (mesh_selection.aabb_min, mesh_selection.aabb_max);
            validate_typed_join(context.world_frame, row)?;
            let (disclosure, visibility_reason) = disclosure(political, context, row)?;
            let canonical = row.identity.canonical_id();
            mesh_selections.insert(canonical.clone(), mesh_selection);
            let selected = context
                .viewer
                .selected_object_ids
                .iter()
                .any(|value| value == &canonical);
            units.push(PreparedXmap13Unit {
                identity: row.identity.clone(),
                semantic_source_id: row.semantic_source_id.clone(),
                entity_instance_id: row.entity_instance_id,
                owner_country: row.owner_country.clone(),
                anchor_province_id: row.anchor_province_id,
                entity_asset: row.entity_asset.clone(),
                model_registry: row.model_registry.clone(),
                mesh: row.mesh.clone(),
                shader_source: row.shader_source.clone(),
                animation_asset: row.animation_asset.clone(),
                ordered_texture_content: row.ordered_texture_content.clone(),
                entity_name: row.entity_name.clone(),
                model_name: row.model_name.clone(),
                state_name: row.state_name.clone(),
                material_variant: row.material_variant.clone(),
                animation_name: row.animation_name.clone(),
                animation_phase_f32_bits: row.animation_phase_f32_bits,
                animation_speed_f32_bits: row.animation_speed_f32_bits,
                authored_event_block_count: row.authored_event_block_count,
                authored_state_events: row.authored_state_events.clone(),
                task_force_members: row.task_force_members.clone(),
                counter: row.counter.clone(),
                world_matrix_f32_bits: row.world_matrix_f32_bits,
                disclosure,
                visibility_reason,
                selected,
                selected_lod_index: row.selected_lod_index,
                selected_shape_indices: row.selected_shape_indices.clone(),
                selected_mesh: row.mesh.normalized_game_relative_path.clone(),
                groups: row.groups.clone(),
                skinned: row.skinned,
                skin_palette_sha256: row.skin_palette_sha256.clone(),
                skin_palette_3200_bytes: row.skin_palette_3200_bytes.clone(),
                partition_mask: 0,
                casts_light_depth: row.casts_light_depth,
                local_aabb_min_f32_bits: aabb_min.map(f32::to_bits),
                local_aabb_max_f32_bits: aabb_max.map(f32::to_bits),
            });
        }
        validate_census(context.world_frame, &units)?;
        validate_formation_census(context.world_frame, &envelope.ordered_formations)?;
        let census_sha256 = ordered_identity_digest(units.iter().map(|row| &row.identity))?;
        let formation_census_sha256 =
            ordered_formation_identity_digest(&envelope.ordered_formations)?;
        let admission_sha256 = Sha256::hex(
            format!(
                "{}\0{}\0{}\0{}",
                envelope.deterministic_sha256,
                census_sha256,
                formation_census_sha256,
                used_content_digest(&used_content.values().cloned().collect::<Vec<_>>())?
            )
            .as_bytes(),
        );
        // The frustum partition is draw-derived, so it is written back onto the
        // ONE unit census before anything is cloned out of it. Writing it only
        // onto a clone left `PreparedXmap13Frame::validate`'s
        // visible-units-versus-provider comparison failing on any frame that
        // actually materialized a unit, because the provider had been cloned
        // while every mask was still zero.
        let visible_indices = units
            .iter()
            .enumerate()
            .filter(|(_, row)| {
                matches!(
                    row.disclosure,
                    ExactUnitDisclosure::MapVisible | ExactUnitDisclosure::ExactDetails
                )
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let mut native_entity_draws = Vec::new();
        for index in visible_indices.iter().copied() {
            let order = u32::try_from(native_entity_draws.len())?;
            let selection = mesh_selections
                .get(&units[index].identity.canonical_id())
                .ok_or("XMAP13 unit has no admitted mesh LOD table")?;
            if let Some(draw) =
                self.materialize_native_entity(camera, order, &units[index], selection.lod_count)?
            {
                units[index].partition_mask = draw.partition_mask;
                // The hit target is projected from these bounds, so they follow
                // the LOD the viewer's camera actually drew rather than the one
                // the recorder declared.
                if let Some(Some((min, max))) =
                    selection.lod_bounds.get(draw.selected_lod as usize)
                {
                    units[index].local_aabb_min_f32_bits = min.map(f32::to_bits);
                    units[index].local_aabb_max_f32_bits = max.map(f32::to_bits);
                }
                native_entity_draws.push(draw);
            }
        }
        let provider = Xmap13IdentityProvider {
            hour_index: envelope.hour_index,
            ordered_units: units.clone(),
            ordered_formations: envelope.ordered_formations.clone(),
            census_sha256,
            formation_census_sha256,
            admission_sha256,
        };
        let visible_units = visible_indices
            .iter()
            .map(|index| units[*index].clone())
            .collect::<Vec<_>>();
        let native_ids = native_entity_draws
            .iter()
            .map(|draw| draw.stable_typed_id.as_str())
            .collect::<BTreeSet<_>>();
        let rendered_units = visible_units
            .iter()
            .filter(|unit| native_ids.contains(unit.identity.canonical_id().as_str()))
            .cloned()
            .collect::<Vec<_>>();
        let displayed_counter_groups =
            build_counter_groups(context, &rendered_units, grouping_tier)?;
        let hit_targets = rendered_units
            .iter()
            .filter_map(|row| {
                project_hit_target(
                    &production_camera,
                    context.viewer.viewport_width,
                    context.viewer.viewport_height,
                    row,
                )
            })
            .collect::<Vec<_>>();
        let light_depth_casters = native_entity_draws
            .iter()
            .filter(|draw| {
                rendered_units.iter().any(|row| {
                    row.casts_light_depth && row.identity.canonical_id() == draw.stable_typed_id
                })
            })
            .map(crate::Xmap12PdxMeshMaterialization::l03_caster_row)
            .collect::<Vec<_>>();
        let mut prepared = PreparedXmap13Frame {
            schema: XMAP13_PREPARED_SCHEMA.into(),
            hour_index: envelope.hour_index,
            ordered_used_content: used_content.into_values().collect(),
            unit_entity_assignment,
            counter_quad_assignment,
            identity_provider: provider,
            visible_units,
            native_entity_draws,
            displayed_counter_groups,
            hit_targets,
            light_depth_casters,
            semantic_input_sha256: envelope.deterministic_sha256.clone(),
            deterministic_sha256: String::new(),
        };
        prepared.deterministic_sha256 = prepared_digest(&prepared)?;
        prepared.validate()?;
        Ok(prepared)
    }

    /// Materialize one unit's entity through the shared PdxMesh backend.
    ///
    /// THE LOD IS THE VIEWER'S, NOT THE RECORDER'S.  `selected_lod_override` is
    /// deliberately `None` so `xmap12_pdxmesh.rs::select_pdxmesh_lod` runs
    /// against this viewer's camera. Passing the row's `selected_lod_index`
    /// here -- as this did -- meant the camera path never executed and the
    /// check below compared the override with itself. Under the replay-layer
    /// precedent a per-frame, camera-distance LOD is client presentation: the
    /// recorded index is the RECORDER's camera answering a question only the
    /// viewer's camera may answer, so it cannot be imposed and it cannot be
    /// compared for equality either -- two different cameras disagreeing is the
    /// normal case, not a corruption. `selected_shape_indices` follows for the
    /// same reason: the shape set is exactly the mesh's shapes at the selected
    /// LOD, so pinning it to the recorder's LOD would empty the draw whenever
    /// the two cameras disagreed.
    ///
    /// What IS checked is the pair of facts that do not depend on a camera: the
    /// world transform must survive materialization unchanged, and the derived
    /// LOD must be one the admitted mesh actually declares.
    fn materialize_native_entity(
        &self,
        camera: &crate::ViewerMapCameraState,
        visible_order: u32,
        unit: &PreparedXmap13Unit,
        mesh_lod_count: usize,
    ) -> Result<Option<crate::Xmap12PdxMeshMaterialization>> {
        let entity_source = latin1(
            &self
                .resolver
                .read_admitted_used_content(&unit.entity_asset)?,
        );
        let entity = parse_building_entity_bindings(
            &crate::strip_comments(&entity_source),
            &unit.entity_asset.normalized_game_relative_path,
        )?
        .into_iter()
        .find(|row| row.name == unit.entity_name)
        .ok_or("XMAP13 native entity binding disappeared")?;
        let registry_source = latin1(
            &self
                .resolver
                .read_admitted_used_content(&unit.model_registry)?,
        );
        let pdxmesh = parse_building_pdxmesh_bindings(
            &registry_source,
            &unit.model_registry.normalized_game_relative_path,
        )?
        .into_iter()
        .find(|row| row.name == unit.model_name)
        .ok_or("XMAP13 native PdxMesh binding disappeared")?;
        let mesh_settings = pdxmesh
            .mesh_settings
            .iter()
            .filter_map(|row| {
                row.index
                    .map(|shape_index| crate::Xmap12MeshSettingOverride {
                        shape_index,
                        texture_diffuse: row.texture_diffuse.clone(),
                        texture_normal: row.texture_normal.clone(),
                        texture_specular: row.texture_specular.clone(),
                        shader: row.shader.clone(),
                    })
            })
            .collect();
        let ordered_texture_sources = unit
            .ordered_texture_content
            .iter()
            .flat_map(|content| {
                ["diffuse", "normal", "specular"].map(|slot| crate::Xmap12TextureSource {
                    slot: slot.into(),
                    content: content.clone(),
                })
            })
            .collect();
        let request = crate::Xmap12PdxMeshRequest {
            visible_order,
            stable_typed_id: unit.identity.canonical_id(),
            owner: L03TypedOwner::M01Unit,
            entity_token: entity.name,
            pdxmesh_token: pdxmesh.name,
            entity_source: unit.entity_asset.clone(),
            pdxmesh_declaration_source: unit.model_registry.clone(),
            mesh_source: unit.mesh.clone(),
            shader_source: unit.shader_source.clone(),
            ordered_texture_sources,
            mesh_settings,
            world_matrix_f32_bits: unit.world_matrix_f32_bits,
            entity_scale_f32_bits: 1.0f32.to_bits(),
            pdxmesh_scale_f32_bits: 1.0f32.to_bits(),
            material_family: crate::Xmap12PdxMeshMaterialFamily::Standard,
            selected_lod_override: None,
            selected_shape_indices: None,
            skin_palette_3200_bytes: unit.skin_palette_3200_bytes.clone(),
        };
        let Some(draw) =
            crate::materialize_xmap12_pdxmesh(&self.resolver, &self.admission, camera, &request)?
        else {
            return Ok(None);
        };
        if draw.world_matrix_f32_bits != unit.world_matrix_f32_bits {
            return Err("XMAP13 native PdxMesh world transform differs from semantic producer".into());
        }
        // The bound is `>`, not `>=`, and deliberately matches what
        // `Xmap13Content::validate_mesh_selection` already admits of the
        // producer's own declared index. `xmap12_pdxmesh.rs::select_pdxmesh_lod`
        // initialises its result to exactly `lod_distances.len()` and lowers it
        // only when some LOD distance is within range, so the terminal value IS
        // a legitimate selector result and must not be rejected here.
        if draw.selected_lod as usize > mesh_lod_count {
            return Err(
                "XMAP13 viewer-camera LOD selection is outside the admitted mesh LOD table".into(),
            );
        }
        Ok(Some(draw))
    }

    fn admit_content(&self, content: &UsedContentKey) -> Result<()> {
        content
            .validate()
            .map_err(|error| format!("XMAP13 content identity: {error}"))?;
        let admitted = self
            .admission
            .require_used_content(&content.normalized_game_relative_path)
            .map_err(|error| format!("XMAP13 content is not admitted: {error}"))?;
        if admitted != content {
            return Err("XMAP13 content identity differs from admission".into());
        }
        self.resolver.read_admitted_used_content(content)?;
        Ok(())
    }

    fn validate_entity_model_binding(&self, row: &Xmap13SemanticRow) -> Result<()> {
        let entity_text = latin1(
            &self
                .resolver
                .read_admitted_used_content(&row.entity_asset)?,
        );
        let entity_source = crate::strip_comments(&entity_text);
        let entities = parse_building_entity_bindings(
            &entity_source,
            &row.entity_asset.normalized_game_relative_path,
        )?;
        let entity = entities
            .iter()
            .find(|entry| entry.name == row.entity_name)
            .ok_or("XMAP13 selected entity is absent from admitted entity asset")?;
        if entity.pdxmesh != row.model_name {
            return Err("XMAP13 selected entity/model binding disagrees with asset".into());
        }
        let registry_text = latin1(
            &self
                .resolver
                .read_admitted_used_content(&row.model_registry)?,
        );
        let models = parse_building_pdxmesh_bindings(
            &registry_text,
            &row.model_registry.normalized_game_relative_path,
        )?;
        let model = models
            .iter()
            .find(|entry| entry.name == row.model_name)
            .ok_or("XMAP13 selected model is absent from admitted pdxmesh registry")?;
        if normalize(&model.mesh) != normalize(&row.mesh.normalized_game_relative_path) {
            return Err("XMAP13 selected model/mesh binding disagrees with registry".into());
        }
        let state = selected_entity_state_block(&entity_source, &row.entity_name, &row.state_name)
            .ok_or("XMAP13 selected entity state is absent from selected admitted entity")?;
        let authored_event_blocks = ordered_assignment_blocks(&state, "event");
        if authored_event_blocks.len() != row.authored_event_block_count as usize {
            return Err(
                "XMAP13 authored state-event census differs from selected entity state".into(),
            );
        }
        let expected_effect_count = authored_event_blocks
            .iter()
            .map(|block| authored_effect_invocations(block).len())
            .sum::<usize>();
        if expected_effect_count != row.authored_state_events.len() {
            return Err("XMAP13 authored effect census differs from selected entity state".into());
        }
        for event in &row.authored_state_events {
            let block = authored_event_blocks
                .get(event.authored_event_index as usize)
                .ok_or("XMAP13 authored state-event index is outside selected entity state")?;
            let invocation = authored_effect_invocations(block)
                .into_iter()
                .nth(event.authored_effect_index as usize)
                .ok_or("XMAP13 authored effect index is outside selected event block")?;
            if invocation.0 != event.effect_kind || invocation.1 != event.effect_token {
                return Err(
                    "XMAP13 authored effect kind/token differs from selected entity state".into(),
                );
            }
            match (event.effect_kind, event.node.as_deref()) {
                (Xmap13AuthoredEffectKind::Particle, Some(node))
                    if !node.trim().is_empty() && block.contains(node) => {}
                (Xmap13AuthoredEffectKind::Sound, None) => {}
                (Xmap13AuthoredEffectKind::Sound, Some(node))
                    if !node.trim().is_empty() && block.contains(node) => {}
                _ => {
                    return Err(
                        "XMAP13 authored effect lifecycle node differs from selected event".into(),
                    )
                }
            }
            validate_authored_event_presence(block, event)?;
        }
        Ok(())
    }

    fn validate_animation_asset(&self, row: &Xmap13SemanticRow) -> Result<()> {
        let Some(animation) = &row.animation_asset else {
            return Ok(());
        };
        let bytes = self.resolver.read_admitted_used_content(animation)?;
        let tree = crate::pdx_mesh::parse_pdx_asset(&bytes)?;
        crate::pdx_mesh::extract_pdx_anim(&tree)?;
        Ok(())
    }

    fn validate_task_force_member_content(&self, row: &Xmap13SemanticRow) -> Result<()> {
        let Some(members) = &row.task_force_members else {
            return Ok(());
        };
        for member in members {
            let definition_text = latin1(
                &self
                    .resolver
                    .read_admitted_used_content(&member.definition_content)?,
            );
            if crate::extract_block_after(&definition_text, &member.definition_id, 0).is_none() {
                return Err(
                    "XMAP13 task-force ship definition is absent from admitted content".into(),
                );
            }
            let hull_text = latin1(
                &self
                    .resolver
                    .read_admitted_used_content(&member.hull_content)?,
            );
            if crate::extract_block_after(&hull_text, &member.hull_id, 0).is_none() {
                return Err("XMAP13 task-force hull is absent from admitted content".into());
            }
        }
        Ok(())
    }

    fn validate_counter_content(&self, counter: Option<&Xmap13CounterSemantic>) -> Result<()> {
        let Some(counter) = counter else {
            return Ok(());
        };
        let manifest = &self.counter_assets;
        for presentation in &counter.ordered_presentations {
            let item_kind = counter
                .ordered_group_memberships
                .iter()
                .find(|membership| {
                    membership.grouping_tier == presentation.grouping_tier
                        && membership.allegiance == presentation.allegiance
                        && membership.counter_item_id == presentation.counter_item_id
                })
                .map(|membership| membership.counter_item_kind)
                .ok_or("XMAP13 presentation has no exact counter-item membership")?;
            let layout = xmap13_counter_layout(manifest, item_kind);
            xmap13_require_counter_content(self, counter, &layout.source_file)?;

            let drawable_controls = layout
                .controls
                .iter()
                .enumerate()
                .filter(|(_, control)| {
                    control.sprite_type.is_some()
                        || control.quad_texture_sprite.is_some()
                        || control.font.is_some()
                })
                .collect::<Vec<_>>();
            let mut control_ordinals = BTreeMap::new();
            for (ordinal, control) in &drawable_controls {
                if !control.name.trim().is_empty()
                    && control_ordinals
                        .insert(control.name.as_str(), *ordinal)
                        .is_some()
                {
                    return Err(
                        "XMAP13 effective GUI has a duplicate named drawable control".into(),
                    );
                }
            }
            let active_ability_grid = layout
                .controls
                .iter()
                .enumerate()
                .filter(|(_, control)| control.name == "active_abilities")
                .collect::<Vec<_>>();
            let active_ability_template_layout = match item_kind {
                Xmap13CounterItemKind::Land => Some(
                    manifest
                        .gui_layouts
                        .active_ability_item
                        .as_ref()
                        .ok_or("XMAP13 land counter lacks the active-ability item template")?,
                ),
                _ => None,
            };
            let active_ability_template = active_ability_template_layout
                .map(xmap13_active_ability_template_control)
                .transpose()?;
            let active_ability_grid = match item_kind {
                Xmap13CounterItemKind::Land => Some(
                    *active_ability_grid
                        .as_slice()
                        .first()
                        .filter(|_| active_ability_grid.len() == 1)
                        .ok_or("XMAP13 land GUI lacks one exact active-abilities grid")?,
                ),
                _ if presentation.active_ability_instance_count == 0 => None,
                _ => return Err("XMAP13 non-land counter carries active-ability instances".into()),
            };
            let mut covered_controls = BTreeSet::new();
            let mut covered_active_abilities = BTreeSet::new();
            let mut prior_painter_key = None;
            for primitive in &presentation.ordered_primitives {
                let control_name = match primitive {
                    Xmap13CounterPrimitive::Sprite {
                        authored_control_name,
                        ..
                    }
                    | Xmap13CounterPrimitive::Text {
                        authored_control_name,
                        ..
                    }
                    | Xmap13CounterPrimitive::Hidden {
                        authored_control_name,
                        ..
                    } => authored_control_name,
                };
                let runtime_instance = match primitive {
                    Xmap13CounterPrimitive::Sprite {
                        runtime_collection_instance_ordinal,
                        ..
                    } => *runtime_collection_instance_ordinal,
                    _ => None,
                };
                let (control, painter_key, active_context) = if let Some(instance_ordinal) =
                    runtime_instance
                {
                    let (grid_ordinal, grid) = active_ability_grid
                        .ok_or("XMAP13 active-ability instance has no admitted grid")?;
                    if control_name != "active_ability"
                        || instance_ordinal >= presentation.active_ability_instance_count
                        || !covered_active_abilities.insert(instance_ordinal)
                    {
                        return Err(
                            "XMAP13 active-ability runtime instance is invalid or duplicated"
                                .into(),
                        );
                    }
                    (
                        active_ability_template
                            .ok_or("XMAP13 active-ability instance has no admitted template")?,
                        (grid_ordinal, instance_ordinal),
                        Some((
                            grid,
                            active_ability_template_layout.ok_or(
                                "XMAP13 active-ability instance has no admitted template layout",
                            )?,
                            instance_ordinal,
                        )),
                    )
                } else {
                    let ordinal = xmap13_resolve_control_ordinal(
                        &drawable_controls,
                        &control_ordinals,
                        &covered_controls,
                        control_name,
                    )?;
                    if !covered_controls.insert(ordinal) {
                        return Err("XMAP13 effective GUI control has duplicate primitives".into());
                    }
                    (&layout.controls[ordinal], (ordinal, 0), None)
                };
                if prior_painter_key.is_some_and(|prior| painter_key <= prior) {
                    return Err(
                        "XMAP13 primitive painter order differs from effective GUI order".into(),
                    );
                }
                prior_painter_key = Some(painter_key);
                xmap13_validate_counter_primitive(
                    self,
                    counter,
                    manifest,
                    item_kind,
                    control,
                    primitive,
                    active_context,
                )?;
            }
            if covered_controls.len() != drawable_controls.len() {
                return Err(
                    "XMAP13 presentation does not census every effective GUI drawable control"
                        .into(),
                );
            }
            if covered_active_abilities != (0..presentation.active_ability_instance_count).collect()
            {
                return Err(
                    "XMAP13 presentation does not census every active-ability instance".into(),
                );
            }
        }
        Ok(())
    }

    fn validate_mesh_selection(&self, row: &Xmap13SemanticRow) -> Result<Xmap13MeshSelection> {
        let bytes = self.resolver.read_admitted_used_content(&row.mesh)?;
        let tree = crate::pdx_mesh::parse_pdx_asset(&bytes)?;
        let model = crate::pdx_mesh::extract_pdx_model(&tree)?;
        let registry_text = latin1(
            &self
                .resolver
                .read_admitted_used_content(&row.model_registry)?,
        );
        let bindings = parse_building_pdxmesh_bindings(
            &registry_text,
            &row.model_registry.normalized_game_relative_path,
        )?;
        let binding = bindings
            .iter()
            .find(|entry| entry.name == row.model_name)
            .ok_or("XMAP13 selected model is absent from admitted pdxmesh registry")?;
        if row.selected_lod_index as usize > model.lod_distances.len() {
            return Err("XMAP13 selected LOD index exceeds admitted mesh LOD table".into());
        }
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        let mut saw_geometry = false;
        let mut expected_textures = Vec::new();
        let mut seen_expected_textures = BTreeSet::new();
        for (group, shape_index) in row.groups.iter().zip(&row.selected_shape_indices) {
            let shape = model
                .shapes
                .get(*shape_index as usize)
                .ok_or("XMAP13 selected shape index is outside admitted mesh")?;
            if let Some(lod) = shape.lod {
                if lod < 0 || lod as u32 != row.selected_lod_index {
                    return Err(
                        "XMAP13 selected shape LOD disagrees with semantic selection".into(),
                    );
                }
            }
            if group.submeshes.len() != shape.meshes.len() {
                return Err(
                    "XMAP13 caster group/submesh census disagrees with admitted mesh".into(),
                );
            }
            let overrides = binding
                .mesh_settings
                .iter()
                .filter(|setting| setting.index.is_none() || setting.index == Some(*shape_index))
                .collect::<Vec<_>>();
            for (submesh, mesh) in group.submeshes.iter().zip(&shape.meshes) {
                validate_draw(mesh.indices.len(), mesh.verts.len(), &submesh.draw)?;
                if submesh.selected_submesh_material.trim().is_empty() {
                    return Err("XMAP13 selected submesh material is blank".into());
                }
                if mesh.material.is_collision() {
                    continue;
                }
                if overrides.is_empty() {
                    for texture in [
                        mesh.material.diffuse.as_deref(),
                        mesh.material.normal_map.as_deref(),
                        mesh.material.specular.as_deref(),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        let texture = texture_basename(texture);
                        if seen_expected_textures.insert(texture.clone()) {
                            expected_textures.push(texture);
                        }
                    }
                }
                for setting in &overrides {
                    for texture in [
                        setting.texture_diffuse.as_deref(),
                        setting.texture_normal.as_deref(),
                        setting.texture_specular.as_deref(),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        let texture = texture_basename(texture);
                        if seen_expected_textures.insert(texture.clone()) {
                            expected_textures.push(texture);
                        }
                    }
                    if let Some(shader) = &setting.shader {
                        if shader != &submesh.selected_submesh_material {
                            return Err(
                                "XMAP13 selected material disagrees with admitted meshsettings shader"
                                    .into(),
                            );
                        }
                    }
                }
                saw_geometry = true;
                for axis in 0..3 {
                    min[axis] = min[axis].min(mesh.aabb_min[axis]);
                    max[axis] = max[axis].max(mesh.aabb_max[axis]);
                }
            }
        }
        if !saw_geometry || min.into_iter().chain(max).any(|value| !value.is_finite()) {
            return Err("XMAP13 selected mesh has no finite drawable bounds".into());
        }
        let declared_textures = row
            .ordered_texture_content
            .iter()
            .map(|content| texture_basename(&content.normalized_game_relative_path))
            .collect::<Vec<_>>();
        if declared_textures.iter().collect::<BTreeSet<_>>().len() != declared_textures.len() {
            return Err("XMAP13 admitted texture closure contains duplicate identities".into());
        }
        if expected_textures != declared_textures {
            return Err(
                "XMAP13 admitted texture closure differs from selected mesh/material textures"
                    .into(),
            );
        }
        validate_skin_palette(
            row.skinned,
            row.skin_palette_sha256.as_deref(),
            row.skin_palette_3200_bytes.as_deref(),
        )?;
        // The hit-test AABB has to follow the LOD the VIEWER's camera selects,
        // not the one the recorder's camera did, so bounds are collected for
        // every LOD the mesh declares and the caller indexes them with the LOD
        // that was actually drawn.
        let mut lod_bounds = vec![None; model.lod_distances.len().saturating_add(1)];
        for shape in &model.shapes {
            let lod = shape.lod.unwrap_or(0).max(0) as usize;
            let Some(slot) = lod_bounds.get_mut(lod) else {
                continue;
            };
            for mesh in &shape.meshes {
                // The same three categories `xmap12_pdxmesh.rs` skips when it
                // materializes. Bounds that include geometry the renderer never
                // draws are a hit target larger than the thing on screen.
                if mesh.material.is_collision() || mesh.verts.is_empty() || mesh.indices.is_empty()
                {
                    continue;
                }
                let entry = slot.get_or_insert(([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]));
                for axis in 0..3 {
                    entry.0[axis] = entry.0[axis].min(mesh.aabb_min[axis]);
                    entry.1[axis] = entry.1[axis].max(mesh.aabb_max[axis]);
                }
            }
        }
        Ok(Xmap13MeshSelection {
            aabb_min: min,
            aabb_max: max,
            lod_count: model.lod_distances.len(),
            lod_bounds,
        })
    }
}

/// What `Xmap13Content::validate_mesh_selection` proves about one row's mesh.
struct Xmap13MeshSelection {
    /// Bounds of the shapes the PRODUCER declared, kept as the fallback for a
    /// unit the viewer's camera never materializes.
    aabb_min: [f32; 3],
    aabb_max: [f32; 3],
    /// Bounds per LOD, indexed by LOD, for the LOD the viewer's camera picks.
    lod_bounds: Vec<Option<([f32; 3], [f32; 3])>>,
    lod_count: usize,
}

fn validate_skin_palette(
    skinned: bool,
    palette_sha256: Option<&str>,
    palette_bytes: Option<&[u8]>,
) -> Result<()> {
    match (skinned, palette_sha256, palette_bytes) {
        (true, Some(palette_sha256), Some(palette_bytes)) if palette_bytes.len() == 3_200 => {
            validate_sha256(palette_sha256, "XMAP13 skin palette")?;
            if Sha256::hex(palette_bytes) != palette_sha256 {
                return Err("XMAP13 skin palette bytes/digest disagree".into());
            }
        }
        (false, None, None) => {}
        _ => return Err("XMAP13 skinned tag/palette payload disagree".into()),
    }
    Ok(())
}

fn validate_typed_join(frame: &replay_core::WorldFrame, row: &Xmap13SemanticRow) -> Result<()> {
    match &row.identity {
        Xmap13UnitIdentity::Division { type_id, stable_id } => {
            let unit = frame
                .division_units()
                .iter()
                .find(|unit| unit.div_uid == row.semantic_source_id)
                .ok_or("XMAP13 division semantic row has no typed WorldFrame join")?;
            if unit.raw_division_id != *stable_id
                || unit.raw_division_type.measured() != Some(type_id)
                || unit.owner.measured().map(String::as_str) != Some(row.owner_country.as_str())
                || unit.location_province.measured() != Some(&row.anchor_province_id)
            {
                return Err("XMAP13 division typed identity/owner/anchor join mismatch".into());
            }
        }
        Xmap13UnitIdentity::TaskForce { .. } => {
            let unit = frame
                .fleets()
                .iter()
                .find(|unit| unit.task_force_uid == row.semantic_source_id)
                .ok_or("XMAP13 task-force semantic row has no typed WorldFrame join")?;
            if unit.owner.measured().map(String::as_str) != Some(row.owner_country.as_str())
                || unit.location_province.measured() != Some(&row.anchor_province_id)
            {
                return Err("XMAP13 task-force owner/anchor join mismatch".into());
            }
            let members = row
                .task_force_members
                .as_ref()
                .ok_or("XMAP13 task-force row lacks complete ship members")?;
            if let Some(ship_count) = unit.ship_count.measured() {
                if *ship_count < 0 || usize::try_from(*ship_count)? != members.len() {
                    return Err(
                        "XMAP13 task-force ship count disagrees with typed WorldFrame".into(),
                    );
                }
            }
            if let Some(ships) = unit.ships.measured() {
                if ships.len() != members.len() {
                    return Err(
                        "XMAP13 task-force member census disagrees with typed WorldFrame".into(),
                    );
                }
                for member in members {
                    let ship = ships
                        .iter()
                        .find(|ship| {
                            ship.ship_uid.measured().map(String::as_str)
                                == Some(member.semantic_ship_uid.as_str())
                        })
                        .ok_or("XMAP13 task-force member has no typed ship-composition join")?;
                    if ship.definition.measured().map(String::as_str)
                        != Some(member.definition_id.as_str())
                    {
                        return Err(
                            "XMAP13 task-force ship definition disagrees with typed WorldFrame"
                                .into(),
                        );
                    }
                }
            }
        }
        Xmap13UnitIdentity::AirWing { stable_id } => {
            if stable_id != &row.semantic_source_id {
                return Err("XMAP13 air-wing stable/source identities disagree".into());
            }
            let unit = frame
                .wings()
                .iter()
                .find(|unit| unit.wing_uid == row.semantic_source_id)
                .ok_or("XMAP13 air-wing semantic row has no typed WorldFrame join")?;
            if unit.owner.measured().map(String::as_str) != Some(row.owner_country.as_str())
                || unit.air_base_province.measured() != Some(&row.anchor_province_id)
            {
                return Err("XMAP13 air-wing owner/air-base join mismatch".into());
            }
        }
    }
    Ok(())
}

fn validate_census(frame: &replay_core::WorldFrame, units: &[PreparedXmap13Unit]) -> Result<()> {
    let divisions = units
        .iter()
        .filter(|row| matches!(row.identity, Xmap13UnitIdentity::Division { .. }))
        .count();
    let task_forces = units
        .iter()
        .filter(|row| matches!(row.identity, Xmap13UnitIdentity::TaskForce { .. }))
        .count();
    let air_wings = units
        .iter()
        .filter(|row| matches!(row.identity, Xmap13UnitIdentity::AirWing { .. }))
        .count();
    if divisions != frame.division_units().len()
        || task_forces != frame.fleets().len()
        || air_wings != frame.wings().len()
    {
        return Err("XMAP13 semantic rows do not exactly cover typed unit censuses".into());
    }
    Ok(())
}

fn validate_formation_census(
    frame: &replay_core::WorldFrame,
    formations: &[Xmap13FormationIdentity],
) -> Result<()> {
    validate_formation_rows(formations)?;
    let manager = frame.army_orders_group_manager();
    if manager.status != ViewStatus::Present
        || !manager.complete
        || !manager.owner_identities_complete
    {
        return Err("XMAP13 requires the complete typed army/army-group identity census".into());
    }
    let expected = manager
        .armies
        .iter()
        .map(|row| (false, row.army_id))
        .chain(manager.army_group_ids.iter().map(|id| (true, *id)))
        .collect::<BTreeSet<_>>();
    let actual = formations
        .iter()
        .map(|formation| (formation.owner_is_army_group, formation.owner_id))
        .collect::<BTreeSet<_>>();
    for formation in formations {
        if !formation.owner_is_army_group {
            for division in frame
                .division_units()
                .iter()
                .filter(|division| division.army_id.measured() == Some(&formation.owner_id))
            {
                if division.owner.measured().map(String::as_str)
                    != Some(formation.owner_country.as_str())
                {
                    return Err(
                        "XMAP13 army formation country disagrees with an assigned division".into(),
                    );
                }
            }
        }
    }
    if actual != expected {
        return Err("XMAP13 formation identities do not exactly cover typed manager census".into());
    }
    Ok(())
}

fn validate_formation_rows(formations: &[Xmap13FormationIdentity]) -> Result<()> {
    let mut identities = BTreeSet::new();
    for formation in formations {
        if formation.owner_id == 0
            || formation.owner_country.trim().is_empty()
            || !identities.insert((formation.owner_is_army_group, formation.owner_id))
        {
            return Err("XMAP13 formation identity is invalid or duplicated".into());
        }
    }
    Ok(())
}

fn disclosure(
    political: &crate::PreparedPoliticalFrame,
    context: &FoundationFeatureContext<'_>,
    row: &Xmap13SemanticRow,
) -> Result<(ExactUnitDisclosure, Xmap05VisibilityReason)> {
    match &row.identity {
        Xmap13UnitIdentity::Division { type_id, stable_id } => {
            Ok(political.visibility.land_disclosure(*type_id, *stable_id))
        }
        Xmap13UnitIdentity::TaskForce { type_id, stable_id } => Ok(political
            .visibility
            .taskforce_disclosure(*type_id, *stable_id)),
        Xmap13UnitIdentity::AirWing { .. } => {
            if context.viewer.observer_country_id.is_some() {
                return Err(
                    "XMAP05 explicitly has no authoritative air-wing spotting producer for country POV"
                        .into(),
                );
            }
            Ok((
                ExactUnitDisclosure::ExactDetails,
                Xmap05VisibilityReason::OmniscientViewer,
            ))
        }
    }
}

fn counter_allegiance(
    context: &FoundationFeatureContext<'_>,
    owner: &str,
) -> Result<Xmap13CounterAllegiance> {
    let Some(observer) = context.viewer.observer_country_id.as_deref() else {
        return Ok(Xmap13CounterAllegiance::WorldObserver);
    };
    if observer == owner {
        return Ok(Xmap13CounterAllegiance::Ours);
    }
    let key = if observer < owner {
        (observer.to_string(), owner.to_string())
    } else {
        (owner.to_string(), observer.to_string())
    };
    let pair = context
        .world_frame
        .diplomacy()
        .pair(&key.0, &key.1)
        .ok_or("XMAP13 counter allegiance has no exact diplomatic pair")?;
    match pair.at_war {
        Leaf::Measured(true) => Ok(Xmap13CounterAllegiance::Enemy),
        Leaf::Measured(false) => Ok(Xmap13CounterAllegiance::Other),
        _ => Err("XMAP13 counter allegiance at-war leaf is unavailable".into()),
    }
}

fn counter_allegiance_unavailable(
    context: &FoundationFeatureContext<'_>,
    envelope: &Xmap13SemanticEnvelope,
    political: &crate::PreparedPoliticalFrame,
) -> Result<Option<String>> {
    let Some(observer) = context.viewer.observer_country_id.as_deref() else {
        return Ok(None);
    };
    for row in &envelope.rows {
        if row.owner_country == observer {
            continue;
        }
        let (disclosure, _) = disclosure(political, context, row)?;
        if !matches!(
            disclosure,
            ExactUnitDisclosure::MapVisible | ExactUnitDisclosure::ExactDetails
        ) {
            continue;
        }
        let key = if observer < row.owner_country.as_str() {
            (observer.to_string(), row.owner_country.clone())
        } else {
            (row.owner_country.clone(), observer.to_string())
        };
        let measured = context
            .world_frame
            .diplomacy()
            .pair(&key.0, &key.1)
            .is_some_and(|pair| matches!(pair.at_war, Leaf::Measured(_)));
        if !measured {
            return Ok(Some(format!(
                "counter allegiance is unavailable for observer {observer} and owner {}",
                row.owner_country
            )));
        }
    }
    Ok(None)
}

fn build_counter_groups(
    context: &FoundationFeatureContext<'_>,
    visible_units: &[PreparedXmap13Unit],
    grouping_tier: u8,
) -> Result<Vec<PreparedXmap13CounterGroup>> {
    if grouping_tier > 3 {
        return Err("XMAP13 selected grouping tier is outside the engine range".into());
    }

    #[derive(Clone)]
    struct GroupAccumulator {
        allegiance: Xmap13CounterAllegiance,
        members: Vec<(u32, Xmap13UnitIdentity, Xmap13CounterGroupMembership)>,
    }

    let mut group_order = Vec::new();
    let mut groups = BTreeMap::<(Xmap13CounterAllegiance, String), GroupAccumulator>::new();
    for row in visible_units {
        let Some(counter) = row.counter.as_ref() else {
            continue;
        };
        let allegiance = counter_allegiance(context, &row.owner_country)?;
        let membership = counter
            .ordered_group_memberships
            .iter()
            .find(|membership| {
                membership.grouping_tier == grouping_tier && membership.allegiance == allegiance
            })
            .ok_or("XMAP13 counter has no exact membership for selected tier/allegiance")?;
        let group_key = xmap13_selected_group_key(allegiance, &membership.stable_group_id);
        if !groups.contains_key(&group_key) {
            group_order.push(group_key.clone());
            groups.insert(
                group_key.clone(),
                GroupAccumulator {
                    allegiance,
                    members: Vec::new(),
                },
            );
        }
        let group = groups
            .get_mut(&group_key)
            .ok_or("XMAP13 internal counter-group insertion failed")?;
        if group.allegiance != allegiance {
            return Err("XMAP13 counter-group members disagree on allegiance".into());
        }
        group.members.push((
            membership.member_ordinal,
            row.identity.clone(),
            membership.clone(),
        ));
    }

    let mut prepared = Vec::with_capacity(group_order.len());
    for (allegiance, stable_group_id) in group_order {
        let mut group = groups
            .remove(&(allegiance, stable_group_id.clone()))
            .ok_or("XMAP13 counter-group order references an absent group")?;
        group.members.sort_by_key(|member| member.0);
        if group.members.windows(2).any(|pair| pair[0].0 >= pair[1].0)
            || grouping_tier == 0 && group.members.len() != 1
        {
            return Err(
                "XMAP13 filtered counter-group order is duplicated or invalid at tier zero".into(),
            );
        }

        let mut anchor = [0.0f32; 3];
        for (_, _, membership) in &group.members {
            let member_anchor = membership.member_world_anchor_f32_bits.map(f32::from_bits);
            for lane in 0..3 {
                anchor[lane] += member_anchor[lane];
            }
        }
        let divisor = group.members.len() as f32;
        for lane in &mut anchor {
            *lane /= divisor;
        }

        let mut items =
            BTreeMap::<String, (u32, Xmap13CounterItemKind, Vec<(u32, Xmap13UnitIdentity)>)>::new();
        for (_, identity, membership) in &group.members {
            let item = items.entry(membership.counter_item_id.clone()).or_insert((
                membership.counter_item_ordinal,
                membership.counter_item_kind,
                Vec::new(),
            ));
            if item.0 != membership.counter_item_ordinal || item.1 != membership.counter_item_kind {
                return Err("XMAP13 counter-item identity drifted across filtered members".into());
            }
            item.2
                .push((membership.counter_item_member_ordinal, identity.clone()));
        }
        let mut items = items.into_iter().collect::<Vec<_>>();
        items.sort_by_key(|(_, (ordinal, _, _))| *ordinal);
        if items.windows(2).any(|pair| (pair[0].1).0 >= (pair[1].1).0) {
            return Err("XMAP13 filtered counter-item source order is duplicated".into());
        }
        let ordered_items = items
            .into_iter()
            .enumerate()
            .map(
                |(runtime_ordinal, (stable_item_id, (_, kind, mut members)))| {
                    members.sort_by_key(|member| member.0);
                    if members.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
                        return Err(
                            "XMAP13 filtered counter-item member order is duplicated".into()
                        );
                    }
                    Ok(PreparedXmap13CounterItem {
                        stable_item_id,
                        kind,
                        item_ordinal: u32::try_from(runtime_ordinal)?,
                        ordered_member_identities: members
                            .into_iter()
                            .map(|member| member.1)
                            .collect(),
                    })
                },
            )
            .collect::<Result<Vec<_>>>()?;

        prepared.push(PreparedXmap13CounterGroup {
            stable_group_id,
            grouping_tier,
            group_world_anchor_f32_bits: anchor.map(f32::to_bits),
            allegiance: group.allegiance,
            ordered_member_identities: group.members.into_iter().map(|member| member.1).collect(),
            ordered_items,
        });
    }
    Ok(prepared)
}

fn xmap13_selected_group_key(
    allegiance: Xmap13CounterAllegiance,
    stable_group_id: &str,
) -> (Xmap13CounterAllegiance, String) {
    (allegiance, stable_group_id.to_string())
}

fn validate_prepared_counter_groups<'a>(
    visible_units: &'a [PreparedXmap13Unit],
    groups: &'a [PreparedXmap13CounterGroup],
) -> Result<BTreeSet<&'a Xmap13UnitIdentity>> {
    let by_identity = visible_units
        .iter()
        .map(|row| (&row.identity, row))
        .collect::<BTreeMap<_, _>>();
    let mut group_ids = BTreeSet::new();
    let mut member_ids = BTreeSet::new();
    for group in groups {
        if group.stable_group_id.trim().is_empty()
            || group.grouping_tier > 3
            || group.ordered_member_identities.is_empty()
            || !group_ids.insert((group.allegiance, group.stable_group_id.as_str()))
            || group
                .group_world_anchor_f32_bits
                .map(f32::from_bits)
                .into_iter()
                .any(|value| !value.is_finite())
        {
            return Err("XMAP13 prepared counter group is invalid or duplicated".into());
        }
        let mut prior_member_ordinal = None;
        for (ordinal, identity) in group.ordered_member_identities.iter().enumerate() {
            if !member_ids.insert(identity) {
                return Err("XMAP13 prepared counter member is duplicated".into());
            }
            let row = by_identity
                .get(identity)
                .ok_or("XMAP13 prepared counter member is not visible")?;
            let membership = row
                .counter
                .as_ref()
                .and_then(|counter| {
                    counter.ordered_group_memberships.iter().find(|membership| {
                        membership.grouping_tier == group.grouping_tier
                            && membership.allegiance == group.allegiance
                    })
                })
                .ok_or("XMAP13 prepared counter member lacks selected membership alternative")?;
            if membership.stable_group_id != group.stable_group_id
                || prior_member_ordinal
                    .replace(membership.member_ordinal)
                    .is_some_and(|prior| prior >= membership.member_ordinal)
                || ordinal == 0
                    && group.grouping_tier == 0
                    && group.ordered_member_identities.len() != 1
            {
                return Err("XMAP13 prepared counter group drifted from source membership".into());
            }
        }
        let rebuilt = rebuild_counter_group_without_context(group, &by_identity)?;
        if &rebuilt != group {
            return Err("XMAP13 prepared counter group/item closure drifted".into());
        }
    }
    Ok(member_ids)
}

fn rebuild_counter_group_without_context(
    group: &PreparedXmap13CounterGroup,
    by_identity: &BTreeMap<&Xmap13UnitIdentity, &PreparedXmap13Unit>,
) -> Result<PreparedXmap13CounterGroup> {
    let mut anchor = [0.0f32; 3];
    let mut item_map = BTreeMap::<u32, PreparedXmap13CounterItem>::new();
    for identity in &group.ordered_member_identities {
        let row = by_identity
            .get(identity)
            .ok_or("XMAP13 prepared group member disappeared")?;
        let membership = row
            .counter
            .as_ref()
            .and_then(|counter| {
                counter.ordered_group_memberships.iter().find(|membership| {
                    membership.grouping_tier == group.grouping_tier
                        && membership.allegiance == group.allegiance
                })
            })
            .ok_or("XMAP13 prepared group member lacks source membership")?;
        let member_anchor = membership.member_world_anchor_f32_bits.map(f32::from_bits);
        for lane in 0..3 {
            anchor[lane] += member_anchor[lane];
        }
        let item = item_map
            .entry(membership.counter_item_ordinal)
            .or_insert_with(|| PreparedXmap13CounterItem {
                stable_item_id: membership.counter_item_id.clone(),
                kind: membership.counter_item_kind,
                item_ordinal: 0,
                ordered_member_identities: Vec::new(),
            });
        if item.stable_item_id != membership.counter_item_id
            || item.kind != membership.counter_item_kind
        {
            return Err("XMAP13 prepared counter item disagrees with source partition".into());
        }
        item.ordered_member_identities.push(identity.clone());
    }
    let divisor = group.ordered_member_identities.len() as f32;
    for lane in &mut anchor {
        *lane /= divisor;
    }
    let mut ordered_items = item_map.into_values().collect::<Vec<_>>();
    for (ordinal, item) in ordered_items.iter_mut().enumerate() {
        item.item_ordinal = u32::try_from(ordinal)?;
        let stable_item_id = item.stable_item_id.clone();
        item.ordered_member_identities.sort_by_key(|identity| {
            by_identity
                .get(identity)
                .and_then(|row| row.counter.as_ref())
                .and_then(|counter| {
                    counter.ordered_group_memberships.iter().find(|membership| {
                        membership.grouping_tier == group.grouping_tier
                            && membership.allegiance == group.allegiance
                            && membership.counter_item_id == stable_item_id
                    })
                })
                .map(|membership| membership.counter_item_member_ordinal)
                .unwrap_or(u32::MAX)
        });
    }
    Ok(PreparedXmap13CounterGroup {
        stable_group_id: group.stable_group_id.clone(),
        grouping_tier: group.grouping_tier,
        group_world_anchor_f32_bits: anchor.map(f32::to_bits),
        allegiance: group.allegiance,
        ordered_member_identities: group.ordered_member_identities.clone(),
        ordered_items,
    })
}

fn project_hit_target(
    camera: &crate::ProductionMapCamera,
    viewport_width: u32,
    viewport_height: u32,
    row: &PreparedXmap13Unit,
) -> Option<PreparedXmap13HitTarget> {
    let min = row.local_aabb_min_f32_bits.map(f32::from_bits);
    let max = row.local_aabb_max_f32_bits.map(f32::from_bits);
    let matrix = row.world_matrix_f32_bits.map(f32::from_bits);
    let mut screen_min = [f32::INFINITY; 2];
    let mut screen_max = [f32::NEG_INFINITY; 2];
    let mut nearest = f32::INFINITY;
    for x in [min[0], max[0]] {
        for y in [min[1], max[1]] {
            for z in [min[2], max[2]] {
                let world = transform_point([x, y, z], matrix);
                let (sx, sy) =
                    camera.world_to_screen(world, viewport_width as f32, viewport_height as f32)?;
                screen_min[0] = screen_min[0].min(sx);
                screen_min[1] = screen_min[1].min(sy);
                screen_max[0] = screen_max[0].max(sx);
                screen_max[1] = screen_max[1].max(sy);
                let dx = world[0] - camera.position[0];
                let dy = world[1] - camera.position[1];
                let dz = world[2] - camera.position[2];
                nearest = nearest.min(dx * dx + dy * dy + dz * dz);
            }
        }
    }
    (screen_min.into_iter().chain(screen_max).all(f32::is_finite)
        && nearest.is_finite()
        && screen_max[0] >= 0.0
        && screen_max[1] >= 0.0
        && screen_min[0] < viewport_width as f32
        && screen_min[1] < viewport_height as f32)
        .then(|| PreparedXmap13HitTarget {
            identity: row.identity.clone(),
            min_screen_f32_bits: screen_min.map(f32::to_bits),
            max_screen_f32_bits: screen_max.map(f32::to_bits),
            depth_f32_bits: nearest.to_bits(),
        })
}

fn transform_point(point: [f32; 3], matrix: [f32; 16]) -> [f32; 3] {
    [
        point[0] * matrix[0] + point[1] * matrix[4] + point[2] * matrix[8] + matrix[12],
        point[0] * matrix[1] + point[1] * matrix[5] + point[2] * matrix[9] + matrix[13],
        point[0] * matrix[2] + point[1] * matrix[6] + point[2] * matrix[10] + matrix[14],
    ]
}

fn validate_world_matrix(bits: [u32; 16]) -> Result<()> {
    if bits
        .map(f32::from_bits)
        .into_iter()
        .any(|value| !value.is_finite())
    {
        return Err("XMAP13 world matrix contains a non-finite lane".into());
    }
    Ok(())
}

fn validate_animation(row: &Xmap13SemanticRow) -> Result<()> {
    let phase = f32::from_bits(row.animation_phase_f32_bits);
    let speed = f32::from_bits(row.animation_speed_f32_bits);
    if !phase.is_finite() || phase < 0.0 || !speed.is_finite() {
        return Err("XMAP13 animation phase/speed is invalid".into());
    }
    match (&row.animation_name, &row.animation_asset) {
        (None, None) => Ok(()),
        (Some(name), Some(_)) if !name.trim().is_empty() => Ok(()),
        _ => Err("XMAP13 animation name/content identity is incomplete".into()),
    }
}

fn validate_events(block_count: u32, events: &[Xmap13AuthoredStateEvent]) -> Result<()> {
    let mut previous = None;
    let mut next_effect_by_event = BTreeMap::new();
    for event in events {
        let identity = (event.authored_event_index, event.authored_effect_index);
        let next_effect = next_effect_by_event
            .entry(event.authored_event_index)
            .or_insert(0u32);
        if event.authored_event_index >= block_count
            || previous.is_some_and(|previous| previous >= identity)
            || event.authored_effect_index != *next_effect
            || matches!(event.effect_kind, Xmap13AuthoredEffectKind::Particle)
                && event.effect_token.trim().is_empty()
            || matches!(event.effect_kind, Xmap13AuthoredEffectKind::Particle)
                && event
                    .node
                    .as_deref()
                    .map_or(true, |node| node.trim().is_empty())
            || event
                .node
                .as_deref()
                .is_some_and(|node| node.trim().is_empty())
            || event
                .event_time_f32_bits
                .is_some_and(|bits| !f32::from_bits(bits).is_finite() || f32::from_bits(bits) < 0.0)
        {
            return Err("XMAP13 authored state-event row is invalid or duplicated".into());
        }
        *next_effect += 1;
        previous = Some(identity);
    }
    Ok(())
}

fn validate_counter(counter: Option<&Xmap13CounterSemantic>) -> Result<()> {
    let counter =
        counter.ok_or("XMAP13 unit row omitted its complete counter/icon presentation")?;
    if counter.counter_kind.trim().is_empty()
        || counter.ordered_interface_content.is_empty()
        || counter.ordered_presentations.is_empty()
    {
        return Err("XMAP13 counter semantic row is invalid".into());
    }
    let mut memberships = BTreeSet::new();
    for group in &counter.ordered_group_memberships {
        if group.grouping_tier > 3
            || !memberships.insert((group.grouping_tier, group.allegiance))
            || group.stable_group_id.trim().is_empty()
            || group
                .member_world_anchor_f32_bits
                .map(f32::from_bits)
                .into_iter()
                .any(|value| !value.is_finite())
            || group.member_count == 0
            || group.member_ordinal >= group.member_count
            || group.grouping_tier == 0 && group.member_count != 1
            || group.counter_item_id.trim().is_empty()
            || group.counter_item_count == 0
            || group.counter_item_ordinal >= group.counter_item_count
            || group.counter_item_member_count == 0
            || group.counter_item_member_ordinal >= group.counter_item_member_count
        {
            return Err("XMAP13 counter grouping membership is invalid".into());
        }
    }
    let expected_memberships = (0..4)
        .flat_map(|tier| {
            [
                Xmap13CounterAllegiance::WorldObserver,
                Xmap13CounterAllegiance::Ours,
                Xmap13CounterAllegiance::Enemy,
                Xmap13CounterAllegiance::Other,
            ]
            .into_iter()
            .map(move |allegiance| (tier, allegiance))
        })
        .collect::<BTreeSet<_>>();
    if memberships != expected_memberships {
        return Err("XMAP13 counter lacks an exact membership for every tier/allegiance".into());
    }
    let membership_items = counter
        .ordered_group_memberships
        .iter()
        .map(|membership| {
            (
                (membership.grouping_tier, membership.allegiance),
                membership.counter_item_id.as_str(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut variants = BTreeSet::new();
    for presentation in &counter.ordered_presentations {
        let key = (
            presentation.grouping_tier,
            presentation.counter_item_id.as_str(),
            presentation.allegiance,
            presentation.selected,
        );
        if presentation.grouping_tier > 3
            || presentation.counter_item_id.trim().is_empty()
            || membership_items.get(&(presentation.grouping_tier, presentation.allegiance))
                != Some(&presentation.counter_item_id.as_str())
            || !variants.insert(key)
            || presentation.ordered_primitives.is_empty()
            || presentation
                .anchor_offset_px_f32_bits
                .map(f32::from_bits)
                .into_iter()
                .chain([f32::from_bits(presentation.stack_advance_px_f32_bits)])
                .any(|value| !value.is_finite())
            || f32::from_bits(presentation.stack_advance_px_f32_bits) <= 0.0
        {
            return Err("XMAP13 counter presentation variant is invalid or duplicated".into());
        }
        let mut layer_ids = BTreeSet::new();
        let mut active_ability_ordinals = BTreeSet::new();
        for primitive in &presentation.ordered_primitives {
            let (stable_layer_id, authored_control_name, rect, modulation) = match primitive {
                Xmap13CounterPrimitive::Sprite {
                    stable_layer_id,
                    authored_control_name,
                    authored_sprite_name,
                    binding,
                    authored_frame_index,
                    frame_selection,
                    runtime_collection_instance_ordinal,
                    texture_content,
                    source_rect_px,
                    destination_rect_f32_bits,
                    rotation_f32_bits,
                    visible_fraction_f32_bits,
                    modulation_rgba_f32_bits,
                    ..
                } => {
                    texture_content.validate().map_err(|error| {
                        format!("XMAP13 counter sprite content identity: {error}")
                    })?;
                    if authored_sprite_name.trim().is_empty()
                        || matches!(
                            binding,
                            Xmap13CounterSpriteBinding::RuntimeRebound {
                                authored_gui_sprite_name
                            } if authored_gui_sprite_name.trim().is_empty()
                        )
                        || matches!(
                            frame_selection,
                            Xmap13CounterFrameSelection::AuthoredFixed
                                | Xmap13CounterFrameSelection::AuthoredDefaultFirst
                                | Xmap13CounterFrameSelection::RuntimeSelectedOverAuthoredInitial
                        ) && authored_frame_index.is_none()
                        || source_rect_px[2] == 0
                        || source_rect_px[3] == 0
                        || !f32::from_bits(*rotation_f32_bits).is_finite()
                        || visible_fraction_f32_bits.is_some_and(|bits| {
                            let value = f32::from_bits(bits);
                            !value.is_finite() || !(0.0 < value && value <= 1.0)
                        })
                        || matches!(authored_control_name.as_str(), "bar_org" | "bar_str")
                            != visible_fraction_f32_bits.is_some()
                        || (authored_control_name == "active_ability")
                            != runtime_collection_instance_ordinal.is_some()
                    {
                        return Err("XMAP13 counter sprite source rectangle is empty".into());
                    }
                    if let Some(ordinal) = runtime_collection_instance_ordinal {
                        if *ordinal >= presentation.active_ability_instance_count
                            || !active_ability_ordinals.insert(*ordinal)
                        {
                            return Err("XMAP13 active-ability instance census is invalid".into());
                        }
                    }
                    (
                        stable_layer_id,
                        authored_control_name,
                        Some(*destination_rect_f32_bits),
                        Some(*modulation_rgba_f32_bits),
                    )
                }
                Xmap13CounterPrimitive::Text {
                    stable_layer_id,
                    authored_control_name,
                    font_name,
                    ordered_font_content,
                    value,
                    control_rect_f32_bits,
                    ..
                } => {
                    if font_name.trim().is_empty()
                        || ordered_font_content.is_empty()
                        || matches!(value, Xmap13CounterTextValue::Literal { value } if value.is_empty())
                    {
                        return Err("XMAP13 counter text primitive is incomplete".into());
                    }
                    for content in ordered_font_content {
                        content.validate().map_err(|error| {
                            format!("XMAP13 counter font content identity: {error}")
                        })?;
                    }
                    (
                        stable_layer_id,
                        authored_control_name,
                        Some(*control_rect_f32_bits),
                        None,
                    )
                }
                Xmap13CounterPrimitive::Hidden {
                    stable_layer_id,
                    authored_control_name,
                } => (stable_layer_id, authored_control_name, None, None),
            };
            if stable_layer_id.trim().is_empty()
                || authored_control_name == "active_ability"
                    && !matches!(primitive, Xmap13CounterPrimitive::Sprite { .. })
                || !layer_ids.insert(stable_layer_id.as_str())
                || rect.is_some_and(|bits| {
                    let rect = bits.map(f32::from_bits);
                    rect.into_iter().any(|value| !value.is_finite())
                        || rect[2] <= 0.0
                        || rect[3] <= 0.0
                })
                || modulation.is_some_and(|bits| {
                    bits.map(f32::from_bits)
                        .into_iter()
                        .any(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
                })
            {
                return Err("XMAP13 counter presentation primitive is invalid".into());
            }
        }
        if active_ability_ordinals != (0..presentation.active_ability_instance_count).collect() {
            return Err("XMAP13 active-ability instance census is incomplete".into());
        }
    }
    for membership in &counter.ordered_group_memberships {
        for selected in [false, true] {
            if !variants.contains(&(
                membership.grouping_tier,
                membership.counter_item_id.as_str(),
                membership.allegiance,
                selected,
            )) {
                return Err(
                    "XMAP13 counter lacks its exact membership presentation variant".into(),
                );
            }
        }
    }
    Ok(())
}

#[derive(Default)]
struct Xmap13MembershipItemAudit {
    stable_item_id: String,
    kind: Option<Xmap13CounterItemKind>,
    declared_member_count: Option<u32>,
    member_ordinals: BTreeSet<u32>,
}

#[derive(Default)]
struct Xmap13MembershipGroupAudit {
    declared_member_count: Option<u32>,
    declared_item_count: Option<u32>,
    member_ordinals: BTreeSet<u32>,
    items: BTreeMap<u32, Xmap13MembershipItemAudit>,
}

fn validate_complete_counter_memberships(rows: &[Xmap13SemanticRow]) -> Result<()> {
    let mut groups =
        BTreeMap::<(u8, Xmap13CounterAllegiance, String), Xmap13MembershipGroupAudit>::new();
    for row in rows {
        let Some(counter) = &row.counter else {
            continue;
        };
        for membership in &counter.ordered_group_memberships {
            let group = groups
                .entry((
                    membership.grouping_tier,
                    membership.allegiance,
                    membership.stable_group_id.clone(),
                ))
                .or_default();
            if group
                .declared_member_count
                .replace(membership.member_count)
                .is_some_and(|count| count != membership.member_count)
                || group
                    .declared_item_count
                    .replace(membership.counter_item_count)
                    .is_some_and(|count| count != membership.counter_item_count)
                || !group.member_ordinals.insert(membership.member_ordinal)
            {
                return Err(
                    "XMAP13 source counter-group census is inconsistent or duplicated".into(),
                );
            }
            let item = group
                .items
                .entry(membership.counter_item_ordinal)
                .or_default();
            if item.stable_item_id.is_empty() {
                item.stable_item_id = membership.counter_item_id.clone();
                item.kind = Some(membership.counter_item_kind);
            }
            if item.stable_item_id != membership.counter_item_id
                || item.kind != Some(membership.counter_item_kind)
                || item
                    .declared_member_count
                    .replace(membership.counter_item_member_count)
                    .is_some_and(|count| count != membership.counter_item_member_count)
                || !item
                    .member_ordinals
                    .insert(membership.counter_item_member_ordinal)
            {
                return Err(
                    "XMAP13 source counter-item census is inconsistent or duplicated".into(),
                );
            }
        }
    }
    validate_membership_group_audits(&groups)
}

fn validate_membership_group_audits(
    groups: &BTreeMap<(u8, Xmap13CounterAllegiance, String), Xmap13MembershipGroupAudit>,
) -> Result<()> {
    for group in groups.values() {
        let member_count = group
            .declared_member_count
            .ok_or("XMAP13 source counter group lacks a member count")?;
        let item_count = group
            .declared_item_count
            .ok_or("XMAP13 source counter group lacks an item count")?;
        if group.member_ordinals != (0..member_count).collect()
            || group.items.len() != item_count as usize
            || group.items.keys().copied().collect::<BTreeSet<_>>() != (0..item_count).collect()
        {
            return Err("XMAP13 source counter-group census is incomplete".into());
        }
        for item in group.items.values() {
            let item_member_count = item
                .declared_member_count
                .ok_or("XMAP13 source counter item lacks a member count")?;
            if item.member_ordinals != (0..item_member_count).collect() {
                return Err("XMAP13 source counter-item member census is incomplete".into());
            }
        }
    }
    Ok(())
}

fn validate_counter_kind(row: &Xmap13SemanticRow) -> Result<()> {
    let expected = match &row.identity {
        Xmap13UnitIdentity::Division { .. } => "land",
        Xmap13UnitIdentity::TaskForce { .. } => "naval",
        Xmap13UnitIdentity::AirWing { .. } => "air",
    };
    if row
        .counter
        .as_ref()
        .map(|counter| counter.counter_kind.as_str())
        != Some(expected)
    {
        return Err("XMAP13 counter kind disagrees with typed unit domain".into());
    }
    let item_kinds_are_valid = row.counter.as_ref().is_some_and(|counter| {
        counter.ordered_group_memberships.iter().all(|membership| {
            matches!(
                (&row.identity, membership.counter_item_kind),
                (
                    Xmap13UnitIdentity::Division { .. },
                    Xmap13CounterItemKind::Land | Xmap13CounterItemKind::RailwayGun
                ) | (
                    Xmap13UnitIdentity::TaskForce { .. },
                    Xmap13CounterItemKind::Naval
                ) | (
                    Xmap13UnitIdentity::AirWing { .. },
                    Xmap13CounterItemKind::Air
                )
            )
        })
    });
    if !item_kinds_are_valid {
        return Err("XMAP13 counter-item kind disagrees with typed unit domain".into());
    }
    Ok(())
}

fn validate_task_force_members(
    row: &Xmap13SemanticRow,
    seen_ship_instance: &mut BTreeSet<i64>,
) -> Result<()> {
    let members = match (&row.identity, &row.task_force_members) {
        (Xmap13UnitIdentity::TaskForce { .. }, Some(members)) => members,
        (Xmap13UnitIdentity::TaskForce { .. }, None) => {
            return Err("XMAP13 task-force row lacks a complete ship census".into())
        }
        (_, None) => return Ok(()),
        (_, Some(_)) => return Err("XMAP13 non-task-force row carries task-force members".into()),
    };
    let mut semantic_uids = BTreeSet::new();
    for member in members {
        if member.ship_instance_id <= 0
            || !seen_ship_instance.insert(member.ship_instance_id)
            || member.semantic_ship_uid.trim().is_empty()
            || !semantic_uids.insert(member.semantic_ship_uid.as_str())
            || member.definition_id.trim().is_empty()
            || member.hull_id.trim().is_empty()
            || member.ordered_category_ids.is_empty()
            || member
                .ordered_category_ids
                .iter()
                .any(|category| category.trim().is_empty())
            || member
                .ordered_category_ids
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                != member.ordered_category_ids.len()
        {
            return Err("XMAP13 task-force ship census is incomplete or duplicated".into());
        }
    }
    Ok(())
}

fn validate_draw(index_count: usize, vertex_count: usize, draw: &L03DrawArguments) -> Result<()> {
    match draw {
        L03DrawArguments::Draw {
            vertex_count: count,
            start_vertex,
        } if *count != 0
            && usize::try_from(*count)? <= vertex_count
            && usize::try_from(*start_vertex)? < vertex_count =>
        {
            Ok(())
        }
        L03DrawArguments::DrawIndexed {
            index_count: count,
            start_index,
            ..
        } if *count != 0
            && usize::try_from(*count)? <= index_count
            && usize::try_from(*start_index)? < index_count =>
        {
            Ok(())
        }
        L03DrawArguments::DrawIndexedInstanced {
            index_count_per_instance,
            instance_count,
            start_index,
            ..
        } if *index_count_per_instance != 0
            && *instance_count != 0
            && usize::try_from(*index_count_per_instance)? <= index_count
            && usize::try_from(*start_index)? < index_count =>
        {
            Ok(())
        }
        _ => Err("XMAP13 selected draw arguments exceed admitted mesh geometry".into()),
    }
}

fn row_content(row: &Xmap13SemanticRow) -> Vec<&UsedContentKey> {
    let mut out = vec![
        &row.entity_asset,
        &row.model_registry,
        &row.mesh,
        &row.shader_source,
    ];
    if let Some(animation) = &row.animation_asset {
        out.push(animation);
    }
    out.extend(&row.ordered_texture_content);
    if let Some(counter) = &row.counter {
        out.extend(&counter.ordered_interface_content);
    }
    if let Some(members) = &row.task_force_members {
        for member in members {
            out.push(&member.definition_content);
            out.push(&member.hull_content);
        }
    }
    out
}

fn unavailable_output(
    hour: i64,
    consumed_gap_ids: Vec<String>,
    reason: &str,
) -> FoundationFeatureOutput {
    unavailable_output_with_gap(
        hour,
        consumed_gap_ids,
        "XMAP13.semantic_unit_census_unavailable",
        reason,
    )
}

fn unavailable_output_with_gap(
    hour: i64,
    consumed_gap_ids: Vec<String>,
    gap_id: &str,
    reason: &str,
) -> FoundationFeatureOutput {
    let gap_id = gap_id.to_string();
    let witness_sha256 = Sha256::hex(format!("{gap_id}\n{hour}\n{hour}\n{reason}").as_bytes());
    FoundationFeatureOutput {
        disposition: FamilyDisposition::Unavailable {
            gap: FoundationGap {
                gap_id,
                first_unavailable_hour: hour,
                last_unavailable_hour: hour,
                witness_sha256,
                reason: reason.into(),
            },
        },
        prepared_terrain: None,
        prepared_political: None,
        prepared_thin_borders: None,
        prepared_rivers_straits: None,
        prepared_water: None,
        prepared_lighting: None,
        prepared_weather: None,
        prepared_effects: None,
        prepared_units: None,
        prepared_combat: None,
        prepared_orders: None,
        prepared_rail_supply: None,
        prepared_map_modes: None,
        prepared_map_furniture: None,
        consumed_gap_ids,
    }
}

fn ordered_identity_digest<'a>(
    identities: impl Iterator<Item = &'a Xmap13UnitIdentity>,
) -> Result<String> {
    let mut bytes = b"XMAP13_ORDERED_IDENTITIES_V1\0".to_vec();
    for identity in identities {
        bytes.extend_from_slice(&serde_json::to_vec(identity)?);
        bytes.push(0);
    }
    Ok(Sha256::hex(&bytes))
}

fn semantic_source_id_digest(identities: &[String]) -> String {
    let mut bytes = b"XMAP13_ORDERED_EFFECT_SEMANTIC_SOURCES_V1\0".to_vec();
    for identity in identities {
        bytes.extend_from_slice(identity.as_bytes());
        bytes.push(0);
    }
    Sha256::hex(&bytes)
}

fn ordered_formation_identity_digest(formations: &[Xmap13FormationIdentity]) -> Result<String> {
    let mut bytes = b"XMAP13_ORDERED_FORMATION_IDENTITIES_V1\0".to_vec();
    for formation in formations {
        bytes.extend_from_slice(&serde_json::to_vec(formation)?);
        bytes.push(0);
    }
    Ok(Sha256::hex(&bytes))
}

fn used_content_digest(content: &[UsedContentKey]) -> Result<String> {
    let mut bytes = b"XMAP13_USED_CONTENT_V1\0".to_vec();
    for row in content {
        row.validate()
            .map_err(|error| format!("XMAP13 used-content digest: {error}"))?;
        bytes.extend_from_slice(&serde_json::to_vec(row)?);
        bytes.push(0);
    }
    Ok(Sha256::hex(&bytes))
}

fn prepared_digest(frame: &PreparedXmap13Frame) -> Result<String> {
    let mut bytes = b"XMAP13_PREPARED_V1\0".to_vec();
    bytes.extend_from_slice(frame.schema.as_bytes());
    bytes.extend_from_slice(&frame.hour_index.to_le_bytes());
    bytes.extend_from_slice(frame.semantic_input_sha256.as_bytes());
    bytes.extend_from_slice(frame.identity_provider.census_sha256.as_bytes());
    bytes.extend_from_slice(frame.identity_provider.formation_census_sha256.as_bytes());
    bytes.extend_from_slice(frame.identity_provider.admission_sha256.as_bytes());
    bytes.extend_from_slice(&serde_json::to_vec(&frame.unit_entity_assignment)?);
    bytes.push(0);
    bytes.extend_from_slice(&serde_json::to_vec(&frame.counter_quad_assignment)?);
    bytes.push(0);
    bytes.extend_from_slice(used_content_digest(&frame.ordered_used_content)?.as_bytes());
    for row in &frame.identity_provider.ordered_units {
        append_prepared_unit_digest(&mut bytes, row)?;
    }
    for formation in &frame.identity_provider.ordered_formations {
        bytes.extend_from_slice(&serde_json::to_vec(formation)?);
        bytes.push(0);
    }
    for row in &frame.visible_units {
        bytes.extend_from_slice(row.identity.canonical_id().as_bytes());
        bytes.push(disclosure_code(row.disclosure));
        bytes.push(visibility_reason_code(row.visibility_reason));
        bytes.push(row.selected as u8);
        bytes.push(0);
    }
    for draw in &frame.native_entity_draws {
        bytes.extend_from_slice(draw.deterministic_sha256.as_bytes());
        bytes.push(0);
    }
    for group in &frame.displayed_counter_groups {
        bytes.extend_from_slice(group.stable_group_id.as_bytes());
        bytes.push(0);
        bytes.push(group.grouping_tier);
        bytes.push(counter_allegiance_code(group.allegiance));
        for lane in group.group_world_anchor_f32_bits {
            bytes.extend_from_slice(&lane.to_le_bytes());
        }
        for identity in &group.ordered_member_identities {
            bytes.extend_from_slice(identity.canonical_id().as_bytes());
            bytes.push(0);
        }
        for item in &group.ordered_items {
            bytes.extend_from_slice(item.stable_item_id.as_bytes());
            bytes.push(0);
            bytes.extend_from_slice(&item.item_ordinal.to_le_bytes());
            bytes.extend_from_slice(&serde_json::to_vec(&item.kind)?);
            bytes.push(0);
            for identity in &item.ordered_member_identities {
                bytes.extend_from_slice(identity.canonical_id().as_bytes());
                bytes.push(0);
            }
        }
        bytes.push(0);
    }
    for caster in &frame.light_depth_casters {
        bytes.extend_from_slice(&serde_json::to_vec(caster)?);
        bytes.push(0);
    }
    for hit in &frame.hit_targets {
        bytes.extend_from_slice(hit.identity.canonical_id().as_bytes());
        for lane in hit
            .min_screen_f32_bits
            .into_iter()
            .chain(hit.max_screen_f32_bits)
            .chain([hit.depth_f32_bits])
        {
            bytes.extend_from_slice(&lane.to_le_bytes());
        }
    }
    Ok(Sha256::hex(&bytes))
}

fn append_prepared_unit_digest(bytes: &mut Vec<u8>, row: &PreparedXmap13Unit) -> Result<()> {
    bytes.extend_from_slice(row.identity.canonical_id().as_bytes());
    bytes.push(0);
    for value in [
        row.semantic_source_id.as_str(),
        row.owner_country.as_str(),
        row.entity_name.as_str(),
        row.model_name.as_str(),
        row.state_name.as_str(),
        row.material_variant.as_str(),
        row.animation_name.as_deref().unwrap_or("<none>"),
        row.selected_mesh.as_str(),
        row.skin_palette_sha256.as_deref().unwrap_or("<none>"),
    ] {
        bytes.extend_from_slice(value.as_bytes());
        bytes.push(0);
    }
    bytes.extend_from_slice(&row.anchor_province_id.to_le_bytes());
    for content in [
        &row.entity_asset,
        &row.model_registry,
        &row.mesh,
        &row.shader_source,
    ] {
        bytes.extend_from_slice(&serde_json::to_vec(content)?);
        bytes.push(0);
    }
    bytes.extend_from_slice(&serde_json::to_vec(&row.animation_asset)?);
    bytes.extend_from_slice(&serde_json::to_vec(&row.ordered_texture_content)?);
    bytes.extend_from_slice(&serde_json::to_vec(&row.authored_state_events)?);
    bytes.extend_from_slice(&serde_json::to_vec(&row.task_force_members)?);
    bytes.extend_from_slice(&serde_json::to_vec(&row.counter)?);
    bytes.extend_from_slice(&row.animation_phase_f32_bits.to_le_bytes());
    bytes.extend_from_slice(&row.animation_speed_f32_bits.to_le_bytes());
    bytes.extend_from_slice(&row.authored_event_block_count.to_le_bytes());
    bytes.extend_from_slice(&row.world_matrix_f32_bits.concat_bytes());
    bytes.push(disclosure_code(row.disclosure));
    bytes.push(visibility_reason_code(row.visibility_reason));
    bytes.push(row.selected as u8);
    bytes.extend_from_slice(&row.selected_lod_index.to_le_bytes());
    bytes.extend_from_slice(&serde_json::to_vec(&row.selected_shape_indices)?);
    bytes.extend_from_slice(&serde_json::to_vec(&row.groups)?);
    bytes.push(row.skinned as u8);
    bytes.extend_from_slice(&serde_json::to_vec(&row.skin_palette_3200_bytes)?);
    bytes.push(row.casts_light_depth as u8);
    bytes.extend_from_slice(&row.local_aabb_min_f32_bits.concat_bytes());
    bytes.extend_from_slice(&row.local_aabb_max_f32_bits.concat_bytes());
    Ok(())
}

fn disclosure_code(value: ExactUnitDisclosure) -> u8 {
    match value {
        ExactUnitDisclosure::Hidden => 0,
        ExactUnitDisclosure::Detected => 1,
        ExactUnitDisclosure::MapVisible => 2,
        ExactUnitDisclosure::ExactDetails => 3,
    }
}

fn visibility_reason_code(value: Xmap05VisibilityReason) -> u8 {
    match value {
        Xmap05VisibilityReason::OmniscientViewer => 0,
        Xmap05VisibilityReason::D95MapVisibilityThresholdExcluded => 1,
        Xmap05VisibilityReason::D95DetectedTier => 2,
        Xmap05VisibilityReason::ExactD95MapVisible => 3,
        Xmap05VisibilityReason::ExactD95ExactDetails => 4,
    }
}

fn counter_allegiance_code(value: Xmap13CounterAllegiance) -> u8 {
    match value {
        Xmap13CounterAllegiance::WorldObserver => 0,
        Xmap13CounterAllegiance::Ours => 1,
        Xmap13CounterAllegiance::Enemy => 2,
        Xmap13CounterAllegiance::Other => 3,
    }
}

trait U32ArrayBytes {
    fn concat_bytes(&self) -> Vec<u8>;
}

impl<const N: usize> U32ArrayBytes for [u32; N] {
    fn concat_bytes(&self) -> Vec<u8> {
        self.iter().flat_map(|value| value.to_le_bytes()).collect()
    }
}

fn validate_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{label} is not canonical SHA-256").into());
    }
    Ok(())
}

fn normalize(value: &str) -> String {
    value
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_ascii_lowercase()
}

fn xmap13_counter_layout(
    manifest: &UnitCounterAssetManifest,
    kind: Xmap13CounterItemKind,
) -> &GuiContainerLayout {
    match kind {
        Xmap13CounterItemKind::Land => &manifest.gui_layouts.land,
        Xmap13CounterItemKind::Naval => &manifest.gui_layouts.navy,
        Xmap13CounterItemKind::RailwayGun => &manifest.gui_layouts.railway_gun,
        Xmap13CounterItemKind::Air => &manifest.gui_layouts.air_wing,
    }
}

fn xmap13_resolve_control_ordinal(
    drawable_controls: &[(usize, &GuiControlLayout)],
    named_control_ordinals: &BTreeMap<&str, usize>,
    covered_control_ordinals: &BTreeSet<usize>,
    authored_control_name: &str,
) -> Result<usize> {
    if authored_control_name.trim().is_empty() {
        return drawable_controls
            .iter()
            .find(|(ordinal, control)| {
                control.name.trim().is_empty() && !covered_control_ordinals.contains(ordinal)
            })
            .map(|(ordinal, _)| *ordinal)
            .ok_or_else(|| {
                "XMAP13 unnamed primitive has no remaining unnamed drawable control".into()
            });
    }
    named_control_ordinals
        .get(authored_control_name)
        .copied()
        .ok_or_else(|| "XMAP13 primitive names no drawable control in the effective GUI".into())
}

fn xmap13_counter_content<'a>(
    counter: &'a Xmap13CounterSemantic,
    path: &str,
) -> Option<&'a UsedContentKey> {
    let path = normalize(path);
    counter
        .ordered_interface_content
        .iter()
        .find(|content| normalize(&content.normalized_game_relative_path) == path)
}

fn xmap13_require_counter_content(
    adapter: &Xmap13Content,
    counter: &Xmap13CounterSemantic,
    path: &str,
) -> Result<()> {
    let content = xmap13_counter_content(counter, path)
        .ok_or_else(|| format!("XMAP13 effective content {path} is absent from the closure"))?;
    adapter.resolver.read_admitted_used_content(content)?;
    Ok(())
}

fn xmap13_counter_sprite<'a>(
    manifest: &'a UnitCounterAssetManifest,
    name: &str,
) -> Option<&'a crate::CounterSpriteAsset> {
    manifest
        .map_counter_sprites
        .get(name)
        .or_else(|| manifest.unitview_sprites.get(name))
        .or_else(|| {
            manifest
                .on_map_division_symbols
                .iter()
                .find(|asset| asset.name == name)
        })
        .or_else(|| {
            manifest
                .on_map_ship_symbols
                .iter()
                .find(|asset| asset.name == name)
        })
        .or_else(|| {
            manifest
                .nato_text_icons
                .iter()
                .find(|asset| asset.name == name)
        })
}

fn xmap13_runtime_sprite_rebind_is_admitted(
    manifest: &UnitCounterAssetManifest,
    item_kind: Xmap13CounterItemKind,
    control_name: &str,
    selected_sprite_name: &str,
) -> bool {
    match control_name {
        "type" => match item_kind {
            Xmap13CounterItemKind::Land | Xmap13CounterItemKind::RailwayGun => manifest
                .on_map_division_symbols
                .iter()
                .any(|asset| asset.name == selected_sprite_name),
            Xmap13CounterItemKind::Naval => manifest
                .on_map_ship_symbols
                .iter()
                .any(|asset| asset.name == selected_sprite_name),
            Xmap13CounterItemKind::Air => false,
        },
        "general_portrait" | "active_ability" => !selected_sprite_name.trim().is_empty(),
        _ => false,
    }
}

fn xmap13_active_ability_template_control(
    template: &GuiContainerLayout,
) -> Result<&GuiControlLayout> {
    let drawable_controls = template
        .controls
        .iter()
        .filter(|control| {
            control.sprite_type.is_some()
                || control.quad_texture_sprite.is_some()
                || control.font.is_some()
        })
        .collect::<Vec<_>>();
    if drawable_controls.len() != 1
        || !drawable_controls[0].kind.eq_ignore_ascii_case("iconType")
        || drawable_controls[0].name != "active_ability"
        || drawable_controls[0].sprite_type.is_none()
        || drawable_controls[0].quad_texture_sprite.is_some()
        || drawable_controls[0].font.is_some()
    {
        return Err("XMAP13 active-ability item template has an unsupported drawable shape".into());
    }
    Ok(drawable_controls[0])
}

fn xmap13_validate_frame_selection(
    control: &GuiControlLayout,
    selection: Xmap13CounterFrameSelection,
    selected_frame: Option<u32>,
    frame_count: Option<u32>,
    code_textured: bool,
    runtime_gui_initial_override_admitted: bool,
) -> Result<()> {
    if code_textured {
        if selection == Xmap13CounterFrameSelection::RuntimeSelected
            && control.frame.is_none()
            && selected_frame.is_none()
            && frame_count.is_none()
        {
            return Ok(());
        }
        return Err("XMAP13 code-textured sprite frame source is invalid".into());
    }
    let selected = selected_frame.ok_or("XMAP13 file-textured sprite omitted its frame")?;
    let count = frame_count.ok_or("XMAP13 file-textured sprite omitted its frame count")?;
    if selected >= count {
        return Err("XMAP13 selected sprite frame is outside its effective atlas".into());
    }
    match selection {
        Xmap13CounterFrameSelection::AuthoredFixed => {
            let authored = control
                .frame
                .and_then(|frame| frame.checked_sub(1))
                .ok_or("XMAP13 fixed GUI frame must be positive")?;
            if selected != authored {
                return Err(
                    "XMAP13 selected frame differs from the fixed effective-GUI frame".into(),
                );
            }
        }
        Xmap13CounterFrameSelection::AuthoredDefaultFirst => {
            if control.frame.is_some() || selected != 0 {
                return Err("XMAP13 default-first frame source differs from effective GUI".into());
            }
        }
        Xmap13CounterFrameSelection::RuntimeSelected => {
            if control.frame.is_some() {
                return Err(
                    "XMAP13 runtime frame selection overrides a fixed effective-GUI frame".into(),
                );
            }
        }
        Xmap13CounterFrameSelection::RuntimeSelectedOverAuthoredInitial => {
            if !runtime_gui_initial_override_admitted || control.frame != Some(1) {
                return Err(
                    "XMAP13 runtime frame selection has no admitted authored-initial override"
                        .into(),
                );
            }
        }
    }
    Ok(())
}

fn xmap13_validate_fractional_bar_rotation(
    rotation: f32,
    has_visible_fraction: bool,
) -> Result<()> {
    if has_visible_fraction && rotation.to_bits() != 1.5708f32.to_bits() {
        return Err("XMAP13 fractional bar has no proved non-vanilla rotation law".into());
    }
    Ok(())
}

fn xmap13_validate_sprite_position(
    control: &GuiControlLayout,
    destination: [f32; 4],
    active_ability_context: Option<(&GuiControlLayout, &GuiContainerLayout, u32)>,
) -> Result<()> {
    if let Some((grid, template, instance_ordinal)) = active_ability_context {
        let grid_position = xmap13_effective_ancestor_origin(grid)?;
        let slot = grid
            .slot_size
            .ok_or("XMAP13 active-abilities grid lacks an effective slot size")?;
        let template_size = template
            .size
            .ok_or("XMAP13 active-ability item template lacks an effective size")?;
        let control_parent_size = control
            .parent_size
            .ok_or("XMAP13 active-ability control lacks an effective parent size")?;
        let control_position = xmap13_effective_control_base(control)?;
        if grid.kind.to_ascii_lowercase() != "gridboxtype"
            || grid.max_slots_vertical != Some(1)
            || grid
                .format
                .as_deref()
                .map(|format| format.eq_ignore_ascii_case("upper_left"))
                != Some(true)
            || control
                .orientation
                .as_deref()
                .map(|orientation| orientation.eq_ignore_ascii_case("center"))
                != Some(true)
            || slot.width <= 0
            || slot.height <= 0
            || template_size.width <= 0
            || template_size.height <= 0
            || control_parent_size.width <= 0
            || control_parent_size.height <= 0
        {
            return Err("XMAP13 active-ability grid/template geometry is unsupported".into());
        }
        let expected_x = grid_position[0]
            + instance_ordinal as f32 * slot.width as f32
            + control_position[0]
            + (control_parent_size.width as f32 - destination[2]) * 0.5;
        let expected_y = grid_position[1]
            + control_position[1]
            + (control_parent_size.height as f32 - destination[3]) * 0.5;
        if destination[0].to_bits() != expected_x.to_bits()
            || destination[1].to_bits() != expected_y.to_bits()
        {
            return Err(
                "XMAP13 active-ability instance position differs from its effective grid/template"
                    .into(),
            );
        }
        return Ok(());
    }
    xmap13_validate_oriented_control_position(control, destination)
}

fn xmap13_validate_oriented_control_position(
    control: &GuiControlLayout,
    rectangle: [f32; 4],
) -> Result<()> {
    let position = xmap13_effective_control_base(control)?;
    let mut expected_x = position[0];
    let mut expected_y = position[1];
    match control.orientation.as_deref() {
        None | Some("upper_left") => {}
        Some(orientation) if orientation.eq_ignore_ascii_case("upper_left") => {}
        Some(orientation) if orientation.eq_ignore_ascii_case("center") => {
            let parent_size = control
                .parent_size
                .ok_or("XMAP13 centered control lacks an effective parent size")?;
            if parent_size.width <= 0 || parent_size.height <= 0 {
                return Err("XMAP13 centered control has a nonpositive parent extent".into());
            }
            expected_x += (parent_size.width as f32 - rectangle[2]) * 0.5;
            expected_y += (parent_size.height as f32 - rectangle[3]) * 0.5;
        }
        Some(_) => return Err("XMAP13 drawable control orientation is unsupported".into()),
    }
    if rectangle[0].to_bits() != expected_x.to_bits()
        || rectangle[1].to_bits() != expected_y.to_bits()
    {
        return Err("XMAP13 drawable position differs from the effective GUI control".into());
    }
    Ok(())
}

fn xmap13_effective_control_base(control: &GuiControlLayout) -> Result<[f32; 2]> {
    if let Some(bits) = control.effective_position_f32_bits {
        return Ok([f32::from_bits(bits[0]), f32::from_bits(bits[1])]);
    }
    let position = control
        .position
        .ok_or("XMAP13 control lacks an effective base position")?;
    Ok([position.x as f32, position.y as f32])
}

fn xmap13_effective_ancestor_origin(control: &GuiControlLayout) -> Result<[f32; 2]> {
    let mut origin = xmap13_effective_control_base(control)?;
    match control.orientation.as_deref() {
        None | Some("upper_left") => {}
        Some(orientation) if orientation.eq_ignore_ascii_case("upper_left") => {}
        Some(orientation) if orientation.eq_ignore_ascii_case("center") => {
            let parent = control
                .parent_size
                .ok_or("XMAP13 centered runtime ancestor lacks a parent size")?;
            let extent = control
                .size
                .ok_or("XMAP13 centered runtime ancestor lacks an authored size")?;
            if parent.width <= 0 || parent.height <= 0 || extent.width <= 0 || extent.height <= 0 {
                return Err("XMAP13 centered runtime ancestor has a nonpositive extent".into());
            }
            origin[0] += (parent.width as f32 - extent.width as f32) * 0.5;
            origin[1] += (parent.height as f32 - extent.height as f32) * 0.5;
        }
        Some(_) => return Err("XMAP13 runtime ancestor orientation is unsupported".into()),
    }
    Ok(origin)
}

fn xmap13_validate_counter_primitive(
    adapter: &Xmap13Content,
    counter: &Xmap13CounterSemantic,
    manifest: &UnitCounterAssetManifest,
    item_kind: Xmap13CounterItemKind,
    control: &GuiControlLayout,
    primitive: &Xmap13CounterPrimitive,
    active_ability_context: Option<(&GuiControlLayout, &GuiContainerLayout, u32)>,
) -> Result<()> {
    match primitive {
        Xmap13CounterPrimitive::Hidden { .. } => Ok(()),
        Xmap13CounterPrimitive::Sprite {
            authored_sprite_name,
            binding,
            authored_frame_index,
            frame_selection,
            texture_content,
            source_rect_px,
            destination_rect_f32_bits,
            rotation_f32_bits,
            visible_fraction_f32_bits,
            ..
        } => {
            let control_sprite = control
                .sprite_type
                .as_deref()
                .or(control.quad_texture_sprite.as_deref())
                .ok_or("XMAP13 sprite primitive is bound to a non-sprite GUI control")?;
            match binding {
                Xmap13CounterSpriteBinding::AuthoredGui
                    if authored_sprite_name == control_sprite => {}
                Xmap13CounterSpriteBinding::RuntimeRebound {
                    authored_gui_sprite_name,
                } if authored_gui_sprite_name == control_sprite
                    && *frame_selection == Xmap13CounterFrameSelection::RuntimeSelected
                    && xmap13_runtime_sprite_rebind_is_admitted(
                        manifest,
                        item_kind,
                        &control.name,
                        authored_sprite_name,
                    ) => {}
                _ => {
                    return Err(
                        "XMAP13 selected sprite/GUI placeholder binding is not authenticated"
                            .into(),
                    )
                }
            }
            let rotation = f32::from_bits(*rotation_f32_bits);
            if rotation.to_bits() != control.rotation.unwrap_or(0.0).to_bits() {
                return Err("XMAP13 sprite rotation differs from the effective GUI control".into());
            }
            xmap13_validate_fractional_bar_rotation(rotation, visible_fraction_f32_bits.is_some())?;
            let destination = destination_rect_f32_bits.map(f32::from_bits);
            xmap13_validate_sprite_position(control, destination, active_ability_context)?;
            if let Some(asset) = xmap13_counter_sprite(manifest, authored_sprite_name) {
                let frame_index = authored_frame_index
                    .ok_or("XMAP13 file-textured sprite lacks its effective atlas frame")?;
                xmap13_validate_frame_selection(
                    control,
                    *frame_selection,
                    *authored_frame_index,
                    Some(asset.no_of_frames.max(1)),
                    false,
                    item_kind == Xmap13CounterItemKind::Air && control.name == "wing_type",
                )?;
                let frame = asset
                    .frame(frame_index)
                    .ok_or("XMAP13 sprite frame is outside the effective atlas")?;
                let exact_rect = [
                    frame.rect.x,
                    frame.rect.y,
                    frame.rect.width,
                    frame.rect.height,
                ];
                if *source_rect_px != exact_rect
                    || normalize(&texture_content.normalized_game_relative_path)
                        != normalize(&asset.texture_file)
                {
                    return Err("XMAP13 sprite texture/frame differs from effective GFX".into());
                }
                let expected_size = control
                    .size
                    .or(control.slot_size)
                    .map(|size| [size.width as f32, size.height as f32])
                    .unwrap_or_else(|| {
                        let scale = control.scale.unwrap_or(1.0);
                        if visible_fraction_f32_bits.is_some() {
                            [
                                frame.rect.height as f32 * scale,
                                frame.rect.width as f32 * scale,
                            ]
                        } else {
                            [
                                frame.rect.width as f32 * scale,
                                frame.rect.height as f32 * scale,
                            ]
                        }
                    });
                if destination[2].to_bits() != expected_size[0].to_bits()
                    || destination[3].to_bits() != expected_size[1].to_bits()
                {
                    return Err("XMAP13 sprite extent differs from effective GUI/GFX".into());
                }
                xmap13_require_counter_content(adapter, counter, &asset.source_file)?;
            } else {
                let (raw, raw_content) =
                    xmap13_raw_sprite_from_closure(adapter, counter, authored_sprite_name)?;
                match &raw.texture_file {
                    Some(texture_file) => {
                        let frame_index = authored_frame_index
                            .ok_or("XMAP13 file-textured sprite lacks its effective atlas frame")?;
                        xmap13_validate_frame_selection(
                            control,
                            *frame_selection,
                            *authored_frame_index,
                            Some(raw.no_of_frames.max(1)),
                            false,
                            item_kind == Xmap13CounterItemKind::Air && control.name == "wing_type",
                        )?;
                        if frame_index >= raw.no_of_frames.max(1)
                            || normalize(&texture_content.normalized_game_relative_path)
                                != normalize(texture_file)
                        {
                            return Err(
                                "XMAP13 sprite texture/frame differs from effective GFX".into()
                            );
                        }
                        let texture = crate::counter_texture_rgba_from_resolver(
                            &adapter.resolver,
                            texture_file,
                        )?;
                        let frame_count = raw.no_of_frames.max(1);
                        let (frame_width, frame_height, source_x, source_y) = if raw
                            .frame_direction_y
                        {
                            if texture.height % frame_count as usize != 0 {
                                return Err("XMAP13 vertical sprite atlas does not divide into authored frames".into());
                            }
                            let height = u32::try_from(texture.height)? / frame_count;
                            (
                                u32::try_from(texture.width)?,
                                height,
                                0,
                                frame_index * height,
                            )
                        } else {
                            if texture.width % frame_count as usize != 0 {
                                return Err("XMAP13 horizontal sprite atlas does not divide into authored frames".into());
                            }
                            let width = u32::try_from(texture.width)? / frame_count;
                            (
                                width,
                                u32::try_from(texture.height)?,
                                frame_index * width,
                                0,
                            )
                        };
                        if *source_rect_px != [source_x, source_y, frame_width, frame_height] {
                            return Err(
                                "XMAP13 sprite source rectangle differs from effective GFX atlas"
                                    .into(),
                            );
                        }
                        let expected_size = control
                            .size
                            .or(control.slot_size)
                            .map(|size| [size.width as f32, size.height as f32])
                            .unwrap_or_else(|| {
                                let scale = control.scale.unwrap_or(1.0);
                                if visible_fraction_f32_bits.is_some() {
                                    [frame_height as f32 * scale, frame_width as f32 * scale]
                                } else {
                                    [frame_width as f32 * scale, frame_height as f32 * scale]
                                }
                            });
                        if destination[2].to_bits() != expected_size[0].to_bits()
                            || destination[3].to_bits() != expected_size[1].to_bits()
                        {
                            return Err(
                                "XMAP13 sprite extent differs from effective GUI/GFX".into()
                            );
                        }
                    }
                    None => {
                        if xmap13_validate_frame_selection(
                            control,
                            *frame_selection,
                            *authored_frame_index,
                            None,
                            true,
                            false,
                        )
                        .is_err()
                        {
                            return Err(
                                "XMAP13 code-textured sprite has an invalid authored frame/binding"
                                    .into(),
                            );
                        }
                        let texture = crate::counter_texture_rgba_from_resolver(
                            &adapter.resolver,
                            &texture_content.normalized_game_relative_path,
                        )?;
                        let texture_width = u32::try_from(texture.width)?;
                        let texture_height = u32::try_from(texture.height)?;
                        if source_rect_px[0]
                            .checked_add(source_rect_px[2])
                            .is_none_or(|end| end > texture_width)
                            || source_rect_px[1]
                                .checked_add(source_rect_px[3])
                                .is_none_or(|end| end > texture_height)
                        {
                            return Err("XMAP13 code-textured sprite rectangle exceeds its admitted runtime texture".into());
                        }
                        let expected_size = control
                            .size
                            .or(control.slot_size)
                            .map(|size| [size.width as f32, size.height as f32])
                            .unwrap_or_else(|| {
                                let scale = control.scale.unwrap_or(1.0);
                                [
                                    source_rect_px[2] as f32 * scale,
                                    source_rect_px[3] as f32 * scale,
                                ]
                            });
                        if destination[2].to_bits() != expected_size[0].to_bits()
                            || destination[3].to_bits() != expected_size[1].to_bits()
                        {
                            return Err("XMAP13 code-textured sprite extent differs from effective GUI/runtime texture".into());
                        }
                    }
                }
                adapter.resolver.read_admitted_used_content(&raw_content)?;
            }
            if xmap13_counter_content(counter, &texture_content.normalized_game_relative_path)
                != Some(texture_content)
            {
                return Err("XMAP13 runtime sprite texture is absent from admitted closure".into());
            }
            adapter
                .resolver
                .read_admitted_used_content(texture_content)?;
            Ok(())
        }
        Xmap13CounterPrimitive::Text {
            font_name,
            ordered_font_content,
            control_rect_f32_bits,
            horizontal_alignment,
            vertical_alignment,
            ..
        } => {
            if control.font.as_deref() != Some(font_name.as_str()) {
                return Err("XMAP13 text font differs from the effective GUI control".into());
            }
            let expected_horizontal =
                xmap13_horizontal_alignment_from_gui_format(control.format.as_deref());
            if *horizontal_alignment != expected_horizontal
                || *vertical_alignment != Xmap13CounterVerticalAlignment::Top
            {
                return Err(
                    "XMAP13 text alignment differs from the proved GUI/font placement law".into(),
                );
            }
            let rect = control_rect_f32_bits.map(f32::from_bits);
            xmap13_validate_oriented_control_position(control, rect)?;
            let expected_width = control
                .max_width
                .map(|value| value as f32)
                .or_else(|| control.size.map(|size| size.width as f32));
            let expected_height = control
                .max_height
                .map(|value| value as f32)
                .or_else(|| control.size.map(|size| size.height as f32));
            if expected_width.is_some_and(|width| rect[2].to_bits() != width.to_bits())
                || expected_height.is_some_and(|height| rect[3].to_bits() != height.to_bits())
            {
                return Err("XMAP13 text extent differs from the effective GUI control".into());
            }
            let font = manifest
                .fonts
                .iter()
                .find(|font| font.name == *font_name)
                .ok_or("XMAP13 text font is absent from effective core.gfx")?;
            xmap13_require_counter_content(adapter, counter, UNIT_COUNTER_CORE_GFX_SOURCE)?;
            let font_stems = font
                .path
                .iter()
                .chain(font.font_files.iter())
                .map(|path| normalize(path))
                .collect::<BTreeSet<_>>();
            let mut saw_descriptor = false;
            let mut saw_atlas = false;
            for content in ordered_font_content {
                if xmap13_counter_content(counter, &content.normalized_game_relative_path)
                    != Some(content)
                {
                    return Err(
                        "XMAP13 font file is absent from the admitted counter closure".into(),
                    );
                }
                adapter.resolver.read_admitted_used_content(content)?;
                let path = normalize(&content.normalized_game_relative_path);
                saw_descriptor |= path.ends_with(".fnt")
                    && font_stems.iter().any(|stem| path == format!("{stem}.fnt"));
                saw_atlas |= path.ends_with(".dds") || path.ends_with(".tga");
            }
            if !saw_descriptor || !saw_atlas {
                return Err("XMAP13 font closure lacks the exact descriptor or atlas page".into());
            }
            Ok(())
        }
    }
}

fn xmap13_horizontal_alignment_from_gui_format(
    format: Option<&str>,
) -> Xmap13CounterHorizontalAlignment {
    match format
        .unwrap_or("left")
        .trim_matches('"')
        .to_ascii_lowercase()
        .as_str()
    {
        "center" | "centre" => Xmap13CounterHorizontalAlignment::Center,
        "right" => Xmap13CounterHorizontalAlignment::Right,
        _ => Xmap13CounterHorizontalAlignment::Left,
    }
}

fn xmap13_raw_sprite_from_closure(
    adapter: &Xmap13Content,
    counter: &Xmap13CounterSemantic,
    name: &str,
) -> Result<(crate::RawSpriteDef, UsedContentKey)> {
    let mut matches = Vec::new();
    for content in &counter.ordered_interface_content {
        if !normalize(&content.normalized_game_relative_path).ends_with(".gfx") {
            continue;
        }
        let text = latin1(&adapter.resolver.read_admitted_used_content(content)?);
        for definition in crate::unit_counter_assets::parse_sprite_defs_exact(
            &text,
            &content.normalized_game_relative_path,
        )? {
            if definition.name == name {
                matches.push((definition, content.clone()));
            }
        }
    }
    if matches.len() != 1 {
        return Err(format!(
            "XMAP13 sprite {name} has {} effective GFX definitions in its admitted closure",
            matches.len()
        )
        .into());
    }
    Ok(matches.pop().expect("single XMAP13 sprite definition"))
}

fn texture_basename(value: &str) -> String {
    normalize(value)
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_string()
}

fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| char::from(*byte)).collect()
}

fn selected_entity_state_block(text: &str, entity_name: &str, state_name: &str) -> Option<String> {
    let mut entity_cursor = 0;
    while let Some((entity, next_entity)) =
        crate::extract_block_after(text, "entity", entity_cursor)
    {
        entity_cursor = next_entity;
        if !contains_assignment_token(&entity, "name", entity_name) {
            continue;
        }
        let mut state_cursor = 0;
        while let Some((state, next_state)) =
            crate::extract_block_after(&entity, "state", state_cursor)
        {
            state_cursor = next_state;
            if contains_assignment_token(&state, "name", state_name) {
                return Some(state);
            }
        }
        return None;
    }
    None
}

fn ordered_assignment_blocks(text: &str, key: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut cursor = 0;
    while let Some((block, next)) = crate::extract_block_after(text, key, cursor) {
        blocks.push(block);
        cursor = next;
    }
    blocks
}

fn authored_effect_invocations(block: &str) -> Vec<(Xmap13AuthoredEffectKind, String)> {
    let mut invocations = Vec::new();
    for (kind, key) in [
        (Xmap13AuthoredEffectKind::Particle, "particle"),
        (Xmap13AuthoredEffectKind::Sound, "soundeffect"),
    ] {
        let mut cursor = 0;
        while let Some(value_start) = crate::find_assignment(block, key, cursor) {
            let Some(token) = crate::parse_token_at(&block[value_start..]) else {
                break;
            };
            let token = token.trim_matches('"').to_string();
            invocations.push((value_start, kind, token));
            cursor = value_start.saturating_add(1);
        }
    }
    invocations.sort_by_key(|row| row.0);
    invocations
        .into_iter()
        .map(|(_, kind, token)| (kind, token))
        .collect()
}

fn validate_authored_event_presence(block: &str, event: &Xmap13AuthoredStateEvent) -> Result<()> {
    let authored_time = crate::parse_f32_value(block, "time").map(f32::to_bits);
    if authored_time != event.event_time_f32_bits {
        return Err("XMAP13 authored event time source-presence/value drifted".into());
    }
    for (key, carried) in [
        ("keep_particle", event.keep_particle),
        ("trigger_once", event.trigger_once),
    ] {
        let authored = match crate::parse_word_value(block, key).as_deref() {
            None => None,
            Some("yes") => Some(true),
            Some("no") => Some(false),
            Some(_) => return Err(format!("XMAP13 authored event {key} is not yes/no").into()),
        };
        if authored != carried {
            return Err(
                format!("XMAP13 authored event {key} source presence/value drifted").into(),
            );
        }
    }
    Ok(())
}

fn contains_assignment_token(text: &str, key: &str, value: &str) -> bool {
    let compact = text
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    compact.contains(&format!("{key}=\"{value}\"")) || compact.contains(&format!("{key}={value}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_content_manifest::ContentLayerKind;

    fn effect_content(path: &str, fill: char) -> UsedContentKey {
        UsedContentKey {
            effective_vfs_manifest_sha256: "a".repeat(64),
            normalized_game_relative_path: path.into(),
            winning_layer_ordinal: 0,
            layer_kind: ContentLayerKind::Base,
            layer_id: "base".into(),
            source_sha256: fill.to_string().repeat(64),
            payload_sha256: fill.to_string().repeat(64),
            payload_bytes: 64,
        }
    }

    fn gap(input_id: &str) -> crate::MapAuthorityGap {
        crate::MapAuthorityGap {
            input_id: input_id.into(),
            disposition: crate::MapAuthorityRequirementDisposition::Unavailable,
            reason: "fixture blocker".into(),
        }
    }

    fn admitted(input_id: &str) -> crate::MapAuthorityAdmittedInput {
        let proven = crate::map_authority_owner_proven_admission(input_id)
            .unwrap_or_else(|| panic!("{input_id} is not an owner-proven map-authority input"));
        crate::MapAuthorityAdmittedInput {
            input_id: input_id.into(),
            owner_packet: proven.owner_packet.into(),
            payload_schema: proven.payload_schema.into(),
            cadence: proven.cadence.into(),
            identity_order: proven.identity_order.into(),
        }
    }

    /// kills: reverting XMAP13 to the gap-only accounting it used before the
    /// four `units.*` rows became owner-proven. That form returned `Err` for any
    /// owned input that was not a gap, so proving the rows would have aborted
    /// the whole foundation graph instead of leaving XMAP13 typed-unavailable.
    #[test]
    fn the_four_proven_unit_rows_are_accepted_as_admitted_not_only_as_gaps() {
        let hierarchy = "orders.theatre_army_hierarchy";
        let proven = ["units.divisions", "units.taskforces_fleets", "units.wings", "units.air_base_location"];
        for input_id in proven {
            assert!(
                crate::map_authority_owner_proven_admission(input_id).is_some(),
                "{input_id} must be owner-proven for this packet's accounting to hold"
            );
        }

        // The configuration this change actually creates: the four typed unit
        // rows admitted, the hierarchy row still a gap.
        let admitted_rows: Vec<_> = proven.iter().map(|id| admitted(id)).collect();
        assert_eq!(
            crate::account_owned_map_authority_inputs(
                "XMAP13",
                &OWNED_GAPS,
                &[gap(hierarchy)],
                &admitted_rows,
            )
            .unwrap(),
            vec![hierarchy.to_string()],
            "only the still-gapped input may be reported as consumed"
        );

        // All five gapped: every one is consumed, as before this change.
        let all_gaps: Vec<_> = OWNED_GAPS.iter().map(|id| gap(id)).collect();
        assert_eq!(
            crate::account_owned_map_authority_inputs("XMAP13", &OWNED_GAPS, &all_gaps, &[])
                .unwrap(),
            OWNED_GAPS.iter().map(|id| id.to_string()).collect::<Vec<_>>()
        );

        // An owned input the projection never mentioned is still a refusal.
        assert!(
            crate::account_owned_map_authority_inputs(
                "XMAP13",
                &OWNED_GAPS,
                &[],
                &admitted_rows,
            )
            .unwrap_err()
            .contains("omitted owned input orders.theatre_army_hierarchy")
        );

        // And one reported in both states is a contradiction, not a preference.
        assert!(crate::account_owned_map_authority_inputs(
            "XMAP13",
            &OWNED_GAPS,
            &[gap("units.divisions"), gap(hierarchy)],
            &admitted_rows,
        )
        .unwrap_err()
        .contains("both admitted and gapped owned input units.divisions"));
    }

    /// kills: XMAP13 growing a second copy of the tier law. There is one
    /// implementation, in `xmap13_counter_grouping_law.rs::grouping_tier`, and
    /// this pins the boundaries the packet depends on through it.
    #[test]
    fn grouping_tier_uses_exact_current_eye_scalar_boundaries() {
        use crate::xmap13_counter_grouping_law::grouping_tier;
        let thresholds = [0.0009, 0.0018, 0.0035];
        assert_eq!(grouping_tier(0.000899, thresholds).unwrap(), 0);
        assert_eq!(grouping_tier(0.0009, thresholds).unwrap(), 1);
        assert_eq!(grouping_tier(0.001799, thresholds).unwrap(), 1);
        assert_eq!(grouping_tier(0.0018, thresholds).unwrap(), 2);
        assert_eq!(grouping_tier(0.003499, thresholds).unwrap(), 2);
        assert_eq!(grouping_tier(0.0035, thresholds).unwrap(), 3);
        assert!(grouping_tier(f32::NAN, thresholds).is_err());
    }

    #[test]
    fn effective_gui_text_format_controls_only_the_horizontal_axis() {
        assert_eq!(
            xmap13_horizontal_alignment_from_gui_format(Some("center")),
            Xmap13CounterHorizontalAlignment::Center,
        );
        assert_eq!(
            xmap13_horizontal_alignment_from_gui_format(Some("UPPER_LEFT")),
            Xmap13CounterHorizontalAlignment::Left,
        );
        assert_eq!(
            xmap13_horizontal_alignment_from_gui_format(Some("diagonal")),
            Xmap13CounterHorizontalAlignment::Left,
        );
    }

    #[test]
    fn selected_group_identity_retains_allegiance() {
        assert_ne!(
            xmap13_selected_group_key(Xmap13CounterAllegiance::Ours, "opaque-group"),
            xmap13_selected_group_key(Xmap13CounterAllegiance::Enemy, "opaque-group"),
        );
    }

    #[test]
    fn frame_source_enforces_fixed_default_runtime_and_code_texture_laws() {
        let mut control = GuiControlLayout {
            kind: "iconType".into(),
            name: "fixed".into(),
            source_file: "fixture.gui".into(),
            source_line: 1,
            sprite_type: Some("GFX_fixture".into()),
            quad_texture_sprite: None,
            parent_size: None,
            position: None,
            effective_position_f32_bits: None,
            size: None,
            slot_size: None,
            max_width: None,
            max_height: None,
            font: None,
            format: None,
            orientation: None,
            max_slots_vertical: None,
            rotation: None,
            scale: None,
            frame: Some(2),
            center_position: None,
        };
        assert!(xmap13_validate_frame_selection(
            &control,
            Xmap13CounterFrameSelection::AuthoredFixed,
            Some(1),
            Some(3),
            false,
            false,
        )
        .is_ok());
        assert!(xmap13_validate_frame_selection(
            &control,
            Xmap13CounterFrameSelection::AuthoredFixed,
            Some(0),
            Some(3),
            false,
            false,
        )
        .is_err());
        assert!(xmap13_validate_frame_selection(
            &control,
            Xmap13CounterFrameSelection::RuntimeSelected,
            Some(1),
            Some(3),
            false,
            false,
        )
        .is_err());

        control.frame = None;
        assert!(xmap13_validate_frame_selection(
            &control,
            Xmap13CounterFrameSelection::AuthoredDefaultFirst,
            Some(0),
            Some(3),
            false,
            false,
        )
        .is_ok());
        assert!(xmap13_validate_frame_selection(
            &control,
            Xmap13CounterFrameSelection::AuthoredDefaultFirst,
            Some(1),
            Some(3),
            false,
            false,
        )
        .is_err());
        assert!(xmap13_validate_frame_selection(
            &control,
            Xmap13CounterFrameSelection::RuntimeSelected,
            Some(1),
            Some(3),
            false,
            false,
        )
        .is_ok());
        assert!(xmap13_validate_frame_selection(
            &control,
            Xmap13CounterFrameSelection::RuntimeSelected,
            None,
            None,
            true,
            false,
        )
        .is_ok());

        control.frame = Some(1);
        assert!(xmap13_validate_frame_selection(
            &control,
            Xmap13CounterFrameSelection::RuntimeSelectedOverAuthoredInitial,
            Some(47),
            Some(48),
            false,
            true,
        )
        .is_ok());
        assert!(xmap13_validate_frame_selection(
            &control,
            Xmap13CounterFrameSelection::RuntimeSelectedOverAuthoredInitial,
            Some(47),
            Some(48),
            false,
            false,
        )
        .is_err());
    }

    #[test]
    fn active_ability_template_refuses_uncensused_drawable_children() {
        let active_ability = GuiControlLayout {
            kind: "iconType".into(),
            name: "active_ability".into(),
            source_file: "fixture.gui".into(),
            source_line: 2,
            sprite_type: Some("GFX_active_ability".into()),
            quad_texture_sprite: None,
            parent_size: Some(crate::GuiSize {
                width: 20,
                height: 20,
            }),
            position: None,
            effective_position_f32_bits: None,
            size: None,
            slot_size: None,
            max_width: None,
            max_height: None,
            font: None,
            format: None,
            orientation: Some("center".into()),
            max_slots_vertical: None,
            rotation: None,
            scale: Some(0.8),
            frame: None,
            center_position: None,
        };
        let mut template = GuiContainerLayout {
            name: "active_ability_item".into(),
            source_file: "fixture.gui".into(),
            source_line: 1,
            position: None,
            size: Some(crate::GuiSize {
                width: 20,
                height: 20,
            }),
            controls: vec![active_ability.clone()],
        };
        assert!(xmap13_active_ability_template_control(&template).is_ok());
        let mut structure = active_ability.clone();
        structure.kind = "positionType".into();
        structure.name = "harmless_structure".into();
        structure.sprite_type = None;
        structure.orientation = None;
        template.controls.push(structure);
        assert!(xmap13_active_ability_template_control(&template).is_ok());
        let mut extra = active_ability;
        extra.name = "modded_extra_drawable".into();
        template.controls.push(extra);
        assert!(xmap13_active_ability_template_control(&template).is_err());
    }

    #[test]
    fn centered_nested_control_uses_parent_extent_and_accumulated_base() {
        let control = GuiControlLayout {
            kind: "iconType".into(),
            name: "centered".into(),
            source_file: "fixture.gui".into(),
            source_line: 1,
            sprite_type: Some("GFX_centered".into()),
            quad_texture_sprite: None,
            parent_size: Some(crate::GuiSize {
                width: 20,
                height: 20,
            }),
            position: Some(crate::GuiPoint { x: 10, y: 20 }),
            effective_position_f32_bits: None,
            size: None,
            slot_size: None,
            max_width: None,
            max_height: None,
            font: None,
            format: None,
            orientation: Some("center".into()),
            max_slots_vertical: None,
            rotation: None,
            scale: None,
            frame: None,
            center_position: None,
        };
        assert!(
            xmap13_validate_oriented_control_position(&control, [15.0, 25.0, 10.0, 10.0],).is_ok()
        );
        assert!(
            xmap13_validate_oriented_control_position(&control, [10.0, 20.0, 10.0, 10.0],).is_err()
        );

        let mut runtime_ancestor = control;
        runtime_ancestor.kind = "gridBoxType".into();
        runtime_ancestor.size = Some(crate::GuiSize {
            width: 10,
            height: 10,
        });
        assert_eq!(
            xmap13_effective_ancestor_origin(&runtime_ancestor).expect("centered grid origin"),
            [15.0, 25.0]
        );
        runtime_ancestor.orientation = Some("lower_right".into());
        assert!(xmap13_effective_ancestor_origin(&runtime_ancestor).is_err());
        runtime_ancestor.orientation = Some("center".into());
        runtime_ancestor.size = None;
        assert!(xmap13_effective_ancestor_origin(&runtime_ancestor).is_err());
    }

    #[test]
    fn unnamed_effective_controls_resolve_in_exact_painter_order() {
        let control = |name: &str| GuiControlLayout {
            kind: "background".into(),
            name: name.into(),
            source_file: "fixture.gui".into(),
            source_line: 1,
            sprite_type: None,
            quad_texture_sprite: Some("GFX_fixture".into()),
            parent_size: None,
            position: None,
            effective_position_f32_bits: None,
            size: None,
            slot_size: None,
            max_width: None,
            max_height: None,
            font: None,
            format: None,
            orientation: None,
            max_slots_vertical: None,
            rotation: None,
            scale: None,
            frame: None,
            center_position: None,
        };
        let first = control("");
        let named = control("named");
        let second = control("");
        let drawables = vec![(2, &first), (3, &named), (4, &second)];
        let named_ordinals = BTreeMap::from([("named", 3)]);
        let mut covered = BTreeSet::new();
        assert_eq!(
            xmap13_resolve_control_ordinal(&drawables, &named_ordinals, &covered, "")
                .expect("first unnamed"),
            2
        );
        covered.insert(2);
        assert_eq!(
            xmap13_resolve_control_ordinal(&drawables, &named_ordinals, &covered, "")
                .expect("second unnamed"),
            4
        );
        assert_eq!(
            xmap13_resolve_control_ordinal(&drawables, &named_ordinals, &covered, "named")
                .expect("named"),
            3
        );
        covered.insert(4);
        assert!(xmap13_resolve_control_ordinal(&drawables, &named_ordinals, &covered, "").is_err());
    }

    #[test]
    fn fractional_bar_rotation_is_exact_or_rejected() {
        assert!(xmap13_validate_fractional_bar_rotation(1.5708, true).is_ok());
        assert!(xmap13_validate_fractional_bar_rotation(1.5707, true).is_err());
        assert!(xmap13_validate_fractional_bar_rotation(0.0, false).is_ok());
    }

    #[test]
    fn source_membership_audit_requires_complete_group_and_item_ordinals() {
        let key = (0, Xmap13CounterAllegiance::Ours, "group".to_string());
        let mut groups = BTreeMap::new();
        groups.insert(
            key,
            Xmap13MembershipGroupAudit {
                declared_member_count: Some(2),
                declared_item_count: Some(1),
                member_ordinals: BTreeSet::from([0]),
                items: BTreeMap::from([(
                    0,
                    Xmap13MembershipItemAudit {
                        stable_item_id: "item".into(),
                        kind: Some(Xmap13CounterItemKind::Land),
                        declared_member_count: Some(2),
                        member_ordinals: BTreeSet::from([0]),
                    },
                )]),
            },
        );
        assert!(validate_membership_group_audits(&groups).is_err());
        let group = groups.values_mut().next().unwrap();
        group.member_ordinals.insert(1);
        group.items.get_mut(&0).unwrap().member_ordinals.insert(1);
        validate_membership_group_audits(&groups).expect("complete source census");
    }

    #[test]
    fn counter_runtime_presentation_rejects_non_finite_or_unbounded_modulation() {
        let texture = effect_content("gfx/interface/counter.dds", '2');
        let mut counter = Xmap13CounterSemantic {
            counter_kind: "land".into(),
            ordered_interface_content: vec![
                effect_content("interface/mapicons.gui", '1'),
                texture.clone(),
            ],
            ordered_group_memberships: (0..4)
                .flat_map(|tier| {
                    [
                        Xmap13CounterAllegiance::WorldObserver,
                        Xmap13CounterAllegiance::Ours,
                        Xmap13CounterAllegiance::Enemy,
                        Xmap13CounterAllegiance::Other,
                    ]
                    .into_iter()
                    .map(move |allegiance| Xmap13CounterGroupMembership {
                        grouping_tier: tier,
                        allegiance,
                        stable_group_id: format!("group-{tier}-{allegiance:?}"),
                        member_world_anchor_f32_bits: [0.0f32.to_bits(); 3],
                        member_ordinal: 0,
                        member_count: 1,
                        counter_item_id: format!("item-{tier}-{allegiance:?}"),
                        counter_item_kind: Xmap13CounterItemKind::Land,
                        counter_item_ordinal: 0,
                        counter_item_count: 1,
                        counter_item_member_ordinal: 0,
                        counter_item_member_count: 1,
                    })
                })
                .collect(),
            ordered_presentations: (0..4)
                .flat_map(|tier| {
                    [
                        Xmap13CounterAllegiance::WorldObserver,
                        Xmap13CounterAllegiance::Ours,
                        Xmap13CounterAllegiance::Enemy,
                        Xmap13CounterAllegiance::Other,
                    ]
                    .into_iter()
                    .flat_map({
                        let texture = texture.clone();
                        move |allegiance| {
                            [false, true].into_iter().map({
                                let texture = texture.clone();
                                move |selected| Xmap13CounterPresentation {
                                    grouping_tier: tier,
                                    counter_item_id: format!("item-{tier}-{allegiance:?}"),
                                    allegiance,
                                    selected,
                                    anchor_offset_px_f32_bits: [0.0f32.to_bits(); 2],
                                    stack_advance_px_f32_bits: 28.0f32.to_bits(),
                                    active_ability_instance_count: 0,
                                    ordered_primitives: vec![Xmap13CounterPrimitive::Sprite {
                                        stable_layer_id: "backing".into(),
                                        authored_control_name: "counter_bg".into(),
                                        authored_sprite_name: "GFX_onmap_unit_counter".into(),
                                        binding: Xmap13CounterSpriteBinding::AuthoredGui,
                                        authored_frame_index: Some(0),
                                        frame_selection:
                                            Xmap13CounterFrameSelection::AuthoredDefaultFirst,
                                        runtime_collection_instance_ordinal: None,
                                        texture_content: texture.clone(),
                                        source_rect_px: [0, 0, 10, 10],
                                        destination_rect_f32_bits: [
                                            0.0f32.to_bits(),
                                            0.0f32.to_bits(),
                                            10.0f32.to_bits(),
                                            10.0f32.to_bits(),
                                        ],
                                        rotation_f32_bits: 0.0f32.to_bits(),
                                        visible_fraction_f32_bits: None,
                                        modulation_rgba_f32_bits: [1.0f32.to_bits(); 4],
                                    }],
                                }
                            })
                        }
                    })
                })
                .collect(),
        };
        validate_counter(Some(&counter)).expect("exact runtime presentation");
        counter.ordered_presentations[0].active_ability_instance_count = 1;
        assert!(validate_counter(Some(&counter)).is_err());
        counter.ordered_presentations[0].active_ability_instance_count = 0;
        match &mut counter.ordered_presentations[0].ordered_primitives[0] {
            Xmap13CounterPrimitive::Sprite {
                modulation_rgba_f32_bits,
                ..
            } => modulation_rgba_f32_bits[0] = f32::NAN.to_bits(),
            _ => panic!("sprite primitive"),
        }
        assert!(validate_counter(Some(&counter)).is_err());
        match &mut counter.ordered_presentations[0].ordered_primitives[0] {
            Xmap13CounterPrimitive::Sprite {
                modulation_rgba_f32_bits,
                ..
            } => modulation_rgba_f32_bits[0] = 1.1f32.to_bits(),
            _ => panic!("sprite primitive"),
        }
        assert!(validate_counter(Some(&counter)).is_err());
    }

    fn target_identity() -> Xmap13TargetExecutionIdentity {
        Xmap13TargetExecutionIdentity {
            hour_index: 17,
            content_manifest_sha256: "a".repeat(64),
            foundation_content_operation_sha256: "b".repeat(64),
            viewer_state_sha256: "c".repeat(64),
            main_scene_target_sha256: "d".repeat(64),
            counter_target_sha256: Some("6".repeat(64)),
            map_copy_receipt_set_sha256: "e".repeat(64),
        }
    }

    fn component_receipt(draw: Xmap13NativeDrawClosure) -> Xmap13NativeComponentReceipt {
        Xmap13NativeComponentReceipt {
            schema: XMAP13_NATIVE_RECEIPT_SCHEMA.into(),
            component: Xmap13NativeComponent::UnitEntities,
            target_execution_identity: target_identity(),
            assigned_topology_sha256: "f".repeat(64),
            assigned_stage_id: "map.units.entities".into(),
            assigned_operation_id: "draw".into(),
            assigned_ordinal: 7,
            assigned_assignment_sha256: "1".repeat(64),
            prepared_frame_sha256: "2".repeat(64),
            draw,
            packet_draw_receipt_sha256: String::new(),
        }
    }

    #[test]
    fn typed_identity_is_canonical_and_unambiguous() {
        assert_eq!(
            Xmap13UnitIdentity::Division {
                type_id: 7,
                stable_id: 19
            }
            .canonical_id(),
            "xmap13:division:7:19"
        );
        assert_eq!(
            Xmap13UnitIdentity::TaskForce {
                type_id: 13,
                stable_id: 2
            }
            .canonical_id(),
            "xmap13:task_force:13:2"
        );
        assert_eq!(
            Xmap13UnitIdentity::AirWing {
                stable_id: "wing:GER:4".into()
            }
            .canonical_id(),
            "xmap13:air_wing:wing:GER:4"
        );
    }

    #[test]
    fn formation_identity_join_is_exact_for_unassigned_plan_owners() {
        let provider = Xmap13IdentityProvider {
            hour_index: 17,
            ordered_units: Vec::new(),
            ordered_formations: vec![
                Xmap13FormationIdentity {
                    owner_is_army_group: false,
                    owner_id: 41,
                    owner_country: "GER".into(),
                },
                Xmap13FormationIdentity {
                    owner_is_army_group: true,
                    owner_id: 7,
                    owner_country: "GER".into(),
                },
            ],
            census_sha256: "a".repeat(64),
            formation_census_sha256: "b".repeat(64),
            admission_sha256: "c".repeat(64),
        };
        assert_eq!(provider.formation_identity_match_count(false, 41, "GER"), 1);
        assert_eq!(provider.formation_identity_match_count(true, 7, "GER"), 1);
        assert_eq!(provider.formation_identity_match_count(false, 7, "GER"), 0);
        assert_eq!(provider.formation_identity_match_count(true, 7, "ITA"), 0);
    }

    #[test]
    fn matrix_transform_uses_engine_row_vector_layout() {
        let mut matrix = [0.0f32; 16];
        matrix[0] = 1.0;
        matrix[5] = 1.0;
        matrix[10] = 1.0;
        matrix[15] = 1.0;
        matrix[12] = 5.0;
        matrix[13] = 7.0;
        matrix[14] = 11.0;
        assert_eq!(transform_point([1.0, 2.0, 3.0], matrix), [6.0, 9.0, 14.0]);
    }

    #[test]
    fn semantic_envelope_seal_authenticates_order_and_payload() {
        let envelope = Xmap13SemanticEnvelope {
            schema: XMAP13_INPUT_SCHEMA.into(),
            hour_index: 17,
            cadence: "every_world_frame".into(),
            identity_order: "engine_enumerator_order".into(),
            complete_division_census: true,
            complete_task_force_census: true,
            complete_air_wing_census: true,
            complete_formation_identity_census: true,
            producer_executable_sha256: XMAP13_ENGINE_SHA256.into(),
            producer_evidence_manifest_sha256: "a".repeat(64),
            ordered_identity_sha256: String::new(),
            ordered_formation_identity_sha256: String::new(),
            rows: Vec::new(),
            ordered_formations: Vec::new(),
            deterministic_sha256: String::new(),
        }
        .seal()
        .expect("complete empty census seals");
        envelope.validate_for_hour(17).expect("sealed envelope");
        let mut tampered = envelope;
        tampered.hour_index = 18;
        assert!(tampered.validate_for_hour(18).is_err());
    }

    #[test]
    fn effect_identity_receipt_and_trigger_are_exact_and_tamper_evident() {
        let semantic_source_id = "division:12".to_string();
        let receipt = Xmap13EffectIdentityReceipt {
            schema: XMAP13_EFFECT_IDENTITY_RECEIPT_SCHEMA.into(),
            hour_index: 17,
            semantic_envelope_sha256: "b".repeat(64),
            producer_evidence_manifest_sha256: "c".repeat(64),
            ordered_identity_sha256: "d".repeat(64),
            ordered_semantic_source_ids: vec![semantic_source_id.clone()],
            ordered_semantic_source_ids_sha256: semantic_source_id_digest(&[
                semantic_source_id.clone()
            ]),
            admitted_content_sha256: "e".repeat(64),
            deterministic_sha256: String::new(),
        }
        .seal()
        .expect("identity receipt seals");
        let dependency = receipt
            .xmap20_dependency_receipt()
            .expect("XMAP20 dependency receipt");
        assert_eq!(dependency.owner_packet, XMAP20_ENTITY_IDENTITY_OWNER);
        assert_eq!(dependency.execution_sha256, receipt.deterministic_sha256);

        let entity_asset = effect_content("gfx/entities/units.asset", 'f');
        let row = Xmap13SemanticRow {
            identity: Xmap13UnitIdentity::Division {
                type_id: 1,
                stable_id: 12,
            },
            semantic_source_id: semantic_source_id.clone(),
            entity_instance_id: 12,
            owner_country: "GER".into(),
            anchor_province_id: 42,
            entity_asset: entity_asset.clone(),
            model_registry: effect_content("gfx/models/units.gfx", '1'),
            mesh: effect_content("gfx/models/units.mesh", '2'),
            shader_source: effect_content("gfx/fx/pdxmesh.shader", '3'),
            animation_asset: None,
            ordered_texture_content: Vec::new(),
            entity_name: "GER_infantry".into(),
            model_name: "infantry".into(),
            state_name: "idle".into(),
            material_variant: "default".into(),
            animation_name: None,
            animation_phase_f32_bits: 0.0f32.to_bits(),
            animation_speed_f32_bits: 1.0f32.to_bits(),
            selected_lod_index: 0,
            selected_shape_indices: Vec::new(),
            groups: Vec::new(),
            world_matrix_f32_bits: [0; 16],
            skinned: false,
            skin_palette_sha256: None,
            skin_palette_3200_bytes: None,
            casts_light_depth: true,
            authored_event_block_count: 1,
            authored_state_events: vec![Xmap13AuthoredStateEvent {
                authored_event_index: 4,
                authored_effect_index: 0,
                effect_kind: Xmap13AuthoredEffectKind::Particle,
                effect_token: "mg_muzzle_smoke".into(),
                node: Some("muzzle".into()),
                event_time_f32_bits: Some(0.0f32.to_bits()),
                keep_particle: Some(false),
                trigger_once: Some(true),
            }],
            counter: None,
            task_force_members: None,
        };
        let provider = Xmap13EffectIdentityProvider {
            receipt: receipt.clone(),
            ordered_rows: vec![row],
        };
        let effect = Xmap20ActiveEffect {
            effect_instance_id: 7,
            registration_token: "mg_muzzle_smoke".into(),
            registration_hash_u32: 1,
            resolved_definition_sha256: "4".repeat(64),
            producer_kind: "unit".into(),
            producer_instance_id: semantic_source_id,
            authored_event_index: 4,
            authored_effect_index: 0,
            parent_anchor: ParentAnchor::Entity {
                entity_instance_id: 12,
                node_token: "muzzle".into(),
            },
            initial_rng_seed_u32: 2,
            initial_rng_counter_u32: 3,
            elapsed_f32_bits: 1.0f32.to_bits(),
            time_scale_f32_bits: 1.0f32.to_bits(),
            active_flags_u32: 1,
            last_operation: portable_render_stream::EffectOperation::Activate,
            last_sequence: 5,
            last_update_index: 6,
            ordered_packet_keys: Vec::new(),
        };
        let trigger = provider
            .trigger_for_effect(&effect)
            .expect("exact authored trigger joins");
        assert_eq!(trigger.source_content, entity_asset);
        assert_eq!(trigger.owner_admission_sha256, receipt.deterministic_sha256);

        let mut wrong_entity = effect.clone();
        wrong_entity.parent_anchor = ParentAnchor::Entity {
            entity_instance_id: 13,
            node_token: "muzzle".into(),
        };
        assert!(provider.trigger_for_effect(&wrong_entity).is_err());

        let mut wrong_kind = effect.clone();
        wrong_kind.producer_kind = "weather".into();
        assert!(provider.trigger_for_effect(&wrong_kind).is_err());

        let mut wrong_node = effect;
        wrong_node.parent_anchor = ParentAnchor::Entity {
            entity_instance_id: 12,
            node_token: "root".into(),
        };
        assert!(provider.trigger_for_effect(&wrong_node).is_err());

        let mut tampered = receipt;
        tampered
            .ordered_semantic_source_ids
            .push("division:13".into());
        assert!(tampered.validate().is_err());
    }

    #[test]
    fn native_component_receipt_seals_present_and_authenticated_empty() {
        let present = component_receipt(Xmap13NativeDrawClosure::Present {
            executor_target_sha256: "d".repeat(64),
            actual_draw_count: 2,
            ordered_draw_receipt_sha256s: vec!["3".repeat(64), "4".repeat(64)],
        })
        .seal()
        .expect("present receipt seals");
        present.validate().expect("present receipt validates");

        let empty = component_receipt(Xmap13NativeDrawClosure::AuthenticatedEmpty {
            enumerator_witness_sha256: "5".repeat(64),
        })
        .seal()
        .expect("authenticated-empty receipt seals");
        empty.validate().expect("empty receipt validates");
    }

    #[test]
    fn counter_present_receipt_requires_its_independent_target() {
        let mut wrong = component_receipt(Xmap13NativeDrawClosure::Present {
            executor_target_sha256: "d".repeat(64),
            actual_draw_count: 1,
            ordered_draw_receipt_sha256s: vec!["3".repeat(64)],
        });
        wrong.component = Xmap13NativeComponent::CounterQuads;
        assert!(wrong.seal().is_err());

        let mut exact = component_receipt(Xmap13NativeDrawClosure::Present {
            executor_target_sha256: "6".repeat(64),
            actual_draw_count: 1,
            ordered_draw_receipt_sha256s: vec!["3".repeat(64)],
        });
        exact.component = Xmap13NativeComponent::CounterQuads;
        exact.seal().expect("counter target identity matches");
    }

    #[test]
    fn native_component_receipt_rejects_empty_present_and_tampering() {
        let error = component_receipt(Xmap13NativeDrawClosure::Present {
            executor_target_sha256: "d".repeat(64),
            actual_draw_count: 0,
            ordered_draw_receipt_sha256s: Vec::new(),
        })
        .seal()
        .expect_err("zero-draw Present must fail");
        assert!(error
            .to_string()
            .contains("empty, inconsistent, duplicated, or mistargeted"));

        for invalid in [
            Xmap13NativeDrawClosure::Present {
                executor_target_sha256: "d".repeat(64),
                actual_draw_count: 2,
                ordered_draw_receipt_sha256s: vec!["3".repeat(64)],
            },
            Xmap13NativeDrawClosure::Present {
                executor_target_sha256: "d".repeat(64),
                actual_draw_count: 2,
                ordered_draw_receipt_sha256s: vec!["3".repeat(64), "3".repeat(64)],
            },
        ] {
            assert!(component_receipt(invalid).seal().is_err());
        }

        let mut sealed = component_receipt(Xmap13NativeDrawClosure::AuthenticatedEmpty {
            enumerator_witness_sha256: "5".repeat(64),
        })
        .seal()
        .expect("receipt seals");
        sealed.assigned_ordinal += 1;
        assert!(sealed.validate().is_err());
    }

    #[test]
    fn authored_event_invocations_preserve_mixed_file_order_and_empty_sound() {
        let source = crate::strip_comments(
            r#"
                # event = { particle = "commented" node = "root" }
                event = {
                    particle = "muzzle"
                    node = "gun"
                    soundeffect = "fire"
                    particle = "smoke"
                }
                event = {
                    soundeffect = ""
                    trigger_once = yes
                }
            "#,
        );
        let blocks = ordered_assignment_blocks(&source, "event");
        assert_eq!(blocks.len(), 2);
        assert_eq!(
            authored_effect_invocations(&blocks[0]),
            vec![
                (Xmap13AuthoredEffectKind::Particle, "muzzle".into()),
                (Xmap13AuthoredEffectKind::Sound, "fire".into()),
                (Xmap13AuthoredEffectKind::Particle, "smoke".into()),
            ]
        );
        assert_eq!(
            authored_effect_invocations(&blocks[1]),
            vec![(Xmap13AuthoredEffectKind::Sound, String::new())]
        );
    }

    #[test]
    fn authored_event_census_accepts_multiple_effects_per_block_only_in_order() {
        let events = vec![
            Xmap13AuthoredStateEvent {
                authored_event_index: 0,
                authored_effect_index: 0,
                effect_kind: Xmap13AuthoredEffectKind::Particle,
                effect_token: "muzzle".into(),
                node: Some("gun".into()),
                event_time_f32_bits: None,
                keep_particle: None,
                trigger_once: None,
            },
            Xmap13AuthoredStateEvent {
                authored_event_index: 0,
                authored_effect_index: 1,
                effect_kind: Xmap13AuthoredEffectKind::Sound,
                effect_token: String::new(),
                node: None,
                event_time_f32_bits: None,
                keep_particle: None,
                trigger_once: None,
            },
        ];
        validate_events(1, &events).expect("ordered mixed effects are complete");
        let mut skipped = events.clone();
        skipped[1].authored_effect_index = 2;
        assert!(validate_events(1, &skipped).is_err());
        let mut missing_block = events;
        missing_block[1].authored_event_index = 1;
        assert!(validate_events(2, &missing_block).is_err());
    }

    #[test]
    fn skinned_rows_require_exact_palette_bytes_not_only_a_digest() {
        let palette = vec![0x5au8; 3_200];
        let digest = Sha256::hex(&palette);
        validate_skin_palette(true, Some(&digest), Some(&palette))
            .expect("exact palette bytes and digest");
        assert!(validate_skin_palette(true, Some(&digest), None).is_err());
        assert!(validate_skin_palette(true, Some(&"0".repeat(64)), Some(&palette)).is_err());
        assert!(validate_skin_palette(true, Some(&digest), Some(&palette[..3_199])).is_err());
        validate_skin_palette(false, None, None).expect("non-skinned rows omit palette");
        assert!(validate_skin_palette(false, Some(&digest), Some(&palette)).is_err());
    }
}
