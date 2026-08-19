
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};

pub const M01_RUNTIME_INPUT_SCHEMA: &str = "hoi4-m01-runtime-input-v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum M01UnitKind {
    Division,
    Fleet,
    Wing,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum M01VisibilityOrIntelState {
    Visible,
    Spotted,
    LastKnownStale,
    Hidden,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "province")]
pub enum M01AirBaseProvinceIfWing {
    NotWing,
    Province(u32),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct M01PositiveRuntimeRow {
    pub clock: u32,
    pub stable_unit_id: String,
    pub unit_kind: M01UnitKind,
    pub a7_selected_entity: String,
    pub a7_selected_model: String,
    pub a7_air_base_province_if_wing: M01AirBaseProvinceIfWing,
    pub a14_pov_tag: String,
    pub a14_visibility_or_intel_state: M01VisibilityOrIntelState,
    pub a14_spotted_decision: bool,
    pub render_included: bool,
    pub immutable_ib_hash64: String,
    pub world_matrix_le_hex_64_bytes: String,
    pub pre_color_draw_event: u64,
    pub shadow_draw_event: u64,
    pub post_color_draw_event: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct M01A14ControlState {
    pub pov_tag: String,
    pub visibility_or_intel_state: M01VisibilityOrIntelState,
    pub spotted_decision: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct M01NegativeRuntimeControl {
    pub same_stable_unit_id: String,
    pub changed_a14_pov_or_spotted_state: M01A14ControlState,
    pub render_included_false: bool,
    pub explicit_exclusion_reason: String,
    pub zero_draw_memberships_for_that_clock: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct M01RuntimeRowPair {
    pub positive: M01PositiveRuntimeRow,
    pub negative_control: M01NegativeRuntimeControl,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct M01RuntimeInputEnvelope {
    pub schema: String,
    pub rows: Vec<M01RuntimeRowPair>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct M01AdmittedRuntimeRows {
    pairs: Vec<M01RuntimeRowPair>,
}

impl M01AdmittedRuntimeRows {
    pub fn render_rows(&self) -> impl ExactSizeIterator<Item = &M01PositiveRuntimeRow> {
        self.pairs.iter().map(|pair| &pair.positive)
    }

    pub fn controls(&self) -> impl ExactSizeIterator<Item = &M01NegativeRuntimeControl> {
        self.pairs.iter().map(|pair| &pair.negative_control)
    }

    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum M01AdmissionError {
    MissingInput,
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
    StableUnitMismatch {
        row: usize,
    },
    UnchangedA14Control {
        row: usize,
    },
    DuplicateStableUnitAtClock {
        row: usize,
    },
    DuplicateGpuKeyAtClock {
        row: usize,
    },
    DuplicateDrawEventAtClock {
        field: &'static str,
        row: usize,
    },
    PassSequenceMismatch {
        clock: u32,
    },
}

impl fmt::Display for M01AdmissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingInput => write!(f, "M01 runtime input is absent"),
            Self::InvalidJson(error) => write!(f, "invalid M01 runtime JSON: {error}"),
            Self::UnsupportedSchema(schema) => {
                write!(f, "unsupported M01 runtime schema {schema:?}")
            }
            Self::EmptyRows => write!(f, "M01 runtime input has no positive/negative row pairs"),
            Self::EmptyField { field, row } => write!(f, "M01 row {row} has empty field {field}"),
            Self::InvalidField { field, row, reason } => {
                write!(f, "M01 row {row} has invalid field {field}: {reason}")
            }
            Self::StableUnitMismatch { row } => {
                write!(f, "M01 row {row} positive/control stable unit IDs differ")
            }
            Self::UnchangedA14Control { row } => {
                write!(
                    f,
                    "M01 row {row} negative control does not change A14 state"
                )
            }
            Self::DuplicateStableUnitAtClock { row } => {
                write!(
                    f,
                    "M01 row {row} duplicates a stable unit at the sampled clock"
                )
            }
            Self::DuplicateGpuKeyAtClock { row } => {
                write!(f, "M01 row {row} collides on the frame-local GPU key")
            }
            Self::DuplicateDrawEventAtClock { field, row } => {
                write!(f, "M01 row {row} duplicates {field} at the sampled clock")
            }
            Self::PassSequenceMismatch { clock } => {
                write!(f, "M01 pass sequences differ at clock {clock}")
            }
        }
    }
}

impl Error for M01AdmissionError {}

pub fn parse_and_admit_m01_runtime_rows(
    json: &str,
) -> Result<M01AdmittedRuntimeRows, M01AdmissionError> {
    let envelope: M01RuntimeInputEnvelope = serde_json::from_str(json)
        .map_err(|error| M01AdmissionError::InvalidJson(error.to_string()))?;
    if envelope.schema != M01_RUNTIME_INPUT_SCHEMA {
        return Err(M01AdmissionError::UnsupportedSchema(envelope.schema));
    }
    admit_m01_runtime_rows(Some(&envelope.rows))
}

pub fn admit_m01_runtime_rows(
    rows: Option<&[M01RuntimeRowPair]>,
) -> Result<M01AdmittedRuntimeRows, M01AdmissionError> {
    let rows = rows.ok_or(M01AdmissionError::MissingInput)?;
    if rows.is_empty() {
        return Err(M01AdmissionError::EmptyRows);
    }

    let mut stable_at_clock = BTreeSet::new();
    let mut gpu_key_at_clock = BTreeSet::new();
    let mut pre_events = BTreeSet::new();
    let mut shadow_events = BTreeSet::new();
    let mut post_events = BTreeSet::new();
    let mut pass_rows: BTreeMap<u32, Vec<&M01PositiveRuntimeRow>> = BTreeMap::new();

    for (row_index, pair) in rows.iter().enumerate() {
        validate_pair(pair, row_index)?;
        let row = &pair.positive;

        if !stable_at_clock.insert((row.clock, row.stable_unit_id.as_str())) {
            return Err(M01AdmissionError::DuplicateStableUnitAtClock { row: row_index });
        }
        if !gpu_key_at_clock.insert((
            row.clock,
            row.immutable_ib_hash64.as_str(),
            row.world_matrix_le_hex_64_bytes.as_str(),
        )) {
            return Err(M01AdmissionError::DuplicateGpuKeyAtClock { row: row_index });
        }
        for (field, event, set) in [
            (
                "pre_color_draw_event",
                row.pre_color_draw_event,
                &mut pre_events,
            ),
            (
                "shadow_draw_event",
                row.shadow_draw_event,
                &mut shadow_events,
            ),
            (
                "post_color_draw_event",
                row.post_color_draw_event,
                &mut post_events,
            ),
        ] {
            if !set.insert((row.clock, event)) {
                return Err(M01AdmissionError::DuplicateDrawEventAtClock {
                    field,
                    row: row_index,
                });
            }
        }
        pass_rows.entry(row.clock).or_default().push(row);
    }

    for (clock, rows_at_clock) in pass_rows {
        let mut pre = rows_at_clock.clone();
        let mut shadow = rows_at_clock.clone();
        let mut post = rows_at_clock;
        pre.sort_by_key(|row| row.pre_color_draw_event);
        shadow.sort_by_key(|row| row.shadow_draw_event);
        post.sort_by_key(|row| row.post_color_draw_event);
        let pre_ids: Vec<_> = pre.iter().map(|row| row.stable_unit_id.as_str()).collect();
        let shadow_ids: Vec<_> = shadow
            .iter()
            .map(|row| row.stable_unit_id.as_str())
            .collect();
        let post_ids: Vec<_> = post.iter().map(|row| row.stable_unit_id.as_str()).collect();
        if pre_ids != shadow_ids || pre_ids != post_ids {
            return Err(M01AdmissionError::PassSequenceMismatch { clock });
        }
    }

    Ok(M01AdmittedRuntimeRows {
        pairs: rows.to_vec(),
    })
}

fn validate_pair(pair: &M01RuntimeRowPair, row_index: usize) -> Result<(), M01AdmissionError> {
    let positive = &pair.positive;
    let negative = &pair.negative_control;

    for (field, value) in [
        ("stable_unit_id", positive.stable_unit_id.as_str()),
        ("a7_selected_entity", positive.a7_selected_entity.as_str()),
        ("a7_selected_model", positive.a7_selected_model.as_str()),
        ("a14_pov_tag", positive.a14_pov_tag.as_str()),
        ("same_stable_unit_id", negative.same_stable_unit_id.as_str()),
        (
            "changed_a14_pov_or_spotted_state.pov_tag",
            negative.changed_a14_pov_or_spotted_state.pov_tag.as_str(),
        ),
        (
            "explicit_exclusion_reason",
            negative.explicit_exclusion_reason.as_str(),
        ),
    ] {
        if value.trim().is_empty() {
            return Err(M01AdmissionError::EmptyField {
                field,
                row: row_index,
            });
        }
    }

    if positive.stable_unit_id != negative.same_stable_unit_id {
        return Err(M01AdmissionError::StableUnitMismatch { row: row_index });
    }
    if !positive.render_included {
        return Err(M01AdmissionError::InvalidField {
            field: "render_included",
            row: row_index,
            reason: "positive row must be included",
        });
    }
    if negative.render_included_false {
        return Err(M01AdmissionError::InvalidField {
            field: "render_included_false",
            row: row_index,
            reason: "negative control must contain literal false",
        });
    }
    if !negative.zero_draw_memberships_for_that_clock {
        return Err(M01AdmissionError::InvalidField {
            field: "zero_draw_memberships_for_that_clock",
            row: row_index,
            reason: "negative control must certify zero memberships",
        });
    }

    let control = &negative.changed_a14_pov_or_spotted_state;
    if control.pov_tag == positive.a14_pov_tag
        && control.visibility_or_intel_state == positive.a14_visibility_or_intel_state
        && control.spotted_decision == positive.a14_spotted_decision
    {
        return Err(M01AdmissionError::UnchangedA14Control { row: row_index });
    }

    match (positive.unit_kind, positive.a7_air_base_province_if_wing) {
        (M01UnitKind::Wing, M01AirBaseProvinceIfWing::Province(province)) if province != 0 => {}
        (M01UnitKind::Wing, _) => {
            return Err(M01AdmissionError::InvalidField {
                field: "a7_air_base_province_if_wing",
                row: row_index,
                reason: "wing rows require a nonzero province",
            })
        }
        (_, M01AirBaseProvinceIfWing::NotWing) => {}
        (_, M01AirBaseProvinceIfWing::Province(_)) => {
            return Err(M01AdmissionError::InvalidField {
                field: "a7_air_base_province_if_wing",
                row: row_index,
                reason: "non-wing rows require null",
            })
        }
    }

    if !is_canonical_hex(&positive.immutable_ib_hash64, 16) {
        return Err(M01AdmissionError::InvalidField {
            field: "immutable_ib_hash64",
            row: row_index,
            reason: "expected exactly 16 lowercase hexadecimal characters",
        });
    }
    if !is_canonical_hex(&positive.world_matrix_le_hex_64_bytes, 128) {
        return Err(M01AdmissionError::InvalidField {
            field: "world_matrix_le_hex_64_bytes",
            row: row_index,
            reason: "expected exactly 128 lowercase hexadecimal characters",
        });
    }
    let events = [
        positive.pre_color_draw_event,
        positive.shadow_draw_event,
        positive.post_color_draw_event,
    ];
    if events.contains(&0) {
        return Err(M01AdmissionError::InvalidField {
            field: "draw_event",
            row: row_index,
            reason: "draw event IDs must be nonzero",
        });
    }
    if events[0] == events[1] || events[0] == events[2] || events[1] == events[2] {
        return Err(M01AdmissionError::InvalidField {
            field: "draw_event",
            row: row_index,
            reason: "the three pass memberships must be distinct",
        });
    }
    Ok(())
}

fn is_canonical_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
