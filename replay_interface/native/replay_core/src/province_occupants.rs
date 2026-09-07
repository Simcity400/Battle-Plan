//! Current D22 province order. Class filtering precedes reached identity joins.
use crate::{MapLayerSource, ViewStatus, WorldFrame};
use portable_compact_stream::province_occupants as wire;
use serde_json::{Map, Value};
pub use wire::{Cause, Family, Reference, Slot, SlotValue, Status};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProvinceOccupantRefusal {
    Missing,
    Unobtained,
    Invalid(String),
    MalformedLedger,
    DomainProblem(String),
    HourMismatch,
    CurrentMapUnavailable,
    ProvinceUnwitnessed,
    Acquisition(Cause),
    UnknownClass { ordinal: u32, cause: Cause },
    SlotIdentity { ordinal: u32, cause: Cause },
}
type Result<T> = std::result::Result<T, ProvinceOccupantRefusal>;
fn invalid(s: impl Into<String>) -> ProvinceOccupantRefusal {
    ProvinceOccupantRefusal::Invalid(s.into())
}
fn object<'a>(v: &'a Value, keys: &[&str]) -> Result<&'a Map<String, Value>> {
    let o = v.as_object().ok_or_else(|| invalid("object"))?;
    if o.len() != keys.len() || keys.iter().any(|k| !o.contains_key(*k)) {
        return Err(invalid("mandatory/unknown fields"));
    }
    Ok(o)
}
fn u32v(v: &Value) -> Result<u32> {
    v.as_u64()
        .and_then(|n| n.try_into().ok())
        .ok_or_else(|| invalid("u32"))
}
fn i32v(v: &Value) -> Result<i32> {
    v.as_i64()
        .and_then(|n| n.try_into().ok())
        .ok_or_else(|| invalid("i32"))
}
fn nullable<T>(v: &Value, f: impl FnOnce(&Value) -> Result<T>) -> Result<Option<T>> {
    if v.is_null() {
        Ok(None)
    } else {
        f(v).map(Some)
    }
}
fn byte(v: &Value) -> Result<u8> {
    u32v(v)?.try_into().map_err(|_| invalid("u8"))
}
fn status(v: &Value) -> Result<Status> {
    Status::from_raw(byte(v)?).map_err(invalid)
}
fn cause(v: &Value) -> Result<Cause> {
    Cause::from_raw(byte(v)?).map_err(invalid)
}
fn array(v: &Value, max: usize) -> Result<&Vec<Value>> {
    let a = v.as_array().ok_or_else(|| invalid("array"))?;
    if a.len() > max {
        return Err(invalid("array bound"));
    }
    Ok(a)
}

fn parse(raw: &Value) -> Result<wire::Census> {
    let o = object(
        raw,
        &[
            "schema",
            "game_hour_raw",
            "observation_ordinal",
            "root_slot_count",
            "root_coverage",
            "provinces",
        ],
    )?;
    if u32v(&o["schema"])? != 1 {
        return Err(invalid("schema"));
    }
    let coverage = object(&o["root_coverage"], &["status", "cause", "covered_prefix"])?;
    let mut c = wire::Census {
        game_hour_raw: i32v(&o["game_hour_raw"])?,
        observation_ordinal: o["observation_ordinal"]
            .as_u64()
            .ok_or_else(|| invalid("ordinal"))?,
        root_slot_count: nullable(&o["root_slot_count"], u32v)?,
        status: status(&coverage["status"])?,
        cause: cause(&coverage["cause"])?,
        root_covered_prefix: u32v(&coverage["covered_prefix"])?,
        provinces: Vec::new(),
    };
    let rows = array(&o["provinces"], wire::MAX_PROVINCES)?;
    // Bound all allocations before decoding the nested carrier.
    let mut slots = 0usize;
    for p in rows {
        for v in array(p.get("vectors").ok_or_else(|| invalid("vectors"))?, 4)? {
            slots = slots
                .checked_add(
                    array(
                        v.get("slots").ok_or_else(|| invalid("slots"))?,
                        wire::MAX_SLOTS,
                    )?
                    .len(),
                )
                .ok_or_else(|| invalid("slot overflow"))?;
            if slots > wire::MAX_SLOTS {
                return Err(invalid("total slots"));
            }
        }
    }
    for p in rows {
        let p = object(
            p,
            &[
                "physical_ordinal",
                "province_id",
                "status",
                "cause",
                "vectors",
            ],
        )?;
        let mut province = wire::Province {
            physical_ordinal: u32v(&p["physical_ordinal"])?,
            province_id: nullable(&p["province_id"], u32v)?,
            status: status(&p["status"])?,
            cause: cause(&p["cause"])?,
            vectors: Vec::new(),
        };
        for v in array(&p["vectors"], 4)? {
            let v = object(
                v,
                &[
                    "family",
                    "status",
                    "cause",
                    "declared_count",
                    "covered_prefix",
                    "slots",
                ],
            )?;
            let mut vector = wire::Vector {
                family: Family::from_raw(byte(&v["family"])?).map_err(invalid)?,
                status: status(&v["status"])?,
                cause: cause(&v["cause"])?,
                declared_count: nullable(&v["declared_count"], i32v)?,
                covered_prefix: u32v(&v["covered_prefix"])?,
                slots: Vec::new(),
            };
            for s in array(&v["slots"], wire::MAX_SLOTS)? {
                let s = object(
                    s,
                    &[
                        "physical_ordinal",
                        "status",
                        "cause",
                        "class_raw",
                        "reference",
                    ],
                )?;
                let class_raw = nullable(&s["class_raw"], i32v)?;
                let why = cause(&s["cause"])?;
                let value = match byte(&s["status"])? {
                    1 if why == Cause::None => {
                        let r = object(&s["reference"], &["class_raw", "type_raw", "id_raw"])?;
                        let r = Reference {
                            class_raw: i32v(&r["class_raw"])?,
                            type_raw: u32v(&r["type_raw"])?,
                            id_raw: u32v(&r["id_raw"])?,
                        };
                        if class_raw != Some(r.class_raw) {
                            return Err(invalid("class parity"));
                        }
                        SlotValue::Measured(r)
                    }
                    2 if why == Cause::None && class_raw.is_none() && s["reference"].is_null() => {
                        SlotValue::Null
                    }
                    3 if s["reference"].is_null() => SlotValue::Failed {
                        cause: why,
                        class_raw,
                    },
                    _ => return Err(invalid("slot status/fields")),
                };
                vector.slots.push(Slot {
                    physical_ordinal: u32v(&s["physical_ordinal"])?,
                    value,
                });
            }
            province.vectors.push(vector);
        }
        c.provinces.push(province);
    }
    c.validate().map_err(invalid)?;
    Ok(c)
}
pub(crate) fn decode_carrier(raw: Option<&Value>) -> Result<wire::Census> {
    match raw {
        None => Err(ProvinceOccupantRefusal::Missing),
        Some(Value::Null) => Err(ProvinceOccupantRefusal::Unobtained),
        Some(raw) => parse(raw),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvinceOccupantObservation<'a> {
    game_hour_raw: i32,
    observation_ordinal: u64,
    province_id: u32,
    physical_province_ordinal: u32,
    vector: &'a wire::Vector,
}
impl ProvinceOccupantObservation<'_> {
    pub fn game_hour_raw(&self) -> i32 {
        self.game_hour_raw
    }
    pub fn observation_ordinal(&self) -> u64 {
        self.observation_ordinal
    }
    pub fn province_id(&self) -> u32 {
        self.province_id
    }
    pub fn physical_province_ordinal(&self) -> u32 {
        self.physical_province_ordinal
    }
    pub fn family(&self) -> Family {
        self.vector.family
    }
    pub fn status(&self) -> Status {
        self.vector.status
    }
    pub fn cause(&self) -> Cause {
        self.vector.cause
    }
    pub fn declared_count(&self) -> Option<i32> {
        self.vector.declared_count
    }
    pub fn covered_prefix(&self) -> u32 {
        self.vector.covered_prefix
    }
    pub fn slots(&self) -> &[Slot] {
        &self.vector.slots
    }
    /// Only a witnessed nonmatching class is excluded. Null and unknown class
    /// remain explicit slots for the caller's native null/refusal law.
    /// Callers resolve each returned full pair against current membership.
    pub fn class_slots(&self, class: i32) -> impl Iterator<Item = &Slot> {
        self.vector
            .slots
            .iter()
            .filter(move |s| !matches!(s.value.class_witness(),Some(c) if c!=class))
    }
}
impl WorldFrame {
    pub fn province_occupants(
        &self,
        province_id: u32,
        family: Family,
    ) -> Result<ProvinceOccupantObservation<'_>> {
        match crate::world::rederive_collector_domain_problems(self.raw()) {
            Err(()) => return Err(ProvinceOccupantRefusal::MalformedLedger),
            Ok(Some(problems)) => {
                for p in problems {
                    if p.domain == 22 && !crate::world::domain_problem_is_per_family_accounted(&p) {
                        return Err(ProvinceOccupantRefusal::DomainProblem(p.problem));
                    }
                }
            }
            Ok(None) => {}
        }
        if self.control_by_province_status() != ViewStatus::Present
            || self.control_by_province_source() != MapLayerSource::MapMirror
        {
            return Err(ProvinceOccupantRefusal::CurrentMapUnavailable);
        }
        let c = self
            .province_occupants_carrier
            .as_ref()
            .map_err(Clone::clone)?;
        if i64::from(c.game_hour_raw) - i64::from(portable_admission::DATE_EPOCH_CONST)
            != self.hour_index()
        {
            return Err(ProvinceOccupantRefusal::HourMismatch);
        }
        if c.status != Status::Measured {
            return Err(ProvinceOccupantRefusal::Acquisition(c.cause));
        }
        // The political mirror omits measured no-controller provinces. D22
        // retains every province row with these two independently nullable
        // fields; require that row as well as the fresh POC identity below.
        let current_map_province = self.raw().get("provinces")
            .and_then(Value::as_object)
            .and_then(|rows| rows.get(&province_id.to_string()))
            .and_then(Value::as_object)
            .filter(|row| row.contains_key("controller") && row.contains_key("state_id"));
        if current_map_province.is_none() {
            return Err(ProvinceOccupantRefusal::ProvinceUnwitnessed);
        }
        let p = c
            .provinces
            .get(province_id as usize)
            .filter(|p| p.province_id == Some(province_id))
            .ok_or(ProvinceOccupantRefusal::ProvinceUnwitnessed)?;
        if p.status != Status::Measured {
            return Err(ProvinceOccupantRefusal::Acquisition(p.cause));
        }
        let vector = &p.vectors[family as usize];
        Ok(ProvinceOccupantObservation {
            game_hour_raw: c.game_hour_raw,
            observation_ordinal: c.observation_ordinal,
            province_id,
            physical_province_ordinal: p.physical_ordinal,
            vector,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn frame(raw: Value, hour: i64) -> WorldFrame {
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
    fn carrier() -> Value {
        let vectors:Vec<_>=Family::ALL.into_iter().map(|f|json!({"family":f as u8,"status":1,"cause":0,"declared_count":0,"covered_prefix":0,"slots":[]})).collect();
        json!({"map":{"control_by_province":{"1":null}},"provinces":{"1":{"controller":null,"state_id":1}},"province_occupants":{
            "schema":1,"game_hour_raw":portable_admission::DATE_EPOCH_CONST+7,"observation_ordinal":91,"root_slot_count":2,"root_coverage":{"status":1,"cause":0,"covered_prefix":2},
            "provinces":[{"physical_ordinal":0,"province_id":null,"status":5,"cause":0,"vectors":[]},
                {"physical_ordinal":1,"province_id":1,"status":1,"cause":0,"vectors":vectors}]
        }})
    }
    #[test]
    fn current_hour_empty_and_missing_null_remain_distinct() {
        let raw = carrier();
        let world = frame(raw.clone(), 7);
        let got = world.province_occupants(1, Family::Mixed).unwrap();
        assert_eq!(got.declared_count(), Some(0));
        assert!(got.slots().is_empty());
        assert_eq!(
            frame(raw.clone(), 8).province_occupants(1, Family::Mixed),
            Err(ProvinceOccupantRefusal::HourMismatch)
        );
        let mut missing = raw.clone();
        missing
            .as_object_mut()
            .unwrap()
            .remove("province_occupants");
        assert_eq!(
            frame(missing, 7).province_occupants(1, Family::Mixed),
            Err(ProvinceOccupantRefusal::Missing)
        );
        let mut null = raw;
        null["province_occupants"] = Value::Null;
        assert_eq!(
            frame(null, 7).province_occupants(1, Family::Mixed),
            Err(ProvinceOccupantRefusal::Unobtained)
        );
    }
    #[test]
    fn d22_ledger_and_malformed_ledger_precede_even_empty_filter() {
        for ledger in [
            json!([{"domain_id":22,"problem":"malformed_payload"}]),
            json!("invalid"),
        ] {
            let mut raw = carrier();
            raw["provenance"] = json!({"collector":{"domain_problems":ledger}});
            assert!(matches!(
                frame(raw, 7).province_occupants(1, Family::Mixed),
                Err(ProvinceOccupantRefusal::DomainProblem(_)
                    | ProvinceOccupantRefusal::MalformedLedger)
            ));
        }
        let mut raw = carrier();
        raw["map"]["control_by_province"] = Value::Null;
        assert_eq!(
            frame(raw, 7).province_occupants(1, Family::Mixed),
            Err(ProvinceOccupantRefusal::CurrentMapUnavailable)
        );
    }
    #[test]
    fn uncontrolled_province_requires_current_d22_row_and_fresh_poc_identity() {
        let mut raw = carrier();
        raw["map"]["control_by_province"] = json!({});
        let world = frame(raw.clone(), 7);
        assert!(world.control_by_province().is_empty());
        assert_eq!(world.province_occupants(1, Family::Mixed).unwrap().declared_count(), Some(0));
        // No state (sea/lake) is independent of witnessed province existence.
        let mut no_state = raw.clone();
        no_state["provinces"]["1"]["state_id"] = Value::Null;
        assert!(frame(no_state, 7).province_occupants(1, Family::Mixed).is_ok());
        for rows in [Value::Null, json!({}), json!({"2":{"controller":null,"state_id":1}}),
            json!({"1":null}), json!({"1":{"controller":null}}), json!({"1":{"state_id":1}})] {
            let mut missing = raw.clone();
            missing["provinces"] = rows;
            // A political entry cannot manufacture the missing D22 row.
            missing["map"]["control_by_province"] = json!({"1":null});
            assert_eq!(frame(missing, 7).province_occupants(1, Family::Mixed),
                Err(ProvinceOccupantRefusal::ProvinceUnwitnessed));
        }
        let mut missing = raw.clone();
        missing.as_object_mut().unwrap().remove("provinces");
        assert_eq!(frame(missing, 7).province_occupants(1, Family::Mixed),
            Err(ProvinceOccupantRefusal::ProvinceUnwitnessed));
        let mut vacant = raw.clone();
        vacant["province_occupants"]["provinces"][1] = json!({
            "physical_ordinal":1,"province_id":null,"status":5,"cause":0,"vectors":[]});
        assert_eq!(frame(vacant, 7).province_occupants(1, Family::Mixed),
            Err(ProvinceOccupantRefusal::ProvinceUnwitnessed));
        let mut failed = raw.clone();
        failed["map"]["control_by_province"] = Value::Null;
        assert_eq!(frame(failed, 7).province_occupants(1, Family::Mixed),
            Err(ProvinceOccupantRefusal::CurrentMapUnavailable));
        raw["provenance"] = json!({"collector":{"domain_problems":[{"domain_id":22,"problem":"malformed_payload"}]}});
        assert!(matches!(frame(raw, 7).province_occupants(1, Family::Mixed),
            Err(ProvinceOccupantRefusal::DomainProblem(_))));
    }
    #[test]
    fn strict_nullable_and_class_parity_are_validated_before_access() {
        for key in ["root_slot_count", "observation_ordinal", "root_coverage"] {
            let mut raw = carrier();
            raw["province_occupants"]
                .as_object_mut()
                .unwrap()
                .remove(key);
            assert!(frame(raw, 7).province_occupants(1, Family::Mixed).is_err());
        }
        let mut raw = carrier();
        let v = &mut raw["province_occupants"]["provinces"][1]["vectors"][0];
        v["declared_count"] = json!(1);
        v["covered_prefix"] = json!(1);
        v["slots"] = json!([{"physical_ordinal":0,"status":1,"cause":0,"class_raw":0,"reference":{"class_raw":13,"type_raw":51,"id_raw":7}}]);
        assert!(matches!(
            frame(raw, 7).province_occupants(1, Family::Mixed),
            Err(ProvinceOccupantRefusal::Invalid(_))
        ));
    }
    #[test]
    #[ignore = "requires hash-bound verbatim native producer/fold output"]
    fn actual_producer_fold_stages_preserve_filter_order_and_whole_hour_refusal() {
        use sha2::{Digest, Sha256};
        let path = std::env::var_os("BATTLE_PLAN_POC1_JOINT_INPUT").or_else(|| std::env::var_os("HOI4_POC1_JOINT_INPUT")).expect("native output required");
        assert!(std::fs::metadata(&path).unwrap().len() <= 1024 * 1024);
        let bytes = std::fs::read(path).unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            std::env::var("BATTLE_PLAN_POC1_JOINT_SHA256").or_else(|_| std::env::var("HOI4_POC1_JOINT_SHA256")).unwrap()
        );
        let artifact: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(artifact["schema"], 1);
        let stages = artifact["stages"].as_array().unwrap();
        assert_eq!(stages.len(), 4);
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
            assert_eq!(raw.rest.to_map(), expected, "verbatim snapshot");
            let world =
                WorldFrame::from_raw_scheduled(raw, None, None, None, crate::BuildSchedule::Serial);
            assert_eq!(world.hour_index(), if i == 3 { 16_959_368 } else { 16_959_367 });
            let result = world.province_occupants(1, Family::Mixed);
            if i == 0 || i == 3 {
                assert!(world.control_by_province().is_empty(),
                    "actual no-controller province is absent from the political mirror");
                assert_eq!(value["provinces"]["1"]["controller"], Value::Null);
                let got = result.unwrap();
                let selected: Vec<_> = got
                    .class_slots(13)
                    .map(|s| {
                        (
                            s.physical_ordinal,
                            match s.value {
                                SlotValue::Measured(r) => r.id_raw,
                                _ => panic!("reached identity unavailable"),
                            },
                        )
                    })
                    .collect();
                assert_eq!(selected, vec![(1, 7), (2, 3)]);
                assert_eq!(
                    world
                        .province_occupants(1, Family::Class13)
                        .unwrap()
                        .declared_count(),
                    Some(0),
                    "independent native class13 vector is not mixed-filter substitute"
                );
            } else {
                assert!(matches!(
                    result,
                    Err(ProvinceOccupantRefusal::DomainProblem(_))
                ));
            }
        }
    }
    #[test]
    fn class_filter_preserves_null_duplicate_occurrences_and_physical_ordinals() {
        let mut raw = carrier();
        let v = &mut raw["province_occupants"]["provinces"][1]["vectors"][0];
        v["declared_count"] = json!(4);
        v["covered_prefix"] = json!(4);
        v["slots"] = json!([
            {"physical_ordinal":0,"status":1,"cause":0,"class_raw":0,"reference":{"class_raw":0,"type_raw":47,"id_raw":9}},
            {"physical_ordinal":1,"status":2,"cause":0,"class_raw":null,"reference":null},
            {"physical_ordinal":2,"status":1,"cause":0,"class_raw":13,"reference":{"class_raw":13,"type_raw":4294967295u32,"id_raw":4294967295u32}},
            {"physical_ordinal":3,"status":1,"cause":0,"class_raw":13,"reference":{"class_raw":13,"type_raw":4294967295u32,"id_raw":4294967295u32}}
        ]);
        let world = frame(raw, 7);
        let observed = world.province_occupants(1, Family::Mixed).unwrap();
        assert_eq!(observed.physical_province_ordinal(), 1);
        let filtered: Vec<_> = observed.class_slots(13).copied().collect();
        assert_eq!(
            filtered
                .iter()
                .map(|s| s.physical_ordinal)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(filtered[0].value, SlotValue::Null);
        assert_eq!(
            filtered[1].value,
            SlotValue::Measured(Reference {
                class_raw: 13,
                type_raw: u32::MAX,
                id_raw: u32::MAX
            })
        );
        assert_eq!(
            filtered[2].value, filtered[1].value,
            "duplicate membership occurrence is not compacted"
        );
        assert!(
            world
                .province_occupants(1, Family::Class13)
                .unwrap()
                .slots()
                .is_empty(),
            "independent class13 vector is not a mixed substitute"
        );
    }
}
