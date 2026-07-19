//! Portable M02 combat-indicator carrier.
//!
//! The extractor and renderer communicate only through this value.  The
//! carrier intentionally contains the exact evaluated indicator inputs and
//! exact draw packets.  It does not derive combat presentation from a save,
//! a frozen vanilla catalog, screen pixels, or process-local pointers.

use super::{
    check_sha256, decimal_u64, validate_static_map_draw, PreparedRenderTarget, StaticMapDraw,
    UsedContentKey,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const M02_COMBAT_SCHEMA: &str = "hoi4.production-m02-combat-indicators.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct M02CombatFrame {
    pub schema: String,
    #[serde(with = "decimal_u64")]
    pub update_index: u64,
    #[serde(with = "decimal_u64")]
    pub first_sequence: u64,
    #[serde(with = "decimal_u64")]
    pub last_sequence: u64,
    /// Complete authenticated draw frontier of the containing UI pass. Every
    /// M02 draw ordinal is global in that pass and must be below this count.
    pub ui_pass_draw_count: u32,
    /// The five discriminating laws accepted for this carrier version.  This
    /// must be the exact A-E set; it is not per-row gameplay state.
    pub validated_laws: Vec<M02LawState>,
    pub indicators: Vec<M02CombatIndicator>,
    /// Exact cross-indicator draw order.  No consumer may sort by position,
    /// combat handle, progress, or asset name.
    pub ordered_draws: Vec<M02CombatDrawRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum M02LawState {
    AActiveBaseline,
    BZoomBoundary,
    CResultLifecycle,
    DPortStrikeDiscriminator,
    EVisibilityControl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum M02CombatKind {
    LandActive,
    NavalActive,
    NavalResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExactM02FieldStatus {
    Recorded,
    NotApplicable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExactM02Field<T> {
    pub status: ExactM02FieldStatus,
    pub value: Option<T>,
}

impl<T> ExactM02Field<T> {
    pub fn recorded(value: T) -> Self {
        Self {
            status: ExactM02FieldStatus::Recorded,
            value: Some(value),
        }
    }

    pub fn not_applicable() -> Self {
        Self {
            status: ExactM02FieldStatus::NotApplicable,
            value: None,
        }
    }

    fn validate(&self, name: &str) -> Result<(), String> {
        match (self.status, self.value.is_some()) {
            (ExactM02FieldStatus::Recorded, true) | (ExactM02FieldStatus::NotApplicable, false) => {
                Ok(())
            }
            _ => Err(format!("M02 {name} status/value mismatch")),
        }
    }

    fn is_recorded(&self) -> bool {
        self.status == ExactM02FieldStatus::Recorded && self.value.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct M02CombatIndicator {
    /// Exact eight-byte engine combat handle in little-endian byte order.
    /// Process-local semantic object pointers are forbidden.
    pub stable_combat_handle_le_hex: String,
    pub kind: M02CombatKind,
    #[serde(with = "decimal_u64")]
    pub lifecycle_sequence: u64,
    pub location_province_id: ExactM02Field<u32>,
    pub participants: ExactM02Field<M02Participants>,
    pub progress_percent: ExactM02Field<u8>,
    pub direction: ExactM02Field<M02Direction>,
    pub result: ExactM02Field<M02ResultState>,
    pub observer_country_id: ExactM02Field<u32>,
    pub fog_of_war: ExactM02Field<M02FogState>,
    pub visible: ExactM02Field<bool>,
    pub overlap: ExactM02Field<M02OverlapState>,
    /// Exact input to the `350.0f < scale` selector, not reconstructed camera
    /// height.  `small_scale` is the authenticated comparator result.
    pub scale_f32_bits: ExactM02Field<u32>,
    pub small_scale: ExactM02Field<bool>,
    pub raw_widget_states: Vec<M02WidgetState>,
    /// Content actually used by this indicator, resolved through the effective
    /// playback VFS.  No vanilla asset path or payload is supplied by default.
    pub used_content: Vec<UsedContentKey>,
    pub draws: Vec<M02CombatDraw>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct M02Participants {
    pub attacker_country_id: u32,
    pub defender_country_id: u32,
    pub attacker_source_id: String,
    pub defender_source_id: String,
    pub attacker_selector_byte_0xba: u8,
    pub defender_selector_byte_0xba: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum M02Direction {
    AttackerToDefender,
    DefenderToAttacker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum M02FogState {
    Visible,
    Hidden,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct M02OverlapState {
    pub group_id: String,
    pub depth_ordinal: u32,
    pub anchor_world_xyz_f32_bits: [u32; 3],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct M02ResultState {
    pub winner_country_id: u32,
    pub observer_relation: M02ObserverRelation,
    /// Exact producer byte copied from active +0x10C to result +0x128.
    pub port_strike_byte_0x128: u8,
    pub presentation: M02ResultPresentation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum M02ObserverRelation {
    Winner,
    Loser,
    Neutral,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum M02ResultPresentation {
    NavalCombat,
    PortStrike,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct M02WidgetState {
    pub semantic_role: String,
    pub owner_offset: u32,
    /// Pointer-free, canonical raw widget state actually consumed by the draw.
    pub canonical_bytes: Vec<u8>,
    pub canonical_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct M02CombatDraw {
    pub draw_ordinal: u32,
    pub semantic_role: String,
    pub draw: StaticMapDraw,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct M02CombatDrawRef {
    pub stable_combat_handle_le_hex: String,
    pub draw_ordinal: u32,
}

impl M02CombatFrame {
    pub fn draw(&self, reference: &M02CombatDrawRef) -> Option<&M02CombatDraw> {
        self.indicators
            .iter()
            .find(|indicator| {
                indicator.stable_combat_handle_le_hex == reference.stable_combat_handle_le_hex
            })
            .and_then(|indicator| {
                indicator
                    .draws
                    .iter()
                    .find(|draw| draw.draw_ordinal == reference.draw_ordinal)
            })
    }

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
        if self.schema != M02_COMBAT_SCHEMA {
            return Err("unsupported M02 combat-indicator schema".into());
        }
        if self.first_sequence > self.last_sequence {
            return Err("M02 sequence range is reversed".into());
        }
        if self.ui_pass_draw_count == 0 {
            return Err("M02 UI-pass draw frontier is zero".into());
        }
        let laws = self.validated_laws.iter().copied().collect::<BTreeSet<_>>();
        let expected = [
            M02LawState::AActiveBaseline,
            M02LawState::BZoomBoundary,
            M02LawState::CResultLifecycle,
            M02LawState::DPortStrikeDiscriminator,
            M02LawState::EVisibilityControl,
        ]
        .into_iter()
        .collect::<BTreeSet<_>>();
        if laws != expected || self.validated_laws.len() != expected.len() {
            return Err("M02 carrier does not bind the exact A-E law set".into());
        }

        let mut handles = BTreeSet::new();
        let mut available_draws = BTreeSet::new();
        for indicator in &self.indicators {
            indicator.validate(self.first_sequence, self.last_sequence)?;
            if !handles.insert(indicator.stable_combat_handle_le_hex.clone()) {
                return Err("duplicate M02 stable combat handle".into());
            }
            for draw in &indicator.draws {
                if !available_draws.insert(M02CombatDrawRef {
                    stable_combat_handle_le_hex: indicator.stable_combat_handle_le_hex.clone(),
                    draw_ordinal: draw.draw_ordinal,
                }) {
                    return Err("duplicate M02 combat draw identity".into());
                }
            }
        }
        let ordered = self.ordered_draws.iter().cloned().collect::<BTreeSet<_>>();
        if ordered.len() != self.ordered_draws.len() || ordered != available_draws {
            return Err("M02 ordered draw list is not an exact one-to-one cover".into());
        }
        let mut previous = None;
        for output in &self.ordered_draws {
            if output.draw_ordinal >= self.ui_pass_draw_count {
                return Err("M02 draw ordinal exceeds the authenticated UI-pass frontier".into());
            }
            if previous.is_some_and(|ordinal| output.draw_ordinal <= ordinal) {
                return Err("M02 ordered draws change the authenticated UI-pass order".into());
            }
            previous = Some(output.draw_ordinal);
        }
        Ok(())
    }
}

impl M02CombatIndicator {
    fn validate(&self, first_sequence: u64, last_sequence: u64) -> Result<(), String> {
        validate_handle(&self.stable_combat_handle_le_hex)?;
        if self.lifecycle_sequence < first_sequence || self.lifecycle_sequence > last_sequence {
            return Err("M02 lifecycle sequence is outside frame range".into());
        }
        self.location_province_id.validate("location")?;
        self.participants.validate("participants")?;
        self.progress_percent.validate("progress")?;
        self.direction.validate("direction")?;
        self.result.validate("result")?;
        self.observer_country_id.validate("observer country")?;
        self.fog_of_war.validate("fog of war")?;
        self.visible.validate("visibility")?;
        self.overlap.validate("overlap")?;
        self.scale_f32_bits.validate("scale")?;
        self.small_scale.validate("small-scale selector")?;

        if self.progress_percent.value.is_some_and(|value| value > 100) {
            return Err("M02 progress is above 100".into());
        }
        if let Some(participants) = &self.participants.value {
            if participants.attacker_source_id.is_empty()
                || participants.defender_source_id.is_empty()
                || participants.attacker_selector_byte_0xba > 1
                || participants.defender_selector_byte_0xba > 1
            {
                return Err("M02 participant carrier is not canonical".into());
            }
        }
        if let Some(result) = &self.result.value {
            if result.port_strike_byte_0x128 > 1
                || (result.port_strike_byte_0x128 == 1
                    && result.presentation != M02ResultPresentation::PortStrike)
                || (result.port_strike_byte_0x128 == 0
                    && result.presentation != M02ResultPresentation::NavalCombat)
            {
                return Err("M02 result byte/presentation mismatch".into());
            }
        }
        if let (Some(bits), Some(small)) = (self.scale_f32_bits.value, self.small_scale.value) {
            let scale = f32::from_bits(bits);
            if !scale.is_finite() {
                return Err("M02 scale is not finite".into());
            }
            let exact = 350.0f32 < scale;
            if exact != small {
                return Err("M02 small-scale result disagrees with exact 350.0f comparator".into());
            }
        }

        match self.kind {
            M02CombatKind::LandActive => {
                require_recorded(&self.location_province_id, "land location")?;
                require_recorded(&self.participants, "land participants")?;
                require_recorded(&self.progress_percent, "land progress")?;
                require_recorded(&self.direction, "land direction")?;
                require_not_applicable(&self.result, "land result")?;
            }
            M02CombatKind::NavalActive => {
                require_recorded(&self.location_province_id, "naval location")?;
                require_recorded(&self.participants, "naval participants")?;
                require_recorded(&self.progress_percent, "naval progress")?;
                require_not_applicable(&self.result, "active naval result")?;
                require_recorded(&self.scale_f32_bits, "naval scale")?;
                require_recorded(&self.small_scale, "naval scale selector")?;
            }
            M02CombatKind::NavalResult => {
                require_recorded(&self.result, "naval result")?;
                require_not_applicable(&self.progress_percent, "result progress")?;
                require_not_applicable(&self.direction, "result direction")?;
            }
        }
        require_recorded(&self.observer_country_id, "observer country")?;
        require_recorded(&self.fog_of_war, "fog of war")?;
        require_recorded(&self.visible, "visibility")?;
        require_recorded(&self.overlap, "overlap")?;

        let mut roles = BTreeSet::new();
        for state in &self.raw_widget_states {
            if state.semantic_role.trim().is_empty() || !roles.insert(state.semantic_role.as_str())
            {
                return Err("M02 widget roles are empty or duplicated".into());
            }
            if state.canonical_bytes.is_empty() {
                return Err("M02 widget state is empty".into());
            }
            check_sha256(&state.canonical_sha256, Some(&state.canonical_bytes))?;
        }
        if self.raw_widget_states.is_empty() {
            return Err("M02 indicator has no authenticated widget state".into());
        }

        let mut content = BTreeSet::new();
        for key in &self.used_content {
            key.validate()
                .map_err(|error| format!("invalid M02 UsedContentKey: {error}"))?;
            if key.normalized_game_relative_path.trim().is_empty()
                || !content.insert(key.normalized_game_relative_path.as_str())
            {
                return Err("M02 used-content paths are empty or duplicated".into());
            }
        }
        if self.used_content.is_empty() {
            return Err("M02 indicator has no VFS-bound used content".into());
        }

        let mut draw_ordinals = BTreeSet::new();
        for draw in &self.draws {
            if draw.semantic_role.trim().is_empty() || !draw_ordinals.insert(draw.draw_ordinal) {
                return Err("M02 draw role is empty or ordinal is duplicated".into());
            }
            validate_static_map_draw(&draw.draw)?;
            if draw.draw.output_state.ordered_color_targets.len() != 1
                || draw.draw.output_state.ordered_color_targets[0].prepared_target
                    != PreparedRenderTarget::SwapchainBackBuffer
                || draw.draw.output_state.depth_target.is_some()
            {
                return Err("M02 draw is not an exact depth-free swapchain UI-overlay draw".into());
            }
        }
        if self.visible.value == Some(true) && self.draws.is_empty() {
            return Err("visible M02 indicator has no exact draw packet".into());
        }
        if self.visible.value == Some(false) && !self.draws.is_empty() {
            return Err("hidden M02 indicator submitted a draw".into());
        }
        Ok(())
    }
}

fn require_recorded<T>(field: &ExactM02Field<T>, name: &str) -> Result<(), String> {
    if field.is_recorded() {
        Ok(())
    } else {
        Err(format!("M02 {name} is not recorded"))
    }
}

fn require_not_applicable<T>(field: &ExactM02Field<T>, name: &str) -> Result<(), String> {
    if field.status == ExactM02FieldStatus::NotApplicable && field.value.is_none() {
        Ok(())
    } else {
        Err(format!("M02 {name} is unexpectedly applicable"))
    }
}

fn validate_handle(value: &str) -> Result<(), String> {
    if value.len() != 16
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || value == "0000000000000000"
    {
        return Err("M02 stable combat handle is not 16 lowercase hex digits".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_content_manifest::ContentLayerKind;
    use sha2::{Digest, Sha256};

    fn hash(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn content() -> UsedContentKey {
        UsedContentKey {
            effective_vfs_manifest_sha256: "a".repeat(64),
            normalized_game_relative_path: "interface/navalcombatview.gfx".into(),
            winning_layer_ordinal: 2,
            layer_kind: ContentLayerKind::Mod,
            layer_id: "mod/test".into(),
            source_sha256: "b".repeat(64),
            payload_sha256: "c".repeat(64),
            payload_bytes: 64,
        }
    }

    fn hidden_indicator() -> M02CombatIndicator {
        let widget = vec![1, 2, 3, 4];
        M02CombatIndicator {
            stable_combat_handle_le_hex: "000250be00001269".into(),
            kind: M02CombatKind::NavalActive,
            lifecycle_sequence: 7,
            location_province_id: ExactM02Field::recorded(8819),
            participants: ExactM02Field::recorded(M02Participants {
                attacker_country_id: 1,
                defender_country_id: 2,
                attacker_source_id: "task_force:1".into(),
                defender_source_id: "task_force:2".into(),
                attacker_selector_byte_0xba: 0,
                defender_selector_byte_0xba: 1,
            }),
            progress_percent: ExactM02Field::recorded(92),
            direction: ExactM02Field::not_applicable(),
            result: ExactM02Field::not_applicable(),
            observer_country_id: ExactM02Field::recorded(1),
            fog_of_war: ExactM02Field::recorded(M02FogState::Hidden),
            visible: ExactM02Field::recorded(false),
            overlap: ExactM02Field::recorded(M02OverlapState {
                group_id: "naval:8819".into(),
                depth_ordinal: 0,
                anchor_world_xyz_f32_bits: [0; 3],
            }),
            scale_f32_bits: ExactM02Field::recorded(355.158_57f32.to_bits()),
            small_scale: ExactM02Field::recorded(true),
            raw_widget_states: vec![M02WidgetState {
                semantic_role: "progress_small".into(),
                owner_offset: 0x120,
                canonical_sha256: hash(&widget),
                canonical_bytes: widget,
            }],
            used_content: vec![content()],
            draws: vec![],
        }
    }

    fn frame() -> M02CombatFrame {
        M02CombatFrame {
            schema: M02_COMBAT_SCHEMA.into(),
            update_index: 3,
            first_sequence: 1,
            last_sequence: 9,
            ui_pass_draw_count: 1166,
            validated_laws: vec![
                M02LawState::AActiveBaseline,
                M02LawState::BZoomBoundary,
                M02LawState::CResultLifecycle,
                M02LawState::DPortStrikeDiscriminator,
                M02LawState::EVisibilityControl,
            ],
            indicators: vec![hidden_indicator()],
            ordered_draws: vec![],
        }
    }

    #[test]
    fn hidden_exact_indicator_round_trips_without_a_draw() {
        let encoded = frame().encode_json().unwrap();
        assert_eq!(M02CombatFrame::decode_json(&encoded).unwrap(), frame());
        assert!(!String::from_utf8(encoded)
            .unwrap()
            .contains("semantic_identity"));
    }

    #[test]
    fn rejects_missing_law_and_unknown_fields() {
        let mut missing = frame();
        missing.validated_laws.pop();
        assert!(missing.validate().unwrap_err().contains("A-E"));

        let mut value = serde_json::to_value(frame()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("guess".into(), 1.into());
        assert!(M02CombatFrame::decode_json(&serde_json::to_vec(&value).unwrap()).is_err());
    }

    #[test]
    fn rejects_inferred_scale_and_hidden_draw_substitution() {
        let mut bad = frame();
        bad.indicators[0].small_scale = ExactM02Field::recorded(false);
        assert!(bad.validate().unwrap_err().contains("350.0f"));

        let mut bad_status = frame();
        bad_status.indicators[0].visible = ExactM02Field {
            status: ExactM02FieldStatus::Recorded,
            value: None,
        };
        assert!(bad_status.validate().unwrap_err().contains("status/value"));
    }

    #[test]
    fn rejects_missing_ui_pass_frontier() {
        let mut bad = frame();
        bad.ui_pass_draw_count = 0;
        assert!(bad.validate().unwrap_err().contains("frontier"));
    }
}
