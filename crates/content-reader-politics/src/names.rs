
use content_reader_layers::{ResolvedLayers, script_files};
use content_reader_parser::parser::Item;
use content_reader_script::native::{self, Shape};
use std::{collections::BTreeMap, io};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pool {
    pub names: Vec<Vec<u8>>,
    pub surnames: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CountryNames {
    pub male: Pool,
    pub female: Pool,
}

impl CountryNames {
    fn has_names(&self) -> bool {
        self.male.names.len() + self.female.names.len() != 0
            && self.male.surnames.len() + self.female.surnames.len() != 0
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NameLists {
    default: CountryNames,
    countries: BTreeMap<Vec<u8>, CountryNames>,
    pub unknown: Vec<String>,
}

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
        _ => Shape::Skip,
    }
}

fn read_file(input: &[u8]) -> native::Read {
    native::read(
        input,
        native::Mode::SkipMark,
        native::Top::File(Node::Top),
        None,
        &mut |node, field| match node {
            Node::Top => Shape::Object(Node::Names),
            Node::Names => match field.keyword().as_deref() {
                Some(b"male" | b"female") => Shape::Object(Node::Pool),
                Some(b"ace") => Shape::Object(Node::Ace),
                _ => pool_field(field),
            },
            Node::Pool => pool_field(field),
            Node::Operation => Shape::List,
            Node::Ace => match field.keyword().as_deref() {
                Some(b"name" | b"type" | b"surname" | b"callsign" | b"is_female") => Shape::One,
                _ => Shape::Skip,
            },
        },
    )
}

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

    pub fn for_cosmetic(&self, cosmetic_tag: &str) -> Option<&CountryNames> {
        (!cosmetic_tag.is_empty())
            .then(|| self.countries.get(cosmetic_tag.as_bytes()))
            .flatten()
    }

    pub fn for_country(&self, cosmetic_tag: &str, tag: &str) -> &CountryNames {
        self.for_cosmetic(cosmetic_tag)
            .or_else(|| self.countries.get(tag.as_bytes()))
            .unwrap_or(&self.default)
    }

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

pub fn read(layers: &ResolvedLayers) -> io::Result<NameLists> {
    let mut lists = NameLists::default();
    for path in script_files::paths(layers, "common/names/", Some(".txt"))? {
        lists.apply(&path.text(), &path.read(layers)?);
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
        assert_eq!(names.for_country("GER_nazi", "GER"), ger);
        assert_eq!(
            names.draw(names.for_country("", "SOV"), 3),
            Some((b"John".as_slice(), b"Smith".as_slice()))
        );
        assert_eq!(names.draw(names.for_country("", "ITA"), 3), None);
    }

    #[test]
    fn the_draw_hashes_the_seed() {
        let names = lists(
            b"SYN = { male = { names = { Aster Brin Coda Dune Eira Flux } surnames = { Quill Stone Vale Wren Yarrow } } }",
        );
        let pool = names.for_country("", "SYN");
        for (seed, first, surname) in [
            (19, "Brin", "Yarrow"),
            (-43, "Dune", "Stone"),
            (1_024, "Flux", "Stone"),
            (i32::MIN, "Brin", "Wren"),
            (i32::MAX, "Flux", "Wren"),
        ] {
            assert_eq!(
                names.draw(pool, seed),
                Some((first.as_bytes(), surname.as_bytes())),
                "seed {seed}"
            );
        }
    }

    #[test]
    fn an_entry_that_is_not_utf8_keeps_its_place_in_the_pool() {
        let names = lists(b"GER = { male = { names = { A \"\xfe\" C } surnames = { s } } }");
        let ger = names.for_country("", "GER");
        assert_eq!(ger.male.names, [b"A".to_vec(), vec![0xfe], b"C".to_vec()]);
    }
}
