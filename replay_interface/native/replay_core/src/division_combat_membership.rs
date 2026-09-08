//! Current D20 first-combat country census and membership of its paired raw country operand.
use crate::country_identity::CountryHandleEqualityRefusal;
use crate::division_training_inputs::DivisionTrainingRefusal;
use crate::{Leaf, ViewStatus, WorldFrame};
pub use portable_compact_stream::division_routes::{
    CombatClass, CombatIdentity, CombatMembershipCause, CombatMembershipField,
    CombatMembershipRefusal, CombatMembershipStage, CountryWordsObservation,
    FirstCombatCountriesObservation,
};
use portable_compact_stream::division_routes::{TrainingObservation, TrainingRefusal};
use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DivisionCombatMembershipRefusal {
    Missing,
    Unobtained,
    Malformed(String),
    MalformedLedger,
    DomainProblem(String),
    CurrentIdentityUnavailable,
    Unavailable(CombatMembershipRefusal),
    TrainingScalars(DivisionTrainingRefusal),
    CountryOperandUnavailable(TrainingRefusal),
    CountryMapping(CountryHandleEqualityRefusal),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FirstCombatMembershipResult {
    NoFirstCombat,
    Present {
        combat: CombatIdentity,
        is_member: bool,
    },
}

type Refusal = DivisionCombatMembershipRefusal;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CombatJson {
    class: String,
    #[serde(rename = "type")]
    kind: u32,
    id: u32,
}

impl CombatJson {
    fn into_identity(self) -> Result<CombatIdentity, String> {
        let class = match self.class.as_str() {
            "land" => CombatClass::Land,
            "border_war" => CombatClass::BorderWar,
            _ => return Err("unknown combat class".into()),
        };
        Ok(CombatIdentity {
            class,
            kind: self.kind,
            id: self.id,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RefusalJson {
    cause: String,
    field: String,
    stage: String,
    // Value makes these nullable fields required: omitted ordinals are malformed.
    ordinal: Value,
    original_ordinal: Value,
}

impl RefusalJson {
    fn into_refusal(self) -> Result<CombatMembershipRefusal, String> {
        use CombatMembershipCause as C;
        use CombatMembershipField as F;
        use CombatMembershipStage as S;
        let cause = match self.cause.as_str() {
            "not_observed" => C::NotObserved,
            "unreadable" => C::Unreadable,
            "changed" => C::Changed,
            "invalid_identity" => C::InvalidIdentity,
            "null_side" => C::NullSide,
            "cycle" => C::Cycle,
            "verification_budget_exceeded" => C::VerificationBudgetExceeded,
            "retained_budget_exceeded" => C::RetainedBudgetExceeded,
            "allocation_failed" => C::AllocationFailed,
            "address_overflow" => C::AddressOverflow,
            "invalid_address" => C::InvalidAddress,
            "root_changed" => C::RootChanged,
            "hour_changed" => C::HourChanged,
            "hour_unreadable" => C::HourUnreadable,
            _ => return Err("unknown combat membership cause".into()),
        };
        let field = match self.field.as_str() {
            "frame_root" => F::FrameRoot,
            "hour" => F::Hour,
            "division_identity" => F::DivisionIdentity,
            "first_combat_vector" => F::FirstCombatVector,
            "first_combat_pointer" => F::FirstCombatPointer,
            "combat_identity" => F::CombatIdentity,
            "side_pointer" => F::SidePointer,
            "side_class" => F::SideClass,
            "side_backlink" => F::SideBacklink,
            "head" => F::Head,
            "country" => F::Country,
            "next" => F::Next,
            "ordinary_route" => F::OrdinaryRoute,
            "alternate_route" => F::AlternateRoute,
            "route_head" => F::RouteHead,
            "numerator" => F::Numerator,
            "transfer" => F::Transfer,
            "target" => F::Target,
            "retreat" => F::Retreat,
            "division_country_operand" => F::DivisionCountryOperand,
            "attachment_identity" => F::AttachmentIdentity,
            "campaign_identity" => F::CampaignIdentity,
            "producer_binding" => F::ProducerBinding,
            "strength" => F::Strength,
            "organisation" => F::Organisation,
            _ => return Err("unknown combat membership field".into()),
        };
        let stage = match self.stage.as_str() {
            "admission" => S::Admission,
            "open" => S::Open,
            "close" => S::Close,
            _ => return Err("unknown combat membership stage".into()),
        };
        fn ordinal(value: Value) -> Result<Option<u32>, String> {
            if value.is_null() {
                Ok(None)
            } else {
                value
                    .as_u64()
                    .and_then(|v| u32::try_from(v).ok())
                    .map(Some)
                    .ok_or_else(|| "invalid combat membership ordinal".into())
            }
        }
        let refusal = CombatMembershipRefusal {
            cause,
            field,
            stage,
            ordinal: ordinal(self.ordinal)?,
            original_ordinal: ordinal(self.original_ordinal)?,
        };
        refusal.validate()?;
        Ok(refusal)
    }
}

#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum CountriesJson {
    Measured { raw_i32: Vec<i32> },
    Unavailable { refusal: RefusalJson },
}

#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum ObservationJson {
    NoFirstCombat {
        schema: u32,
    },
    Present {
        schema: u32,
        combat: CombatJson,
        countries: CountriesJson,
    },
    Unavailable {
        schema: u32,
        refusal: RefusalJson,
    },
}

fn parse_observation(value: &Value) -> Result<FirstCombatCountriesObservation, String> {
    let parsed = ObservationJson::deserialize(value).map_err(|e| e.to_string())?;
    let (schema, observation) = match parsed {
        ObservationJson::NoFirstCombat { schema } => {
            (schema, FirstCombatCountriesObservation::NoFirstCombat)
        }
        ObservationJson::Present {
            schema,
            combat,
            countries,
        } => {
            let countries = match countries {
                CountriesJson::Measured { raw_i32 } => CountryWordsObservation::Measured(raw_i32),
                CountriesJson::Unavailable { refusal } => {
                    CountryWordsObservation::Unavailable(refusal.into_refusal()?)
                }
            };
            (
                schema,
                FirstCombatCountriesObservation::Present {
                    combat: combat.into_identity()?,
                    countries,
                },
            )
        }
        ObservationJson::Unavailable { schema, refusal } => (
            schema,
            FirstCombatCountriesObservation::Unavailable(refusal.into_refusal()?),
        ),
    };
    if schema != 1 {
        return Err("first_combat_countries schema".into());
    }
    observation.validate()?;
    Ok(observation)
}

#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum ProjectionJson {
    Absent {},
    Measured {
        class: String,
        #[serde(rename = "type")]
        kind: u32,
        id: u32,
    },
}

fn parse_projection(value: &Value) -> Result<Option<Option<CombatIdentity>>, String> {
    if value.is_null() {
        return Ok(None);
    }
    match ProjectionJson::deserialize(value).map_err(|e| e.to_string())? {
        ProjectionJson::Absent {} => Ok(Some(None)),
        ProjectionJson::Measured { class, kind, id } => CombatJson { class, kind, id }
            .into_identity()
            .map(|v| Some(Some(v))),
    }
}

impl WorldFrame {
    /// Read the complete observation from exactly one current D20 identity. Historical
    /// D20 faults refuse even when retained raw rows contain a later successful sample.
    pub fn division_first_combat_countries(
        &self,
        id: u32,
        kind: u32,
    ) -> Result<FirstCombatCountriesObservation, Refusal> {
        match crate::world::rederive_collector_domain_problems(self.raw()) {
            Err(()) => return Err(Refusal::MalformedLedger),
            Ok(Some(problems)) => {
                for problem in problems {
                    if problem.domain == 20
                        && !crate::world::domain_problem_is_per_family_accounted(&problem)
                    {
                        return Err(Refusal::DomainProblem(problem.problem));
                    }
                }
            }
            Ok(None) => {}
        }
        if self.division_units_status() != ViewStatus::Present
            || self
                .division_units()
                .iter()
                .filter(|row| {
                    row.raw_division_id == id
                        && matches!(row.raw_division_type, Leaf::Measured(value) if value == kind)
                })
                .count()
                != 1
        {
            return Err(Refusal::CurrentIdentityUnavailable);
        }
        let rows = self
            .raw()
            .get("divisions")
            .and_then(Value::as_array)
            .ok_or(Refusal::CurrentIdentityUnavailable)?;
        let mut found = rows.iter().filter(|row| {
            row.get("div_id").and_then(Value::as_u64) == Some(u64::from(id))
                && row.get("div_type").and_then(Value::as_u64) == Some(u64::from(kind))
        });
        let row = found.next().ok_or(Refusal::CurrentIdentityUnavailable)?;
        if found.next().is_some() {
            return Err(Refusal::CurrentIdentityUnavailable);
        }
        let value = match row.get("first_combat_countries") {
            None => return Err(Refusal::Missing),
            Some(Value::Null) => return Err(Refusal::Unobtained),
            Some(value) => value,
        };
        let observation = parse_observation(value).map_err(Refusal::Malformed)?;
        let projection = row
            .get("first_combat")
            .ok_or_else(|| Refusal::Malformed("missing first_combat projection".into()))?;
        if parse_projection(projection).map_err(Refusal::Malformed)? != observation.projection() {
            return Err(Refusal::Malformed(
                "first_combat projection mismatch".into(),
            ));
        }
        Ok(observation)
    }

    /// Evaluate the paired training country operand only after validating the whole census.
    /// Raw equality may prove membership despite a refused mapping elsewhere in a complete list.
    pub fn division_first_combat_country_membership(
        &self,
        id: u32,
        kind: u32,
    ) -> Result<FirstCombatMembershipResult, Refusal> {
        let (combat, countries) = match self.division_first_combat_countries(id, kind)? {
            FirstCombatCountriesObservation::NoFirstCombat => {
                return Ok(FirstCombatMembershipResult::NoFirstCombat)
            }
            FirstCombatCountriesObservation::Unavailable(reason) => {
                return Err(Refusal::Unavailable(reason))
            }
            FirstCombatCountriesObservation::Present { combat, countries } => (combat, countries),
        };
        let countries = match countries {
            CountryWordsObservation::Measured(countries) => countries,
            CountryWordsObservation::Unavailable(reason) => {
                return Err(Refusal::Unavailable(reason))
            }
        };
        let operand = match self
            .division_training_scalars(id, kind)
            .map_err(Refusal::TrainingScalars)?
            .country_operand
        {
            TrainingObservation::Measured(value) => value,
            TrainingObservation::Unavailable(reason) => {
                return Err(Refusal::CountryOperandUnavailable(reason))
            }
        };
        let mut matched = false;
        let mut first_mapping_refusal = None;
        for country in countries {
            // Casts preserve every signed raw word's bits, including negative handles.
            match self.country_handles_equal(operand as u32, country as u32) {
                Ok(true) => matched = true,
                Ok(false) => {}
                Err(reason) => {
                    first_mapping_refusal.get_or_insert(reason);
                }
            }
        }
        if !matched {
            if let Some(reason) = first_mapping_refusal {
                return Err(Refusal::CountryMapping(reason));
            }
        }
        Ok(FirstCombatMembershipResult::Present {
            combat,
            is_member: matched,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_compact_stream::division_routes::TrainingScalars;
    use serde_json::json;

    const COMBAT: CombatIdentity = CombatIdentity {
        class: CombatClass::Land,
        kind: 62,
        id: 79,
    };

    fn present(words: Vec<i32>) -> FirstCombatCountriesObservation {
        FirstCombatCountriesObservation::Present {
            combat: COMBAT,
            countries: CountryWordsObservation::Measured(words),
        }
    }

    fn source(
        observation: &FirstCombatCountriesObservation,
        operand: TrainingObservation<i32>,
    ) -> Value {
        let training = TrainingScalars {
            country_operand: operand,
            ..TrainingScalars::default()
        };
        let projection = match observation.projection() {
            None => Value::Null,
            Some(None) => json!({"status":"absent"}),
            Some(Some(combat)) => json!({"status":"measured", "class":match combat.class {
                CombatClass::Land => "land", CombatClass::BorderWar => "border_war",
            }, "type":combat.kind, "id":combat.id}),
        };
        json!({"divisions":[{"div_id":7,"div_type":4713,"div_uid":"7:4713",
            "first_combat":projection,
            "first_combat_countries":serde_json::from_str::<Value>(&observation.json()).unwrap(),
            "training_scalars":serde_json::from_str::<Value>(&training.json()).unwrap()}]})
    }

    fn world(raw: Value, hour: i64) -> WorldFrame {
        WorldFrame::from_raw_scheduled(
            crate::RawFrame {
                hour_index: hour,
                date: None,
                save_version: None,
                schema_version: None,
                rest: raw.as_object().unwrap().clone().into(),
            },
            None,
            None,
            None,
            crate::BuildSchedule::Serial,
        )
    }

    fn membership(raw: Value) -> Result<FirstCombatMembershipResult, Refusal> {
        world(raw, 7).division_first_combat_country_membership(7, 4713)
    }

    fn expected(is_member: bool) -> Result<FirstCombatMembershipResult, Refusal> {
        Ok(FirstCombatMembershipResult::Present {
            combat: COMBAT,
            is_member,
        })
    }

    fn refusal() -> CombatMembershipRefusal {
        CombatMembershipRefusal {
            cause: CombatMembershipCause::Cycle,
            field: CombatMembershipField::Next,
            stage: CombatMembershipStage::Admission,
            ordinal: Some(4),
            original_ordinal: Some(1),
        }
    }

    fn with_links(mut raw: Value, links: Value) -> Value {
        raw["tag_db_id_index"] = json!({"country_link_indices":links});
        raw
    }

    #[test]
    fn complete_census_preserves_signed_order_duplicates_empty_and_combat_identity() {
        for combat in [
            COMBAT,
            CombatIdentity {
                class: CombatClass::BorderWar,
                kind: 999,
                id: i32::MAX as u32,
            },
        ] {
            for words in [vec![], vec![i32::MIN, 0, -1, 0, i32::MAX]] {
                let observation = FirstCombatCountriesObservation::Present {
                    combat,
                    countries: CountryWordsObservation::Measured(words),
                };
                assert_eq!(
                    world(source(&observation, TrainingObservation::Measured(-1)), 7)
                        .division_first_combat_countries(7, 4713),
                    Ok(observation)
                );
            }
        }
        assert_eq!(
            membership(source(&present(vec![]), TrainingObservation::Measured(7))),
            expected(false)
        );
    }

    #[test]
    fn no_first_combat_is_distinct_from_false_and_needs_no_country_operand() {
        let mut raw = source(
            &FirstCombatCountriesObservation::NoFirstCombat,
            TrainingObservation::Unavailable(TrainingRefusal::NotObserved),
        );
        raw["divisions"][0]
            .as_object_mut()
            .unwrap()
            .remove("training_scalars");
        assert_eq!(
            membership(raw),
            Ok(FirstCombatMembershipResult::NoFirstCombat)
        );
    }

    #[test]
    fn complete_raw_equal_and_null_short_circuits_need_no_mapping() {
        for operand in [i32::MIN, -1, 0, 7, i32::MAX] {
            assert_eq!(
                membership(source(
                    &present(vec![operand]),
                    TrainingObservation::Measured(operand)
                )),
                expected(true)
            );
        }
        for (operand, words) in [
            (0, vec![i32::MIN, -1, 7, i32::MAX]),
            (i32::MIN, vec![0]),
            (i32::MAX, vec![0]),
        ] {
            assert_eq!(
                membership(source(
                    &present(words),
                    TrainingObservation::Measured(operand)
                )),
                expected(false)
            );
        }
    }

    #[test]
    fn complete_raw_match_prevails_over_missing_mapping_in_either_order() {
        for words in [vec![i32::MIN, 7], vec![7, i32::MIN]] {
            assert_eq!(
                membership(source(&present(words), TrainingObservation::Measured(7))),
                expected(true)
            );
        }
        assert_eq!(
            membership(source(
                &present(vec![i32::MIN]),
                TrainingObservation::Measured(7)
            )),
            Err(Refusal::CountryMapping(
                CountryHandleEqualityRefusal::Missing
            ))
        );
    }

    #[test]
    fn mapped_equality_uses_all_cells_without_owner_or_roster_substitution() {
        let mut raw = source(&present(vec![2]), TrainingObservation::Measured(1));
        raw["divisions"][0]["owner"] = json!("OWNER");
        raw["countries"] = json!({"OWNER":{"_r2_tag_db_id":99}});
        assert_eq!(
            membership(with_links(
                raw.clone(),
                json!({"status":"measured","indices":[0,0,0]})
            )),
            expected(true)
        );
        assert_eq!(
            membership(with_links(
                raw,
                json!({"status":"measured","indices":[0,3,4]})
            )),
            expected(false)
        );
    }

    #[test]
    fn unequal_signed_extremes_preserve_bits_and_mapping_refusals() {
        for (operand, word, handle) in [
            (i32::MIN, i32::MAX, 0x8000_0000),
            (i32::MAX, i32::MIN, 0x7fff_ffff),
            (1, -1, u32::MAX),
        ] {
            let raw = with_links(
                source(&present(vec![word]), TrainingObservation::Measured(operand)),
                json!({"status":"measured","indices":[0,1]}),
            );
            assert_eq!(
                membership(raw),
                Err(Refusal::CountryMapping(
                    CountryHandleEqualityRefusal::OutOfRange { handle, count: 2 }
                ))
            );
        }
        let raw = with_links(
            source(
                &present(vec![-1, i32::MIN]),
                TrainingObservation::Measured(1),
            ),
            json!({"status":"measured","indices":[0,1]}),
        );
        assert_eq!(
            membership(raw),
            Err(Refusal::CountryMapping(
                CountryHandleEqualityRefusal::OutOfRange {
                    handle: u32::MAX,
                    count: 2
                }
            ))
        );
    }

    #[test]
    fn unavailable_census_refuses_before_comparison_or_missing_training() {
        let observation = FirstCombatCountriesObservation::Present {
            combat: COMBAT,
            countries: CountryWordsObservation::Unavailable(refusal()),
        };
        let mut raw = source(&observation, TrainingObservation::Measured(7));
        raw["divisions"][0]
            .as_object_mut()
            .unwrap()
            .remove("training_scalars");
        assert_eq!(
            world(raw.clone(), 7).division_first_combat_countries(7, 4713),
            Ok(observation)
        );
        assert_eq!(
            membership(raw.clone()),
            Err(Refusal::Unavailable(refusal()))
        );
        // A captured matching prefix is forbidden on an unavailable census.
        raw["divisions"][0]["first_combat_countries"]["countries"]["raw_i32"] = json!([7]);
        assert!(matches!(membership(raw), Err(Refusal::Malformed(_))));
    }

    #[test]
    fn outer_unavailable_preserves_refusal_and_rejects_a_retained_projection() {
        let reason = CombatMembershipRefusal {
            cause: CombatMembershipCause::RootChanged,
            field: CombatMembershipField::FrameRoot,
            stage: CombatMembershipStage::Close,
            ordinal: None,
            original_ordinal: None,
        };
        let observation = FirstCombatCountriesObservation::Unavailable(reason);
        let mut raw = source(&observation, TrainingObservation::Measured(7));
        assert_eq!(
            world(raw.clone(), 7).division_first_combat_countries(7, 4713),
            Ok(observation)
        );
        assert_eq!(membership(raw.clone()), Err(Refusal::Unavailable(reason)));
        raw["divisions"][0]["first_combat"] = json!({"status":"absent"});
        assert!(matches!(membership(raw), Err(Refusal::Malformed(_))));
    }

    #[test]
    fn paired_training_operand_requires_full_current_strict_shape_even_for_empty_census() {
        for words in [vec![], vec![7]] {
            for reason in [
                TrainingRefusal::NotObserved,
                TrainingRefusal::Unreadable,
                TrainingRefusal::InvalidIdentity,
                TrainingRefusal::Changed,
                TrainingRefusal::BoundExceeded,
            ] {
                assert_eq!(
                    membership(source(
                        &present(words.clone()),
                        TrainingObservation::Unavailable(reason)
                    )),
                    Err(Refusal::CountryOperandUnavailable(reason))
                );
            }
            let clean = source(&present(words), TrainingObservation::Measured(7));
            let mut missing = clean.clone();
            missing["divisions"][0]
                .as_object_mut()
                .unwrap()
                .remove("training_scalars");
            assert_eq!(
                membership(missing),
                Err(Refusal::TrainingScalars(DivisionTrainingRefusal::Missing))
            );
            let mut null = clean.clone();
            null["divisions"][0]["training_scalars"] = Value::Null;
            assert_eq!(
                membership(null),
                Err(Refusal::TrainingScalars(
                    DivisionTrainingRefusal::Unobtained
                ))
            );
            for field in [
                "experience",
                "current_hp",
                "maximum_hp",
                "template_count",
                "army",
                "country_operand",
            ] {
                let mut bad = clean.clone();
                bad["divisions"][0]["training_scalars"]
                    .as_object_mut()
                    .unwrap()
                    .remove(field);
                assert!(matches!(
                    membership(bad),
                    Err(Refusal::TrainingScalars(
                        DivisionTrainingRefusal::Malformed(_)
                    ))
                ));
            }
        }
    }

    #[test]
    fn current_identity_cannot_come_from_stale_id_kind_or_duplicate_rows() {
        let clean = source(&present(vec![7]), TrainingObservation::Measured(7));
        for rows in [Value::Null, json!([]), json!({}), json!([null])] {
            let mut bad = clean.clone();
            bad["divisions"] = rows;
            assert_eq!(membership(bad), Err(Refusal::CurrentIdentityUnavailable));
        }
        for (field, value) in [
            ("div_id", json!(0)),
            ("div_id", json!(8)),
            ("div_id", json!("7")),
            ("div_type", json!(51)),
            ("div_type", Value::Null),
            ("div_type", json!(4294967296u64)),
        ] {
            let mut bad = clean.clone();
            bad["divisions"][0][field] = value;
            assert_eq!(membership(bad), Err(Refusal::CurrentIdentityUnavailable));
        }
        let mut duplicate = clean.clone();
        duplicate["divisions"]
            .as_array_mut()
            .unwrap()
            .push(clean["divisions"][0].clone());
        assert_eq!(
            membership(duplicate),
            Err(Refusal::CurrentIdentityUnavailable)
        );
        let mut other_kind = clean.clone();
        let mut other = clean["divisions"][0].clone();
        other["div_type"] = json!(51);
        other["first_combat_countries"] = Value::Null;
        other_kind["divisions"].as_array_mut().unwrap().push(other);
        let frame = world(other_kind, 7);
        assert_eq!(
            frame.division_first_combat_country_membership(7, 4713),
            expected(true)
        );
        assert_eq!(
            frame.division_first_combat_country_membership(7, 51),
            Err(Refusal::Unobtained)
        );
    }

    #[test]
    fn d20_gap_historical_fault_and_malformed_ledger_precede_raw_equality() {
        let clean = source(&present(vec![7]), TrainingObservation::Measured(7));
        for problem in [
            "domain_gap",
            "malformed_payload",
            "partial_data",
            "family_gap",
            "source_unavailable",
            "compact_status_failure",
        ] {
            let mut bad = clean.clone();
            bad["provenance"] =
                json!({"collector":{"domain_problems":[{"domain_id":20,"problem":problem}]}});
            assert_eq!(membership(bad), Err(Refusal::DomainProblem(problem.into())));
        }
        for ledger in [
            Value::Null,
            json!(42),
            json!([{}]),
            json!([{"domain_id":"20","problem":"partial_data"}]),
        ] {
            let mut bad = clean.clone();
            bad["provenance"] = json!({"collector":{"domain_problems":ledger}});
            assert_eq!(membership(bad), Err(Refusal::MalformedLedger));
        }
        let mut unrelated = clean.clone();
        unrelated["provenance"] = json!({"collector":{"domain_problems":[{"domain_id":98,"problem":"source_unavailable"}]}});
        assert_eq!(membership(unrelated), expected(true));
        assert_eq!(
            world(clean, 8).division_first_combat_country_membership(7, 4713),
            expected(true)
        );
    }

    #[test]
    fn mapping_needed_uses_current_d18_ledger_without_reusing_a_previous_frame() {
        let clean = with_links(
            source(&present(vec![2]), TrainingObservation::Measured(1)),
            json!({"status":"measured","indices":[0,3,3]}),
        );
        assert_eq!(
            world(clean.clone(), 6).division_first_combat_country_membership(7, 4713),
            expected(true)
        );
        let mut blocked = clean.clone();
        blocked["provenance"] =
            json!({"collector":{"domain_problems":[{"domain_id":18,"problem":"partial_data"}]}});
        assert_eq!(
            membership(blocked.clone()),
            Err(Refusal::CountryMapping(
                CountryHandleEqualityRefusal::DomainUnavailable("partial_data".into())
            ))
        );
        blocked["divisions"][0]["first_combat_countries"]["countries"]["raw_i32"] = json!([2, 1]);
        assert_eq!(membership(blocked), expected(true));
        assert_eq!(
            world(clean, 8).division_first_combat_country_membership(7, 4713),
            expected(true)
        );
        assert_eq!(
            membership(source(&present(vec![2]), TrainingObservation::Measured(1))),
            Err(Refusal::CountryMapping(
                CountryHandleEqualityRefusal::Missing
            ))
        );
    }

    #[test]
    fn strict_missing_null_schema_and_nested_carrier_shapes_are_named() {
        let clean = source(&present(vec![7]), TrainingObservation::Measured(7));
        let mut missing = clean.clone();
        missing["divisions"][0]
            .as_object_mut()
            .unwrap()
            .remove("first_combat_countries");
        assert_eq!(membership(missing), Err(Refusal::Missing));
        let mut null = clean.clone();
        null["divisions"][0]["first_combat_countries"] = Value::Null;
        assert_eq!(membership(null), Err(Refusal::Unobtained));
        let carrier = clean["divisions"][0]["first_combat_countries"].clone();
        for field in ["schema", "status", "combat", "countries"] {
            let mut bad = carrier.clone();
            bad.as_object_mut().unwrap().remove(field);
            assert!(parse_observation(&bad).is_err(), "missing {field}");
            let mut bad = carrier.clone();
            bad[field] = Value::Null;
            assert!(parse_observation(&bad).is_err(), "null {field}");
        }
        for schema in [json!(0), json!(2), json!(1.0), json!("1")] {
            let mut bad = carrier.clone();
            bad["schema"] = schema;
            assert!(parse_observation(&bad).is_err());
        }
        for pointer in ["", "/combat", "/countries"] {
            let mut bad = carrier.clone();
            bad.pointer_mut(pointer).unwrap()["role"] = json!("defender");
            assert!(
                parse_observation(&bad).is_err(),
                "unknown role at {pointer}"
            );
        }
        for countries in [
            json!({"status":"measured"}),
            json!({"status":"measured","raw_i32":null}),
            json!({"status":"measured","raw_i32":[2147483648u64]}),
            json!({"status":"measured","raw_i32":[-2147483649i64]}),
            json!({"status":"measured","raw_i32":[7.0]}),
            json!({"status":"measured","raw_i32":["7"]}),
            json!({"status":"partial","raw_i32":[7]}),
            json!({"status":"measured","raw_i32":[7],"refusal":null}),
        ] {
            let mut bad = carrier.clone();
            bad["countries"] = countries;
            assert!(parse_observation(&bad).is_err());
        }
        for bad in [
            json!({"schema":1,"status":"no_first_combat","countries":{"status":"measured","raw_i32":[]}}),
            json!({"schema":1,"status":"future"}),
            json!({"schema":1,"status":{"present":null}}),
        ] {
            assert!(parse_observation(&bad).is_err());
        }
    }

    #[test]
    fn full_combat_projection_and_identity_are_strict() {
        let clean = source(&present(vec![7]), TrainingObservation::Measured(7));
        for projection in [
            Value::Null,
            json!({"status":"absent"}),
            json!({"status":"measured","class":"border_war","type":62,"id":79}),
            json!({"status":"measured","class":"land","type":63,"id":79}),
            json!({"status":"measured","class":"land","type":62,"id":80}),
            json!({"status":"measured","class":"land","type":62,"id":79,"role":"attacker"}),
        ] {
            let mut bad = clean.clone();
            bad["divisions"][0]["first_combat"] = projection;
            assert!(matches!(membership(bad), Err(Refusal::Malformed(_))));
        }
        let mut missing = clean.clone();
        missing["divisions"][0]
            .as_object_mut()
            .unwrap()
            .remove("first_combat");
        assert!(matches!(membership(missing), Err(Refusal::Malformed(_))));
        for (field, value) in [
            ("class", json!("naval")),
            ("type", json!(63)),
            ("type", json!(-1)),
            ("id", json!(0)),
            ("id", json!(2147483648u64)),
            ("id", json!("79")),
        ] {
            let mut bad = clean.clone();
            bad["divisions"][0]["first_combat_countries"]["combat"][field] = value.clone();
            bad["divisions"][0]["first_combat"][field] = value;
            assert!(matches!(membership(bad), Err(Refusal::Malformed(_))));
        }
    }

    #[test]
    fn strict_refusal_words_ordinals_and_cycles_are_not_defaulted() {
        let observation = FirstCombatCountriesObservation::Present {
            combat: COMBAT,
            countries: CountryWordsObservation::Unavailable(refusal()),
        };
        let carrier: Value = serde_json::from_str(&observation.json()).unwrap();
        for field in ["cause", "field", "stage", "ordinal", "original_ordinal"] {
            let mut bad = carrier.clone();
            bad["countries"]["refusal"]
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert!(parse_observation(&bad).is_err(), "missing {field}");
        }
        for field in ["cause", "field", "stage"] {
            for value in [
                json!("future"),
                json!({"cycle":null}),
                Value::Null,
                json!(0),
            ] {
                let mut bad = carrier.clone();
                bad["countries"]["refusal"][field] = value;
                assert!(parse_observation(&bad).is_err(), "{field}");
            }
        }
        for (ordinal, original) in [
            (Value::Null, json!(1)),
            (json!(4), Value::Null),
            (json!(4), json!(4)),
            (json!(4), json!(5)),
            (json!(-1), json!(0)),
            (json!(4294967296u64), json!(0)),
            (json!(4.0), json!(1)),
            (json!(4), json!("1")),
        ] {
            let mut bad = carrier.clone();
            bad["countries"]["refusal"]["ordinal"] = ordinal;
            bad["countries"]["refusal"]["original_ordinal"] = original;
            assert!(parse_observation(&bad).is_err());
        }
        let mut bad = carrier;
        bad["countries"]["refusal"]["address"] = json!(4096);
        assert!(parse_observation(&bad).is_err());
    }

    #[test]
    fn every_portable_refusal_name_survives_strict_json() {
        for cause in 0..=13 {
            for field in 0..=24 {
                for stage in 0..=2 {
                    let observation =
                        FirstCombatCountriesObservation::Unavailable(CombatMembershipRefusal {
                            cause: CombatMembershipCause::from_raw(cause).unwrap(),
                            field: CombatMembershipField::from_raw(field).unwrap(),
                            stage: CombatMembershipStage::from_raw(stage).unwrap(),
                            ordinal: Some(4),
                            original_ordinal: Some(1),
                        });
                    assert_eq!(
                        parse_observation(
                            &serde_json::from_str::<Value>(&observation.json()).unwrap()
                        ),
                        Ok(observation)
                    );
                }
            }
        }
    }

    #[test]
    #[ignore = "requires hash-bound verbatim D2RA paired sampler/publisher/fold output"]
    fn actual_sampler_fold_stages_preserve_membership_and_recovery() {
        use sha2::{Digest, Sha256};
        let path = std::env::var_os("BATTLE_PLAN_D2RA_MEMBERSHIP_JOINT_INPUT").or_else(|| std::env::var_os("HOI4_D2RA_MEMBERSHIP_JOINT_INPUT"))
            .expect("native paired sampler output required");
        assert!(std::path::Path::new(&path).is_absolute());
        assert!(std::fs::metadata(&path).unwrap().len() <= 1024 * 1024);
        let bytes = std::fs::read(path).unwrap();
        assert!(bytes.len() <= 1024 * 1024);
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            std::env::var("BATTLE_PLAN_D2RA_MEMBERSHIP_JOINT_SHA256").or_else(|_| std::env::var("HOI4_D2RA_MEMBERSHIP_JOINT_SHA256")).unwrap()
        );
        let artifact: Value = snapshot::parse_json_strict(&bytes).unwrap();
        assert_eq!(artifact["schema"], 1);
        let stages = artifact["stages"].as_array().unwrap();
        let expected_stages = [
            ("measured", 0),
            ("unavailable_same_hour", 0),
            ("gap_same_hour", 0),
            ("full_same_hour_after_gap", 0),
            ("recovery_next_hour", 1),
            ("empty_delta_after_recovery", 2),
            ("malformed_pair", 2),
            ("empty_after_malformed", 2),
        ];
        assert_eq!(stages.len(), expected_stages.len());
        for (stage, (name, hour_offset)) in stages.iter().zip(expected_stages) {
            assert_eq!(stage["name"], name);
            let value = &stage["snapshot"];
            assert_eq!(
                format!("{:x}", Sha256::digest(serde_json::to_vec(value).unwrap())),
                stage["snapshot_sha256"].as_str().unwrap(),
                "{name}: exact owning fold snapshot"
            );
            let hex = stage["raw_h4cr_hex"].as_str().unwrap();
            assert_eq!(hex.len() % 2, 0);
            let raw_bytes: Vec<u8> = hex
                .as_bytes()
                .chunks_exact(2)
                .map(|part| u8::from_str_radix(std::str::from_utf8(part).unwrap(), 16).unwrap())
                .collect();
            assert!(raw_bytes.starts_with(b"H4CR"));
            assert_eq!(
                format!("{:x}", Sha256::digest(&raw_bytes)),
                stage["raw_h4cr_sha256"].as_str().unwrap(),
                "{name}: exact sampler/publisher records"
            );
            // The collector exporter proves these bytes with its strict decoder and both
            // owning WorldFold routes before writing the artifact; this side never edits it.
            let snapshot = snapshot::Snapshot::from_world_value(value.clone()).unwrap();
            let expected_rest = snapshot.rest.clone();
            let raw = crate::RawFrame {
                hour_index: snapshot.hour_index.unwrap(),
                date: snapshot.date,
                save_version: snapshot.save_version,
                schema_version: snapshot.schema_version,
                rest: snapshot.rest.into(),
            };
            assert_eq!(
                raw.rest.to_map(),
                expected_rest,
                "{name}: no reconstructed fixture rows"
            );
            let frame =
                WorldFrame::from_raw_scheduled(raw, None, None, None, crate::BuildSchedule::Serial);
            assert_eq!(frame.hour_index(), 16_959_367 + hour_offset);
            assert_eq!(
                frame.raw().get("divisions"),
                value.get("divisions"),
                "{name}: retain owning D20 snapshot"
            );
            let result = frame.division_first_combat_country_membership(7, 4713);
            let problem = match name {
                "gap_same_hour" | "full_same_hour_after_gap" => Some("domain_gap"),
                "malformed_pair" | "empty_after_malformed" => Some("malformed_payload"),
                _ => None,
            };
            if let Some(problem) = problem {
                assert!(
                    value["provenance"]["collector"]["domain_problems"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|p| p["domain_id"] == 20 && p["problem"] == problem),
                    "{name}"
                );
                assert_eq!(
                    result,
                    Err(Refusal::DomainProblem(problem.into())),
                    "{name}"
                );
                assert_eq!(
                    frame.division_first_combat_countries(7, 4713),
                    Err(Refusal::DomainProblem(problem.into())),
                    "{name}: history precedes raw state"
                );
                continue;
            }
            assert_eq!(
                frame
                    .division_training_scalars(7, 4713)
                    .unwrap()
                    .country_operand,
                TrainingObservation::Measured(i32::MIN),
                "{name}: use paired signed primary operand"
            );
            if name == "unavailable_same_hour" {
                let reason = CombatMembershipRefusal {
                    cause: CombatMembershipCause::Unreadable,
                    field: CombatMembershipField::Next,
                    stage: CombatMembershipStage::Open,
                    ordinal: Some(0),
                    original_ordinal: None,
                };
                assert_eq!(
                    frame.division_first_combat_countries(7, 4713),
                    Ok(FirstCombatCountriesObservation::Present {
                        combat: COMBAT,
                        countries: CountryWordsObservation::Unavailable(reason)
                    })
                );
                assert_eq!(
                    result,
                    Err(Refusal::Unavailable(reason)),
                    "an unavailable changed row must overwrite its earlier measured census"
                );
            } else {
                assert_eq!(
                    frame.division_first_combat_countries(7, 4713),
                    Ok(present(vec![i32::MAX, i32::MIN, 0, i32::MIN])),
                    "{name}"
                );
                assert_eq!(
                    result,
                    expected(true),
                    "{name}: raw equality survives absent country map"
                );
            }
        }
    }
}
