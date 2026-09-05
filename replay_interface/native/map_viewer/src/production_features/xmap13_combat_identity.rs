use crate::{PreparedXmap13Frame, Result, Xmap13CounterSemantic, Xmap13UnitIdentity};
use replay_core::{Leaf, ViewStatus, WorldFrame};
use serde::Serialize;
use snapshot::sha256::Sha256;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize)]
struct DivisionIdentityRow {
    stable_id: u32,
    #[serde(skip_serializing_if = "Leaf::is_absent")]
    type_id: Leaf<u32>,
    #[serde(skip_serializing_if = "Leaf::is_absent")]
    owner_country: Leaf<String>,
    #[serde(skip_serializing_if = "Leaf::is_absent")]
    anchor_province_id: Leaf<u32>,
}

pub struct Xmap13CombatUnit<'a> {
    pub owner_country: Option<&'a str>,
    pub anchor_province_id: Option<u32>,
}

pub struct Xmap13CombatIdentityProvider<'a> {
    hour_index: i64,
    division_status: ViewStatus,
    divisions: Vec<DivisionIdentityRow>,
    by_identity: BTreeMap<Xmap13UnitIdentity, usize>,
    by_stable_id: BTreeMap<u32, Vec<Option<Xmap13UnitIdentity>>>,
    visual: Option<&'a PreparedXmap13Frame>,
    census_sha256: String,
    admission_sha256: String,
}

impl<'a> Xmap13CombatIdentityProvider<'a> {
    pub fn from_world_frame(
        frame: &WorldFrame,
        foundation_content_operation_sha256: &str,
        visual: Option<&'a PreparedXmap13Frame>,
    ) -> Result<Self> {
        if foundation_content_operation_sha256.len() != 64
            || !foundation_content_operation_sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("XMAP13 combat content receipt digest is invalid".into());
        }
        if let Some(visual) = visual {
            super::xmap13::validate_combat_visual_frame(frame, visual)?;
        }
        let source = frame.domain("divisions");
        let source_sha256 = Sha256::hex(&serde_json::to_vec(&(source.is_some(), source))?);
        let mut division_status = frame.division_units_status();
        if division_status == ViewStatus::Present
            && source.and_then(serde_json::Value::as_array).map(Vec::len) != Some(frame.division_units().len())
        {
            division_status = ViewStatus::DomainUnavailable;
        }
        let divisions = frame.division_units().iter().map(|row| DivisionIdentityRow {
            stable_id: row.raw_division_id,
            type_id: row.raw_division_type.clone(),
            owner_country: row.owner.clone().map(|owner| owner.tag().to_owned()),
            anchor_province_id: row.location_province.clone(),
        }).collect();
        Self::from_rows(frame.hour_index(), division_status, divisions, &source_sha256,
            foundation_content_operation_sha256, visual)
    }

    fn from_rows(
        hour_index: i64, division_status: ViewStatus, divisions: Vec<DivisionIdentityRow>,
        source_sha256: &str, content_digest: &str, visual: Option<&'a PreparedXmap13Frame>,
    ) -> Result<Self> {
        let mut by_identity = BTreeMap::new();
        let mut by_stable_id = BTreeMap::<u32, Vec<Option<Xmap13UnitIdentity>>>::new();
        if division_status == ViewStatus::Present {
            for (index, row) in divisions.iter().enumerate() {
                let identity = row.type_id.measured().map(|type_id| Xmap13UnitIdentity::Division {
                    type_id: *type_id, stable_id: row.stable_id,
                }).filter(|identity| identity.validate().is_ok());
                by_stable_id.entry(row.stable_id).or_default().push(identity.clone());
                let Some(identity) = identity else { continue };
                if by_identity.insert(identity, index).is_some() {
                    return Err("XMAP13 combat division census repeats a typed identity".into());
                }
            }
        }
        let status = match division_status {
            ViewStatus::Present => "present",
            ViewStatus::DomainAbsent => "absent",
            ViewStatus::DomainUnavailable => "unavailable",
        };
        let census_sha256 = Sha256::hex(&serde_json::to_vec(&(
            "XMAP13_COMBAT_IDENTITY_CENSUS_V1", hour_index, status, source_sha256, &divisions,
            visual.map(|value| value.identity_provider().census_sha256()),
        ))?);
        let admission_sha256 = Sha256::hex(&serde_json::to_vec(&(
            "XMAP13_COMBAT_IDENTITY_ADMISSION_V1", hour_index, &census_sha256,
            content_digest, visual.map(|value| value.identity_provider().admission_sha256()),
        ))?);
        Ok(Self { hour_index, division_status, divisions, by_identity, by_stable_id,
            visual, census_sha256, admission_sha256 })
    }

    pub fn hour_index(&self) -> i64 { self.hour_index }
    pub fn division_status(&self) -> ViewStatus { self.division_status }
    pub fn division_count(&self) -> Option<usize> {
        (self.division_status == ViewStatus::Present).then_some(self.divisions.len())
    }
    pub fn census_sha256(&self) -> &str { &self.census_sha256 }
    pub fn admission_sha256(&self) -> &str { &self.admission_sha256 }

    pub fn division_identities(&self, stable_id: u32) -> &[Option<Xmap13UnitIdentity>] {
        self.by_stable_id.get(&stable_id).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn division_stable_id_match_count(&self, stable_id: u32) -> usize {
        match self.division_identities(stable_id) {
            [None] => 0,
            candidates => candidates.len(),
        }
    }

    pub fn lookup(&self, identity: &Xmap13UnitIdentity) -> Option<Xmap13CombatUnit<'_>> {
        if let Xmap13UnitIdentity::Division { stable_id, .. } = identity {
            if self.division_identities(*stable_id).iter().any(Option::is_none) { return None }
            let row = &self.divisions[*self.by_identity.get(identity)?];
            return Some(Xmap13CombatUnit {
                owner_country: row.owner_country.measured().map(String::as_str),
                anchor_province_id: row.anchor_province_id.measured().copied().filter(|id| *id != 0),
            });
        }
        let row = self.visual?.identity_provider().lookup(identity)?;
        Some(Xmap13CombatUnit { owner_country: Some(&row.owner_country), anchor_province_id: Some(row.anchor_province_id) })
    }

    pub fn counter(&self, identity: &Xmap13UnitIdentity) -> Option<&Xmap13CounterSemantic> {
        self.visual?.identity_provider().lookup(identity)?.counter.as_ref()
    }

    #[cfg(test)]
    pub(super) fn for_land_placement_test(hour: i64, units: Vec<(u32, u32, u32)>) -> Self {
        let rows: Vec<_> = units.into_iter().map(|(type_id, stable_id, province)| DivisionIdentityRow {
            stable_id, type_id: Leaf::Measured(type_id), owner_country: Leaf::Measured("GER".into()),
            anchor_province_id: Leaf::Measured(province),
        }).collect();
        let source_sha256 = Sha256::hex(&serde_json::to_vec(&rows).unwrap());
        Self::from_rows(hour, ViewStatus::Present, rows, &source_sha256, &"a".repeat(64), None).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    struct Source { hours: Vec<i64>, rest: Value }
    impl replay_core::ReplaySource for Source {
        fn game_id(&self) -> &str { "combat-identity-fixture" }
        fn hours(&self) -> &[i64] { &self.hours }
        fn warnings(&self) -> &[String] { &[] }
        fn frame(&self, hour_index: i64) -> std::result::Result<replay_core::RawFrame, replay_core::SourceError> {
            Ok(replay_core::RawFrame { hour_index, date: None, save_version: None, schema_version: None,
                rest: self.rest.as_object().unwrap().clone().into() })
        }
    }
    fn frame(hour: i64, rest: Value) -> std::sync::Arc<WorldFrame> {
        replay_core::WorldTimeline::open(Box::new(Source { hours: vec![hour], rest }))
            .unwrap().frame(hour).unwrap()
    }
    fn input() -> Value {
        json!({"map_unit_presentation": null, "order_of_battle": null,
            "divisions": [{"div_id": 7, "div_type": 51, "owner": "GER", "location_province": 42}]})
    }
    fn identity(type_id: u32, stable_id: u32) -> Xmap13UnitIdentity {
        Xmap13UnitIdentity::Division { type_id, stable_id }
    }

    #[test]
    fn combat_identity_uses_current_divisions_without_visual_admission() {
        let world = frame(42, input());
        let content = "a".repeat(64);
        let provider = Xmap13CombatIdentityProvider::from_world_frame(&world, &content, None).unwrap();
        assert_eq!(provider.hour_index(), 42);
        assert_eq!(provider.division_status(), ViewStatus::Present);
        assert_eq!(provider.division_count(), Some(1));
        assert_eq!(provider.division_identities(7), &[Some(identity(51, 7))]);
        assert_eq!(provider.lookup(&identity(51, 7)).unwrap().anchor_province_id, Some(42));
        assert!(provider.counter(&identity(51, 7)).is_none());
        assert!(provider.lookup(&Xmap13UnitIdentity::TaskForce { type_id: 51, stable_id: 7 }).is_none());
        assert!(provider.lookup(&Xmap13UnitIdentity::AirWing { stable_id: "7".into() }).is_none());
        for (hour, content, province) in [(43, "a".repeat(64), 42), (42, "b".repeat(64), 42), (42, "a".repeat(64), 43)] {
            let mut changed = input();
            changed["divisions"][0]["location_province"] = json!(province);
            let changed = frame(hour, changed);
            let changed = Xmap13CombatIdentityProvider::from_world_frame(&changed, &content, None).unwrap();
            assert_ne!(provider.admission_sha256(), changed.admission_sha256());
        }
    }

    #[test]
    fn combat_identity_preserves_missing_and_ambiguous_operands() {
        let mut raw = input();
        raw["divisions"].as_array_mut().unwrap().push(json!({"div_id": 7, "div_type": 61}));
        raw["divisions"].as_array_mut().unwrap().push(json!({"div_id": 8, "div_type": null}));
        let world = frame(42, raw.clone());
        let provider = Xmap13CombatIdentityProvider::from_world_frame(&world, &"a".repeat(64), None).unwrap();
        assert_eq!(provider.division_count(), Some(3));
        assert_eq!(provider.division_stable_id_match_count(7), 2);
        assert_eq!(provider.division_stable_id_match_count(8), 0);
        assert!(provider.lookup(&identity(61, 7)).unwrap().anchor_province_id.is_none());
        assert!(provider.lookup(&identity(61, 7)).unwrap().owner_country.is_none());
        let before = provider.census_sha256().to_owned();
        raw["divisions"][2].as_object_mut().unwrap().remove("div_type");
        let changed = frame(42, raw.clone());
        let changed = Xmap13CombatIdentityProvider::from_world_frame(&changed, &"a".repeat(64), None).unwrap();
        assert_ne!(before, changed.census_sha256());
        raw["divisions"][1]["div_type"] = json!(51);
        let duplicate = frame(42, raw);
        assert!(Xmap13CombatIdentityProvider::from_world_frame(&duplicate, &"a".repeat(64), None).is_err());
    }


    #[test]
    fn combat_identity_retains_unresolved_competitors_for_the_same_stable_id() {
        for type_id in [None, Some(Value::Null), Some(json!(0))] {
            let mut competitor = json!({"div_id": 7});
            if let Some(type_id) = type_id { competitor["div_type"] = type_id; }
            for first in [false, true] {
                let mut raw = input();
                raw["divisions"].as_array_mut().unwrap().insert(usize::from(!first), competitor.clone());
                let world = frame(42, raw);
                assert_eq!(world.division_units().len(), 2);
                let provider = Xmap13CombatIdentityProvider::from_world_frame(&world, &"a".repeat(64), None).unwrap();
                assert_eq!(provider.division_stable_id_match_count(7), 2);
                assert_eq!(provider.division_identities(7).len(), 2);
                assert_eq!(provider.division_identities(7).iter().filter(|id| id.is_none()).count(), 1);
                assert!(provider.lookup(&identity(51, 7)).is_none());
            }
        }
    }

    #[test]
    fn combat_identity_refuses_a_source_census_with_skipped_rows() {
        let mut digests = std::collections::BTreeSet::new();
        for malformed in [Value::Null, json!(7), json!({"div_type": 51}),
            json!({"div_id": 0}), json!({"div_id": -1}), json!({"div_id": 4294967296u64})] {
            let mut raw = input();
            raw["divisions"].as_array_mut().unwrap().push(malformed);
            let world = frame(42, raw);
            assert_eq!(world.division_units_status(), ViewStatus::Present);
            assert_eq!(world.division_units().len(), 1);
            let provider = Xmap13CombatIdentityProvider::from_world_frame(&world, &"a".repeat(64), None).unwrap();
            assert_eq!(provider.division_status(), ViewStatus::DomainUnavailable);
            assert_eq!(provider.division_count(), None);
            assert!(provider.division_identities(7).is_empty());
            assert!(provider.lookup(&identity(51, 7)).is_none());
            assert!(digests.insert(provider.census_sha256().to_owned()));
        }
    }

    #[test]
    fn combat_identity_does_not_call_unknown_censuses_empty() {
        let mut digests = std::collections::BTreeSet::new();
        for (raw, expected) in [(json!({}), ViewStatus::DomainAbsent),
            (json!({"divisions": null}), ViewStatus::DomainUnavailable),
            (json!({"divisions": []}), ViewStatus::Present)] {
            let world = frame(42, raw);
            let provider = Xmap13CombatIdentityProvider::from_world_frame(&world, &"a".repeat(64), None).unwrap();
            assert_eq!(provider.division_status(), expected);
            assert_eq!(provider.division_count(), (expected == ViewStatus::Present).then_some(0));
            assert!(digests.insert(provider.census_sha256().to_owned()));
        }
        let world = frame(42, input());
        for digest in ["", "not-a-receipt"] {
            assert!(Xmap13CombatIdentityProvider::from_world_frame(&world, digest, None).is_err());
        }
    }
}
