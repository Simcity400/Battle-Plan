use super::{
    check_sha256, collect_static_draw_used_content, decimal_u64, validate_static_map_draw,
    StaticMapDraw, UsedContentKey,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const M03_ARROW_SCHEMA: &str = "hoi4.production-m03-order-movement-arrows.v1";
pub const M03_A729_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const M03_STATIC_PROOF_SHA256: &str =
    "040a1fdf596845eb9ec8b4d07086b00a129b3f38d2cd947a7439781073e31efe";
pub const XMAP15_STATE_FIRST_SCHEMA: &str = "hoi4.xmap15-orders-fronts-movement-state-first.v1";
pub const XMAP15_SEMANTIC_INPUT_SCHEMA: &str = "hoi4.xmap15-orders-fronts-movement-semantic.v1";
pub const XMAP15_SEMANTIC_PRODUCER_REGISTRATION_DESCRIPTOR: &str =
    "XMAP15/unconditional-complete-current-hour-census/v1";

pub fn xmap15_semantic_producer_registration_sha256() -> String {
    format!(
        "{:x}",
        Sha256::digest(
            format!(
                "XMAP15\0{XMAP15_SEMANTIC_INPUT_SCHEMA}\0{M03_A729_EXECUTABLE_SHA256}\0{XMAP15_SEMANTIC_PRODUCER_REGISTRATION_DESCRIPTOR}"
            )
            .as_bytes()
        )
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct M03ArrowFrame {
    pub schema: String,
    pub executable_sha256: String,
    pub static_semantic_proof_sha256: String,
    #[serde(with = "decimal_u64")]
    pub update_index: u64,
    #[serde(with = "decimal_u64")]
    pub first_sequence: u64,
    #[serde(with = "decimal_u64")]
    pub last_sequence: u64,
    pub used_content: Vec<UsedContentKey>,
    pub rows: Vec<M03ArrowRow>,
    pub draws: Vec<M03ArrowDraw>,
    pub ordered_draws: Vec<M03ArrowDrawRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum M03StableKey {
    Plan {
        owner_country_id: u32,
        army_id: u32,
        engine_order_instance_id: u32,
        generation: u32,
        slot_index: u32,
    },
    Movement {
        owner_country_id: u32,
        division_id: u32,
        movement_generation: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum M03PlanSubtype {
    Front,
    Offensive,
    Blitz,
    Fallback,
    NavalInvasion,
    Paradrop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum M03Visibility {
    Submitted,
    EngineCulled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct M03ArrowRow {
    #[serde(with = "decimal_u64")]
    pub event_sequence: u64,
    pub stable_key: M03StableKey,
    pub definition_name: String,
    pub plan_subtype: Option<M03PlanSubtype>,
    pub ordered_division_ids: Vec<u32>,
    pub ordered_fixed_point_input_points_1e5: Vec<[i32; 2]>,
    pub ordered_engine_working_points_1e5: Option<Vec<[i32; 2]>>,
    pub assigned: bool,
    pub active: bool,
    pub selected: bool,
    pub progress_raw_u32: u32,
    pub fade_in_f32_bits: u32,
    pub fade_out_f32_bits: u32,
    pub culling_flags_u16: u16,
    pub visibility: M03Visibility,
    pub manager_insertion_ordinal: u32,
    pub definition_group_ordinal: u32,
    pub comparator_definition_a0_f32_bits: u32,
    pub comparator_definition_a4_f32_bits: u32,
    pub comparator_definition_c8_f32_bits: u32,
    pub comparator_arrow_178_f32_bits: u32,
    pub comparator_primary_f32_bits: u32,
    pub comparator_tie_17c_f32_bits: u32,
    pub draw_members: Vec<M03ArrowDrawMemberRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct M03ArrowDraw {
    pub engine_draw_ordinal: u32,
    pub draw: StaticMapDraw,
    pub members: Vec<M03ArrowDrawMember>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct M03ArrowDrawRef {
    pub engine_draw_ordinal: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct M03ArrowDrawMemberRef {
    pub engine_draw_ordinal: u32,
    pub member_ordinal: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct M03ArrowDrawMember {
    pub stable_key: M03StableKey,
    pub member_ordinal: u32,
    pub first_vertex: u32,
    pub vertex_count: u32,
}

impl M03ArrowFrame {
    pub fn encode_json(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|error| error.to_string())
    }

    pub fn decode_json(bytes: &[u8]) -> Result<Self, String> {
        let value: Self = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != M03_ARROW_SCHEMA {
            return Err("unsupported M03 arrow schema".into());
        }
        if self.executable_sha256 != M03_A729_EXECUTABLE_SHA256
            || self.static_semantic_proof_sha256 != M03_STATIC_PROOF_SHA256
        {
            return Err("M03 carrier is not bound to the promoted a729 static proof".into());
        }
        check_sha256(&self.executable_sha256, None)?;
        check_sha256(&self.static_semantic_proof_sha256, None)?;
        if self.first_sequence > self.last_sequence {
            return Err("M03 sequence range is reversed".into());
        }

        let mut content_paths = BTreeSet::new();
        for content in &self.used_content {
            content
                .validate()
                .map_err(|error| format!("invalid M03 UsedContentKey: {error}"))?;
            if !content_paths.insert(content.normalized_game_relative_path.as_str()) {
                return Err("M03 used-content path is duplicated".into());
            }
        }
        if self.used_content.is_empty() {
            return Err("M03 carrier has no VFS-bound used content".into());
        }
        if self.used_content.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err("M03 used-content keys are not in deterministic order".into());
        }

        let mut row_keys = BTreeSet::new();
        let mut row_member_refs = BTreeSet::new();
        for row in &self.rows {
            row.validate(self.first_sequence, self.last_sequence)?;
            if !row_keys.insert(row.stable_key.clone()) {
                return Err("M03 stable row key is duplicated".into());
            }
            for reference in &row.draw_members {
                if !row_member_refs.insert((row.stable_key.clone(), reference.clone())) {
                    return Err("M03 row repeats a draw-member reference".into());
                }
            }
        }

        let mut draw_refs = BTreeSet::new();
        let mut actual_members = BTreeSet::new();
        for draw in &self.draws {
            validate_static_map_draw(&draw.draw)?;
            if !draw_refs.insert(M03ArrowDrawRef {
                engine_draw_ordinal: draw.engine_draw_ordinal,
            }) {
                return Err("M03 engine draw ordinal is duplicated".into());
            }
            if draw.members.is_empty() {
                return Err("M03 draw has no semantic members".into());
            }
            let mut previous_end = 0u32;
            for (index, member) in draw.members.iter().enumerate() {
                if member.member_ordinal as usize != index
                    || member.vertex_count == 0
                    || member.first_vertex < previous_end
                    || !row_keys.contains(&member.stable_key)
                {
                    return Err("M03 draw-member order/range/key is invalid".into());
                }
                previous_end = member
                    .first_vertex
                    .checked_add(member.vertex_count)
                    .ok_or("M03 member vertex range overflows")?;
                actual_members.insert((
                    member.stable_key.clone(),
                    M03ArrowDrawMemberRef {
                        engine_draw_ordinal: draw.engine_draw_ordinal,
                        member_ordinal: member.member_ordinal,
                    },
                ));
            }
        }
        let exact_draw_content = self
            .draws
            .iter()
            .flat_map(|packet| collect_static_draw_used_content(&packet.draw))
            .collect::<BTreeSet<_>>();
        let declared_content = self.used_content.iter().cloned().collect::<BTreeSet<_>>();
        if declared_content != exact_draw_content {
            return Err("M03 used-content list is not an exact cover of draw content".into());
        }
        if actual_members != row_member_refs {
            return Err("M03 row/draw semantic-member join is not exact".into());
        }

        let ordered = self.ordered_draws.iter().cloned().collect::<BTreeSet<_>>();
        if ordered.len() != self.ordered_draws.len() || ordered != draw_refs {
            return Err("M03 ordered draw list is not an exact one-to-one cover".into());
        }
        if self
            .ordered_draws
            .windows(2)
            .any(|pair| pair[0].engine_draw_ordinal >= pair[1].engine_draw_ordinal)
        {
            return Err("M03 ordered draws change engine draw order".into());
        }
        Ok(())
    }

    pub fn draw(&self, reference: &M03ArrowDrawRef) -> Option<&M03ArrowDraw> {
        self.draws
            .iter()
            .find(|draw| draw.engine_draw_ordinal == reference.engine_draw_ordinal)
    }
}

impl M03ArrowRow {
    fn validate(&self, first_sequence: u64, last_sequence: u64) -> Result<(), String> {
        if self.event_sequence < first_sequence || self.event_sequence > last_sequence {
            return Err("M03 event sequence is outside frame range".into());
        }
        if self.ordered_fixed_point_input_points_1e5.len() < 2 {
            return Err("M03 row has fewer than two exact input points".into());
        }
        if self
            .ordered_engine_working_points_1e5
            .as_ref()
            .is_some_and(|points| points.len() < 2)
        {
            return Err("M03 row has an invalid engine working-point vector".into());
        }
        if !self.assigned && self.active {
            return Err("unassigned M03 row is active".into());
        }
        match (&self.stable_key, self.plan_subtype) {
            (M03StableKey::Movement { .. }, None) => {
                const NAMES: [&str; 8] = [
                    "moveArrow",
                    "followOrdersArrow",
                    "retreatArrow",
                    "withdrawArrow",
                    "attackArrow",
                    "supportArrow",
                    "navalMissionArrow",
                    "transportArrow",
                ];
                if !NAMES.contains(&self.definition_name.as_str())
                    || !self.ordered_division_ids.is_empty()
                {
                    return Err("M03 movement definition/identity payload is invalid".into());
                }
            }
            (M03StableKey::Plan { .. }, Some(subtype)) => {
                let expected = match subtype {
                    M03PlanSubtype::Front => "move_thick",
                    M03PlanSubtype::Offensive => "offence_line",
                    M03PlanSubtype::Blitz => "blitz_line",
                    M03PlanSubtype::Fallback => "fallback_line",
                    M03PlanSubtype::NavalInvasion => "invasion",
                    M03PlanSubtype::Paradrop => "paradrop",
                };
                if self.definition_name != expected || self.ordered_division_ids.is_empty() {
                    return Err("M03 plan subtype/definition/division payload is invalid".into());
                }
            }
            _ => return Err("M03 stable-key family and plan subtype disagree".into()),
        }

        let finite_nonnegative = |bits: u32| {
            let value = f32::from_bits(bits);
            value.is_finite() && value >= 0.0
        };
        if !finite_nonnegative(self.fade_in_f32_bits) || !finite_nonnegative(self.fade_out_f32_bits)
        {
            return Err("M03 spatial fade input is non-finite or negative".into());
        }
        let a0 = f32::from_bits(self.comparator_definition_a0_f32_bits);
        let a4 = f32::from_bits(self.comparator_definition_a4_f32_bits);
        let c8 = f32::from_bits(self.comparator_definition_c8_f32_bits);
        let arrow = f32::from_bits(self.comparator_arrow_178_f32_bits);
        let tie = f32::from_bits(self.comparator_tie_17c_f32_bits);
        if ![a0, a4, c8, arrow, tie].into_iter().all(f32::is_finite) {
            return Err("M03 comparator input is non-finite".into());
        }
        let product = arrow * c8;
        let primary = a4.max(a0) + product;
        if primary.to_bits() != self.comparator_primary_f32_bits {
            return Err("M03 comparator primary key disagrees with exact a729 law".into());
        }
        match self.visibility {
            M03Visibility::Submitted if self.draw_members.is_empty() => {
                return Err("submitted M03 row has no draw member".into())
            }
            M03Visibility::EngineCulled if !self.draw_members.is_empty() => {
                return Err("engine-culled M03 row has a draw member".into())
            }
            _ => {}
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap15SemanticFrame {
    pub schema: String,
    pub executable_sha256: String,
    pub producer_registration_sha256: String,
    pub hour_index: i64,
    #[serde(with = "decimal_u64")]
    pub observation_sequence: u64,
    pub baseline_complete: bool,
    pub omitted_plan_rows: u32,
    pub omitted_movement_rows: u32,
    pub plan_object_census: u32,
    pub movement_object_census: u32,
    pub plans: Vec<Xmap15PlanSemanticRow>,
    pub movements: Vec<Xmap15MovementSemanticRow>,
    pub enumerator_witness_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Xmap15OwnerKind {
    Army,
    ArmyGroup,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap15PlanKey {
    pub owner_kind: Xmap15OwnerKind,
    pub owner_id: u32,
    pub engine_order_instance_id: u32,
    #[serde(with = "decimal_u64")]
    pub generation: u64,
    pub slot_index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap15MovementKey {
    pub owner_country: String,
    pub division_id: u32,
    #[serde(with = "decimal_u64")]
    pub movement_generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap15PlanDefinitionSelector {
    pub order_type: u8,
    pub move_special_118: bool,
    pub withdraw_119: bool,
    pub fallback_298: bool,
    pub virtual_299: bool,
    pub blitz_118: bool,
    pub army_group_style_39_and_378: bool,
}

impl Xmap15PlanDefinitionSelector {
    pub fn resolved_definition_name(self) -> Option<&'static str> {
        match self.order_type {
            1 if self.move_special_118 => Some("blitz"),
            1 if self.withdraw_119 => Some("withdraw"),
            1 if self.army_group_style_39_and_378 => Some("ag_move_thick"),
            1 => Some("move_thick"),
            2 if self.virtual_299 => Some("virtual_line"),
            2 if self.fallback_298 => Some("fallback_line"),
            2 if self.blitz_118 => Some("blitz_line"),
            2 if self.withdraw_119 => Some("withdraw_line"),
            2 if self.army_group_style_39_and_378 => Some("ag_offence_line"),
            2 => Some("offence_line"),
            3 => Some("invasion"),
            4 => Some("paradrop"),
            _ => None,
        }
    }

    fn validate(self) -> Result<(), String> {
        let impossible = match self.order_type {
            1 => self.fallback_298 || self.virtual_299 || self.blitz_118,
            2 => self.move_special_118,
            3 | 4 => {
                self.move_special_118
                    || self.withdraw_119
                    || self.fallback_298
                    || self.virtual_299
                    || self.blitz_118
                    || self.army_group_style_39_and_378
            }
            _ => true,
        };
        if impossible {
            return Err("XMAP15 plan selector contains predicates outside its order type".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Xmap15MovementDefinition {
    MoveArrow,
    FollowOrdersArrow,
    RetreatArrow,
    WithdrawArrow,
    AttackArrow,
    SupportArrow,
    NavalMissionArrow,
    TransportArrow,
}

impl Xmap15MovementDefinition {
    pub const fn definition_name(self) -> &'static str {
        match self {
            Self::MoveArrow => "moveArrow",
            Self::FollowOrdersArrow => "followOrdersArrow",
            Self::RetreatArrow => "retreatArrow",
            Self::WithdrawArrow => "withdrawArrow",
            Self::AttackArrow => "attackArrow",
            Self::SupportArrow => "supportArrow",
            Self::NavalMissionArrow => "navalMissionArrow",
            Self::TransportArrow => "transportArrow",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap15PreparedRoute {
    pub ordered_province_ids: Vec<u32>,
    pub builder_input_points_fixed5: Vec<[i64; 2]>,
    pub engine_pre_smoother_points_fixed5: Option<Vec<[i64; 2]>>,
    pub point_width_classes: Vec<u8>,
    pub explicit_height_f32_bits: Option<Vec<u32>>,
    pub wrap_offset_world: i32,
}

impl Xmap15PreparedRoute {
    pub fn validate(&self) -> Result<(), String> {
        if self.ordered_province_ids.is_empty()
            || self.builder_input_points_fixed5.len() < 2
            || self.point_width_classes.len() != self.builder_input_points_fixed5.len()
        {
            return Err("XMAP15 route has fewer than two exact points".into());
        }
        if self.ordered_province_ids.iter().any(|province_id| {
            *province_id == 0 || *province_id > portable_admission::SUPPLY_MAX_PROVINCE_ID
        }) {
            return Err("XMAP15 route contains an invalid province identity".into());
        }
        if self
            .engine_pre_smoother_points_fixed5
            .as_ref()
            .is_some_and(|points| points.len() < 2)
        {
            return Err("XMAP15 route has an invalid pre-smoother vector".into());
        }
        if let Some(heights) = &self.explicit_height_f32_bits {
            if heights.len() != self.builder_input_points_fixed5.len()
                || heights
                    .iter()
                    .any(|bits| !f32::from_bits(*bits).is_finite())
            {
                return Err("XMAP15 route has invalid explicit height values".into());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap15PlanSemanticRow {
    pub key: Xmap15PlanKey,
    pub owner_country: String,
    pub selector: Xmap15PlanDefinitionSelector,
    pub resolved_definition_name: String,
    pub ordered_assigned_division_ids: Vec<u32>,
    pub assigned: bool,
    pub active: bool,
    pub route: Xmap15PreparedRoute,
    pub manager_insertion_ordinal: Option<u32>,
    pub definition_group_ordinal: Option<u32>,
    pub object_width_multiplier_f32_bits: u32,
    pub comparator_tie_17c_f32_bits: u32,
    pub fade_in_f32_bits: u32,
    pub fade_out_f32_bits: u32,
    pub culling_offset_f32_bits: u32,
    pub inner_rgba_f32_bits: [u32; 4],
    pub outer_rgba_f32_bits: [u32; 4],
    pub texture_variant: u8,
    pub see_through: bool,
    pub visible: bool,
    pub final_engine_draw_ordinal: Option<u32>,
    pub exact_head_vertices_f32_bits: Option<[[u32; 8]; 4]>,
    pub lifecycle_tombstone_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap15MovementRoute {
    pub ordered_province_ids: Vec<u32>,
}

impl Xmap15MovementRoute {
    pub fn validate(&self) -> Result<(), String> {
        if self.ordered_province_ids.is_empty()
            || self.ordered_province_ids.iter().any(|province_id| {
                *province_id == 0 || *province_id > portable_admission::SUPPLY_MAX_PROVINCE_ID
            })
        {
            return Err("XMAP15 movement route contains an invalid province identity".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap15MovementSemanticRow {
    pub key: Xmap15MovementKey,
    pub definition: Xmap15MovementDefinition,
    pub progress_raw_u32: u32,
    pub progress_move_army_f32_bits: [u32; 3],
    pub desaturation_lengths_f32_bits: [u32; 4],
    pub offset_x_f32_bits: u32,
    pub route: Xmap15MovementRoute,
    pub manager_insertion_ordinal: u32,
    pub definition_group_ordinal: u32,
    pub object_width_multiplier_f32_bits: u32,
    pub comparator_tie_17c_f32_bits: u32,
    pub fade_in_f32_bits: u32,
    pub fade_out_f32_bits: u32,
    pub inner_rgba_f32_bits: [u32; 4],
    pub outer_rgba_f32_bits: [u32; 4],
    pub texture_variant: u8,
    pub see_through: bool,
    pub visible: bool,
    pub final_engine_draw_ordinal: u32,
    pub exact_vertex_stride_bytes: u32,
    pub exact_vertex_count: u32,
    pub exact_vertex_bytes: Vec<u8>,
    pub exact_index_format_u32: u32,
    pub exact_index_count: u32,
    pub exact_index_bytes: Vec<u8>,
    pub exact_primitive_topology_u32: u32,
    pub lifecycle_tombstone_sha256: String,
}

impl Xmap15SemanticFrame {
    pub fn decode_json(bytes: &[u8]) -> Result<Self, String> {
        let frame: Self = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        frame.validate()?;
        Ok(frame)
    }

    pub fn encode_json(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|error| error.to_string())
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != XMAP15_SEMANTIC_INPUT_SCHEMA
            || self.executable_sha256 != M03_A729_EXECUTABLE_SHA256
            || self.producer_registration_sha256 != xmap15_semantic_producer_registration_sha256()
        {
            return Err("XMAP15 semantic frame has unsupported authority".into());
        }
        check_sha256(&self.executable_sha256, None)?;
        check_sha256(&self.producer_registration_sha256, None)?;
        check_sha256(&self.enumerator_witness_sha256, None)?;
        if !self.baseline_complete
            || self.observation_sequence == 0
            || self.omitted_plan_rows != 0
            || self.omitted_movement_rows != 0
            || self.plan_object_census as usize != self.plans.len()
            || self.movement_object_census as usize != self.movements.len()
        {
            return Err("XMAP15 semantic frame is not a complete current-hour census".into());
        }
        let mut plan_keys = BTreeSet::new();
        let mut final_draw_ordinals = BTreeSet::new();
        for row in &self.plans {
            row.selector.validate()?;
            if row.key.owner_id == 0
                || row.key.engine_order_instance_id == 0
                || row.key.slot_index >= 64
                || row.owner_country.trim().is_empty()
                || row.selector.resolved_definition_name() != Some(&row.resolved_definition_name)
                || row.ordered_assigned_division_ids.is_empty() != !row.assigned
                || (!row.assigned && row.active)
                || row.manager_insertion_ordinal.is_none()
                || row.definition_group_ordinal.is_none()
                || row.final_engine_draw_ordinal.is_none()
                || !plan_keys.insert(row.key.clone())
                || !final_draw_ordinals.insert(row.final_engine_draw_ordinal.unwrap())
            {
                return Err("XMAP15 plan semantic row violates identity/selector state".into());
            }
            let mut members = BTreeSet::new();
            if row
                .ordered_assigned_division_ids
                .iter()
                .any(|id| *id == 0 || !members.insert(*id))
            {
                return Err("XMAP15 plan has invalid or duplicate assigned divisions".into());
            }
            validate_finite_bits(
                row.object_width_multiplier_f32_bits,
                "plan width multiplier",
            )?;
            validate_finite_bits(row.comparator_tie_17c_f32_bits, "plan comparator tie")?;
            validate_runtime_draw_inputs(
                row.fade_in_f32_bits,
                row.fade_out_f32_bits,
                Some(row.culling_offset_f32_bits),
                row.inner_rgba_f32_bits,
                row.outer_rgba_f32_bits,
            )?;
            if let Some(vertices) = &row.exact_head_vertices_f32_bits {
                if vertices
                    .iter()
                    .flatten()
                    .any(|bits| !f32::from_bits(*bits).is_finite())
                {
                    return Err("XMAP15 exact head vertex contains non-finite bits".into());
                }
            }
            check_sha256(row.lifecycle_tombstone_sha256.as_deref()
                .ok_or("XMAP15 native plan lacks its lifecycle witness")?, None)?;
            row.route.validate()?;
        }
        let mut movement_keys = BTreeSet::new();
        for row in &self.movements {
            if row.key.owner_country.trim().is_empty()
                || row.key.division_id == 0
                || !movement_keys.insert(row.key.clone())
                || !final_draw_ordinals.insert(row.final_engine_draw_ordinal)
            {
                return Err("XMAP15 movement semantic row has invalid/duplicate identity".into());
            }
            validate_finite_bits(
                row.object_width_multiplier_f32_bits,
                "movement width multiplier",
            )?;
            validate_finite_bits(row.comparator_tie_17c_f32_bits, "movement comparator tie")?;
            validate_runtime_draw_inputs(
                row.fade_in_f32_bits,
                row.fade_out_f32_bits,
                None,
                row.inner_rgba_f32_bits,
                row.outer_rgba_f32_bits,
            )?;
            if row
                .progress_move_army_f32_bits
                .into_iter()
                .chain(row.desaturation_lengths_f32_bits)
                .chain(std::iter::once(row.offset_x_f32_bits))
                .any(|bits| !f32::from_bits(bits).is_finite())
            {
                return Err("XMAP15 movement CB1 contains non-finite float bits".into());
            }
            let expected_vertex_bytes = usize::try_from(row.exact_vertex_stride_bytes)
                .ok()
                .and_then(|stride| {
                    usize::try_from(row.exact_vertex_count)
                        .ok()
                        .and_then(|count| stride.checked_mul(count))
                });
            if row.exact_vertex_stride_bytes != 32
                || row.exact_vertex_count == 0
                || expected_vertex_bytes != Some(row.exact_vertex_bytes.len())
            {
                return Err("XMAP15 movement tessellation bytes are incomplete".into());
            }
            let index_width: usize = match row.exact_index_format_u32 {
                0 if row.exact_index_count == 0 => 0,
                57 => 2,
                42 => 4,
                _ => return Err("XMAP15 movement index format is unsupported".into()),
            };
            let expected_index_bytes = usize::try_from(row.exact_index_count)
                .ok()
                .and_then(|count| index_width.checked_mul(count));
            if expected_index_bytes != Some(row.exact_index_bytes.len()) {
                return Err("XMAP15 movement index bytes are incomplete".into());
            }
            if !matches!(row.exact_primitive_topology_u32, 4 | 5) {
                return Err("XMAP15 movement primitive topology is unsupported".into());
            }
            check_sha256(&row.lifecycle_tombstone_sha256, None)?;
            row.route.validate()?;
        }
        Ok(())
    }
}

fn validate_finite_bits(bits: u32, label: &str) -> Result<(), String> {
    if !f32::from_bits(bits).is_finite() {
        return Err(format!("XMAP15 {label} is non-finite"));
    }
    Ok(())
}

fn validate_runtime_draw_inputs(
    fade_in_bits: u32,
    fade_out_bits: u32,
    culling_bits: Option<u32>,
    inner: [u32; 4],
    outer: [u32; 4],
) -> Result<(), String> {
    let finite_nonnegative = |bits| {
        let value = f32::from_bits(bits);
        value.is_finite() && value >= 0.0
    };
    if !finite_nonnegative(fade_in_bits)
        || !finite_nonnegative(fade_out_bits)
        || culling_bits.is_some_and(|bits| !f32::from_bits(bits).is_finite())
        || inner
            .into_iter()
            .chain(outer)
            .any(|bits| !f32::from_bits(bits).is_finite())
    {
        return Err("XMAP15 runtime draw inputs contain invalid float bits".into());
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap15PreparedFrame {
    pub schema: String,
    pub hour_index: i64,
    #[serde(with = "decimal_u64")]
    pub observation_sequence: u64,
    pub semantic_input_sha256: String,
    pub enumerator_witness_sha256: String,
    pub xmap13_census_sha256: String,
    pub xmap13_admission_sha256: String,
    pub selected_plan: Option<Xmap15PlanKey>,
    pub viewer_runtime: Option<Xmap15ViewerRuntimeFrame>,
    pub ordered_rows: Vec<Xmap15PreparedRow>,
    pub ordered_used_content: Vec<UsedContentKey>,
    pub deterministic_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap15ViewerRuntimeFrame {
    pub retained_plans: Vec<Xmap15PlanKey>,
    pub draw_inputs: Vec<Xmap15ViewerPlanDrawInputs>,
    pub refusals: Vec<Xmap15ViewerPlanRefusal>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap15ViewerPlanDrawInputs {
    pub key: Xmap15PlanKey,
    pub base_rgba_f32_bits: [u32; 4],
    pub inner_highlight: bool,
    pub animation_speed_f32_bits: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Xmap15ViewerMissingInput {
    OwnerIdentity, OwnerColour, OwnerOverride, VirtualPresentation, SelectedPresentation,
    FrontBinding, CurrentFrontResolver, RootTraversal, RouteGeometry, AssignedUnitFilter,
    ActiveReadiness, CullingDirection, Definition, MovementRuntime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap15ViewerPlanRefusal {
    pub key: Option<Xmap15PlanKey>,
    pub input: Xmap15ViewerMissingInput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Xmap15PreparedRow {
    Plan {
        semantic: Xmap15PlanSemanticRow,
        definition_sha256: String,
    },
    Movement {
        semantic: Xmap15MovementSemanticRow,
        definition_sha256: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Xmap15PlanGeometryProfile {
    pub body_start_width_f32_bits: u32,
    pub body_end_width_f32_bits: u32,
    pub runtime_width_scale_f32_bits: u32,
    pub body_uv_scale_f32_bits: u32,
    pub width_class_zero_f32_bits: u32,
    pub width_class_one_f32_bits: u32,
    pub global_scale_min_size_f32_bits: u32,
    pub global_scale_max_points: u32,
    pub head_length_f32_bits: u32,
}

impl Xmap15PlanGeometryProfile {
    pub fn validate(&self) -> Result<(), String> {
        for bits in [
            self.body_start_width_f32_bits,
            self.body_end_width_f32_bits,
            self.runtime_width_scale_f32_bits,
            self.body_uv_scale_f32_bits,
            self.width_class_zero_f32_bits,
            self.width_class_one_f32_bits,
            self.global_scale_min_size_f32_bits,
            self.head_length_f32_bits,
        ] {
            let value = f32::from_bits(bits);
            if !value.is_finite() {
                return Err("XMAP15 geometry profile contains an invalid scalar".into());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Xmap15MapArrowVertex {
    pub position: [f32; 3],
    pub padding: f32,
    pub uv_is_head_variant: [f32; 4],
}

#[derive(Debug, Clone, PartialEq)]
pub struct Xmap15PlanMapArrowMesh {
    pub vertices: Vec<Xmap15MapArrowVertex>,
    pub body_vertex_count: u32,
    pub head_first_vertex: Option<u32>,
    pub total_length_f32_bits: u32,
    pub wrap_offset_world: i32,
}

impl Xmap15PlanSemanticRow {
    pub fn uses_native_terrain_height(&self) -> bool {
        matches!(self.selector.order_type, 1..=3) && self.route.explicit_height_f32_bits.is_none()
    }

    pub fn build_map_arrow_mesh(
        &self,
        profile: Xmap15PlanGeometryProfile,
        map_width_world: u32,
        terrain_height: impl FnMut(f32, f32) -> Result<f32, String>,
    ) -> Result<Xmap15PlanMapArrowMesh, String> {
        self.build_native_map_arrow_mesh(profile, map_width_world, None, terrain_height)
    }

    pub fn build_paradrop_map_arrow_mesh(
        &self,
        profile: Xmap15PlanGeometryProfile,
        map_width_world: u32,
        paradrop: crate::Xmap15ParadropProfile,
        terrain_height: impl FnMut(f32, f32) -> Result<f32, String>,
    ) -> Result<Xmap15PlanMapArrowMesh, String> {
        if self.selector.order_type != 4 || self.route.explicit_height_f32_bits.is_some() {
            return Err("XMAP15 paradrop callback conflicts with the row's height source".into());
        }
        self.build_native_map_arrow_mesh(profile, map_width_world, Some(paradrop), terrain_height)
    }

    fn build_native_map_arrow_mesh(
        &self,
        profile: Xmap15PlanGeometryProfile,
        map_width_world: u32,
        paradrop: Option<crate::Xmap15ParadropProfile>,
        mut terrain_height: impl FnMut(f32, f32) -> Result<f32, String>,
    ) -> Result<Xmap15PlanMapArrowMesh, String> {
        self.route.validate()?;
        profile.validate()?;
        if map_width_world == 0 || map_width_world > i32::MAX as u32 {
            return Err("XMAP15 authenticated map width is outside the native signed range".into());
        }
        let curve = paradrop.map(|parameters| crate::xmap15_paradrop::ParadropCurve::new(
            &self.route.builder_input_points_fixed5, map_width_world, parameters,
        )).transpose()?;
        if self.route.explicit_height_f32_bits.is_none() && !self.uses_native_terrain_height() && curve.is_none() {
            return Err("XMAP15 native height callback inputs are unavailable".into());
        }
        let smoothed = smooth_xmap15_points(&self.route.builder_input_points_fixed5)?;
        if smoothed.len() > 8_192 {
            return Err("XMAP15 map-arrow point count exceeds the exact builder cap".into());
        }
        let mut accumulated = Vec::with_capacity(smoothed.len());
        let mut total_length = 0.0f32;
        accumulated.push(0.0f32);
        for pair in smoothed.windows(2) {
            let dx = pair[1][0] - pair[0][0];
            let dz = pair[1][1] - pair[0][1];
            total_length += (dx * dx + dz * dz).sqrt();
            accumulated.push(total_length);
        }
        if !total_length.is_finite() || total_length <= 0.0 {
            return Err("XMAP15 smoothed map-arrow length is zero or invalid".into());
        }

        let point_count =
            u32::try_from(smoothed.len()).map_err(|_| "XMAP15 point count exceeds u32")?;
        let minimum = f32::from_bits(profile.global_scale_min_size_f32_bits);
        let global_scale = if profile.global_scale_max_points > 0
            && point_count < profile.global_scale_max_points
        {
            minimum
                + (point_count as f32 / profile.global_scale_max_points as f32) * (1.0 - minimum)
        } else {
            1.0
        };
        let start = f32::from_bits(profile.body_start_width_f32_bits)
            + f32::from_bits(self.object_width_multiplier_f32_bits)
                * f32::from_bits(profile.runtime_width_scale_f32_bits);
        let end = f32::from_bits(profile.body_end_width_f32_bits);
        let head_length = f32::from_bits(profile.head_length_f32_bits) * global_scale;
        let headed = f32::from_bits(profile.head_length_f32_bits) > 0.0;
        let callback_denominator = smoothed.len() + if headed { 2 } else { 0 };
        let uv_scale = f32::from_bits(profile.body_uv_scale_f32_bits);
        let mut vertices = Vec::with_capacity(smoothed.len() * 2 + 4);
        let mut head_geometry = None;
        for (index, mut center) in smoothed.iter().copied().enumerate() {
            let next_index = index.saturating_add(1).min(smoothed.len() - 1);
            let mut next = smoothed[next_index];
            let mut previous = smoothed[index.saturating_sub(1)];
            let trimmed = headed && index > 0 && total_length - head_length <= accumulated[index];
            if trimmed {
                let delta = [center[0] - previous[0], center[1] - previous[1]];
                let segment_length = (delta[1] * delta[1] + delta[0] * delta[0]).sqrt();
                let remaining = (total_length - head_length) - (accumulated[index] - segment_length);
                let direction = normalize_xmap15_direction(delta);
                center = [direction[0] * remaining + previous[0], direction[1] * remaining + previous[1]];
                previous = center;
                next = smoothed[smoothed.len() - 1];
            }
            let mut outgoing = normalize_xmap15_direction([
                next[0] - center[0], next[1] - center[1],
            ]);
            let mut incoming = normalize_xmap15_direction([
                center[0] - previous[0], center[1] - previous[1],
            ]);
            if outgoing == [0.0, 0.0] {
                outgoing = incoming;
            }
            if incoming == [0.0, 0.0] {
                incoming = outgoing;
            }
            let lateral = normalize_xmap15_direction([
                -(incoming[1] + outgoing[1]) * 0.5,
                (incoming[0] + outgoing[0]) * 0.5,
            ]);
            if lateral == [0.0, 0.0] {
                return Err("XMAP15 map-arrow tangent has no exact lateral".into());
            }
            let t = accumulated[index] / total_length;
            let u = if uv_scale != 0.0 { accumulated[index] / uv_scale } else { t };
            let class_scale = match self.route.point_width_classes[index] {
                0 => f32::from_bits(profile.width_class_zero_f32_bits),
                1 => f32::from_bits(profile.width_class_one_f32_bits),
                _ => 1.0,
            };
            let unclassed_width = (start + (end - start) * t) * global_scale;
            let width = unclassed_width * class_scale;
            if !width.is_finite() || !unclassed_width.is_finite() || !head_length.is_finite() {
                return Err("XMAP15 map-arrow width became non-finite".into());
            }
            head_geometry = Some((center, lateral, if trimmed {
                normalize_xmap15_direction([next[0] - center[0], next[1] - center[1]])
            } else { outgoing }, unclassed_width));
            let explicit_height = match &curve {
                Some(curve) => Some(curve.height(index, callback_denominator, &mut terrain_height)?),
                None => self.route.explicit_height_f32_bits.as_ref()
                    .map(|heights| f32::from_bits(heights[index])),
            };
            for (v, sign) in [(0.0f32, 1.0f32), (1.0f32, -1.0f32)] {
                let x = center[0] + lateral[0] * width * sign;
                let z = center[1] + lateral[1] * width * sign;
                let y = match explicit_height {
                    Some(value) => value,
                    None => xmap15_terrain_height([x, z], map_width_world, &mut terrain_height)?,
                };
                if [x, y, z].into_iter().any(|value| !value.is_finite()) {
                    return Err("XMAP15 terrain projection produced non-finite geometry".into());
                }
                vertices.push(Xmap15MapArrowVertex {
                    position: [x, y, z],
                    padding: 0.0,
                    uv_is_head_variant: [
                        u,
                        v,
                        0.0,
                        self.route.point_width_classes[index] as i8 as f32,
                    ],
                });
            }
            if trimmed {
                // The engine repeats the cut pair without visiting later points or height callbacks.
                let pair = [vertices[vertices.len() - 2], vertices[vertices.len() - 1]];
                for _ in index + 1..smoothed.len() {
                    vertices.extend(pair);
                }
                break;
            }
        }
        let body_vertex_count =
            u32::try_from(vertices.len()).map_err(|_| "XMAP15 body vertices exceed u32")?;
        vertices.reverse();
        let head_first_vertex = if headed {
            let head = if self.uses_native_terrain_height() || curve.is_some() {
                let (center, lateral, direction, width) = head_geometry
                    .ok_or("XMAP15 terrain head has no body endpoint")?;
                let mut head = Vec::with_capacity(4);
                let mut callback_height = None;
                for (tip, sign, uv) in [
                    (false, 1.0, [0.0, 0.0, 1.0, 0.0]),
                    (false, -1.0, [0.0, 1.0, 1.0, 0.0]),
                    (true, 1.0, [1.0, 0.0, 1.0, 0.0]),
                    (true, -1.0, [1.0, 1.0, 1.0, 0.0]),
                ] {
                    let lateral_offset = [lateral[0] * width, lateral[1] * width];
                    let mut position = if sign > 0.0 {
                        [center[0] + lateral_offset[0], center[1] + lateral_offset[1]]
                    } else {
                        [center[0] - lateral_offset[0], center[1] - lateral_offset[1]]
                    };
                    if tip {
                        position = [direction[0] * head_length + position[0],
                            direction[1] * head_length + position[1]];
                    }
                    if let Some(curve) = &curve {
                        if sign > 0.0 {
                            let index = callback_denominator - if tip { 0 } else { 1 };
                            callback_height = Some(curve.height(index, callback_denominator, &mut terrain_height)?);
                        }
                    }
                    let y = match callback_height {
                        Some(value) => value,
                        None => xmap15_terrain_height(position, map_width_world, &mut terrain_height)?,
                    };
                    head.push(Xmap15MapArrowVertex {
                        position: [position[0], y, position[1]], padding: 0.0,
                        uv_is_head_variant: uv,
                    });
                }
                head.reverse();
                // Native +0x80/+0xa0 are the final body's negative/positive sides.
                let negative = vertices[0].position;
                let positive = vertices[1].position;
                let delta = std::array::from_fn(|axis| positive[axis] - negative[axis]);
                let midpoint: [f32; 3] = std::array::from_fn(|axis| delta[axis] * 0.5 + negative[axis]);
                let direction = normalize_xmap15_head_base(delta)?;
                head[3].position = std::array::from_fn(|axis| direction[axis] * width + midpoint[axis]);
                head[2].position = std::array::from_fn(|axis| midpoint[axis] - direction[axis] * width);
                if head.iter().any(|vertex| vertex.position.iter().any(|value| !value.is_finite())) {
                    return Err("XMAP15 terrain head produced non-finite geometry".into());
                }
                if let Some(exact) = &self.exact_head_vertices_f32_bits {
                    for (derived, exact) in head.iter().zip(exact) {
                        if derived.position.map(f32::to_bits) != exact[..3]
                            || derived.uv_is_head_variant.map(f32::to_bits) != exact[4..] {
                            return Err("XMAP15 exact head disagrees with native terrain derivation".into());
                        }
                    }
                }
                head
            } else {
                let exact = self.exact_head_vertices_f32_bits.as_ref()
                    .ok_or("XMAP15 native height callback lacks exact head inputs")?;
                exact.iter().map(|vertex| Xmap15MapArrowVertex {
                    position: std::array::from_fn(|axis| f32::from_bits(vertex[axis])),
                    padding: f32::from_bits(vertex[3]),
                    uv_is_head_variant: std::array::from_fn(|axis| f32::from_bits(vertex[axis + 4])),
                }).collect()
            };
            vertices.extend(head);
            vertices.rotate_right(4);
            Some(0)
        } else {
            if self.exact_head_vertices_f32_bits.is_some() {
                return Err("XMAP15 no-head definition carries unexpected head vertices".into());
            }
            None
        };
        Ok(Xmap15PlanMapArrowMesh {
            vertices,
            body_vertex_count,
            head_first_vertex,
            total_length_f32_bits: total_length.to_bits(),
            wrap_offset_world: self.route.wrap_offset_world,
        })
    }
}

fn xmap15_terrain_height(
    position: [f32; 2], map_width_world: u32,
    terrain_height: &mut impl FnMut(f32, f32) -> Result<f32, String>,
) -> Result<f32, String> {
    if position.iter().any(|value| !value.is_finite()) {
        return Err("XMAP15 terrain vertex is non-finite".into());
    }
    let width = map_width_world as f32;
    let x = if position[0] < width {
        if position[0] < 0.0 { position[0] + width } else { position[0] }
    } else { position[0] - width };
    let height = terrain_height(x + 0.5, position[1] + 0.5)? + 0.01;
    if !height.is_finite() {
        return Err("XMAP15 terrain projection produced a non-finite height".into());
    }
    Ok(height)
}

fn normalize_xmap15_head_base(value: [f32; 3]) -> Result<[f32; 3], String> {
    let length = ((value[0] * value[0] + value[1] * value[1]) + value[2] * value[2]).sqrt();
    if !length.is_finite() {
        return Err("XMAP15 native head base normalization is non-finite".into());
    }
    Ok(if length != 0.0 { value.map(|axis| axis / length) } else { value })
}

fn smooth_xmap15_points(points: &[[i64; 2]]) -> Result<Vec<[f32; 2]>, String> {
    if points.len() < 2 {
        return Err("XMAP15 smoothing input has fewer than two points".into());
    }
    points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            let fixed = if index == 0 || index + 1 == points.len() {
                *point
            } else {
                let previous = points[index - 1];
                let next = points[index + 1];
                let component = |axis: usize| -> Result<i64, String> {
                    let current_quarter = point[axis]
                        .checked_mul(25_000)
                        .ok_or("XMAP15 smoothing overflow")?
                        / 100_000;
                    let neighbor_half = previous[axis]
                        .checked_add(next[axis])
                        .and_then(|value| value.checked_mul(50_000))
                        .ok_or("XMAP15 smoothing overflow")?
                        / 100_000;
                    current_quarter
                        .checked_add(
                            neighbor_half
                                .checked_mul(75_000)
                                .ok_or("XMAP15 smoothing overflow")?
                                / 100_000,
                        )
                        .ok_or_else(|| "XMAP15 smoothing overflow".into())
                };
                [component(0)?, component(1)?]
            };
            Ok([fixed[0] as f32 / 100_000.0, fixed[1] as f32 / 100_000.0])
        })
        .collect()
}

fn normalize_xmap15_direction(value: [f32; 2]) -> [f32; 2] {
    let length = (value[0] * value[0] + value[1] * value[1]).sqrt();
    if length > 0.0 && length.is_finite() {
        [value[0] / length, value[1] / length]
    } else {
        [0.0, 0.0]
    }
}

impl Xmap15PreparedFrame {
    pub fn seal(mut self) -> Result<Self, String> {
        self.deterministic_sha256.clear();
        self.validate_body()?;
        self.deterministic_sha256 = self.body_sha256()?;
        self.validate()?;
        Ok(self)
    }

    pub fn validate(&self) -> Result<(), String> {
        self.validate_body()?;
        check_sha256(&self.deterministic_sha256, None)?;
        if self.deterministic_sha256 != self.body_sha256()? {
            return Err("XMAP15 prepared-frame digest mismatch".into());
        }
        Ok(())
    }

    fn body_sha256(&self) -> Result<String, String> {
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        let bytes = serde_json::to_vec(&unsigned).map_err(|error| error.to_string())?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }

    fn validate_body(&self) -> Result<(), String> {
        if self.schema != XMAP15_STATE_FIRST_SCHEMA {
            return Err("unsupported XMAP15 prepared schema".into());
        }
        if self.observation_sequence == 0 {
            return Err("XMAP15 prepared frame has no producer observation sequence".into());
        }
        for digest in [
            &self.semantic_input_sha256,
            &self.enumerator_witness_sha256,
            &self.xmap13_census_sha256,
            &self.xmap13_admission_sha256,
        ] {
            check_sha256(digest, None)?;
        }
        let mut content = BTreeSet::new();
        for key in &self.ordered_used_content {
            key.validate()
                .map_err(|error| format!("invalid XMAP15 content identity: {error}"))?;
            if !content.insert(key.normalized_game_relative_path.as_str()) {
                return Err("XMAP15 content closure repeats a path".into());
            }
        }
        if self
            .ordered_used_content
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        {
            return Err("XMAP15 content closure is not in deterministic order".into());
        }
        let mut rows = BTreeSet::new();
        let mut plan_keys = BTreeSet::new();
        for row in &self.ordered_rows {
            match row {
                Xmap15PreparedRow::Plan {
                    semantic,
                    definition_sha256,
                } => {
                    semantic.selector.validate()?;
                    semantic.route.validate()?;
                    check_sha256(definition_sha256, None)?;
                    plan_keys.insert(semantic.key.clone());
                    if !rows.insert(format!("p:{:?}", semantic.key)) {
                        return Err("XMAP15 prepared frame repeats a plan row".into());
                    }
                }
                Xmap15PreparedRow::Movement {
                    semantic,
                    definition_sha256,
                } => {
                    semantic.route.validate()?;
                    check_sha256(definition_sha256, None)?;
                    if !rows.insert(format!("m:{:?}", semantic.key)) {
                        return Err("XMAP15 prepared frame repeats a movement row".into());
                    }
                }
            }
        }
        if self.ordered_rows.windows(2).any(|pair| {
            let ordinal = |row: &Xmap15PreparedRow| match row {
                Xmap15PreparedRow::Plan { semantic, .. } => semantic.final_engine_draw_ordinal,
                Xmap15PreparedRow::Movement { semantic, .. } => Some(semantic.final_engine_draw_ordinal),
            };
            match (ordinal(&pair[0]), ordinal(&pair[1])) {
                (Some(a), Some(b)) => a >= b,
                (None, None) => false,
                _ => true,
            }
        }) {
            return Err("XMAP15 prepared rows are not in strict final engine draw order".into());
        }
        if let Some(runtime) = &self.viewer_runtime {
            let retained = runtime.retained_plans.iter().cloned().collect::<BTreeSet<_>>();
            if retained.len() != runtime.retained_plans.len() || !plan_keys.is_subset(&retained) {
                return Err("XMAP15 viewer object census is duplicate or omits a drawn plan".into());
            }
            let mut inputs = BTreeSet::new();
            for draw in &runtime.draw_inputs {
                if !inputs.insert(draw.key.clone())
                    || draw.base_rgba_f32_bits.iter().chain(std::iter::once(&draw.animation_speed_f32_bits))
                        .any(|bits| !f32::from_bits(*bits).is_finite()) {
                    return Err("XMAP15 viewer draw inputs are duplicate or non-finite".into());
                }
            }
            if inputs != plan_keys {
                return Err("XMAP15 viewer draw inputs do not match drawable plans".into());
            }
            plan_keys.extend(retained);
            plan_keys.extend(runtime.refusals.iter().filter_map(|row| row.key.clone()));
        }
        if self
            .selected_plan
            .as_ref()
            .is_some_and(|selected| !plan_keys.contains(selected))
        {
            return Err("XMAP15 selection is not a current-hour plan identity".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn culled_movement_row() -> M03ArrowRow {
        let a0 = 2.0f32;
        let a4 = 3.0f32;
        let c8 = 0.5f32;
        let arrow = 4.0f32;
        let product = arrow * c8;
        M03ArrowRow {
            event_sequence: 12,
            stable_key: M03StableKey::Movement {
                owner_country_id: 1,
                division_id: 42,
                movement_generation: 3,
            },
            definition_name: "moveArrow".into(),
            plan_subtype: None,
            ordered_division_ids: Vec::new(),
            ordered_fixed_point_input_points_1e5: vec![[0, 0], [100_000, 100_000]],
            ordered_engine_working_points_1e5: Some(vec![[0, 0], [100_000, 100_000]]),
            assigned: true,
            active: false,
            selected: false,
            progress_raw_u32: 0,
            fade_in_f32_bits: 0.2f32.to_bits(),
            fade_out_f32_bits: 0.0f32.to_bits(),
            culling_flags_u16: 1,
            visibility: M03Visibility::EngineCulled,
            manager_insertion_ordinal: 4,
            definition_group_ordinal: 2,
            comparator_definition_a0_f32_bits: a0.to_bits(),
            comparator_definition_a4_f32_bits: a4.to_bits(),
            comparator_definition_c8_f32_bits: c8.to_bits(),
            comparator_arrow_178_f32_bits: arrow.to_bits(),
            comparator_primary_f32_bits: (a4.max(a0) + product).to_bits(),
            comparator_tie_17c_f32_bits: 1.0f32.to_bits(),
            draw_members: Vec::new(),
        }
    }

    #[test]
    fn exact_a729_comparator_key_is_accepted() {
        culled_movement_row().validate(10, 20).unwrap();
    }

    #[test]
    fn comparator_drift_fails_closed() {
        let mut row = culled_movement_row();
        row.comparator_primary_f32_bits = 6.0f32.to_bits();
        assert!(row.validate(10, 20).unwrap_err().contains("exact a729 law"));
    }

    #[test]
    fn active_unassigned_row_is_rejected() {
        let mut row = culled_movement_row();
        row.assigned = false;
        row.active = true;
        assert!(row.validate(10, 20).unwrap_err().contains("unassigned"));
    }

    #[test]
    fn exact_plan_selector_uses_engine_file_invasion_name() {
        let selector = Xmap15PlanDefinitionSelector {
            order_type: 3,
            move_special_118: false,
            withdraw_119: false,
            fallback_298: false,
            virtual_299: false,
            blitz_118: false,
            army_group_style_39_and_378: false,
        };
        selector.validate().unwrap();
        assert_eq!(selector.resolved_definition_name(), Some("invasion"));
    }

    #[test]
    fn semantic_producer_registration_is_exact_not_caller_chosen() {
        let frame = Xmap15SemanticFrame {
            schema: XMAP15_SEMANTIC_INPUT_SCHEMA.into(),
            executable_sha256: M03_A729_EXECUTABLE_SHA256.into(),
            producer_registration_sha256: xmap15_semantic_producer_registration_sha256(),
            hour_index: 7,
            observation_sequence: 9,
            baseline_complete: true,
            omitted_plan_rows: 0,
            omitted_movement_rows: 0,
            plan_object_census: 0,
            movement_object_census: 0,
            plans: Vec::new(),
            movements: Vec::new(),
            enumerator_witness_sha256: "a".repeat(64),
        };
        frame.validate().unwrap();
        let mut caller_chosen = frame;
        caller_chosen.producer_registration_sha256 = "b".repeat(64);
        assert!(caller_chosen
            .validate()
            .unwrap_err()
            .contains("unsupported authority"));
    }

    #[test]
    fn exact_signed_fixed5_smoother_preserves_each_truncation() {
        let points = [[-100_001, 50_001], [100_003, -50_003], [300_007, 150_009]];
        let smoothed = smooth_xmap15_points(&points).unwrap();
        let component = |previous: i64, current: i64, next: i64| {
            current * 25_000 / 100_000 + (((previous + next) * 50_000 / 100_000) * 75_000 / 100_000)
        };
        assert_eq!(
            smoothed[1],
            [
                component(points[0][0], points[1][0], points[2][0]) as f32 / 100_000.0,
                component(points[0][1], points[1][1], points[2][1]) as f32 / 100_000.0,
            ]
        );
        assert_eq!(smoothed[0], [-1.00001, 0.50001]);
        assert_eq!(smoothed[2], [3.00007, 1.50009]);
    }

    fn terrain_arrow_fixture() -> (Xmap15PlanSemanticRow, Xmap15PlanGeometryProfile) {
        let row = Xmap15PlanSemanticRow {
            key: Xmap15PlanKey {
                owner_kind: Xmap15OwnerKind::Army,
                owner_id: 7,
                engine_order_instance_id: 9,
                generation: 2,
                slot_index: 0,
            },
            owner_country: "GER".into(),
            selector: Xmap15PlanDefinitionSelector {
                order_type: 2,
                move_special_118: false,
                withdraw_119: false,
                fallback_298: false,
                virtual_299: false,
                blitz_118: false,
                army_group_style_39_and_378: false,
            },
            resolved_definition_name: "offence_line".into(),
            ordered_assigned_division_ids: vec![11],
            assigned: true,
            active: true,
            route: Xmap15PreparedRoute {
                ordered_province_ids: vec![1, 2],
                builder_input_points_fixed5: vec![[0, 0], [100_000, 0]],
                engine_pre_smoother_points_fixed5: None,
                point_width_classes: vec![0, 0],
                explicit_height_f32_bits: None,
                wrap_offset_world: 0,
            },
            manager_insertion_ordinal: Some(0),
            definition_group_ordinal: Some(0),
            object_width_multiplier_f32_bits: 0.0f32.to_bits(),
            comparator_tie_17c_f32_bits: 0.0f32.to_bits(),
            fade_in_f32_bits: 0.0f32.to_bits(),
            fade_out_f32_bits: 0.0f32.to_bits(),
            culling_offset_f32_bits: 0.0f32.to_bits(),
            inner_rgba_f32_bits: [0.0f32.to_bits(); 4],
            outer_rgba_f32_bits: [0.0f32.to_bits(); 4],
            texture_variant: 0,
            see_through: false,
            visible: true,
            final_engine_draw_ordinal: Some(0),
            exact_head_vertices_f32_bits: None,
            lifecycle_tombstone_sha256: Some("a".repeat(64)),
        };
        let one = 1.0f32.to_bits();
        let profile = Xmap15PlanGeometryProfile {
            body_start_width_f32_bits: one,
            body_end_width_f32_bits: one,
            runtime_width_scale_f32_bits: 0.0f32.to_bits(),
            body_uv_scale_f32_bits: one,
            width_class_zero_f32_bits: one,
            width_class_one_f32_bits: one,
            global_scale_min_size_f32_bits: one,
            global_scale_max_points: 1,
            head_length_f32_bits: 0.0f32.to_bits(),
        };
        (row, profile)
    }

    #[test]
    fn viewer_rows_allow_missing_provenance_without_weakening_native_sidecar() {
        let (row, _) = terrain_arrow_fixture();
        let native = Xmap15SemanticFrame {
            schema: XMAP15_SEMANTIC_INPUT_SCHEMA.into(), executable_sha256: M03_A729_EXECUTABLE_SHA256.into(),
            producer_registration_sha256: xmap15_semantic_producer_registration_sha256(),
            hour_index: 17, observation_sequence: 1, baseline_complete: true,
            omitted_plan_rows: 0, omitted_movement_rows: 0, plan_object_census: 1, movement_object_census: 0,
            plans: vec![row.clone()], movements: vec![], enumerator_witness_sha256: "a".repeat(64),
        };
        native.validate().unwrap();
        for field in ["manager_insertion_ordinal", "definition_group_ordinal", "final_engine_draw_ordinal", "lifecycle_tombstone_sha256"] {
            for missing in [false, true] {
                let mut value = serde_json::to_value(&native).unwrap();
                if missing { value["plans"][0].as_object_mut().unwrap().remove(field); }
                else { value["plans"][0][field] = serde_json::Value::Null; }
                assert!(Xmap15SemanticFrame::decode_json(&serde_json::to_vec(&value).unwrap()).is_err());
            }
        }
        let mut first = row;
        first.lifecycle_tombstone_sha256 = None;
        first.manager_insertion_ordinal = None;
        first.definition_group_ordinal = None;
        first.final_engine_draw_ordinal = None;
        let mut second = first.clone();
        second.key.engine_order_instance_id += 1;
        let prepared = Xmap15PreparedFrame {
            schema: XMAP15_STATE_FIRST_SCHEMA.into(), hour_index: 17, observation_sequence: 1,
            semantic_input_sha256: "a".repeat(64), enumerator_witness_sha256: "b".repeat(64),
            xmap13_census_sha256: "c".repeat(64), xmap13_admission_sha256: "d".repeat(64),
            selected_plan: None, viewer_runtime: None, ordered_rows: vec![first.clone(), second].into_iter().map(|semantic|
                Xmap15PreparedRow::Plan { semantic, definition_sha256: "e".repeat(64) }).collect(),
            ordered_used_content: vec![], deterministic_sha256: String::new(),
        }.seal().unwrap();
        let mut duplicate = prepared.clone();
        duplicate.ordered_rows[1] = duplicate.ordered_rows[0].clone();
        assert!(duplicate.seal().is_err());
        let mut mixed = prepared;
        let Xmap15PreparedRow::Plan { semantic, .. } = &mut mixed.ordered_rows[1] else { unreachable!() };
        semantic.final_engine_draw_ordinal = Some(0);
        assert!(mixed.seal().is_err());
    }

    #[test]
    fn paradrop_callback_builds_body_and_head_with_native_index_schedule() {
        let (mut row, mut profile) = terrain_arrow_fixture();
        row.selector.order_type = 4;
        row.resolved_definition_name = "paradrop".into();
        row.route.builder_input_points_fixed5 = vec![[0, 0], [1_000_000, 0], [2_000_000, 0], [3_000_000, 0]];
        row.route.point_width_classes = vec![0; 4];
        let parameters = crate::Xmap15ParadropProfile {
            height_to_length_f32_bits: 1.0f32.to_bits(), max_height_f32_bits: 6.0f32.to_bits(),
        };
        let mut queries = Vec::new();
        let mesh = row.build_paradrop_map_arrow_mesh(profile, 100, parameters, |x, z| {
            queries.push([x, z]); Ok(x)
        }).unwrap();
        assert_eq!(mesh.body_vertex_count, 8);
        assert_eq!(mesh.head_first_vertex, None);
        assert_eq!(queries, [[0.0, 0.0], [30.0, 0.0]].repeat(4));
        assert_eq!(mesh.vertices[2].position[1], 21.0);
        assert_eq!(mesh.vertices[3].position[1], 21.0);
        assert_eq!(mesh.vertices[6].position[1], 0.0);
        assert!(row.build_map_arrow_mesh(profile, 100, |_, _| Ok(0.0)).is_err());

        profile.head_length_f32_bits = 18.0f32.to_bits();
        queries.clear();
        let mesh = row.build_paradrop_map_arrow_mesh(profile, 100, parameters, |x, z| {
            queries.push([x, z]); Ok(x)
        }).unwrap();
        // Three visited body indices, then N+1/N+2 head callbacks; the cut pair is repeated.
        assert_eq!(queries, [[0.0, 0.0], [30.0, 0.0]].repeat(5));
        assert_eq!(mesh.vertices.len(), 12);
        assert_eq!(mesh.head_first_vertex, Some(0));
        assert_eq!(mesh.vertices[0].position[0], 30.0);
        assert_eq!(mesh.vertices[0].position[1], 30.0);
        assert_eq!(mesh.vertices[0].position[1], mesh.vertices[1].position[1]);
        assert_eq!(mesh.vertices[2].position[1], mesh.vertices[4].position[1]);
        assert_eq!(mesh.vertices[4..6], mesh.vertices[6..8]);
        assert_eq!(mesh.vertices[0].uv_is_head_variant, [1.0, 1.0, 1.0, 0.0]);
        row.route.explicit_height_f32_bits = Some(vec![0; 4]);
        assert!(row.build_paradrop_map_arrow_mesh(profile, 100, parameters, |_, _| Ok(0.0)).is_err());
    }

    #[test]
    fn exact_no_head_body_builder_emits_stride32_strip() {
        let (row, profile) = terrain_arrow_fixture();
        let mesh = row.build_map_arrow_mesh(profile, 100, |_x, _z| Ok(0.0)).unwrap();
        assert_eq!(mesh.body_vertex_count, 4);
        assert_eq!(mesh.head_first_vertex, None);
        assert_eq!(mesh.vertices[0].position, [1.0, 0.01, -1.0]);
        assert_eq!(mesh.vertices[1].position, [1.0, 0.01, 1.0]);
        assert_eq!(mesh.vertices[2].position, [0.0, 0.01, -1.0]);
        assert_eq!(mesh.vertices[3].position, [0.0, 0.01, 1.0]);
        assert_eq!(mesh.vertices[0].uv_is_head_variant, [1.0, 1.0, 0.0, 0.0]);

        let mut mixed = row;
        mixed.route.builder_input_points_fixed5 =
            vec![[0, 0], [100_000, 0], [200_000, 0], [300_000, 0]];
        mixed.route.point_width_classes = vec![0, 1, 128, 255];
        mixed.texture_variant = 7;
        let mixed_profile = Xmap15PlanGeometryProfile {
            width_class_zero_f32_bits: 2.0f32.to_bits(),
            width_class_one_f32_bits: 3.0f32.to_bits(),
            ..profile
        };
        let mixed_mesh = mixed
            .build_map_arrow_mesh(mixed_profile, 100, |_x, _z| Ok(0.0))
            .unwrap();
        assert_eq!(mixed_mesh.body_vertex_count, 8);
        assert_eq!(mixed_mesh.head_first_vertex, None);
        assert_eq!(mixed_mesh.vertices.len(), 8);
        for (pair, (signed_class, width)) in mixed_mesh
            .vertices
            .chunks_exact(2)
            .zip([(-1.0, 1.0), (-128.0, 1.0), (1.0, 3.0), (0.0, 2.0)])
        {
            assert_eq!(pair[0].uv_is_head_variant[3], signed_class);
            assert_eq!(pair[1].uv_is_head_variant[3], signed_class);
            assert_eq!(pair[0].position[2], -width);
            assert_eq!(pair[1].position[2], width);
        }
        mixed.texture_variant = 13;
        let other_texture = mixed
            .build_map_arrow_mesh(mixed_profile, 100, |_x, _z| Ok(0.0))
            .unwrap();
        assert_eq!(other_texture.vertices, mixed_mesh.vertices);

        let normalized = mixed.build_map_arrow_mesh(Xmap15PlanGeometryProfile {
            body_uv_scale_f32_bits: 0.0f32.to_bits(), ..mixed_profile
        }, 100, |_x, _z| Ok(0.0)).unwrap();
        let negative_zero = mixed.build_map_arrow_mesh(Xmap15PlanGeometryProfile {
            body_uv_scale_f32_bits: (-0.0f32).to_bits(), ..mixed_profile
        }, 100, |_x, _z| Ok(0.0)).unwrap();
        assert_eq!(normalized.vertices, negative_zero.vertices);
        assert_eq!(normalized.vertices[0].uv_is_head_variant[0], 1.0);
        assert_eq!(mixed_mesh.vertices[0].uv_is_head_variant[0], 3.0);

        mixed.route.builder_input_points_fixed5 = vec![[0, 0], [1_000_000, 0], [2_000_000, 0], [3_000_000, 0]];
        mixed.route.explicit_height_f32_bits = Some([2.0f32, 3.0, 4.0, 99.0].map(f32::to_bits).to_vec());
        let exact_head = [
            [30.0f32, 8.0, -3.0, 0.0, 1.0, 1.0, 1.0, 0.0],
            [30.0, 8.0, 3.0, 0.0, 1.0, 0.0, 1.0, 0.0],
            [12.0, 4.0, -3.0, 0.0, 0.0, 1.0, 1.0, 0.0],
            [12.0, 4.0, 3.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        ].map(|vertex| vertex.map(f32::to_bits));
        mixed.exact_head_vertices_f32_bits = Some(exact_head);
        let headed = mixed.build_map_arrow_mesh(Xmap15PlanGeometryProfile {
            body_end_width_f32_bits: 4.0f32.to_bits(),
            body_uv_scale_f32_bits: 5.0f32.to_bits(),
            head_length_f32_bits: 18.0f32.to_bits(),
            ..profile
        }, 100, |_x, _z| panic!("explicit heights must not read terrain")).unwrap();
        assert_eq!(headed.head_first_vertex, Some(0));
        assert_eq!(headed.body_vertex_count, 8);
        assert_eq!(headed.vertices.len(), 12);
        for (vertex, exact) in headed.vertices[..4].iter().zip(exact_head) {
            assert_eq!(vertex.position.map(f32::to_bits), exact[..3]);
            assert_eq!(vertex.uv_is_head_variant.map(f32::to_bits), exact[4..]);
        }
        assert_eq!(headed.vertices[4..6], headed.vertices[6..8]);
        assert_eq!(headed.vertices[4].position, [12.0, 4.0, -3.0]);
        assert_eq!(headed.vertices[5].position, [12.0, 4.0, 3.0]);
        assert_eq!(headed.vertices[4].uv_is_head_variant, [4.0, 1.0, 0.0, -128.0]);
        assert_eq!(headed.vertices[8].position, [10.0, 3.0, -2.0]);
        assert_eq!(headed.vertices[10].position, [0.0, 2.0, -1.0]);
    }
    #[test]
    fn terrain_heads_use_unclassed_width_wrap_sampling_and_rewrite_the_sloping_base() {
        let (mut row, mut profile) = terrain_arrow_fixture();
        row.route.builder_input_points_fixed5 = vec![[0, 0], [1_000_000, 0], [2_000_000, 0], [3_000_000, 0]];
        row.route.point_width_classes = vec![0; 4];
        row.route.wrap_offset_world = 20;
        profile.body_end_width_f32_bits = 4.0f32.to_bits();
        profile.width_class_zero_f32_bits = 2.0f32.to_bits();
        profile.head_length_f32_bits = 18.0f32.to_bits();
        let mut samples = Vec::new();
        let mesh = row.build_map_arrow_mesh(profile, 20, |x, z| {
            samples.push([x, z]); Ok(0.0)
        }).unwrap();
        assert_eq!(mesh.head_first_vertex, Some(0));
        assert_eq!(mesh.body_vertex_count, 8);
        assert_eq!(mesh.vertices.len(), 12);
        assert_eq!(samples, [
            [0.5, 2.5], [0.5, -1.5], [10.5, 4.5], [10.5, -3.5],
            [12.5, 6.5], [12.5, -5.5], [12.5, 3.5], [12.5, -2.5],
            [10.5, 3.5], [10.5, -2.5],
        ]);
        for (vertex, (position, uv)) in mesh.vertices[..4].iter().zip([
            ([30.0, 0.01, -3.0], [1.0, 1.0, 1.0, 0.0]),
            ([30.0, 0.01, 3.0], [1.0, 0.0, 1.0, 0.0]),
            ([12.0, 0.01, -3.0], [0.0, 1.0, 1.0, 0.0]),
            ([12.0, 0.01, 3.0], [0.0, 0.0, 1.0, 0.0]),
        ]) {
            assert_eq!(vertex.position.map(f32::to_bits), position.map(f32::to_bits));
            assert_eq!(vertex.uv_is_head_variant, uv);
        }
        assert_eq!(mesh.vertices[4].position, [12.0, 0.01, -6.0]);
        let slope = row.build_map_arrow_mesh(profile, 20, |_x, z| Ok(z)).unwrap();
        let a = slope.vertices[2].position;
        let b = slope.vertices[3].position;
        let span = ((b[0]-a[0]).powi(2)+(b[1]-a[1]).powi(2)+(b[2]-a[2]).powi(2)).sqrt();
        assert!((span - 6.0).abs() < 0.000002);
        assert!((b[2] - 3.0).abs() > 0.5, "head base must be rebuilt along the 3D body cross-section");
        assert_eq!(slope.vertices[0].position, [30.0, -2.5 + 0.01, -3.0]);
        let mut samples = Vec::new();
        row.build_map_arrow_mesh(profile, 10, |x, z| { samples.push([x,z]); Ok(0.0) }).unwrap();
        assert_eq!(samples[8][0], 20.5, "native wrapping subtracts width once, not modulo");
        row.route.builder_input_points_fixed5 = vec![[0, 0], [0, 1_000_000], [0, 2_000_000], [0, 3_000_000]];
        let mut samples = Vec::new();
        row.build_map_arrow_mesh(profile, 20, |x,z| { samples.push([x,z]); Ok(0.0) }).unwrap();
        assert_eq!(samples[6], [17.5, 12.5]);
        assert_eq!(samples[8], [17.5, 30.5]);
    }

    #[test]
    fn terrain_head_inputs_fail_by_name_without_fabricating_callback_or_width() {
        let (mut row, mut profile) = terrain_arrow_fixture();
        profile.head_length_f32_bits = 0.25f32.to_bits();
        for width in [0, u32::MAX] {
            assert!(row.build_map_arrow_mesh(profile, width, |_,_| panic!("unadmitted width"))
                .unwrap_err().contains("authenticated map width"));
        }
        let mesh = row.build_map_arrow_mesh(profile, 100, |_,_| Ok(0.0)).unwrap();
        row.exact_head_vertices_f32_bits = Some(std::array::from_fn(|index| {
            let v = mesh.vertices[index];
            [v.position[0], v.position[1], v.position[2], v.padding,
                v.uv_is_head_variant[0], v.uv_is_head_variant[1], v.uv_is_head_variant[2], v.uv_is_head_variant[3]].map(f32::to_bits)
        }));
        assert_eq!(row.build_map_arrow_mesh(profile, 100, |_,_| Ok(0.0)).unwrap(), mesh);
        row.exact_head_vertices_f32_bits.as_mut().unwrap()[0][0] = 4.0f32.to_bits();
        assert!(row.build_map_arrow_mesh(profile, 100, |_,_| Ok(0.0)).unwrap_err().contains("disagrees"));
        row.exact_head_vertices_f32_bits = None;
        row.route.explicit_height_f32_bits = Some(vec![0; 2]);
        assert!(row.build_map_arrow_mesh(profile, 100, |_,_| panic!("callback path"))
            .unwrap_err().contains("callback lacks exact head"));
        row.route.explicit_height_f32_bits = None;
        row.selector.order_type = 4;
        assert!(row.build_map_arrow_mesh(profile, 100, |_,_| panic!("missing paradrop callback"))
            .unwrap_err().contains("height callback inputs are unavailable"));
    }

}
