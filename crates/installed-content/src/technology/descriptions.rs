//! Presentation fields from the same native technology reader as equipment links.
use super::*;
use crate::tokens::Token;

#[derive(Clone, Debug, Default)]
pub struct Definition {
    pub id: Vec<u8>,
    pub parent: Vec<u8>,
    pub small_layout: bool,
    pub equipment: Vec<Item>,
    /// CSubUnitStatBonus at +0x150; selectors retain their content spelling.
    pub unit_bonus: Vec<Item>,
    /// CModifier at +0x188, formatted from its accumulator +0x198.
    pub modifiers: modifiers::Modifiers,
    pub modules: Vec<Item>,
    pub sub_units: Vec<Item>,
    /// Building token and level at the moment enable_building is read.
    pub buildings: Vec<(Vec<u8>, i32)>,
    pub tactics: Vec<Item>,
    pub on_research_complete: Vec<Item>,
    pub on_research_complete_limit: Vec<Item>,
}

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub definitions: BTreeMap<Token, Definition>,
    pub categories: BTreeMap<Token, Vec<u8>>,
    /// Full categories/aliases differ from KeyNames' registration-only roster.
    pub equipment: crate::equipment::Database,
    /// Demanded only by a technology that unlocks a tactic (exact-name lookup).
    pub tactics: Option<Result<crate::combat_tactics::Database, String>>,
    /// A reader form that cannot prove the effect projection. Names stay usable.
    pub effect_refusal: Option<String>,
    pub notes: Vec<String>,
    pub refusal: Option<String>,
}

impl Catalog {
    pub fn read(layers: &ResolvedLayers, names: &KeyNames) -> io::Result<Self> {
        // Reuse KeyNames' sub-unit/category/terrain rosters; do not reread them.
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
            equipment: crate::equipment::Database::load_complete(layers)?,
            ..Self::default()
        };
        // 140AC1BA0: direct directory, mode 0. 140ABCFC0/140AC3C70 are
        // shared with the equipment projection, including @variable consumption.
        for path in layers.directory_files("common/technologies", ".txt")? {
            let (read, undecided, refused) = read_templates(&layers.read(&path)?, &registry, names);
            catalog
                .notes
                .extend(read.misreads.iter().map(|note| format!("{path}: {note}")));
            if let Some(why) = &read.unsettled {
                catalog.refusal = Some(format!("{path}: technology description unread: {why}"));
            }
            if let Some(why) = undecided
                .first()
                .map(ToString::to_string)
                // Folder presentation is outside the 0,1,1 callback body.
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
            .values()
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
                // 140ABCFC0 -> 140ABB190: anonymous category names; lookup
                // 140AC12B0 returns the first valid object with the token.
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
                let identity = Token::c_str(id);
                if self.definitions.contains_key(&identity) {
                    continue;
                }
                let mut definition = Definition {
                    id: id.to_vec(),
                    ..Definition::default()
                };
                let mut max_level = 1; // 140ABBF20 initializes +0x3DC to one.
                // +0x3B0 parent, +0x40A force_use_small_tech_layout; constructor
                // 140ABBF20 clears the flag. 140AC3C70 keeps the equipment list.
                for field in item.value.items().unwrap_or_default() {
                    if field.keyword_is("parent") {
                        definition.parent = field.value.read_string().unwrap_or_default().to_vec();
                    } else if field.keyword_is("force_use_small_tech_layout") {
                        definition.small_layout = field.value_keyword().as_deref() == Some("yes");
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
                            .and_then(|value| crate::crt::scan_i32(value).stored())
                        {
                            max_level = value;
                        }
                    } else if field.keyword_is("enable_building") {
                        // 140AC3C70 token32FD defaults the level to the current
                        // max_level; building/level replace their value slots.
                        let mut name = Vec::new();
                        let mut level = max_level;
                        for entry in field.value.items().unwrap_or_default() {
                            if entry.keyword_is("building") {
                                name = entry.value.read_string().unwrap_or_default().to_vec();
                            } else if entry.keyword_is("level") {
                                // Inline ParseTokenInt32 1424B0080 assigns its
                                // result (zero on failure), unlike max_level.
                                level = crate::fixed5::decimal_i32(
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
                            // Both forms load the same CAndTrigger +0x470;
                            // CTrigger::LoadBody 1405431C0 appends child keys.
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
                // 140ABCFC0 allocates every repeated definition afresh;
                // 140AB9020/140AC1A80 return the first registered token.
                self.definitions.insert(identity, definition);
            }
        }
    }

    pub fn get(&self, operand: &[u8]) -> Result<Option<&Definition>, String> {
        if let Some(why) = &self.refusal {
            return Err(why.clone());
        }
        Ok(self.definitions.get(&Token::c_str(operand)))
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
    fn technology_description_names_use_native_parent_and_layout_flag() {
        let (read, _, _) = read_templates(b"@title=mod_parent technology_categories={mod_category} technologies={mod_tech={parent=@title force_use_small_tech_layout=yes enable_equipments={mod_equipment}}}", &units::Registry::default(), &KeyNames::default());
        assert!(read.unsettled.is_none(), "{read:?}");
        let mut catalog = Catalog::default();
        catalog.append(
            &read.items,
            &units::Registry::default(),
            &KeyNames::default(),
        );
        let definition = catalog.get(b"MOD_TECH").unwrap().unwrap();
        // parent uses the current-token reader, not the resolved value slot.
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
    fn technology_effect_projection_retains_read_order_and_shared_completion_limit() {
        let registry = units::Registry {
            units: BTreeSet::from([b"mod_unit".to_vec()]),
            ..Default::default()
        };
        let names = KeyNames::default();
        // Both external limits append to +0x470. An empty inline limit
        // leaves the value slot at `{`, so ReadScript loads the effect body.
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
        // 140AC3C70 loads the inline trigger before calling ReadScript
        // (140534280). Its last scalar leaves `yes`, so no effect body loads.
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
                    && note.what.contains("140534280")
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
