use crate::{Leaf, ViewStatus, WorldFrame};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CountryRosterKind { Divisions, Fleets }
impl CountryRosterKind {
    fn key(self) -> &'static str { match self { Self::Divisions => "divisions", Self::Fleets => "fleets" } }
    fn domain(self) -> i64 { match self { Self::Divisions => 20, Self::Fleets => 28 } }
    fn slot_limit(self) -> usize { match self { Self::Divisions => 20_000, Self::Fleets => 10_000 } }
    fn total_limit(self) -> usize { match self { Self::Divisions => 20_000, Self::Fleets => 250_000 } }
    fn accepts(self, value: u32) -> bool { match self { Self::Divisions => matches!(value, 4713 | 51), Self::Fleets => value == 61 } }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CountryRosterRefusal {
    InvalidCarrier(String), HourMismatch, CountryIdentity(String), DomainProblem(String),
    MalformedLedger, Unavailable(u32), OwnerUnwitnessed, CurrentMembershipUnavailable,
    CurrentMembershipMismatch,
}
fn required_u32<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u32>, D::Error> {
    let value = Value::deserialize(d)?;
    if value.is_null() { Ok(None) } else {
        u32::deserialize(value).map(Some).map_err(serde::de::Error::custom)
    }
}

fn required_reference<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<CountryRosterReference>, D::Error> {
    let value = Value::deserialize(d)?;
    if value.is_null() { Ok(None) } else {
        CountryRosterReference::deserialize(value).map(Some).map_err(serde::de::Error::custom)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CountryRosterReference {
    pub native_type: u32,
    pub id: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CountryRosterSlot {
    pub ordinal: u32,
    #[serde(deserialize_with = "required_reference")]
    pub reference: Option<CountryRosterReference>,
    #[serde(deserialize_with = "required_u32")]
    pub failure: Option<u32>,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CountryRosterObservation {
    pub physical_slot: u32,
    #[serde(deserialize_with = "required_u32")]
    pub owner_tag_db_id: Option<u32>,
    pub acquisition: u8,
    #[serde(deserialize_with = "required_u32")]
    pub observed_count: Option<u32>,
    #[serde(deserialize_with = "required_u32")]
    pub failure: Option<u32>,
    pub slots: Vec<CountryRosterSlot>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Table {
    #[serde(deserialize_with = "required_u32")]
    country_count: Option<u32>,
    #[serde(deserialize_with = "required_u32")]
    failure: Option<u32>,
    countries: Vec<CountryRosterObservation>,
}
fn valid_cause(value: Option<u32>) -> bool { value.is_none_or(|v| (1..=20).contains(&v)) }

impl WorldFrame {
    pub fn country_roster_observation(
        &self, kind: CountryRosterKind, tag: &str,
    ) -> Result<Leaf<CountryRosterObservation>, CountryRosterRefusal> {
        let invalid = |field: &str| CountryRosterRefusal::InvalidCarrier(field.into());
        let Some(root) = self.raw().get("country_semantic_rosters") else { return Ok(Leaf::Absent) };
        if root.is_null() { return Ok(Leaf::Unobtained) }
        let root = root.as_object().ok_or_else(|| invalid("root"))?;
        if root.len() != 4 || root.get("schema").and_then(Value::as_u64) != Some(1) {
            return Err(invalid("root schema/fields"));
        }
        if root.get("hour_index").and_then(Value::as_i64) != Some(self.hour_index()) {
            return Err(CountryRosterRefusal::HourMismatch);
        }
        for key in ["divisions", "fleets"] { if !root.contains_key(key) { return Err(invalid(key)); } }
        let raw = &root[kind.key()];
        if raw.is_null() { return Ok(Leaf::Unobtained) }
        match crate::world::rederive_collector_domain_problems(self.raw()) {
            Err(()) => return Err(CountryRosterRefusal::MalformedLedger),
            Ok(Some(problems)) => for problem in problems {
                if [18, kind.domain()].contains(&problem.domain)
                    && !crate::world::domain_problem_is_per_family_accounted(&problem) {
                    return Err(CountryRosterRefusal::DomainProblem(problem.problem));
                }
            },
            Ok(None) => {}
        }
        let owner = self.country_identity_for_tag(tag).map_err(CountryRosterRefusal::CountryIdentity)?;
        let rows = raw.get("countries").and_then(Value::as_array).ok_or_else(|| invalid("countries"))?;
        if rows.len() > 65_536 { return Err(invalid("country bound")); }
        let mut total = 0usize;
        for row in rows {
            let slots = row.get("slots").and_then(Value::as_array).ok_or_else(|| invalid("slots"))?;
            total = total.checked_add(slots.len()).ok_or_else(|| invalid("slot overflow"))?;
            if slots.len() > kind.slot_limit() || total > kind.total_limit() { return Err(invalid("slot bound")); }
        }
        let table = Table::deserialize(raw).map_err(|e| invalid(&e.to_string()))?;
        if !valid_cause(table.failure) { return Err(invalid("global cause")); }
        match table.country_count {
            None if table.failure.is_none() || !table.countries.is_empty() => return Err(invalid("unknown census")),
            Some(count) if count as usize != table.countries.len() => return Err(invalid("physical cardinality")),
            _ => {}
        }
        let mut owners = BTreeSet::new();
        let mut members = BTreeSet::new();
        for (physical, row) in table.countries.iter().enumerate() {
            if row.physical_slot as usize != physical || !valid_cause(row.failure) || row.acquisition > 2 {
                return Err(invalid("physical/acquisition/cause"));
            }
            if let Some(id) = row.owner_tag_db_id {
                if id == 0 || !owners.insert(id) { return Err(invalid("ambiguous owner")); }
            }
            if (row.owner_tag_db_id.is_none() || row.acquisition != 1 || row.observed_count.is_none())
                && row.failure.is_none() { return Err(invalid("unwitnessed complete")); }
            match row.observed_count {
                None if !row.slots.is_empty() => return Err(invalid("unknown count with slots")),
                Some(count) if count as usize != row.slots.len()
                    && (row.failure.is_none() || !row.slots.is_empty()) => return Err(invalid("slot cardinality")),
                _ => {}
            }
            for (index, slot) in row.slots.iter().enumerate() {
                if slot.ordinal as usize != index || !valid_cause(slot.failure) { return Err(invalid("slot ordinal/cause")); }
                match (slot.reference, slot.failure) {
                    (Some(reference), None) if reference.id != 0 && kind.accepts(reference.native_type)
                        && members.insert(reference) => {}
                    (None, Some(_)) => {}
                    _ => return Err(invalid("slot reference/cause")),
                }
            }
        }
        if let Some(cause) = table.failure { return Err(CountryRosterRefusal::Unavailable(cause)); }
        table.countries.into_iter().find(|row| row.owner_tag_db_id == Some(owner))
            .map(Leaf::Measured).ok_or(CountryRosterRefusal::OwnerUnwitnessed)
    }

    /// Current-hour references only; this API does not infer lifetime generations or UI history.
    pub fn ordered_country_references(
        &self, kind: CountryRosterKind, tag: &str,
    ) -> Result<Leaf<Vec<CountryRosterReference>>, CountryRosterRefusal> {
        let row = match self.country_roster_observation(kind, tag)? {
            Leaf::Measured(row) => row, Leaf::Absent => return Ok(Leaf::Absent),
            _ => return Ok(Leaf::Unobtained),
        };
        if let Some(cause) = row.failure { return Err(CountryRosterRefusal::Unavailable(cause)); }
        let mut ordered = Vec::with_capacity(row.slots.len());
        for slot in row.slots {
            match (slot.reference, slot.failure) {
                (Some(reference), None) => ordered.push(reference),
                (_, Some(cause)) => return Err(CountryRosterRefusal::Unavailable(cause)),
                _ => return Err(CountryRosterRefusal::CurrentMembershipUnavailable),
            }
        }
        let mut current = BTreeMap::<CountryRosterReference, usize>::new();
        match kind {
            CountryRosterKind::Divisions => {
                if self.division_units_status() != ViewStatus::Present {
                    return Err(CountryRosterRefusal::CurrentMembershipUnavailable);
                }
                for division in self.division_units() {
                    let Leaf::Measured(owner) = &division.owner else { continue };
                    if owner.tag() != tag { continue; }
                    let Leaf::Measured(native_type) = &division.raw_division_type else {
                        return Err(CountryRosterRefusal::CurrentMembershipUnavailable);
                    };
                    *current.entry(CountryRosterReference { native_type: *native_type, id: division.raw_division_id }).or_default() += 1;
                }
            }
            CountryRosterKind::Fleets => {
                let rows = self.raw().get("order_of_battle").and_then(|v| v.get("fleets"))
                    .and_then(Value::as_array).ok_or(CountryRosterRefusal::CurrentMembershipUnavailable)?;
                for fleet in rows {
                    let fleet = fleet.as_object()
                        .ok_or(CountryRosterRefusal::CurrentMembershipUnavailable)?;
                    let (owner, _) = crate::world::country_identity_leaf_pairing(
                        fleet, &["owner"], row.owner_tag_db_id, "country roster fleet",
                        self.contended_census(), &mut Vec::new(),
                    );
                    let Leaf::Measured(owner) = owner else { continue };
                    if owner.tag() != tag { continue; }
                    if let Some(raw_id) = fleet.get("owner_tag_db_id") {
                        if raw_id.as_u64() != row.owner_tag_db_id.map(u64::from) {
                            return Err(CountryRosterRefusal::CurrentMembershipMismatch);
                        }
                    }
                    let id = fleet.get("id").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok())
                        .filter(|v| *v != 0).ok_or(CountryRosterRefusal::CurrentMembershipUnavailable)?;
                    *current.entry(CountryRosterReference { native_type: 61, id }).or_default() += 1;
                }
            }
        }
        if current.values().any(|count| *count != 1)
            || current.keys().copied().collect::<BTreeSet<_>>() != ordered.iter().copied().collect() {
            return Err(CountryRosterRefusal::CurrentMembershipMismatch);
        }
        Ok(Leaf::Measured(ordered))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn raw(ids: &[u32]) -> Value {
        json!({
            "tag_db_id_index":{"country_composition_version":2},
            "countries":{"GER":{"_r2_tag_db_id":7}},
            "country_semantic_rosters":{"schema":1,"hour_index":7,"divisions":null,"fleets":{
                "country_count":1,"failure":null,"countries":[{
                    "physical_slot":0,"owner_tag_db_id":7,"acquisition":1,"observed_count":ids.len(),"failure":null,
                    "slots":ids.iter().enumerate().map(|(i,id)|json!({"ordinal":i,"reference":{"native_type":61,"id":id},"failure":null})).collect::<Vec<_>>(),
                }]
            }},
            "order_of_battle":{"fleets":ids.iter().map(|id|json!({"id":id,"owner":"GER"})).collect::<Vec<_>>()}
        })
    }
    fn frame(raw: Value) -> WorldFrame {
        WorldFrame::from_raw_scheduled(crate::source::RawFrame {
            hour_index:7,date:None,save_version:None,schema_version:None,
            rest:raw.as_object().unwrap().clone().into(),
        }, None, None, None, crate::BuildSchedule::Serial)
    }
    #[test]
    fn ordered_fleets_and_measured_empty_join_actual_current_membership() {
        for ids in [vec![9,2], vec![2,9], vec![]] {
            let world = frame(raw(&ids));
            assert_eq!(world.ordered_country_references(CountryRosterKind::Fleets,"GER").unwrap(),
                Leaf::Measured(ids.iter().map(|id|CountryRosterReference {native_type:61,id:*id}).collect()));
        }
    }
    #[test]
    fn refused_slot_and_unknown_count_never_certify_empty() {
        let mut value = raw(&[9]);
        let country = &mut value["country_semantic_rosters"]["fleets"]["countries"][0];
        country["slots"][0] = json!({"ordinal":0,"reference":null,"failure":8});
        assert_eq!(frame(value).ordered_country_references(CountryRosterKind::Fleets,"GER"),
            Err(CountryRosterRefusal::Unavailable(8)));
        let mut value=raw(&[]);
        let country=&mut value["country_semantic_rosters"]["fleets"]["countries"][0];
        country["observed_count"]=Value::Null; country["failure"]=json!(8);
        assert_eq!(frame(value).ordered_country_references(CountryRosterKind::Fleets,"GER"),
            Err(CountryRosterRefusal::Unavailable(8)));
    }
    #[test]
    fn same_hour_problem_or_missing_current_reference_refuses_then_clean_hour_recovers() {
        let mut value=raw(&[9,2]);
        value["provenance"]=json!({"collector":{"domain_problems":[{"domain_id":28,"problem":"torn_read"}]}});
        assert!(matches!(frame(value).ordered_country_references(CountryRosterKind::Fleets,"GER"),Err(CountryRosterRefusal::DomainProblem(_))));
        let mut value=raw(&[9,2]); value["order_of_battle"]["fleets"].as_array_mut().unwrap().pop();
        assert_eq!(frame(value).ordered_country_references(CountryRosterKind::Fleets,"GER"),Err(CountryRosterRefusal::CurrentMembershipMismatch));
        assert!(matches!(frame(raw(&[2,9])).ordered_country_references(CountryRosterKind::Fleets,"GER"),Ok(Leaf::Measured(_))));
    }
    #[test]
    fn missing_nullable_field_cross_hour_and_compacted_physical_slot_are_rejected() {
        let mut value=raw(&[9]);
        value["country_semantic_rosters"]["fleets"]["countries"][0].as_object_mut().unwrap().remove("failure");
        assert!(frame(value).country_roster_observation(CountryRosterKind::Fleets,"GER").is_err());
        let mut value=raw(&[9]); value["country_semantic_rosters"]["hour_index"]=json!(8);
        assert_eq!(frame(value).country_roster_observation(CountryRosterKind::Fleets,"GER"),Err(CountryRosterRefusal::HourMismatch));
        let mut value=raw(&[9]); value["country_semantic_rosters"]["fleets"]["countries"][0]["physical_slot"]=json!(3);
        assert!(frame(value).country_roster_observation(CountryRosterKind::Fleets,"GER").is_err());
    }
    #[test]
    fn division_type_51_survives_current_typed_membership_and_exact_reference_join() {
        let mut value=raw(&[]);
        let mut table=value["country_semantic_rosters"]["fleets"].take();
        table["countries"][0]["observed_count"]=json!(2);
        table["countries"][0]["slots"]=json!([
            {"ordinal":0,"reference":{"native_type":51,"id":9},"failure":null},
            {"ordinal":1,"reference":{"native_type":4713,"id":2},"failure":null}
        ]);
        value["country_semantic_rosters"]["divisions"]=table;
        value["divisions"]=json!([
            {"div_id":9,"div_type":51,"div_uid":"9:51","owner":"GER"},
            {"div_id":2,"div_type":4713,"div_uid":"2:4713","owner":"GER"}
        ]);
        let world=frame(value.clone());
        assert_eq!(world.ordered_country_references(CountryRosterKind::Divisions,"GER").unwrap(),
            Leaf::Measured(vec![CountryRosterReference {native_type:51,id:9},
                CountryRosterReference {native_type:4713,id:2}]));
        value["divisions"][0]["div_type"]=json!(4713);
        assert_eq!(frame(value).ordered_country_references(CountryRosterKind::Divisions,"GER"),
            Err(CountryRosterRefusal::CurrentMembershipMismatch));
    }

    #[test]
    fn current_fleet_owner_cannot_contradict_its_census_paired_id() {
        for raw_id in [json!(8), Value::Null, json!(-1), json!("7")] {
            let mut value = raw(&[9]);
            value["order_of_battle"]["fleets"][0]["owner_tag_db_id"] = raw_id;
            assert_eq!(
                frame(value).ordered_country_references(CountryRosterKind::Fleets, "GER"),
                Err(CountryRosterRefusal::CurrentMembershipMismatch),
            );
        }
        let mut value = raw(&[9]);
        value["order_of_battle"]["fleets"][0]["owner_tag_db_id"] = json!(7);
        assert!(matches!(
            frame(value).ordered_country_references(CountryRosterKind::Fleets, "GER"),
            Ok(Leaf::Measured(_)),
        ));
        for key in [Value::Null, json!("FRA")] {
            let mut value = raw(&[9]);
            value["order_of_battle"]["fleets"][0]["owner"] = key;
            assert_eq!(
                frame(value).ordered_country_references(CountryRosterKind::Fleets, "GER"),
                Err(CountryRosterRefusal::CurrentMembershipMismatch),
            );
        }
    }

}
