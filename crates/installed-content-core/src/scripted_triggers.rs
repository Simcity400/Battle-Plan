//! The content's scripted triggers: the one reader of
//! `common/scripted_triggers`, in the game's two passes, each file on its
//! own reader through the reader replay ([`native`]).
//!
//! The database is read in two passes:
//!
//! 1. Names: the directory's own `.txt` files. Every top-level key is
//!    folded and looked up; a name already held is skipped, a new one
//!    becomes a registered template. Each value is skipped, defining
//!    nothing.
//! 2. Bodies, after the combat tactics load: every file below the directory.
//!    A key names its template by hash and ASCII case-insensitive comparison.
//!    One the name pass did not register logs a mismatch and its block is
//!    not skipped: the reader reads its `{` as the value and its contents
//!    as the file's own keys. A registered one has its children cleared and
//!    its body read as a tactic trigger is ([`crate::combat_tactics`]):
//!    the last body read wins.
//!
//! A trigger key reaches a template by its text without case, whatever its
//! token kind, after the lookups by token ID ([`crate::trigger_keys`]). A
//! scripted trigger reads a yes/no value and passes when the template's
//! result equals that value. No parameter is substituted: the value reader
//! takes one token, and bodies are parsed at load. Neither scripted-trigger
//! evaluation nor ordinary trigger evaluation guards depth.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use crate::{
    ResolvedLayers,
    combat_tactics::{self, Node},
    key_names::KeyNames,
    parser::{
        Item, Value, c_string,
        native::{self, Shape},
    },
};

const DIRECTORY: &str = "common/scripted_triggers/";

/// One template.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Template {
    /// The name as the name pass first read it.
    pub name: Vec<u8>,
    /// The last body read (children resolved as a tactic trigger's are,
    /// definitions consumed), empty when none was read; or why the port
    /// cannot follow the reading that gives it.
    pub body: Result<Vec<Item>, String>,
}

/// The scripted-trigger database as the viewer reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScriptedTriggers {
    /// Templates by ASCII-lowercased name.
    pub templates: BTreeMap<Vec<u8>, Template>,
    /// Errors the game logs and reads past, and the places the game's
    /// reading leaves the text, each read as the game reads it.
    pub logged: Vec<String>,
}

/// What a key's text finds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lookup<'a> {
    Template(&'a Template),
    /// No template of that name.
    Missing,
}

impl ScriptedTriggers {
    /// The template `text` names without ASCII case, as a C string:
    /// the key slot is read to its first NUL.
    pub fn lookup(&self, text: &[u8]) -> Lookup<'_> {
        match self.templates.get(&c_string(text).to_ascii_lowercase()) {
            Some(template) => Lookup::Template(template),
            None => Lookup::Missing,
        }
    }

    /// The name pass over one file.
    fn read_names(&mut self, bytes: &[u8], path: &str) {
        // The name pass constructs the lexer in mode 0.
        let read = native::read(
            bytes,
            native::Mode::KeepMark,
            native::Top::File(()),
            None,
            &mut |(), _| Shape::SkipIfOpen,
        );
        self.note(path, &read);
        for item in &read.items {
            // The name pass hashes the key's C string.
            let key = c_string(item.key.as_deref().unwrap_or_default()).to_vec();
            self.templates
                .entry(key.to_ascii_lowercase())
                .or_insert(Template {
                    name: key,
                    body: Ok(Vec::new()),
                });
        }
    }

    /// The name pass over the directory's own files.
    fn read_all_names(&mut self, layers: &ResolvedLayers) -> io::Result<()> {
        for path in crate::script_files::paths(layers, DIRECTORY, Some(".txt")) {
            self.read_names(&layers.read(&path)?, &path);
        }
        Ok(())
    }

    /// The body pass over one file, its keys read against `names`.
    fn read_bodies(&mut self, bytes: &[u8], path: &str, names: &KeyNames, tooltip_bindings: bool) {
        let templates = &self.templates;
        // The body pass constructs the lexer in mode 0.
        let mut shaper = combat_tactics::Shaper::new(Some(names));
        let read = native::read(
            bytes,
            native::Mode::KeepMark,
            native::Top::File(Node::Top),
            Some(names),
            &mut |node, field| match node {
                Node::Top
                    if templates.contains_key(&c_string(&field.key.text).to_ascii_lowercase()) =>
                {
                    Shape::Body(Node::Trigger)
                }
                // The mismatch skips nothing: the reader reads on from the value token.
                Node::Top => Shape::One,
                node => shaper.shape(node, field),
            },
        );
        self.note(path, &read);
        for item in read.items {
            // The body pass finds the template by the key's C string.
            let key = c_string(&item.key.unwrap_or_default()).to_vec();
            let name = String::from_utf8_lossy(&key).into_owned();
            let folded = key.to_ascii_lowercase();
            if !self.templates.contains_key(&folded) {
                self.logged.push(format!(
                    "{path}: scripted trigger {name}: the body pass logs a mismatch for a key the name pass did not register"
                ));
                continue;
            }
            let children = match item.value {
                Value::Block(mut children) => {
                    let place = format!("{path}: scripted trigger {name}");
                    combat_tactics::normalise_trigger_body(
                        &place,
                        &mut children,
                        &mut self.logged,
                        tooltip_bindings,
                    );
                    children
                }
                // For a value that opens no block, record an error; the cleared
                // template holds no child.
                Value::Scalar(text) => {
                    self.logged.push(format!(
                        "{path}: scripted trigger {name} = {} is not a block: the trigger-body reader logs an error and the template holds no condition",
                        String::from_utf8_lossy(&text)
                    ));
                    Vec::new()
                }
                Value::Tagged { .. } => Vec::new(),
            };
            if let Some(template) = self.templates.get_mut(&folded) {
                template.body = Ok(children);
            }
        }
        if let Some(unsettled) = read.unsettled {
            // The rest of the file may give any template its last body; a
            // later file's body settles it again.
            let reason = format!(
                "the scripted-trigger body pass over {path} cannot be followed from {unsettled}"
            );
            for template in self.templates.values_mut() {
                template.body = Err(reason.clone());
            }
            self.logged.push(reason);
        }
    }

    /// Each place a pass's reading leaves the text.
    fn note(&mut self, path: &str, read: &native::Read) {
        self.logged.extend(
            read.misreads
                .iter()
                .map(|misread| format!("{path} {misread}; read as the game reads it")),
        );
    }
}

impl ScriptedTriggers {
    /// Both passes over `files` (path and text), each file the directory's
    /// own, in order: for callers that hold scripted-trigger text alone. The
    /// bodies are read against `names` with the name pass's names.
    pub fn parse(files: &[(&str, &[u8])], names: &KeyNames) -> Self {
        let mut triggers = Self::default();
        for (path, text) in files {
            triggers.read_names(text, path);
        }
        let names = KeyNames {
            scripted: triggers.templates.keys().cloned().collect(),
            ..names.clone()
        };
        for (path, text) in files {
            triggers.read_bodies(text, path, &names, false);
        }
        triggers
    }
}

/// The name pass over the recorded content layers: every template name,
/// folded.
pub fn names(layers: &ResolvedLayers) -> io::Result<BTreeSet<Vec<u8>>> {
    let mut triggers = ScriptedTriggers::default();
    triggers.read_all_names(layers)?;
    Ok(triggers.templates.into_keys().collect())
}

/// Reads the scripted triggers of the recorded content layers.
pub fn read(layers: &ResolvedLayers) -> io::Result<ScriptedTriggers> {
    read_with_bindings(layers, false)
}

/// The same reader with presentation operands retained: tooltip/not_tooltip
/// bindings are stored separately from predicate children; descriptions
/// read these bindings for relation-rule targets.
pub fn read_descriptions(layers: &ResolvedLayers) -> io::Result<ScriptedTriggers> {
    read_with_bindings(layers, true)
}

fn read_with_bindings(
    layers: &ResolvedLayers,
    tooltip_bindings: bool,
) -> io::Result<ScriptedTriggers> {
    let names = layers.key_names()?;
    let mut triggers = ScriptedTriggers::default();
    let listed = layers.listed_paths(DIRECTORY);
    triggers.read_all_names(layers)?;
    for path in crate::defines::load_order(DIRECTORY, ".txt", listed.iter().map(String::as_str)) {
        triggers.read_bodies(&layers.read(&path)?, &path, &names, tooltip_bindings);
    }
    Ok(triggers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_files(names: &[(&str, &[u8])], bodies: &[(&str, &[u8])]) -> ScriptedTriggers {
        let mut triggers = ScriptedTriggers::default();
        for (path, text) in names {
            triggers.read_names(text, path);
        }
        let names = KeyNames {
            scripted: triggers.templates.keys().cloned().collect(),
            ..KeyNames::default()
        };
        for (path, text) in bodies {
            triggers.read_bodies(text, path, &names, false);
        }
        triggers
    }

    fn keys(items: &[Item]) -> Vec<String> {
        items
            .iter()
            .map(|item| String::from_utf8_lossy(item.key.as_deref().unwrap()).into_owned())
            .collect()
    }

    fn body(triggers: &ScriptedTriggers, name: &[u8]) -> Result<Vec<Item>, String> {
        match triggers.lookup(name) {
            Lookup::Template(template) => template.body.clone(),
            Lookup::Missing => panic!("{} is missing", String::from_utf8_lossy(name)),
        }
    }

    /// The first name wins in the name pass; in the body pass the last body
    /// read wins, found without case, with its definitions consumed.
    #[test]
    fn names_register_first_and_bodies_read_last() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let a: &[u8] = b"@x = yes My_Check = { is_attacker = @x } other = { }";
        let b: &[u8] = b"MY_CHECK = { has_reserves = yes }";
        let triggers = read_files(&[("a.txt", a), ("b.txt", b)], &[("a.txt", a), ("b.txt", b)]);
        let Lookup::Template(template) = triggers.lookup(b"my_check") else {
            panic!("my_check");
        };
        assert_eq!(template.name, b"My_Check");
        assert_eq!(keys(template.body.as_ref().unwrap()), ["has_reserves"]);
        let a_only = read_files(&[("a.txt", a)], &[("a.txt", a)]);
        let body = body(&a_only, b"MY_CHECK").unwrap();
        assert_eq!(keys(&body), ["is_attacker"]);
        assert_eq!(body[0].value.scalar(), Some(b"yes".as_slice()));
        assert_eq!(a_only.lookup(b"absent"), Lookup::Missing);
    }

    /// The name pass skips each value, counting the braces it reads as
    /// written and resolving nothing: a reference to a definition that kept
    /// only a `{` opens no block, so both names register.
    #[test]
    fn the_name_pass_skips_braces_as_written() {
        let mut triggers = ScriptedTriggers::default();
        triggers.read_names(b"@o = { first = { @o } second = { }", "a.txt");
        assert!(matches!(triggers.lookup(b"first"), Lookup::Template(_)));
        assert!(matches!(triggers.lookup(b"second"), Lookup::Template(_)));
    }

    /// A key the name pass did not register (a file below the directory)
    /// logs a mismatch and its block is not skipped: the reader reads its
    /// contents as the file's keys and its `}` as a key. A scalar body is
    /// logged and holds no condition.
    #[test]
    fn a_mismatch_leaves_the_file_out_of_step() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let top: &[u8] = b"first = { is_attacker = yes } later = { has_reserves = yes } flat = yes";
        let nested: &[u8] =
            b"first = { is_attacker = no } unregistered = { later = { } } flat = { frontage_full = yes }";
        let triggers = read_files(
            &[("top.txt", top)],
            &[("top.txt", top), ("sub/nested.txt", nested)],
        );
        assert_eq!(keys(&body(&triggers, b"first").unwrap()), ["is_attacker"]);
        // `later = { }` inside the mismatch is read as a top-level body.
        assert_eq!(body(&triggers, b"later"), Ok(Vec::new()));
        assert!(
            triggers
                .logged
                .iter()
                .any(|line| line.contains("flat = yes is not a block")),
            "{:?}",
            triggers.logged
        );
        assert!(
            triggers.logged.iter().any(|line| line
                .contains("scripted trigger unregistered: the body pass logs a mismatch")),
            "{:?}",
            triggers.logged
        );
        assert_eq!(triggers.lookup(b"unregistered"), Lookup::Missing);
    }

    /// The name pass skips each value and registers every key it reads:
    /// a value without a key is read as a key, with the next token taken
    /// as its operator.
    #[test]
    fn an_unproven_name_pass_makes_misses_unproven() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let triggers = read_files(&[("a.txt", b"a = { } 5 b = { } c = { }")], &[]);
        assert!(matches!(triggers.lookup(b"A"), Lookup::Template(_)));
        // `5` consumes `b` as its operator: `b` names no template.
        assert!(matches!(triggers.lookup(b"5"), Lookup::Template(_)));
        assert_eq!(triggers.lookup(b"b"), Lookup::Missing);
        assert!(
            triggers.logged[0].contains("key `5` has no operator"),
            "{:?}",
            triggers.logged
        );
    }

    /// A block under a key whose reader the names cannot tell leaves the
    /// rest of the file unknown: every template's body is unproven from
    /// there, until a later file reads it.
    #[test]
    fn an_unsettled_body_leaves_every_template_unproven_until_read_again() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let a: &[u8] = b"first = { is_attacker = yes } second = { OR = { x = { } } }";
        let b: &[u8] = b"FIRST = { has_reserves = yes }";
        let triggers = ScriptedTriggers::parse(&[("a.txt", a), ("b.txt", b)], &KeyNames::unknown());
        assert_eq!(keys(&body(&triggers, b"first").unwrap()), ["has_reserves"]);
        let why = body(&triggers, b"second").unwrap_err();
        assert!(
            why.contains("field `x`: trigger dispatch reads it"),
            "{why}"
        );
    }

    /// A block under a scripted trigger's key is read as one value, with no
    /// block skipped: its contents are the enclosing body's children and its
    /// `}` ends that body, so the file is read on from there.
    #[test]
    fn a_block_under_a_scripted_trigger_is_read_as_one_value() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let a: &[u8] =
            b"inner = { } outer = { inner = { is_attacker = yes } } after = { is_attacker = no }";
        let triggers = ScriptedTriggers::parse(&[("a.txt", a)], &KeyNames::default());
        let outer = body(&triggers, b"outer").unwrap();
        assert_eq!(keys(&outer), ["inner", "is_attacker"]);
        assert_eq!(outer[0].value.scalar(), Some(b"{".as_slice()));
        assert_eq!(body(&triggers, b"after").unwrap(), []);
        assert!(
            triggers
                .logged
                .iter()
                .any(|line| line
                    .contains("field `inner` is a block where its reader takes one value")),
            "{:?}",
            triggers.logged
        );
    }

    #[test]
    fn description_read_retains_bindings_without_changing_consumption() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let text = b"target={FROM={custom_trigger_tooltip={tooltip=SYN_POS not_tooltip=SYN_NEG @limit=2 skill > @limit}}}";
        for retain in [false, true] {
            let mut database = ScriptedTriggers::default();
            database.read_names(text, "synthetic.txt");
            let names = KeyNames {
                scripted: database.templates.keys().cloned().collect(),
                ..KeyNames::default()
            };
            database.read_bodies(text, "synthetic.txt", &names, retain);
            assert!(database.logged.is_empty(), "{:?}", database.logged);
            let target = body(&database, b"target").unwrap();
            let context = target[0].value.items().unwrap();
            let fields = context[0].value.items().unwrap();
            assert_eq!(
                keys(fields),
                if retain {
                    vec![
                        "tooltip".to_owned(),
                        "not_tooltip".to_owned(),
                        "skill".to_owned(),
                    ]
                } else {
                    vec!["skill".to_owned()]
                }
            );
            assert_eq!(fields.last().unwrap().value.scalar(), Some(b"2".as_slice()));
            if retain {
                assert_eq!(fields[0].value.scalar(), Some(b"SYN_POS".as_slice()));
                assert_eq!(fields[1].value.scalar(), Some(b"SYN_NEG".as_slice()));
            }
        }
    }
}
