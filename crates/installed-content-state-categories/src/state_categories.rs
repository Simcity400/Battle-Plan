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
//! ([`installed_content_core::ideologies::read_color`]): each read updates the state
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
use std::io;

/// Authored category names and their parsed cap for one building.
pub type BuildingCaps<'a> = Vec<(&'a [u8], Result<i32, String>)>;

#[derive(Default)]
pub struct Database {
    /// First-declaration order and spelling; repeated bodies reuse the object.
    pub names: Vec<Vec<u8>>,
    /// Each name's colour after every `color` read of its bodies, in [`Self::names`] order;
    /// `Ok(None)` when no body reads one, `Err` when a read refused.
    pub colors: Vec<Result<Option<[f32; 4]>, String>>,
    /// Each category's `buildings_max_level` entries in [`Self::names`] order:
    /// the building name as authored and its integer maximum, `None` when the
    /// value does not scan as one.
    pub capped: Vec<Vec<(Vec<u8>, Option<i32>)>>,
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
        self.colors[index].clone()
    }

    /// Each category whose `buildings_max_level` names `building` under the
    /// lexer's ASCII fold, in declaration order, with the maximum it sets.
    /// `Err` names an entry that does not scan as an integer or a building
    /// named again with a different maximum, whose stored value is unverified.
    pub fn capping(&self, building: &str) -> Result<BuildingCaps<'_>, String> {
        if let Some(why) = &self.gap {
            return Err(format!("state-category definitions uncertain: {why}"));
        }
        Ok(self
            .names
            .iter()
            .zip(&self.capped)
            .filter_map(|(name, capped)| {
                let values = capped
                    .iter()
                    .filter(|(key, _)| key.eq_ignore_ascii_case(building.as_bytes()))
                    .map(|(_, value)| *value)
                    .collect::<Vec<_>>();
                let first = *values.first()?;
                let category = String::from_utf8_lossy(name);
                let value = match first {
                    _ if values.iter().any(|value| *value != first) => Err(format!(
                        "state category {category} sets {building}'s maximum more than once with different values"
                    )),
                    Some(value) => Ok(value),
                    None => Err(format!(
                        "state category {category}'s maximum for {building} is not an integer"
                    )),
                };
                Some((name.as_slice(), value))
            })
            .collect())
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
                self.colors.push(Ok(None));
                self.capped.push(Vec::new());
                self.names.len() - 1
            }
        }
    }

    /// Applies one body's `color` reads to the named object.
    fn read_colors(&mut self, index: usize, body: &[crate::parser::Item]) {
        for field in body.iter().filter(|field| field.keyword_is("color")) {
            let slot = &mut self.colors[index];
            if let Ok(state) = slot {
                let mut color = state.unwrap_or([1.0; 4]);
                *slot =
                    installed_content_core::ideologies::read_color(&mut color, field).map(|()| Some(color));
            }
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
                for field in body
                    .iter()
                    .filter(|field| field.keyword_is("buildings_max_level"))
                {
                    self.capped[index].extend(field.value.items().unwrap_or_default().iter().map(
                        |entry| {
                            (
                                c_string(entry.key.as_deref().unwrap_or_default()).to_vec(),
                                entry.value.scalar().and_then(|value| {
                                    installed_content_core::crt::scan_i32(value).stored()
                                }),
                            )
                        },
                    ));
                }
            }
        }
    }
}

/// One file's categories and, per category, the bodies whose `color` reads replay in order.
struct Parsed {
    database: Database,
    bodies: Vec<Vec<Vec<crate::parser::Item>>>,
}

/// Colour, per-building maximums, then shared modifier fields.
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
    for path in script_files::paths(layers, "common/state_category/", Some(".txt")) {
        match parse_file(&layers.read(&path)?, &names) {
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
    fn state_categories_name_the_buildings_they_cap_across_bodies() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let database = parse(
            b"state_categories={isle={buildings_max_level={naval_base=8}} town={} ISLE={buildings_max_level={Roads=2}} coast={buildings_max_level={roads=1 air_base=2}}}",
            &KeyNames::default(),
        )
        .unwrap();
        assert_eq!(
            database.capping("roads").unwrap(),
            [(b"isle".as_slice(), Ok(2)), (b"coast", Ok(1))]
        );
        assert_eq!(
            database.capping("air_base").unwrap(),
            [(b"coast".as_slice(), Ok(2))]
        );
        assert!(database.capping("rails").unwrap().is_empty());
        let unclear = parse(
            b"state_categories={a={buildings_max_level={roads=2 roads=3}} b={buildings_max_level={roads=x}} c={buildings_max_level={roads=4 roads=4}}}",
            &KeyNames::default(),
        )
        .unwrap();
        let capping = unclear.capping("roads").unwrap();
        assert!(
            capping[0]
                .1
                .as_ref()
                .unwrap_err()
                .contains("more than once")
        );
        assert!(
            capping[1]
                .1
                .as_ref()
                .unwrap_err()
                .contains("not an integer")
        );
        assert_eq!(capping[2].1, Ok(4));
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
