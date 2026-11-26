use std::{
    collections::{BTreeMap, BTreeSet},
};

use crate::{equipment_stats, native::{self, Kind, Shape}};
use content_reader_parser::parser::c_string;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubUnit<'a> {
    Index(u32),
    Unknown,
    Uncertain(&'a str),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SubUnits {
    indices: BTreeMap<Vec<u8>, (Vec<u8>, u32)>,
    categories: BTreeSet<Vec<u8>>,
    category_spellings: BTreeMap<Vec<u8>, Vec<u8>>,
    modifier_names: BTreeSet<Vec<u8>>,
    artillery: BTreeMap<Vec<u8>, Result<bool, String>>,
    special_forces: BTreeMap<Vec<u8>, Result<bool, String>>,
    support: BTreeMap<Vec<u8>, Result<bool, String>>,
    groups: BTreeMap<Vec<u8>, Result<Option<Vec<u8>>, String>>,
    categories_by_unit: BTreeMap<Vec<u8>, Result<u64, String>>,
    air_wings: BTreeMap<Vec<u8>, Result<AirWing, String>>,
    uncertain: Option<String>,
    warnings: Vec<String>,
}

const NAVAL_CATEGORIES: u64 = 0x80_0040_03C1;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AirWing {
    pub sizes: [i32; 4],
    pub first_requirement: Option<Vec<u8>>,
}

impl SubUnits {
    pub fn air_wing(&self, name: &[u8]) -> Result<&AirWing, String> {
        self
            .air_wings
            .get(&identity(name))
            .ok_or_else(|| {
                self.uncertain.clone().unwrap_or_else(|| {
                    format!(
                        "sub-unit {} is not in the content's sub_units",
                        String::from_utf8_lossy(name)
                    )
                })
            })?
            .as_ref()
            .map_err(Clone::clone)
    }

    pub fn registered(&self) -> impl Iterator<Item = &[u8]> {
        self.indices
            .values()
            .map(|(spelling, _)| spelling.as_slice())
    }

    pub fn in_content_order(&self) -> impl Iterator<Item = &[u8]> {
        let mut units: Vec<_> = self.indices.values().collect();
        units.sort_unstable_by_key(|(_, index)| *index);
        units.into_iter().map(|(spelling, _)| spelling.as_slice())
    }

    pub fn naval(&self, name: &[u8]) -> Result<bool, String> {
        self.category(name).map(|categories| categories & NAVAL_CATEGORIES != 0)
    }

    pub fn categories(&self) -> &BTreeSet<Vec<u8>> {
        &self.categories
    }

    pub fn category_name(&self, token: &content_reader_parser::tokens::Token) -> Result<Option<&str>, String> {
        let content_reader_parser::tokens::Token::Name(name) = token else {
            return Ok(None);
        };
        match self.category_spellings.get(name) {
            Some(spelling) => std::str::from_utf8(c_string(spelling))
                .map(Some)
                .map_err(|_| {
                    format!(
                        "sub-unit category {} is not UTF-8",
                        String::from_utf8_lossy(spelling)
                    )
                }),
            None => match &self.uncertain {
                Some(why) => Err(format!("sub-unit category lookup is uncertain: {why}")),
                None => Ok(None),
            },
        }
    }

    pub fn modifiers(&self) -> &BTreeSet<Vec<u8>> {
        &self.modifier_names
    }

    pub fn gap(&self) -> Option<&str> {
        self.uncertain.as_deref()
    }

    pub fn get(&self, key: &str) -> SubUnit<'_> {
        match self.indices.get(&identity(key.as_bytes())) {
            Some((_, index)) => SubUnit::Index(*index),
            None => match &self.uncertain {
                Some(reason) => SubUnit::Uncertain(reason),
                None => SubUnit::Unknown,
            },
        }
    }

    pub fn name(&self, index: u32) -> Option<Result<&str, String>> {
        self.indices
            .values()
            .find(|(_, candidate)| *candidate == index)
            .map(|(spelling, _)| {
                std::str::from_utf8(spelling).map_err(|_| {
                    format!(
                        "sub-unit {}'s name is not UTF-8",
                        String::from_utf8_lossy(spelling)
                    )
                })
            })
    }

    pub fn token_name(&self, token: &content_reader_parser::tokens::Token) -> Result<&str, String> {
        let found = self
            .indices
            .values()
            .filter(|(spelling, _)| content_reader_parser::tokens::Token::c_str(spelling) == *token)
            .min_by_key(|(_, index)| *index);
        match found {
            Some((spelling, _)) => {
                std::str::from_utf8(content_reader_parser::parser::c_string(spelling)).map_err(|_| {
                    format!(
                        "sub-unit {}'s name is not UTF-8",
                        String::from_utf8_lossy(spelling)
                    )
                })
            }
            None => match &self.uncertain {
                Some(reason) => Err(format!("sub-unit token lookup is uncertain: {reason}")),
                None => Ok(""),
            },
        }
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub fn artillery(&self, name: &[u8]) -> Result<bool, String> {
        match self.artillery.get(&identity(name)) {
            Some(value) => value.clone(),
            None => match &self.uncertain {
                Some(reason) => Err(reason.clone()),
                None => Err(format!(
                    "sub-unit {} is not in the content's sub_units",
                    String::from_utf8_lossy(name)
                )),
            },
        }
    }

    pub fn special_forces(&self, name: &[u8]) -> Result<bool, String> {
        match self.special_forces.get(&identity(name)) {
            Some(value) => value.clone(),
            None => match &self.uncertain {
                Some(reason) => Err(reason.clone()),
                None => Err(format!(
                    "sub-unit {} is not in the content's sub_units",
                    String::from_utf8_lossy(name)
                )),
            },
        }
    }

    pub fn support(&self, name: &[u8]) -> Result<bool, String> {
        match self.support.get(&identity(name)) {
            Some(value) => value.clone(),
            None => match &self.uncertain {
                Some(reason) => Err(reason.clone()),
                None => Err(format!(
                    "sub-unit {} is not in the content's sub_units",
                    String::from_utf8_lossy(name)
                )),
            },
        }
    }

    pub fn group(&self, name: &[u8]) -> Result<Option<&str>, String> {
        match self.groups.get(&identity(name)) {
            Some(Ok(Some(group))) => std::str::from_utf8(group).map(Some).map_err(|_| {
                format!(
                    "sub-unit {}'s group is not UTF-8",
                    String::from_utf8_lossy(name)
                )
            }),
            Some(Ok(None)) => Ok(None),
            Some(Err(reason)) => Err(reason.clone()),
            None => Err(self.uncertain.clone().unwrap_or_else(|| {
                format!(
                    "sub-unit {} is not in the content's sub_units",
                    String::from_utf8_lossy(name)
                )
            })),
        }
    }

    pub fn category(&self, name: &[u8]) -> Result<u64, String> {
        match self.categories_by_unit.get(&identity(name)) {
            Some(value) => value.clone(),
            None => Err(self.uncertain.clone().unwrap_or_else(|| {
                format!(
                    "sub-unit {} is not in the content's sub_units",
                    String::from_utf8_lossy(name)
                )
            })),
        }
    }

    pub fn read(&mut self, bytes: &[u8], path: &str) {
        #[derive(Clone, Copy)]
        enum Node {
            Top,
            Table,
            Body,
        }
        let mut keys = Vec::new();
        let mut scalar_categories = Vec::new();
        let support_bit = match engine_executable::executable_tables::current() {
            Some(tables) => match tables
                .auxiliary
                .as_ref()
                .and_then(|auxiliary| auxiliary.category_names.as_ref())
            {
                Ok(_) => Ok(equipment_stats::category_mask("support")
                    .expect("equipment categories define the support category")),
                Err(withheld) => Err(withheld.to_string()),
            },
            None => Ok(equipment_stats::category_mask("support")
                .expect("equipment categories define the support category")),
        };
        let category = |text: &[u8], kind: Kind| {
            if kind == Kind::Bare && support_bit.is_ok() {
                equipment_stats::category_mask(c_string(text)).unwrap_or(0)
            } else {
                0
            }
        };
        let read = native::read(
            bytes,
            native::Mode::KeepMark,
            native::Top::File(Node::Top),
            None,
            &mut |node, field| match node {
                Node::Top if field.keyword_is("sub_units") => Shape::Object(Node::Table),
                Node::Top if field.keyword_is("sub_unit_categories") => Shape::RawList,
                Node::Top if field.keyword_is("sub_unit_modifiers") => Shape::List,
                Node::Top => Shape::Skip,
                Node::Table => {
                    keys.push(field.key.text.clone());
                    scalar_categories.push(0);
                    Shape::Object(Node::Body)
                }
                Node::Body if field.keyword_is("is_artillery_brigade") => Shape::One,
                Node::Body if field.keyword_is("special_forces") => Shape::One,
                Node::Body if field.keyword_is("group") => Shape::One,
                Node::Body if field.keyword_is("type") => {
                    if field.opens() {
                        Shape::RawListFirst
                    } else {
                        *scalar_categories
                            .last_mut()
                            .expect("a definition is being read") |=
                            category(&field.current.text, field.current.kind);
                        Shape::Current
                    }
                }
                Node::Body => Shape::ByKey,
            },
        );
        for item in read
            .items
            .iter()
            .filter(|item| item.keyword_is("sub_unit_categories"))
        {
            for category in item.value.items().unwrap_or_default() {
                if let Some(bytes) = category.value.scalar()
                    && let content_reader_parser::tokens::Token::Name(name) = content_reader_parser::tokens::Token::c_str(bytes)
                {
                    self.category_spellings
                        .entry(name.clone())
                        .or_insert_with(|| c_string(bytes).to_vec());
                    self.categories.insert(name);
                }
            }
        }
        for item in read
            .items
            .iter()
            .filter(|item| item.keyword_is("sub_unit_modifiers"))
        {
            for modifier in item.value.items().unwrap_or_default() {
                if let Some(bytes) = modifier.value.read_string()
                    && let content_reader_parser::tokens::Token::Name(name) = content_reader_parser::tokens::Token::c_str(bytes)
                {
                    self.modifier_names.insert(name);
                }
            }
        }
        let entries = read
            .items
            .iter()
            .filter(|item| item.keyword_is("sub_units"))
            .flat_map(|item| item.value.items().unwrap_or_default());
        for (index, entry) in entries.enumerate() {
            let mut artillery = false;
            let mut special_forces = false;
            let mut categories = scalar_categories[index];
            let mut air_wing = AirWing::default();
            let unit = identity(entry.key.as_deref().unwrap_or_default());
            let mut group = self.groups.get(&unit).cloned().unwrap_or_else(|| {
                self.uncertain
                    .as_ref()
                    .map_or(Ok(None), |why| Err(why.clone()))
            });
            for field in entry.value.items().unwrap_or_default() {
                let wing_size = [
                    "land_air_wing_size",
                    "carrier_air_wing_size",
                    "submarine_carrier_air_wing_size",
                    "mega_carrier_air_wing_size",
                ]
                .iter()
                .position(|name| field.keyword_is(name));
                if let Some(index) = wing_size {
                    if let Some(bytes) = field.value.scalar()
                        && let content_reader_parser::crt::Scanned::Stored(value) = content_reader_parser::crt::scan_i32(bytes)
                    {
                        air_wing.sizes[index] = value;
                    }
                } else if field.keyword_is("need") && air_wing.first_requirement.is_none() {
                    air_wing.first_requirement = field
                        .value
                        .items()
                        .unwrap_or_default()
                        .first()
                        .and_then(|entry| entry.key.clone());
                }
                if field.keyword_is("is_artillery_brigade") {
                    artillery = field
                        .value
                        .scalar()
                        .and_then(content_reader_parser::parser::read_bool)
                        .unwrap_or(artillery);
                } else if field.keyword_is("special_forces") {
                    special_forces = field
                        .value
                        .scalar()
                        .and_then(content_reader_parser::parser::read_bool)
                        .unwrap_or(special_forces);
                } else if field.keyword_is("type") {
                    for item in field.value.items().unwrap_or_default() {
                        if let Some(text) = item.value.scalar() {
                            categories |= category(text, item.kind);
                        }
                    }
                } else if field.keyword_is("group") {
                    let spelling = c_string(field.value.scalar().unwrap_or_default());
                    group = Ok((!content_reader_parser::tokens::Token::register(spelling).is_rejected())
                        .then(|| spelling.to_vec()));
                }
            }
            self.groups.insert(unit, group);
            self.air_wings.insert(
                identity(entry.key.as_deref().unwrap_or_default()),
                Ok(air_wing),
            );
            self.artillery.insert(
                identity(entry.key.as_deref().unwrap_or_default()),
                Ok(artillery),
            );
            self.special_forces.insert(
                identity(entry.key.as_deref().unwrap_or_default()),
                Ok(special_forces),
            );
            self.support.insert(
                identity(entry.key.as_deref().unwrap_or_default()),
                support_bit
                    .as_ref()
                    .map(|bit| categories & bit != 0)
                    .map_err(Clone::clone),
            );
            self.categories_by_unit.insert(
                identity(entry.key.as_deref().unwrap_or_default()),
                support_bit.as_ref().map(|_| categories).map_err(Clone::clone),
            );
        }
        for misread in &read.misreads {
            self.warnings.push(format!(
                "Sub-unit database content: {path} {misread}; the database follows the game's reading."
            ));
        }
        if self.uncertain.is_none() {
            for key in &keys {
                let identity = identity(key);
                if !self.indices.contains_key(&identity) {
                    let next = self.indices.len() as u32 + 1;
                    self.indices.insert(identity, (key.clone(), next));
                }
            }
        }
        if let Some(unsettled) = read.unsettled {
            let reason = format!("{path} {unsettled}");
            let indices = if self.uncertain.is_none() {
                "every sub-unit first named after it has an unknown index, and "
            } else {
                ""
            };
            self.warnings.push(format!(
                "Sub-unit database content: {reason}; {indices}every sub-unit's is_artillery_brigade and special_forces are unknown until a later file defines it again; its support category is unknown under the same condition; its group is unknown until explicitly assigned again."
            ));
            for value in self.artillery.values_mut() {
                *value = Err(reason.clone());
            }
            for value in self.special_forces.values_mut() {
                *value = Err(reason.clone());
            }
            for value in self.support.values_mut() {
                *value = Err(reason.clone());
            }
            for value in self.categories_by_unit.values_mut() {
                *value = Err(reason.clone());
            }
            for value in self.groups.values_mut() {
                *value = Err(reason.clone());
            }
            for value in self.air_wings.values_mut() {
                *value = Err(reason.clone());
            }
            if let Some(key) = keys.last() {
                self.artillery.insert(identity(key), Err(reason.clone()));
                self.special_forces
                    .insert(identity(key), Err(reason.clone()));
                self.support.insert(identity(key), Err(reason.clone()));
                self.categories_by_unit
                    .insert(identity(key), Err(reason.clone()));
                self.groups.insert(identity(key), Err(reason.clone()));
                self.air_wings.insert(identity(key), Err(reason.clone()));
            }
            self.uncertain.get_or_insert(reason);
        }
    }
}

pub fn identity(name: &[u8]) -> Vec<u8> {
    name.to_ascii_lowercase()
}

#[cfg(test)]
mod naval_tests {
    use super::*;

    #[test]
    fn a_sub_unit_is_naval_by_its_type_categories_not_its_map_icon() {
        crate::test_game::enter();
        let mut units = SubUnits::default();
        units.read(
            b"sub_units = { line = { type = { infantry } map_icon_category = ship } escort = { type = { screen_ship } map_icon_category = infantry } diver = { type = { submarine } } plain = { } }",
            "common/units/base.txt",
        );
        assert_eq!(units.naval(b"line"), Ok(false), "the map icon is presentation; the type decides");
        assert_eq!(units.naval(b"ESCORT"), Ok(true), "names are matched as the game matches them");
        assert_eq!(units.naval(b"diver"), Ok(true));
        assert_eq!(units.naval(b"plain"), Ok(false));
        assert!(units.naval(b"absent").is_err());
        assert_eq!(units.in_content_order().collect::<Vec<_>>(), [&b"line"[..], b"escort", b"diver", b"plain"], "the content's own order, not the alphabet");
        units.read(b"sub_units = { line = { type = { capital_ship } } }", "common/units/mod_override.txt");
        assert_eq!(units.naval(b"line"), Ok(true), "a mod's later definition replaces the earlier one");
    }
}
