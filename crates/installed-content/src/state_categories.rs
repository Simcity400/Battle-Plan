//! State-category identities from the recorded content stack.
//!
//! 1413AFB60 lists direct files in filename order with lexer mode 0;
//! 1413AF3B0 reads `state_categories`, reusing an existing category on a
//! duplicate. 1413AFAC0/1413AEBB0 compare the ASCII-folded hash (140BBA1B0) and name.
//! The constructor (1413AECB0) marks every declared category valid (+0x10).
//! Description 1403184E0 localises the effect's operand, not cached +0xB8
//! or `local_building_slots`; only declared names are needed here.

use crate::{
    ResolvedLayers,
    fields::invalid,
    key_names::KeyNames,
    modifiers,
    parser::{
        c_string,
        native::{self, Shape},
    },
    script_files,
};
use std::io;

#[derive(Default)]
pub struct Database {
    /// First-declaration order and spelling; repeated bodies reuse the object.
    pub names: Vec<Vec<u8>>,
    pub notes: Vec<String>,
    gap: Option<String>,
}

impl Database {
    pub fn find(&self, name: &[u8]) -> Result<Option<&[u8]>, String> {
        if let Some(why) = &self.gap {
            return Err(format!("state-category definitions uncertain: {why}"));
        }
        Ok(self
            .names
            .iter()
            .find(|entry| entry.eq_ignore_ascii_case(c_string(name)))
            .map(Vec::as_slice))
    }

    fn insert(&mut self, name: Vec<u8>) {
        if !self
            .names
            .iter()
            .any(|entry| entry.eq_ignore_ascii_case(&name))
        {
            self.names.push(name);
        }
    }
}

/// 1413AFCF0: colour, per-building maximums, then CModifier::ReadMember.
pub fn parse(input: &[u8], names: &KeyNames) -> io::Result<Database> {
    #[derive(Clone, Copy)]
    enum Node {
        File,
        Categories,
        Category,
        Maximums,
        Modifier(modifiers::Node),
    }
    let mut undecided = Vec::new();
    let read = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::File(Node::File),
        Some(names),
        &mut |node, field| match node {
            Node::File if field.keyword_is("state_categories") => Shape::Object(Node::Categories),
            Node::File => Shape::Skip,
            Node::Categories => Shape::Object(Node::Category),
            Node::Category => match field.keyword().as_deref() {
                Some(b"color") => Shape::Color,
                Some(b"buildings_max_level") => Shape::Object(Node::Maximums),
                _ => modifiers::shape(
                    modifiers::Node::Modifier,
                    field,
                    &|key| modifiers::registered(key, Some(names)),
                    &mut undecided,
                )
                .map(Node::Modifier),
            },
            // Custom uniform statement reader 1413AFDE0 consumes one value.
            Node::Maximums => Shape::One,
            Node::Modifier(node) => modifiers::shape(
                node,
                field,
                &|key| modifiers::registered(key, Some(names)),
                &mut undecided,
            )
            .map(Node::Modifier),
        },
    );
    if let Some(cause) = read.unsettled {
        return Err(invalid(format!("state categories: {cause}")));
    }
    if let Some(cause) = undecided.first() {
        return Err(invalid(format!("state categories: {cause}")));
    }
    let mut database = Database::default();
    database
        .notes
        .extend(read.misreads.iter().map(ToString::to_string));
    for root in read
        .items
        .iter()
        .filter(|item| item.keyword_is("state_categories"))
    {
        for item in root.value.items().unwrap_or_default() {
            database.insert(c_string(item.key.as_deref().unwrap_or_default()).to_vec());
        }
    }
    Ok(database)
}

pub fn read(layers: &ResolvedLayers) -> io::Result<Database> {
    let names = layers.key_names()?;
    let mut database = Database::default();
    for path in script_files::paths(layers, "common/state_category/", Some(".txt")) {
        match parse(&layers.read(&path)?, &names) {
            Ok(parsed) => {
                for name in parsed.names {
                    database.insert(name);
                }
                database.notes.extend(parsed.notes);
            }
            Err(why) => {
                let why = format!("State categories unavailable in {path}: {why}");
                database.gap.get_or_insert_with(|| why.clone());
                database.notes.push(why);
            }
        }
    }
    Ok(database)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Folder(std::path::PathBuf);

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn native_state_category_names_and_consumption() {
        for (input, expected) in [
            (b"state_categories={Syn={local_building_slots=3 color={1 2 3}} SYN={local_supplies=0.1} other={buildings_max_level={mod_building=4}}}".as_slice(), vec![b"Syn".as_slice(), b"other"]),
            (b"\"state_categories\"={hidden={}} STATE_CATEGORIES={\"Mixed\"={}}", vec![b"Mixed".as_slice()]),
            (b"@slots=4 state_categories={first={buildings_max_level={custom=@slots} added_modifier={local_building_slots=@slots}} second={unknown={@hidden=9}} third={local_building_slots=@hidden}}", vec![b"first".as_slice(), b"second", b"third"]),
        ] {
            let database = parse(input, &KeyNames::default()).unwrap();
            assert_eq!(database.names.iter().map(Vec::as_slice).collect::<Vec<_>>(), expected);
            assert_eq!(database.find(b"sYn").unwrap().is_some(), expected.contains(&b"Syn".as_slice()));
            assert!(database.find(b"undefined").unwrap().is_none());
        }
    }

    #[test]
    fn state_category_stack_order_and_replacement() {
        let root = std::env::temp_dir().join(format!("state-category-{}", std::process::id()));
        let _folder = Folder(root.clone());
        let base = root.join("base");
        let over = root.join("over");
        for (dir, path, text) in [
            (
                &base,
                "common/state_category/a.txt",
                "state_categories={hidden={}}",
            ),
            (
                &over,
                "common/state_category/a.txt",
                "state_categories={First={}}",
            ),
            (
                &base,
                "common/state_category/b.txt",
                "state_categories={FIRST={} second={}}",
            ),
            (
                &base,
                "common/state_category/sub/c.txt",
                "state_categories={nested={}}",
            ),
        ] {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let layers = ResolvedLayers::from_folders(&[(&base, &[][..]), (&over, &[][..])]).unwrap();
        let database = read(&layers).unwrap();
        assert_eq!(database.names, [b"First".to_vec(), b"second".to_vec()]);
        let layers = ResolvedLayers::from_folders(&[
            (&base, &[][..]),
            (&over, &["common/state_category"][..]),
        ])
        .unwrap();
        assert_eq!(read(&layers).unwrap().names, [b"First".to_vec()]);
    }

    #[test]
    fn uncertain_state_category_modifier_names_are_named() {
        let mut names = KeyNames::default();
        names.effects.modifiers_gap = Some("synthetic missing registry".into());
        let error = parse(b"state_categories={syn={unresolved_modifier={}}}", &names)
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("unresolved_modifier") && error.contains("line 1"),
            "{error}"
        );
    }
}
