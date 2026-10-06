//! Faction template display names through the recorded content stack.
//! CFaction::display_name (140D77360) uses the nullable template +0x568
//! only when its own +0x18 name is empty, then evaluates +0xD0's bound
//! localization (1404A7A00). The template token is not a name fallback.
//! Loader, token and binding evidence: [engine::cfaction_template](../../engine/src/cfaction_template.rs).

use crate::{
    ResolvedLayers, combat_tactics,
    coverage::Coverage,
    effect_keys,
    gui::native::{BoundArgument, BoundLocalization, BoundNode, BoundValue, FormattedLocalization},
    ideas,
    key_names::KeyNames,
    modifiers,
    parser::{
        Item, Value, c_string,
        native::{self, Shape},
    },
    script_files,
    tokens::Token,
};
use std::{collections::BTreeMap, io};

#[derive(Debug, Default)]
pub struct Catalog {
    definitions: BTreeMap<Token, BoundLocalization>,
    coverage: Coverage,
    pub warnings: Vec<String>,
}

impl Catalog {
    pub fn get(&self, token: &str) -> Result<Option<&BoundLocalization>, String> {
        let token = Token::c_str(token.as_bytes());
        if let Some(why) = self.coverage.unknown(&identity(&token)) {
            return Err(why.to_string());
        }
        Ok(self.definitions.get(&token))
    }
}

fn identity(token: &Token) -> String {
    match token {
        Token::Rejected => "\0".into(),
        Token::Name(name) => String::from_utf8_lossy(name).into_owned(),
    }
}

#[derive(Clone, Copy)]
enum Node {
    File,
    Template,
    UniformGoal,
    Goal,
    Ratio,
    MinMax,
    Sections,
    Section,
    Modifier(modifiers::Node),
    Rule(ideas::RuleNode),
    Trigger(combat_tactics::Node),
    Mtth,
}

fn parse(bytes: &[u8], names: &KeyNames) -> native::Read {
    let registry = |key: &[u8]| modifiers::registered(key, Some(names));
    let mut undecided = Vec::new();
    let mut triggers = combat_tactics::Shaper::new(Some(names));
    // 140140720 -> 14012E4B0: mode 0, loop to EOF, repeated tokens reuse
    // their existing template. CFactionTemplate::ReadToken 140A220D0.
    native::read(
        bytes,
        native::Mode::KeepMark,
        native::Top::File(Node::File),
        Some(names),
        &mut |node, field| match node {
            Node::File => Shape::Object(Node::Template),
            Node::Template => match field.keyword().as_deref() {
                Some(b"name") => {
                    effect_keys::bound_localization_shape(field).map(|()| Node::Template)
                }
                Some(b"icon" | b"can_leader_join_other_factions" | b"manifest") => Shape::One,
                Some(b"color") => Shape::Color,
                Some(b"visible" | b"available") => Shape::Trigger,
                Some(b"default_rules") => Shape::List, // ReadDynamicTokenIdList 1403B6DD0.
                Some(b"goals") => Shape::Statements(Node::UniformGoal), // 1409BFE50.
                _ => Shape::Skip,
            },
            Node::UniformGoal if field.opens() => Shape::Object(Node::Goal),
            Node::UniformGoal => Shape::One,
            // Inline CFactionGoal::ReadToken 140A148A0; consumption matters
            // even though only the enclosing template's name is projected.
            Node::Goal => match field.keyword().as_deref() {
                Some(b"name" | b"description") => {
                    effect_keys::bound_localization_shape(field).map(|()| Node::Goal)
                }
                Some(b"id" | b"category" | b"locked_goal" | b"is_manifest" | b"auto_complete") => {
                    Shape::One
                }
                Some(b"allowed" | b"visible" | b"available" | b"completed" | b"cancel") => {
                    Shape::Trigger
                }
                Some(
                    b"select_effect" | b"complete_effect" | b"cancel_effect" | b"remove_effect",
                ) => Shape::EffectIn(4),
                Some(b"ai_will_do") => Shape::Object(Node::Mtth),
                Some(b"ratio_progress") => Shape::ObjectOrSkip(Node::Ratio), // 140A126A0.
                _ => Shape::Skip,
            },
            // CRatioProgress loop 140A126A0 -> 140A124B0, not a generic tree.
            Node::Ratio => match field.keyword().as_deref() {
                Some(b"min" | b"max" | b"total_amount" | b"completed_amount") => Shape::One,
                Some(b"total_amount_custom_tooltip" | b"completed_amount_custom_tooltip") => {
                    effect_keys::bound_localization_shape(field).map(|()| Node::Ratio)
                }
                Some(b"scale" | b"reversed") => {
                    Shape::Object(Node::Modifier(modifiers::Node::Targeted))
                }
                Some(b"range") => Shape::Object(Node::MinMax), // 140A10AC0.
                Some(b"total_amount_collection" | b"completed_amount_collection") => {
                    Shape::ObjectIfOpen(Node::Trigger(combat_tactics::Node::KeyReader(0x140A0FC90)))
                }
                Some(b"progress_sections") => Shape::Statements(Node::Sections), // 140A12690 -> 140A10D40.
                _ => Shape::Skip,
            },
            Node::MinMax if matches!(field.keyword().as_deref(), Some(b"min" | b"max")) => {
                Shape::One
            }
            Node::MinMax => Shape::Skip,
            // Raw values/anonymous blocks are reported by 1404968A0 /
            // 140496090; assignments create and read a CProgressSection.
            Node::Sections if field.key.text.is_empty() => Shape::Skip,
            Node::Sections => Shape::Object(Node::Section),
            Node::Section => match field.keyword().as_deref() {
                // 140A05820.
                Some(b"min" | b"max") => Shape::One,
                Some(b"modifier") => Shape::Object(Node::Modifier(modifiers::Node::Targeted)),
                Some(b"rule") => Shape::Object(Node::Rule(ideas::RuleNode::Rule)),
                Some(b"on_activate" | b"on_deactivate") => Shape::EffectIn(4),
                _ => Shape::Skip,
            },
            Node::Modifier(node) => {
                let shape =
                    modifiers::shape(node, field, &registry, &mut undecided).map(Node::Modifier);
                if let Some(why) = undecided.last() {
                    Shape::Unsettled(why.to_string())
                } else {
                    shape
                }
            }
            Node::Rule(node) => ideas::rule_field(node, field).map(Node::Rule),
            Node::Trigger(node) => triggers.shape(node, field).map(Node::Trigger),
            Node::Mtth => native::mean_time_to_happen(field),
        },
    )
}

fn formatted(value: &Value) -> FormattedLocalization {
    FormattedLocalization::parse(c_string(value.read_string().unwrap_or(b"{")))
}

/// Projection of the shared native bound reader (142390E40/1423912A0),
/// not a second text parser. A new `name` clears the previous node array.
fn bound(value: &Value) -> BoundLocalization {
    fn node(value: &Value, result: &mut BoundLocalization) -> usize {
        let at = result.nodes.len();
        result.nodes.push(BoundNode::default());
        if let Some(fields) = value.items() {
            for field in fields {
                if field.keyword_is("localization_key") {
                    result.nodes[at].key = formatted(&field.value);
                } else {
                    let value = if field.value.items().is_some() {
                        BoundValue::Node(node(&field.value, result))
                    } else {
                        BoundValue::Localization(formatted(&field.value))
                    };
                    result.nodes[at].arguments.push(BoundArgument {
                        name: c_string(field.key.as_deref().unwrap_or_default()).to_vec(),
                        value,
                    });
                }
            }
        } else {
            result.nodes[at].key = formatted(value);
        }
        at
    }
    let mut result = BoundLocalization::default();
    node(value, &mut result);
    if value.items().is_some() && result.nodes[0].key.is_empty_key() {
        result.nodes.clear(); // 142390E40: root lacks a localization key.
    }
    result
}

fn apply(catalogue: &mut Catalog, items: &[Item]) {
    for item in items {
        let Some(fields) = item.value.items() else {
            continue;
        };
        let token = Token::c_str(item.key.as_deref().unwrap_or_default());
        catalogue.coverage.add(&identity(&token));
        let found = catalogue.definitions.entry(token.clone()).or_default();
        for field in fields {
            if field.keyword_is("name") {
                *found = bound(&field.value);
                catalogue.coverage.replace(&identity(&token));
            }
        }
    }
}

pub fn read(layers: &ResolvedLayers) -> io::Result<Catalog> {
    let names = layers.key_names()?;
    let mut result = Catalog::default();
    // Executable directory literal 142707C90, shared database-load routine
    // 14018B610. Template definitions merge in native file/layer order.
    for path in script_files::paths(layers, "common/factions/templates/", Some(".txt")) {
        result.coverage.next_file();
        let file = parse(&layers.read(&path)?, &names);
        apply(&mut result, &file.items);
        result.warnings.extend(
            file.misreads
                .iter()
                .map(|why| format!("Faction templates in {path}: {why}")),
        );
        if let Some(why) = file.unsettled {
            result.coverage.gap(&path, why.to_string());
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mod_layers_tests::Stack;

    fn catalog(bytes: &[u8]) -> Catalog {
        let file = parse(bytes, &KeyNames::default());
        assert!(file.unsettled.is_none(), "{:?}", file.unsettled);
        let mut result = Catalog::default();
        apply(&mut result, &file.items);
        result
    }

    #[test]
    fn faction_template_names_merge_and_replace_the_whole_bound_tree() {
        let db = catalog(
            br#"@label = first f = { name = @label } F = { icon = art }
            g = { name = { localization_key = old arg = token:17 } name = final }
            empty = { name = old name = { ignored = value } }"#,
        );
        assert_eq!(
            db.get("f").unwrap().unwrap().nodes[0].key,
            FormattedLocalization::Key(b"first".to_vec())
        );
        assert_eq!(
            db.get("G").unwrap().unwrap().nodes,
            vec![BoundNode {
                key: FormattedLocalization::Key(b"final".to_vec()),
                arguments: Vec::new(),
            }]
        );
        assert!(db.get("empty").unwrap().unwrap().is_empty());
        assert!(db.get("absent").unwrap().is_none());
    }

    #[test]
    fn faction_bound_names_keep_native_arguments_and_preorder_nodes() {
        let db = catalog(
            br#"f = { name = { localization_key = ROOT
            arg = { localization_key = CHILD nested = token:17 }
            arg = country|target "localization_key" = literal localization_key = FINAL } }"#,
        );
        assert_eq!(
            db.get("f").unwrap().unwrap().nodes,
            vec![
                BoundNode {
                    key: FormattedLocalization::Key(b"FINAL".to_vec()),
                    arguments: vec![
                        BoundArgument {
                            name: b"arg".to_vec(),
                            value: BoundValue::Node(1)
                        },
                        BoundArgument {
                            name: b"arg".to_vec(),
                            value: BoundValue::Localization(FormattedLocalization::Formatter {
                                formatter: b"country".to_vec(),
                                token: b"target".to_vec()
                            })
                        },
                        BoundArgument {
                            name: b"localization_key".to_vec(),
                            value: BoundValue::Localization(FormattedLocalization::Key(
                                b"literal".to_vec()
                            ))
                        },
                    ]
                },
                BoundNode {
                    key: FormattedLocalization::Key(b"CHILD".to_vec()),
                    arguments: vec![BoundArgument {
                        name: b"nested".to_vec(),
                        value: BoundValue::Localization(FormattedLocalization::Token(
                            b"17".to_vec()
                        ))
                    }]
                },
            ]
        );
    }

    #[test]
    fn faction_template_inline_goals_follow_the_native_nested_readers() {
        let db = catalog(br#"f = { visible = { always = yes } available = { always = yes }
            default_rules = { rule_a rule_b } color = { 1 0 0 } manifest = goal
            goals = { goal inline = { name = { localization_key = GOAL a = B }
                description = DESC category = long_term completed = { always = yes }
                ratio_progress = { total_amount = 100 completed_amount = var:x
                    range = { min = 0 max = 1 } total_amount_collection = { input = faction_members }
                    scale = { research_speed_factor = 1 }
                    progress_sections = { low = { min = 0 max = 0.5 modifier = { research_speed_factor = 0.1 }
                        rule = { can_create_factions = yes } on_activate = { add_stability = 0.1 } } }
                } complete_effect = { add_stability = 0.1 } ai_will_do = { factor = 1 } } }
            name = AFTER } tail = { name = TAIL }"#);
        assert_eq!(
            db.get("f").unwrap().unwrap().nodes[0].key,
            FormattedLocalization::Key(b"AFTER".to_vec())
        );
        assert_eq!(
            db.get("tail").unwrap().unwrap().nodes[0].key,
            FormattedLocalization::Key(b"TAIL".to_vec())
        );
    }

    #[test]
    fn faction_templates_use_winning_files_and_merge_later_tokens() {
        let root = std::env::temp_dir().join(format!(
            "installed-content-faction-templates-{}",
            std::process::id()
        ));
        let base = root.join("base");
        let over = root.join("over");
        for (dir, file, text) in [
            (&base, "a.txt", "f = { name = BASE }"),
            (&over, "a.txt", "f = { name = WINNER }"),
            (&base, "b.txt", "F = { icon = art } g = { name = G }"),
        ] {
            let path = dir.join("common/factions/templates").join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let layers = ResolvedLayers::from_folders(&[(&base, &[][..]), (&over, &[][..])]).unwrap();
        let db = read(&layers).unwrap();
        assert!(db.warnings.is_empty(), "{:?}", db.warnings);
        assert_eq!(
            db.get("f").unwrap().unwrap().nodes[0].key,
            FormattedLocalization::Key(b"WINNER".to_vec())
        );
        assert_eq!(
            db.get("g").unwrap().unwrap().nodes[0].key,
            FormattedLocalization::Key(b"G".to_vec())
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn faction_templates_resolve_mod_files_merge_names_and_localisation() {
        let stack = Stack::new("faction_templates");
        let (base, addon) = (stack.base(), stack.addon());
        stack.write(
            &base,
            "common/factions/templates/a.txt",
            "pact={name=shared_name} removed_by_file={name=old}\n",
        );
        stack.write(
            &addon,
            "common/factions/templates/a.txt",
            "pact={name=shared_name}\n",
        );
        stack.write(
            &base,
            "common/factions/templates/b.txt",
            "merged={name=kept_name}\n",
        );
        stack.write(
            &addon,
            "common/factions/templates/z.txt",
            "MERGED={icon=GFX_mod}\n",
        );
        stack.write(
            &addon,
            "common/factions/templates/sub/ignored.txt",
            "nested={name=ignored}\n",
        );
        stack.write(
            &addon,
            "common/factions/templates/ignored.TXT",
            "uppercase={name=ignored}\n",
        );
        stack.write(
            &base,
            "localisation/english/names_l_english.yml",
            "\u{feff}l_english:\n shared_name:0 \"Base pact\"\n",
        );
        stack.write(
            &addon,
            "localisation/english/names_l_english.yml",
            "\u{feff}l_english:\n shared_name:0 \"Mod pact\"\n",
        );
        let layers = stack.layers(&[]);
        let templates = read(&layers).unwrap();
        let localisation = crate::localisation::read(&layers, "english").unwrap();
        let FormattedLocalization::Key(key) = &templates.get("PACT").unwrap().unwrap().nodes[0].key
        else {
            panic!("expected the template's localisation key");
        };
        assert_eq!(localisation.localise(key).unwrap(), "Mod pact");
        assert_eq!(
            templates.get("merged").unwrap().unwrap().nodes[0].key,
            FormattedLocalization::Key(b"kept_name".to_vec())
        );
        for absent in ["removed_by_file", "nested", "uppercase"] {
            assert!(templates.get(absent).unwrap().is_none());
        }
        assert!(templates.warnings.is_empty(), "{:?}", templates.warnings);
    }

    #[test]
    fn faction_templates_refuse_unknown_suffixes_until_an_explicit_name_replaces_them() {
        let stack = Stack::new("faction_template_coverage");
        let base = stack.base();
        stack.write(
            &base,
            "common/factions/templates/a.txt",
            "kept={name=OLD} hidden={name=OLD}",
        );
        stack.write(
            &base,
            "common/factions/templates/b.txt",
            "uncertain={visible={synthetic_prefix:scope={always=yes}} name=WRONG}",
        );
        stack.write(
            &base,
            "common/factions/templates/c.txt",
            "kept={name=KNOWN} hidden={icon=art}",
        );
        let db = read(&stack.layers(&[])).unwrap();
        assert_eq!(
            db.get("kept").unwrap().unwrap().nodes[0].key,
            FormattedLocalization::Key(b"KNOWN".to_vec())
        );
        for token in ["hidden", "uncertain", "absent"] {
            let why = db.get(token).unwrap_err();
            assert!(why.contains("b.txt"), "{why}");
            assert!(why.contains("synthetic_prefix"), "{why}");
        }
    }
}
