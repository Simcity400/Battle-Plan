use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use super::identity;
use crate::{
    ResolvedLayers, equipment,
    parser::{
        c_string,
        native::{self, Kind, Shape},
    },
    script_files,
};

/// The sub-unit database indices, which order `sub_unit_modifiers`.
/// The database reads `common/units/unit_modifiers`, then `common/unit_tags`,
/// then `common/units/critical_parts` and `common/units`: each directory's
/// own `.txt` files ([`script_files::paths`]), without subdirectories. Every
/// `sub_units` key not seen before takes the next index after the null
/// object at 0. This is the one sub-unit database read: its indices and
/// each definition's authored group, `is_artillery_brigade` and support
/// category bit.
pub fn sub_units(layers: &ResolvedLayers) -> io::Result<SubUnits> {
    let mut indices = SubUnits::default();
    for directory in SUB_UNIT_DIRECTORIES {
        for path in script_files::paths(layers, directory, Some(".txt")) {
            indices.read(&layers.read(&path)?, &path);
        }
    }
    Ok(indices)
}

/// Each directory the database reads, in order.
pub(super) const SUB_UNIT_DIRECTORIES: [&str; 4] = [
    "common/units/unit_modifiers/",
    "common/unit_tags/",
    "common/units/critical_parts/",
    "common/units/",
];

/// What the sub-unit database says about a `sub_unit_modifiers` key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubUnit<'a> {
    /// The database index.
    Index(u32),
    /// No such sub-unit: the modifier reader reports the key and drops it.
    Unknown,
    /// Whether the key registers is unknown, and why.
    Uncertain(&'a str),
}

/// The database's keys as the reader sees them. Lookup hashes a folded
/// copy and compares without ASCII case, so names that differ only in
/// case are one sub-unit, registered under its first spelling.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SubUnits {
    /// Identity to the first spelling and the index.
    indices: BTreeMap<Vec<u8>, (Vec<u8>, u32)>,
    /// The `sub_unit_categories` raw-list names, registered by the
    /// category constructor.
    categories: BTreeSet<Vec<u8>>,
    category_spellings: BTreeMap<Vec<u8>, Vec<u8>>,
    /// Dynamic modifier names registered from the `sub_unit_modifiers`
    /// string list.
    modifier_names: BTreeSet<Vec<u8>>,
    /// Each definition's `is_artillery_brigade` by the same byte identity
    /// as `indices`, or why the reading that gives it cannot be followed.
    artillery: BTreeMap<Vec<u8>, Result<bool, String>>,
    /// Each definition's `special_forces`, read as `is_artillery_brigade` is.
    special_forces: BTreeMap<Vec<u8>, Result<bool, String>>,
    /// The support bit, used by division-template placement.
    support: BTreeMap<Vec<u8>, Result<bool, String>>,
    /// Authored C-string spelling of the group token, or token zero.
    /// This is source text, not the lexical dictionary's label.
    groups: BTreeMap<Vec<u8>, Result<Option<Vec<u8>>, String>>,
    /// Full type mask, including the inline icon prefix categories.
    categories_by_unit: BTreeMap<Vec<u8>, Result<u64, String>>,
    /// A file whose reading the port cannot follow, and why: any key not
    /// registered by then may or may not be.
    uncertain: Option<String>,
    warnings: Vec<String>,
}

impl SubUnits {
    /// Every registered sub-unit spelling, for token-based reader lookups
    /// against the same database.
    pub fn registered(&self) -> impl Iterator<Item = &[u8]> {
        self.indices
            .values()
            .map(|(spelling, _)| spelling.as_slice())
    }

    /// The category names registered by `sub_unit_categories`, folded
    /// with dynamic-token identity.
    pub(crate) fn categories(&self) -> &BTreeSet<Vec<u8>> {
        &self.categories
    }

    /// Find the category by token; its localisable name keeps the
    /// first registered spelling.
    pub fn category_name(&self, token: &crate::tokens::Token) -> Result<Option<&str>, String> {
        let crate::tokens::Token::Name(name) = token else {
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

    /// Names this database registers in the modifier registry.
    pub(crate) fn modifiers(&self) -> &BTreeSet<Vec<u8>> {
        &self.modifier_names
    }

    /// Why an unread name may still have been registered.
    pub(crate) fn gap(&self) -> Option<&str> {
        self.uncertain.as_deref()
    }

    /// The key's index, without case.
    pub fn get(&self, key: &str) -> SubUnit<'_> {
        match self.indices.get(&identity(key.as_bytes())) {
            Some((_, index)) => SubUnit::Index(*index),
            None => match &self.uncertain {
                Some(reason) => SubUnit::Uncertain(reason),
                None => SubUnit::Unknown,
            },
        }
    }

    /// The registered spelling of the sub-unit at `index`, for its name;
    /// `Err` names one that is not UTF-8.
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

    /// Scan real definitions in database order by token, then return
    /// the null object's empty name. Rejected names all share token zero,
    /// so their first definition wins too; an unread tail cannot prove
    /// a missing match.
    pub fn token_name(&self, token: &crate::tokens::Token) -> Result<&str, String> {
        let found = self
            .indices
            .values()
            .filter(|(spelling, _)| crate::tokens::Token::c_str(spelling) == *token)
            .min_by_key(|(_, index)| *index);
        match found {
            Some((spelling, _)) => {
                std::str::from_utf8(crate::parser::c_string(spelling)).map_err(|_| {
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

    /// Where the database's reading leaves the text, and what cannot be
    /// followed.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// A definition's `is_artillery_brigade`, found without case: absent is
    /// `no`. `Err` names why the reading that gives it cannot be followed,
    /// or why a name no definition has may still be one.
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

    /// A definition's `special_forces`, found without case: absent is `no`. `Err`
    /// names why the reading that gives it cannot be followed, or why a name no
    /// definition has may still be one.
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

    /// The support category (bit 24), by the same database identity.
    /// An unknown definition or unread tail is refused.
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

    /// Authored group spelling: interns the resolved value's C string.
    /// Token zero has no group. Construction clears it; repeated-body
    /// reset keeps it.
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

    /// The type mask, reset by each definition appearance and ORed
    /// by repeated `type` fields.
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

    /// One file through `parser::native`: at the top level `sub_units` is
    /// its key loop. Every key finds or creates a definition, first appearance
    /// taking the next index, then reads its body by key; any other key is
    /// reported and skipped. `is_artillery_brigade` takes only exact `yes` or
    /// `no` and otherwise leaves the value. Each appearance starts at `no`,
    /// from construction or the reset before a repeated body's read. So the
    /// last appearance read gives the value.
    /// `type` ORs categories into the mask: the scalar uses the current lexer
    /// token; the do/while raw list resolves its tokens into the lexer. Numeric
    /// and quoted categories have no support bit. Construction and reset
    /// clear this mask; repeated type fields within one appearance OR.
    /// `group` interns the resolved value's C string; reset leaves a prior
    /// group untouched.
    pub fn read(&mut self, bytes: &[u8], path: &str) {
        #[derive(Clone, Copy)]
        enum Node {
            Top,
            Table,
            Body,
        }
        let mut keys = Vec::new();
        // Scalar type reads the lexer, not the resolved/stale value slot.
        // Capture its bit before Current shapes the returned Item.
        let mut scalar_categories = Vec::new();
        let support_bit = equipment::category_mask("support")
            .expect("equipment categories define the support category");
        let category = |text: &[u8], kind: Kind| {
            if kind == Kind::Bare {
                equipment::category_mask(c_string(text)).unwrap_or(0)
            } else {
                0
            }
        };
        // Each file constructs the lexer in mode 0.
        let read = native::read(
            bytes,
            native::Mode::KeepMark,
            native::Top::File(Node::Top),
            None,
            &mut |node, field| match node {
                Node::Top if field.keyword_is("sub_units") => Shape::Object(Node::Table),
                // Resolve raw category tokens without updating the value slot.
                Node::Top if field.keyword_is("sub_unit_categories") => Shape::RawList,
                // Read a string list, then register each modifier.
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
                    && let crate::tokens::Token::Name(name) = crate::tokens::Token::c_str(bytes)
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
                    && let crate::tokens::Token::Name(name) = crate::tokens::Token::c_str(bytes)
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
            // Reset categories, artillery and special forces, but preserve the group.
            let mut artillery = false;
            let mut special_forces = false;
            let mut categories = scalar_categories[index];
            let unit = identity(entry.key.as_deref().unwrap_or_default());
            let mut group = self.groups.get(&unit).cloned().unwrap_or_else(|| {
                self.uncertain
                    .as_ref()
                    .map_or(Ok(None), |why| Err(why.clone()))
            });
            for field in entry.value.items().unwrap_or_default() {
                if field.keyword_is("is_artillery_brigade") {
                    // The artillery boolean field leaves its byte unchanged on a
                    // failed scan.
                    artillery = field
                        .value
                        .scalar()
                        .and_then(crate::parser::read_bool)
                        .unwrap_or(artillery);
                } else if field.keyword_is("special_forces") {
                    special_forces = field
                        .value
                        .scalar()
                        .and_then(crate::parser::read_bool)
                        .unwrap_or(special_forces);
                } else if field.keyword_is("type") {
                    // Raw category tokens resolve into the lexer and never update
                    // the reader's value slot.
                    for item in field.value.items().unwrap_or_default() {
                        if let Some(text) = item.value.scalar() {
                            categories |= category(text, item.kind);
                        }
                    }
                } else if field.keyword_is("group") {
                    let spelling = c_string(field.value.scalar().unwrap_or_default());
                    // Reject empty and numeric-leading token names.
                    group = Ok((!crate::tokens::Token::register(spelling).is_rejected())
                        .then(|| spelling.to_vec()));
                }
            }
            self.groups.insert(unit, group);
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
                Ok(categories & support_bit != 0),
            );
            self.categories_by_unit.insert(
                identity(entry.key.as_deref().unwrap_or_default()),
                Ok(categories),
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
            // The unread tail may define any name again, which may reset
            // and assign its value.
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
            if let Some(key) = keys.last() {
                self.artillery.insert(identity(key), Err(reason.clone()));
                self.special_forces
                    .insert(identity(key), Err(reason.clone()));
                self.support.insert(identity(key), Err(reason.clone()));
                self.categories_by_unit
                    .insert(identity(key), Err(reason.clone()));
                self.groups.insert(identity(key), Err(reason.clone()));
            }
            self.uncertain.get_or_insert(reason);
        }
    }
}
