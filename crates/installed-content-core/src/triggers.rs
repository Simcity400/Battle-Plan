//! Trigger bodies as data, for consumers that decide them from recorded
//! state (the one evaluator is `dashboard::triggers`). Nothing here
//! evaluates a trigger.
//!
//! A body is the conjunction's children: each a key, its operator and its
//! value token, as the content's reader read them ([`crate::parser::Item`]).
//! Content read through the reader replay keeps the game's reading; readers
//! still on the tree hand over its items, which the evaluator reads the
//! same way.

use std::io;

use crate::{
    ResolvedLayers, combat_tactics,
    key_names::KeyNames,
    parser::{Item, Value, native},
};

/// A trigger body: a conjunction's children, in order.
pub type Triggers = Vec<Item>;

/// The body of a trigger field. A value that opens no block is reported
/// and nothing is read, leaving an empty conjunction; `None` says so,
/// for a caller whose reader refuses that form by name.
pub fn body(value: &Value) -> Option<Triggers> {
    value.items().map(<[Item]>::to_vec)
}

/// Read trigger-body text with the scripted-trigger grammar and
/// combat-tactic dispatch ([`combat_tactics`]) against `names`, for callers
/// that hold trigger text alone.
pub fn read(text: &[u8], names: &KeyNames) -> native::Read {
    let mut shaper = combat_tactics::Shaper::new(Some(names));
    native::read(
        text,
        native::Mode::KeepMark,
        native::Top::File(combat_tactics::Node::Trigger),
        Some(names),
        &mut |node, field| shaper.shape(node, field),
    )
}

/// The `.txt` files directly in `folder`, in database load order:
/// `common/scripted_triggers` and `common/raids` with its categories each
/// list one directory through [`ResolvedLayers::directory_files`].
pub fn folder_files(layers: &ResolvedLayers, folder: &str) -> io::Result<Vec<String>> {
    layers.directory_files(folder, ".txt")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Collection size/input/operators/limit uses CONTROLLER. Collections and
    /// railway priority must keep the enclosing trigger-dispatch names
    /// through the public reader.
    #[test]
    fn nested_collection_and_mtth_triggers_keep_content_names() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let names = KeyNames::default();
        let read = read(
            b"collection_size = { input = { operators = { limit = { \
                CONTROLLER = { always = yes } } } } value > 0 } \
              can_build_railway = { controller_priority = { modifier = { \
                factor = 2 CONTROLLER = { always = yes } } } }",
            &names,
        );
        assert!(read.unsettled.is_none(), "{read:?}");
        assert!(read.misreads.is_empty(), "{read:?}");
        assert_eq!(read.items.len(), 2, "{read:?}");
        let nested = |item: &Item| item.value.items().unwrap().to_vec();
        let collection = nested(&read.items[0]);
        let input = nested(&collection[0]);
        let operators = nested(&input[0]);
        let limit = nested(&operators[0]);
        assert!(limit[0].keyword_is("controller"));
        assert!(nested(&limit[0])[0].keyword_is("always"));
        let railway = nested(&read.items[1]);
        let priority = nested(&railway[0]);
        let modifier = nested(&priority[0]);
        assert!(modifier[1].keyword_is("controller"));
        assert!(nested(&modifier[1])[0].keyword_is("always"));
    }

    /// A conditional reads a later `limit` only into an empty limit: once
    /// the first holds a child, the later one is skipped, its definitions
    /// never made. Trigger dispatch decides whether a key adds a child:
    /// a scripted trigger does, a key naming nothing does not, and where
    /// the names cannot tell, the rest is unsettled.
    #[test]
    fn a_skipped_limit_defines_nothing() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let none = KeyNames::default();
        let read = read(
            b"if = { limit = { always = yes } limit = { @v = yes always = yes } always = @v }",
            &none,
        );
        assert!(read.unsettled.is_none(), "{:?}", read.unsettled);
        let body = read.items[0].value.items().unwrap();
        assert_eq!(body[1].value.items(), Some(&[][..]));
        assert_ne!(body[2].value.scalar(), Some(&b"yes"[..]));
        // An empty first limit leaves the second read.
        let read = super::read(
            b"if = { limit = { } limit = { @v = yes always = yes } always = @v }",
            &none,
        );
        let body = read.items[0].value.items().unwrap();
        assert_eq!(body.len(), 3, "{read:?}");
        assert_eq!(body[2].value.scalar(), Some(&b"yes"[..]));
        // A first limit holding a scripted trigger holds a child; one holding
        // a key that names nothing holds none.
        let text = b"if = { limit = { maybe_scripted = yes } limit = { @v = yes always = yes } always = @v }";
        let scripted = KeyNames {
            scripted: [b"maybe_scripted".to_vec()].into(),
            ..KeyNames::default()
        };
        let body = super::read(text, &scripted).items[0]
            .value
            .items()
            .unwrap()
            .to_vec();
        assert_eq!(body[1].value.items(), Some(&[][..]));
        assert_ne!(body[2].value.scalar(), Some(&b"yes"[..]));
        let body = super::read(text, &none).items[0]
            .value
            .items()
            .unwrap()
            .to_vec();
        assert_eq!(body[2].value.scalar(), Some(&b"yes"[..]));
        // Whether it names an ideology group is unknown while an ideology
        // file stops short.
        let mut gap = KeyNames::default();
        gap.counts.ideologies_gap = Some("common/ideologies/b.txt is unknown from line 3".into());
        let why = super::read(text, &gap).unsettled.unwrap().to_string();
        assert!(
            why.contains("reads a later limit only into an empty one"),
            "{why}"
        );
    }

    /// A body class's key reader takes its own keys before trigger dispatch:
    /// `naval_strength_comparison` reads `sub_unit_def_weights`, while a key
    /// its reader does not take and dispatch finds invalid is skipped. A
    /// reader without the fallback (`has_opinion`) reports every other key,
    /// whose skip passes a block.
    #[test]
    fn a_body_class_reads_its_own_keys_first() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let names = KeyNames::default();
        let read = read(
            b"naval_strength_comparison = { other = ENG sub_unit_def_weights = { carrier = 0.5 } nothing = { x = 1 } } \
              has_opinion = { target = GER value > 10 always = { x = 1 } }",
            &names,
        );
        assert!(read.unsettled.is_none(), "{:?}", read.unsettled);
        let naval = read.items[0].value.items().unwrap();
        assert_eq!(naval[1].value.items().map(<[Item]>::len), Some(1));
        assert_eq!(naval[2].value, Value::Block(Vec::new()));
        let opinion = read.items[1].value.items().unwrap();
        assert_eq!(opinion.len(), 3);
        assert_eq!(opinion[2].value, Value::Block(Vec::new()));
        // The variable-check key reader takes each variable's scoped value
        // without skipping a supplied block.
        let read = super::read(b"check_variable = { focus_progress > 699.9 }", &names);
        assert!(read.misreads.is_empty(), "{:?}", read.misreads);
        let variable = read.items[0].value.items().unwrap();
        assert_eq!(variable[0].value.scalar(), Some(b"699.9".as_slice()));
    }

    /// Operators stay on their items (`<` and `>` are one token each);
    /// groups read their bodies and a value opening no block is the
    /// empty conjunction.
    #[test]
    fn a_body_keeps_each_key_operator_and_value() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let read = read(
            b"a > 2 b<2 OR = { has_tech = radar c = yes } flat = yes",
            &KeyNames::default(),
        );
        assert!(read.unsettled.is_none());
        let operators: Vec<_> = read.items.iter().map(|item| item.operator).collect();
        use crate::parser::Operator::*;
        assert_eq!(operators, [Greater, Less, Equals, Equals]);
        assert_eq!(read.items[2].value.items().map(<[Item]>::len), Some(2));
        assert_eq!(body(&read.items[3].value), None);
        assert_eq!(body(&read.items[2].value).map(|body| body.len()), Some(2));
    }
}
