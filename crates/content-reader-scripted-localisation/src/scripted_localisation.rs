use content_reader_key_names::key_name_products::KeyNamesExt as _;

use std::{collections::BTreeMap, io};

use content_reader_script::native::{self, Kind, Misread, Shape};
use crate::{
    ResolvedLayers,
    key_names::KeyNames,
    parser::{
        self, Item,
    },
    script_files,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node {
    Top,
    Localization,
    Text,
    RandomList,
    Member,
    Trigger(content_reader_script::trigger_grammar::Node),
}

struct File {
    entries: Vec<(Vec<u8>, Entry)>,
    misreads: Vec<Misread>,
    unknown: Option<Misread>,
}

fn read_file(input: &[u8], names: &KeyNames) -> File {
    let mut entries = 0;
    let mut holds = false;
    let mut untraced: Option<(usize, Misread)> = None;
    let mut shaper = content_reader_script::trigger_grammar::Shaper::new(Some(names));
    let read = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::File(Node::Top),
        Some(names),
        &mut |node, field| match node {
            Node::Top => {
                entries += 1;
                Shape::Object(Node::Localization)
            }
            Node::Localization => match field.keyword().as_deref() {
                Some(b"name") => Shape::One,
                Some(b"text") => {
                    holds = false;
                    Shape::Object(Node::Text)
                }
                _ => Shape::Skip,
            },
            Node::Text => match field.keyword().as_deref() {
                Some(b"localization_key" | b"random_list") if holds => Shape::Skip,
                Some(b"localization_key") => {
                    holds = true;
                    Shape::One
                }
                Some(b"random_list") => {
                    holds = true;
                    Shape::Object(Node::RandomList)
                }
                Some(b"trigger") => Shape::Body(Node::Trigger(content_reader_script::trigger_grammar::Node::Trigger)),
                _ => Shape::Skip,
            },
            Node::RandomList
                if field.key.kind == Kind::Bare && parser::number_token(&field.key.text, false) =>
            {
                Shape::Object(Node::Member)
            }
            Node::RandomList if field.keyword_is("seed") => Shape::One,
            Node::RandomList => Shape::Skip,
            Node::Member => match field.keyword().as_deref() {
                Some(b"localization_key") if field.opens() => {
                    untraced.get_or_insert_with(|| {
                    (
                        entries - 1,
                        Misread {
                            kind: crate::DiagnosticKind::Unsettled,
                            count: None,
                            line: field.key.line,
                            what: "a random list member's `localization_key` block, whose bound localisation reader is unsupported".into(),
                        },
                    )
                });
                    Shape::SkipIfOpen
                }
                Some(b"localization_key") => Shape::One,
                Some(b"modifier") => Shape::Body(Node::Trigger(content_reader_script::trigger_grammar::Node::Modifier)),
                _ => Shape::Skip,
            },
            Node::Trigger(node) => shaper.shape(node, field).map(Node::Trigger),
        },
    );
    let (known, unknown, misreads) = match (untraced, read.unsettled) {
        (Some((entry, unknown)), _) => {
            let misreads = read
                .misreads
                .into_iter()
                .filter(|misread| misread.line < unknown.line)
                .collect();
            (entry, Some(unknown), misreads)
        }
        (None, Some(unsettled)) => (
            read.items.len().saturating_sub(1),
            Some(unsettled),
            read.misreads,
        ),
        (None, None) => (read.items.len(), None, read.misreads),
    };
    File {
        entries: read.items[..known]
            .iter()
            .map(|item| (name(item), entry(item)))
            .collect(),
        misreads,
        unknown,
    }
}

fn name(item: &Item) -> Vec<u8> {
    item.value
        .items()
        .and_then(|fields| fields.iter().rev().find(|field| field.keyword_is("name")))
        .and_then(|field| field.value.read_string())
        .unwrap_or_default()
        .to_vec()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub texts: Vec<Text>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Text {
    pub value: Option<TextValue>,
    pub trigger: crate::triggers::Triggers,
    pub trigger_present: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextValue {
    Key(Vec<u8>),
    RandomList(RandomList),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RandomList {
    pub seed: Vec<Item>,
    pub members: Vec<RandomMember>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RandomMember {
    pub weight: i64,
    pub modifiers: Vec<Vec<Item>>,
    pub key: Vec<u8>,
}

fn random_list(item: &Item) -> RandomList {
    let mut list = RandomList::default();
    for field in item.value.items().unwrap_or_default() {
        if field.keyword_is("seed") {
            list.seed.push(field.clone());
        } else if !field.key_quoted
            && field
                .key
                .as_deref()
                .is_some_and(|key| parser::number_token(key, false))
        {
            let fields = field.value.items().unwrap_or_default();
            list.members.push(RandomMember {
                weight: i64::from(crate::fixed5::decimal_i32(field.key.as_deref().unwrap()))
                    * 100_000,
                modifiers: fields
                    .iter()
                    .filter(|f| f.keyword_is("modifier"))
                    .map(|f| f.value.items().unwrap_or_default().to_vec())
                    .collect(),
                key: fields
                    .iter()
                    .rev()
                    .find(|f| f.keyword_is("localization_key"))
                    .and_then(|f| f.value.scalar())
                    .unwrap_or_default()
                    .to_vec(),
            });
        }
    }
    list
}

fn entry(item: &Item) -> Entry {
    let mut texts = Vec::new();
    for text in item
        .value
        .items()
        .unwrap_or_default()
        .iter()
        .filter(|f| f.keyword_is("text"))
    {
        let mut value = None;
        let mut trigger = crate::triggers::Triggers::default();
        let mut trigger_present = false;
        for field in text.value.items().unwrap_or_default() {
            if field.keyword_is("localization_key") && value.is_none() {
                value = Some(TextValue::Key(
                    field.value.scalar().unwrap_or_default().to_vec(),
                ));
            } else if field.keyword_is("random_list") && value.is_none() {
                value = Some(TextValue::RandomList(random_list(field)));
            } else if field.keyword_is("trigger") {
                trigger_present = true;
                trigger = crate::triggers::body(&field.value).unwrap_or_default();
            }
        }
        texts.push(Text {
            value,
            trigger,
            trigger_present,
        });
    }
    Entry { texts }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Names {
    pub entries: BTreeMap<Vec<u8>, Entry>,
    pub unknown: Vec<String>,
    pub warnings: Vec<crate::Diagnostic>,
    defined_in: BTreeMap<Vec<u8>, usize>,
    unknown_in: Vec<usize>,
}

impl Names {
    pub fn uncertain(&self, name: &[u8]) -> Option<String> {
        let defined = self.defined_in.get(name).copied();
        let unknown: Vec<&str> = self
            .unknown
            .iter()
            .zip(&self.unknown_in)
            .filter(|&(_, &file)| defined.is_none_or(|defined| file >= defined))
            .map(|(unknown, _)| unknown.as_str())
            .collect();
        (!unknown.is_empty()).then(|| unknown.join("; "))
    }
}

pub fn names(layers: &ResolvedLayers) -> io::Result<Names> {
    let files = script_files::paths(layers, "common/scripted_localisation/", Some(".txt"))?
        .into_iter()
        .map(|path| Ok((path.read(layers)?, path.to_string())))
        .collect::<io::Result<Vec<_>>>()?;
    Ok(read_files(&files, &*layers.key_names()?))
}

fn read_files<B: AsRef<[u8]>>(files: &[(B, String)], key_names: &KeyNames) -> Names {
    let mut names = Names::default();
    for (index, (bytes, path)) in files.iter().enumerate() {
        let file = read_file(bytes.as_ref(), key_names);
        names.warnings.extend(file.misreads.iter().map(|misread| {
            (
                misread.kind,
                format!("Scripted localisation in {path}: {misread}."),
            )
        }));
        if let Some(unknown) = &file.unknown {
            let unknown = format!("{path} is unknown from {unknown}");
            names.warnings.push((crate::DiagnosticKind::Unsettled, format!(
                "Scripted localisation in {unknown}; a name defined in the rest of it is unknown."
            )));
            names.unknown.push(unknown);
            names.unknown_in.push(index);
        }
        for (name, entry) in file.entries {
            names.defined_in.insert(name.clone(), index);
            names.entries.insert(name, entry);
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_name_warning_keeps_unsettled_kind() {
        content_reader_script::test_game::enter();
        let files = vec![(
            b"a = { name = Pick text = { random_list = { 1 = { localization_key = { bad = x } } } } }".to_vec(),
            "common/scripted_localisation/a.txt".to_owned(),
        )];
        let names = read_files(&files, &KeyNames::default());
        assert!(
            names
                .warnings
                .iter()
                .any(|(kind, _)| *kind == crate::DiagnosticKind::Unsettled),
            "{:?}",
            names.warnings
        );
    }

    #[test]
    fn random_lists_keep_weights_seeds_and_modifier_bodies() {
        content_reader_script::test_game::enter();
        let names = read(&[(
            "common/scripted_localisation/a.txt",
            r#"
            a = { name = Pick text = { random_list = {
                seed = 7 seed = 8
                010 = { localization_key = A modifier = { factor = 2 add = 0.5 always = no } }
                -2.9 = { localization_key = B localization_key = C }
                "4" = { localization_key = SKIPPED }
                nested = { random_list = { 1 = { localization_key = SKIPPED } } }
                0 = { }
            } localization_key = SKIPPED trigger = {} } }
        "#,
        )]);
        assert!(names.unknown.is_empty(), "{:?}", names.unknown);
        let text = &names.entries[b"Pick".as_slice()].texts[0];
        let Some(TextValue::RandomList(list)) = &text.value else {
            panic!("not a random list")
        };
        assert_eq!(
            list.members.iter().map(|m| m.weight).collect::<Vec<_>>(),
            [1_000_000, -200_000, 0]
        );
        assert_eq!(list.members[0].key, b"A");
        assert_eq!(list.members[1].key, b"C");
        assert!(list.members[2].key.is_empty());
        assert_eq!(list.members[0].modifiers[0].len(), 3);
        assert_eq!(list.seed.len(), 2);
        assert!(text.trigger_present);
    }

    fn read(files: &[(&str, &str)]) -> Names {
        let files: Vec<_> = files
            .iter()
            .map(|(path, text)| (text.as_bytes().to_vec(), (*path).to_owned()))
            .collect();
        read_files(&files, &KeyNames::default())
    }

    fn listed(names: &Names) -> Vec<String> {
        names
            .entries
            .keys()
            .map(|name| String::from_utf8_lossy(name).into_owned())
            .collect()
    }

    #[test]
    fn a_text_trigger_skips_a_later_limit() {
        content_reader_script::test_game::enter();
        let names = read(&[(
            "common/scripted_localisation/a.txt",
            "defined_text = { name = T text = { trigger = { if = { limit = { always = yes } limit = { @v = yes always = yes } always = @v } } localization_key = X } }",
        )]);
        let trigger = &names.entries[&b"T"[..]].texts[0].trigger;
        let body = trigger[0].value.items().unwrap();
        assert_eq!(body[1].value.items(), Some(&[][..]));
        assert_ne!(body[2].value.scalar(), Some(&b"yes"[..]));
        assert!(names.unknown.is_empty(), "{:?}", names.unknown);
    }

    #[test]
    fn every_entry_of_the_file_loop_is_read_by_keyword() {
        content_reader_script::test_game::enter();
        let names = read(&[(
            "common/scripted_localisation/mod.txt",
            "defined_text = { name = GetTitle text = { trigger = { always = yes } localization_key = X } } \
             DEFINED_TEXT = { Name = GetOther } \"defined_text\" = { name = Quoted } \
             other = { name = AnyKey } unnamed = { \"name\" = NotAName }",
        )]);
        assert_eq!(
            listed(&names),
            ["", "AnyKey", "GetOther", "GetTitle", "Quoted"]
        );
        assert!(names.entries.contains_key(b"GetOther".as_slice()));
        assert!(!names.entries.contains_key(b"getother".as_slice()));
        assert!(!names.entries.contains_key(b"NotAName".as_slice()));
    }

    #[test]
    fn text_blocks_are_read_by_their_own_readers() {
        content_reader_script::test_game::enter();
        let names = read(&[(
            "common/scripted_localisation/mod.txt",
            "a = { name = A TEXT = { Unknown = { @one = Defined } } } \
             b = { name = @one } \
             c = { name = C text = { Localization_Key = K random_list = { @two = Defined } } } \
             d = { name = @two } \
             e = { name = E text = { Random_List = { 10 = { localization_key = K Modifier = { factor = 2 } } Seed = 5 other = { @three = Defined } } } } \
             f = { name = @three }",
        )]);
        assert_eq!(listed(&names), ["@one", "@three", "@two", "A", "C", "E"]);
        assert!(names.unknown.is_empty(), "{:?}", names.unknown);
    }

    #[test]
    fn a_file_ending_in_a_definition_registers_the_empty_name() {
        content_reader_script::test_game::enter();
        let names = read(&[(
            "common/scripted_localisation/mod.txt",
            "a = { name = A } @x = 1",
        )]);
        assert_eq!(listed(&names), ["", "A"]);
    }

    #[test]
    fn names_are_stored_as_read_string_into_stores_them() {
        content_reader_script::test_game::enter();
        let names = read(&[(
            "common/scripted_localisation/mod.txt",
            "a = { name = \"Two\nLines\" } @n = Named b = { name = @n } c = { name = { name = Inner } }",
        )]);
        assert_eq!(listed(&names), ["", "Inner", "Named", "Unreadable String"]);
        assert!(names.warnings[0].1.contains("field `name` is a block"));
    }

    #[test]
    fn an_untraced_reader_leaves_the_rest_of_its_file_unknown() {
        content_reader_script::test_game::enter();
        let names = read(&[
            (
                "common/scripted_localisation/a.txt",
                "a = { name = Before } b = { name = Inside text = { random_list = { 1 = { localization_key = { key = K } } } } } c = { name = After }",
            ),
            ("common/scripted_localisation/b.txt", "d = { name = Other }"),
        ]);
        assert_eq!(listed(&names), ["Before", "Other"]);
        let unknown = names.unknown.join("; ");
        assert!(
            unknown.contains("common/scripted_localisation/a.txt is unknown from line 1")
                && unknown.contains("bound localisation reader is unsupported"),
            "{unknown}"
        );
    }

    #[test]
    fn a_later_file_settles_the_names_it_defines() {
        content_reader_script::test_game::enter();
        let names = read(&[
            (
                "common/scripted_localisation/a.txt",
                "a = { name = Early } b = { name = Inside text = { random_list = { 1 = { localization_key = { key = K } } } } }",
            ),
            (
                "common/scripted_localisation/b.txt",
                "a = { name = Pick text = { localization_key = GOOD } }",
            ),
        ]);
        assert_eq!(names.uncertain(b"Pick"), None);
        for name in ["Early", "Inside", "Undefined"] {
            assert!(
                names
                    .uncertain(name.as_bytes())
                    .is_some_and(|why| why.contains("common/scripted_localisation/a.txt is unknown")),
                "{name}"
            );
        }
    }

    #[test]
    fn the_end_of_the_text_closes_the_entry() {
        content_reader_script::test_game::enter();
        let names = read(&[("common/scripted_localisation/mod.txt", "a = { name = Open ")]);
        assert_eq!(listed(&names), ["Open"]);
        assert!(names.warnings.is_empty(), "{:?}", names.warnings);
    }
}
