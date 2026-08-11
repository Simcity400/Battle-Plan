//! Strict passive all-country XUI19 decision-evaluation carrier.
//!
//! This is a replay-file admission boundary, not an Extractor implementation.
//! It accepts only an atomic current-hour census keyed to the complete D18
//! country roster.  It never evaluates Clausewitz triggers, reads recorder UI
//! state, or adapts the retired selected-Present decision-panel carrier.

use crate::{D18CountryRosterView, ViewStatus};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const XUI19_CARRIER_KEY: &str = "_r2_decision_evaluation_exact";
pub const XUI19_SCHEMA: &str = "xui19.decision_evaluation_exact.v1";
pub const XUI19_AUTHORITY: &str = "exact_a729_passive_all_country_decisions";
pub const XUI19_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
/// Hash of the exact-a729 function evidence manifest cited by the XUI19 law.
pub const XUI19_EVIDENCE_MANIFEST_SHA256: &str =
    "59650965bd79b711974aa4f3cd8c34fdf1b111e36fa0a294916839955dac1ec1";
/// SHA-256 of `EXTERNAL_PRODUCER_REQUIREMENTS.md` with repository LF bytes.
pub const XUI19_PRODUCER_CONTRACT_SHA256: &str =
    "9235a5abd82bb7b916f13ebdc09b03ed4e7ad79166a5c15a908c1aac97624108";

const MAX_COUNTRIES: usize = 65_536;
const MAX_CATEGORIES_PER_COUNTRY: usize = 65_536;
const MAX_CARDS_PER_COUNTRY: usize = 131_072;
const MAX_TEXT_BYTES: usize = 65_536;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionCardAvailabilityExact {
    Available,
    Unavailable,
    Active,
    OnCooldown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionTimerKindExact {
    OrdinaryActive,
    Mission,
    Cooldown,
    Targeted,
    TargetedTimed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DecisionTargetExactV1 {
    Country {
        tag: String,
        tag_db_id: u32,
        name_text: String,
        flag_tag: String,
    },
    State {
        state_id: u32,
        name_text: String,
    },
    Numeric {
        id: u32,
        name_text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionCostExactV1 {
    /// Engine fixed-point value after modifiers.
    pub value_raw: i64,
    pub scale: i64,
    /// Engine resource identity, not a UI-selected icon name.
    pub resource_kind: String,
    pub custom_cost_available: Option<bool>,
    /// Exact engine-formatted text in `localisation_language`.
    pub formatted_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionTimerExactV1 {
    pub kind: DecisionTimerKindExact,
    /// Raw engine value and its proven scale/unit identity.
    pub value_raw: i64,
    pub scale: i64,
    pub unit: String,
    pub progress_current_raw: Option<i64>,
    pub progress_max_raw: Option<i64>,
    /// Exact engine-formatted text in `localisation_language`.
    pub formatted_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionCardExactV1 {
    pub source_ordinal: u32,
    pub source_record_id: String,
    pub source_record_sha256: String,
    pub decision_id: String,
    pub category_id: String,
    pub database_ordinal: u32,
    pub category_priority: i32,
    pub decision_priority: i32,
    /// Rows are the complete engine-visible set.  This explicit witness keeps
    /// a malformed producer from smuggling hidden definitions into the list.
    pub visible: bool,
    pub availability: DecisionCardAvailabilityExact,
    pub interface_state: u32,
    pub target: Option<DecisionTargetExactV1>,
    pub cost: DecisionCostExactV1,
    pub timer: Option<DecisionTimerExactV1>,
    pub alert_to_take: bool,
    pub alert_timeout: bool,
    pub name_text: String,
    pub description_text: String,
    pub tooltip_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionCustomIconExactV1 {
    pub source_ordinal: u32,
    pub source_record_id: String,
    pub authored_tag: String,
    pub value_loc_key: String,
    pub desc_loc_key: String,
    pub value_text: String,
    pub tooltip_text: String,
    pub flag_tag: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionCategoryExactV1 {
    pub source_ordinal: u32,
    /// Stable producer identity for this exact category row.
    pub source_record_id: String,
    pub category_id: String,
    pub category_priority: i32,
    pub name_text: String,
    pub description_text: String,
    pub custom_icon_definition_count: u32,
    /// Complete engine-visible subset in authored declaration order.
    pub custom_icons: Vec<DecisionCustomIconExactV1>,
    /// Exact engine order within this category.
    pub cards: Vec<DecisionCardExactV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionCountryExactV1 {
    pub source_ordinal: u32,
    pub country_tag: String,
    pub tag_db_id: u32,
    pub definition_count: u32,
    pub category_count: u32,
    pub custom_icon_count: u32,
    pub visible_card_count: u32,
    pub complete: bool,
    pub source_record_id: String,
    /// Canonical hash of this row with this member cleared.
    pub source_record_sha256: String,
    /// Exact category and card order selected by the engine.
    pub categories: Vec<DecisionCategoryExactV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionEvaluationExactV1 {
    pub schema: String,
    pub authority: String,
    pub executable_sha256: String,
    pub evidence_manifest_sha256: String,
    pub producer_contract_sha256: String,
    pub content_manifest_sha256: String,
    pub decision_catalog_sha256: String,
    pub localisation_language: String,
    pub frame_hour: i64,
    pub source_sequence: u64,
    pub batch_id_sha256: String,
    pub expected_country_count: u32,
    /// Canonical SHA-256 of the ordered D18 `(tag, tag_db_id)` census.
    pub d18_roster_sha256: String,
    pub passive_all_country: bool,
    pub ui_open_independent: bool,
    pub recorder_pov_independent: bool,
    pub producer_complete: bool,
    pub previous_record_sha256: Option<String>,
    /// Canonical hash of this carrier with this member cleared.
    pub record_sha256: String,
    pub countries: Vec<DecisionCountryExactV1>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionEvaluationExactView {
    pub status: ViewStatus,
    pub carrier: Option<DecisionEvaluationExactV1>,
    pub diagnostic: Option<String>,
}

impl DecisionEvaluationExactView {
    fn absent() -> Self {
        Self {
            status: ViewStatus::DomainAbsent,
            carrier: None,
            diagnostic: Some("XUI19 passive decision carrier was not recorded".into()),
        }
    }

    fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            status: ViewStatus::DomainUnavailable,
            carrier: None,
            diagnostic: Some(reason.into()),
        }
    }

    pub fn country(&self, tag: &str) -> Option<&DecisionCountryExactV1> {
        self.carrier
            .as_ref()?
            .countries
            .iter()
            .find(|country| country.country_tag == tag)
    }
}

fn lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && !value.as_bytes().contains(&0)
        && !value.chars().any(char::is_whitespace)
}

fn text(value: &str, required: bool) -> bool {
    value.len() <= MAX_TEXT_BYTES
        && !value.as_bytes().contains(&0)
        && (!required || !value.is_empty())
}

fn canonical_sha256<T: Serialize + Clone>(value: &T, clear: impl FnOnce(&mut T)) -> Option<String> {
    let mut canonical = value.clone();
    clear(&mut canonical);
    serde_json::to_vec(&canonical)
        .ok()
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
}

pub fn country_record_sha256(country: &DecisionCountryExactV1) -> Option<String> {
    canonical_sha256(country, |value| value.source_record_sha256.clear())
}

pub fn card_record_sha256(card: &DecisionCardExactV1) -> Option<String> {
    canonical_sha256(card, |value| value.source_record_sha256.clear())
}

pub fn carrier_record_sha256(carrier: &DecisionEvaluationExactV1) -> Option<String> {
    canonical_sha256(carrier, |value| value.record_sha256.clear())
}

/// Hash the complete ordered D18 identity census without depending on the
/// replay carrier's independently supplied country rows.
pub fn d18_roster_sha256(roster: &D18CountryRosterView) -> Option<String> {
    if roster.status != ViewStatus::Present {
        return None;
    }
    let canonical: Vec<(&str, u32)> = roster
        .entries
        .iter()
        .map(|entry| (entry.tag.as_str(), entry.tag_db_id))
        .collect();
    serde_json::to_vec(&canonical)
        .ok()
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
}

fn target_valid(target: &DecisionTargetExactV1) -> bool {
    match target {
        DecisionTargetExactV1::Country {
            tag,
            tag_db_id,
            name_text,
            flag_tag,
        } => token(tag) && *tag_db_id != 0 && text(name_text, true) && token(flag_tag),
        DecisionTargetExactV1::State {
            state_id,
            name_text,
        } => *state_id != 0 && text(name_text, true),
        DecisionTargetExactV1::Numeric { id, name_text } => {
            *id != 0 && text(name_text, true)
        }
    }
}

fn timer_valid(timer: &DecisionTimerExactV1) -> bool {
    timer.scale > 0
        && token(&timer.unit)
        && text(&timer.formatted_text, true)
        && match (timer.progress_current_raw, timer.progress_max_raw) {
            (None, None) => true,
            (Some(current), Some(maximum)) => maximum > 0 && current >= 0 && current <= maximum,
            _ => false,
        }
}

fn interface_state_matches_availability(
    interface_state: u32,
    availability: DecisionCardAvailabilityExact,
    cooldown_timer: bool,
) -> bool {
    if cooldown_timer {
        // The exact engine cooldown branch is timer-owned and bypasses the
        // ordinary interface-state family switch. `card_valid` separately
        // requires the OnCooldown availability in both directions.
        return availability == DecisionCardAvailabilityExact::OnCooldown;
    }
    match interface_state {
        1 | 6 | 9 => availability == DecisionCardAvailabilityExact::Available,
        3 | 4 | 10 | 11 => availability == DecisionCardAvailabilityExact::Active,
        0 | 2 | 5 | 7 | 8 | 12 => {
            availability == DecisionCardAvailabilityExact::Unavailable
        }
        _ => false,
    }
}

fn card_valid(
    card: &DecisionCardExactV1,
    expected_ordinal: usize,
    category: &DecisionCategoryExactV1,
    definition_count: u32,
) -> bool {
    let cooldown_timer = card
        .timer
        .as_ref()
        .is_some_and(|timer| timer.kind == DecisionTimerKindExact::Cooldown);
    card.source_ordinal as usize == expected_ordinal
        && token(&card.source_record_id)
        && lower_sha256(&card.source_record_sha256)
        && card_record_sha256(card).as_deref() == Some(card.source_record_sha256.as_str())
        && token(&card.decision_id)
        && card.category_id == category.category_id
        && card.database_ordinal < definition_count
        && card.category_priority == category.category_priority
        && card.visible
        && (card.availability == DecisionCardAvailabilityExact::OnCooldown) == cooldown_timer
        && interface_state_matches_availability(
            card.interface_state,
            card.availability,
            cooldown_timer,
        )
        && (!card.alert_to_take
            || card.availability == DecisionCardAvailabilityExact::Available)
        && (!card.alert_timeout || card.timer.is_some())
        && card.target.as_ref().is_none_or(target_valid)
        && card.cost.scale > 0
        && token(&card.cost.resource_kind)
        && text(&card.cost.formatted_text, true)
        && card.timer.as_ref().is_none_or(timer_valid)
        && text(&card.name_text, true)
        && text(&card.description_text, false)
        && text(&card.tooltip_text, false)
}

/// Admit the all-country carrier.  A malformed row rejects the whole hour;
/// measured-empty countries remain Present with an empty category vector.
pub fn admit_decision_evaluation(
    raw: &Map<String, Value>,
    expected_hour: i64,
    roster: &D18CountryRosterView,
) -> DecisionEvaluationExactView {
    let Some(value) = raw.get(XUI19_CARRIER_KEY) else {
        return DecisionEvaluationExactView::absent();
    };
    if value.is_null() {
        return DecisionEvaluationExactView::unavailable(
            "XUI19 passive decision carrier was explicitly unobtained",
        );
    }
    let carrier: DecisionEvaluationExactV1 = match serde_json::from_value(value.clone()) {
        Ok(value) => value,
        Err(error) => {
            return DecisionEvaluationExactView::unavailable(format!(
                "XUI19 passive decision carrier strict decode failed: {error}"
            ))
        }
    };
    if carrier.schema != XUI19_SCHEMA
        || carrier.authority != XUI19_AUTHORITY
        || carrier.executable_sha256 != XUI19_EXECUTABLE_SHA256
        || carrier.evidence_manifest_sha256 != XUI19_EVIDENCE_MANIFEST_SHA256
        || carrier.producer_contract_sha256 != XUI19_PRODUCER_CONTRACT_SHA256
        || !lower_sha256(&carrier.content_manifest_sha256)
        || !lower_sha256(&carrier.decision_catalog_sha256)
        || !token(&carrier.localisation_language)
        || carrier.frame_hour != expected_hour
        || carrier.source_sequence == 0
        || !lower_sha256(&carrier.batch_id_sha256)
        || !lower_sha256(&carrier.d18_roster_sha256)
        || !carrier.passive_all_country
        || !carrier.ui_open_independent
        || !carrier.recorder_pov_independent
        || !carrier.producer_complete
        || carrier.previous_record_sha256.as_deref().is_some_and(|hash| !lower_sha256(hash))
        || !lower_sha256(&carrier.record_sha256)
        || carrier_record_sha256(&carrier).as_deref() != Some(carrier.record_sha256.as_str())
        || roster.status != ViewStatus::Present
        || d18_roster_sha256(roster).as_deref() != Some(carrier.d18_roster_sha256.as_str())
        || carrier.countries.is_empty()
        || carrier.countries.len() > MAX_COUNTRIES
        || carrier.expected_country_count as usize != carrier.countries.len()
        || carrier.countries.len() != roster.entries.len()
    {
        return DecisionEvaluationExactView::unavailable(
            "XUI19 passive authority, hour, completeness, D18 census, or record hash rejected",
        );
    }

    let mut country_records = BTreeSet::new();
    for (country_ordinal, (country, identity)) in carrier
        .countries
        .iter()
        .zip(roster.entries.iter())
        .enumerate()
    {
        if country.source_ordinal as usize != country_ordinal
            || country.country_tag != identity.tag
            || country.tag_db_id != identity.tag_db_id
            || !country.complete
            || !token(&country.source_record_id)
            || !country_records.insert(country.source_record_id.as_str())
            || !lower_sha256(&country.source_record_sha256)
            || country_record_sha256(country).as_deref()
                != Some(country.source_record_sha256.as_str())
            || country.categories.len() > MAX_CATEGORIES_PER_COUNTRY
        {
            return DecisionEvaluationExactView::unavailable(format!(
                "XUI19 country row {} identity/completeness/hash rejected",
                identity.tag
            ));
        }
        let mut category_ids = BTreeSet::new();
        let mut category_records = BTreeSet::new();
        let mut card_identities = BTreeSet::new();
        let mut card_count = 0usize;
        let mut custom_icon_count = 0usize;
        let mut prior_category_priority = None;
        for (category_ordinal, category) in country.categories.iter().enumerate() {
            if category.source_ordinal as usize != category_ordinal
                || !token(&category.source_record_id)
                || !category_records.insert(category.source_record_id.as_str())
                || !token(&category.category_id)
                || !category_ids.insert(category.category_id.as_str())
                || !text(&category.name_text, true)
                || !text(&category.description_text, false)
                || category.custom_icons.len() > category.custom_icon_definition_count as usize
                || prior_category_priority
                    .is_some_and(|previous| category.category_priority > previous)
            {
                return DecisionEvaluationExactView::unavailable(format!(
                    "XUI19 country {} category order/content rejected",
                    identity.tag
                ));
            }
            prior_category_priority = Some(category.category_priority);
            let mut custom_icon_ids = BTreeSet::new();
            let mut prior_custom_icon_ordinal = None;
            for icon in &category.custom_icons {
                if icon.source_ordinal >= category.custom_icon_definition_count
                    || prior_custom_icon_ordinal
                        .is_some_and(|previous| icon.source_ordinal <= previous)
                    || !token(&icon.source_record_id)
                    || !custom_icon_ids.insert(icon.source_record_id.as_str())
                    || !token(&icon.authored_tag)
                    || !token(&icon.value_loc_key)
                    || !token(&icon.desc_loc_key)
                    || !text(&icon.value_text, true)
                    || !text(&icon.tooltip_text, false)
                    || !token(&icon.flag_tag)
                {
                    return DecisionEvaluationExactView::unavailable(format!(
                        "XUI19 country {} custom-icon order/content rejected",
                        identity.tag
                    ));
                }
                prior_custom_icon_ordinal = Some(icon.source_ordinal);
            }
            custom_icon_count = match custom_icon_count.checked_add(category.custom_icons.len()) {
                Some(value) => value,
                None => {
                    return DecisionEvaluationExactView::unavailable(format!(
                        "XUI19 country {} custom-icon count overflowed",
                        identity.tag
                    ))
                }
            };
            let mut prior_decision_priority = None;
            for (card_ordinal, card) in category.cards.iter().enumerate() {
                if !card_valid(card, card_ordinal, category, country.definition_count)
                    || !card_identities.insert(card.source_record_id.as_str())
                    || prior_decision_priority
                        .is_some_and(|previous| card.decision_priority > previous)
                {
                    return DecisionEvaluationExactView::unavailable(format!(
                        "XUI19 country {} decision order/content rejected",
                        identity.tag
                    ));
                }
                prior_decision_priority = Some(card.decision_priority);
                card_count = match card_count.checked_add(1) {
                    Some(value) if value <= MAX_CARDS_PER_COUNTRY => value,
                    _ => {
                        return DecisionEvaluationExactView::unavailable(format!(
                            "XUI19 country {} visible-card bound exceeded",
                            identity.tag
                        ))
                    }
                };
            }
        }
        if country.category_count as usize != country.categories.len()
            || country.custom_icon_count as usize != custom_icon_count
            || card_count != country.visible_card_count as usize
        {
            return DecisionEvaluationExactView::unavailable(format!(
                "XUI19 country {} category, custom-icon, or visible-card count disagrees with rows",
                identity.tag
            ));
        }
    }

    DecisionEvaluationExactView {
        status: ViewStatus::Present,
        carrier: Some(carrier),
        diagnostic: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{D18CountryRosterEntry, D18CountryRosterView};
    use serde_json::json;

    fn roster() -> D18CountryRosterView {
        D18CountryRosterView {
            status: ViewStatus::Present,
            entries: vec![D18CountryRosterEntry {
                tag: "GER".into(),
                tag_db_id: 7,
            }],
            diagnostic: None,
        }
    }

    fn carrier() -> DecisionEvaluationExactV1 {
        let mut card = DecisionCardExactV1 {
            source_ordinal: 0,
            source_record_id: "card-0".into(),
            source_record_sha256: "1".repeat(64),
            decision_id: "test_decision".into(),
            category_id: "test_category".into(),
            database_ordinal: 0,
            category_priority: 10,
            decision_priority: 3,
            visible: true,
            availability: DecisionCardAvailabilityExact::Available,
            interface_state: 1,
            target: None,
            cost: DecisionCostExactV1 {
                value_raw: 50_000,
                scale: 1_000,
                resource_kind: "political_power".into(),
                custom_cost_available: None,
                formatted_text: "50".into(),
            },
            timer: None,
            alert_to_take: true,
            alert_timeout: false,
            name_text: "Test decision".into(),
            description_text: "Description".into(),
            tooltip_text: "Tooltip".into(),
        };
        card.source_record_sha256 = card_record_sha256(&card).expect("card hash");
        let mut country = DecisionCountryExactV1 {
            source_ordinal: 0,
            country_tag: "GER".into(),
            tag_db_id: 7,
            definition_count: 1,
            category_count: 1,
            custom_icon_count: 0,
            visible_card_count: 1,
            complete: true,
            source_record_id: "country-0".into(),
            source_record_sha256: String::new(),
            categories: vec![DecisionCategoryExactV1 {
                source_ordinal: 0,
                source_record_id: "category-0".into(),
                category_id: "test_category".into(),
                category_priority: 10,
                name_text: "Test category".into(),
                description_text: "Category description".into(),
                custom_icon_definition_count: 0,
                custom_icons: Vec::new(),
                cards: vec![card],
            }],
        };
        country.source_record_sha256 = country_record_sha256(&country).expect("country hash");
        let mut carrier = DecisionEvaluationExactV1 {
            schema: XUI19_SCHEMA.into(),
            authority: XUI19_AUTHORITY.into(),
            executable_sha256: XUI19_EXECUTABLE_SHA256.into(),
            evidence_manifest_sha256: XUI19_EVIDENCE_MANIFEST_SHA256.into(),
            producer_contract_sha256: XUI19_PRODUCER_CONTRACT_SHA256.into(),
            content_manifest_sha256: "3".repeat(64),
            decision_catalog_sha256: "4".repeat(64),
            localisation_language: "l_english".into(),
            frame_hour: 900,
            source_sequence: 10,
            batch_id_sha256: "5".repeat(64),
            expected_country_count: 1,
            d18_roster_sha256: d18_roster_sha256(&roster()).expect("roster hash"),
            passive_all_country: true,
            ui_open_independent: true,
            recorder_pov_independent: true,
            producer_complete: true,
            previous_record_sha256: None,
            record_sha256: String::new(),
            countries: vec![country],
        };
        carrier.record_sha256 = carrier_record_sha256(&carrier).expect("carrier hash");
        carrier
    }

    fn raw(carrier: &DecisionEvaluationExactV1) -> Map<String, Value> {
        json!({XUI19_CARRIER_KEY: carrier})
            .as_object()
            .expect("object")
            .clone()
    }

    #[test]
    fn admits_exact_census_and_preserves_measured_empty() {
        let admitted = admit_decision_evaluation(&raw(&carrier()), 900, &roster());
        assert_eq!(admitted.status, ViewStatus::Present);
        assert_eq!(admitted.country("GER").unwrap().visible_card_count, 1);

        let mut empty = carrier();
        empty.countries[0].definition_count = 0;
        empty.countries[0].category_count = 0;
        empty.countries[0].visible_card_count = 0;
        empty.countries[0].categories.clear();
        empty.countries[0].source_record_sha256 =
            country_record_sha256(&empty.countries[0]).unwrap();
        empty.record_sha256 = carrier_record_sha256(&empty).unwrap();
        let admitted = admit_decision_evaluation(&raw(&empty), 900, &roster());
        assert_eq!(admitted.status, ViewStatus::Present);
        assert!(admitted.country("GER").unwrap().categories.is_empty());
    }

    #[test]
    fn rejects_wrong_hour_partial_roster_and_hash_forgery() {
        assert_eq!(
            admit_decision_evaluation(&raw(&carrier()), 901, &roster()).status,
            ViewStatus::DomainUnavailable
        );
        let mut partial = roster();
        partial.entries.push(D18CountryRosterEntry {
            tag: "FRA".into(),
            tag_db_id: 8,
        });
        assert_eq!(
            admit_decision_evaluation(&raw(&carrier()), 900, &partial).status,
            ViewStatus::DomainUnavailable
        );
        let mut forged = carrier();
        forged.countries[0].categories[0].cards[0].name_text = "forged".into();
        assert_eq!(
            admit_decision_evaluation(&raw(&forged), 900, &roster()).status,
            ViewStatus::DomainUnavailable
        );

        let mut wrong_census_hash = carrier();
        wrong_census_hash.d18_roster_sha256 = "9".repeat(64);
        wrong_census_hash.record_sha256 = carrier_record_sha256(&wrong_census_hash).unwrap();
        assert_eq!(
            admit_decision_evaluation(&raw(&wrong_census_hash), 900, &roster()).status,
            ViewStatus::DomainUnavailable
        );

        let mut wrong_counts = carrier();
        wrong_counts.countries[0].category_count = 0;
        wrong_counts.countries[0].source_record_sha256 =
            country_record_sha256(&wrong_counts.countries[0]).unwrap();
        wrong_counts.record_sha256 = carrier_record_sha256(&wrong_counts).unwrap();
        assert_eq!(
            admit_decision_evaluation(&raw(&wrong_counts), 900, &roster()).status,
            ViewStatus::DomainUnavailable
        );
    }

    #[test]
    fn rejects_interface_state_availability_contradictions() {
        for state in [1, 6, 9] {
            assert!(interface_state_matches_availability(
                state,
                DecisionCardAvailabilityExact::Available,
                false,
            ));
            assert!(!interface_state_matches_availability(
                state,
                DecisionCardAvailabilityExact::Active,
                false,
            ));
        }
        for state in [3, 4, 10, 11] {
            assert!(interface_state_matches_availability(
                state,
                DecisionCardAvailabilityExact::Active,
                false,
            ));
        }
        for state in [0, 2, 5, 7, 8, 12] {
            assert!(interface_state_matches_availability(
                state,
                DecisionCardAvailabilityExact::Unavailable,
                false,
            ));
        }
        assert!(!interface_state_matches_availability(
            13,
            DecisionCardAvailabilityExact::Unavailable,
            false,
        ));
        assert!(interface_state_matches_availability(
            1,
            DecisionCardAvailabilityExact::OnCooldown,
            true,
        ));

        let mut contradictory = carrier();
        contradictory.countries[0].categories[0].cards[0].availability =
            DecisionCardAvailabilityExact::Active;
        contradictory.countries[0].categories[0].cards[0].source_record_sha256 =
            card_record_sha256(&contradictory.countries[0].categories[0].cards[0]).unwrap();
        contradictory.countries[0].source_record_sha256 =
            country_record_sha256(&contradictory.countries[0]).unwrap();
        contradictory.record_sha256 = carrier_record_sha256(&contradictory).unwrap();
        assert_eq!(
            admit_decision_evaluation(&raw(&contradictory), 900, &roster()).status,
            ViewStatus::DomainUnavailable
        );
    }
}
