//! Typed Replay-owned carriers for XUI26 frontend/meta presentation.
//!
//! These values are final producer observations, not instructions to infer UI
//! state from authored names.  The consumer additionally binds every semantic
//! key to the selected effective-VFS role index before a panel can compose.

use crate::world::ViewStatus;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};

pub const XUI26_FRONTEND_CARRIER_SCHEMA: &str = "replay.xui26-frontend.v1";
pub const XUI26_FRONTEND_CARRIER_KEY: &str = "_r2_xui26_frontend_v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Xui26ReplayPayload {
    ScenarioSetup {
        scenario_id: String,
        bookmark_id: String,
        country_tag: String,
        game_rules_sha256: String,
        bookmark_ids_in_order: Vec<String>,
        country_tags_in_order: Vec<String>,
        game_rule_ids_in_order: Vec<String>,
    },
    TutorialState {
        tutorial_id: String,
        step_id: String,
        completed_step_ids: Vec<String>,
    },
    SettingsSnapshot {
        settings_file_sha256: String,
        assignments: BTreeMap<String, String>,
    },
    MessageSettingsSnapshot {
        settings_file_sha256: String,
        assignments: BTreeMap<String, String>,
    },
    MessageFilterSnapshot {
        settings_file_sha256: String,
        assignments: BTreeMap<String, String>,
    },
    SavedGameRules {
        rules_sha256: String,
        assignments: BTreeMap<String, String>,
    },
    PlayerAiPreferences {
        assignments: BTreeMap<String, String>,
    },
    LiveLobby {
        player_keys_in_slot_order: Vec<String>,
        country_tags_in_slot_order: Vec<String>,
    },
    HotjoinState {
        event_record_ids: Vec<String>,
    },
    MatchmakingChat {
        event_record_ids: Vec<String>,
    },
    AiTraceRuntime {
        event_record_ids: Vec<String>,
    },
}

impl Xui26ReplayPayload {
    pub fn slot_id(&self) -> &'static str {
        match self {
            Self::ScenarioSetup { .. } => "scenario_setup",
            Self::TutorialState { .. } => "tutorial_state",
            Self::SettingsSnapshot { .. } => "settings_snapshot",
            Self::MessageSettingsSnapshot { .. } => "message_settings_snapshot",
            Self::MessageFilterSnapshot { .. } => "message_filter_snapshot",
            Self::SavedGameRules { .. } => "saved_game_rules",
            Self::PlayerAiPreferences { .. } => "player_ai_preferences",
            Self::LiveLobby { .. } => "live_lobby",
            Self::HotjoinState { .. } => "hotjoin_state",
            Self::MatchmakingChat { .. } => "matchmaking_chat",
            Self::AiTraceRuntime { .. } => "ai_trace_runtime",
        }
    }

    fn is_valid(&self) -> bool {
        fn sha256(value: &str) -> bool {
            value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        }
        fn nonempty_unique(values: &[String]) -> bool {
            values.iter().all(|value| !value.trim().is_empty())
                && values.iter().collect::<BTreeSet<_>>().len() == values.len()
        }
        fn assignments(values: &BTreeMap<String, String>) -> bool {
            values.keys().all(|key| !key.trim().is_empty())
        }
        match self {
            Self::ScenarioSetup {
                scenario_id,
                bookmark_id,
                country_tag,
                game_rules_sha256,
                bookmark_ids_in_order,
                country_tags_in_order,
                game_rule_ids_in_order,
            } => {
                !scenario_id.trim().is_empty()
                    && !bookmark_id.trim().is_empty()
                    && !country_tag.trim().is_empty()
                    && sha256(game_rules_sha256)
                    && nonempty_unique(bookmark_ids_in_order)
                    && nonempty_unique(country_tags_in_order)
                    && nonempty_unique(game_rule_ids_in_order)
                    && bookmark_ids_in_order.contains(bookmark_id)
                    && country_tags_in_order.contains(country_tag)
            }
            Self::TutorialState {
                tutorial_id,
                step_id,
                completed_step_ids,
            } => {
                !tutorial_id.trim().is_empty()
                    && !step_id.trim().is_empty()
                    && nonempty_unique(completed_step_ids)
            }
            Self::SettingsSnapshot {
                settings_file_sha256,
                assignments: values,
            }
            | Self::MessageSettingsSnapshot {
                settings_file_sha256,
                assignments: values,
            }
            | Self::MessageFilterSnapshot {
                settings_file_sha256,
                assignments: values,
            } => sha256(settings_file_sha256) && assignments(values),
            Self::SavedGameRules {
                rules_sha256,
                assignments: values,
            } => sha256(rules_sha256) && assignments(values),
            Self::PlayerAiPreferences {
                assignments: values,
            } => assignments(values),
            Self::LiveLobby {
                player_keys_in_slot_order,
                country_tags_in_slot_order,
            } => {
                player_keys_in_slot_order.len() == country_tags_in_slot_order.len()
                    && nonempty_unique(player_keys_in_slot_order)
                    && country_tags_in_slot_order
                        .iter()
                        .all(|tag| !tag.trim().is_empty())
            }
            Self::HotjoinState { event_record_ids }
            | Self::MatchmakingChat { event_record_ids }
            | Self::AiTraceRuntime { event_record_ids } => nonempty_unique(event_record_ids),
        }
    }

    /// Family-domain collection groups in canonical host order. Keeping each
    /// group separate prevents a carrier from preserving one flattened row
    /// sequence while moving bookmark/country/rule (or analogous) rows into
    /// the wrong authored collection.
    fn collection_groups(&self) -> Option<Vec<(String, Vec<String>)>> {
        let prefixed = |prefix: &str, values: &[String]| {
            values
                .iter()
                .map(|value| format!("{prefix}:{value}"))
                .collect::<Vec<_>>()
        };
        match self {
            Self::ScenarioSetup {
                bookmark_ids_in_order,
                country_tags_in_order,
                game_rule_ids_in_order,
                ..
            } => Some(vec![
                (
                    "bookmarks".into(),
                    prefixed("bookmark", bookmark_ids_in_order),
                ),
                (
                    "countries".into(),
                    prefixed("country", country_tags_in_order),
                ),
                (
                    "game_rules".into(),
                    prefixed("rule", game_rule_ids_in_order),
                ),
            ]),
            Self::MessageFilterSnapshot { assignments, .. } => Some(vec![(
                "message_filters".into(),
                assignments
                    .keys()
                    .map(|key| format!("filter:{key}"))
                    .collect(),
            )]),
            Self::SavedGameRules { assignments, .. } => Some(vec![(
                "saved_game_rules".into(),
                assignments
                    .keys()
                    .map(|key| format!("rule:{key}"))
                    .collect(),
            )]),
            Self::LiveLobby {
                player_keys_in_slot_order,
                ..
            } => Some(vec![(
                "players".into(),
                prefixed("player", player_keys_in_slot_order),
            )]),
            Self::HotjoinState { event_record_ids } => Some(vec![(
                "hotjoin_requests".into(),
                prefixed("event", event_record_ids),
            )]),
            Self::MatchmakingChat { event_record_ids } => Some(vec![(
                "chat_messages".into(),
                prefixed("event", event_record_ids),
            )]),
            Self::AiTraceRuntime { event_record_ids } => Some(vec![(
                "trace_entries".into(),
                prefixed("event", event_record_ids),
            )]),
            Self::TutorialState { .. }
            | Self::SettingsSnapshot { .. }
            | Self::MessageSettingsSnapshot { .. }
            | Self::PlayerAiPreferences { .. } => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Xui26ElementSemantic {
    Hidden,
    Structural,
    ReadOnlyButton { one_based_frame: u32 },
    PassiveSprite { one_based_frame: u32 },
    Text { resolved_text: String },
    Progress { numerator: u64, denominator: u64 },
}

/// One exact authored row instance in producer order. The producer supplies
/// semantic membership and the effective-VFS template identity; Replay
/// re-admits that template and derives all geometry, clipping and paint order
/// from the authenticated authored collection law.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xui26ReplayCollectionRow {
    pub stable_item_id: String,
    pub subtype_id: String,
    pub template_source_path: String,
    pub template_source_content_sha256: String,
    pub template_surface_id: String,
    pub element_semantics: BTreeMap<String, Xui26ElementSemantic>,
}

impl Xui26ReplayCollectionRow {
    fn is_valid(&self) -> bool {
        !self.stable_item_id.trim().is_empty()
            && !self.subtype_id.trim().is_empty()
            && !self.template_source_path.trim().is_empty()
            && self.template_source_content_sha256.len() == 64
            && self
                .template_source_content_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            && !self.template_surface_id.trim().is_empty()
            && !self.element_semantics.is_empty()
            && self
                .element_semantics
                .iter()
                .all(|(key, semantic)| !key.trim().is_empty() && semantic.is_valid())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xui26ReplayCollection {
    /// Typed family-domain group, independently joined to an exact authored
    /// host/template law by `game_ui`.
    pub group_id: String,
    pub collection_element_id: String,
    pub complete: bool,
    /// Exact effective template retained even for measured-empty membership.
    pub empty_template_source_path: String,
    pub empty_template_source_content_sha256: String,
    pub empty_template_surface_id: String,
    pub rows: Vec<Xui26ReplayCollectionRow>,
}

impl Xui26ReplayCollection {
    fn is_valid(&self) -> bool {
        self.complete
            && !self.group_id.trim().is_empty()
            && !self.collection_element_id.trim().is_empty()
            && !self.empty_template_source_path.trim().is_empty()
            && self.empty_template_source_content_sha256.len() == 64
            && self
                .empty_template_source_content_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            && !self.empty_template_surface_id.trim().is_empty()
            && self.rows.iter().all(Xui26ReplayCollectionRow::is_valid)
            && self
                .rows
                .iter()
                .map(|row| row.stable_item_id.as_str())
                .collect::<BTreeSet<_>>()
                .len()
                == self.rows.len()
    }
}

impl Xui26ElementSemantic {
    fn is_valid(&self) -> bool {
        match self {
            Self::ReadOnlyButton { one_based_frame } | Self::PassiveSprite { one_based_frame } => {
                *one_based_frame > 0
            }
            Self::Text { resolved_text } => !resolved_text.contains('\0'),
            Self::Progress {
                numerator,
                denominator,
            } => *denominator > 0 && numerator <= denominator,
            Self::Hidden | Self::Structural => true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xui26ReplayFamilyCarrier {
    pub family_id: String,
    pub slot_id: String,
    pub source_record_id: String,
    pub source_content_sha256: String,
    pub complete: bool,
    /// Canonical SHA-256 of this carrier with this field empty. This binds the
    /// typed domain payload and final element semantics into one record.
    pub record_sha256: String,
    pub payload: Xui26ReplayPayload,
    pub element_semantics: BTreeMap<String, Xui26ElementSemantic>,
    pub collections: Vec<Xui26ReplayCollection>,
}

impl Xui26ReplayFamilyCarrier {
    fn is_valid(&self) -> bool {
        let family_slot_valid = matches!(
            (self.family_id.as_str(), self.slot_id.as_str()),
            ("frontend_game_setup", "scenario_setup")
                | ("tutorial_screen", "tutorial_state")
                | ("settings", "settings_snapshot")
                | ("message_settings", "message_settings_snapshot")
                | ("message_filters", "message_filter_snapshot")
                | ("saved_game_rules", "saved_game_rules")
                | ("player_ai_preferences", "player_ai_preferences")
                | ("ingame_lobby", "live_lobby")
                | ("player_lobby", "live_lobby")
                | ("hotjoin", "hotjoin_state")
                | ("matchmaking_chat", "matchmaking_chat")
                | ("ai_trace", "ai_trace_runtime")
        );
        let expected_collection_groups = self.payload.collection_groups();
        let collections_valid = self.collections.iter().all(Xui26ReplayCollection::is_valid)
            && self
                .collections
                .iter()
                .map(|collection| collection.collection_element_id.as_str())
                .collect::<BTreeSet<_>>()
                .len()
                == self.collections.len()
            && self
                .collections
                .iter()
                .map(|collection| collection.group_id.as_str())
                .collect::<BTreeSet<_>>()
                .len()
                == self.collections.len()
            && match expected_collection_groups {
                Some(expected) => {
                    expected.len() == self.collections.len()
                        && expected.iter().zip(&self.collections).all(
                            |((expected_group, expected_rows), collection)| {
                                collection.group_id == *expected_group
                                    && collection
                                        .rows
                                        .iter()
                                        .map(|row| row.stable_item_id.as_str())
                                        .eq(expected_rows.iter().map(String::as_str))
                            },
                        )
                }
                None => self.collections.is_empty(),
            };
        !self.family_id.trim().is_empty()
            && family_slot_valid
            && self.slot_id == self.payload.slot_id()
            && !self.source_record_id.trim().is_empty()
            && self.complete
            && xui26_canonical_record_sha256(self).as_deref() == Some(self.record_sha256.as_str())
            && self.source_content_sha256.len() == 64
            && self
                .source_content_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            && self.payload.is_valid()
            && !self.element_semantics.is_empty()
            && self
                .element_semantics
                .iter()
                .all(|(key, semantic)| !key.trim().is_empty() && semantic.is_valid())
            && collections_valid
    }
}

pub fn xui26_canonical_record_sha256(carrier: &Xui26ReplayFamilyCarrier) -> Option<String> {
    let mut canonical = carrier.clone();
    canonical.record_sha256.clear();
    serde_json::to_vec(&canonical)
        .ok()
        .map(|bytes| Sha256::hex(&bytes))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Xui26FrontendView {
    pub status: ViewStatus,
    pub frame_hour: i64,
    pub product_modset_sha256: Option<String>,
    pub localisation_language: Option<String>,
    pub carriers: BTreeMap<String, Xui26ReplayFamilyCarrier>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    schema: String,
    hour_index: i64,
    authority: String,
    complete: bool,
    product_modset_sha256: String,
    localisation_language: String,
    carriers: Vec<Value>,
}

pub(crate) fn build_xui26_frontend(
    rest: &Map<String, Value>,
    frame_hour: i64,
    warnings: &mut Vec<String>,
) -> Xui26FrontendView {
    let Some(raw) = rest.get(XUI26_FRONTEND_CARRIER_KEY) else {
        return Xui26FrontendView {
            status: ViewStatus::DomainAbsent,
            frame_hour,
            product_modset_sha256: None,
            localisation_language: None,
            carriers: BTreeMap::new(),
        };
    };
    if raw.is_null() {
        return Xui26FrontendView {
            status: ViewStatus::DomainUnavailable,
            frame_hour,
            product_modset_sha256: None,
            localisation_language: None,
            carriers: BTreeMap::new(),
        };
    }
    let envelope: Envelope = match serde_json::from_value(raw.clone()) {
        Ok(value) => value,
        Err(error) => {
            warnings.push(format!(
                "{XUI26_FRONTEND_CARRIER_KEY}: malformed carrier: {error}"
            ));
            return Xui26FrontendView {
                status: ViewStatus::DomainUnavailable,
                frame_hour,
                product_modset_sha256: None,
                localisation_language: None,
                carriers: BTreeMap::new(),
            };
        }
    };
    let sha = |value: &str| {
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    };
    let envelope_valid = envelope.schema == XUI26_FRONTEND_CARRIER_SCHEMA
        && envelope.hour_index == frame_hour
        && envelope.authority == "replay_payload"
        && envelope.complete
        && sha(&envelope.product_modset_sha256)
        && !envelope.localisation_language.trim().is_empty();
    if !envelope_valid {
        warnings.push(format!("{XUI26_FRONTEND_CARRIER_KEY}: envelope identity or completeness rejected for WorldFrame hour {frame_hour}"));
        return Xui26FrontendView {
            status: ViewStatus::DomainUnavailable,
            frame_hour,
            product_modset_sha256: None,
            localisation_language: None,
            carriers: BTreeMap::new(),
        };
    }
    let mut carriers = BTreeMap::new();
    let mut rejected_families = BTreeSet::new();
    for raw_carrier in envelope.carriers {
        let family_hint = raw_carrier
            .get("family_id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_owned);
        let parsed = serde_json::from_value::<Xui26ReplayFamilyCarrier>(raw_carrier);
        let Ok(carrier) = parsed else {
            if let Some(family) = family_hint {
                rejected_families.insert(family.clone());
                carriers.remove(&family);
                warnings.push(format!(
                    "{XUI26_FRONTEND_CARRIER_KEY}: malformed family {family:?} rejected locally"
                ));
            } else {
                warnings.push(format!(
                    "{XUI26_FRONTEND_CARRIER_KEY}: unidentifiable malformed family row ignored"
                ));
            }
            continue;
        };
        let family = carrier.family_id.clone();
        if rejected_families.contains(&family)
            || !carrier.is_valid()
            || carriers.insert(family.clone(), carrier).is_some()
        {
            rejected_families.insert(family.clone());
            carriers.remove(&family);
            warnings.push(format!(
                "{XUI26_FRONTEND_CARRIER_KEY}: invalid or duplicate family {family:?} rejected locally"
            ));
        }
    }
    Xui26FrontendView {
        status: ViewStatus::Present,
        frame_hour,
        product_modset_sha256: Some(envelope.product_modset_sha256),
        localisation_language: Some(envelope.localisation_language),
        carriers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn valid() -> Value {
        let mut value = json!({
            "schema": XUI26_FRONTEND_CARRIER_SCHEMA,
            "hour_index": 7,
            "authority": "replay_payload",
            "complete": true,
            "product_modset_sha256": "a".repeat(64),
            "localisation_language": "l_english",
            "carriers": [{
                "family_id": "hotjoin", "slot_id": "hotjoin_state",
                "source_record_id": "d57:hotjoin:7", "source_content_sha256": "b".repeat(64),
                "complete": true, "record_sha256": "",
                "payload": { "kind": "hotjoin_state", "event_record_ids": ["event:7"] },
                "element_semantics": { "role:0": { "kind": "text", "resolved_text": "Joining" } },
                "collections": [{
                    "group_id": "hotjoin_requests",
                    "collection_element_id": "role:grid",
                    "complete": true,
                    "empty_template_source_path": "interface/hotjoin.gui",
                    "empty_template_source_content_sha256": "b".repeat(64),
                    "empty_template_surface_id": "hotjoin_event_entry",
                    "rows": [{
                        "stable_item_id": "event:event:7",
                        "subtype_id": "hotjoin_event",
                        "template_source_path": "interface/hotjoin.gui",
                        "template_source_content_sha256": "b".repeat(64),
                        "template_surface_id": "hotjoin_event_entry",
                        "element_semantics": { "row:0": { "kind": "text", "resolved_text": "Joining" } }
                    }]
                }]
            }]
        });
        let carrier: Xui26ReplayFamilyCarrier =
            serde_json::from_value(value["carriers"][0].clone()).unwrap();
        value["carriers"][0]["record_sha256"] =
            json!(xui26_canonical_record_sha256(&carrier).unwrap());
        value
    }

    #[test]
    fn missing_empty_and_malformed_remain_distinct() {
        let mut warnings = Vec::new();
        assert_eq!(
            build_xui26_frontend(&Map::new(), 7, &mut warnings).status,
            ViewStatus::DomainAbsent
        );
        let mut rest = Map::from_iter([(XUI26_FRONTEND_CARRIER_KEY.into(), Value::Null)]);
        assert_eq!(
            build_xui26_frontend(&rest, 7, &mut warnings).status,
            ViewStatus::DomainUnavailable
        );
        rest.insert(
            XUI26_FRONTEND_CARRIER_KEY.into(),
            json!({"schema": "wrong"}),
        );
        assert_eq!(
            build_xui26_frontend(&rest, 7, &mut warnings).status,
            ViewStatus::DomainUnavailable
        );
    }

    #[test]
    fn complete_typed_carrier_is_present_and_invalid_family_is_rejected_locally() {
        let mut warnings = Vec::new();
        let rest = Map::from_iter([(XUI26_FRONTEND_CARRIER_KEY.into(), valid())]);
        assert_eq!(
            build_xui26_frontend(&rest, 7, &mut warnings).status,
            ViewStatus::Present
        );
        assert_eq!(
            build_xui26_frontend(&rest, 8, &mut warnings).status,
            ViewStatus::DomainUnavailable
        );
        let mut bad = valid();
        bad["carriers"][0]["element_semantics"] = json!({});
        let rest = Map::from_iter([(XUI26_FRONTEND_CARRIER_KEY.into(), bad)]);
        let view = build_xui26_frontend(&rest, 7, &mut warnings);
        assert_eq!(view.status, ViewStatus::Present);
        assert!(view.carriers.is_empty());

        let mut tampered = valid();
        tampered["carriers"][0]["payload"]["event_record_ids"] = json!(["event:changed"]);
        let rest = Map::from_iter([(XUI26_FRONTEND_CARRIER_KEY.into(), tampered)]);
        let view = build_xui26_frontend(&rest, 7, &mut warnings);
        assert_eq!(view.status, ViewStatus::Present);
        assert!(view.carriers.is_empty());
    }

    #[test]
    fn one_malformed_family_does_not_hide_an_independent_valid_family() {
        let mut value = valid();
        let valid_row = value["carriers"][0].clone();
        let mut malformed = valid_row.clone();
        malformed["family_id"] = json!("settings");
        malformed["payload"] = json!({"kind": "not_a_payload"});
        value["carriers"] = json!([malformed, valid_row]);
        let rest = Map::from_iter([(XUI26_FRONTEND_CARRIER_KEY.into(), value)]);
        let mut warnings = Vec::new();
        let view = build_xui26_frontend(&rest, 7, &mut warnings);
        assert_eq!(view.status, ViewStatus::Present);
        assert_eq!(
            view.carriers.keys().cloned().collect::<Vec<_>>(),
            vec!["hotjoin"]
        );
        assert!(warnings.iter().any(|warning| warning.contains("settings")));
    }

    #[test]
    fn collection_order_and_seal_are_authenticated_and_empty_is_present() {
        let mut out_of_order = valid();
        out_of_order["carriers"][0]["collections"][0]["rows"][0]["stable_item_id"] =
            json!("event:other");
        let carrier: Xui26ReplayFamilyCarrier =
            serde_json::from_value(out_of_order["carriers"][0].clone()).unwrap();
        out_of_order["carriers"][0]["record_sha256"] =
            json!(xui26_canonical_record_sha256(&carrier).unwrap());
        let rest = Map::from_iter([(XUI26_FRONTEND_CARRIER_KEY.into(), out_of_order)]);
        let mut warnings = Vec::new();
        assert!(build_xui26_frontend(&rest, 7, &mut warnings)
            .carriers
            .is_empty());

        let mut measured_empty = valid();
        measured_empty["carriers"][0]["payload"]["event_record_ids"] = json!([]);
        measured_empty["carriers"][0]["collections"][0]["rows"] = json!([]);
        let carrier: Xui26ReplayFamilyCarrier =
            serde_json::from_value(measured_empty["carriers"][0].clone()).unwrap();
        measured_empty["carriers"][0]["record_sha256"] =
            json!(xui26_canonical_record_sha256(&carrier).unwrap());
        let rest = Map::from_iter([(XUI26_FRONTEND_CARRIER_KEY.into(), measured_empty)]);
        let view = build_xui26_frontend(&rest, 7, &mut warnings);
        assert_eq!(view.carriers["hotjoin"].collections.len(), 1);
        assert!(view.carriers["hotjoin"].collections[0].rows.is_empty());
    }

    #[test]
    fn scenario_collection_groups_cannot_be_swapped_behind_a_valid_seal() {
        let mut value = valid();
        let base_collection = value["carriers"][0]["collections"][0].clone();
        let collection = |group: &str, host: &str, stable_id: &str| {
            let mut collection = base_collection.clone();
            collection["group_id"] = json!(group);
            collection["collection_element_id"] = json!(host);
            collection["rows"][0]["stable_item_id"] = json!(stable_id);
            collection
        };
        value["carriers"][0]["family_id"] = json!("frontend_game_setup");
        value["carriers"][0]["slot_id"] = json!("scenario_setup");
        value["carriers"][0]["payload"] = json!({
            "kind": "scenario_setup",
            "scenario_id": "1936",
            "bookmark_id": "1936",
            "country_tag": "GER",
            "game_rules_sha256": "c".repeat(64),
            "bookmark_ids_in_order": ["1936"],
            "country_tags_in_order": ["GER"],
            "game_rule_ids_in_order": ["difficulty"]
        });
        value["carriers"][0]["collections"] = json!([
            collection("bookmarks", "role:bookmarks", "bookmark:1936"),
            collection("countries", "role:countries", "country:GER"),
            collection("game_rules", "role:rules", "rule:difficulty")
        ]);
        let carrier: Xui26ReplayFamilyCarrier =
            serde_json::from_value(value["carriers"][0].clone()).unwrap();
        value["carriers"][0]["record_sha256"] =
            json!(xui26_canonical_record_sha256(&carrier).unwrap());
        let valid_carrier: Xui26ReplayFamilyCarrier =
            serde_json::from_value(value["carriers"][0].clone()).unwrap();
        assert!(valid_carrier.is_valid());

        value["carriers"][0]["collections"]
            .as_array_mut()
            .unwrap()
            .swap(0, 1);
        let swapped: Xui26ReplayFamilyCarrier =
            serde_json::from_value(value["carriers"][0].clone()).unwrap();
        value["carriers"][0]["record_sha256"] =
            json!(xui26_canonical_record_sha256(&swapped).unwrap());
        let rest = Map::from_iter([(XUI26_FRONTEND_CARRIER_KEY.into(), value)]);
        let mut warnings = Vec::new();
        assert!(build_xui26_frontend(&rest, 7, &mut warnings)
            .carriers
            .is_empty());
    }
}
