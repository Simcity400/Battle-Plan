
use crate::world::ViewStatus;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub const STATUS_PRESENT_ABSENT_REASON: &str =
    "d16 decisions_live status_present was not carried for this country";
pub const TIMED_LIVE_UNPORTED_REASON: &str = "d16 decisions_live timed_live was not carried for \
     this country; the compact wire port deliberately stops that collection \
     (wire_format/src/d16.rs:11, :203, schema test :750), which is the expected cause";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionsLiveLeaf<T> {
    Absent { reason: String },
    Unavailable { reason: String },
    Measured(T),
}

impl<T> DecisionsLiveLeaf<T> {
    pub fn measured(&self) -> Option<&T> {
        match self {
            Self::Measured(value) => Some(value),
            Self::Absent { .. } | Self::Unavailable { .. } => None,
        }
    }

    pub fn unavailable_reason(&self) -> Option<&str> {
        match self {
            Self::Absent { reason } | Self::Unavailable { reason } => Some(reason),
            Self::Measured(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecisionStatusCounters {
    pub active_timed: u32,
    pub active_targeted: u32,
    pub active_targeted_timed: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionTimedLiveRow {
    pub decision_id: String,
    pub days_raw: i32,
    pub state_raw: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountryDecisionsLive {
    pub status_present: DecisionsLiveLeaf<DecisionStatusCounters>,
    pub timed_live: DecisionsLiveLeaf<Vec<DecisionTimedLiveRow>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionsLiveView {
    pub status: ViewStatus,
    pub countries: BTreeMap<String, CountryDecisionsLive>,
}

impl DecisionsLiveView {
    fn absent() -> Self {
        Self {
            status: ViewStatus::DomainAbsent,
            countries: BTreeMap::new(),
        }
    }

    fn unavailable() -> Self {
        Self {
            status: ViewStatus::DomainUnavailable,
            countries: BTreeMap::new(),
        }
    }

    pub(crate) fn mark_unavailable(&mut self) {
        if self.status == ViewStatus::Present {
            *self = Self::unavailable();
        }
    }

    pub fn country(&self, tag: &str) -> Option<&CountryDecisionsLive> {
        self.countries.get(tag)
    }
}

fn parse_counter(object: &Map<String, Value>, key: &str, path: &str) -> Result<u32, String> {
    let value = object
        .get(key)
        .ok_or_else(|| format!("{path}.{key} is absent"))?;
    let raw = value
        .as_u64()
        .ok_or_else(|| format!("{path}.{key} must be a non-negative integer"))?;
    u32::try_from(raw).map_err(|_| format!("{path}.{key} does not fit u32"))
}

fn parse_status_present(value: &Value, path: &str) -> Result<DecisionStatusCounters, String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{path} must be an object"))?;
    Ok(DecisionStatusCounters {
        active_timed: parse_counter(object, "active_timed", path)?,
        active_targeted: parse_counter(object, "active_targeted", path)?,
        active_targeted_timed: parse_counter(object, "active_targeted_timed", path)?,
    })
}

fn parse_days_raw(value: &Value, path: &str) -> Result<i32, String> {
    let raw = value
        .as_i64()
        .ok_or_else(|| format!("{path} must be an integer"))?;
    i32::try_from(raw).map_err(|_| format!("{path} does not fit i32"))
}

fn parse_i32(value: &Value, path: &str) -> Result<i32, String> {
    let raw = value
        .as_i64()
        .ok_or_else(|| format!("{path} must be an integer"))?;
    i32::try_from(raw).map_err(|_| format!("{path} does not fit i32"))
}

fn parse_timed_live(value: &Value, path: &str) -> Result<Vec<DecisionTimedLiveRow>, String> {
    let rows = value
        .as_array()
        .ok_or_else(|| format!("{path} must be an array"))?;
    rows.iter()
        .enumerate()
        .map(|(index, value)| {
            let row_path = format!("{path}[{index}]");
            let object = value
                .as_object()
                .ok_or_else(|| format!("{row_path} must be an object"))?;
            let decision_id = object
                .get("decision")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| format!("{row_path}.decision must be a non-blank string"))?
                .to_string();
            let days_raw = parse_days_raw(
                object
                    .get("days")
                    .ok_or_else(|| format!("{row_path}.days is absent"))?,
                &format!("{row_path}.days"),
            )?;
            let state_raw = parse_i32(
                object
                    .get("state")
                    .ok_or_else(|| format!("{row_path}.state is absent"))?,
                &format!("{row_path}.state"),
            )?;
            Ok(DecisionTimedLiveRow {
                decision_id,
                days_raw,
                state_raw,
            })
        })
        .collect()
}

pub(crate) fn build_decisions_live(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> DecisionsLiveView {
    let result = (|| -> Result<DecisionsLiveView, String> {
        let mut view = DecisionsLiveView::absent();
        let Some(countries) = rest.get("countries") else {
            return Ok(view);
        };
        let countries = countries
            .as_object()
            .ok_or_else(|| "countries: expected object for d16 decisions_live".to_string())?;
        let mut carried_any_d16_leaf = false;

        for (tag, value) in countries {
            let Some(country) = value.as_object() else {
                continue;
            };
            let decisions = match country.get("decisions") {
                None => None,
                Some(value) => Some(
                    value
                        .as_object()
                        .ok_or_else(|| format!("countries.{tag}.decisions must be an object"))?,
                ),
            };

            let status_present = match decisions.and_then(|value| value.get("status_present")) {
                None => DecisionsLiveLeaf::Absent {
                    reason: STATUS_PRESENT_ABSENT_REASON.to_string(),
                },
                Some(Value::Null) => {
                    carried_any_d16_leaf = true;
                    DecisionsLiveLeaf::Absent {
                        reason: format!(
                            "countries.{tag}.decisions.status_present was carried as null, which \
                             is not the measured counter triple, not measured zero, and not a \
                             malformed carried object"
                        ),
                    }
                }
                Some(value) => {
                    carried_any_d16_leaf = true;
                    match parse_status_present(
                        value,
                        &format!("countries.{tag}.decisions.status_present"),
                    ) {
                        Ok(counters) => DecisionsLiveLeaf::Measured(counters),
                        Err(reason) => DecisionsLiveLeaf::Unavailable { reason },
                    }
                }
            };

            let timed_live = match decisions.and_then(|value| value.get("timed_live")) {
                None if decisions.is_none() => DecisionsLiveLeaf::Absent {
                    reason: format!(
                        "countries.{tag} carries no decisions object at all, so no d16 leaf was \
                         written for it; this is not the stopped timed_live collection"
                    ),
                },
                None => DecisionsLiveLeaf::Absent {
                    reason: TIMED_LIVE_UNPORTED_REASON.to_string(),
                },
                Some(Value::Null) => {
                    carried_any_d16_leaf = true;
                    DecisionsLiveLeaf::Absent {
                        reason: format!(
                            "countries.{tag}.decisions.timed_live was carried as null, which is \
                             not an empty list and not a measurement"
                        ),
                    }
                }
                Some(value) => {
                    carried_any_d16_leaf = true;
                    match parse_timed_live(value, &format!("countries.{tag}.decisions.timed_live"))
                    {
                        Ok(rows) => DecisionsLiveLeaf::Measured(rows),
                        Err(reason) => DecisionsLiveLeaf::Unavailable { reason },
                    }
                }
            };

            view.countries.insert(
                tag.clone(),
                CountryDecisionsLive {
                    status_present,
                    timed_live,
                },
            );
        }

        if carried_any_d16_leaf {
            view.status = ViewStatus::Present;
        } else {
            view.countries.clear();
        }
        Ok(view)
    })();

    match result {
        Ok(view) => view,
        Err(reason) => {
            warnings.push(format!(
                "d16 decisions_live: {reason}; entire counter view unavailable"
            ));
            DecisionsLiveView::unavailable()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn build(value: Value) -> (DecisionsLiveView, Vec<String>) {
        let mut warnings = Vec::new();
        let view =
            build_decisions_live(value.as_object().expect("test root object"), &mut warnings);
        (view, warnings)
    }

    #[test]
    fn a_domain_absent_view_hands_out_no_country_leaves() {
        let (view, warnings) = build(json!({"countries": {"GER": {"decisions": {}}}}));
        assert_eq!(view.status, ViewStatus::DomainAbsent);
        assert!(warnings.is_empty());
        assert!(view.countries.is_empty());
        assert!(view.country("GER").is_none());
    }

    #[test]
    fn per_country_absence_is_explicit_once_the_domain_is_present() {
        let (view, warnings) = build(json!({"countries": {
            "GER": {"decisions": {"status_present": {
                "active_timed": 1,
                "active_targeted": 0,
                "active_targeted_timed": 0
            }}},
            "FRA": {"decisions": {}}
        }}));
        assert_eq!(view.status, ViewStatus::Present);
        assert!(warnings.is_empty());
        let france = view.country("FRA").expect("country leaf");
        assert!(matches!(
            france.status_present,
            DecisionsLiveLeaf::Absent { .. }
        ));
        assert!(france.status_present.measured().is_none());
    }

    #[test]
    fn a_malformed_or_partial_status_present_rejects_only_that_country() {
        for status_present in [
            json!({"active_timed": 1, "active_targeted": 2}),
            json!({
                "active_timed": 1,
                "active_targeted": -1,
                "active_targeted_timed": 2
            }),
            json!({"political_action": 3, "market_action": 1}),
            json!("not an object"),
        ] {
            let (view, warnings) = build(json!({"countries": {
                "GER": {"decisions": {"status_present": status_present}},
                "FRA": {"decisions": {"status_present": {
                    "active_timed": 4,
                    "active_targeted": 5,
                    "active_targeted_timed": 6
                }}}
            }}));
            assert_eq!(view.status, ViewStatus::Present);
            assert!(
                matches!(
                    view.country("GER").expect("GER leaf").status_present,
                    DecisionsLiveLeaf::Unavailable { .. }
                ),
                "the malformed country is rejected"
            );
            assert_eq!(
                view.country("FRA")
                    .expect("FRA leaf")
                    .status_present
                    .measured()
                    .map(|counters| counters.active_timed),
                Some(4),
                "a sibling country keeps its measurement"
            );
            assert!(warnings.is_empty());
        }
    }

    #[test]
    fn a_domain_problem_does_not_promote_absence_into_rejection() {
        let mut absent = build(json!({"countries": {"GER": {"decisions": {}}}})).0;
        assert_eq!(absent.status, ViewStatus::DomainAbsent);
        absent.mark_unavailable();
        assert_eq!(
            absent.status,
            ViewStatus::DomainAbsent,
            "an absent domain stays absent"
        );

        let mut present = build(json!({"countries": {"GER": {"decisions": {"status_present": {
            "active_timed": 1, "active_targeted": 0, "active_targeted_timed": 0
        }}}}}))
        .0;
        assert_eq!(present.status, ViewStatus::Present);
        present.mark_unavailable();
        assert_eq!(present.status, ViewStatus::DomainUnavailable);
        assert!(present.countries.is_empty());
    }

    #[test]
    fn carried_as_null_is_unmeasured_and_neither_empty_nor_malformed() {
        let (view, warnings) = build(json!({"countries": {
            "ENG": {"decisions": {"status_present": null, "timed_live": null}}
        }}));
        assert_ne!(view.status, ViewStatus::DomainUnavailable);
        assert!(warnings.is_empty(), "null is not a coverage warning");
        let country = view.country("ENG").expect("ENG leaf");
        for leaf_reason in [
            country.status_present.unavailable_reason(),
            country.timed_live.unavailable_reason(),
        ] {
            assert!(leaf_reason.expect("null carries a reason").contains("null"));
        }
        assert!(country.status_present.measured().is_none());
        assert!(country.timed_live.measured().is_none());
    }

    #[test]
    fn all_zero_counters_are_an_admitted_measurement() {
        let (view, warnings) = build(json!({
            "countries": {"GER": {"decisions": {"status_present": {
                "active_timed": 0,
                "active_targeted": 0,
                "active_targeted_timed": 0
            }}}}
        }));
        assert_eq!(view.status, ViewStatus::Present);
        assert!(warnings.is_empty());
        assert_eq!(
            view.country("GER")
                .and_then(|country| country.status_present.measured()),
            Some(&DecisionStatusCounters {
                active_timed: 0,
                active_targeted: 0,
                active_targeted_timed: 0,
            })
        );
    }

    #[test]
    fn absent_timed_live_states_the_observation_and_cites_its_expected_cause() {
        let (view, warnings) = build(json!({
            "countries": {"GER": {"decisions": {"status_present": {
                "active_timed": 1,
                "active_targeted": 2,
                "active_targeted_timed": 3
            }}}}
        }));
        assert!(warnings.is_empty());
        let timed_live = &view.country("GER").expect("country leaf").timed_live;
        assert!(matches!(timed_live, DecisionsLiveLeaf::Absent { .. }));
        let reason = timed_live.unavailable_reason().expect("absence reason");
        assert!(reason.contains("was not carried"), "state the observation");
        assert!(
            reason.contains("wire_format/src/d16.rs:11"),
            "cite the stopped collection rather than asserting a bare cause"
        );
        assert!(
            reason.contains("expected cause"),
            "name the cause as an expectation, not as a proven fact"
        );
        assert!(timed_live.measured().is_none(), "never measured-empty");
    }

    #[test]
    fn a_negative_days_raw_is_carried_rather_than_refused_on_an_unproven_bound() {
        let (view, warnings) = build(json!({
            "countries": {"GER": {"decisions": {
                "status_present": {
                    "active_timed": 1,
                    "active_targeted": 0,
                    "active_targeted_timed": 0
                },
                "timed_live": [
                    {"decision": "GER_x", "days": -1, "state": 0},
                    {"decision": "GER_y", "days": 5, "state": 0}
                ]
            }}}
        }));
        assert_eq!(view.status, ViewStatus::Present);
        let rows = view
            .country("GER")
            .expect("country leaf")
            .timed_live
            .measured()
            .expect("a negative days_raw does not reject the vector");
        assert_eq!(rows.len(), 2, "the sibling row survives");
        assert_eq!(rows[0].days_raw, -1, "carried verbatim, sign and all");
        assert_eq!(rows[1].days_raw, 5);
        assert!(warnings.is_empty());
    }

    #[test]
    fn a_non_integer_days_is_still_rejected() {
        let (view, _) = build(json!({
            "countries": {"GER": {"decisions": {
                "status_present": {
                    "active_timed": 1,
                    "active_targeted": 0,
                    "active_targeted_timed": 0
                },
                "timed_live": [{"decision": "GER_x", "days": "soon", "state": 0}]
            }}}
        }));
        let reason = view
            .country("GER")
            .expect("country leaf")
            .timed_live
            .unavailable_reason()
            .expect("rejection reason")
            .to_string();
        assert!(reason.contains("must be an integer"));
        assert!(
            reason.contains("timed_live[0].days"),
            "name the row and field that failed"
        );
    }

    #[test]
    fn present_empty_timed_live_is_measured_empty() {
        let (view, warnings) = build(json!({
            "countries": {"GER": {"decisions": {
                "status_present": {
                    "active_timed": 0,
                    "active_targeted": 0,
                    "active_targeted_timed": 0
                },
                "timed_live": []
            }}}
        }));
        assert!(warnings.is_empty());
        assert!(matches!(
            view.country("GER").expect("country leaf").timed_live,
            DecisionsLiveLeaf::Measured(ref rows) if rows.is_empty()
        ));
    }

    #[test]
    fn days_raw_is_carried_without_unit_conversion() {
        let (view, warnings) = build(json!({
            "countries": {"GER": {"decisions": {
                "status_present": {
                    "active_timed": 1,
                    "active_targeted": 0,
                    "active_targeted_timed": 0
                },
                "timed_live": [{
                    "decision": "GER_raw_timer",
                    "days": 17,
                    "state": 0
                }]
            }}}
        }));
        assert!(warnings.is_empty());
        let rows = view
            .country("GER")
            .and_then(|country| country.timed_live.measured())
            .expect("measured timed_live");
        assert_eq!(rows[0].days_raw, 17);
        assert_eq!(rows[0].state_raw, 0);
    }
}
