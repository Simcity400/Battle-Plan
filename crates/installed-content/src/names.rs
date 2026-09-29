//! The country name lists (`common/names`, the names database
//! (140A5BCF0), for an idea the game names without a localisation key
//! (CIdea::GetDisplayNameForCountry 140FBCF90 -> 140FBBEE0).
//!
//! 140A5C200 lists the folder (`.txt`, depth 0; [`script_files::paths`])
//! and 140A5AF10 reads each file to its end: `default` (0x2C8D) reads into
//! the default names (+0x68), so its lists add up across blocks and files;
//! any other key names a country's entry by its text, read into a new
//! CCountryNames whose pools then replace the entry's (140A59DD0,
//! 140A5ABC0). CCountryNames::vf4 (140A5C520): `male` (0x31E7) and `female`
//! (0x2A15) are SNamesPools, `ace` (0x31E2) an SAce, anything else the
//! general pool's field (+0x298). SNamesPool::vf4 (140A5C790): `names`
//! (0x31DF), `surnames` (0x31E0), `callsigns`, `first`, `second` through
//! ReadStringList (each append, 1401426E0), `operation` its own loop of
//! lists, `prefix` and the operation suffixes ReadStringInto,
//! `use_geographical_default_operation_names` read_bool_field; anything
//! else ReportUnexpectedToken. The draw reads only the male and female
//! pools' names and surnames.

use crate::{
    ResolvedLayers,
    parser::{
        Item,
        native::{self, Shape},
    },
    script_files,
};
use std::{collections::BTreeMap, io};

/// One SNamesPool's lists the draw reads, each entry the bytes
/// ReadStringInto stored (1424ABB50 filters nothing), so every entry counts
/// in the draw whatever its encoding; the caller converts the drawn one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pool {
    pub names: Vec<Vec<u8>>,
    pub surnames: Vec<Vec<u8>>,
}

/// One CCountryNames: its male (+0x8) and female (+0x150) pools.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CountryNames {
    pub male: Pool,
    pub female: Pool,
}

impl CountryNames {
    /// 140A5BCF0's first test: names and surnames in either pool.
    fn has_names(&self) -> bool {
        self.male.names.len() + self.female.names.len() != 0
            && self.male.surnames.len() + self.female.surnames.len() != 0
    }
}

/// The names database.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NameLists {
    default: CountryNames,
    /// By the key's bytes, exactly (FNV-1a, 140A5BF90).
    countries: BTreeMap<Vec<u8>, CountryNames>,
    /// Files whose reading stops short, with the cause: their later
    /// entries are unknown.
    pub unknown: Vec<String>,
}

/// Which reader a field goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node {
    Top,
    Names,
    Pool,
    Operation,
    Ace,
}

fn pool_field(field: &native::Field<'_>) -> Shape<Node> {
    match field.keyword().as_deref() {
        Some(b"names" | b"surnames" | b"callsigns" | b"first" | b"second") => Shape::List,
        Some(b"operation") => Shape::Object(Node::Operation),
        Some(
            b"prefix"
            | b"offensive_operation_suffix"
            | b"defensive_operation_suffix"
            | b"naval_operation_suffix"
            | b"use_geographical_default_operation_names",
        ) => Shape::One,
        // ReportUnexpectedToken (1424AD100 -> 1424ACF90 -> 1424AD270).
        _ => Shape::Skip,
    }
}

/// One file as 140A5AF10 reads it.
fn read_file(input: &[u8]) -> native::Read {
    native::read(input, Node::Top, &mut |node, field| match node {
        Node::Top => Shape::Object(Node::Names),
        Node::Names => match field.keyword().as_deref() {
            Some(b"male" | b"female") => Shape::Object(Node::Pool),
            Some(b"ace") => Shape::Object(Node::Ace),
            _ => pool_field(field),
        },
        Node::Pool => pool_field(field),
        // The `operation` loop: each entry's key through ParseTokenInt32,
        // its value ReadStringList.
        Node::Operation => Shape::List,
        // SAce::vf4 (140A5C710): `name`, `type`, `surname`, `callsign`
        // ReadStringInto, `is_female` read_bool_field.
        Node::Ace => match field.keyword().as_deref() {
            Some(b"name" | b"type" | b"surname" | b"callsign" | b"is_female") => Shape::One,
            _ => Shape::Skip,
        },
    })
}

/// A list's entries as ReadStringList stores them (ReadStringInto, check on).
fn strings(items: &[Item], key: &str) -> Vec<Vec<u8>> {
    items
        .iter()
        .filter(|item| item.keyword_is(key))
        .flat_map(|item| item.value.items().unwrap_or_default())
        .filter_map(|entry| entry.value.read_string().map(<[u8]>::to_vec))
        .collect()
}

fn pool(items: &[Item]) -> Pool {
    Pool {
        names: strings(items, "names"),
        surnames: strings(items, "surnames"),
    }
}

/// Reads one entry's pools into `names`, appending.
fn read_entry(names: &mut CountryNames, fields: &[Item]) {
    for field in fields {
        let target = if field.keyword_is("male") {
            &mut names.male
        } else if field.keyword_is("female") {
            &mut names.female
        } else {
            continue;
        };
        let read = pool(field.value.items().unwrap_or_default());
        target.names.extend(read.names);
        target.surnames.extend(read.surnames);
    }
}

impl NameLists {
    /// Adds one file.
    fn apply(&mut self, path: &str, input: &[u8]) {
        let read = read_file(input);
        for item in &read.items {
            let fields = item.value.items().unwrap_or_default();
            if item.keyword_is("default") {
                read_entry(&mut self.default, fields);
                continue;
            }
            let Some(key) = item.key.as_deref() else {
                continue;
            };
            let mut names = CountryNames::default();
            read_entry(&mut names, fields);
            self.countries.insert(key.to_vec(), names);
        }
        if let Some(unsettled) = read.unsettled {
            self.unknown.push(format!("{path}: {unsettled}"));
        }
    }

    /// The entry 140A5BF90 finds for a country: its cosmetic tag's
    /// (CCountry+0x1488) when set and listed, else its original tag's
    /// (+0x130C, CCountry::GetOriginalTagOrTag 1406DF830) or its own; the
    /// default names when that is not listed either.
    pub fn for_country(&self, cosmetic_tag: &str, tag: &str) -> &CountryNames {
        (!cosmetic_tag.is_empty())
            .then(|| self.countries.get(cosmetic_tag.as_bytes()))
            .flatten()
            .or_else(|| self.countries.get(tag.as_bytes()))
            .unwrap_or(&self.default)
    }

    /// 140A5BCF0 for a male name (140FBBEE0 passes 1): the entry's pools,
    /// or the default's when the entry has no names or no surnames in
    /// either pool; then a first name and a surname from the male pool,
    /// each index a hash of `seed`. `None` when the male pool lacks either
    /// list (140FBBEE0 then yields empty text).
    pub fn draw<'a>(&'a self, names: &'a CountryNames, seed: i32) -> Option<(&'a [u8], &'a [u8])> {
        let names = if names.has_names() {
            names
        } else if self.default.has_names() {
            &self.default
        } else {
            return None;
        };
        let pool = &names.male;
        if pool.names.is_empty() || pool.surnames.is_empty() {
            return None;
        }
        let seed = seed as u32;
        let a = seed.wrapping_mul(0x7EB8_DA44);
        let h1 = (a ^ (a >> 8)).wrapping_add(0x10D8_365E);
        let h1 = ((h1 << 8) ^ h1).wrapping_mul(0x6D22_F31B);
        let h1 = (h1 >> 8) ^ h1;
        let b = seed.wrapping_mul(0x205A_D647);
        let h2 = ((b >> 8) ^ b).wrapping_add(0xB4F3_C315);
        let h2 = ((h2 << 8) ^ h2).wrapping_mul(0x387A_9F3A);
        let h2 = (h2 >> 8) ^ h2;
        let index = |step: u32, count: usize| {
            let u = h1.wrapping_add(step.wrapping_mul(0xB529_7A4D));
            let u = ((u >> 8) ^ u).wrapping_add(0x68E3_1DA4);
            let u = ((u << 8) ^ u).wrapping_mul(0x1B56_C4E9);
            let u = ((u >> 8) ^ u).wrapping_mul(0x92D6_8CA2);
            (((u >> 8) ^ u) & 0x7FFF_FFFF) as usize % count
        };
        Some((
            &pool.names[index(h2, pool.names.len())],
            &pool.surnames[index(h2.wrapping_add(1), pool.surnames.len())],
        ))
    }
}

/// The names database of the content stack.
pub fn read(layers: &ResolvedLayers) -> io::Result<NameLists> {
    let mut lists = NameLists::default();
    for path in script_files::paths(layers, "common/names/", Some(".txt")) {
        lists.apply(&path, &layers.read(&path)?);
    }
    Ok(lists)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lists(text: &[u8]) -> NameLists {
        let mut lists = NameLists::default();
        lists.apply("common/names/test.txt", text);
        lists
    }

    /// `default` adds up; a country entry is replaced by a later one;
    /// the general pool and aces are read past; a country with no names or
    /// no surnames draws from the default.
    #[test]
    fn entries_read_and_draw_as_the_database_does() {
        let names = lists(
            b"default = { male = { names = { John } surnames = { } } surnames = { General } } \
              default = { male = { surnames = { Smith } } } \
              GER = { male = { names = { Old } surnames = { Old } } } \
              GER = { male = { names = { Karl Otto } surnames = { Weber Braun Kurz } } \
                ace = { name = Hans surname = Asch type = fighter } callsigns = { X } } \
              ITA = { female = { names = { Anna } surnames = { Rossi } } }",
        );
        assert!(names.unknown.is_empty());
        let ger = names.for_country("", "GER");
        assert_eq!(ger.male.names, [b"Karl".to_vec(), b"Otto".to_vec()]);
        assert_eq!(names.draw(ger, 0), names.draw(ger, 0));
        let (first, surname) = names.draw(ger, 7).unwrap();
        assert!([b"Karl".as_slice(), b"Otto"].contains(&first));
        assert!([b"Weber".as_slice(), b"Braun", b"Kurz"].contains(&surname));
        // A cosmetic tag the database does not list falls back to the tag.
        assert_eq!(names.for_country("GER_nazi", "GER"), ger);
        // An unlisted tag is the default; ITA has names but no male pool.
        assert_eq!(
            names.draw(names.for_country("", "SOV"), 3),
            Some((b"John".as_slice(), b"Smith".as_slice()))
        );
        assert_eq!(names.draw(names.for_country("", "ITA"), 3), None);
    }

    /// The hashes of 140A5BCF0, written out for one seed.
    #[test]
    fn the_draw_hashes_the_seed() {
        let names =
            lists(b"GER = { male = { names = { A B C D E F G } surnames = { a b c d e } } }");
        let ger = names.for_country("", "GER");
        let draws: Vec<_> = (0..4).map(|seed| names.draw(ger, seed).unwrap()).collect();
        let seed = 0_u32;
        let a = seed.wrapping_mul(0x7EB8_DA44);
        let h1 = (a ^ (a >> 8)).wrapping_add(0x10D8_365E);
        let h1 = ((h1 << 8) ^ h1).wrapping_mul(0x6D22_F31B);
        let h1 = (h1 >> 8) ^ h1;
        let h2 = 0xB4F3_C315_u32;
        let h2 = ((h2 << 8) ^ h2).wrapping_mul(0x387A_9F3A);
        let h2 = (h2 >> 8) ^ h2;
        let u = h1.wrapping_add(h2.wrapping_mul(0xB529_7A4D));
        let u = ((u >> 8) ^ u).wrapping_add(0x68E3_1DA4);
        let u = ((u << 8) ^ u).wrapping_mul(0x1B56_C4E9);
        let u = ((u >> 8) ^ u).wrapping_mul(0x92D6_8CA2);
        let first =
            ["A", "B", "C", "D", "E", "F", "G"][(((u >> 8) ^ u) & 0x7FFF_FFFF) as usize % 7];
        assert_eq!(draws[0].0, first.as_bytes());
    }

    /// ReadStringInto stores an entry's bytes whatever they are, so a name
    /// that is not UTF-8 still counts in the pool the draw indexes.
    #[test]
    fn an_entry_that_is_not_utf8_keeps_its_place_in_the_pool() {
        let names = lists(b"GER = { male = { names = { A \"\xfe\" C } surnames = { s } } }");
        let ger = names.for_country("", "GER");
        assert_eq!(ger.male.names, [b"A".to_vec(), vec![0xfe], b"C".to_vec()]);
    }
}
