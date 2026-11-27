
use std::{collections::BTreeMap, io};

use super::sub_units::sub_unit_files;
use content_reader_layers::ResolvedLayers;
use crate::equipment::Database;
use content_reader_parser::{fixed5::authored_fixed5, parser::{self, Document, Value, fold_token}};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SubUnitNeeds {
    pub needs: BTreeMap<String, Vec<(String, i64)>>,
    pub dropped: Vec<String>,
}

impl SubUnitNeeds {
    pub fn read(layers: &ResolvedLayers, database: &Database) -> io::Result<Self> {
        let mut documents = Vec::new();
        for path in sub_unit_files(layers)? {
            let shown = path.text();
            let document = parser::parse(&path.read(layers)?).map_err(|error| invalid(format!("{shown}: {error}")))?;
            documents.push((shown.into_owned(), document));
        }
        Self::from_documents(&documents, |name| archetype(database, name))
    }

    pub fn from_documents(documents: &[(String, Document)], archetype: impl Fn(&str) -> Option<io::Result<String>>) -> io::Result<Self> {
        let mut authored: BTreeMap<String, (String, Vec<(String, i64)>)> = BTreeMap::new();
        for (shown, document) in documents {
            for root in document.items.iter().filter(|root| root.keyword_is("sub_units")) {
                for item in root.value.items().unwrap_or_default() {
                    let (Some(key), Some(fields)) = (item.key.as_deref(), item.value.items()) else {
                        continue;
                    };
                    let spelling = text(key).map_err(|error| invalid(format!("{shown}: {error}")))?;
                    let mut need = Vec::new();
                    for field in fields.iter().filter(|field| field.keyword_is("need")) {
                        need.extend(entries(&field.value).map_err(|error| invalid(format!("{shown}: {error}")))?);
                    }
                    let identity = fold_token(spelling);
                    let key = authored.get(&identity).map_or_else(|| spelling.to_owned(), |(key, _)| key.clone());
                    authored.insert(identity, (key, need));
                }
            }
        }
        let mut out = Self::default();
        for (identity, (key, need)) in authored {
            let canonical = canonical(&key, &need, &archetype, &mut out.dropped)?;
            out.needs.insert(identity, canonical);
        }
        Ok(out)
    }

    pub fn get(&self, key: &str) -> Option<&[(String, i64)]> {
        self.needs.get(&fold_token(key)).map(Vec::as_slice)
    }
}

pub fn entries(value: &Value) -> io::Result<Vec<(String, i64)>> {
    let items = value.items().ok_or_else(|| invalid("model content field is not a block"))?;
    let mut out = Vec::with_capacity(items.len());
    for need in items {
        let name = need.key.as_deref().ok_or_else(|| invalid("unkeyed subunit equipment requirement"))?;
        let amount = need.value.scalar_str().ok_or_else(|| invalid("model content field is not a UTF8 scalar"))?;
        out.push((text(name)?.to_owned(), authored_fixed5(amount.as_bytes())?));
    }
    Ok(out)
}

pub fn canonical(key: &str, need: &[(String, i64)], archetype: &impl Fn(&str) -> Option<io::Result<String>>, dropped: &mut Vec<String>) -> io::Result<Vec<(String, i64)>> {
    let mut out = Vec::<(String, i64)>::new();
    for (name, amount) in need {
        let Some(archetype) = archetype(name) else {
            dropped.push(format!("sub-unit {key} needs {name}, which is no equipment"));
            continue;
        };
        let archetype = archetype?;
        match out.iter_mut().find(|(held, _)| *held == archetype) {
            Some((_, value)) => *value = *amount,
            None => out.push((archetype, *amount)),
        }
    }
    Ok(out)
}

pub fn archetype(database: &Database, name: &str) -> Option<io::Result<String>> {
    let equipment = database.named(name)?;
    Some(
        equipment
            .archetype
            .ok_or_else(|| invalid(format!("equipment {name} has no archetype in the content")))
            .and_then(|at| named(database, at)),
    )
}

fn named(database: &Database, at: usize) -> io::Result<String> {
    let equipment = &database.equipment[at];
    equipment
        .name_str()
        .map(str::to_owned)
        .map_err(|refused| invalid(format!("equipment {} cannot be named: {refused}", String::from_utf8_lossy(&equipment.name))))
}

fn text(bytes: &[u8]) -> io::Result<&str> {
    std::str::from_utf8(bytes).map_err(|_| invalid("non-UTF8 authored model key"))
}

fn invalid(message: impl Into<String>) -> io::Error {
    content_reader_parser::fields::invalid(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archetypes(name: &str) -> Option<io::Result<String>> {
        match name {
            "infantry_equipment_1" | "infantry_equipment_2" => Some(Ok("infantry_equipment".to_owned())),
            "support_equipment_1" => Some(Ok("support_equipment".to_owned())),
            "broken" => Some(Err(invalid("equipment broken has no archetype in the content"))),
            _ => None,
        }
    }

    fn documents(texts: &[&str]) -> Vec<(String, Document)> {
        texts.iter().enumerate().map(|(at, text)| (format!("units/{at}.txt"), parser::parse(text.as_bytes()).unwrap())).collect()
    }

    #[test]
    fn needs_resolve_to_archetypes_in_first_order_and_a_redefinition_starts_them_again() {
        let read = SubUnitNeeds::from_documents(
            &documents(&[
                "sub_units = { Infantry = { need = { infantry_equipment_1 = 100 support_equipment_1 = 0.123456 infantry_equipment_2 = 120 ghost = 3 } } \
                 engineer = { need = { support_equipment_1 = 10 } } }",
                "sub_units = { engineer = { sprite = x } INFANTRY = { need = { support_equipment_1 = 1 } need = { infantry_equipment_1 = 5 } } }",
            ]),
            archetypes,
        )
        .unwrap();
        assert_eq!(read.get("infantry").unwrap(), [("support_equipment".to_owned(), 100_000), ("infantry_equipment".to_owned(), 500_000)]);
        assert!(read.get("Engineer").unwrap().is_empty(), "a redefinition without need needs nothing");
        assert!(read.dropped.is_empty(), "the replaced declaration's names are not resolved");
        let first = SubUnitNeeds::from_documents(&documents(&["sub_units = { A = { need = { infantry_equipment_1 = 100 support_equipment_1 = 0.123456 ghost = 3 infantry_equipment_2 = 120 } } }"]), archetypes).unwrap();
        assert_eq!(first.get("a").unwrap(), [("infantry_equipment".to_owned(), 12_000_000), ("support_equipment".to_owned(), 12_345)]);
        assert_eq!(first.dropped, ["sub-unit A needs ghost, which is no equipment"]);
        assert!(SubUnitNeeds::from_documents(&documents(&["sub_units = { a = { need = { broken = 1 } } }"]), archetypes).is_err());
    }
}
