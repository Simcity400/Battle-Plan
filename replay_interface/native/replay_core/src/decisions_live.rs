//! Fail-closed replay view for d16 `decisions_live`.
//!
//! The three `status_present` counters are class-A authoritative country state.
//! They are admitted together or rejected together WITHIN a country, and a
//! rejection stays local to that country rather than blinding its siblings;
//! zero is a measured value.
//! The `timed_live` occurrence list is a separate class-B leaf because the
//! compact wire port deliberately stops that collection, so its absence is
//! never a measured-empty list.
//!
//! # What this view refuses to say
//!
//! It reports what was and was not carried. It does not report WHY, because it
//! cannot know: the expected cause of a missing `timed_live` is the stopped wire
//! collection, but a fold that did not cover a country produces the same
//! absence, and a recording taken through the JSON path carries the rows. The
//! reasons below therefore state the observation and name the expected cause as
//! an expectation, with its citation, rather than asserting it as fact.

use crate::world::ViewStatus;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub const STATUS_PRESENT_ABSENT_REASON: &str =
    "d16 decisions_live status_present was not carried for this country";
/// Observation first, expected cause second, and labelled as an expectation.
///
/// The compact port stops the `timed_live` collection deliberately —
/// `extractor/native/wire_format/src/d16.rs:11` calls d16 a split deliverable,
/// `:203` records that no collection id is claimed for it, and the schema test
/// at `:750` asserts `timed_live is stopped, not ported` — so a production
/// recording carries the counters without the rows. That is the expected cause
/// of this absence, not a proven one.
pub const TIMED_LIVE_UNPORTED_REASON: &str = "d16 decisions_live timed_live was not carried for \
     this country; the compact wire port deliberately stops that collection \
     (wire_format/src/d16.rs:11, :203, schema test :750), which is the expected cause";

/// One leaf of the split d16 view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionsLiveLeaf<T> {
    /// The field was not carried, in any of the several ways that can happen —
    /// key missing, `decisions` object missing, or the value carried as `null`.
    /// The reason says WHICH. It never holds a measured-empty collection: that
    /// is [`Self::Measured`] with an empty value, and the two must not be read
    /// as the same thing.
    Absent { reason: String },
    /// The field was carried but could not be admitted.
    Unavailable { reason: String },
    /// An admitted measurement. `Measured(Vec::new())` and measured zero
    /// counters are real observations.
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

/// The complete all-or-nothing d16 `status_present` measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecisionStatusCounters {
    pub active_timed: u32,
    pub active_targeted: u32,
    pub active_targeted_timed: u32,
}

/// One row from the stopped d16 `timed_live` occurrence list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionTimedLiveRow {
    pub decision_id: String,
    /// Raw signed integer carried without conversion. The producer/fold
    /// contract stores an `i32`, so this view enforces representability only.
    /// It does not assign a nonnegative bound, UI-countdown meaning, or time
    /// unit.
    pub days_raw: i32,
    /// Raw engine state word. Zero is the only named active value; this view
    /// does not invent a lifecycle enum for any other value.
    pub state_raw: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountryDecisionsLive {
    pub status_present: DecisionsLiveLeaf<DecisionStatusCounters>,
    pub timed_live: DecisionsLiveLeaf<Vec<DecisionTimedLiveRow>>,
}

/// The d16 view.
///
/// `countries` is non-empty only at [`ViewStatus::Present`]. Both non-present
/// statuses carry an empty map, which is the same contract the sibling
/// per-country builders hold (see `build_country_army_upgrade_exact` in
/// `world.rs`) and the one `mark_unavailable` enforces by replacing the whole
/// view. The alternative — a `DomainAbsent` view that still answers
/// [`Self::country`] — would let a consumer that checks the map without checking
/// the status read per-country leaves out of a domain the recording never
/// carried.
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

    /// Downgrade a PRESENT view to unavailable on a reported domain problem.
    ///
    /// The guard matters and is not decoration. A domain-problem row for domain
    /// 16 can arrive on a frame whose recording never carried d16 at all; without
    /// the guard, `DomainAbsent` would be rewritten to `DomainUnavailable`, and a
    /// consumer would be told the domain was observed and rejected when in truth
    /// it was never carried. That is exactly the absent/unavailable collapse this
    /// module exists to prevent. This mirrors the guarded `downgrade_present_status`
    /// used by the sibling scalar views (`world.rs:15690`).
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

/// Parse the raw timed-decision integer without narrowing the producer's `i32`
/// carrier. Adjacent status counters use unsigned carriers, but this field is
/// decoded as `i32`; rejecting negative values would invent a bound and could
/// discard the rest of the country-atomic vector. The full `i32` range is
/// carried through without UI-countdown or time-unit interpretation.
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
                // `null` is treated as UNMEASURED rather than as malformed, but
                // NOT on the authority of the fold's null_if_unmeasured_collection
                // helper (`recorder_host/src/world_fold.rs:25563`) — an earlier
                // version of this comment cited that, and the citation was wrong
                // twice over. That helper only nulls a value that is ALREADY
                // empty, and the d16 fold never routes these keys through it: it
                // writes status_present as a populated three-key object and
                // timed_live as `json!([])` (`world_fold.rs:13816`-`:13824`).
                //
                // The real justification is narrower and is about what null can
                // possibly mean rather than about who wrote it. No producer in
                // evidence emits null here at all except the shipped fixture
                // corpus (`replay_fixture_gen/src/main.rs:2125`). Whatever wrote
                // it, `null` is certainly not the measured triple, and reading it
                // as an empty or zero measurement would manufacture a fact. So it
                // is carried as its own absence, distinct from a missing key and
                // from a rejection, and it says so.
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
                    // Per-country locality is deliberate. A single country whose
                    // counters do not parse must not blind every other country:
                    // the shipped fixture corpus already carries a foreign
                    // `status_present` shape, and escalating that to the whole
                    // view discards good measurements for every other tag.
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
                // Two different observations that an earlier version gave the
                // same words. A country that carries a `decisions` object
                // without a `timed_live` key is the case the stopped wire
                // collection explains. A country with no `decisions` object at
                // all was not covered by the fold, and naming the wire port
                // there would point the reader at the wrong cause entirely.
                None if decisions.is_none() => DecisionsLiveLeaf::Absent {
                    reason: format!(
                        "countries.{tag} carries no decisions object at all, so no d16 leaf was \
                         written for it; this is not the stopped timed_live collection"
                    ),
                },
                None => DecisionsLiveLeaf::Absent {
                    reason: TIMED_LIVE_UNPORTED_REASON.to_string(),
                },
                // Same reasoning as status_present above: null cannot be the
                // measured list, so it is carried as its own absence and stays
                // separable from "the key was never written". No claim is made
                // about which producer wrote it.
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
            // No country carried any d16 leaf, so the domain is absent for this
            // frame and the per-country leaves collected above describe nothing
            // that was observed. Dropping them keeps `country()` from answering
            // out of a domain-absent view; see the type's own doc comment.
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
    /// The contract asserted here is deliberate, not incidental: a view that is
    /// not `Present` carries NO per-country leaves, so `country()` cannot answer
    /// out of a domain the recording never carried.
    fn a_domain_absent_view_hands_out_no_country_leaves() {
        let (view, warnings) = build(json!({"countries": {"GER": {"decisions": {}}}}));
        assert_eq!(view.status, ViewStatus::DomainAbsent);
        assert!(warnings.is_empty());
        assert!(view.countries.is_empty());
        assert!(view.country("GER").is_none());
    }

    /// Per-country absence is only expressible once the domain itself is
    /// `Present`. One country carrying counters makes the domain present, and a
    /// country that carries nothing then says so explicitly rather than
    /// vanishing.
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

    /// The three counters are all-or-nothing WITHIN a country: a partial or
    /// out-of-range triple never admits the fields that happened to parse.
    ///
    /// The rejection is also LOCAL to that country. The shipped fixture corpus
    /// carries a foreign `status_present` shape (`replay_fixture_gen/src/main.rs:850`
    /// writes `political_action`/`market_action` under the same key), so a global
    /// escalation would discard every other country's good measurement on account
    /// of one country's — which is data loss on input the corpus really produces.
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
            // A per-country leaf rejection is carried IN the leaf, which is the
            // typed diagnosable refusal. It is not also a view-level coverage
            // warning: `roundtrip_gates::well_formed_fixtures_emit_no_coverage_warnings`
            // treats those as corpus defects, and one country's shape is not one.
            assert!(warnings.is_empty());
        }
    }

    /// A domain-problem row must not rewrite "never carried" into "observed and
    /// rejected". Only a PRESENT view downgrades.
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

    /// `null` is neither an empty measurement nor a malformed one, and it must
    /// not take the view down. The shipped base fixture writes exactly this
    /// (`replay_fixture_gen/src/main.rs:2125`), which is the only producer of
    /// the shape in evidence — the d16 fold itself writes a populated object and
    /// `json!([])` (`world_fold.rs:13816`-`:13824`).
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

    /// A missing `timed_live` must never read as "this country has no active
    /// timed decisions", and its reason must state the OBSERVATION with the
    /// stopped wire collection named as the expected cause and cited — not
    /// asserted as the proven one, which this code cannot know.
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

    /// A negative `days_raw` is CARRIED, not refused.
    ///
    /// The producer reads this field as `i32`, so a real recording can carry a
    /// negative. Since the vector is all-or-nothing per country, inventing a
    /// nonnegative bound would discard every sibling row alongside it, which
    /// this test also pins.
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

    /// A non-integer `days` is still a real rejection — the field must be an
    /// integer even though its magnitude is unbounded.
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
