use crate::WorldFrame;
use serde_json::{Map, Value};
use portable_compact_stream::country_capital_slot::{CountryGroup, Operands};
pub use portable_compact_stream::country_capital_slot::{Outcome as CapitalSlotOutcome, Failure as CapitalSlotFailure, TemplateFailure as CapitalTemplateFailure};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesignerIconContextRefusal {
    CountryIdentity(String),
    MalformedCollectorLedger,
    DomainProblem(String),
    Missing(&'static str),
    Unobtained(&'static str),
    Malformed(&'static str),
    CapitalSlotIdentity,
    CapitalSlotHour { frame: i64, observation: i32 },
    CapitalSlotOperands,
    CapitalSlotContract(String),
}

fn signed_operand(value: Option<&Value>, field: &'static str) -> Result<i32, DesignerIconContextRefusal> {
    match value {
        None => Err(DesignerIconContextRefusal::Missing(field)),
        Some(Value::Null) => Err(DesignerIconContextRefusal::Unobtained(field)),
        Some(value) => value.as_i64().and_then(|raw| i32::try_from(raw).ok())
            .ok_or(DesignerIconContextRefusal::Malformed(field)),
    }
}

impl WorldFrame {
    /// The selected signed operand, before the icon resolver's state-range check.
    pub fn designer_effective_capital(&self, tag: &str) -> Result<i32, DesignerIconContextRefusal> {
        let (_, politics) = self.designer_politics(tag)?;
        let override_raw = signed_operand(politics.get("capital_override_raw"), "capital_override_raw")?;
        // country_capital_override engine evidence owns 1406ea260: only measured zero selects +1018.
        if override_raw != 0 { return Ok(override_raw); }
        signed_operand(politics.get("capital_state"), "capital_state")
    }

    fn designer_politics(&self, tag: &str) -> Result<(u32, &Map<String,Value>), DesignerIconContextRefusal> {
        let identity = self.country_identity_for_tag(tag).map_err(DesignerIconContextRefusal::CountryIdentity)?;
        match crate::world::rederive_collector_domain_problems(self.raw()) {
            Err(()) => return Err(DesignerIconContextRefusal::MalformedCollectorLedger),
            Ok(Some(problems)) => {
                if let Some(problem) = problems.iter().find(|problem| problem.domain == 14
                    && !crate::world::domain_problem_is_per_family_accounted(problem)) {
                    return Err(DesignerIconContextRefusal::DomainProblem(problem.problem.clone()));
                }
            }
            Ok(None) => {}
        }
        let country = self.raw().get("countries").and_then(Value::as_object)
            .and_then(|countries| countries.get(tag))
            .ok_or(DesignerIconContextRefusal::Missing("country"))?;
        let politics = match country.get("politics") {
            None => return Err(DesignerIconContextRefusal::Missing("politics")),
            Some(Value::Null) => return Err(DesignerIconContextRefusal::Unobtained("politics")),
            Some(value) => value.as_object().ok_or(DesignerIconContextRefusal::Malformed("politics"))?,
        };
        Ok((identity, politics))
    }

    pub fn designer_capital_slot(&self, tag: &str) -> Result<DesignerCapitalSlotObservation, DesignerIconContextRefusal> {
        let (identity, politics) = self.designer_politics(tag)?;
        let raw = match politics.get("capital_slot") {
            None => return Err(DesignerIconContextRefusal::Missing("capital_slot")),
            Some(Value::Null) => return Err(DesignerIconContextRefusal::Unobtained("capital_slot")),
            Some(value) => value,
        };
        let observation = parse_capital_slot(raw)?;
        if observation.group.tag_db_id != identity { return Err(DesignerIconContextRefusal::CapitalSlotIdentity); }
        if i64::from(observation.game_hour_raw) - i64::from(portable_admission::DATE_EPOCH_CONST) != self.hour_index() {
            return Err(DesignerIconContextRefusal::CapitalSlotHour { frame: self.hour_index(), observation: observation.game_hour_raw });
        }
        let ordinary = Operands {
            override_raw: nullable_operand(politics, "capital_override_raw")?,
            base_raw: nullable_operand(politics, "capital_state")?,
        };
        if observation.group.operands != ordinary { return Err(DesignerIconContextRefusal::CapitalSlotOperands); }
        Ok(observation)
    }

}


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DesignerCapitalSlotObservation {
    game_hour_raw: i32,
    observation_ordinal: u64,
    group: CountryGroup,
}

impl DesignerCapitalSlotObservation {
    pub fn game_hour_raw(self) -> i32 { self.game_hour_raw }
    /// Authenticated CPNI/CCS same-payload custody, not an independent JSON acquisition anchor.
    pub fn observation_ordinal(self) -> u64 { self.observation_ordinal }
    pub fn tag_db_id(self) -> u32 { self.group.tag_db_id }
    pub fn operands(self) -> Operands { self.group.operands }
    pub fn outcome(self) -> CapitalSlotOutcome { self.group.outcome }
}

fn exact_object<'a>(value: &'a Value, keys: &[&str]) -> Result<&'a Map<String,Value>, DesignerIconContextRefusal> {
    let object = value.as_object().ok_or(DesignerIconContextRefusal::Malformed("capital_slot"))?;
    if object.len() != keys.len() || !keys.iter().all(|key| object.contains_key(*key)) {
        return Err(DesignerIconContextRefusal::Malformed("capital_slot fields"));
    }
    Ok(object)
}

fn nullable_operand(object: &Map<String,Value>, field: &'static str) -> Result<Option<i32>, DesignerIconContextRefusal> {
    if object.get(field) == Some(&Value::Null) { Ok(None) }
    else { signed_operand(object.get(field), field).map(Some) }
}

fn parse_capital_slot(value: &Value) -> Result<DesignerCapitalSlotObservation, DesignerIconContextRefusal> {
    let object = exact_object(value, &["schema","game_hour_raw","observation_ordinal","tag_db_id","operands","outcome"])?;
    if object["schema"].as_u64() != Some(1) { return Err(DesignerIconContextRefusal::Malformed("capital_slot schema")); }
    let game_hour_raw = signed_operand(object.get("game_hour_raw"), "capital_slot game_hour_raw")?;
    let observation_ordinal = object["observation_ordinal"].as_u64().ok_or(DesignerIconContextRefusal::Malformed("capital_slot observation_ordinal"))?;
    let tag_db_id = object["tag_db_id"].as_u64().and_then(|n| u32::try_from(n).ok())
        .ok_or(DesignerIconContextRefusal::Malformed("capital_slot tag_db_id"))?;
    let operands = exact_object(&object["operands"], &["override_raw","base_raw"])?;
    let operands = Operands { override_raw: nullable_operand(operands,"override_raw")?, base_raw: nullable_operand(operands,"base_raw")? };
    let raw = &object["outcome"];
    let kind = raw.get("kind").and_then(Value::as_str).ok_or(DesignerIconContextRefusal::Malformed("capital_slot outcome"))?;
    let fields = match kind {
        "absent" | "non_positive" => exact_object(raw,&["kind"]),
        "failed" => exact_object(raw,&["kind","reason"]),
        "out_of_range" | "null_slot" => exact_object(raw,&["kind","slot_count"]),
        "present" => exact_object(raw,&["kind","slot_count","state_id","template_id"]),
        "template_unavailable" => exact_object(raw,&["kind","slot_count","state_id","reason"]),
        _ => return Err(DesignerIconContextRefusal::Malformed("capital_slot outcome kind")),
    }?;
    let count = || signed_operand(fields.get("slot_count"),"capital_slot slot_count");
    let state = || signed_operand(fields.get("state_id"),"capital_slot state_id");
    let outcome = match kind {
        "absent" => CapitalSlotOutcome::Absent,
        "non_positive" => CapitalSlotOutcome::NonPositive,
        "failed" => CapitalSlotOutcome::Failed(match fields["reason"].as_str() {
            Some("country_identity") => CapitalSlotFailure::CountryIdentity,
            Some("operand_unobtained") => CapitalSlotFailure::OperandUnobtained,
            Some("game_state_unreadable") => CapitalSlotFailure::GameStateUnreadable,
            Some("slot_header_unreadable") => CapitalSlotFailure::SlotHeaderUnreadable,
            Some("slot_unreadable") => CapitalSlotFailure::SlotUnreadable,
            Some("state_unreadable") => CapitalSlotFailure::StateUnreadable,
            Some("incoherent") => CapitalSlotFailure::Incoherent,
            Some("bounds") => CapitalSlotFailure::Bounds,
            _ => return Err(DesignerIconContextRefusal::Malformed("capital_slot failure reason")),
        }),
        "out_of_range" => CapitalSlotOutcome::OutOfRange { slot_count: count()? },
        "null_slot" => CapitalSlotOutcome::NullSlot { slot_count: count()? },
        "present" => CapitalSlotOutcome::Present { slot_count: count()?, state_id: state()?, template_id: signed_operand(fields.get("template_id"),"capital_slot template_id")? },
        "template_unavailable" => CapitalSlotOutcome::TemplateUnavailable { slot_count: count()?, state_id: state()?, reason: match fields["reason"].as_str() {
            Some("null_pointer") => CapitalTemplateFailure::NullPointer,
            Some("unreadable") => CapitalTemplateFailure::Unreadable,
            Some("database_unavailable") => CapitalTemplateFailure::DatabaseUnavailable,
            Some("membership_unverified") => CapitalTemplateFailure::MembershipUnverified,
            _ => return Err(DesignerIconContextRefusal::Malformed("capital_slot template reason")),
        } },
        _ => unreachable!(),
    };
    let group = CountryGroup { tag_db_id, operands, outcome };
    group.validate().map_err(DesignerIconContextRefusal::CapitalSlotContract)?;
    Ok(DesignerCapitalSlotObservation { game_hour_raw, observation_ordinal, group })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn raw() -> Value {
        json!({"tag_db_id_index":{"country_composition_version":2},
            "countries":{"GER":{"_r2_tag_db_id":7,"politics":{"capital_state":64,"capital_override_raw":112}},
                "ITA":{"_r2_tag_db_id":3}}})
    }

    fn frame(raw: Value, hour: i64) -> WorldFrame {
        WorldFrame::from_raw_scheduled(crate::source::RawFrame {
            hour_index:hour,date:None,save_version:None,schema_version:None,
            rest:raw.as_object().unwrap().clone().into(),
        }, None, None, None, crate::BuildSchedule::Serial)
    }

    #[test]
    fn nonzero_override_is_independent_of_current_and_keeps_signed_bits() {
        for raw_override in [112,64,-1,i32::MIN,i32::MAX] {
            let mut input=raw();input["countries"]["GER"]["politics"]["capital_override_raw"]=json!(raw_override);
            for current in [None,Some(Value::Null),Some(json!("unreadable")),Some(json!(999))] {
                let politics=input["countries"]["GER"]["politics"].as_object_mut().unwrap();
                match current { Some(value)=>{politics.insert("capital_state".into(),value);},None=>{politics.remove("capital_state");} }
                assert_eq!(frame(input.clone(),7).designer_effective_capital("GER"),Ok(raw_override));
            }
        }
    }

    #[test]
    fn only_measured_zero_authorizes_current_capital_lookup() {
        let mut input=raw();input["countries"]["GER"]["politics"]["capital_override_raw"]=json!(0);
        assert_eq!(frame(input.clone(),7).designer_effective_capital("GER"),Ok(64));
        input["countries"]["GER"]["politics"]["capital_state"]=json!(0);
        assert_eq!(frame(input.clone(),7).designer_effective_capital("GER"),Ok(0));
        input["countries"]["GER"]["politics"]["capital_state"]=Value::Null;
        assert_eq!(frame(input.clone(),7).designer_effective_capital("GER"),Err(DesignerIconContextRefusal::Unobtained("capital_state")));
        input["countries"]["GER"]["politics"]["capital_state"]=json!(64);
        input["countries"]["GER"]["politics"]["capital_override_raw"]=Value::Null;
        assert_eq!(frame(input.clone(),7).designer_effective_capital("GER"),Err(DesignerIconContextRefusal::Unobtained("capital_override_raw")));
        input["countries"]["GER"]["politics"].as_object_mut().unwrap().remove("capital_override_raw");
        input["countries"]["GER"]["identity"]=json!({"original_capital":112});
        assert_eq!(frame(input,7).designer_effective_capital("GER"),Err(DesignerIconContextRefusal::Missing("capital_override_raw")));
    }

    #[test]
    fn malformed_and_out_of_range_operands_never_become_zero() {
        for field in ["capital_override_raw","capital_state"] {
            for bad in [json!(true),json!("0"),json!(0.5),json!([]),json!({}),json!(i32::MAX as i64+1),json!(i32::MIN as i64-1),json!(u64::MAX)] {
                let mut input=raw();input["countries"]["GER"]["politics"]["capital_override_raw"]=json!(0);
                input["countries"]["GER"]["politics"][field]=bad;
                assert_eq!(frame(input,7).designer_effective_capital("GER"),Err(DesignerIconContextRefusal::Malformed(field)));
            }
        }
    }

    #[test]
    fn d14_problem_and_malformed_ledger_withdraw_retained_raw_and_clean_hour_recovers() {
        for problem in ["partial_data","compact_status_failure_rows_applied","refused_domain_data_repeat"] {
            let mut input=raw();input["provenance"]=json!({"collector":{"domain_problems":[{"domain_id":14,"problem":problem}]}});
            let rejected=frame(input,7);
            assert_eq!(rejected.raw()["countries"]["GER"]["politics"]["capital_override_raw"],json!(112));
            assert_eq!(rejected.designer_effective_capital("GER"),Err(DesignerIconContextRefusal::DomainProblem(problem.into())));
        }
        for provenance in [Value::Null,json!({"collector":false}),json!({"collector":{"domain_problems":null}})] {
            let mut input=raw();input["provenance"]=provenance;
            assert_eq!(frame(input,7).designer_effective_capital("GER"),Err(DesignerIconContextRefusal::MalformedCollectorLedger));
        }
        for hour in [7,8] { assert_eq!(frame(raw(),hour).designer_effective_capital("GER"),Ok(112)); }
        let mut input=raw();input["countries"]["GER"]["politics"]["capital_override_raw"]=json!(0);
        assert_eq!(frame(input,9).designer_effective_capital("GER"),Ok(64));
    }

    #[test]
    fn exact_country_witness_required_without_borrowing_another_country_or_unrelated_gap() {
        let mut input=raw();input["countries"]["GER"].as_object_mut().unwrap().remove("_r2_tag_db_id");
        let world=frame(input,7);
        assert_eq!(world.country_identity_for_tag("ITA"),Ok(3));
        assert!(matches!(world.designer_effective_capital("GER"),Err(DesignerIconContextRefusal::CountryIdentity(_))));
        assert!(matches!(frame(raw(),7).designer_effective_capital("UNKNOWN"),Err(DesignerIconContextRefusal::CountryIdentity(_))));
        let mut input=raw();input["countries"]["ITA"]["_r2_tag_db_id"]=json!(7);
        assert!(matches!(frame(input,7).designer_effective_capital("GER"),Err(DesignerIconContextRefusal::CountryIdentity(_))));
        let mut input=raw();input["provenance"]=json!({"collector":{"domain_problems":[{"domain_id":15,"problem":"partial_data"}]}});
        assert_eq!(frame(input,7).designer_effective_capital("GER"),Ok(112));
    }

    fn slot_input() -> Value {
        let mut input=raw();
        input["countries"]["GER"]["politics"]["capital_slot"]=json!({"schema":1,
            "game_hour_raw":43800006,"observation_ordinal":u64::MAX,"tag_db_id":7,
            "operands":{"override_raw":112,"base_raw":64},
            "outcome":{"kind":"present","slot_count":200,"state_id":73,"template_id":21}});
        input
    }

    #[test]
    fn capital_observation_retains_distinct_slot_state_template_and_ordinal_authority() {
        let observed=frame(slot_input(),7).designer_capital_slot("GER").unwrap();
        assert_eq!(observed.observation_ordinal(),u64::MAX);
        assert_eq!(observed.game_hour_raw(),43800006);
        assert_eq!(observed.tag_db_id(),7);
        assert_eq!(observed.operands(),Operands {override_raw:Some(112),base_raw:Some(64)});
        assert_eq!(observed.outcome(),CapitalSlotOutcome::Present {slot_count:200,state_id:73,template_id:21});
        let mut input=slot_input();
        input["countries"]["GER"]["politics"]["capital_state"]=Value::Null;
        input["countries"]["GER"]["politics"]["capital_slot"]["operands"]["base_raw"]=Value::Null;
        assert_eq!(frame(input,7).designer_capital_slot("GER").unwrap().outcome(),observed.outcome());
    }

    #[test]
    fn missing_null_and_explicit_absent_capital_observations_remain_distinct() {
        let mut input=slot_input();input["countries"]["GER"]["politics"].as_object_mut().unwrap().remove("capital_slot");
        assert_eq!(frame(input.clone(),7).designer_capital_slot("GER"),Err(DesignerIconContextRefusal::Missing("capital_slot")));
        input["countries"]["GER"]["politics"]["capital_slot"]=Value::Null;
        assert_eq!(frame(input,7).designer_capital_slot("GER"),Err(DesignerIconContextRefusal::Unobtained("capital_slot")));
        let mut input=slot_input();let politics=&mut input["countries"]["GER"]["politics"];
        politics["capital_override_raw"]=Value::Null;politics["capital_state"]=Value::Null;
        politics["capital_slot"]["operands"]=json!({"override_raw":null,"base_raw":null});
        politics["capital_slot"]["outcome"]=json!({"kind":"absent"});
        assert_eq!(frame(input,7).designer_capital_slot("GER").unwrap().outcome(),CapitalSlotOutcome::Absent);
    }

    #[test]
    fn capital_observation_requires_every_nullable_key_and_rejects_unknown_fields_and_kinds() {
        for key in ["schema","game_hour_raw","observation_ordinal","tag_db_id","operands","outcome"] {
            let mut input=slot_input();input["countries"]["GER"]["politics"]["capital_slot"].as_object_mut().unwrap().remove(key);
            assert!(frame(input,7).designer_capital_slot("GER").is_err(),"{key}");
        }
        for key in ["override_raw","base_raw"] {
            let mut input=slot_input();input["countries"]["GER"]["politics"]["capital_slot"]["operands"].as_object_mut().unwrap().remove(key);
            assert!(frame(input,7).designer_capital_slot("GER").is_err(),"{key}");
        }
        for pointer in ["/countries/GER/politics/capital_slot","/countries/GER/politics/capital_slot/operands","/countries/GER/politics/capital_slot/outcome"] {
            let mut input=slot_input();input.pointer_mut(pointer).unwrap().as_object_mut().unwrap().insert("unknown".into(),Value::Null);
            assert!(frame(input,7).designer_capital_slot("GER").is_err(),"{pointer}");
        }
        for value in [json!({"kind":"future"}),json!({"kind":"failed","reason":"future"}),
            json!({"kind":"template_unavailable","slot_count":200,"state_id":73,"reason":"future"}),
            json!({"kind":"null_slot","slot_count":112}),json!({"kind":"out_of_range","slot_count":113}),
            json!({"kind":"present","slot_count":200,"state_id":73,"template_id":-1})] {
            let mut input=slot_input();input["countries"]["GER"]["politics"]["capital_slot"]["outcome"]=value.clone();
            assert!(frame(input,7).designer_capital_slot("GER").is_err(),"{value}");
        }
    }

    #[test]
    fn all_capital_failures_preserve_closed_named_outcomes_without_fabricated_absence() {
        for (name,reason) in [("country_identity",CapitalSlotFailure::CountryIdentity),("operand_unobtained",CapitalSlotFailure::OperandUnobtained),
            ("game_state_unreadable",CapitalSlotFailure::GameStateUnreadable),("slot_header_unreadable",CapitalSlotFailure::SlotHeaderUnreadable),
            ("slot_unreadable",CapitalSlotFailure::SlotUnreadable),("state_unreadable",CapitalSlotFailure::StateUnreadable),
            ("incoherent",CapitalSlotFailure::Incoherent),("bounds",CapitalSlotFailure::Bounds)] {
            let mut input=slot_input();input["countries"]["GER"]["politics"]["capital_slot"]["outcome"]=json!({"kind":"failed","reason":name});
            assert_eq!(frame(input,7).designer_capital_slot("GER").unwrap().outcome(),CapitalSlotOutcome::Failed(reason));
        }
        for (name,reason) in [("null_pointer",CapitalTemplateFailure::NullPointer),("unreadable",CapitalTemplateFailure::Unreadable),
            ("database_unavailable",CapitalTemplateFailure::DatabaseUnavailable),("membership_unverified",CapitalTemplateFailure::MembershipUnverified)] {
            let mut input=slot_input();input["countries"]["GER"]["politics"]["capital_slot"]["outcome"]=json!({"kind":"template_unavailable","slot_count":200,"state_id":73,"reason":name});
            assert_eq!(frame(input,7).designer_capital_slot("GER").unwrap().outcome(),CapitalSlotOutcome::TemplateUnavailable {slot_count:200,state_id:73,reason});
        }
    }

    #[test]
    fn capital_slot_cannot_reuse_another_hour_country_or_operand_even_with_valid_codec_shape() {
        assert!(matches!(frame(slot_input(),8).designer_capital_slot("GER"),Err(DesignerIconContextRefusal::CapitalSlotHour {..})));
        let mut input=slot_input();input["countries"]["GER"]["politics"]["capital_slot"]["tag_db_id"]=json!(3);
        assert_eq!(frame(input,7).designer_capital_slot("GER"),Err(DesignerIconContextRefusal::CapitalSlotIdentity));
        for key in ["override_raw","base_raw"] {
            let mut input=slot_input();input["countries"]["GER"]["politics"]["capital_slot"]["operands"][key]=json!(111);
            assert_eq!(frame(input,7).designer_capital_slot("GER"),Err(DesignerIconContextRefusal::CapitalSlotOperands));
        }
        let mut input=slot_input();input["countries"]["GER"].as_object_mut().unwrap().remove("_r2_tag_db_id");
        assert!(matches!(frame(input,7).designer_capital_slot("GER"),Err(DesignerIconContextRefusal::CountryIdentity(_))));
    }

    #[test]
    fn capital_slot_honors_d14_and_malformed_ledger_without_blanking_unrelated_domains() {
        for problem in ["partial_data","refused_domain_data_repeat"] {
            let mut input=slot_input();input["provenance"]=json!({"collector":{"domain_problems":[{"domain_id":14,"problem":problem}]}});
            assert_eq!(frame(input,7).designer_capital_slot("GER"),Err(DesignerIconContextRefusal::DomainProblem(problem.into())));
        }
        let mut input=slot_input();input["provenance"]=json!({"collector":{"domain_problems":null}});
        assert_eq!(frame(input,7).designer_capital_slot("GER"),Err(DesignerIconContextRefusal::MalformedCollectorLedger));
        let mut input=slot_input();input["provenance"]=json!({"collector":{"domain_problems":[{"domain_id":15,"problem":"partial_data"}]}});
        assert!(frame(input,7).designer_capital_slot("GER").is_ok());
    }

}
