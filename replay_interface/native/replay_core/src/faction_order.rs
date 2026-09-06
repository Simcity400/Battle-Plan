use crate::{Leaf, WorldFrame};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderedFactionMember {
    pub tag_db_id: u32,
    pub tag: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderedFactionRow {
    pub faction_id: u32,
    /// The leader is the first member; subsequent members retain producer order.
    pub members: Vec<OrderedFactionMember>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FactionOrderRefusal {
    InvalidCarrier(String),
    OrderUnwitnessed,
    MalformedCollectorLedger,
    CollectorDomainProblem(String),
    CountryIdentity(String),
    RosterUnavailable,
}

#[derive(Deserialize)]
struct WireRow {
    faction_id: u32,
    leader: String,
    leader_tag_db_id: u32,
    member_count: usize,
    members: Vec<String>,
    member_tag_db_ids: Vec<u32>,
}

impl WorldFrame {
    pub fn ordered_faction_roster(&self) -> Result<Leaf<Vec<OrderedFactionRow>>, FactionOrderRefusal> {
        let invalid = |field: &str| FactionOrderRefusal::InvalidCarrier(field.into());
        let Some(world) = self.raw().get("diplomacy_world") else { return Ok(Leaf::Absent) };
        if world.is_null() { return Ok(Leaf::Unobtained) }
        let world = world.as_object().ok_or_else(|| invalid("diplomacy_world"))?;
        let Some(extra) = world.get("domain53_extra") else { return Ok(Leaf::Absent) };
        if extra.is_null() { return Ok(Leaf::Unobtained) }
        let extra = extra.as_object().ok_or_else(|| invalid("domain53_extra"))?;
        let nonempty = match extra.get("faction_roster_status") {
            None if !extra.contains_key("faction_rows") && !extra.contains_key("faction_member_order_schema") => return Ok(Leaf::Absent),
            Some(Value::String(status)) if status == "unavailable" || status == "ambiguous_member_exclusivity" => return Ok(Leaf::Unobtained),
            Some(Value::String(status)) if status == "measured_empty" => false,
            Some(Value::String(status)) if status == "measured_nonempty" => true,
            _ => return Err(invalid("faction_roster_status")),
        };
        if extra.get("faction_member_order_schema").and_then(Value::as_u64) != Some(1) {
            return Err(FactionOrderRefusal::OrderUnwitnessed);
        }
        match crate::world::rederive_collector_domain_problems(self.raw()) {
            Err(()) => return Err(FactionOrderRefusal::MalformedCollectorLedger),
            Ok(Some(problems)) => {
                if let Some(problem) = problems.iter().find(|problem| problem.domain == 53
                    && !crate::world::domain_problem_is_per_family_accounted(problem)) {
                    return Err(FactionOrderRefusal::CollectorDomainProblem(problem.problem.clone()));
                }
            }
            Ok(None) => {}
        }
        let rows = extra.get("faction_rows").and_then(Value::as_array)
            .ok_or_else(|| invalid("faction_rows"))?;
        if rows.is_empty() == nonempty || rows.len() > 4096 {
            return Err(invalid("faction_rows count/status"));
        }
        let mut result = Vec::with_capacity(rows.len());
        let mut membership = BTreeMap::new();
        let mut all_ids = BTreeSet::new();
        let mut all_tags = BTreeSet::new();
        for value in rows {
            for field in ["members", "member_tag_db_ids"] {
                if !value.get(field).and_then(Value::as_array).is_some_and(|items| !items.is_empty() && items.len() <= 256) {
                    return Err(invalid("faction member array bound"));
                }
            }
            let row = WireRow::deserialize(value)
                .map_err(|error| FactionOrderRefusal::InvalidCarrier(error.to_string()))?;
            if row.faction_id == 0 || row.member_count == 0 || row.member_count > 256
                || row.member_count != row.members.len() || row.member_count != row.member_tag_db_ids.len() {
                return Err(invalid("faction row identity/count"));
            }
            if row.members.first() != Some(&row.leader)
                || row.member_tag_db_ids.first() != Some(&row.leader_tag_db_id) {
                return Err(invalid("faction leader is not first member"));
            }
            let mut members = Vec::with_capacity(row.member_count);
            for (tag, tag_db_id) in row.members.iter().zip(&row.member_tag_db_ids) {
                if tag.is_empty() || *tag_db_id == 0 || !all_ids.insert(*tag_db_id) || !all_tags.insert(tag.clone()) {
                    return Err(invalid("duplicate/invalid faction member"));
                }
                let observed = self.country_identity_for_tag(tag).map_err(FactionOrderRefusal::CountryIdentity)?;
                if observed != *tag_db_id {
                    return Err(FactionOrderRefusal::CountryIdentity(format!("{tag}: faction id {tag_db_id} differs from census {observed}")));
                }
                members.push(OrderedFactionMember { tag_db_id: *tag_db_id, tag: tag.clone() });
            }
            if membership.insert(row.faction_id.to_string(), row.members).is_some() {
                return Err(invalid("duplicate faction_id"));
            }
            result.push(OrderedFactionRow { faction_id: row.faction_id, members });
        }
        match &self.diplomacy().faction_members {
            Leaf::Measured(current) if current == &membership => Ok(Leaf::Measured(result)),
            _ => Err(FactionOrderRefusal::RosterUnavailable),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn raw() -> Value {
        json!({
            "tag_db_id_index": {"country_composition_version": 2},
            "countries": {"GER":{"_r2_tag_db_id":7},"ITA":{"_r2_tag_db_id":3},"FRA":{"_r2_tag_db_id":9}},
            "diplomacy_world":{"domain53_extra":{
                "faction_member_order_schema":1,"faction_roster_status":"measured_nonempty",
                "faction_rows":[{"faction_id":1,"leader":"GER","leader_tag_db_id":7,
                    "members":["GER","ITA","FRA"],"member_tag_db_ids":[7,3,9],"member_count":3}]
            }}
        })
    }

    fn frame(raw: Value) -> WorldFrame {
        WorldFrame::from_raw_scheduled(crate::source::RawFrame {
            hour_index:7,date:None,save_version:None,schema_version:None,
            rest:raw.as_object().unwrap().clone().into(),
        }, None, None, None, crate::BuildSchedule::Serial)
    }

    fn measured(raw: Value) -> Vec<OrderedFactionRow> {
        match frame(raw).ordered_faction_roster().unwrap() {
            Leaf::Measured(rows) => rows,
            other => panic!("expected measured roster, found {other:?}"),
        }
    }

    #[test]
    fn original_order_and_reordering_survive_even_when_membership_set_is_identical() {
        let original = measured(raw());
        assert_eq!(original[0].members.iter().map(|m|m.tag_db_id).collect::<Vec<_>>(), [7,3,9]);
        let mut reordered = raw();
        reordered["diplomacy_world"]["domain53_extra"]["faction_rows"][0]["members"] = json!(["GER","FRA","ITA"]);
        reordered["diplomacy_world"]["domain53_extra"]["faction_rows"][0]["member_tag_db_ids"] = json!([7,9,3]);
        let reordered = measured(reordered);
        assert_ne!(original,reordered);
        assert_eq!(original[0].members.iter().map(|m|m.tag_db_id).collect::<BTreeSet<_>>(),
            reordered[0].members.iter().map(|m|m.tag_db_id).collect());
    }

    #[test]
    fn markerless_sorted_rows_cannot_certify_order_even_if_membership_parser_accepts() {
        for marker in [None, Some(Value::Null), Some(json!(0)), Some(json!(2)), Some(json!("1"))] {
            let mut value = raw();
            let extra = value["diplomacy_world"]["domain53_extra"].as_object_mut().unwrap();
            match marker { Some(marker) => { extra.insert("faction_member_order_schema".into(),marker); }, None => { extra.remove("faction_member_order_schema"); } }
            let world = frame(value);
            assert!(matches!(world.diplomacy().faction_members,Leaf::Measured(_)));
            assert_eq!(world.ordered_faction_roster(),Err(FactionOrderRefusal::OrderUnwitnessed));
        }
    }

    #[test]
    fn ids_tags_counts_leader_and_exclusivity_are_checked_independently() {
        for (field,value) in [
            ("member_tag_db_ids",json!([7,9,3])),("member_tag_db_ids",json!([7,3])),
            ("member_tag_db_ids",json!([7,3,3])),("member_count",json!(2)),
            ("leader",json!("ITA")),("leader_tag_db_id",json!(3)),
            ("members",json!(["GER","ITA","ITA"])),("members",json!(["GER","ITA","UNKNOWN"])),
            ("faction_id",json!(0)),
        ] {
            let mut input=raw(); input["diplomacy_world"]["domain53_extra"]["faction_rows"][0][field]=value;
            assert!(frame(input).ordered_faction_roster().is_err(),"{field}");
        }
        for field in ["leader_tag_db_id","member_tag_db_ids","member_count"] {
            let mut input=raw(); input["diplomacy_world"]["domain53_extra"]["faction_rows"][0].as_object_mut().unwrap().remove(field);
            assert!(frame(input).ordered_faction_roster().is_err(),"missing {field}");
        }
        let mut input=raw();
        let mut second=input["diplomacy_world"]["domain53_extra"]["faction_rows"][0].clone();
        second["faction_id"]=json!(2);
        input["diplomacy_world"]["domain53_extra"]["faction_rows"].as_array_mut().unwrap().push(second);
        assert!(frame(input).ordered_faction_roster().is_err());
    }

    #[test]
    fn measured_empty_absent_unobtained_and_invalid_empty_are_distinct() {
        let mut input=raw();
        input["diplomacy_world"]["domain53_extra"]["faction_rows"]=json!([]);
        assert!(frame(input.clone()).ordered_faction_roster().is_err());
        input["diplomacy_world"]["domain53_extra"]["faction_roster_status"]=json!("measured_empty");
        assert!(measured(input.clone()).is_empty());
        input["diplomacy_world"]["domain53_extra"].as_object_mut().unwrap().remove("faction_member_order_schema");
        assert_eq!(frame(input.clone()).ordered_faction_roster(),Err(FactionOrderRefusal::OrderUnwitnessed));
        input["diplomacy_world"]["domain53_extra"]=json!({});
        assert_eq!(frame(input.clone()).ordered_faction_roster(),Ok(Leaf::Absent));
        input["diplomacy_world"]["domain53_extra"]=Value::Null;
        assert_eq!(frame(input.clone()).ordered_faction_roster(),Ok(Leaf::Unobtained));
        input["diplomacy_world"]["domain53_extra"]=json!({"faction_roster_status":"unavailable","faction_rows":[]});
        assert_eq!(frame(input).ordered_faction_roster(),Ok(Leaf::Unobtained));
    }

    #[test]
    fn raw_order_cannot_bypass_domain_refusal_or_malformed_ledger() {
        for problem in ["partial_data","family_gap","refused_domain_data_repeat"] {
            let mut input=raw();input["provenance"]=json!({"collector":{"domain_problems":[{"domain_id":53,"problem":problem}]}});
            assert_eq!(frame(input).ordered_faction_roster(),Err(FactionOrderRefusal::CollectorDomainProblem(problem.into())));
        }
        for provenance in [Value::Null,json!({"collector":null}),json!({"collector":{"domain_problems":null}})] {
            let mut input=raw(); input["provenance"]=provenance;
            assert_eq!(frame(input).ordered_faction_roster(),Err(FactionOrderRefusal::MalformedCollectorLedger));
        }
        assert_eq!(measured(raw())[0].members[0].tag,"GER");
    }

    #[test]
    fn country_census_identity_and_d18_gap_are_required_but_unrelated_d14_is_not() {
        let mut mismatch=raw(); mismatch["countries"]["GER"]["_r2_tag_db_id"]=json!(11);
        assert!(matches!(frame(mismatch).ordered_faction_roster(),Err(FactionOrderRefusal::CountryIdentity(_))));
        let mut absent=raw(); absent["countries"].as_object_mut().unwrap().remove("ITA");
        assert!(matches!(frame(absent).ordered_faction_roster(),Err(FactionOrderRefusal::CountryIdentity(_))));
        let mut input=raw();input["provenance"]=json!({"collector":{"domain_problems":[{"domain_id":18,"problem":"partial_data"}]}});
        assert_eq!(measured(input.clone())[0].members[1].tag_db_id,3);
        // D18 withdraws its own per-country identity witness on an unmeasured row.
        input["countries"]["ITA"].as_object_mut().unwrap().remove("_r2_tag_db_id");
        assert!(matches!(frame(input).ordered_faction_roster(),Err(FactionOrderRefusal::CountryIdentity(_))));
        let mut input=raw();input["provenance"]=json!({"collector":{"domain_problems":[{"domain_id":14,"problem":"partial_data"}]}});
        assert_eq!(measured(input)[0].members[0].tag_db_id,7);
    }
}
