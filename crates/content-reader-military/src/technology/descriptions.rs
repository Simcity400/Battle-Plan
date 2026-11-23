use super::*;
use content_reader_parser::tokens::Token;

#[derive(Clone, Debug, Default)]
pub struct Definition {
    pub id: Vec<u8>,
    pub parent: Vec<u8>,
    pub small_layout: bool,
    pub equipment: Vec<Item>,
    pub unit_bonus: Vec<Item>,
    pub modifiers: modifiers::Modifiers,
    pub modules: Vec<Item>,
    pub sub_units: Vec<Item>,
    pub buildings: Vec<(Vec<u8>, i32)>,
    pub tactics: Vec<Item>,
    pub on_research_complete: Vec<Item>,
    pub on_research_complete_limit: Vec<Item>,
}

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub definitions: Vec<Definition>,
    by_name: BTreeMap<Vec<u8>, usize>,
    pub categories: BTreeMap<Token, Vec<u8>>,
    pub equipment: content_reader_key_names::equipment::Database,
    pub tactics: Option<Result<crate::combat_tactics::Database, String>>,
    pub effect_refusal: Option<String>,
    pub notes: Vec<String>,
    pub refusal: Option<String>,
}

impl Catalog {
    pub fn read(layers: &ResolvedLayers, names: &KeyNames) -> io::Result<Self> {
        let sub_units = names
            .sub_units
            .as_ref()
            .ok_or_else(|| invalid("technology description sub-unit roster absent"))?;
        let registry = units::Registry {
            units: sub_units
                .registered()
                .map(|name| name.to_ascii_lowercase())
                .collect(),
            categories: sub_units.categories().clone(),
            terrains: names.effects.terrains.clone(),
            equipment: names.effects.equipment.clone(),
        };
        let mut catalog = Self {
            equipment: content_reader_key_names::equipment::Database::load_complete(layers)?,
            ..Self::default()
        };
        for path in layers.directory_files("common/technologies", ".txt")? {
            let (read, undecided, refused) = read_templates(&path.read(layers)?, &registry, names);
            catalog
                .notes
                .extend(read.misreads.iter().map(|note| format!("{path}: {note}")));
            if let Some(why) = &read.unsettled {
                catalog.refusal = Some(format!("{path}: technology description unread: {why}"));
            }
            if let Some(why) = undecided
                .first()
                .map(ToString::to_string)
                .or_else(|| {
                    refused
                        .iter()
                        .find(|why| !why.starts_with("technology_folders:"))
                        .cloned()
                })
            {
                catalog.effect_refusal = Some(format!("{path}: technology effects unread: {why}"));
            }
            catalog.append(&read.items, &registry, names);
        }
        if catalog
            .definitions
            .iter()
            .any(|definition| !definition.tactics.is_empty())
        {
            catalog.tactics =
                Some(crate::combat_tactics::read(layers).map_err(|why| why.to_string()));
        }
        Ok(catalog)
    }

    fn append(&mut self, roots: &[Item], registry: &units::Registry, names: &KeyNames) {
        for root in roots {
            if root.keyword_is("technology_categories") {
                for entry in root.value.items().unwrap_or_default() {
                    if let Some(name) = entry.value.scalar() {
                        self.categories
                            .entry(Token::c_str(name))
                            .or_insert_with(|| parser::c_string(name).to_vec());
                    }
                }
            }
            if !root.keyword_is("technologies") {
                continue;
            }
            for item in root.value.items().unwrap_or_default() {
                let id = parser::c_string(item.key.as_deref().unwrap_or_default());
                let identity = id.to_ascii_lowercase();
                let mut definition = Definition {
                    id: id.to_vec(),
                    ..Definition::default()
                };
                let mut max_level = 1;
                for field in item.value.items().unwrap_or_default() {
                    if field.keyword_is("parent") {
                        definition.parent = field.value.read_string().unwrap_or_default().to_vec();
                    } else if field.keyword_is("force_use_small_tech_layout") {
                        if let Some(value) = field.value.scalar().and_then(parser::read_bool) {
                            definition.small_layout = value;
                        }
                    } else if field.keyword_is("enable_equipments") {
                        definition
                            .equipment
                            .extend(field.value.items().unwrap_or_default().iter().cloned());
                    } else if field.keyword_is("enable_equipment_modules") {
                        definition
                            .modules
                            .extend(field.value.items().unwrap_or_default().iter().cloned());
                    } else if field.keyword_is("enable_subunits") {
                        definition.sub_units.push(field.clone());
                    } else if field.keyword_is("max_level") {
                        if let Some(value) = field
                            .value
                            .scalar()
                            .and_then(|value| content_reader_parser::crt::scan_i32(value).stored())
                        {
                            max_level = value;
                        }
                    } else if field.keyword_is("enable_building") {
                        let mut name = Vec::new();
                        let mut level = max_level;
                        for entry in field.value.items().unwrap_or_default() {
                            if entry.keyword_is("building") {
                                name = entry.value.read_string().unwrap_or_default().to_vec();
                            } else if entry.keyword_is("level") {
                                level = content_reader_parser::fixed5::decimal_i32(
                                    entry.value.read_string().unwrap_or_default(),
                                );
                            }
                        }
                        definition.buildings.push((name, level));
                    } else if field.keyword_is("enable_tactic") {
                        definition.tactics.push(field.clone());
                    } else if field.keyword_is("on_research_complete_limit") {
                        definition
                            .on_research_complete_limit
                            .extend(field.value.items().unwrap_or_default().iter().cloned());
                    } else if field.keyword_is("on_research_complete") {
                        let body = field.value.items().unwrap_or_default();
                        let leading = body.first().filter(|item| item.keyword_is("limit"));
                        if let Some(limit) = leading {
                            definition
                                .on_research_complete_limit
                                .extend(limit.value.items().unwrap_or_default().iter().cloned());
                        }
                        definition
                            .on_research_complete
                            .extend(body[usize::from(leading.is_some())..].iter().cloned());
                    } else if field.keyword().is_some_and(|key| {
                        registry.units.contains(&key.folded())
                            || registry.categories.contains(&key.folded())
                    }) {
                        definition.unit_bonus.push(field.clone());
                    } else if field.keyword_is("modifier") {
                        definition
                            .modifiers
                            .read_all(field.value.items().unwrap_or_default());
                    } else if field.keyword_is("custom_modifier_tooltip")
                        || field.keyword_is("hidden_modifier")
                        || field.keyword_is("added_modifier")
                        || field.key.as_deref().is_some_and(|key| {
                            modifiers::registered(key, Some(names)) == Some(true)
                        })
                    {
                        definition.modifiers.read(field);
                    }
                }
                self.by_name
                    .entry(identity)
                    .or_insert(self.definitions.len());
                self.definitions.push(definition);
            }
        }
    }

    pub fn get(&self, operand: &[u8]) -> Result<Option<&Definition>, String> {
        if let Some(why) = &self.refusal {
            return Err(why.clone());
        }
        Ok(self
            .by_name
            .get(&parser::c_string(operand).to_ascii_lowercase())
            .map(|&index| &self.definitions[index]))
    }

    pub fn template(&self, database_index: i32, id: &[u8]) -> Result<Option<&Definition>, String> {
        if let Some(why) = &self.refusal {
            return Err(why.clone());
        }
        let Some(definition) = usize::try_from(database_index)
            .ok()
            .and_then(|index| index.checked_sub(1))
            .and_then(|at| self.definitions.get(at))
        else {
            return Ok(None);
        };
        let id = parser::c_string(id);
        if !parser::c_string(&definition.id).eq_ignore_ascii_case(id) {
            return Err(format!(
                "technology {}: recorded template {database_index} is {} in the content",
                String::from_utf8_lossy(id),
                String::from_utf8_lossy(&definition.id)
            ));
        }
        Ok(Some(definition))
    }

    pub fn inherited_buildings(&self) -> Vec<Vec<(Vec<u8>, i32)>> {
        let mut lists = self
            .definitions
            .iter()
            .map(|definition| definition.buildings.clone())
            .collect::<Vec<_>>();
        for index in 0..lists.len() {
            let definition = &self.definitions[index];
            let parent = parser::c_string(&definition.parent);
            if parent.is_empty() || parent.eq_ignore_ascii_case(parser::c_string(&definition.id)) {
                continue;
            }
            if let Some(found) = self
                .definitions
                .iter()
                .position(|other| parser::c_string(&other.id).eq_ignore_ascii_case(parent))
            {
                let inherited = lists[found].clone();
                lists[index].extend(inherited);
            }
        }
        lists
    }

    pub fn category(&self, operand: &[u8]) -> Result<Option<&[u8]>, String> {
        if let Some(why) = &self.refusal {
            return Err(why.clone());
        }
        Ok(self
            .categories
            .get(&Token::c_str(operand))
            .map(Vec::as_slice))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn layered_repeated_technology_declarations_keep_every_index_and_first_name_lookup() {
        content_reader_script::test_game::enter();
        let stack = content_reader_layers::test_support::Stack::new("technology_occurrences");
        stack.write(
            &stack.base(),
            "common/technologies/vanilla.txt",
            "technologies={hidden={}} ",
        );
        stack.write(
            &stack.addon(),
            "common/technologies/a.txt",
            "technologies={repeat={parent=first} REPEAT={parent=second}}",
        );
        stack.write(
            &stack.addon(),
            "common/technologies/b.txt",
            "technologies={repeat={parent=third}}",
        );
        stack.write(
            &stack.addon(),
            "common/technologies/nested/ignored.txt",
            "technologies={ignored={}}",
        );
        let layers = stack.layers(&["common/technologies"]);
        let names = content_reader_key_names::key_name_products::KeyNameProducts::read(&layers).unwrap().names;
        let catalog = Catalog::read(&layers, &names).unwrap();
        assert!(catalog.refusal.is_none(), "{:?}", catalog.refusal);
        assert_eq!(catalog.definitions.len(), 3);
        assert_eq!(catalog.get(b"RePeAt").unwrap().unwrap().parent, b"first");
        assert_eq!(catalog.definitions[1].parent, b"second");
        assert_eq!(catalog.definitions[2].parent, b"third");
        assert!(catalog.get(b"hidden").unwrap().is_none());
        assert!(catalog.get(b"ignored").unwrap().is_none());
    }

    #[test]
    fn loading_appends_each_parents_building_entries_in_database_order() {
        let definition = |id: &str, parent: &str, buildings: &[(&str, i32)]| Definition {
            id: id.as_bytes().to_vec(),
            parent: parent.as_bytes().to_vec(),
            buildings: buildings
                .iter()
                .map(|&(name, level)| (name.as_bytes().to_vec(), level))
                .collect(),
            ..Default::default()
        };
        let catalog = Catalog {
            definitions: vec![
                definition("base", "", &[("roads", 4)]),
                definition("child", "BASE", &[("rails", 1)]),
                definition("early", "late", &[]),
                definition("late", "child", &[("ports", 2)]),
                definition("own", "OWN", &[("roads", 1)]),
                definition("orphan", "missing", &[("rails", 3)]),
            ],
            ..Default::default()
        };
        let names = |list: &Vec<(Vec<u8>, i32)>| {
            list.iter()
                .map(|(name, level)| (String::from_utf8(name.clone()).unwrap(), *level))
                .collect::<Vec<_>>()
        };
        let lists = catalog.inherited_buildings();
        let lists = lists.iter().map(names).collect::<Vec<_>>();
        let owned = |entries: &[(&str, i32)]| {
            entries
                .iter()
                .map(|&(name, level)| (name.to_owned(), level))
                .collect::<Vec<_>>()
        };
        assert_eq!(lists[0], owned(&[("roads", 4)]));
        assert_eq!(lists[1], owned(&[("rails", 1), ("roads", 4)]));
        assert_eq!(lists[2], owned(&[("ports", 2)]));
        assert_eq!(lists[3], owned(&[("ports", 2), ("rails", 1), ("roads", 4)]));
        assert_eq!(lists[4], owned(&[("roads", 1)]));
        assert_eq!(lists[5], owned(&[("rails", 3)]));
        assert_eq!(catalog.definitions[1].buildings.len(), 1);
    }

    #[test]
    fn technology_description_names_use_parent_and_layout_flag() {
        content_reader_script::test_game::enter();
        let (read, _, _) = read_templates(b"@title=mod_parent technology_categories={mod_category} technologies={mod_tech={parent=@title force_use_small_tech_layout=yes enable_equipments={mod_equipment}}}", &units::Registry::default(), &KeyNames::default());
        assert!(read.unsettled.is_none(), "{read:?}");
        let mut catalog = Catalog::default();
        catalog.append(
            &read.items,
            &units::Registry::default(),
            &KeyNames::default(),
        );
        let definition = catalog.get(b"MOD_TECH").unwrap().unwrap();
        assert_eq!(definition.parent, b"@title");
        assert!(definition.small_layout);
        assert_eq!(
            catalog.category(b"MOD_CATEGORY").unwrap(),
            Some(&b"mod_category"[..])
        );
        assert_eq!(
            definition.equipment[0].value.scalar(),
            Some(&b"mod_equipment"[..])
        );
    }

    #[test]
    fn small_layout_takes_exact_booleans_and_keeps_the_preceding_value() {
        content_reader_script::test_game::enter();
        for (assignments, expected) in [
            (
                "force_use_small_tech_layout=yes force_use_small_tech_layout=invalid",
                true,
            ),
            ("force_use_small_tech_layout=YES", false),
            (
                "force_use_small_tech_layout=yes force_use_small_tech_layout=no",
                false,
            ),
        ] {
            let text = format!("technologies={{mod_tech={{{assignments}}}}}");
            let registry = units::Registry::default();
            let names = KeyNames::default();
            let (read, _, _) = read_templates(text.as_bytes(), &registry, &names);
            assert!(read.unsettled.is_none(), "{read:?}");
            let mut catalog = Catalog::default();
            catalog.append(&read.items, &registry, &names);
            let definition = catalog.get(b"mod_tech").unwrap().unwrap();
            assert_eq!(definition.small_layout, expected, "{assignments}");
        }
    }

    #[test]
    fn technology_effect_projection_retains_read_order_and_shared_completion_limit() {
        content_reader_script::test_game::enter();
        let registry = units::Registry {
            units: BTreeSet::from([b"mod_unit".to_vec()]),
            ..Default::default()
        };
        let names = KeyNames::default();
        let (read, _, _) = read_templates(b"technologies={mod_tech={enable_building={building=mod_first} max_level=4 enable_building={building=mod_second} enable_building={building=mod_third level=bad} mod_unit={soft_attack=0.1} enable_equipment_modules={mod_module} enable_subunits={mod_unit} enable_tactic=Mod_Tactic on_research_complete_limit={always=no} on_research_complete_limit={always=yes} on_research_complete={limit={} custom_effect_tooltip=mod_tooltip} modifier={custom_modifier_tooltip=mod_modifier}}}", &registry, &names);
        assert!(read.unsettled.is_none(), "{read:?}");
        let mut catalog = Catalog::default();
        catalog.append(&read.items, &registry, &names);
        let definition = catalog.get(b"mod_tech").unwrap().unwrap();
        assert_eq!(
            definition.buildings,
            vec![
                (b"mod_first".to_vec(), 1),
                (b"mod_second".to_vec(), 4),
                (b"mod_third".to_vec(), 0)
            ]
        );
        assert!(definition.unit_bonus[0].keyword_is("mod_unit"));
        assert_eq!(
            definition.modules[0].value.scalar(),
            Some(&b"mod_module"[..])
        );
        assert_eq!(
            definition.tactics[0].value.read_string(),
            Some(&b"Mod_Tactic"[..])
        );
        assert_eq!(definition.on_research_complete_limit.len(), 2);
        assert_eq!(definition.on_research_complete.len(), 1);
        assert!(definition.on_research_complete[0].keyword_is("custom_effect_tooltip"));
        assert_eq!(definition.modifiers.tooltips(), vec!["mod_modifier"]);
    }

    #[test]
    fn technology_completion_projection_keeps_limit_when_body_guard_rejects_tail() {
        content_reader_script::test_game::enter();
        let names = KeyNames::default();
        let registry = units::Registry::default();
        let (read, _, _) = read_templates(
            b"technologies={mod_tech={on_research_complete={limit={always=yes} custom_effect_tooltip=mod_tooltip}}}",
            &registry,
            &names,
        );
        assert!(read.unsettled.is_none(), "{read:?}");
        assert!(
            read.misreads.iter().any(|note| {
                note.what.contains("on_research_complete")
                    && note.what.contains("`yes`")
                    && note.what.contains("where its effect reads a block")
                    && note.what.contains("nothing more is read")
            }),
            "{read:?}"
        );
        let mut catalog = Catalog::default();
        catalog.append(&read.items, &registry, &names);
        let definition = catalog.get(b"mod_tech").unwrap().unwrap();
        assert_eq!(definition.on_research_complete_limit.len(), 1);
        assert!(definition.on_research_complete_limit[0].keyword_is("always"));
        assert!(definition.on_research_complete.is_empty());
    }
}
