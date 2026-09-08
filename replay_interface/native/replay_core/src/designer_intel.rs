use crate::design_catalog::NativeDesignerInputs;
use crate::{Leaf, WorldFrame};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesignerIntelRefusal {
    Absent,
    Unobtained,
    InvalidCarrier(String),
    HourMismatch { frame: i64, catalog: u64 },
    CensusUnavailable,
    CollectorDomainProblem(String),
    MalformedCollectorLedger,
    GlobalUnavailable(String),
    CountryIdentity(String),
    PhysicalSlotUnavailable { slot: u32, status: String },
    PhysicalSlotMismatch { slot: u32, expected_tag_db_id: u32, observed_tag_db_id: Option<u32> },
    OwnerUnavailable(String),
    UncoveredPhysicalSlot { slot: u32, copied_prefix_slots: u32 },
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawDesignerIntel {
    schema: u32,
    pub census: RawDesignerCensus,
    status: RawIntelStatus,
    #[serde(deserialize_with = "nullable")]
    pub cause: Option<String>,
    pub owners: Vec<RawDesignerIntelOwner>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawDesignerCensus {
    pub country_count: u32,
    pub header_stable: bool,
    pub allocation_complete: bool,
    pub slots: Vec<RawDesignerCountrySlot>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawDesignerCountrySlot {
    pub physical_slot: u32,
    pub status: String,
    #[serde(deserialize_with = "nullable")]
    pub tag_db_id: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawDesignerIntelOwner {
    pub owner_physical_slot: u32,
    pub owner_tag_db_id: u32,
    #[serde(deserialize_with = "nullable")]
    pub table_row_count: Option<u32>,
    #[serde(deserialize_with = "nullable")]
    pub copied_prefix_slots: Option<u32>,
    status: RawIntelStatus,
    #[serde(deserialize_with = "nullable")]
    pub cause: Option<String>,
    pub lane2_raw: Vec<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RawIntelStatus { Measured, Unavailable }

fn nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where D: Deserializer<'de>, T: Deserialize<'de> {
    Option::<T>::deserialize(deserializer)
}

impl RawDesignerIntel {
    fn parse(value: &Value) -> Result<Self, DesignerIntelRefusal> {
        let raw = Self::deserialize(value).map_err(|error| DesignerIntelRefusal::InvalidCarrier(error.to_string()))?;
        raw.validate()?;
        Ok(raw)
    }

    fn validate(&self) -> Result<(), DesignerIntelRefusal> {
        let invalid = |reason: &str| DesignerIntelRefusal::InvalidCarrier(reason.into());
        if self.schema != 1 { return Err(invalid("intel_raw_lane2.schema")) }
        if self.census.country_count > MAX_COUNTRY_SLOTS
            || self.census.slots.len() != self.census.country_count as usize {
            return Err(invalid("intel_raw_lane2.census coverage"));
        }
        let mut accepted = BTreeSet::new();
        let mut ambiguous = BTreeSet::new();
        let mut accepted_slots = Vec::new();
        for (index, slot) in self.census.slots.iter().enumerate() {
            if slot.physical_slot as usize != index || !SLOT_STATUSES.contains(&slot.status.as_str()) {
                return Err(invalid("intel_raw_lane2.census physical order/status"));
            }
            let needs_tag = matches!(slot.status.as_str(), "accepted" | "ambiguous_duplicate_tag");
            if needs_tag != slot.tag_db_id.is_some()
                || slot.tag_db_id.is_some_and(|tag| tag == 0 || tag > MAX_TAG_DB_ID) {
                return Err(invalid("intel_raw_lane2.census tag identity"));
            }
            if slot.status == "accepted" {
                let tag = slot.tag_db_id.unwrap();
                if !accepted.insert(tag) { return Err(invalid("duplicate accepted tag_db_id")) }
                accepted_slots.push((slot.physical_slot, tag));
            } else if slot.status == "ambiguous_duplicate_tag" {
                ambiguous.insert(slot.tag_db_id.unwrap());
            }
        }
        if !accepted.is_disjoint(&ambiguous) { return Err(invalid("accepted tag is also ambiguous")) }
        if !status_cause(self.status, self.cause.as_deref(), GLOBAL_CAUSES) {
            return Err(invalid("intel_raw_lane2.status/cause"));
        }
        if self.status == RawIntelStatus::Unavailable {
            if !self.owners.is_empty() { return Err(invalid("global refusal claims owners")) }
            return Ok(());
        }
        if self.owners.len() != accepted_slots.len() {
            return Err(invalid("measured raw family omits accepted owners"));
        }
        let mut raw_count = 0usize;
        for (owner, identity) in self.owners.iter().zip(accepted_slots) {
            if (owner.owner_physical_slot, owner.owner_tag_db_id) != identity {
                return Err(invalid("owner disagrees with ordered accepted physical census"));
            }
            if owner.table_row_count.is_some_and(|count| count == 0 || count > MAX_TABLE_ROWS) {
                return Err(invalid("owner table count outside bound"));
            }
            if !status_cause(owner.status, owner.cause.as_deref(), OWNER_CAUSES) {
                return Err(invalid("owner status/cause"));
            }
            match owner.status {
                RawIntelStatus::Measured => {
                    let count = owner.table_row_count.ok_or_else(|| invalid("measured owner table count unknown"))?;
                    let prefix = owner.copied_prefix_slots.ok_or_else(|| invalid("measured owner prefix unknown"))?;
                    if prefix > count || owner.lane2_raw.len() != prefix as usize {
                        return Err(invalid("owner raw prefix coverage mismatch"));
                    }
                }
                RawIntelStatus::Unavailable => {
                    if owner.copied_prefix_slots.is_some() || !owner.lane2_raw.is_empty() {
                        return Err(invalid("unavailable owner claims copied prefix"));
                    }
                    let observed_count = match owner.cause.as_deref().unwrap() {
                        "no_fog_skip" => owner.table_row_count.is_some(),
                        "intel_table_span_unreadable" | "copy_budget_exceeded" => true,
                        _ => false,
                    };
                    if owner.table_row_count.is_some() != observed_count {
                        return Err(invalid("owner refusal disagrees with table count observation"));
                    }
                }
            }
            raw_count = raw_count.checked_add(owner.lane2_raw.len())
                .ok_or_else(|| invalid("raw lane count overflow"))?;
            if raw_count > MAX_RAW_VALUES { return Err(invalid("raw lane copy bound")) }
        }
        Ok(())
    }

    fn lane2(&self, owner_slot: u32, owner_id: u32, target_slot: u32, target_id: u32) -> Result<i64, DesignerIntelRefusal> {
        if !self.census.header_stable || !self.census.allocation_complete {
            return Err(DesignerIntelRefusal::CensusUnavailable);
        }
        if self.status == RawIntelStatus::Unavailable {
            return Err(DesignerIntelRefusal::GlobalUnavailable(self.cause.clone().unwrap()));
        }
        self.join_slot(owner_slot, owner_id)?;
        self.join_slot(target_slot, target_id)?;
        let owner = self.owners.iter().find(|row| row.owner_physical_slot == owner_slot)
            .ok_or_else(|| DesignerIntelRefusal::OwnerUnavailable("owner not observed".into()))?;
        if owner.status == RawIntelStatus::Unavailable {
            return Err(DesignerIntelRefusal::OwnerUnavailable(owner.cause.clone().unwrap()));
        }
        owner.lane2_raw.get(target_slot as usize).copied().ok_or_else(|| {
            DesignerIntelRefusal::UncoveredPhysicalSlot { slot: target_slot, copied_prefix_slots: owner.copied_prefix_slots.unwrap() }
        })
    }

    fn join_slot(&self, physical_slot: u32, tag_db_id: u32) -> Result<(), DesignerIntelRefusal> {
        let slot = self.census.slots.get(physical_slot as usize)
            .ok_or_else(|| DesignerIntelRefusal::PhysicalSlotUnavailable { slot: physical_slot, status: "outside_census".into() })?;
        if slot.status != "accepted" {
            return Err(DesignerIntelRefusal::PhysicalSlotUnavailable { slot: physical_slot, status: slot.status.clone() });
        }
        if slot.tag_db_id != Some(tag_db_id) {
            return Err(DesignerIntelRefusal::PhysicalSlotMismatch {
                slot: physical_slot, expected_tag_db_id: tag_db_id, observed_tag_db_id: slot.tag_db_id,
            });
        }
        Ok(())
    }
}

// D53 v4 producer bounds: compact_contract build_profile and compact record envelope.
const MAX_COUNTRY_SLOTS: u32 = 65_536;
const MAX_TABLE_ROWS: u32 = 8_192;
const MAX_TAG_DB_ID: u32 = i32::MAX as u32 - 1;
const MAX_RAW_VALUES: usize = (1 << 28) / 18;
const SLOT_STATUSES: &[&str] = &["accepted", "vacant", "unreadable", "non_heap_pointer",
    "vtable_unreadable", "vtable_mismatch", "tag_unreadable", "tag_invalid", "ambiguous_duplicate_tag"];
const GLOBAL_CAUSES: &[&str] = &["countries_empty", "denominators_root_unavailable",
    "denominators_span_unreadable", "civilian_denominator_invalid", "army_denominator_invalid",
    "navy_denominator_invalid", "air_denominator_invalid", "no_fog_flag_unreadable", "family_off", "pair_budget_exceeded"];
const OWNER_CAUSES: &[&str] = &["no_fog_skip", "viewer_intel_table_unavailable",
    "intel_table_data_unavailable", "intel_table_count_invalid", "intel_table_span_unreadable", "copy_budget_exceeded"];

fn status_cause(status: RawIntelStatus, cause: Option<&str>, permitted: &[&str]) -> bool {
    match status {
        RawIntelStatus::Measured => cause.is_none(),
        RawIntelStatus::Unavailable => cause.is_some_and(|cause| permitted.contains(&cause)),
    }
}

impl WorldFrame {
    pub fn raw_designer_intel(&self) -> Result<Leaf<RawDesignerIntel>, DesignerIntelRefusal> {
        let Some(world) = self.raw().get("diplomacy_world") else { return Ok(Leaf::Absent) };
        if world.is_null() { return Ok(Leaf::Unobtained) }
        let world = world.as_object().ok_or_else(|| DesignerIntelRefusal::InvalidCarrier("diplomacy_world is not an object".into()))?;
        let Some(extra) = world.get("domain53_extra") else { return Ok(Leaf::Absent) };
        if extra.is_null() { return Ok(Leaf::Unobtained) }
        let extra = extra.as_object().ok_or_else(|| DesignerIntelRefusal::InvalidCarrier("domain53_extra is not an object".into()))?;
        match extra.get("intel_raw_lane2") {
            None => Ok(Leaf::Absent),
            Some(Value::Null) => Ok(Leaf::Unobtained),
            Some(value) => {
                match crate::world::rederive_collector_domain_problems(self.raw()) {
                    Err(()) => return Err(DesignerIntelRefusal::MalformedCollectorLedger),
                    Ok(Some(problems)) => {
                        if let Some(problem) = problems.iter().find(|problem| problem.domain == 53
                            && !crate::world::domain_problem_is_per_family_accounted(problem)) {
                            return Err(DesignerIntelRefusal::CollectorDomainProblem(problem.problem.clone()));
                        }
                    }
                    Ok(None) => {}
                }
                RawDesignerIntel::parse(value).map(Leaf::Measured)
            },
        }
    }

    pub fn designer_intel_lane2(&self, catalog: &NativeDesignerInputs, owner: &str, target: &str) -> Result<i64, DesignerIntelRefusal> {
        if u64::try_from(self.hour_index()).ok() != Some(catalog.hour) {
            return Err(DesignerIntelRefusal::HourMismatch { frame: self.hour_index(), catalog: catalog.hour });
        }
        let identity = |tag: &str| {
            let mut countries = catalog.countries.iter().filter(|country| country.tag == tag);
            let country = match (countries.next(), countries.next()) {
                (Some(country), None) => country,
                _ => return Err(DesignerIntelRefusal::CountryIdentity(format!("A27 country {tag:?} absent/ambiguous"))),
            };
            let slot = u32::try_from(country.registry_index).map_err(|_| DesignerIntelRefusal::CountryIdentity("A27 physical slot exceeds u32".into()))?;
            let id = self.country_identity_for_tag(tag).map_err(DesignerIntelRefusal::CountryIdentity)?;
            Ok((slot, id))
        };
        let (owner_slot, owner_id) = identity(owner)?;
        let (target_slot, target_id) = identity(target)?;
        match self.raw_designer_intel()? {
            Leaf::Absent => Err(DesignerIntelRefusal::Absent),
            Leaf::Unobtained => Err(DesignerIntelRefusal::Unobtained),
            Leaf::Measured(raw) => raw.lane2(owner_slot, owner_id, target_slot, target_id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn carrier() -> Value {
        json!({"schema":1,"census":{"country_count":4,"header_stable":true,"allocation_complete":true,
            "slots":[{"physical_slot":0,"status":"accepted","tag_db_id":800},
            {"physical_slot":1,"status":"vacant","tag_db_id":null},
            {"physical_slot":2,"status":"tag_invalid","tag_db_id":null},
            {"physical_slot":3,"status":"accepted","tag_db_id":27}]},
            "status":"measured","cause":null,"owners":[
            {"owner_physical_slot":0,"owner_tag_db_id":800,"table_row_count":8,"copied_prefix_slots":4,"status":"measured","cause":null,"lane2_raw":[0,i64::MIN,-7,i64::MAX]},
            {"owner_physical_slot":3,"owner_tag_db_id":27,"table_row_count":8,"copied_prefix_slots":1,"status":"measured","cause":null,"lane2_raw":[-5000000]}]})
    }

    #[test]
    fn physical_index_is_not_tag_id_or_compacted_accepted_order() {
        let raw = RawDesignerIntel::parse(&carrier()).unwrap();
        assert_eq!(raw.lane2(0,800,3,27).unwrap(), i64::MAX);
        assert_eq!(raw.lane2(3,27,0,800).unwrap(), -5000000);
        assert!(matches!(raw.lane2(0,800,1,27), Err(DesignerIntelRefusal::PhysicalSlotUnavailable { slot:1, .. })));
        assert!(matches!(raw.lane2(0,800,3,800), Err(DesignerIntelRefusal::PhysicalSlotMismatch { slot:3, .. })));
        assert!(matches!(raw.lane2(3,27,3,27), Err(DesignerIntelRefusal::UncoveredPhysicalSlot { slot:3, copied_prefix_slots:1 })));
        assert_eq!(raw.owners[0].lane2_raw, [0,i64::MIN,-7,i64::MAX]);
    }

    #[test]
    fn unknown_counts_and_refusals_remain_distinct_from_measured_zero() {
        let mut value = carrier();
        value["status"] = json!("unavailable"); value["cause"] = json!("army_denominator_invalid"); value["owners"] = json!([]);
        let raw = RawDesignerIntel::parse(&value).unwrap();
        assert_eq!(raw.census.country_count,4);
        assert_eq!(raw.lane2(0,800,3,27), Err(DesignerIntelRefusal::GlobalUnavailable("army_denominator_invalid".into())));
        let mut value = carrier();
        let owner = &mut value["owners"][0];
        owner["status"] = json!("unavailable"); owner["cause"] = json!("no_fog_skip"); owner["copied_prefix_slots"] = Value::Null; owner["lane2_raw"] = json!([]);
        let raw = RawDesignerIntel::parse(&value).unwrap();
        assert_eq!(raw.owners[0].table_row_count,Some(8));
        assert_eq!(raw.owners[0].copied_prefix_slots,None);
        assert_eq!(raw.lane2(0,800,3,27), Err(DesignerIntelRefusal::OwnerUnavailable("no_fog_skip".into())));
        value["owners"][0]["copied_prefix_slots"] = json!(0);
        assert!(RawDesignerIntel::parse(&value).is_err());
    }

    #[test]
    fn malformed_presence_census_and_prefix_are_rejected() {
        let value = carrier();
        let mut cases = Vec::new();
        let mut x=value.clone(); x["schema"]=json!(2); cases.push(x);
        let mut x=value.clone(); x["owners"][0].as_object_mut().unwrap().remove("table_row_count"); cases.push(x);
        let mut x=value.clone(); x["cause"]=json!("impossible"); cases.push(x);
        let mut x=value.clone(); x["owners"][0]["copied_prefix_slots"]=json!(3); cases.push(x);
        let mut x=value.clone(); x["census"]["slots"][3]["physical_slot"]=json!(1); cases.push(x);
        let mut x=value.clone(); x["census"]["slots"][3]["tag_db_id"]=json!(800); cases.push(x);
        let mut x=value.clone(); x["owners"][0]["owner_tag_db_id"]=json!(27); cases.push(x);
        let mut x=value.clone(); x["owners"].as_array_mut().unwrap().pop(); cases.push(x);
        for (index,x) in cases.iter().enumerate() { assert!(RawDesignerIntel::parse(x).is_err(),"case {index}"); }
        let mut x=value; x["census"]["header_stable"]=json!(false);
        assert_eq!(RawDesignerIntel::parse(&x).unwrap().lane2(0,800,3,27),Err(DesignerIntelRefusal::CensusUnavailable));
    }
    fn frame(hour: i64, raw: Option<Value>, target_id: u32) -> WorldFrame {
        frame_with_provenance(hour, raw, target_id, None)
    }

    fn frame_with_provenance(hour: i64, raw: Option<Value>, target_id: u32, provenance: Option<Value>) -> WorldFrame {
        let mut rest = json!({
            "tag_db_id_index": {"country_composition_version": 2},
            "countries": {"GER": {"_r2_tag_db_id": 800}, "FRA": {"_r2_tag_db_id": target_id}},
            "diplomacy_world": {"domain53_extra": {}}
        });
        if let Some(raw) = raw { rest["diplomacy_world"]["domain53_extra"]["intel_raw_lane2"] = raw; }
        if let Some(provenance) = provenance { rest["provenance"] = provenance; }
        WorldFrame::from_raw_scheduled(crate::source::RawFrame {
            hour_index: hour, date: None, save_version: None, schema_version: None,
            rest: rest.as_object().unwrap().clone().into(),
        }, None, None, None, crate::BuildSchedule::Serial)
    }

    fn catalog(hour: u64) -> NativeDesignerInputs {
        use crate::design_catalog::NativeDesignerCountryInputs;
        NativeDesignerInputs { templates: vec![], runtime_dlc_mask: crate::design_catalog::DesignU32Observation::Measured { raw_u32: 0 }, source_record_id: "exact-a27-hour".into(), hour, variants: vec![], countries: vec![
            NativeDesignerCountryInputs { registry_index: 3, tag: "FRA".into(), designer_pointer_order_ordinal: Some(0) },
            NativeDesignerCountryInputs { registry_index: 0, tag: "GER".into(), designer_pointer_order_ordinal: Some(1) },
        ] }
    }

    #[test]
    fn world_join_uses_exact_hour_d18_identity_and_a27_physical_slot() {
        let world = frame(7, Some(carrier()), 27);
        let mut inputs = catalog(7);
        assert_eq!(world.designer_intel_lane2(&inputs, "GER", "FRA"), Ok(i64::MAX));
        assert_eq!(world.designer_intel_lane2(&inputs, "FRA", "GER"), Ok(-5_000_000));
        inputs.hour = 8;
        assert_eq!(world.designer_intel_lane2(&inputs, "GER", "FRA"),
            Err(DesignerIntelRefusal::HourMismatch { frame: 7, catalog: 8 }));
        inputs.hour = 7;
        inputs.countries[0].registry_index = 1;
        assert!(matches!(world.designer_intel_lane2(&inputs, "GER", "FRA"),
            Err(DesignerIntelRefusal::PhysicalSlotUnavailable { slot: 1, .. })));
        let mismatched = frame(7, Some(carrier()), 28);
        assert_eq!(mismatched.designer_intel_lane2(&catalog(7), "GER", "FRA"),
            Err(DesignerIntelRefusal::PhysicalSlotMismatch { slot: 3, expected_tag_db_id: 28, observed_tag_db_id: Some(27) }));
        let mut duplicated = catalog(7);
        duplicated.countries.push(duplicated.countries[0].clone());
        assert!(matches!(world.designer_intel_lane2(&duplicated, "GER", "FRA"), Err(DesignerIntelRefusal::CountryIdentity(_))));
        assert!(matches!(world.designer_intel_lane2(&catalog(7), "GER", "ENG"), Err(DesignerIntelRefusal::CountryIdentity(_))));
    }

    #[test]
    fn each_hour_preserves_absent_unobtained_and_resumed_values() {
        let measured = frame(7, Some(carrier()), 27);
        assert_eq!(measured.designer_intel_lane2(&catalog(7), "GER", "FRA"), Ok(i64::MAX));
        let absent = frame(8, None, 27);
        assert!(matches!(absent.raw_designer_intel(), Ok(Leaf::Absent)));
        assert_eq!(absent.designer_intel_lane2(&catalog(8), "GER", "FRA"), Err(DesignerIntelRefusal::Absent));
        let gap = frame(9, Some(Value::Null), 27);
        assert!(matches!(gap.raw_designer_intel(), Ok(Leaf::Unobtained)));
        assert_eq!(gap.designer_intel_lane2(&catalog(9), "GER", "FRA"), Err(DesignerIntelRefusal::Unobtained));
        let mut next = carrier(); next["owners"][0]["lane2_raw"][3] = json!(0);
        assert_eq!(frame(10, Some(next), 27).designer_intel_lane2(&catalog(10), "GER", "FRA"), Ok(0));
    }

    #[test]
    fn closed_vocabulary_nullable_presence_and_refusal_observation_custody() {
        for cause in GLOBAL_CAUSES {
            let mut value = carrier(); value["status"] = json!("unavailable");
            value["cause"] = json!(cause); value["owners"] = json!([]);
            assert_eq!(RawDesignerIntel::parse(&value).unwrap().lane2(0,800,3,27),
                Err(DesignerIntelRefusal::GlobalUnavailable((*cause).into())));
        }
        for cause in OWNER_CAUSES {
            for count in [Value::Null, json!(8)] {
                let mut value = carrier(); let owner = &mut value["owners"][0];
                owner["status"] = json!("unavailable"); owner["cause"] = json!(cause);
                owner["table_row_count"] = count.clone(); owner["copied_prefix_slots"] = Value::Null; owner["lane2_raw"] = json!([]);
                let permitted = match *cause {
                    "no_fog_skip" => true,
                    "intel_table_span_unreadable" | "copy_budget_exceeded" => !count.is_null(),
                    _ => count.is_null(),
                };
                assert_eq!(RawDesignerIntel::parse(&value).is_ok(), permitted, "{cause} {count}");
            }
        }
        for path in ["/cause", "/census/slots/1/tag_db_id", "/owners/0/cause", "/owners/0/table_row_count", "/owners/0/copied_prefix_slots"] {
            let mut value = carrier();
            let (parent, key) = path.rsplit_once('/').unwrap();
            value.pointer_mut(parent).unwrap().as_object_mut().unwrap().remove(key);
            assert!(RawDesignerIntel::parse(&value).is_err(), "omitted {path}");
        }
        let mut cases = Vec::new();
        let mut x=carrier(); x["census"]["slots"][1]["status"]=json!("unknown_status"); cases.push(x);
        let mut x=carrier(); x["census"]["slots"][1]["tag_db_id"]=json!(800); cases.push(x);
        let mut x=carrier(); x["census"]["slots"][1]=json!({"physical_slot":1,"status":"ambiguous_duplicate_tag","tag_db_id":800}); cases.push(x);
        let mut x=carrier(); x["owners"].as_array_mut().unwrap().reverse(); cases.push(x);
        let mut x=carrier(); x["owners"][0]["table_row_count"]=json!(0); cases.push(x);
        let mut x=carrier(); x["owners"][0]["table_row_count"]=json!(MAX_TABLE_ROWS+1); cases.push(x);
        let mut x=carrier(); x["status"]=json!("unavailable"); x["cause"]=json!("family_off"); cases.push(x);
        let mut x=carrier(); x["status"]=json!("unavailable"); x["cause"]=json!("no_fog_skip"); x["owners"]=json!([]); cases.push(x);
        let mut x=carrier(); x["census"]["allocation_complete"]=json!(false); x["census"]["country_count"]=json!(5); cases.push(x);
        for (index, value) in cases.iter().enumerate() { assert!(RawDesignerIntel::parse(value).is_err(), "invalid case {index}"); }
        let mut zero=carrier(); zero["owners"][0]["copied_prefix_slots"]=json!(0); zero["owners"][0]["lane2_raw"]=json!([]);
        assert_eq!(RawDesignerIntel::parse(&zero).unwrap().lane2(0,800,3,27),
            Err(DesignerIntelRefusal::UncoveredPhysicalSlot { slot:3, copied_prefix_slots:0 }));
    }

    #[test]
    fn retained_raw_measurement_cannot_bypass_d53_problem_and_clean_recovery() {
        for problem in ["compact_status_failure", "source_unavailable", "partial_data", "family_gap", "domain_gap"] {
            let provenance = json!({"collector":{"domain_problems":[{"domain_id":53,"problem":problem}]}});
            let rejected = frame_with_provenance(7, Some(carrier()), 27, Some(provenance));
            assert_eq!(rejected.raw()["diplomacy_world"]["domain53_extra"]["intel_raw_lane2"], carrier());
            assert_eq!(rejected.raw_designer_intel(), Err(DesignerIntelRefusal::CollectorDomainProblem(problem.into())));
            assert_eq!(rejected.designer_intel_lane2(&catalog(7), "GER", "FRA"),
                Err(DesignerIntelRefusal::CollectorDomainProblem(problem.into())));
        }
        for hour in [7, 8] {
            let clean = frame_with_provenance(hour, Some(carrier()), 27,
                Some(json!({"collector":{"domain_problems":[]}})));
            assert_eq!(clean.designer_intel_lane2(&catalog(hour as u64), "GER", "FRA"), Ok(i64::MAX));
        }
    }

    #[test]
    fn malformed_collector_ledger_refuses_retained_raw_but_preserves_absence_and_null() {
        let mut provenances = vec![Value::Null, json!(false), json!({"collector":null}), json!({"collector":42})];
        for problems in [Value::Null, json!("bad"), json!([null]), json!([{}]),
            json!([{"domain_id":53}]), json!([{"domain_id":"53","problem":"failure"}]),
            json!([{"domain_id":53,"problem":" "}]), json!([{"domain_id":65536,"problem":"failure"}])] {
            provenances.push(json!({"collector":{"domain_problems":problems}}));
        }
        for provenance in provenances {
            let rejected = frame_with_provenance(7, Some(carrier()), 27, Some(provenance.clone()));
            assert_eq!(rejected.raw_designer_intel(), Err(DesignerIntelRefusal::MalformedCollectorLedger), "{provenance}");
            assert!(rejected.designer_intel_lane2(&catalog(7), "GER", "FRA").is_err(), "{provenance}");
            assert_eq!(frame_with_provenance(7, None, 27, Some(provenance.clone())).raw_designer_intel(), Ok(Leaf::Absent));
            assert_eq!(frame_with_provenance(7, Some(Value::Null), 27, Some(provenance)).raw_designer_intel(), Ok(Leaf::Unobtained));
        }
    }

    #[test]
    fn unrelated_problems_and_family_refusals_keep_their_own_scope() {
        for provenance in [json!({}), json!({"collector":{}}),
            json!({"collector":{"domain_problems":[{"domain_id":47,"problem":"partial_data"}]}}),
            json!({"collector":{"domain_problems":[{"domain_id":68,"problem":"source_unavailable"}]}})] {
            let world = frame_with_provenance(7, Some(carrier()), 27, Some(provenance.clone()));
            assert_eq!(world.designer_intel_lane2(&catalog(7), "GER", "FRA"), Ok(i64::MAX));
            let mut raw = carrier();
            raw["owners"][0]["status"] = json!("unavailable");
            raw["owners"][0]["cause"] = json!("no_fog_skip");
            raw["owners"][0]["copied_prefix_slots"] = Value::Null;
            raw["owners"][0]["lane2_raw"] = json!([]);
            let partial = frame_with_provenance(7, Some(raw), 27, Some(provenance));
            assert_eq!(partial.designer_intel_lane2(&catalog(7), "GER", "FRA"),
                Err(DesignerIntelRefusal::OwnerUnavailable("no_fog_skip".into())));
            assert_eq!(partial.designer_intel_lane2(&catalog(7), "FRA", "GER"), Ok(-5_000_000));
        }
    }

}
