//! The entries `common/scripted_localisation` defines.
//!
//! A bracket's last segment that is no built-in property is looked up among
//! these names, exactly, using FNV-1a hash and byte comparison. The entry's
//! first text block whose trigger passes is rendered. An undefined name is
//! empty; an uncertain file suffix is refused. Ordered text alternatives
//! and triggers are retained for the shared recorded evaluator.
//!
//! Each `.txt` file gets its own reader through [`native`]. Until the end
//! token, each key/operator/value creates a new entry, whatever the key
//! was; `defined_text` is not a required keyword. The entry is stored under
//! its `name`.

use std::{collections::BTreeMap, io};

use crate::{
    ResolvedLayers, combat_tactics,
    key_names::KeyNames,
    parser::{
        self, Item,
        native::{self, Kind, Misread, Shape},
    },
    script_files,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node {
    /// The file loop.
    Top,
    /// A localisation entry.
    Localization,
    /// A `text` block.
    Text,
    /// A random localisation list.
    RandomList,
    /// A random localisation list member.
    Member,
    /// A trigger body's field, the one trigger dispatch
    /// ([`combat_tactics::Shaper`]).
    Trigger(combat_tactics::Node),
}

/// One file as its reader reads it.
struct File {
    /// Each entry's name, up to the first one the port cannot read.
    entries: Vec<(Vec<u8>, Entry)>,
    misreads: Vec<Misread>,
    /// Where the port stops knowing the file, and why.
    unknown: Option<Misread>,
}

fn read_file(input: &[u8], names: &KeyNames) -> File {
    let mut entries = 0;
    // The pair already holds a localisation key or list.
    let mut holds = false;
    let mut untraced: Option<(usize, Misread)> = None;
    let mut shaper = combat_tactics::Shaper::new(Some(names));
    // Each file constructs the lexer in mode 0.
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
            // `name` reads one string; `text` reads a text/trigger pair.
            // Any other key is reported and its block skipped.
            Node::Localization => match field.keyword().as_deref() {
                Some(b"name") => Shape::One,
                Some(b"text") => {
                    holds = false;
                    Shape::Object(Node::Text)
                }
                _ => Shape::Skip,
            },
            // `localization_key` takes the value token; `random_list` reads a list.
            // Either one on a pair that holds one is reported and skipped.
            // `trigger` reads a conjunction body; any other key is reported
            // and its block skipped.
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
                Some(b"trigger") => Shape::Body(Node::Trigger(combat_tactics::Node::Trigger)),
                _ => Shape::Skip,
            },
            // A number key is a member object; `seed` reads a scoped-variable
            // value token. Any other key is reported and its block skipped.
            Node::RandomList
                if field.key.kind == Kind::Bare && parser::number_token(&field.key.text, false) =>
            {
                Shape::Object(Node::Member)
            }
            Node::RandomList if field.keyword_is("seed") => Shape::One,
            Node::RandomList => Shape::Skip,
            // `localization_key` takes a value token or a bound localisation
            // block, whose reader is unsupported; `modifier` reads a scriptable
            // value modifier body. Any other key is reported and its block skipped.
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
                // Scriptable value modifiers share the mean-time modifier key reader.
                Some(b"modifier") => Shape::Body(Node::Trigger(combat_tactics::Node::Modifier)),
                _ => Shape::Skip,
            },
            Node::Trigger(node) => shaper.shape(node, field).map(Node::Trigger),
        },
    );
    let (known, unknown, misreads) = match (untraced, read.unsettled) {
        // Everything from the entry holding it on is unknown.
        (Some((entry, unknown)), _) => {
            let misreads = read
                .misreads
                .into_iter()
                .filter(|misread| misread.line < unknown.line)
                .collect();
            (entry, Some(unknown), misreads)
        }
        // The reading stopped inside the last entry read.
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

/// An entry's name: its last `name` string, with newline checking;
/// the empty name without one.
fn name(item: &Item) -> Vec<u8> {
    item.value
        .items()
        .and_then(|fields| fields.iter().rev().find(|field| field.keyword_is("name")))
        .and_then(|field| field.value.read_string())
        .unwrap_or_default()
        .to_vec()
}

/// Ordered alternatives of localisation resolution.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub texts: Vec<Text>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Text {
    /// Absent value pointers are skipped before their trigger is checked.
    pub value: Option<TextValue>,
    /// The text's conjunction reads nothing of a value that opens no block,
    /// which leaves it empty.
    pub trigger: crate::triggers::Triggers,
    /// Even an empty trigger object starts a scoped-variable context when
    /// checked; an absent trigger starts none.
    pub trigger_present: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextValue {
    Key(Vec<u8>),
    RandomList(RandomList),
}

/// A random localisation list, in authored member order. The seed's
/// assignments remain ordered because reassignment does not reset every
/// operand field.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RandomList {
    pub seed: Vec<Item>,
    pub members: Vec<RandomMember>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RandomMember {
    /// Signed integer conversion of the number key, times 100000.
    pub weight: i64,
    /// Scriptable value modifiers, read through shared mean-time modifier
    /// field dispatch.
    pub modifiers: Vec<Vec<Item>>,
    /// Bound localisation; an absent key is empty.
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

/// The scripted localisation names the port knows the content stack to
/// define, and where it cannot know.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Names {
    /// Every entry read, by its exact name; a later file's entry of a name
    /// replaces the earlier one.
    pub entries: BTreeMap<Vec<u8>, Entry>,
    /// Each file the port cannot read to its end, with where and why: a
    /// name `entries` lacks may be defined, or one it holds redefined, in the
    /// rest of it.
    pub unknown: Vec<String>,
    /// Places the reading left the text, and the unknown files, by file.
    pub warnings: Vec<crate::Diagnostic>,
}

/// Every scripted localisation name in the content stack, as its files'
/// readers read them.
pub fn names(layers: &ResolvedLayers) -> io::Result<Names> {
    let files = script_files::paths(layers, "common/scripted_localisation/", Some(".txt"))
        .into_iter()
        .map(|path| Ok((layers.read(&path)?, path)))
        .collect::<io::Result<Vec<_>>>()?;
    Ok(read_files(&files, &*layers.key_names()?))
}

/// The database's files in load order, each with its path, their triggers
/// read against `key_names`.
fn read_files(files: &[(Vec<u8>, String)], key_names: &KeyNames) -> Names {
    let mut names = Names::default();
    for (bytes, path) in files {
        let file = read_file(bytes, key_names);
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
        }
        names.entries.extend(file.entries);
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_name_warning_keeps_unsettled_kind() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

    /// The files as [`names`] reads them after listing them; the
    /// dashboard's catalogue tests read them from a content folder.
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

    /// A text's trigger is read by the one trigger dispatch: a later limit
    /// in a conditional is skipped, defining nothing.
    #[test]
    fn a_text_trigger_skips_a_later_limit() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

    /// Every top-level key is read as a localisation entry; keywords fold,
    /// a quoted key is none; an entry without a `name` is the empty name.
    #[test]
    fn every_entry_of_the_file_loop_is_read_by_keyword() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        // Names match exactly.
        assert!(names.entries.contains_key(b"GetOther".as_slice()));
        assert!(!names.entries.contains_key(b"getother".as_slice()));
        assert!(!names.entries.contains_key(b"NotAName".as_slice()));
    }

    /// Text-pair and random-list readers skip a key they do not know, so
    /// a variable defined in its block is never defined; a second key or list
    /// in one pair is skipped the same way.
    #[test]
    fn text_blocks_are_read_by_their_own_readers() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

    /// A file ending in a definition: no key is read and the file loop reads
    /// an entry for the key slot as it stands, whose block is empty at the
    /// end of the text: the empty name registers.
    #[test]
    fn a_file_ending_in_a_definition_registers_the_empty_name() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let names = read(&[(
            "common/scripted_localisation/mod.txt",
            "a = { name = A } @x = 1",
        )]);
        assert_eq!(listed(&names), ["", "A"]);
    }

    /// A string reader stores `Unreadable String` for a name holding a line
    /// feed; a name read where the reader takes one value reads a block
    /// as the entry's fields.
    #[test]
    fn names_are_stored_as_read_string_into_stores_them() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let names = read(&[(
            "common/scripted_localisation/mod.txt",
            "a = { name = \"Two\nLines\" } @n = Named b = { name = @n } c = { name = { name = Inner } }",
        )]);
        // The `}` left over is a key the file loop reads an entry for.
        assert_eq!(listed(&names), ["", "Inner", "Named", "Unreadable String"]);
        assert!(names.warnings[0].1.contains("field `name` is a block"));
    }

    /// A member's `localization_key` block has an unsupported bound
    /// localisation reader: its entry and everything after it in the file are
    /// unknown, so a name no read entry defines is not proven undefined.
    #[test]
    fn an_untraced_reader_leaves_the_rest_of_its_file_unknown() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

    /// The end of the text closes every loop.
    #[test]
    fn the_end_of_the_text_closes_the_entry() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let names = read(&[("common/scripted_localisation/mod.txt", "a = { name = Open ")]);
        assert_eq!(listed(&names), ["Open"]);
        assert!(names.warnings.is_empty(), "{:?}", names.warnings);
    }
}
