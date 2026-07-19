//! Portable M03 order and movement-arrow carrier.
//!
//! Every value is either an engine-authored semantic value or an exact draw
//! packet. Playback never reconstructs curves from province centroids, chooses
//! definitions from colour, or tunes constants against captured pixels.

use super::{check_sha256, decimal_u64, validate_static_map_draw, StaticMapDraw, UsedContentKey};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const M03_ARROW_SCHEMA: &str = "hoi4.production-m03-order-movement-arrows.v1";
pub const M03_A729_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const M03_STATIC_PROOF_SHA256: &str =
    "040a1fdf596845eb9ec8b4d07086b00a129b3f38d2cd947a7439781073e31efe";

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
                    M03PlanSubtype::NavalInvasion => "naval_invasion",
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
}
