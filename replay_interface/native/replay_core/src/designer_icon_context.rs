use crate::WorldFrame;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesignerIconContextRefusal {
    CountryIdentity(String),
    MalformedCollectorLedger,
    DomainProblem(String),
    Missing(&'static str),
    Unobtained(&'static str),
    Malformed(&'static str),
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
        self.country_identity_for_tag(tag).map_err(DesignerIconContextRefusal::CountryIdentity)?;
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
        let override_raw = signed_operand(politics.get("capital_override_raw"), "capital_override_raw")?;
        // country_capital_override engine evidence owns 1406ea260: only measured zero selects +1018.
        if override_raw != 0 { return Ok(override_raw); }
        signed_operand(politics.get("capital_state"), "capital_state")
    }
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
}
