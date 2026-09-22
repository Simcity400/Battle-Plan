//! Authored equipment links for technology localisation. Equipment-unlocking
//! technologies can use the equipment's generic name/description instead of a
//! technology-localisation key (e.g. advanced_infantry_weapons in infantry.txt).
use crate::{ResolvedLayers, parser};
use std::{collections::BTreeMap, io};

pub fn parse_equipment(input: &[u8]) -> io::Result<BTreeMap<String, Option<String>>> {
    let document = parser::parse(input)?;
    let mut result = BTreeMap::new();
    for root in document.get_all(b"technologies") {
        for technology in root.items().unwrap_or_default() {
            let (Some(key), Some(fields)) = (&technology.key, technology.value.items()) else {
                continue;
            };
            let Ok(token) = std::str::from_utf8(key) else {
                continue;
            };
            // An override without an unlock clears the earlier link.
            result.insert(token.to_owned(), None);
            let Some(equipment) = fields
                .iter()
                .rev()
                .find(|field| field.key.as_deref() == Some(b"enable_equipments"))
                .and_then(|field| field.value.items())
            else {
                continue;
            };
            // Multiple unlocks and scripts are ambiguous; never choose an
            // arbitrary first equipment as the name of the whole technology.
            if equipment.len() != 1 || equipment[0].key.is_some() {
                continue;
            }
            if let Some(equipment) = equipment[0].value.scalar_str() {
                result.insert(token.to_owned(), Some(equipment.to_owned()));
            }
        }
    }
    Ok(result)
}

pub fn read_equipment(
    layers: &ResolvedLayers,
) -> io::Result<(BTreeMap<String, String>, Vec<String>)> {
    let mut paths = layers
        .paths()
        .filter(|p| p.starts_with("common/technologies/") && p.ends_with(".txt"))
        .map(|p| (layers.layer_index(p).unwrap_or(0), p.to_owned()))
        .collect::<Vec<_>>();
    paths.sort();
    let mut result = BTreeMap::new();
    let mut warnings = Vec::new();
    for (_, path) in paths {
        match parse_equipment(&layers.read(&path)?) {
            Ok(definitions) => result.extend(definitions),
            Err(error) => warnings.push(format!(
                "Technology equipment links unavailable in {path}: {error}"
            )),
        }
    }
    Ok((
        result
            .into_iter()
            .filter_map(|(token, equipment)| equipment.map(|equipment| (token, equipment)))
            .collect(),
        warnings,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equipment_localisation_follows_only_an_unambiguous_authored_unlock() {
        let links = parse_equipment(
            br#"technologies = {
            @cost = 1
            weapons = { enable_equipments = { infantry_equipment_3 } }
            cars = { enable_equipments = { armored_car_equipment_2 } }
            multiple = { enable_equipments = { a b } }
            scripted = { enable_equipments = { if = { limit = { always = yes } a } } }
            modifier_only = { research_cost = 2 }
        }"#,
        )
        .unwrap();
        assert_eq!(links["weapons"].as_deref(), Some("infantry_equipment_3"));
        assert_eq!(links["cars"].as_deref(), Some("armored_car_equipment_2"));
        for token in ["multiple", "scripted", "modifier_only"] {
            assert_eq!(links[token], None);
        }
        let overrides =
            parse_equipment(br#"technologies = { a = { enable_equipments = { one } } a = {} }"#)
                .unwrap();
        assert_eq!(overrides["a"], None);
    }
}
