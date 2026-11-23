use content_reader_key_names::key_name_products::KeyNamesExt as _;

use content_reader_layers::{ResolvedLayers, script_files};
use content_reader_parser::{coverage::Coverage, parser::Item, tokens::Token};
use content_reader_script::{effect_keys, localization_binding::BoundLocalization, key_names::KeyNames, modifiers, native::{self, Shape}};
use std::{collections::BTreeMap, io, sync::Arc};

#[derive(Debug, Default)]
pub struct Catalog {
    definitions: BTreeMap<Token, BoundLocalization>,
    icons: BTreeMap<Token, Vec<u8>>,
    names: Coverage,
    icon_sources: Coverage,
    pub warnings: Vec<String>,
}

impl Catalog {
    pub fn get(&self, token: &str) -> Result<Option<&BoundLocalization>, String> {
        let token = Token::c_str(token.as_bytes());
        if let Some(why) = self.names.unknown(&identity(&token)) {
            return Err(why.to_string());
        }
        Ok(self.definitions.get(&token))
    }
    pub fn icon(&self, token: &str) -> Result<Option<&[u8]>, String> {
        let token = Token::c_str(token.as_bytes());
        if let Some(why) = self.icon_sources.unknown(&identity(&token)) {
            return Err(why.to_string());
        }
        Ok(self.icons.get(&token).map(Vec::as_slice))
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
    Rule(content_reader_script::idea_fields::RuleNode),
    Trigger(content_reader_script::trigger_grammar::Node),
    Mtth,
}

pub(crate) fn parse(bytes: &[u8], names: &KeyNames) -> native::Read {
    parse_top(bytes, names, Node::File)
}

pub(crate) fn parse_goal_file(bytes: &[u8], names: &KeyNames) -> native::Read {
    parse_top(bytes, names, Node::UniformGoal)
}

fn parse_top(bytes: &[u8], names: &KeyNames, top: Node) -> native::Read {
    let registry = |key: &[u8]| modifiers::registered(key, Some(names));
    let mut undecided = Vec::new();
    let mut triggers = content_reader_script::trigger_grammar::Shaper::new(Some(names));
    native::read(
        bytes,
        native::Mode::KeepMark,
        native::Top::File(top),
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
                Some(b"default_rules") => Shape::List,
                Some(b"goals") => Shape::Statements(Node::UniformGoal),
                _ => Shape::Skip,
            },
            Node::UniformGoal if field.opens() => Shape::Object(Node::Goal),
            Node::UniformGoal => Shape::One,
            Node::Goal => match field.keyword().as_deref() {
                Some(b"name" | b"description") => {
                    effect_keys::bound_localization_shape(field).map(|()| Node::Goal)
                }
                Some(
                    b"id" | b"group" | b"category" | b"locked_goal" | b"is_manifest"
                    | b"auto_complete",
                ) => Shape::One,
                Some(b"allowed" | b"visible" | b"available" | b"completed" | b"cancel") => {
                    Shape::Trigger
                }
                Some(
                    b"select_effect" | b"complete_effect" | b"cancel_effect" | b"remove_effect",
                ) => Shape::EffectIn(4),
                Some(b"ai_will_do") => Shape::Object(Node::Mtth),
                Some(b"ratio_progress") => Shape::ObjectOrSkip(Node::Ratio),
                _ => Shape::Skip,
            },
            Node::Ratio => match field.keyword().as_deref() {
                Some(b"min" | b"max" | b"total_amount" | b"completed_amount") => Shape::One,
                Some(b"total_amount_custom_tooltip" | b"completed_amount_custom_tooltip") => {
                    effect_keys::bound_localization_shape(field).map(|()| Node::Ratio)
                }
                Some(b"scale" | b"reversed") => {
                    Shape::Object(Node::Modifier(modifiers::Node::Targeted))
                }
                Some(b"range") => Shape::Object(Node::MinMax),
                Some(b"total_amount_collection" | b"completed_amount_collection") => {
                    Shape::ObjectIfOpen(Node::Trigger(content_reader_script::trigger_grammar::Node::KeyReader(
                        crate::collections::COLLECTION_KEY_READER,
                    )))
                }
                Some(b"progress_sections") => Shape::Statements(Node::Sections),
                _ => Shape::Skip,
            },
            Node::MinMax if matches!(field.keyword().as_deref(), Some(b"min" | b"max")) => {
                Shape::One
            }
            Node::MinMax => Shape::Skip,
            Node::Sections if field.key.text.is_empty() => Shape::Skip,
            Node::Sections => Shape::Object(Node::Section),
            Node::Section => match field.keyword().as_deref() {
                Some(b"min" | b"max") => Shape::One,
                Some(b"modifier") => Shape::Object(Node::Modifier(modifiers::Node::Targeted)),
                Some(b"rule") => Shape::Object(Node::Rule(content_reader_script::idea_fields::RuleNode::Rule)),
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
            Node::Rule(node) => content_reader_script::idea_fields::rule_field(node, field).map(Node::Rule),
            Node::Trigger(node) => triggers.shape(node, field).map(Node::Trigger),
            Node::Mtth => native::mean_time_to_happen(field),
        },
    )
}

fn apply(catalogue: &mut Catalog, items: &[Item]) {
    for item in items {
        let Some(fields) = item.value.items() else {
            continue;
        };
        let token = Token::c_str(item.key.as_deref().unwrap_or_default());
        let found = catalogue.definitions.entry(token.clone()).or_default();
        for field in fields {
            if field.keyword_is("name") {
                *found = BoundLocalization::from_value(&field.value);
                catalogue.names.replace(&identity(&token));
            } else if field.keyword_is("icon")
                && let Some(icon) = field.value.scalar()
            {
                catalogue.icons.insert(token.clone(), icon.to_vec());
                catalogue.icon_sources.replace(&identity(&token));
            }
        }
    }
}

pub fn read(layers: &ResolvedLayers) -> io::Result<Catalog> {
    let names = layers.key_names()?;
    let mut result = Catalog::default();
    for path in script_files::paths(layers, "common/factions/templates/", Some(".txt"))? {
        result.names.next_file();
        result.icon_sources.next_file();
        let file = parse(&path.read(layers)?, &names);
        apply(&mut result, &file.items);
        result.warnings.extend(
            file.misreads
                .iter()
                .map(|why| format!("Faction templates in {path}: {why}")),
        );
        if let Some(why) = file.unsettled {
            result.names.gap(&path.text(), why.to_string());
            result.icon_sources.gap(&path.text(), why.to_string());
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use content_reader_script::localization_binding::{BoundArgument, BoundNode, BoundValue, FormattedLocalization};
    use content_reader_layers::test_support::Stack;

    fn catalog(bytes: &[u8]) -> Catalog {
        let file = parse(bytes, &KeyNames::default());
        assert!(file.unsettled.is_none(), "{:?}", file.unsettled);
        let mut result = Catalog::default();
        apply(&mut result, &file.items);
        result
    }

    #[test]
    fn a_template_icon_is_its_last_definitions_sprite() {
        let read = catalog(b"a = { icon = GFX_first } a = { name = A icon = GFX_second } b = { name = B }");
        assert_eq!(read.icon("a").unwrap(), Some(&b"GFX_second"[..]));
        assert_eq!(read.icon("b").unwrap(), None);
        assert_eq!(read.icon("absent").unwrap(), None);
    }

    #[test]
    fn faction_template_names_merge_and_replace_the_whole_bound_tree() {
        content_reader_script::test_game::enter();
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
    fn faction_bound_names_keep_arguments_and_preorder_nodes() {
        content_reader_script::test_game::enter();
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
    fn faction_template_inline_goals_follow_the_nested_readers() {
        content_reader_script::test_game::enter();
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
        content_reader_script::test_game::enter();
        let root = std::env::temp_dir().join(format!(
            "content-reader-faction-templates-{}",
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
    fn faction_templates_refuse_unknown_suffixes_until_an_explicit_field_replaces_them() {
        content_reader_script::test_game::enter();
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
            "kept={name=KNOWN} hidden={icon=art} late={icon=late_art}",
        );
        let db = read(&stack.layers(&[])).unwrap();
        assert_eq!(
            db.get("kept").unwrap().unwrap().nodes[0].key,
            FormattedLocalization::Key(b"KNOWN".to_vec())
        );
        for token in ["hidden", "uncertain", "absent", "late"] {
            let why = db.get(token).unwrap_err();
            assert!(why.contains("b.txt"), "{why}");
            assert!(why.contains("synthetic_prefix"), "{why}");
        }
        for token in ["kept", "uncertain", "absent"] {
            let why = db.icon(token).unwrap_err();
            assert!(why.contains("b.txt"), "{why}");
        }
        assert_eq!(db.icon("hidden").unwrap(), Some(&b"art"[..]));
        assert_eq!(db.icon("late").unwrap(), Some(&b"late_art"[..]));
    }
}

pub trait FactionTemplateProducts {
    fn faction_templates(&self) -> Result<Arc<crate::faction_templates::Catalog>, String>;
    fn faction_templates_read(&self) -> bool;
    fn faction_template_diagnostics(&self) -> Vec<String>;
}

impl FactionTemplateProducts for ResolvedLayers {
    fn faction_templates(&self) -> Result<Arc<crate::faction_templates::Catalog>, String> {
        self.retained_product_with_errors("faction templates", 1, &[], true, crate::faction_templates::read)
            .map_err(|error| format!("Faction templates are unreadable: {error}"))
    }

    fn faction_templates_read(&self) -> bool {
        self.retained_value::<crate::faction_templates::Catalog>("faction templates", 1, &[]).is_some()
    }

    fn faction_template_diagnostics(&self) -> Vec<String> {
        self.retained_value::<crate::faction_templates::Catalog>("faction templates", 1, &[])
            .and_then(Result::ok)
            .map_or_else(Vec::new, |templates| templates.warnings.clone())
    }
}
