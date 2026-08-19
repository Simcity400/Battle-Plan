
use crate::world::{
    AceRosterExactView, CharacterRosterExactView, CharacterTraitNodeState, D18CountryRosterView,
    ViewStatus,
};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const XUI17_G8_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const XUI17_G8_POST_HOUR_BOUNDARY_MANIFEST_SHA256: &str =
    "9959a34c326dddc65cd171010a70e5cff9f646295bd47b37416203cc9689156f";
pub const XUI17_G8_COMPLETE_MEMBERSHIP_MANIFEST_SHA256: &str =
    "46c5a9c7ad2d0e3e1b2d38bbb954a12302c773d91b95179aeb438f510fee9b79";
pub const XUI17_G11_ACE_SEMANTICS_MANIFEST_SHA256: &str =
    "63c52460b7142ac8f0346655931c0650c3abf2ebb484de7ddc3a55fd3a8796e2";
pub const XUI17_G8_SUPERSEDING_BOUNDARY_CLAIM_ARTIFACT_SHA256: &str =
    "77f02b5552b83323565d341df62a9156a5c73e107c463fa902d8a5b809afa625";
pub const XUI17_PRODUCER_CONTRACT_MANIFEST_SHA256: &str =
    "f907073429230425c6d71e1bde7c722d010d63e98cd4eb140eeb875ec5d10c0c";
pub const XUI17_EVIDENCE_MANIFEST_SHA256: &str =
    "fbc359fb46a2c7fe2b593ee0af410a034c1e30988d9c73b542df0ce74302d288";

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterG8TraitNodeObservation {
    pub source_ordinal: u32,
    pub trait_token: String,
    pub definition_kind: u32,
    pub state_raw: u8,
    pub state: CharacterTraitNodeState,
    pub progress_raw: Option<i64>,
    pub progress_scale: Option<i64>,
    pub blocked_reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterG8TraitTreeObservation {
    pub owner_tag_db_id: u32,
    pub unit_leader_id: u64,
    pub tree_root_offset: u16,
    pub expected_node_count: u32,
    pub nodes: Vec<CharacterG8TraitNodeObservation>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterG8TraitDefinitionObservation {
    pub source_ordinal: u32,
    pub trait_token: String,
    pub definition_kind: u32,
    pub tree_root_offset: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterG8TraitBatch {
    pub source_sequence: u64,
    pub observed_game_hour_raw: i32,
    pub catalog_epoch: u64,
    pub content_manifest_sha256: String,
    pub d47_identity_digest: String,
    pub ordered_digest: String,
    pub expected_leader_count: u32,
    pub expected_owner_count: u32,
    pub expected_definition_count: u32,
    pub definition_census: Vec<CharacterG8TraitDefinitionObservation>,
    pub trees: Vec<CharacterG8TraitTreeObservation>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterG11AceObservation {
    pub owner_tag_db_id: u32,
    pub ace_id: u64,
    pub country_ace_ordinal: u32,
    pub graphical_culture: String,
    pub gender_raw: u8,
    pub portrait_pool_index: u32,
    pub modifier_text: String,
    pub wing_display_name: Option<String>,
    pub wing_equipment_sprite: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterG11AceBatch {
    pub source_sequence: u64,
    pub observed_game_hour_raw: i32,
    pub catalog_epoch: u64,
    pub content_manifest_sha256: String,
    pub d46_identity_digest: String,
    pub ordered_digest: String,
    pub expected_ace_count: u32,
    pub rows: Vec<CharacterG11AceObservation>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterPresentationG8Exact {
    pub schema_version: u32,
    pub authority: String,
    pub executable_sha256: String,
    pub producer_contract_manifest_sha256: String,
    pub evidence_manifest_sha256: String,
    pub post_hour_boundary_manifest_sha256: String,
    pub complete_membership_manifest_sha256: String,
    pub ace_semantics_manifest_sha256: String,
    pub superseding_boundary_claim_artifact_sha256: String,
    pub elapsed_wall_ms: u64,
    pub frame_hour: u64,
    pub completeness_record_identity: String,
    pub trait_status_flags: u32,
    pub trait_failure_reason: String,
    pub traits: Option<CharacterG8TraitBatch>,
    pub ace_status_flags: u32,
    pub ace_failure_reason: String,
    pub aces: Option<CharacterG11AceBatch>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CharacterPresentationG8Envelope {
    schema_version: u32,
    authority: String,
    executable_sha256: String,
    producer_contract_manifest_sha256: String,
    evidence_manifest_sha256: String,
    post_hour_boundary_manifest_sha256: String,
    complete_membership_manifest_sha256: String,
    ace_semantics_manifest_sha256: String,
    superseding_boundary_claim_artifact_sha256: String,
    elapsed_wall_ms: u64,
    frame_hour: u64,
    completeness_record_identity: String,
    trait_status_flags: u32,
    trait_failure_reason: String,
    traits: Option<Value>,
    ace_status_flags: u32,
    ace_failure_reason: String,
    aces: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterPresentationG8ExactView {
    pub status: ViewStatus,
    pub presentation: Option<CharacterPresentationG8Exact>,
}

pub(crate) fn build_character_presentation_g8_exact(
    rest: &Map<String, Value>,
    expected_hour: i64,
    d18_country_roster: &D18CountryRosterView,
    rosters: &BTreeMap<String, CharacterRosterExactView>,
    roster_status: ViewStatus,
    ace_rosters: &BTreeMap<String, AceRosterExactView>,
    ace_roster_status: ViewStatus,
    warnings: &mut Vec<String>,
) -> CharacterPresentationG8ExactView {
    const KEY: &str = "_r2_character_presentation_live";
    let unavailable = || CharacterPresentationG8ExactView {
        status: ViewStatus::DomainUnavailable,
        presentation: None,
    };
    let Some(value) = rest.get(KEY) else {
        return CharacterPresentationG8ExactView {
            status: ViewStatus::DomainAbsent,
            presentation: None,
        };
    };
    if value.is_null() {
        return unavailable();
    }
    if value.get("schema_version").and_then(Value::as_u64) != Some(4) {
        return CharacterPresentationG8ExactView {
            status: ViewStatus::DomainAbsent,
            presentation: None,
        };
    }
    let envelope: CharacterPresentationG8Envelope = match serde::Deserialize::deserialize(value) {
        Ok(envelope) => envelope,
        Err(error) => {
            warnings.push(format!(
                "{KEY} v4 malformed: {error}; treated as unavailable"
            ));
            return unavailable();
        }
    };
    let traits = decode_component::<CharacterG8TraitBatch>(KEY, "trait", envelope.traits, warnings);
    let aces = decode_component::<CharacterG11AceBatch>(KEY, "ace", envelope.aces, warnings);
    let mut presentation = CharacterPresentationG8Exact {
        schema_version: envelope.schema_version,
        authority: envelope.authority,
        executable_sha256: envelope.executable_sha256,
        producer_contract_manifest_sha256: envelope.producer_contract_manifest_sha256,
        evidence_manifest_sha256: envelope.evidence_manifest_sha256,
        post_hour_boundary_manifest_sha256: envelope.post_hour_boundary_manifest_sha256,
        complete_membership_manifest_sha256: envelope.complete_membership_manifest_sha256,
        ace_semantics_manifest_sha256: envelope.ace_semantics_manifest_sha256,
        superseding_boundary_claim_artifact_sha256: envelope
            .superseding_boundary_claim_artifact_sha256,
        elapsed_wall_ms: envelope.elapsed_wall_ms,
        frame_hour: envelope.frame_hour,
        completeness_record_identity: envelope.completeness_record_identity,
        trait_status_flags: envelope.trait_status_flags,
        trait_failure_reason: envelope.trait_failure_reason,
        traits,
        ace_status_flags: envelope.ace_status_flags,
        ace_failure_reason: envelope.ace_failure_reason,
        aces,
    };
    let Some(expected_hour) = u64::try_from(expected_hour).ok() else {
        warnings.push(format!("{KEY} v4 cannot bind to a negative replay hour"));
        return unavailable();
    };
    if presentation.schema_version != 4
        || presentation.authority != "exact_a729_g8_g11_post_hour"
        || presentation.executable_sha256 != XUI17_G8_EXECUTABLE_SHA256
        || presentation.producer_contract_manifest_sha256 != XUI17_PRODUCER_CONTRACT_MANIFEST_SHA256
        || presentation.evidence_manifest_sha256 != XUI17_EVIDENCE_MANIFEST_SHA256
        || presentation.post_hour_boundary_manifest_sha256
            != XUI17_G8_POST_HOUR_BOUNDARY_MANIFEST_SHA256
        || presentation.complete_membership_manifest_sha256
            != XUI17_G8_COMPLETE_MEMBERSHIP_MANIFEST_SHA256
        || presentation.ace_semantics_manifest_sha256 != XUI17_G11_ACE_SEMANTICS_MANIFEST_SHA256
        || presentation.superseding_boundary_claim_artifact_sha256
            != XUI17_G8_SUPERSEDING_BOUNDARY_CLAIM_ARTIFACT_SHA256
        || presentation.frame_hour != expected_hour
        || presentation.completeness_record_identity.trim().is_empty()
        || presentation.completeness_record_identity.len() > 1_024
    {
        warnings.push(format!("{KEY} v4 authority/frame header rejected"));
        return unavailable();
    }

    if !component_shape(
        presentation.trait_status_flags,
        &presentation.trait_failure_reason,
        presentation.traits.is_some(),
    ) {
        reject_traits(&mut presentation, "component header", warnings);
    }
    if !component_shape(
        presentation.ace_status_flags,
        &presentation.ace_failure_reason,
        presentation.aces.is_some(),
    ) {
        reject_aces(&mut presentation, "component header", warnings);
    }
    if presentation.traits.as_ref().is_some_and(|batch| {
        !traits_valid(
            expected_hour,
            batch,
            d18_country_roster,
            rosters,
            roster_status,
        )
    }) {
        reject_traits(&mut presentation, "identity/order/content join", warnings);
    }
    if presentation
        .aces
        .as_ref()
        .is_some_and(|batch| !aces_valid(expected_hour, batch, ace_rosters, ace_roster_status))
    {
        reject_aces(&mut presentation, "identity/order join", warnings);
    }
    CharacterPresentationG8ExactView {
        status: ViewStatus::Present,
        presentation: Some(presentation),
    }
}

fn decode_component<T: serde::de::DeserializeOwned>(
    key: &str,
    name: &str,
    value: Option<Value>,
    warnings: &mut Vec<String>,
) -> Option<T> {
    value.and_then(|value| match serde_json::from_value(value) {
        Ok(component) => Some(component),
        Err(error) => {
            warnings.push(format!(
                "{key} v4 {name} component malformed: {error}; component treated as unavailable"
            ));
            None
        }
    })
}

fn component_shape(status_flags: u32, failure_reason: &str, present: bool) -> bool {
    (present && status_flags == 0 && failure_reason.is_empty())
        || (!present
            && status_flags != 0
            && !failure_reason.trim().is_empty()
            && failure_reason.len() <= 4_096)
}

fn reject_traits(
    presentation: &mut CharacterPresentationG8Exact,
    detail: &str,
    warnings: &mut Vec<String>,
) {
    warnings.push(format!(
        "_r2_character_presentation_live v4 trait {detail} rejected"
    ));
    presentation.traits = None;
    presentation.trait_status_flags = 1 << 1;
    presentation.trait_failure_reason = format!("replay admission rejected the trait {detail}");
}

fn reject_aces(
    presentation: &mut CharacterPresentationG8Exact,
    detail: &str,
    warnings: &mut Vec<String>,
) {
    warnings.push(format!(
        "_r2_character_presentation_live v4 ace {detail} rejected"
    ));
    presentation.aces = None;
    presentation.ace_status_flags = 1 << 1;
    presentation.ace_failure_reason = format!("replay admission rejected the ace {detail}");
}

fn traits_valid(
    expected_hour: u64,
    batch: &CharacterG8TraitBatch,
    d18_country_roster: &D18CountryRosterView,
    rosters: &BTreeMap<String, CharacterRosterExactView>,
    roster_status: ViewStatus,
) -> bool {
    let d18_country_ids = d18_country_roster
        .entries
        .iter()
        .map(|entry| (entry.tag.as_str(), entry.tag_db_id))
        .collect::<BTreeMap<_, _>>();
    let mut current_leaders = BTreeSet::new();
    let mut identity_complete = d18_country_roster.status == ViewStatus::Present
        && !d18_country_ids.is_empty()
        && roster_status == ViewStatus::Present
        && rosters.len() == d18_country_ids.len()
        && d18_country_ids.keys().all(|tag| rosters.contains_key(*tag));
    for (tag, view) in rosters {
        identity_complete &=
            view.status == ViewStatus::Present && view.family_observed("unit_leaders");
        let Some(owner) = d18_country_ids.get(tag.as_str()).copied() else {
            identity_complete = false;
            continue;
        };
        for leader in &view.unit_leaders {
            identity_complete &= leader.unit_leader_id != 0
                && current_leaders.insert((owner, leader.unit_leader_id));
        }
    }
    let observed_leaders = batch
        .trees
        .iter()
        .map(|tree| (tree.owner_tag_db_id, tree.unit_leader_id))
        .collect::<BTreeSet<_>>();
    let mut tree_keys = BTreeSet::new();
    let mut definition_tokens = BTreeSet::new();
    let mut definitions = BTreeSet::new();
    let definition_census_valid =
        batch
            .definition_census
            .iter()
            .enumerate()
            .all(|(ordinal, definition)| {
                definition.source_ordinal as usize == ordinal
                    && !definition.trait_token.trim().is_empty()
                    && definition.trait_token.len() <= 1_024
                    && definition_tokens.insert(definition.trait_token.as_str())
                    && definitions.insert((
                        definition.trait_token.as_str(),
                        definition.definition_kind,
                        definition.tree_root_offset,
                    ))
                    && matches!(
                        (definition.definition_kind, definition.tree_root_offset),
                        (2 | 4, 0x120) | (3 | 5, 0x130)
                    )
            });
    let mut total_nodes = 0usize;
    let trees_valid = batch.trees.iter().all(|tree| {
        total_nodes = total_nodes.saturating_add(tree.nodes.len());
        let mut tokens = BTreeSet::new();
        tree.owner_tag_db_id != 0
            && tree.expected_node_count as usize == tree.nodes.len()
            && current_leaders.contains(&(tree.owner_tag_db_id, tree.unit_leader_id))
            && matches!(tree.tree_root_offset, 0x120 | 0x130)
            && tree_keys.insert((
                tree.owner_tag_db_id,
                tree.unit_leader_id,
                tree.tree_root_offset,
            ))
            && total_nodes <= 131_072
            && tree.nodes.iter().enumerate().all(|(ordinal, node)| {
                node.source_ordinal as usize == ordinal
                    && !node.trait_token.trim().is_empty()
                    && node.trait_token.len() <= 1_024
                    && tokens.insert(node.trait_token.as_str())
                    && definitions.contains(&(
                        node.trait_token.as_str(),
                        node.definition_kind,
                        tree.tree_root_offset,
                    ))
                    && (matches!(node.definition_kind, 2 | 3) == node.progress_raw.is_some())
                    && (node.progress_raw.is_some() == node.progress_scale.is_some())
                    && node.progress_scale.is_none_or(|scale| scale == 100_000)
                    && matches!(
                        (node.state_raw, node.state),
                        (0, CharacterTraitNodeState::Blocked)
                            | (1, CharacterTraitNodeState::Pickable)
                            | (2, CharacterTraitNodeState::Picked)
                    )
                    && (node.state == CharacterTraitNodeState::Blocked
                        || node.blocked_reason.is_empty())
                    && node.blocked_reason.len() <= 4_096
            })
    });
    let roots_complete = current_leaders.iter().all(|(owner, leader)| {
        batch
            .trees
            .iter()
            .filter(|tree| tree.owner_tag_db_id == *owner && tree.unit_leader_id == *leader)
            .map(|tree| tree.tree_root_offset)
            .collect::<BTreeSet<_>>()
            == BTreeSet::from([0x120, 0x130])
    });
    identity_complete
        && batch.source_sequence != 0
        && batch.catalog_epoch != 0
        && valid_hash(&batch.content_manifest_sha256)
        && batch.expected_owner_count as usize == d18_country_ids.len()
        && batch.expected_leader_count as usize == current_leaders.len()
        && observed_leaders == current_leaders
        && batch.trees.len() == current_leaders.len().saturating_mul(2)
        && valid_hash(&batch.d47_identity_digest)
        && valid_hash(&batch.ordered_digest)
        && batch.expected_definition_count != 0
        && batch.expected_definition_count as usize == batch.definition_census.len()
        && definition_census_valid
        && d47_census_digest(batch) == batch.d47_identity_digest
        && trees_valid
        && roots_complete
        && observed_hour_matches(batch.observed_game_hour_raw, expected_hour)
        && trait_ordered_digest(&batch.trees) == batch.ordered_digest
}

fn aces_valid(
    expected_hour: u64,
    batch: &CharacterG11AceBatch,
    rosters: &BTreeMap<String, AceRosterExactView>,
    roster_status: ViewStatus,
) -> bool {
    let current = rosters
        .values()
        .flat_map(|view| {
            view.aces.iter().map(|row| {
                (
                    u32::try_from(row.owner_tag_db_id).unwrap_or(0),
                    row.ace_id,
                    u32::try_from(row.country_ace_index).unwrap_or(u32::MAX),
                    row.wing_id,
                )
            })
        })
        .collect::<BTreeSet<_>>();
    let observed = batch
        .rows
        .iter()
        .map(|row| (row.owner_tag_db_id, row.ace_id, row.country_ace_ordinal))
        .collect::<BTreeSet<_>>();
    let current_identity = current
        .iter()
        .map(|(owner, ace, ordinal, _)| (*owner, *ace, *ordinal))
        .collect::<BTreeSet<_>>();
    let mut total_text = 0usize;
    roster_status == ViewStatus::Present
        && !rosters.is_empty()
        && rosters
            .values()
            .all(|view| view.status == ViewStatus::Present)
        && batch.source_sequence != 0
        && batch.catalog_epoch != 0
        && valid_hash(&batch.content_manifest_sha256)
        && observed_hour_matches(batch.observed_game_hour_raw, expected_hour)
        && valid_hash(&batch.d46_identity_digest)
        && valid_hash(&batch.ordered_digest)
        && ace_identity_digest(&batch.rows) == batch.d46_identity_digest
        && ace_ordered_digest(&batch.rows) == batch.ordered_digest
        && batch.expected_ace_count as usize == batch.rows.len()
        && observed.len() == batch.rows.len()
        && observed == current_identity
        && batch.rows.iter().all(|row| {
            total_text = total_text.saturating_add(row.modifier_text.len());
            let live_wing_id = current
                .iter()
                .find(|(owner, ace, ordinal, _)| {
                    *owner == row.owner_tag_db_id
                        && *ace == row.ace_id
                        && *ordinal == row.country_ace_ordinal
                })
                .map(|(_, _, _, wing)| *wing)
                .unwrap_or(u64::MAX);
            let wing_pair = match live_wing_id {
                0 => row.wing_display_name.is_none() && row.wing_equipment_sprite.is_none(),
                u64::MAX => false,
                _ => row.wing_display_name.is_some() && row.wing_equipment_sprite.is_some(),
            };
            row.owner_tag_db_id != 0
                && row.ace_id != 0
                && matches!(row.gender_raw, 0 | 1)
                && !row.graphical_culture.trim().is_empty()
                && row.graphical_culture.len() <= 1_024
                && row.modifier_text.len() <= (1 << 20)
                && total_text <= (64 << 20)
                && wing_pair
                && row
                    .wing_display_name
                    .as_deref()
                    .is_none_or(|value| !value.trim().is_empty() && value.len() <= 65_536)
                && row
                    .wing_equipment_sprite
                    .as_deref()
                    .is_none_or(|value| !value.trim().is_empty() && value.len() <= 1_024)
        })
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && value.bytes().any(|byte| byte != b'0')
}

fn observed_hour_matches(observed_game_hour_raw: i32, expected_hour: u64) -> bool {
    const DATE_EPOCH_CONST: i32 = 43_799_999;
    let min = (1850 * 365 * 24) + DATE_EPOCH_CONST;
    let max = (2200 * 365 * 24) + DATE_EPOCH_CONST;
    observed_game_hour_raw >= min
        && observed_game_hour_raw <= max
        && u64::try_from(observed_game_hour_raw - DATE_EPOCH_CONST).ok() == Some(expected_hour)
}

fn digest_hex(write: impl FnOnce(&mut Sha256)) -> String {
    let mut hasher = Sha256::new();
    write(&mut hasher);
    format!("{:x}", hasher.finalize())
}

fn digest_string(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value.as_bytes());
}

pub fn trait_ordered_digest(trees: &[CharacterG8TraitTreeObservation]) -> String {
    digest_hex(|hasher| {
        hasher.update(b"xui17:g8:traits:v2");
        for tree in trees {
            hasher.update(tree.owner_tag_db_id.to_le_bytes());
            hasher.update(tree.unit_leader_id.to_le_bytes());
            hasher.update(tree.tree_root_offset.to_le_bytes());
            hasher.update(tree.expected_node_count.to_le_bytes());
            for node in &tree.nodes {
                digest_string(hasher, &node.trait_token);
                hasher.update(node.definition_kind.to_le_bytes());
                hasher.update([node.state_raw]);
                hasher.update(node.progress_raw.unwrap_or_default().to_le_bytes());
                hasher.update(node.progress_scale.unwrap_or_default().to_le_bytes());
                digest_string(hasher, &node.blocked_reason);
            }
        }
    })
}

pub fn d47_census_digest(batch: &CharacterG8TraitBatch) -> String {
    digest_hex(|hasher| {
        hasher.update(b"xui17:g8:d47:census:v2");
        hasher.update(batch.expected_owner_count.to_le_bytes());
        hasher.update(batch.catalog_epoch.to_le_bytes());
        let leaders = batch
            .trees
            .iter()
            .map(|tree| (tree.owner_tag_db_id, tree.unit_leader_id))
            .collect::<BTreeSet<_>>();
        hasher.update((leaders.len() as u64).to_le_bytes());
        for (owner, leader) in leaders {
            hasher.update(owner.to_le_bytes());
            hasher.update(leader.to_le_bytes());
        }
        hasher.update((batch.definition_census.len() as u64).to_le_bytes());
        for definition in &batch.definition_census {
            digest_string(hasher, &definition.trait_token);
            hasher.update(definition.definition_kind.to_le_bytes());
            hasher.update(definition.tree_root_offset.to_le_bytes());
        }
    })
}

pub fn ace_identity_digest(rows: &[CharacterG11AceObservation]) -> String {
    digest_hex(|hasher| {
        hasher.update(b"xui17:g8:d46:v1");
        for row in rows {
            hasher.update(row.owner_tag_db_id.to_le_bytes());
            hasher.update(row.ace_id.to_le_bytes());
            hasher.update(row.country_ace_ordinal.to_le_bytes());
        }
    })
}

pub fn ace_ordered_digest(rows: &[CharacterG11AceObservation]) -> String {
    digest_hex(|hasher| {
        hasher.update(b"xui17:g11:aces:v1");
        for row in rows {
            hasher.update(row.owner_tag_db_id.to_le_bytes());
            hasher.update(row.ace_id.to_le_bytes());
            hasher.update(row.country_ace_ordinal.to_le_bytes());
            digest_string(hasher, &row.graphical_culture);
            hasher.update([row.gender_raw]);
            hasher.update(row.portrait_pool_index.to_le_bytes());
            digest_string(hasher, &row.modifier_text);
            digest_string(hasher, row.wing_display_name.as_deref().unwrap_or(""));
            digest_string(hasher, row.wing_equipment_sprite.as_deref().unwrap_or(""));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{
        AceExactRow, AceLifecycleState, CharacterRosterExactView, D18CountryRosterEntry,
        UnitLeaderExactRow, UnitLeaderSkillExact,
    };

    const FRAME_HOUR: u64 = 16_960_000;
    const OBSERVED_GAME_HOUR_RAW: i32 = 43_799_999 + FRAME_HOUR as i32;

    fn canonical_lf_sha256(text: &str) -> String {
        format!(
            "{:x}",
            Sha256::digest(text.replace("\r\n", "\n").as_bytes())
        )
    }

    fn leader_roster() -> CharacterRosterExactView {
        CharacterRosterExactView {
            status: ViewStatus::Present,
            command_assignments_complete: true,
            fleet_ui_complete: true,
            fleet_ui: Vec::new(),
            characters: Vec::new(),
            unit_leaders: vec![UnitLeaderExactRow {
                producer_ordinal: 0,
                unit_leader_id: 7,
                char_id: 70,
                leader_kind: "army".into(),
                leader_kind_raw: 1,
                leader_skill: UnitLeaderSkillExact::Measured {
                    leader_skill_type_raw: 1,
                    experience_raw: 0,
                    skill_effect_text: String::new(),
                    skill_effect_text_language: "l_english".into(),
                },
                skill: 4,
                attack: 4,
                defense: 4,
                planning: 4,
                logistics: 4,
                maneuvering: 0,
                coordination: 0,
                traits: Vec::new(),
                assignment: None,
                token: "GER_test".into(),
                name: "Test Leader".into(),
            }],
            country_leader_roles: Vec::new(),
            active_rulers: Vec::new(),
            advisor_roster_complete: true,
            advisor_roster: Vec::new(),
            appointed_advisors_complete: true,
            appointed_advisors: Vec::new(),
            character_portraits: Vec::new(),
            family_gaps: BTreeSet::new(),
        }
    }

    fn ace_roster() -> AceRosterExactView {
        AceRosterExactView {
            status: ViewStatus::Present,
            owner_tag_db_id: 47,
            carrier_version: 2,
            observed_country_rows: 1,
            observed_ace_rows: 1,
            wing_assignment_complete: true,
            aces: vec![AceExactRow {
                owner_tag_db_id: 47,
                ace_id: 9,
                country_ace_index: 0,
                source_mask: 0x73,
                name: "A".into(),
                surname: "B".into(),
                callsign: String::new(),
                alive: true,
                handled: Some(false),
                kill_type_raw: Some(0),
                lifecycle: Some(AceLifecycleState::Alive),
                wing_id: 88,
                killer_ace_id: 0,
                killer_country_tag_db_id: 0,
            }],
        }
    }

    fn d18_roster(entries: &[(&str, u32)]) -> D18CountryRosterView {
        D18CountryRosterView {
            status: ViewStatus::Present,
            entries: entries
                .iter()
                .map(|(tag, tag_db_id)| D18CountryRosterEntry {
                    tag: (*tag).into(),
                    tag_db_id: *tag_db_id,
                })
                .collect(),
            diagnostic: None,
        }
    }

    fn components() -> (CharacterG8TraitBatch, CharacterG11AceBatch) {
        let trees = vec![
            CharacterG8TraitTreeObservation {
                owner_tag_db_id: 47,
                unit_leader_id: 7,
                tree_root_offset: 0x120,
                expected_node_count: 1,
                nodes: vec![CharacterG8TraitNodeObservation {
                    source_ordinal: 0,
                    trait_token: "panzer_leader".into(),
                    definition_kind: 2,
                    state_raw: 0,
                    state: CharacterTraitNodeState::Blocked,
                    progress_raw: Some(0),
                    progress_scale: Some(100_000),
                    blocked_reason: "Requires armor experience".into(),
                }],
            },
            CharacterG8TraitTreeObservation {
                owner_tag_db_id: 47,
                unit_leader_id: 7,
                tree_root_offset: 0x130,
                expected_node_count: 0,
                nodes: Vec::new(),
            },
        ];
        let mut traits = CharacterG8TraitBatch {
            source_sequence: 1,
            observed_game_hour_raw: OBSERVED_GAME_HOUR_RAW,
            catalog_epoch: 2,
            content_manifest_sha256: "a".repeat(64),
            d47_identity_digest: String::new(),
            ordered_digest: trait_ordered_digest(&trees),
            expected_leader_count: 1,
            expected_owner_count: 1,
            expected_definition_count: 1,
            definition_census: vec![CharacterG8TraitDefinitionObservation {
                source_ordinal: 0,
                trait_token: "panzer_leader".into(),
                definition_kind: 2,
                tree_root_offset: 0x120,
            }],
            trees,
        };
        traits.d47_identity_digest = d47_census_digest(&traits);
        let rows = vec![CharacterG11AceObservation {
            owner_tag_db_id: 47,
            ace_id: 9,
            country_ace_ordinal: 0,
            graphical_culture: "western_european_gfx".into(),
            gender_raw: 0,
            portrait_pool_index: 3,
            modifier_text: "+10% agility".into(),
            wing_display_name: Some("1. Jagdgeschwader".into()),
            wing_equipment_sprite: Some("GFX_plane_fighter_1_medium".into()),
        }];
        let aces = CharacterG11AceBatch {
            source_sequence: 3,
            observed_game_hour_raw: OBSERVED_GAME_HOUR_RAW,
            catalog_epoch: 4,
            content_manifest_sha256: "a".repeat(64),
            d46_identity_digest: ace_identity_digest(&rows),
            ordered_digest: ace_ordered_digest(&rows),
            expected_ace_count: 1,
            rows,
        };
        (traits, aces)
    }

    fn frame_value(traits: &CharacterG8TraitBatch, aces: &CharacterG11AceBatch) -> Value {
        serde_json::json!({
            "countries": {"GER": {"_r2_tag_db_id": 47}},
            "_r2_character_presentation_live": {
                "schema_version": 4,
                "authority": "exact_a729_g8_g11_post_hour",
                "executable_sha256": XUI17_G8_EXECUTABLE_SHA256,
                "producer_contract_manifest_sha256": XUI17_PRODUCER_CONTRACT_MANIFEST_SHA256,
                "evidence_manifest_sha256": XUI17_EVIDENCE_MANIFEST_SHA256,
                "post_hour_boundary_manifest_sha256": XUI17_G8_POST_HOUR_BOUNDARY_MANIFEST_SHA256,
                "complete_membership_manifest_sha256": XUI17_G8_COMPLETE_MEMBERSHIP_MANIFEST_SHA256,
                "ace_semantics_manifest_sha256": XUI17_G11_ACE_SEMANTICS_MANIFEST_SHA256,
                "superseding_boundary_claim_artifact_sha256": XUI17_G8_SUPERSEDING_BOUNDARY_CLAIM_ARTIFACT_SHA256,
                "elapsed_wall_ms": 1,
                "frame_hour": FRAME_HOUR,
                "completeness_record_identity": "xui17-test-receipt",
                "trait_status_flags": 0,
                "trait_failure_reason": "",
                "traits": traits,
                "ace_status_flags": 0,
                "ace_failure_reason": "",
                "aces": aces
            }
        })
    }

    fn admit(value: Value) -> CharacterPresentationG8ExactView {
        let rest = value.as_object().unwrap();
        build_character_presentation_g8_exact(
            rest,
            FRAME_HOUR as i64,
            &d18_roster(&[("GER", 47)]),
            &BTreeMap::from([("GER".into(), leader_roster())]),
            ViewStatus::Present,
            &BTreeMap::from([("GER".into(), ace_roster())]),
            ViewStatus::Present,
            &mut Vec::new(),
        )
    }

    #[test]
    fn packaged_contract_and_evidence_hashes_are_exact() {
        assert_eq!(
            canonical_lf_sha256(include_str!(
                "../../../re_evidence/rendering_laws/XUI17/producer_contract_manifest.json"
            )),
            XUI17_PRODUCER_CONTRACT_MANIFEST_SHA256
        );
        assert_eq!(
            canonical_lf_sha256(include_str!(
                "../../../re_evidence/rendering_laws/XUI17/evidence_manifest.json"
            )),
            XUI17_EVIDENCE_MANIFEST_SHA256
        );
    }

    #[test]
    fn complete_all_country_g8_and_g11_components_admit() {
        let (traits, aces) = components();
        let view = admit(frame_value(&traits, &aces));
        assert_eq!(view.status, ViewStatus::Present);
        let presentation = view.presentation.unwrap();
        assert!(presentation.traits.is_some());
        assert!(presentation.aces.is_some());
    }

    #[test]
    fn g8_owner_set_must_exactly_equal_the_canonical_d18_roster() {
        let (traits, aces) = components();
        let value = frame_value(&traits, &aces);
        let rest = value.as_object().unwrap();
        let leaders = BTreeMap::from([("GER".into(), leader_roster())]);
        let ace_rosters = BTreeMap::from([("GER".into(), ace_roster())]);

        for d18 in [
            d18_roster(&[("GER", 47), ("ENG", 48)]),
            d18_roster(&[("ENG", 48)]),
            d18_roster(&[("GER", 48)]),
        ] {
            let view = build_character_presentation_g8_exact(
                rest,
                FRAME_HOUR as i64,
                &d18,
                &leaders,
                ViewStatus::Present,
                &ace_rosters,
                ViewStatus::Present,
                &mut Vec::new(),
            );
            let presentation = view.presentation.expect("valid outer envelope");
            assert!(presentation.traits.is_none());
            assert!(presentation.aces.is_some());
            assert!(presentation
                .trait_failure_reason
                .contains("identity/order/content join"));
        }
    }

    #[test]
    fn complete_two_country_d18_and_d47_owner_sets_admit_g8() {
        let (mut traits, aces) = components();
        let mut eng = leader_roster();
        eng.unit_leaders[0].unit_leader_id = 8;
        eng.unit_leaders[0].char_id = 80;
        eng.unit_leaders[0].token = "ENG_test".into();
        for root in [0x120, 0x130] {
            traits.trees.push(CharacterG8TraitTreeObservation {
                owner_tag_db_id: 48,
                unit_leader_id: 8,
                tree_root_offset: root,
                expected_node_count: 0,
                nodes: Vec::new(),
            });
        }
        traits.expected_owner_count = 2;
        traits.expected_leader_count = 2;
        traits.d47_identity_digest = d47_census_digest(&traits);
        traits.ordered_digest = trait_ordered_digest(&traits.trees);
        let mut value = frame_value(&traits, &aces);
        value["countries"]["ENG"] = serde_json::json!({"_r2_tag_db_id": 48});
        let view = build_character_presentation_g8_exact(
            value.as_object().unwrap(),
            FRAME_HOUR as i64,
            &d18_roster(&[("ENG", 48), ("GER", 47)]),
            &BTreeMap::from([("ENG".into(), eng), ("GER".into(), leader_roster())]),
            ViewStatus::Present,
            &BTreeMap::from([("GER".into(), ace_roster())]),
            ViewStatus::Present,
            &mut Vec::new(),
        );
        assert!(view.presentation.unwrap().traits.is_some());
    }

    #[test]
    fn unavailable_d18_and_cross_hour_g8_reject_only_the_trait_component() {
        let (traits, aces) = components();
        let value = frame_value(&traits, &aces);
        let rest = value.as_object().unwrap();
        let unavailable_d18 = D18CountryRosterView {
            status: ViewStatus::DomainUnavailable,
            entries: Vec::new(),
            diagnostic: Some("test D18 gap".into()),
        };
        let leaders = BTreeMap::from([("GER".into(), leader_roster())]);
        let ace_rosters = BTreeMap::from([("GER".into(), ace_roster())]);
        let view = build_character_presentation_g8_exact(
            rest,
            FRAME_HOUR as i64,
            &unavailable_d18,
            &leaders,
            ViewStatus::Present,
            &ace_rosters,
            ViewStatus::Present,
            &mut Vec::new(),
        );
        let presentation = view.presentation.unwrap();
        assert!(presentation.traits.is_none());
        assert!(presentation.aces.is_some());

        let mut cross_hour_traits = traits;
        cross_hour_traits.observed_game_hour_raw += 1;
        cross_hour_traits.d47_identity_digest = d47_census_digest(&cross_hour_traits);
        cross_hour_traits.ordered_digest = trait_ordered_digest(&cross_hour_traits.trees);
        let presentation = admit(frame_value(&cross_hour_traits, &aces))
            .presentation
            .unwrap();
        assert!(presentation.traits.is_none());
        assert!(presentation.aces.is_some());

        let mut stale_envelope = frame_value(&components().0, &aces);
        stale_envelope["_r2_character_presentation_live"]["frame_hour"] =
            serde_json::json!(FRAME_HOUR - 1);
        assert_eq!(admit(stale_envelope).status, ViewStatus::DomainUnavailable);
    }

    #[test]
    fn invalid_assigned_wing_rejects_only_g11_component() {
        let (traits, mut aces) = components();
        aces.rows[0].wing_display_name = None;
        aces.ordered_digest = ace_ordered_digest(&aces.rows);
        let presentation = admit(frame_value(&traits, &aces)).presentation.unwrap();
        assert!(presentation.traits.is_some());
        assert!(presentation.aces.is_none());
        assert!(!presentation.ace_failure_reason.is_empty());
    }

    #[test]
    fn out_of_codomain_gender_rejects_only_g11_component() {
        let (traits, mut aces) = components();
        aces.rows[0].gender_raw = 2;
        aces.ordered_digest = ace_ordered_digest(&aces.rows);
        let presentation = admit(frame_value(&traits, &aces)).presentation.unwrap();
        assert!(presentation.traits.is_some());
        assert!(presentation.aces.is_none());
        assert!(!presentation.ace_failure_reason.is_empty());
    }

    #[test]
    fn content_manifest_tamper_rejects_only_affected_component() {
        let (mut traits, aces) = components();
        traits.content_manifest_sha256 = "not-a-hash".into();
        let presentation = admit(frame_value(&traits, &aces)).presentation.unwrap();
        assert!(presentation.traits.is_none());
        assert!(presentation.aces.is_some());
    }
}
