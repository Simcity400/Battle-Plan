//! Fail-closed XN13 dynamic 3D-unit presentation admission.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};
use snapshot::sha256::Sha256;

use crate::m01_runtime_admission::M01AdmittedRuntimeRows;

pub const XN13_INPUT_SCHEMA: &str = "hoi4-xn13-unit-presentation-input-v1";
const PALETTE_HEX_LEN: usize = 50 * 16 * 4 * 2;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Xn13AnimationBinding {
    NoClip,
    Clip {
        animation_name: String,
        phase_f32_bits: u32,
        speed_f32_bits: u32,
    },
    Transition {
        from_state: String,
        from_animation: String,
        from_phase_f32_bits: u32,
        to_state: String,
        to_animation: String,
        to_phase_f32_bits: u32,
        elapsed_f32_bits: u32,
        duration_f32_bits: u32,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Xn13DamageVariant {
    Intact,
    AssetState { state_name: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Xn13SelectionEffect {
    NotSelected,
    SeparateEffect { effect_token: String },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Xn13EffectKind {
    Particle,
    Sound,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Xn13EffectTrigger {
    pub state_event_index: u32,
    pub effect_kind: Xn13EffectKind,
    pub effect_token: String,
    pub node: String,
    pub event_time_f32_bits: u32,
    pub keep_particle: bool,
    pub trigger_once: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Xn13PoseBinding {
    NotSkinned,
    Palette {
        sha256: String,
        le_hex_3200_bytes: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Xn13PassPoses {
    pub color_a: Xn13PoseBinding,
    pub light_depth: Xn13PoseBinding,
    pub main_hdr: Xn13PoseBinding,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Xn13PresentationRow {
    pub clock: u32,
    pub stable_unit_id: String,
    pub entity_name: String,
    pub model_name: String,
    pub state_name: String,
    pub state_enter_clock: u32,
    pub animation: Xn13AnimationBinding,
    pub selected_lod_index: u16,
    pub camera_distance_f32_bits: u32,
    pub selected_loddist_f32_bits: u32,
    pub material_variant: String,
    pub damage_variant: Xn13DamageVariant,
    pub selection_effect: Xn13SelectionEffect,
    pub state_effect_triggers: Vec<Xn13EffectTrigger>,
    pub pass_poses: Xn13PassPoses,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Xn13InputEnvelope {
    pub schema: String,
    pub rows: Vec<Xn13PresentationRow>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Xn13AdmittedPresentationRows {
    rows: Vec<Xn13PresentationRow>,
}

impl Xn13AdmittedPresentationRows {
    pub fn rows(&self) -> impl ExactSizeIterator<Item = &Xn13PresentationRow> {
        self.rows.iter()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Xn13AdmissionError {
    InvalidJson(String),
    UnsupportedSchema(String),
    EmptyRows,
    EmptyField {
        field: &'static str,
        row: usize,
    },
    InvalidField {
        field: &'static str,
        row: usize,
        reason: &'static str,
    },
    DuplicateStableUnitAtClock {
        row: usize,
    },
    M01RowSetMismatch,
}

impl fmt::Display for Xn13AdmissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(error) => write!(f, "invalid XN13 runtime JSON: {error}"),
            Self::UnsupportedSchema(schema) => write!(f, "unsupported XN13 schema {schema:?}"),
            Self::EmptyRows => write!(f, "XN13 input has no rows"),
            Self::EmptyField { field, row } => write!(f, "XN13 row {row} has empty field {field}"),
            Self::InvalidField { field, row, reason } => {
                write!(f, "XN13 row {row} has invalid field {field}: {reason}")
            }
            Self::DuplicateStableUnitAtClock { row } => {
                write!(f, "XN13 row {row} duplicates a stable unit at its clock")
            }
            Self::M01RowSetMismatch => {
                write!(f, "XN13 rows do not exactly cover admitted M01 rows")
            }
        }
    }
}

impl Error for Xn13AdmissionError {}

pub fn parse_and_admit_xn13_rows(
    json: &str,
    m01: &M01AdmittedRuntimeRows,
) -> Result<Xn13AdmittedPresentationRows, Xn13AdmissionError> {
    let envelope: Xn13InputEnvelope = serde_json::from_str(json)
        .map_err(|error| Xn13AdmissionError::InvalidJson(error.to_string()))?;
    if envelope.schema != XN13_INPUT_SCHEMA {
        return Err(Xn13AdmissionError::UnsupportedSchema(envelope.schema));
    }
    admit_xn13_rows(&envelope.rows, m01)
}

pub fn admit_xn13_rows(
    rows: &[Xn13PresentationRow],
    m01: &M01AdmittedRuntimeRows,
) -> Result<Xn13AdmittedPresentationRows, Xn13AdmissionError> {
    if rows.is_empty() {
        return Err(Xn13AdmissionError::EmptyRows);
    }
    let mut keys = BTreeSet::new();
    for (index, row) in rows.iter().enumerate() {
        validate_row(row, index)?;
        if !keys.insert((row.clock, row.stable_unit_id.as_str())) {
            return Err(Xn13AdmissionError::DuplicateStableUnitAtClock { row: index });
        }
    }
    let m01_keys: BTreeSet<_> = m01
        .render_rows()
        .map(|row| (row.clock, row.stable_unit_id.as_str()))
        .collect();
    if keys != m01_keys {
        return Err(Xn13AdmissionError::M01RowSetMismatch);
    }
    Ok(Xn13AdmittedPresentationRows {
        rows: rows.to_vec(),
    })
}

fn validate_row(row: &Xn13PresentationRow, index: usize) -> Result<(), Xn13AdmissionError> {
    for (field, value) in [
        ("stable_unit_id", row.stable_unit_id.as_str()),
        ("entity_name", row.entity_name.as_str()),
        ("model_name", row.model_name.as_str()),
        ("state_name", row.state_name.as_str()),
        ("material_variant", row.material_variant.as_str()),
    ] {
        nonempty(value, field, index)?;
    }
    if row.state_enter_clock > row.clock {
        return invalid("state_enter_clock", index, "must not exceed sampled clock");
    }
    validate_animation(&row.animation, index)?;
    finite_nonnegative(
        row.camera_distance_f32_bits,
        "camera_distance_f32_bits",
        index,
    )?;
    finite_nonnegative(
        row.selected_loddist_f32_bits,
        "selected_loddist_f32_bits",
        index,
    )?;
    if let Xn13DamageVariant::AssetState { state_name } = &row.damage_variant {
        nonempty(state_name, "damage_variant.state_name", index)?;
    }
    if let Xn13SelectionEffect::SeparateEffect { effect_token } = &row.selection_effect {
        nonempty(effect_token, "selection_effect.effect_token", index)?;
    }
    let mut event_indices = BTreeSet::new();
    for trigger in &row.state_effect_triggers {
        if !event_indices.insert(trigger.state_event_index) {
            return invalid(
                "state_effect_triggers",
                index,
                "duplicate state event index",
            );
        }
        nonempty(
            &trigger.effect_token,
            "state_effect_triggers.effect_token",
            index,
        )?;
        finite_nonnegative(
            trigger.event_time_f32_bits,
            "state_effect_triggers.event_time",
            index,
        )?;
    }
    let poses = [
        &row.pass_poses.color_a,
        &row.pass_poses.light_depth,
        &row.pass_poses.main_hdr,
    ];
    let skinned = matches!(poses[0], Xn13PoseBinding::Palette { .. });
    if poses
        .iter()
        .any(|pose| matches!(pose, Xn13PoseBinding::Palette { .. }) != skinned)
    {
        return invalid(
            "pass_poses",
            index,
            "skinned tag must agree across all three passes",
        );
    }
    for pose in poses {
        validate_pose(pose, index)?;
    }
    Ok(())
}

fn validate_animation(value: &Xn13AnimationBinding, row: usize) -> Result<(), Xn13AdmissionError> {
    match value {
        Xn13AnimationBinding::NoClip => Ok(()),
        Xn13AnimationBinding::Clip {
            animation_name,
            phase_f32_bits,
            speed_f32_bits,
        } => {
            nonempty(animation_name, "animation.animation_name", row)?;
            finite_nonnegative(*phase_f32_bits, "animation.phase", row)?;
            finite(*speed_f32_bits, "animation.speed", row)
        }
        Xn13AnimationBinding::Transition {
            from_state,
            from_animation,
            from_phase_f32_bits,
            to_state,
            to_animation,
            to_phase_f32_bits,
            elapsed_f32_bits,
            duration_f32_bits,
        } => {
            for (field, value) in [
                ("animation.from_state", from_state.as_str()),
                ("animation.from_animation", from_animation.as_str()),
                ("animation.to_state", to_state.as_str()),
                ("animation.to_animation", to_animation.as_str()),
            ] {
                nonempty(value, field, row)?;
            }
            finite_nonnegative(*from_phase_f32_bits, "animation.from_phase", row)?;
            finite_nonnegative(*to_phase_f32_bits, "animation.to_phase", row)?;
            finite_nonnegative(*elapsed_f32_bits, "animation.elapsed", row)?;
            finite_nonnegative(*duration_f32_bits, "animation.duration", row)?;
            let elapsed = f32::from_bits(*elapsed_f32_bits);
            let duration = f32::from_bits(*duration_f32_bits);
            if duration <= 0.0 || elapsed > duration {
                return invalid(
                    "animation.transition",
                    row,
                    "requires 0 <= elapsed <= positive duration",
                );
            }
            Ok(())
        }
    }
}

fn validate_pose(value: &Xn13PoseBinding, row: usize) -> Result<(), Xn13AdmissionError> {
    let Xn13PoseBinding::Palette {
        sha256,
        le_hex_3200_bytes,
    } = value
    else {
        return Ok(());
    };
    if !canonical_hex(sha256, 64) {
        return invalid("pass_poses.sha256", row, "expected canonical SHA-256 hex");
    }
    if !canonical_hex(le_hex_3200_bytes, PALETTE_HEX_LEN) {
        return invalid(
            "pass_poses.le_hex_3200_bytes",
            row,
            "expected 3,200 canonical bytes",
        );
    }
    let bytes = decode_hex(le_hex_3200_bytes);
    if Sha256::hex(&bytes) != *sha256 {
        return invalid("pass_poses.sha256", row, "does not match palette bytes");
    }
    Ok(())
}

fn nonempty(value: &str, field: &'static str, row: usize) -> Result<(), Xn13AdmissionError> {
    if value.trim().is_empty() {
        Err(Xn13AdmissionError::EmptyField { field, row })
    } else {
        Ok(())
    }
}

fn finite(bits: u32, field: &'static str, row: usize) -> Result<(), Xn13AdmissionError> {
    if f32::from_bits(bits).is_finite() {
        Ok(())
    } else {
        invalid(field, row, "must be finite f32 bits")
    }
}

fn finite_nonnegative(
    bits: u32,
    field: &'static str,
    row: usize,
) -> Result<(), Xn13AdmissionError> {
    finite(bits, field, row)?;
    if f32::from_bits(bits) >= 0.0 {
        Ok(())
    } else {
        invalid(field, row, "must be nonnegative")
    }
}

fn invalid<T>(
    field: &'static str,
    row: usize,
    reason: &'static str,
) -> Result<T, Xn13AdmissionError> {
    Err(Xn13AdmissionError::InvalidField { field, row, reason })
}

fn canonical_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn decode_hex(value: &str) -> Vec<u8> {
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            fn nibble(value: u8) -> u8 {
                if value <= b'9' {
                    value - b'0'
                } else {
                    value - b'a' + 10
                }
            }
            nibble(pair[0]) << 4 | nibble(pair[1])
        })
        .collect()
}
