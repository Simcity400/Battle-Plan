
use crate::{Leaf, WorldFrame};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeSet;

/// D69 `tech_sharing_groups_live`, as `DOMAIN_PROBLEM_LAZY_CARRIER` spells it.
pub const DOMAIN_ID: i64 = 69;

pub const TECH_SHARING_BONUS_FIXED5_SCALE: i64 = 100_000;

#[derive(Clone, Debug, PartialEq)]
pub struct TechnologySharingRuntimeView {
    pub groups: Leaf<Vec<TechnologySharingRuntimeGroup>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TechnologySharingRuntimeGroup {
    pub runtime_ordinal: u32,
    pub definition_id: Leaf<String>,
    pub members: Vec<TechnologySharingRuntimeMember>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TechnologySharingRuntimeMember {
    pub member_tag_db_id: u32,
    /// WITHHELD when the frame's census says two ids reach this key: the
    /// member is identified by `member_tag_db_id`, and a name that points at
    /// neither country is not a smaller fact about the member -- it is a
    /// wrong one (Codex round 39).
    pub country_tag: Option<String>,
    pub dynamic_bonus_raw_fixed5: i64,
    pub dynamic_bonus: f64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TechnologySharingRuntimeError {
    pub reason: String,
}

impl std::fmt::Display for TechnologySharingRuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason)
    }
}

impl std::error::Error for TechnologySharingRuntimeError {}

#[derive(Deserialize)]
struct RuntimeRoot {
    count: u64,
    groups: Vec<RuntimeGroup>,
    dynamic_bonus_fixed5_scale: i64,
}

#[derive(Deserialize)]
struct RuntimeGroup {
    id_token: u32,
    runtime_ordinal: u32,
    definition_id: Value,
    member_tag_db_ids: Vec<u32>,
    members: Vec<String>,
    member_bonuses: Vec<RuntimeMember>,
}

#[derive(Deserialize)]
struct RuntimeMember {
    member_tag_db_id: u32,
    country_tag: String,
    dynamic_bonus_raw_fixed5: i64,
    dynamic_bonus: f64,
}

pub fn technology_sharing_runtime(
    frame: &WorldFrame,
) -> Result<TechnologySharingRuntimeView, TechnologySharingRuntimeError> {
    let Some(value) = frame.domain("tech_sharing_group") else {
        return Ok(TechnologySharingRuntimeView {
            groups: Leaf::Absent,
        });
    };
    // The carrier is read straight off the raw map, so a present value is not
    // by itself this hour's measurement. D69 has no unknown test anywhere in
    // the fold - `build_tech_sharing_groups_value` reads the persistent set
    // (world_fold.rs:7319-7323) and a hard-failure frame retains it
    // (world_fold.rs:14776) - so on a hour D69 declared a problem for, these
    // bytes are the PREVIOUS hour's. The frame says so; the raw map still
    // holds them for diagnostics.
    //
    // kills: dropping this check publishes a retained group set as
    // `Leaf::Measured` on a problem hour, which is the hour-misattribution
    // shape this project treats as unacceptable.
    if value.is_null() || frame.lazy_carrier_unavailable(DOMAIN_ID) {
        return Ok(TechnologySharingRuntimeView {
            groups: Leaf::Unobtained,
        });
    }
    parse_runtime_value(value, frame.contended_census())
        .map(|groups| TechnologySharingRuntimeView {
            groups: Leaf::Measured(groups),
        })
        .map_err(|reason| TechnologySharingRuntimeError { reason })
}

fn parse_runtime_value(
    value: &Value,
    contended_keys: &impl crate::world::ContendedKeys,
) -> Result<Vec<TechnologySharingRuntimeGroup>, String> {
    let root: RuntimeRoot = serde_json::from_value(value.clone())
        .map_err(|error| format!("D69 tech_sharing_group has invalid shape: {error}"))?;
    if root.dynamic_bonus_fixed5_scale != TECH_SHARING_BONUS_FIXED5_SCALE {
        return Err(format!(
            "D69 dynamic_bonus_fixed5_scale {} expected {}",
            root.dynamic_bonus_fixed5_scale, TECH_SHARING_BONUS_FIXED5_SCALE
        ));
    }
    if usize::try_from(root.count).ok() != Some(root.groups.len()) {
        return Err(format!(
            "D69 count {} disagrees with {} group rows",
            root.count,
            root.groups.len()
        ));
    }
    let mut ordinals = BTreeSet::new();
    let mut definition_ids = BTreeSet::new();
    root.groups
        .into_iter()
        .enumerate()
        .map(|(index, group)| {
            if group.id_token == 0
                || group.runtime_ordinal != group.id_token
                || !ordinals.insert(group.runtime_ordinal)
            {
                return Err(format!(
                    "D69 group {index} has zero, duplicate, or disagreeing runtime ordinal"
                ));
            }
            let definition_id = match group.definition_id {
                Value::String(value)
                    if !value.is_empty()
                        && value.len() <= 512
                        && value.as_bytes().iter().all(|byte| {
                            byte.is_ascii_alphanumeric()
                                || matches!(*byte, b'_' | b'-' | b'.' | b':' | b'/')
                        })
                        && definition_ids.insert(value.clone()) =>
                {
                    Leaf::Measured(value)
                }
                Value::Null => Leaf::Unobtained,
                _ => {
                    return Err(format!(
                        "D69 group {index} has missing, invalid, or duplicate definition_id"
                    ))
                }
            };
            let count = group.member_tag_db_ids.len();
            if group.members.len() != count || group.member_bonuses.len() != count {
                return Err(format!(
                    "D69 group {index} member id/tag/bonus cardinalities disagree"
                ));
            }
            let mut member_ids = BTreeSet::new();
            let mut members = Vec::with_capacity(count);
            for member_index in 0..count {
                let member = &group.member_bonuses[member_index];
                if member.member_tag_db_id != group.member_tag_db_ids[member_index]
                    || member.country_tag != group.members[member_index]
                    || member.country_tag.trim().is_empty()
                    || !member_ids.insert(member.member_tag_db_id)
                    || !member.dynamic_bonus.is_finite()
                {
                    return Err(format!(
                        "D69 group {index} member {member_index} has inconsistent identity/value"
                    ));
                }
                let expected =
                    member.dynamic_bonus_raw_fixed5 as f64 / TECH_SHARING_BONUS_FIXED5_SCALE as f64;
                if member.dynamic_bonus.to_bits() != expected.to_bits() {
                    return Err(format!(
                        "D69 group {index} member {member_index} dynamic bonus disagrees with fixed5 authority"
                    ));
                }
                members.push(TechnologySharingRuntimeMember {
                    member_tag_db_id: member.member_tag_db_id,
                    country_tag: Some(member.country_tag.clone())
                        .filter(|tag| !contended_keys.holds(tag.as_str())),
                    dynamic_bonus_raw_fixed5: member.dynamic_bonus_raw_fixed5,
                    dynamic_bonus: member.dynamic_bonus,
                });
            }
            Ok(TechnologySharingRuntimeGroup {
                runtime_ordinal: group.runtime_ordinal,
                definition_id,
                members,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn group(definition_id: Value) -> Value {
        json!({
            "count": 1,
            "dynamic_bonus_fixed5_scale": 100000,
            "groups": [{
                "id_token": 17,
                "runtime_ordinal": 17,
                "definition_id": definition_id,
                "member_tag_db_ids": [1],
                "members": ["GER"],
                "member_bonuses": [{
                    "member_tag_db_id": 1,
                    "country_tag": "GER",
                    "dynamic_bonus_raw_fixed5": 10000,
                    "dynamic_bonus": 0.1
                }]
            }]
        })
    }

    #[test]
    fn tsg3_definition_id_is_measured_without_reinterpreting_ordinal() {
        let rows = parse_runtime_value(&group(json!("commonwealth_research")), &BTreeSet::<String>::new())
            .unwrap();
        assert_eq!(rows[0].runtime_ordinal, 17);
        assert_eq!(
            rows[0].definition_id,
            Leaf::Measured("commonwealth_research".to_string())
        );
    }

    #[test]
    fn legacy_tsg2_definition_id_remains_unobtained() {
        let rows = parse_runtime_value(&group(Value::Null), &BTreeSet::<String>::new()).unwrap();
        assert_eq!(rows[0].definition_id, Leaf::Unobtained);
    }

    #[test]
    fn current_carrier_cannot_omit_definition_id() {
        let mut value = group(json!("commonwealth_research"));
        value["groups"][0]
            .as_object_mut()
            .unwrap()
            .remove("definition_id");
        assert!(parse_runtime_value(&value, &BTreeSet::<String>::new())
            .unwrap_err()
            .contains("invalid shape"));
    }

    #[test]
    fn dynamic_bonus_must_match_fixed5_authority() {
        let mut value = group(json!("commonwealth_research"));
        value["groups"][0]["member_bonuses"][0]["dynamic_bonus"] = json!(0.10001);
        assert!(parse_runtime_value(&value, &BTreeSet::<String>::new())
            .unwrap_err()
            .contains("fixed5 authority"));
    }
}
