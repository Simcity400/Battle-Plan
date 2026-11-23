use content_reader_key_names::key_name_products::KeyNamesExt as _;
use content_reader_layers::content::diagnostics::Once as OnceLock;
use content_reader_layers::{ResolvedLayers, script_files};
use content_reader_parser::{coverage::Coverage, parser::Item, tokens::Token};
use crate::faction_templates;
use content_reader_script::localization_binding::BoundLocalization;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};

#[derive(Debug, Default)]
pub struct Definition {
    pub name: BoundLocalization,
    pub description: BoundLocalization,
    pub category: Option<Vec<u8>>,
    pub group: Option<Vec<u8>>,
    pub progress_inputs: Vec<Item>,
    pub progress_sections: Vec<Vec<Item>>,
    pub progress_section_keys: Vec<Option<Vec<u8>>>,
    pub conditions: BTreeMap<String, Vec<Item>>,
    pub effects: BTreeMap<String, Vec<Item>>,
    pub flags: BTreeMap<String, Result<bool, String>>,
}

#[derive(Debug, Default)]
pub struct Catalog {
    definitions: BTreeMap<Token, Definition>,
    coverage: Coverage,
    ambiguous: BTreeSet<Token>,
    pub warnings: Vec<String>,
}
impl Catalog {
    pub fn definitions(&self) -> impl Iterator<Item = (&Token, &Definition)> {
        self.definitions.iter()
    }

    pub fn get(&self, name: &str) -> Result<Option<&Definition>, String> {
        let token = Token::c_str(name.as_bytes());
        if self.ambiguous.contains(&token) {
            return Err(format!(
                "Faction goal {name:?} names distinct inline definitions"
            ));
        }
        if let Some(why) = self.coverage.unknown(&identity(&token)) {
            return Err(why.to_string());
        }
        Ok(self.definitions.get(&token))
    }
}
fn identity(token: &Token) -> String {
    match token {
        Token::Name(name) => String::from_utf8_lossy(name).into_owned(),
        Token::Rejected => "\0".into(),
    }
}
fn apply(catalog: &mut Catalog, key: Option<&[u8]>, fields: &[Item], inline: bool) {
    let token = Token::c_str(key.unwrap_or_default());
    if inline && catalog.definitions.contains_key(&token) {
        catalog.ambiguous.insert(token.clone());
        catalog.warnings.push(format!(
            "Faction goal {:?} names distinct inline definitions",
            identity(&token)
        ));
    }
    catalog.coverage.add(&identity(&token));
    let definition = catalog.definitions.entry(token.clone()).or_default();
    for field in fields {
        if field.keyword_is("name") {
            definition.name = BoundLocalization::from_value(&field.value);
        } else if field.keyword_is("description") {
            definition.description = BoundLocalization::from_value(&field.value);
        } else if let Some(key) = ["allowed", "visible", "available", "completed", "cancel"]
            .into_iter()
            .find(|key| field.keyword_is(key))
        {
            definition
                .conditions
                .entry(key.to_owned())
                .or_default()
                .extend_from_slice(field.value.items().unwrap_or_default());
        } else if let Some(key) = [
            "select_effect",
            "complete_effect",
            "cancel_effect",
            "remove_effect",
        ]
        .into_iter()
        .find(|key| field.keyword_is(key))
        {
            definition
                .effects
                .entry(key.to_owned())
                .or_default()
                .extend_from_slice(field.value.items().unwrap_or_default());
        } else if let Some(key) = ["locked_goal", "is_manifest", "auto_complete"]
            .into_iter()
            .find(|key| field.keyword_is(key))
        {
            definition.flags.insert(
                key.to_owned(),
                content_reader_parser::fields::boolean(&field.value, key).map_err(|why| why.to_string()),
            );
        } else if field.keyword_is("ratio_progress") {
            for input in field.value.items().unwrap_or_default() {
                if input.keyword_is("progress_sections") {
                    for section in input.value.items().unwrap_or_default() {
                        if let Some(body) = section.value.items() {
                            definition.progress_sections.push(body.to_vec());
                            definition
                                .progress_section_keys
                                .push(section.key.as_deref().map(<[u8]>::to_vec));
                        }
                    }
                } else {
                    if let Some(key) = input.key.as_deref() {
                        definition.progress_inputs.retain(|old| {
                            old.key
                                .as_deref()
                                .is_none_or(|old| !old.eq_ignore_ascii_case(key))
                        });
                    }
                    definition.progress_inputs.push(input.clone());
                }
            }
        } else if field.keyword_is("group") {
            definition.group = field.value.scalar().map(<[u8]>::to_vec);
        } else if field.keyword_is("category") {
            let category = field.value.scalar().unwrap_or_default();
            definition.category = [b"short_term".as_slice(), b"medium_term", b"long_term"]
                .into_iter()
                .find(|term| term.eq_ignore_ascii_case(category))
                .map(<[u8]>::to_vec);
            if definition.category.is_none() {
                catalog.warnings.push(format!(
                    "Faction goal {:?}: category {:?} selects no native term",
                    identity(&token),
                    String::from_utf8_lossy(category)
                ));
            }
        }
    }
}
pub fn cached(layers: &ResolvedLayers) -> Result<Arc<Catalog>, String> {
    let memo = layers.derived::<OnceLock<Result<Arc<Catalog>, String>>>();
    memo.get_or_init(|| {
        read(layers)
            .map(Arc::new)
            .map_err(|why| format!("Faction goals are unreadable: {why}"))
    })
    .clone()
}
pub fn read(layers: &ResolvedLayers) -> io::Result<Catalog> {
    let names = layers.key_names()?;
    let mut catalog = Catalog::default();
    for (folder, templates) in [
        ("common/factions/goals/", false),
        ("common/factions/templates/", true),
    ] {
        for path in script_files::paths(layers, folder, Some(".txt"))? {
            catalog.coverage.next_file();
            let bytes = path.read(layers)?;
            let read = if templates {
                faction_templates::parse(&bytes, &names)
            } else {
                faction_templates::parse_goal_file(&bytes, &names)
            };
            for item in &read.items {
                let Some(fields) = item.value.items() else {
                    continue;
                };
                if templates {
                    for field in fields.iter().filter(|field| field.keyword_is("goals")) {
                        for goal in field.value.items().unwrap_or_default() {
                            if let Some(fields) = goal.value.items() {
                                apply(&mut catalog, goal.key.as_deref(), fields, true);
                            }
                        }
                    }
                } else {
                    apply(&mut catalog, item.key.as_deref(), fields, false);
                }
            }
            catalog.warnings.extend(
                read.misreads
                    .iter()
                    .map(|why| format!("Faction goals in {path}: {why}")),
            );
            if let Some(why) = read.unsettled {
                catalog.coverage.gap(&path.text(), why.to_string());
            }
        }
    }
    Ok(catalog)
}

#[cfg(test)]
mod tests {
    use super::*;
    use content_reader_script::key_names::KeyNames;
    #[test]
    fn faction_goal_content_uses_declaration_identity_and_native_category_selection() {
        content_reader_script::test_game::enter();
        let read = faction_templates::parse_goal_file(br#"a_mod_goal={id=chosen name={localization_key=chosen_name P=TEXT} category=mod_term description=chosen_desc}"#, &KeyNames::default());
        assert!(read.unsettled.is_none(), "{:?}", read.unsettled);
        let mut catalog = Catalog::default();
        for item in read.items {
            apply(
                &mut catalog,
                item.key.as_deref(),
                item.value.items().unwrap(),
                false,
            );
        }
        assert!(catalog.get("chosen").unwrap().is_none());
        let definition = catalog.get("a_mod_goal").unwrap().unwrap();
        assert_eq!(definition.category, None);
        assert_eq!(catalog.warnings.len(), 1);
        assert!(!definition.name.is_empty());
        assert!(!definition.description.is_empty());
    }
    #[test]
    fn faction_goal_content_merges_only_reached_fields_and_refuses_inline_identity_collisions() {
        content_reader_script::test_game::enter();
        let mut catalog = Catalog::default();
        for bytes in [
            b"mod_goal={name=first description=kept category=long_term}".as_slice(),
            b"mod_goal={name=second}".as_slice(),
        ] {
            let read = faction_templates::parse_goal_file(bytes, &KeyNames::default());
            assert!(read.unsettled.is_none());
            for item in read.items {
                apply(
                    &mut catalog,
                    item.key.as_deref(),
                    item.value.items().unwrap(),
                    false,
                );
            }
        }
        let definition = catalog.get("mod_goal").unwrap().unwrap();
        assert_eq!(
            definition.category.as_deref(),
            Some(b"long_term".as_slice())
        );
        assert!(!definition.description.is_empty());
        let read = faction_templates::parse_goal_file(
            b"mod_goal={description=inline}",
            &KeyNames::default(),
        );
        let item = &read.items[0];
        apply(
            &mut catalog,
            item.key.as_deref(),
            item.value.items().unwrap(),
            true,
        );
        assert!(
            catalog
                .get("mod_goal")
                .unwrap_err()
                .contains("distinct inline definitions")
        );
        assert_eq!(catalog.warnings.len(), 1);
    }
}

#[cfg(test)]
mod progress_tests {
    use super::*;
    #[test]
    fn faction_goal_retains_ordered_progress_sections_reached_checks_flags_and_effects() {
        content_reader_script::test_game::enter();
        let read=faction_templates::parse_goal_file(br#"mod_goal={ locked_goal=yes allowed={always=no} complete_effect={add_stability=0.1} ratio_progress={total_amount=7 completed_amount=var:mod_value progress_sections={a={min=-1 max=0.5 on_activate={add_stability=0.2}} b={min=0.5 max=1}}}}"#, &content_reader_script::key_names::KeyNames::default());
        assert!(read.unsettled.is_none(), "{:?}", read.unsettled);
        let mut catalog = Catalog::default();
        let item = &read.items[0];
        apply(
            &mut catalog,
            item.key.as_deref(),
            item.value.items().unwrap(),
            false,
        );
        let definition = catalog.get("mod_goal").unwrap().unwrap();
        assert_eq!(definition.flags["locked_goal"], Ok(true));
        assert_eq!(definition.conditions["allowed"].len(), 1);
        assert_eq!(definition.effects["complete_effect"].len(), 1);
        assert_eq!(definition.progress_inputs.len(), 2);
        assert_eq!(definition.progress_sections.len(), 2);
        assert_eq!(
            definition.progress_section_keys,
            [Some(b"a".to_vec()), Some(b"b".to_vec())]
        );
        assert!(
            definition.progress_sections[0]
                .iter()
                .any(|item| item.keyword_is("on_activate"))
        );
    }
}

#[cfg(test)]
mod group_tests {
    use super::*;
    #[test]
    fn faction_goal_group_retention_preserves_mod_spelling_and_last_authored_value() {
        let fields = content_reader_parser::parser::parse(b"group=first group=mod_group").unwrap();
        let mut catalog = Catalog::default();
        apply(&mut catalog, Some(b"mod_goal"), &fields.items, false);
        assert_eq!(
            catalog.get("mod_goal").unwrap().unwrap().group.as_deref(),
            Some(b"mod_group".as_slice())
        );
        apply(&mut catalog, Some(b"mod_goal"), &[], false);
        assert_eq!(
            catalog.get("mod_goal").unwrap().unwrap().group.as_deref(),
            Some(b"mod_group".as_slice())
        );
    }
    #[test]
    fn faction_goal_native_standalone_and_inline_groups_retain_mod_variables_and_quotes() {
        content_reader_script::test_game::enter();
        let stack = content_reader_layers::test_support::Stack::new("faction-goal-groups");
        stack.write(
            &stack.base(),
            "common/factions/goals/removed.txt",
            "old={group=old_group}",
        );
        stack.write(&stack.addon(),"common/factions/goals/mod.txt","@ui_group=mod_group mod_goal={group=@ui_group} quoted_goal={group=\"mod quoted group\"}");
        stack.write(
            &stack.addon(),
            "common/factions/templates/mod.txt",
            "mod_template={goals={inline_goal={group=mod_inline_group}}}",
        );
        let catalog = read(&stack.layers(&["common/factions/goals"])).unwrap();
        assert!(catalog.get("old").unwrap().is_none());
        for (goal, group) in [
            ("mod_goal", "mod_group"),
            ("quoted_goal", "mod quoted group"),
            ("inline_goal", "mod_inline_group"),
        ] {
            assert_eq!(
                catalog.get(goal).unwrap().unwrap().group.as_deref(),
                Some(group.as_bytes())
            );
        }
    }
}
