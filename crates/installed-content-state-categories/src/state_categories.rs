//! State-category identities from the recorded content stack.
//!
//! Direct files load in filename order with lexer mode 0;
//! `state_categories` reuses an existing category on a duplicate.
//! Identity compares ASCII-folded hash and name. Every declared category
//! starts valid. Descriptions localise the effect's operand, rather than
//! the cached name or `local_building_slots`; only declared names are
//! needed here.
//!
//! A category's `color` uses the colour reader ideology groups share
//! ([`installed_content_core::ideologies::ColorReads`]): each read updates the state
//! earlier reads of the same object left, starting from white. A category that
//! reads none has no colour here; the states map mode refuses its states by name.

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
use installed_content_core::ideologies::ColorReads;
use std::io;

#[derive(Default)]
pub struct Database {
    /// First-declaration order and spelling; repeated bodies reuse the object.
    pub names: Vec<Vec<u8>>,
    /// Each name's colour state after every `color` read of its bodies, in [`Self::names`]
    /// order; `None` when no body reads one.
    pub colors: Vec<Option<ColorReads>>,
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

    /// The colour of a declared category, matched like [`Self::find`].
    pub fn color(&self, name: &[u8]) -> Result<Option<[f32; 4]>, String> {
        if let Some(why) = &self.gap {
            return Err(format!("state-category definitions uncertain: {why}"));
        }
        let index = self
            .names
            .iter()
            .position(|entry| entry.eq_ignore_ascii_case(c_string(name)))
            .ok_or_else(|| {
                format!(
                    "state category {} is not declared",
                    String::from_utf8_lossy(name)
                )
            })?;
        self.colors[index].as_ref().map(ColorReads::color).transpose()
    }

    fn insert(&mut self, name: Vec<u8>) -> usize {
        match self
            .names
            .iter()
            .position(|entry| entry.eq_ignore_ascii_case(&name))
        {
            Some(index) => index,
            None => {
                self.names.push(name);
                self.colors.push(None);
                self.names.len() - 1
            }
        }
    }

    /// Applies one body's `color` reads to the named object.
    fn read_colors(&mut self, index: usize, body: &[crate::parser::Item]) {
        for field in body.iter().filter(|field| field.keyword_is("color")) {
            self.colors[index].get_or_insert_default().read(field);
        }
    }

    /// Adds one file's parse. Its colour reads replay over each object's existing colour state,
    /// as the native object keeps it across files.
    fn merge(&mut self, parsed: Parsed) {
        self.notes.extend(parsed.database.notes);
        for (name, bodies) in parsed.database.names.into_iter().zip(parsed.bodies) {
            let index = self.insert(name);
            for body in bodies {
                self.read_colors(index, &body);
            }
        }
    }
}

/// One file's categories and, per category, the bodies whose `color` reads replay in order.
struct Parsed {
    database: Database,
    bodies: Vec<Vec<Vec<crate::parser::Item>>>,
}

/// Colour and shared modifier fields.
pub fn parse(input: &[u8], names: &KeyNames) -> io::Result<Database> {
    let parsed = parse_file(input, names)?;
    let mut database = Database::default();
    database.merge(parsed);
    Ok(database)
}

fn parse_file(input: &[u8], names: &KeyNames) -> io::Result<Parsed> {
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
            // Each building maximum consumes one value.
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
    let mut bodies: Vec<Vec<Vec<crate::parser::Item>>> = Vec::new();
    database
        .notes
        .extend(read.misreads.iter().map(ToString::to_string));
    for root in read
        .items
        .iter()
        .filter(|item| item.keyword_is("state_categories"))
    {
        for item in root.value.items().unwrap_or_default() {
            let index = database.insert(c_string(item.key.as_deref().unwrap_or_default()).to_vec());
            if bodies.len() <= index {
                bodies.resize_with(index + 1, Vec::new);
            }
            bodies[index].push(item.value.items().unwrap_or_default().to_vec());
        }
    }
    Ok(Parsed { database, bodies })
}

pub fn read(layers: &ResolvedLayers) -> io::Result<Database> {
    let names = layers.key_names()?;
    let mut database = Database::default();
    for path in script_files::paths(layers, "common/state_category/", Some(".txt"))? {
        match parse_file(&path.read(layers)?, &names) {
            Ok(parsed) => database.merge(parsed),
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
    fn state_category_names_and_consumption() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    fn state_category_colours_replay_on_the_object() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let database = parse(
            b"state_categories={Syn={color={1 2 3}} plain={} SYN={color={0.5 0.25 0.125 0.75}} late={color={255 0 0}} LATE={color={0 0}}}",
            &KeyNames::default(),
        )
        .unwrap();
        assert_eq!(
            database.color(b"syn").unwrap(),
            Some([0.5, 0.25, 0.125, 0.75])
        );
        assert_eq!(database.color(b"plain").unwrap(), None);
        // The second body's two components continue from the first body's colour.
        assert_eq!(database.color(b"late").unwrap(), Some([0.0, 0.0, 0.0, 1.0]));
        assert!(database.color(b"missing").is_err());
        let first = parse(
            b"state_categories={only={color={2 4 6}}}",
            &KeyNames::default(),
        )
        .unwrap();
        assert_eq!(
            first.color(b"only").unwrap(),
            Some([2.0 / 255.0, 4.0 / 255.0, 6.0 / 255.0, 1.0])
        );
        // A later read that stores every component replaces a packed colour, in the same
        // body or a later one; a partial read leaves the packed refusal.
        let packed = parse(
            b"state_categories={isle={color=16711680 color={0 1 0}} part={color=16711680 color={0 1 x}} late={color=16711680} LATE={color={0 0 1 0.5}}}",
            &KeyNames::default(),
        )
        .unwrap();
        assert_eq!(packed.color(b"isle").unwrap(), Some([0.0, 1.0, 0.0, 1.0]));
        assert!(
            packed
                .color(b"part")
                .unwrap_err()
                .contains("the packed colour 16711680")
        );
        assert_eq!(packed.color(b"late").unwrap(), Some([0.0, 0.0, 1.0, 0.5]));
    }

    #[test]
    fn state_category_stack_order_and_replacement() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
