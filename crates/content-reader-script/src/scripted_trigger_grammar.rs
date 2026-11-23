
use std::collections::BTreeMap;

use crate::{trigger_grammar::Node, key_names::KeyNames, native::{self, Shape}};
use content_reader_parser::parser::{Item, Value, c_string};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Template {
    pub name: Vec<u8>,
    pub body: Result<Vec<Item>, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScriptedTriggers {
    pub templates: BTreeMap<Vec<u8>, Template>,
    pub logged: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lookup<'a> {
    Template(&'a Template),
    Missing,
}

impl ScriptedTriggers {
    pub fn lookup(&self, text: &[u8]) -> Lookup<'_> {
        match self.templates.get(&c_string(text).to_ascii_lowercase()) {
            Some(template) => Lookup::Template(template),
            None => Lookup::Missing,
        }
    }

    pub fn read_names(&mut self, bytes: &[u8], path: &str) {
        let read = native::read(
            bytes,
            native::Mode::KeepMark,
            native::Top::File(()),
            None,
            &mut |(), _| Shape::SkipIfOpen,
        );
        self.note(path, &read);
        for item in &read.items {
            let key = c_string(item.key.as_deref().unwrap_or_default()).to_vec();
            self.templates
                .entry(key.to_ascii_lowercase())
                .or_insert(Template {
                    name: key,
                    body: Ok(Vec::new()),
                });
        }
    }

    pub fn read_bodies(&mut self, bytes: &[u8], path: &str, names: &KeyNames, tooltip_bindings: bool) {
        let templates = &self.templates;
        let mut shaper = crate::trigger_grammar::Shaper::new(Some(names));
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
                Node::Top => Shape::One,
                node => shaper.shape(node, field),
            },
        );
        self.note(path, &read);
        for item in read.items {
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
                    crate::trigger_grammar::normalise_trigger_body(
                        &place,
                        &mut children,
                        &mut self.logged,
                        tooltip_bindings,
                    );
                    children
                }
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
            let reason = format!(
                "the scripted-trigger body pass over {path} cannot be followed from {unsettled}"
            );
            for template in self.templates.values_mut() {
                template.body = Err(reason.clone());
            }
            self.logged.push(reason);
        }
    }

    fn note(&mut self, path: &str, read: &native::Read) {
        self.logged.extend(
            read.misreads
                .iter()
                .map(|misread| format!("{path} {misread}; read as the game reads it")),
        );
    }
}

impl ScriptedTriggers {
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

    #[test]
    fn names_register_first_and_bodies_read_last() {
        crate::test_game::enter();
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

    #[test]
    fn the_name_pass_skips_braces_as_written() {
        let mut triggers = ScriptedTriggers::default();
        triggers.read_names(b"@o = { first = { @o } second = { }", "a.txt");
        assert!(matches!(triggers.lookup(b"first"), Lookup::Template(_)));
        assert!(matches!(triggers.lookup(b"second"), Lookup::Template(_)));
    }

    #[test]
    fn a_mismatch_leaves_the_file_out_of_step() {
        crate::test_game::enter();
        let top: &[u8] = b"first = { is_attacker = yes } later = { has_reserves = yes } flat = yes";
        let nested: &[u8] =
            b"first = { is_attacker = no } unregistered = { later = { } } flat = { frontage_full = yes }";
        let triggers = read_files(
            &[("top.txt", top)],
            &[("top.txt", top), ("sub/nested.txt", nested)],
        );
        assert_eq!(keys(&body(&triggers, b"first").unwrap()), ["is_attacker"]);
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

    #[test]
    fn an_unproven_name_pass_makes_misses_unproven() {
        crate::test_game::enter();
        let triggers = read_files(&[("a.txt", b"a = { } 5 b = { } c = { }")], &[]);
        assert!(matches!(triggers.lookup(b"A"), Lookup::Template(_)));
        assert!(matches!(triggers.lookup(b"5"), Lookup::Template(_)));
        assert_eq!(triggers.lookup(b"b"), Lookup::Missing);
        assert!(
            triggers.logged[0].contains("key `5` has no operator"),
            "{:?}",
            triggers.logged
        );
    }

    #[test]
    fn an_unsettled_body_leaves_every_template_unproven_until_read_again() {
        crate::test_game::enter();
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

    #[test]
    fn a_block_under_a_scripted_trigger_is_read_as_one_value() {
        crate::test_game::enter();
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
        crate::test_game::enter();
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
