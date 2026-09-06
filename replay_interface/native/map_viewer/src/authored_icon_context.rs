use crate::{ContentAdmissionLease, Result};
use portable_content_manifest::{OptionalContentAdmission, paradox_text::{self, Value}};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub struct AuthoredIconContext {
    lease: ContentAdmissionLease,
    continents: Vec<String>,
    definition_path: String,
    province_path: String,
}

impl AuthoredIconContext {
    pub fn from_admission_lease(lease: &ContentAdmissionLease) -> Result<Self> {
        let config = document(&read(lease, "map/default.map")?)?;
        let file = |key: &str| -> Result<String> {
            let values = config.values_ci(key).collect::<Vec<_>>();
            let [value] = values.as_slice() else { return Err(format!("XUI13 map/default.map {key} is absent or repeated").into()); };
            let value = value.as_scalar().ok_or_else(|| format!("XUI13 map/default.map {key} is not a path"))?;
            if value.is_empty() || value.contains('\\') || value.starts_with('/') || value.split('/').any(|part| matches!(part,""|"."|"..")) {
                return Err(format!("XUI13 map/default.map {key} is not a relative map path").into());
            }
            Ok(format!("map/{value}"))
        };
        let continent_file = file("continent")?;
        let continents = continent_names(&document(&read(lease, &continent_file)?)?)?;
        Ok(Self { lease: lease.clone(), continents, definition_path: file("definitions")?, province_path: file("provinces")? })
    }

    pub fn continent_names(&self) -> BTreeSet<String> { self.continents.iter().cloned().collect() }

    /// Authored template assignment only. The caller separately admits the current runtime state slot.
    pub fn state_continents(&self) -> Result<BTreeMap<u32, Option<String>>> {
        let keys = self.lease.require_complete_admitted_dir("history/states", Some("txt"), true, "XUI13 state templates")?;
        let sources = keys.into_iter().map(|key| {
            let bytes = self.lease.read_used_content(&key)?;
            Ok((key.normalized_game_relative_path.clone(), key.normalized_game_relative_path,
                crate::latin1_bytes_to_string(&bytes)))
        }).collect::<Result<Vec<_>>>()?;
        let (states, _, _) = crate::game_data::decode_states(&sources)?;
        let (coastal, province_continent) = crate::rail_files::icon_province_inputs(
            &read(&self.lease, &self.definition_path)?, &read(&self.lease, &self.province_path)?)?;
        states.into_iter().map(|(id, state)| Ok((id,
            state_continent(&state.provinces, &province_continent, &coastal, &self.continents)?))).collect()
    }
}

fn read(lease: &ContentAdmissionLease, path: &str) -> Result<Vec<u8>> {
    match lease.optional_content(path)? {
        OptionalContentAdmission::Present(key) => lease.read_used_content(key),
        status => Err(format!("XUI13 required authored input {path}: {status:?}").into()),
    }
}

fn document(bytes: &[u8]) -> Result<Value> {
    let mut root = paradox_text::parse_str_strict(&crate::latin1_bytes_to_string(bytes))
        .map_err(|error| format!("XUI13 authored syntax: {}", error.message))?;
    paradox_text::expand_file_scoped_variables(&mut root)
        .map_err(|error| format!("XUI13 authored variable: {error}"))?;
    Ok(root)
}

fn continent_names(root: &Value) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for value in root.values_ci("continents") {
        for entry in value.as_object().ok_or("XUI13 continents is not a list")? {
            if entry.key.is_some() { return Err("XUI13 keyed continent definition needs its exact authored law".into()); }
            names.push(entry.value.as_scalar().ok_or("XUI13 continent token is not scalar")?.to_owned());
        }
    }
    Ok(names)
}

fn state_continent(provinces: &[u32], province_continent: &BTreeMap<u32,u8>,
    coastal: &BTreeSet<u32>, continents: &[String]) -> Result<Option<String>> {
    for province in provinces {
        // 140aaf580 excludes invalid province objects before appending to the state.
        let Some(&ordinal) = province_continent.get(province) else { continue; };
        if let Some(index) = ordinal.checked_sub(1) {
            if let Some(name) = continents.get(usize::from(index)) { return Ok(Some(name.clone())); }
        }
        if coastal.contains(province) { break; }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn assignment_keeps_first_valid_continent_and_stops_at_earlier_coastal_bit() {
        let provinces = [(1,0),(2,2),(3,1),(4,255)].into_iter().collect();
        let names = vec!["first".into(),"second".into()];
        assert_eq!(state_continent(&[1,2,3],&provinces,&BTreeSet::new(),&names).unwrap(),Some("second".into()));
        assert_eq!(state_continent(&[1,2,3],&provinces,&[1].into_iter().collect(),&names).unwrap(),None);
        assert_eq!(state_continent(&[2,3],&provinces,&[2].into_iter().collect(),&names).unwrap(),Some("second".into()));
        assert_eq!(state_continent(&[99,4,3],&provinces,&BTreeSet::new(),&names).unwrap(),Some("first".into()));
        assert_eq!(state_continent(&[],&provinces,&BTreeSet::new(),&names).unwrap(),None);
    }
    #[test]
    fn continent_byte_order_is_separate_from_name_identity_and_repeats_are_retained() {
        let names = continent_names(&document(b"continents = { Z A } continents = { Z }").unwrap()).unwrap();
        assert_eq!(names,["Z","A","Z"]);
        let provinces = [(1,3)].into_iter().collect();
        assert_eq!(state_continent(&[1],&provinces,&BTreeSet::new(),&names).unwrap(),Some("Z".into()));
    }
}
