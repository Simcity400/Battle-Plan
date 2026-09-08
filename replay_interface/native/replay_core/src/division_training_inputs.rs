//! Current D20 underlying operands. Authored thresholds and derived selector results are not captured.
use crate::{Leaf, ViewStatus, WorldFrame};
use portable_compact_stream::division_routes as wire;
use serde::Deserialize;
use serde_json::Value;
pub use wire::{
    TrainingArmy, TrainingCount, TrainingObservation, TrainingRefusal, TrainingScalars,
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DivisionTrainingRefusal {
    Missing,
    Unobtained,
    Malformed(String),
    MalformedLedger,
    DomainProblem(String),
    CurrentIdentityUnavailable,
}
#[derive(Deserialize)]
#[serde(try_from = "String")]
enum Reason {
    NotObserved,
    Unreadable,
    InvalidIdentity,
    Changed,
    BoundExceeded,
}
impl TryFrom<String> for Reason {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "not_observed" => Ok(Self::NotObserved),
            "unreadable" => Ok(Self::Unreadable),
            "invalid_identity" => Ok(Self::InvalidIdentity),
            "changed" => Ok(Self::Changed),
            "bound_exceeded" => Ok(Self::BoundExceeded),
            _ => Err("unknown training refusal"),
        }
    }
}
impl From<Reason> for TrainingRefusal {
    fn from(v: Reason) -> Self {
        match v {
            Reason::NotObserved => Self::NotObserved,
            Reason::Unreadable => Self::Unreadable,
            Reason::InvalidIdentity => Self::InvalidIdentity,
            Reason::Changed => Self::Changed,
            Reason::BoundExceeded => Self::BoundExceeded,
        }
    }
}
#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum Scalar {
    Measured { raw_i64: i64 },
    Unavailable { reason: Reason },
}
impl From<Scalar> for TrainingObservation<i64> {
    fn from(v: Scalar) -> Self {
        match v {
            Scalar::Measured { raw_i64 } => Self::Measured(raw_i64),
            Scalar::Unavailable { reason } => Self::Unavailable(reason.into()),
        }
    }
}
#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum CountryOperand {
    Measured { raw_i32: i32 },
    Unavailable { reason: Reason },
}
#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum Count {
    Measured { entry_count: i32, sum_raw: i32 },
    Unavailable { reason: Reason },
}
#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum Army {
    Null {},
    Measured { kind: u32, id: u32 },
    Unavailable { reason: Reason },
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Carrier {
    schema: u32,
    experience: Scalar,
    current_hp: Scalar,
    maximum_hp: Scalar,
    template_count: Count,
    army: Army,
    country_operand: CountryOperand,
}
fn parse(v: &Value) -> Result<TrainingScalars, DivisionTrainingRefusal> {
    let invalid = |s: &str| DivisionTrainingRefusal::Malformed(s.into());
    let c = Carrier::deserialize(v).map_err(|e| invalid(&e.to_string()))?;
    if c.schema != 1 {
        return Err(invalid("schema"));
    }
    let template_count = match c.template_count {
        Count::Measured {
            entry_count,
            sum_raw,
        } => {
            if entry_count <= 0 && sum_raw != 0 {
                return Err(invalid("nonpositive entry count with nonzero sum"));
            }
            TrainingObservation::Measured(TrainingCount {
                entry_count,
                sum_raw,
            })
        }
        Count::Unavailable { reason } => TrainingObservation::Unavailable(reason.into()),
    };
    let army = match c.army {
        Army::Null {} => TrainingObservation::Measured(None),
        Army::Measured { kind, id } => {
            if id == 0 {
                return Err(invalid("zero army identity"));
            }
            TrainingObservation::Measured(Some(TrainingArmy { kind, id }))
        }
        Army::Unavailable { reason } => TrainingObservation::Unavailable(reason.into()),
    };
    let country_operand = match c.country_operand {
        CountryOperand::Measured { raw_i32 } => TrainingObservation::Measured(raw_i32),
        CountryOperand::Unavailable { reason } => TrainingObservation::Unavailable(reason.into()),
    };
    Ok(TrainingScalars {
        experience: c.experience.into(),
        current_hp: c.current_hp.into(),
        maximum_hp: c.maximum_hp.into(),
        template_count,
        army,
        country_operand,
    })
}
impl WorldFrame {
    /// The returned signed country operand is a raw handle, not an admitted country tag or owner.
    /// Same-hour D20 historical faults remain refusals even if a raw baseline later recovers.
    /// M4 consumes `template_count.sum_raw`; HP operands remain independent.
    /// A present army is only a raw identity; its flags require a current D98 join.
    pub fn division_training_scalars(
        &self,
        id: u32,
        kind: u32,
    ) -> Result<TrainingScalars, DivisionTrainingRefusal> {
        use DivisionTrainingRefusal as R;
        match crate::world::rederive_collector_domain_problems(self.raw()) {
            Err(()) => return Err(R::MalformedLedger),
            Ok(Some(problems)) => {
                for p in problems {
                    if p.domain == 20 && !crate::world::domain_problem_is_per_family_accounted(&p) {
                        return Err(R::DomainProblem(p.problem));
                    }
                }
            }
            Ok(None) => {}
        }
        if self.division_units_status() != ViewStatus::Present
            || self
                .division_units()
                .iter()
                .filter(|r| {
                    r.raw_division_id == id
                        && matches!(r.raw_division_type,Leaf::Measured(t) if t==kind)
                })
                .count()
                != 1
        {
            return Err(R::CurrentIdentityUnavailable);
        }
        let rows = self
            .raw()
            .get("divisions")
            .and_then(Value::as_array)
            .ok_or(R::CurrentIdentityUnavailable)?;
        let mut found = rows.iter().filter(|r| {
            r.get("div_id").and_then(Value::as_u64) == Some(u64::from(id))
                && r.get("div_type").and_then(Value::as_u64) == Some(u64::from(kind))
        });
        let row = found.next().ok_or(R::CurrentIdentityUnavailable)?;
        if found.next().is_some() {
            return Err(R::CurrentIdentityUnavailable);
        }
        match row.get("training_scalars") {
            None => Err(R::Missing),
            Some(Value::Null) => Err(R::Unobtained),
            Some(v) => parse(v),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn raw(v: TrainingScalars) -> Value {
        json!({"divisions":[{"div_id":7,"div_type":4713,"div_uid":"7:4713","training_scalars":serde_json::from_str::<Value>(&v.json()).unwrap()}]})
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
    #[test]
    fn actual_world_keeps_raw_widths_and_count_sum_distinct() {
        let value = TrainingScalars {
            experience: TrainingObservation::Measured(i64::MAX),
            current_hp: TrainingObservation::Measured(i64::MIN),
            maximum_hp: TrainingObservation::Measured((1i64 << 53) + 1),
            template_count: TrainingObservation::Measured(TrainingCount {
                entry_count: 3,
                sum_raw: -7,
            }),
            army: TrainingObservation::Measured(None),
            country_operand: TrainingObservation::Measured(i32::MIN),
        };
        assert_eq!(
            world(raw(value), 7).division_training_scalars(7, 4713),
            Ok(value)
        );
        assert_eq!(
            world(raw(value), 7).division_training_scalars(7, 51),
            Err(DivisionTrainingRefusal::CurrentIdentityUnavailable)
        );
    }
    #[test]
    fn strict_missing_null_old_fields_and_filler_are_not_defaults() {
        let value = TrainingScalars::default();
        let clean = raw(value);
        let mut missing = clean.clone();
        missing["divisions"][0]
            .as_object_mut()
            .unwrap()
            .remove("training_scalars");
        assert_eq!(
            world(missing, 7).division_training_scalars(7, 4713),
            Err(DivisionTrainingRefusal::Missing)
        );
        let mut null = clean.clone();
        null["divisions"][0]["training_scalars"] = Value::Null;
        assert_eq!(
            world(null, 7).division_training_scalars(7, 4713),
            Err(DivisionTrainingRefusal::Unobtained)
        );
        for bad in [
            json!({"status":"measured","raw_i32":7}),
            json!({"status":"unavailable","reason":"unreadable","raw_i64":0}),
            json!({"status":"unavailable","reason":"future"}),
        ] {
            let mut raw = clean.clone();
            raw["divisions"][0]["training_scalars"]["experience"] = bad;
            assert!(world(raw, 7).division_training_scalars(7, 4713).is_err());
        }
        let mut bad = clean;
        bad["divisions"][0]["training_scalars"]["template_count"] =
            json!({"status":"measured","entry_count":0,"sum_raw":1});
        assert!(world(bad, 7).division_training_scalars(7, 4713).is_err());

        let carrier: Value = serde_json::from_str(&value.json()).unwrap();
        for field in [
            "experience",
            "current_hp",
            "maximum_hp",
            "template_count",
            "army",
            "country_operand",
        ] {
            for reason in [
                json!({"unreadable":null}),
                Value::Null,
                json!(0),
                json!("future"),
            ] {
                let mut bad = carrier.clone();
                bad[field] = json!({"status":"unavailable","reason":reason});
                assert!(
                    matches!(parse(&bad), Err(DivisionTrainingRefusal::Malformed(_))),
                    "{field}: {bad}"
                );
            }
            let mut missing = carrier.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(matches!(
                parse(&missing),
                Err(DivisionTrainingRefusal::Malformed(_))
            ));
            let mut null = carrier.clone();
            null[field] = Value::Null;
            assert!(matches!(
                parse(&null),
                Err(DivisionTrainingRefusal::Malformed(_))
            ));
        }
        for (field, bad) in [
            (
                "experience",
                json!({"status":"measured","raw_i64":u64::MAX}),
            ),
            ("current_hp", json!({"status":"measured","raw_i64":1.5})),
            ("maximum_hp", json!({"status":"measured","raw_i64":"7"})),
            (
                "template_count",
                json!({"status":"measured","entry_count":-1,"sum_raw":1}),
            ),
            (
                "template_count",
                json!({"status":"measured","entry_count":2147483648i64,"sum_raw":0}),
            ),
            (
                "template_count",
                json!({"status":"measured","entry_count":1,"sum_raw":2147483648i64}),
            ),
            ("army", json!({"status":"null","id":0})),
            ("army", json!({"status":"measured","kind":53,"id":0})),
            ("army", json!({"status":"measured","kind":-1,"id":7})),
            (
                "army",
                json!({"status":"measured","kind":53,"id":4294967296i64}),
            ),
            (
                "country_operand",
                json!({"status":"measured","raw_i32":2147483648i64}),
            ),
            (
                "country_operand",
                json!({"status":"measured","raw_i32":"DUP"}),
            ),
        ] {
            let mut invalid = carrier.clone();
            invalid[field] = bad;
            assert!(
                matches!(parse(&invalid), Err(DivisionTrainingRefusal::Malformed(_))),
                "{field}: {invalid}"
            );
        }
        for schema in [json!(0), json!(2), Value::Null] {
            let mut bad = carrier.clone();
            bad["schema"] = schema;
            assert!(matches!(
                parse(&bad),
                Err(DivisionTrainingRefusal::Malformed(_))
            ));
        }
        let mut bad = carrier;
        bad["future"] = json!(0);
        assert!(matches!(
            parse(&bad),
            Err(DivisionTrainingRefusal::Malformed(_))
        ));
    }

    #[test]
    fn current_identity_and_status_cannot_be_supplied_by_another_row() {
        let clean = raw(TrainingScalars::default());
        for rows in [Value::Null, json!({}), json!([]), json!([null])] {
            let mut bad = clean.clone();
            bad["divisions"] = rows;
            assert_eq!(
                world(bad, 7).division_training_scalars(7, 4713),
                Err(DivisionTrainingRefusal::CurrentIdentityUnavailable)
            );
        }
        for (field, value) in [
            ("div_id", json!(0)),
            ("div_id", json!("7")),
            ("div_id", json!(8)),
            ("div_type", json!(51)),
            ("div_type", Value::Null),
            ("div_type", json!(4294967296i64)),
        ] {
            let mut bad = clean.clone();
            bad["divisions"][0][field] = value;
            assert_eq!(
                world(bad, 7).division_training_scalars(7, 4713),
                Err(DivisionTrainingRefusal::CurrentIdentityUnavailable)
            );
        }
        let mut duplicate = clean.clone();
        duplicate["divisions"]
            .as_array_mut()
            .unwrap()
            .push(clean["divisions"][0].clone());
        assert_eq!(
            world(duplicate, 7).division_training_scalars(7, 4713),
            Err(DivisionTrainingRefusal::CurrentIdentityUnavailable)
        );
        let mut other_kind = clean.clone();
        let mut other = clean["divisions"][0].clone();
        other["div_type"] = json!(51);
        other["training_scalars"] = Value::Null;
        other_kind["divisions"].as_array_mut().unwrap().push(other);
        let frame = world(other_kind, 7);
        assert_eq!(
            frame.division_training_scalars(7, 4713),
            Ok(TrainingScalars::default())
        );
        assert_eq!(
            frame.division_training_scalars(7, 51),
            Err(DivisionTrainingRefusal::Unobtained)
        );
    }

    #[test]
    fn independent_observations_preserve_reasons_and_raw_country_operand() {
        use TrainingObservation::{Measured, Unavailable};
        for reason in [
            TrainingRefusal::NotObserved,
            TrainingRefusal::Unreadable,
            TrainingRefusal::InvalidIdentity,
            TrainingRefusal::Changed,
            TrainingRefusal::BoundExceeded,
        ] {
            for army in [
                Measured(None),
                Measured(Some(TrainingArmy { kind: 53, id: 7 })),
                Unavailable(reason),
            ] {
                let value = TrainingScalars {
                    experience: Unavailable(reason),
                    current_hp: Measured(i64::MIN),
                    maximum_hp: Unavailable(reason),
                    template_count: Measured(TrainingCount {
                        entry_count: i32::MIN,
                        sum_raw: 0,
                    }),
                    army,
                    country_operand: Measured(-7),
                };
                let mut source = raw(value);
                // Root country admission may refuse the country-composed members.
                // It cannot turn this numeric operand into an owner/TAG or consume D98.
                source["countries"] = json!({"DUP":{"_r2_tag_db_id":441}});
                source["tag_db_id_index"] = json!({"country_composition_version":1});
                source["divisions"][0]["owner"] = json!("DUP");
                assert_eq!(
                    world(source, 7).division_training_scalars(7, 4713),
                    Ok(value)
                );
                let reverse = TrainingScalars {
                    experience: Measured((1i64 << 53) + 1),
                    current_hp: Unavailable(reason),
                    maximum_hp: Measured(i64::MAX),
                    template_count: Unavailable(reason),
                    army,
                    country_operand: Unavailable(reason),
                };
                assert_eq!(
                    world(raw(reverse), 7).division_training_scalars(7, 4713),
                    Ok(reverse)
                );
            }
        }
    }
    #[test]
    fn historical_d20_and_malformed_ledger_refuse_clean_next_hour_recovers() {
        let mut value = TrainingScalars::default();
        value.experience = TrainingObservation::Measured(7);
        let clean = raw(value);
        let mut refused = clean.clone();
        refused["provenance"] = json!({"collector":{"domain_problems":[{"domain_id":20,"problem":"malformed_payload"}]}});
        assert!(matches!(
            world(refused, 7).division_training_scalars(7, 4713),
            Err(DivisionTrainingRefusal::DomainProblem(_))
        ));
        let mut malformed = clean.clone();
        malformed["provenance"] = json!({"collector":{"domain_problems":42}});
        assert_eq!(
            world(malformed, 7).division_training_scalars(7, 4713),
            Err(DivisionTrainingRefusal::MalformedLedger)
        );
        for problem in [
            "partial_data",
            "family_gap",
            "source_unavailable",
            "compact_status_failure",
        ] {
            let mut refused = clean.clone();
            refused["provenance"] =
                json!({"collector":{"domain_problems":[{"domain_id":20,"problem":problem}]}});
            assert_eq!(
                world(refused, 7).division_training_scalars(7, 4713),
                Err(DivisionTrainingRefusal::DomainProblem(problem.into()))
            );
        }
        let mut unrelated = clean.clone();
        unrelated["provenance"] = json!({"collector":{"domain_problems":[{"domain_id":98,"problem":"source_unavailable"}]}});
        assert_eq!(
            world(unrelated, 7).division_training_scalars(7, 4713),
            Ok(value)
        );
        assert_eq!(
            world(clean, 8).division_training_scalars(7, 4713),
            Ok(value)
        );
    }

    #[test]
    #[ignore = "requires hash-bound verbatim D2RA sampler/publisher/fold output"]
    fn actual_sampler_fold_stages_preserve_fields_and_historical_refusal() {
        use sha2::{Digest, Sha256};
        let path = std::env::var_os("BATTLE_PLAN_D2RA_JOINT_INPUT").or_else(|| std::env::var_os("HOI4_D2RA_JOINT_INPUT")).expect("native output required");
        assert!(std::fs::metadata(&path).unwrap().len() <= 1024 * 1024);
        let bytes = std::fs::read(path).unwrap();
        assert!(bytes.len() <= 1024 * 1024);
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            std::env::var("BATTLE_PLAN_D2RA_JOINT_SHA256").or_else(|_| std::env::var("HOI4_D2RA_JOINT_SHA256")).unwrap()
        );
        let artifact: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(artifact["schema"], 1);
        let stages = artifact["stages"].as_array().unwrap();
        assert_eq!(stages.len(), 6);
        for (i, stage) in stages.iter().enumerate() {
            let value = &stage["snapshot"];
            assert_eq!(
                format!("{:x}", Sha256::digest(serde_json::to_vec(value).unwrap())),
                stage["snapshot_sha256"].as_str().unwrap()
            );
            let hex = stage["raw_h4cr_hex"].as_str().unwrap();
            assert_eq!(hex.len() % 2, 0);
            let raw: Vec<u8> = hex
                .as_bytes()
                .chunks_exact(2)
                .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
                .collect();
            assert_eq!(
                format!("{:x}", Sha256::digest(&raw)),
                stage["raw_h4cr_sha256"].as_str().unwrap()
            );
            let snapshot = snapshot::Snapshot::from_world_value(value.clone()).unwrap();
            let expected = snapshot.rest.clone();
            let raw = crate::RawFrame {
                hour_index: snapshot.hour_index.unwrap(),
                date: snapshot.date,
                save_version: snapshot.save_version,
                schema_version: snapshot.schema_version,
                rest: snapshot.rest.into(),
            };
            assert_eq!(
                raw.rest.to_map(),
                expected,
                "unmodified owning fold snapshot"
            );
            let frame =
                WorldFrame::from_raw_scheduled(raw, None, None, None, crate::BuildSchedule::Serial);
            assert_eq!(
                frame.raw().get("divisions"),
                value.get("divisions"),
                "D20 input remains verbatim after root country admission"
            );
            assert_eq!(
                frame.hour_index(),
                if i == 5 { 16_959_368 } else { 16_959_367 }
            );
            let result = frame.division_training_scalars(7, 4713);
            if (2..=4).contains(&i) {
                assert!(value["provenance"]["collector"]["domain_problems"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|problem| problem["domain_id"] == 20
                        && problem["problem"] == "malformed_payload"));
                assert_eq!(value.get("divisions"), Some(&Value::Null));
                assert_eq!(
                    result,
                    Err(DivisionTrainingRefusal::DomainProblem(
                        "malformed_payload".into()
                    )),
                    "stage {i}: the historical D20 fault must own the refusal"
                );
                continue;
            }
            let v = result.unwrap();
            assert_eq!(
                v.experience,
                if i == 1 {
                    TrainingObservation::Unavailable(TrainingRefusal::Unreadable)
                } else {
                    TrainingObservation::Measured(if i == 0 { i64::MAX } else { 19 })
                }
            );
            assert_eq!(v.current_hp, TrainingObservation::Measured(i64::MIN));
            assert_eq!(
                v.maximum_hp,
                TrainingObservation::Measured((1i64 << 53) + 1)
            );
            assert_eq!(
                v.template_count,
                TrainingObservation::Measured(TrainingCount {
                    entry_count: 3,
                    sum_raw: i32::MAX
                })
            );
            assert_eq!(v.army, TrainingObservation::Measured(None));
            assert_eq!(v.country_operand, TrainingObservation::Measured(-7));
        }
    }
}
