//! The current D20 retained byte, after independent current-owner admission.
use crate::{Leaf, ViewStatus, WorldFrame};
pub use portable_compact_stream::division_routes::{
    CombatMembershipCause, CombatMembershipStage, DivisionOwnerAuthority,
    RetainedVisualVariantField, RetainedVisualVariantObservation, RetainedVisualVariantRefusal,
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DivisionRetainedVisualVariantRefusal {
    Missing,
    Unobtained,
    Malformed(String),
    MalformedLedger,
    DomainProblem(String),
    CurrentIdentityUnavailable,
    OwnerAuthorityMissing,
    OwnerAuthorityUnobtained,
    OwnerAuthorityUnavailable(RetainedVisualVariantRefusal),
    OwnerIdentityUnavailable,
    EnclosingAdmissionUnavailable(RetainedVisualVariantRefusal),
}

type Refusal = DivisionRetainedVisualVariantRefusal;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RefusalJson {
    cause: String,
    field: String,
    stage: String,
}

impl RefusalJson {
    fn parse(self) -> Result<RetainedVisualVariantRefusal, String> {
        let result = RetainedVisualVariantRefusal {
            cause: CombatMembershipCause::from_name(&self.cause)?,
            field: RetainedVisualVariantField::from_name(&self.field)?,
            stage: CombatMembershipStage::from_name(&self.stage)?,
        };
        result.validate()?;
        Ok(result)
    }
}

#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum ObservationJson {
    Measured { schema: u32, raw_u8: u8 },
    Unavailable { schema: u32, refusal: RefusalJson },
}

fn parse_observation(value: &Value) -> Result<RetainedVisualVariantObservation, String> {
    let (schema, value) = match ObservationJson::deserialize(value).map_err(|e| e.to_string())? {
        ObservationJson::Measured { schema, raw_u8 } => {
            (schema, RetainedVisualVariantObservation::Measured(raw_u8))
        }
        ObservationJson::Unavailable { schema, refusal } => (
            schema,
            RetainedVisualVariantObservation::Unavailable(refusal.parse()?),
        ),
    };
    if schema != 1 {
        return Err("retained visual variant schema".into());
    }
    Ok(value)
}

#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum OwnerJson {
    Witnessed { schema: u32 },
    Unavailable { schema: u32, refusal: RefusalJson },
}

fn parse_owner(value: &Value) -> Result<DivisionOwnerAuthority, String> {
    let (schema, value) = match OwnerJson::deserialize(value).map_err(|e| e.to_string())? {
        OwnerJson::Witnessed { schema } => (schema, DivisionOwnerAuthority::Witnessed),
        OwnerJson::Unavailable { schema, refusal } => (
            schema,
            DivisionOwnerAuthority::Unavailable(refusal.parse()?),
        ),
    };
    if schema != 1 {
        return Err("division owner authority schema".into());
    }
    Ok(value)
}

impl WorldFrame {
    /// Missing/null/unavailable refers only to the byte after the enclosing row,
    /// owner witness and current country mapping have passed. Those scalar
    /// outcomes alone may permit a consumer's proven all-numbered-candidates-miss rule.
    pub fn division_retained_visual_variant(
        &self,
        id: u32,
        kind: u32,
    ) -> Result<RetainedVisualVariantObservation, Refusal> {
        match crate::world::rederive_collector_domain_problems(self.raw()) {
            Err(()) => return Err(Refusal::MalformedLedger),
            Ok(Some(problems)) => {
                for problem in problems {
                    if matches!(problem.domain, 18 | 20)
                        && !crate::world::domain_problem_is_per_family_accounted(&problem)
                    {
                        return Err(Refusal::DomainProblem(format!(
                            "d{}: {}",
                            problem.domain, problem.problem
                        )));
                    }
                }
            }
            Ok(None) => {}
        }
        if id == 0 || self.division_units_status() != ViewStatus::Present {
            return Err(Refusal::CurrentIdentityUnavailable);
        }
        let mut typed = self.division_units().iter().filter(|row| {
            row.raw_division_id == id
                && matches!(row.raw_division_type, Leaf::Measured(value) if value == kind)
        });
        let typed = match (typed.next(), typed.next()) {
            (Some(row), None) => row,
            _ => return Err(Refusal::CurrentIdentityUnavailable),
        };
        let rows = self
            .raw()
            .get("divisions")
            .and_then(Value::as_array)
            .ok_or(Refusal::CurrentIdentityUnavailable)?;
        let mut found = rows.iter().filter(|row| {
            row.get("div_id").and_then(Value::as_u64) == Some(u64::from(id))
                && row.get("div_type").and_then(Value::as_u64) == Some(u64::from(kind))
        });
        let row = match (found.next(), found.next()) {
            (Some(row), None) => row,
            _ => return Err(Refusal::CurrentIdentityUnavailable),
        };
        let uid = format!("{id}:{kind}");
        if typed.div_uid != uid || row.get("div_uid").and_then(Value::as_str) != Some(uid.as_str())
        {
            return Err(Refusal::CurrentIdentityUnavailable);
        }
        let owner = match row.get("owner_authority") {
            None => return Err(Refusal::OwnerAuthorityMissing),
            Some(Value::Null) => return Err(Refusal::OwnerAuthorityUnobtained),
            Some(value) => parse_owner(value).map_err(Refusal::Malformed)?,
        };
        if let DivisionOwnerAuthority::Unavailable(reason) = owner {
            return Err(Refusal::OwnerAuthorityUnavailable(reason));
        }
        let owner_id = row
            .get("owner_tag_db_id")
            .and_then(Value::as_u64)
            .and_then(|raw| u32::try_from(raw).ok())
            .filter(|raw| *raw > 0 && *raw < i32::MAX as u32)
            .ok_or(Refusal::OwnerIdentityUnavailable)?;
        let owner = match &typed.owner {
            Leaf::Measured(owner) => owner,
            _ => return Err(Refusal::OwnerIdentityUnavailable),
        };
        if self
            .country_identity_for_tag(owner.tag())
            .map_err(|_| Refusal::OwnerIdentityUnavailable)?
            != owner_id
        {
            return Err(Refusal::OwnerIdentityUnavailable);
        }
        match row.get("retained_visual_variant") {
            None => Err(Refusal::Missing),
            Some(Value::Null) => Err(Refusal::Unobtained),
            Some(value) => {
                let observation = parse_observation(value).map_err(Refusal::Malformed)?;
                if let RetainedVisualVariantObservation::Unavailable(reason) = observation {
                    use CombatMembershipCause as C;
                    let scalar_only = reason == RetainedVisualVariantRefusal::default()
                        || (reason.field == RetainedVisualVariantField::RetainedVisualVariant
                            && matches!(
                                reason.cause,
                                C::Unreadable
                                    | C::Changed
                                    | C::VerificationBudgetExceeded
                                    | C::RetainedBudgetExceeded
                                    | C::AllocationFailed
                                    | C::AddressOverflow
                                    | C::InvalidAddress
                            ));
                    if !scalar_only {
                        return Err(Refusal::EnclosingAdmissionUnavailable(reason));
                    }
                }
                Ok(observation)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn source(raw: u8) -> Value {
        json!({"tag_db_id_index":{"country_composition_version":2},
            "countries":{"GER":{"_r2_tag_db_id":7}},
            "divisions":[{"div_id":17,"div_type":4713,"div_uid":"17:4713","owner":"GER","owner_tag_db_id":7,
                "owner_authority":{"schema":1,"status":"witnessed"},
                "retained_visual_variant":{"schema":1,"status":"measured","raw_u8":raw}}]})
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

    fn value(raw: Value) -> Result<RetainedVisualVariantObservation, Refusal> {
        world(raw, 7).division_retained_visual_variant(17, 4713)
    }

    fn unavailable(
        field: RetainedVisualVariantField,
        cause: CombatMembershipCause,
    ) -> RetainedVisualVariantRefusal {
        RetainedVisualVariantRefusal {
            cause,
            field,
            stage: CombatMembershipStage::Close,
        }
    }

    #[test]
    fn current_typed_owner_admits_measured_zero_and_signed_byte_extremes() {
        for raw in [0, 1, 127, 128, 255] {
            assert_eq!(
                value(source(raw)),
                Ok(RetainedVisualVariantObservation::Measured(raw))
            );
        }
    }

    #[test]
    fn scalar_missing_null_and_local_unavailable_remain_distinct() {
        let mut raw = source(0);
        raw["divisions"][0]
            .as_object_mut()
            .unwrap()
            .remove("retained_visual_variant");
        assert_eq!(value(raw.clone()), Err(Refusal::Missing));
        raw["divisions"][0]["retained_visual_variant"] = Value::Null;
        assert_eq!(value(raw.clone()), Err(Refusal::Unobtained));
        for reason in [
            RetainedVisualVariantRefusal::default(),
            unavailable(
                RetainedVisualVariantField::RetainedVisualVariant,
                CombatMembershipCause::Unreadable,
            ),
            unavailable(
                RetainedVisualVariantField::RetainedVisualVariant,
                CombatMembershipCause::Changed,
            ),
        ] {
            let observation = RetainedVisualVariantObservation::Unavailable(reason);
            raw["divisions"][0]["retained_visual_variant"] =
                serde_json::from_str(&observation.json()).unwrap();
            assert_eq!(value(raw.clone()), Ok(observation));
        }
    }

    #[test]
    fn owner_authority_failures_precede_every_scalar_unknown_shortcut() {
        for scalar in [
            Value::Null,
            json!({"schema":1,"status":"unavailable",
            "refusal":{"cause":"not_observed","field":"frame_root","stage":"admission"}}),
        ] {
            let mut raw = source(0);
            raw["divisions"][0]["retained_visual_variant"] = scalar;
            raw["divisions"][0]
                .as_object_mut()
                .unwrap()
                .remove("owner_authority");
            assert_eq!(value(raw.clone()), Err(Refusal::OwnerAuthorityMissing));
            raw["divisions"][0]["owner_authority"] = Value::Null;
            assert_eq!(value(raw.clone()), Err(Refusal::OwnerAuthorityUnobtained));
            let reason = unavailable(
                RetainedVisualVariantField::DivisionOwner,
                CombatMembershipCause::Changed,
            );
            raw["divisions"][0]["owner_authority"] =
                serde_json::from_str(&DivisionOwnerAuthority::Unavailable(reason).json()).unwrap();
            assert_eq!(
                value(raw.clone()),
                Err(Refusal::OwnerAuthorityUnavailable(reason))
            );
            raw["divisions"][0]["owner_authority"] =
                json!({"schema":1,"status":"witnessed","raw_u8":0});
            assert!(matches!(value(raw), Err(Refusal::Malformed(_))));
        }
        let mut raw = source(0);
        raw["divisions"][0]
            .as_object_mut()
            .unwrap()
            .remove("retained_visual_variant");
        raw["divisions"][0]
            .as_object_mut()
            .unwrap()
            .remove("owner_authority");
        assert_eq!(value(raw), Err(Refusal::OwnerAuthorityMissing));
    }

    #[test]
    fn exact_owner_projection_requires_current_uncontended_country_mapping() {
        for bad in [
            Value::Null,
            json!(0),
            json!(-1),
            json!(i32::MAX),
            json!(8),
            json!("7"),
        ] {
            let mut raw = source(128);
            raw["divisions"][0]["owner_tag_db_id"] = bad;
            assert_eq!(value(raw), Err(Refusal::OwnerIdentityUnavailable));
        }
        let mut absent = source(128);
        absent.as_object_mut().unwrap().remove("countries");
        assert_eq!(value(absent), Err(Refusal::OwnerIdentityUnavailable));
        let mut contended = source(128);
        contended["tag_db_id_index"]["withheld_countries"] =
            json!({"contended_keys":{"GER":{"tag_db_ids":[7,8]}}});
        assert_eq!(value(contended), Err(Refusal::OwnerIdentityUnavailable));
        let mut wrong = source(128);
        wrong["countries"]["GER"]["_r2_tag_db_id"] = json!(8);
        assert_eq!(value(wrong), Err(Refusal::OwnerIdentityUnavailable));
    }

    #[test]
    fn malformed_scalar_status_schema_width_filler_and_refusal_are_fatal() {
        for bad in [
            json!({}),
            json!(0),
            json!({"schema":2,"status":"measured","raw_u8":0}),
            json!({"schema":1,"status":"measured","raw_u8":256}),
            json!({"schema":1,"status":"measured","raw_u8":-1}),
            json!({"schema":1,"status":"measured","raw_u8":0,"refusal":null}),
            json!({"schema":1,"status":"unavailable","raw_u8":0,"refusal":{"cause":"unreadable","field":"retained_visual_variant","stage":"open"}}),
            json!({"schema":1,"status":"unavailable","refusal":{"cause":"future","field":"retained_visual_variant","stage":"open"}}),
            json!({"schema":1,"status":"unavailable","refusal":{"cause":"unreadable","field":"future","stage":"open"}}),
            json!({"schema":1,"status":"unavailable","refusal":{"cause":"unreadable","field":"retained_visual_variant","stage":"future"}}),
            json!({"schema":1,"status":"unavailable","refusal":{"cause":"not_observed","field":"retained_visual_variant","stage":"open"}}),
        ] {
            let mut raw = source(0);
            raw["divisions"][0]["retained_visual_variant"] = bad;
            assert!(matches!(value(raw), Err(Refusal::Malformed(_))));
        }
    }

    #[test]
    fn enclosing_refusals_cannot_be_laundered_as_scalar_unavailable() {
        for reason in [
            unavailable(
                RetainedVisualVariantField::FrameRoot,
                CombatMembershipCause::RootChanged,
            ),
            unavailable(
                RetainedVisualVariantField::Hour,
                CombatMembershipCause::HourUnreadable,
            ),
            unavailable(
                RetainedVisualVariantField::DivisionIdentity,
                CombatMembershipCause::Changed,
            ),
            unavailable(
                RetainedVisualVariantField::RetainedVisualVariant,
                CombatMembershipCause::HourChanged,
            ),
            unavailable(
                RetainedVisualVariantField::DivisionOwner,
                CombatMembershipCause::Unreadable,
            ),
        ] {
            let mut raw = source(0);
            raw["divisions"][0]["retained_visual_variant"] =
                serde_json::from_str(&RetainedVisualVariantObservation::Unavailable(reason).json())
                    .unwrap();
            assert_eq!(
                value(raw),
                Err(Refusal::EnclosingAdmissionUnavailable(reason))
            );
        }
    }

    #[test]
    fn domain_faults_and_malformed_ledger_precede_scalar_and_owner() {
        for domain in [18, 20] {
            for problem in [
                "domain_gap",
                "malformed_payload",
                "partial_data",
                "compact_status_failure",
            ] {
                let mut raw = source(0);
                raw["provenance"] = json!({"collector":{"domain_problems":[{"domain_id":domain,"problem":problem}]}});
                raw["divisions"][0]["owner_authority"] = Value::Null;
                assert_eq!(
                    value(raw),
                    Err(Refusal::DomainProblem(format!("d{domain}: {problem}")))
                );
            }
        }
        for ledger in [
            Value::Null,
            json!(42),
            json!([{}]),
            json!([{"domain_id":"20","problem":"partial_data"}]),
        ] {
            let mut raw = source(0);
            raw["provenance"] = json!({"collector":{"domain_problems":ledger}});
            assert_eq!(value(raw), Err(Refusal::MalformedLedger));
        }
    }

    #[test]
    fn duplicate_kind_id_or_uid_mismatch_and_missing_current_row_refuse() {
        let mut duplicate = source(0);
        let row = duplicate["divisions"][0].clone();
        duplicate["divisions"].as_array_mut().unwrap().push(row);
        assert_eq!(value(duplicate), Err(Refusal::CurrentIdentityUnavailable));
        for (key, replacement) in [
            ("div_id", json!(18)),
            ("div_type", json!(51)),
            ("div_uid", json!("17:51")),
            ("div_uid", Value::Null),
        ] {
            let mut raw = source(0);
            raw["divisions"][0][key] = replacement;
            assert_eq!(value(raw), Err(Refusal::CurrentIdentityUnavailable));
        }
        let mut missing = source(0);
        missing["divisions"] = json!([]);
        assert_eq!(value(missing), Err(Refusal::CurrentIdentityUnavailable));
    }

    #[test]
    fn cross_hour_fault_recovery_and_same_hour_branch_frames_never_cache_a_byte() {
        let first = world(source(127), 7);
        let mut faulty = source(128);
        faulty["provenance"] =
            json!({"collector":{"domain_problems":[{"domain_id":20,"problem":"domain_gap"}]}});
        let faulty = world(faulty, 8);
        let recovery = world(source(255), 9);
        let branch = world(source(0), 7);
        for _ in 0..2 {
            assert_eq!(
                recovery.division_retained_visual_variant(17, 4713),
                Ok(RetainedVisualVariantObservation::Measured(255))
            );
            assert!(matches!(
                faulty.division_retained_visual_variant(17, 4713),
                Err(Refusal::DomainProblem(_))
            ));
            assert_eq!(
                first.division_retained_visual_variant(17, 4713),
                Ok(RetainedVisualVariantObservation::Measured(127))
            );
            assert_eq!(
                branch.division_retained_visual_variant(17, 4713),
                Ok(RetainedVisualVariantObservation::Measured(0))
            );
        }
    }

    #[test]
    fn timeline_seek_play_and_cache_eviction_read_each_current_branch_snapshot() {
        struct Frames {
            frames: [Value; 3],
        }
        impl crate::source::ReplaySource for Frames {
            fn game_id(&self) -> &str {
                "retained-branch"
            }
            fn hours(&self) -> &[i64] {
                &[7, 8, 9]
            }
            fn warnings(&self) -> &[String] {
                &[]
            }
            fn frame(
                &self,
                hour_index: i64,
            ) -> Result<crate::RawFrame, crate::source::SourceError> {
                Ok(crate::RawFrame {
                    hour_index,
                    date: None,
                    save_version: None,
                    schema_version: None,
                    rest: self.frames[(hour_index - 7) as usize]
                        .as_object()
                        .unwrap()
                        .clone()
                        .into(),
                })
            }
        }
        for capacity in [0, 1, 3] {
            for branch_byte in [0, 128] {
                let mut gap = source(255);
                gap["provenance"] = json!({"collector":{"domain_problems":[{"domain_id":20,"problem":"domain_gap"}]}});
                let timeline = crate::WorldTimeline::open_with_config(
                    Box::new(Frames {
                        frames: [source(branch_byte), gap, source(127)],
                    }),
                    crate::TimelineConfig {
                        frame_cache_entries: capacity,
                        ..Default::default()
                    },
                )
                .unwrap();
                for hour in [9, 7, 8, 7, 9, 8] {
                    let seek = timeline
                        .seek(hour)
                        .unwrap()
                        .division_retained_visual_variant(17, 4713);
                    let play = timeline
                        .play_to(hour)
                        .unwrap()
                        .division_retained_visual_variant(17, 4713);
                    assert_eq!(seek, play);
                    match hour {
                        7 => assert_eq!(
                            seek,
                            Ok(RetainedVisualVariantObservation::Measured(branch_byte))
                        ),
                        8 => assert!(matches!(seek, Err(Refusal::DomainProblem(_)))),
                        9 => assert_eq!(seek, Ok(RetainedVisualVariantObservation::Measured(127))),
                        _ => unreachable!(),
                    }
                }
            }
        }
    }
}
