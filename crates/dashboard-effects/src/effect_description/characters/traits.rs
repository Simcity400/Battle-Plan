
use super::*;
use content_reader_script::{modifiers::{EquipmentBonus, Modifiers}, sub_units::SubUnit};
use content_reader_key_names::unit_leader::{Adjuster, Leaders, TraitEffects};
use replay_data::rows::{ModifierDefinitionRow, SimulationFeatureRow, UnitLeaderTraitRow};
use std::{borrow::Cow, collections::BTreeSet};

const DEFINITION_TOKEN: u64 = mask::<ModifierDefinitionRow>(&["token"]);
const DEFINITION_GATE: u64 =
    mask::<ModifierDefinitionRow>(&["token", "feature_selector", "eligibility"]);
const FEATURES: u64 = mask::<SimulationFeatureRow>(&["dlc_mask"]);
const TRAIT_ORDINAL: u64 = mask::<UnitLeaderTraitRow>(&["definition_ordinal"]);

#[cfg(test)]
mod tests;

fn newline(text: &mut String) {
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
}

fn operand<'a>(item: &'a Item, name: &str) -> Option<&'a [u8]> {
    children(item)
        .iter()
        .rev()
        .find(|field| field.keyword_is(name))
        .and_then(|field| field.value.read_string())
}

fn trait_operand(item: &Item) -> &[u8] {
    item.value
        .read_string()
        .or_else(|| operand(item, "trait"))
        .unwrap_or_default()
}

impl Renderer<'_, '_, '_> {
    pub(in crate::effect_description) fn char_current_country_key(
        &self,
        key: &[u8],
        scopes: Scopes,
    ) -> Result<String, String> {
        if self.context.perspective > 0 {
            let tag = self.tag(self.context.perspective)?;
            let candidate = format!("{tag}_{}", String::from_utf8_lossy(key));
            if self.context.localisation.get(&candidate).is_some() {
                return self.localised(candidate.as_bytes(), &[], scopes, false);
            }
        }
        self.localised(key, &[], scopes, false)
    }

    pub(in crate::effect_description) fn char_unit_catalogue(
        &self,
    ) -> Result<Cow<'_, Leaders<String>>, String> {
        let content = self.character_content()?;
        let leaders = content.unit_leaders.as_ref().map_err(Clone::clone)?;
        if !leaders.conditional() {
            return Ok(Cow::Borrowed(leaders));
        }
        let definition_tokens: BTreeSet<_> =
            each(self.character_table::<tables::ModifierDefinitions>(DEFINITION_TOKEN)?)
                .map(|definition| definition.map(|definition| definition.token))
                .collect::<Result<_, _>>()?;
        let world = self.context.world()?;
        let refusal = RefCell::new(None);
        let terrain = self.char_terrain()?;
        let leaders = leaders.reread(
            &|name| {
                let category = terrain
                    .categories
                    .iter()
                    .find(|category| category.name.eq_ignore_ascii_case(name.as_bytes()))?;
                String::from_utf8(category.name.clone()).ok()
            },
            &|name| match world.token_of(name) {
                Ok(token) => Some(token.is_some_and(|token| definition_tokens.contains(&token))),
                Err(why) => {
                    refusal.borrow_mut().get_or_insert(why);
                    None
                }
            },
        );
        if let Some(why) = refusal.into_inner() {
            return Err(why.to_string());
        }
        Ok(Cow::Owned(leaders))
    }

    fn char_trait_modifier_definition(
        &self,
        name: &str,
    ) -> Result<Cow<'_, ModifierDefinitionRow>, String> {
        let definitions = self.character_table::<tables::ModifierDefinitions>(DEFINITION_GATE)?;
        let token = self
            .context
            .world()?
            .token_of(name.as_bytes())
            .map_err(|why| why.to_string())?
            .ok_or_else(|| format!("trait modifier {name} has no recorded definition"))?;
        find(definitions, |definition| definition.token == token)?
            .ok_or_else(|| format!("trait modifier {name} has no recorded definition"))
    }

    fn char_trait_modifier_eligible(
        &self,
        definition: &replay_data::rows::ModifierDefinitionRow,
    ) -> Result<bool, String> {
        if definition.feature_selector == 0 {
            return Ok(true);
        }
        let features = self.character_table::<tables::SimulationFeatures>(FEATURES)?;
        let (1, Some(features)) = (features.len(), each(features).next().transpose()?) else {
            return Err("simulation features has no unique feature mask".into());
        };
        Ok(features.dlc_mask & definition.feature_selector != 0)
    }

    pub(in crate::effect_description) fn char_trait_modifier_kept(
        &self,
        source: &Modifiers,
    ) -> Result<bool, String> {
        if let Some(why) = source.unheld.cause() {
            return Err(why.to_owned());
        }
        if !source.tooltips().is_empty() || !source.children.is_empty() {
            return Ok(true);
        }
        for modifier in &source.values {
            if self.char_trait_modifier_eligible(
                &*self.char_trait_modifier_definition(&modifier.name)?,
            )? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(super) fn char_trait_modifier_name_eligible(&self, name: &str) -> Result<bool, String> {
        let definition = self.char_trait_modifier_definition(name)?;
        if definition.feature_selector != 0 && definition.eligibility == 0 {
            return Ok(false);
        }
        self.char_trait_modifier_eligible(&definition)
    }

    fn char_trait_modifiers(
        &self,
        source: &Modifiers,
        prefix: &str,
        separator: &str,
    ) -> Result<String, String> {
        if source == &Modifiers::default() {
            return Ok(String::new());
        }
        self.modifier_description_with(
            source,
            prefix,
            ModifierListOptions {
                category_mask: u32::MAX,
                separator,
                scale: 100_000,
                ..Default::default()
            },
        )
    }

    pub(in crate::effect_description) fn char_equipment_bonuses(
        &self,
        bonuses: &[EquipmentBonus],
        prefix: &str,
        scopes: Scopes,
    ) -> Result<String, String> {
        let mut text = String::new();
        let stripped = prefix.replace('\n', "");
        for bonus in bonuses {
            text.push_str(prefix);
            text.push_str("\u{11}Y");
            text.push_str(&self.localised(bonus.equipment.as_bytes(), &[], scopes, false)?);
            text.push_str("\u{11}!: \n");
            let mut first = true;
            for value in &bonus.values {
                let Some(index) = content_reader_script::equipment_stats::stat_index(value.name.as_bytes())
                else {
                    continue;
                };
                if !first {
                    text.push('\n');
                }
                first = false;
                text.push_str(&" ".repeat(stripped.len() + 4));
                text.push_str(&self.char_equipment_stat_difference(index, value.value, scopes)?);
            }
        }
        Ok(text)
    }

    pub(in crate::effect_description) fn char_equipment_stat_difference(
        &self,
        index: usize,
        value: i64,
        scopes: Scopes,
    ) -> Result<String, String> {
        let (_, name, positive) = content_reader_script::equipment_stats::stat_metadata()[index];
        let mut text = self.char_native_text(name.as_bytes(), &[], scopes, false)?;
        let candidate = format!("{name}_MOD_DIFF");
        let key: &[u8] = if self.context.localisation.get(&candidate).is_some() {
            candidate.as_bytes()
        } else if positive {
            b"STAT_MODIFIER_DIFF"
        } else {
            b"STAT_MODIFIER_DIFF_NEG"
        };
        text.push_str(&self.char_native_text(
            key,
            &[("VALUE", Argument::Fixed5(value))],
            scopes,
            false,
        )?);
        Ok(text)
    }

    fn char_country_trait_body(
        &self,
        trait_: &content_reader_military::leader_traits::LeaderTrait,
        prefix: &str,
        scopes: Scopes,
    ) -> Result<String, String> {
        let mut text = self.char_trait_modifiers(&trait_.modifiers, "", prefix)?;
        newline(&mut text);
        let empty = Scopes::empty();
        let mut targets = String::new();
        for target in &trait_.targeted {
            let tag = target.tag.as_deref().ok_or_else(|| format!(
                "country trait {} targeted-country operand is not retained by the shared catalogue", trait_.token,
            ))?;
            let country = self
                .evaluator()?
                .scoped_country(tag.as_bytes(), empty)
                .map_err(|why| why.to_string())?
                .filter(|country| *country > 0);
            let context = if let Some(country) = country {
                let name = content_reader_localisation::localisation::crt_case_text(
                    &self.country_name(country, true)?,
                    true,
                    true,
                );
                self.char_native_text(
                    b"EFFECT_CONTEXT_COUNTRY",
                    &[
                        ("TAG", Argument::Text(self.tag(country)?)),
                        ("COUNTRY", Argument::Text(&name)),
                    ],
                    empty,
                    false,
                )?
            } else {
                String::new()
            };
            let body = self.modifier_description_with(
                &target.modifiers,
                &context,
                ModifierListOptions {
                    separator: prefix,
                    ..Default::default()
                },
            )?;
            if !body.is_empty() {
                if !targets.is_empty() {
                    targets.push_str(prefix);
                }
                targets.push_str(&body);
            }
        }
        text.push_str(&targets);
        newline(&mut text);
        let equipment_prefix = prefix.replace('\n', "");
        let mut equipment =
            self.char_equipment_bonuses(&trait_.equipment_bonuses, &equipment_prefix, scopes)?;
        if text.is_empty() && !equipment.is_empty() {
            equipment.drain(..equipment_prefix.len());
        }
        text.push_str(&equipment);
        if text == "\n" {
            text.clear();
        }
        Ok(text)
    }

    pub(in crate::effect_description) fn char_country_trait_description(
        &self,
        name: &[u8],
        scopes: Scopes,
    ) -> Result<Option<String>, String> {
        self.char_country_trait_description_with_prefix(name, scopes, "\n  - ")
    }

    pub(in crate::effect_description) fn char_country_trait_description_with_prefix(
        &self,
        name: &[u8],
        scopes: Scopes,
        prefix: &str,
    ) -> Result<Option<String>, String> {
        let name = std::str::from_utf8(name).map_err(|_| "country trait name is not UTF-8")?;
        let traits = self.char_country_traits()?;
        let Some(trait_) = traits.get(name).map_err(|why| why.to_string())? else {
            return Ok(None);
        };
        self.char_country_trait_body(trait_, prefix, scopes)
            .map(Some)
    }

    pub(in crate::effect_description) fn char_adjuster_lines(
        &self,
        adjuster: &Adjuster,
        scopes: Scopes,
    ) -> Result<String, String> {
        let mut text = String::new();
        for (stat, value) in [
            ("MOVEMENT", adjuster.movement),
            ("ATTACK", adjuster.attack),
            ("DEFENCE", adjuster.defence),
        ] {
            if value == 0 {
                continue;
            }
            let key = format!("STAT_ADJUSTER_{stat}");
            text.push_str(&self.char_native_text(key.as_bytes(), &[], scopes, false)?);
            text.push_str(&self.char_native_text(
                format!("{key}_DIFF").as_bytes(),
                &[("VALUE", Argument::Fixed5(value))],
                scopes,
                false,
            )?);
            text.push('\n');
        }
        Ok(text)
    }

    fn char_unit_trait_body(
        &self,
        definition: &TraitEffects,
        scopes: Scopes,
    ) -> Result<String, String> {
        if let Some(why) = definition.unsupported.first() {
            return Err(why.clone());
        }
        if !definition.conditions.is_empty() {
            return Err("unit trait modifier registry conditions remain unresolved".into());
        }
        let mut text = String::new();
        if !definition.override_tooltips.is_empty() {
            for key in &definition.override_tooltips {
                text.push_str(&self.line(key.as_bytes(), &[], scopes, false)?);
            }
            return Ok(text);
        }
        for (index, block) in definition.blocks.iter().enumerate() {
            text.push_str(&self.char_trait_modifiers(block, "", "\n")?);
            if index < 3 {
                newline(&mut text);
            }
        }
        for adjuster in &definition.adjusters {
            newline(&mut text);
            if adjuster.movement == 0 && adjuster.attack == 0 && adjuster.defence == 0 {
                continue;
            }
            text.push_str("\u{11}H");
            text.push_str(&self.localised(adjuster.key.as_bytes(), &[], scopes, false)?);
            text.push_str("\u{11}!\n");
            text.push_str(&self.char_adjuster_lines(adjuster, scopes)?);
        }
        if !definition.sub_units.is_empty() {
            let database = self
                .context
                .text
                .content()
                .effects()?
                .key_names
                .as_ref()
                .ok_or_else(|| self.content_gap("effect key names"))?
                .sub_units
                .as_ref()
                .ok_or("shared effect key names lack the sub-unit catalogue")?;
            let mut entries = BTreeMap::new();
            for entry in &definition.sub_units {
                let index = match database.get(&entry.adjuster.key) {
                    SubUnit::Index(index) => index,
                    SubUnit::Unknown => continue,
                    SubUnit::Uncertain(why) => return Err(why.to_owned()),
                };
                if entries.contains_key(&index) {
                    continue;
                }
                let adjuster = &entry.adjuster;
                if adjuster.attack != 0
                    || adjuster.defence != 0
                    || adjuster.movement != 0
                    || self.char_trait_modifier_kept(&entry.block)?
                {
                    entries.insert(index, entry);
                }
            }
            let mut sub_units = String::new();
            for (index, entry) in entries {
                let block = self.char_trait_modifiers(&entry.block, "    ", "\n")?;
                let lines = self.char_adjuster_lines(&entry.adjuster, scopes)?;
                newline(&mut sub_units);
                let name = database
                    .name(index)
                    .ok_or("sub-unit database index has no name")??;
                sub_units.push_str("\u{11}H");
                sub_units.push_str(&self.char_current_country_key(name.as_bytes(), scopes)?);
                sub_units.push_str("\u{11}!\n");
                sub_units.push_str(&lines);
                sub_units.push_str(&block);
            }
            text.push_str(&sub_units);
        }
        let [
            attack,
            defense,
            logistics,
            planning,
            maneuvering,
            coordination,
        ] = definition.skills;
        if attack != 0
            && attack == defense
            && ((attack == logistics && attack == planning)
                || (attack == maneuvering && attack == coordination))
        {
            newline(&mut text);
            text.push_str(&self.char_native_text(
                b"trait_bonus_all",
                &[("VAL", Argument::Int32(attack))],
                scopes,
                false,
            )?);
        } else {
            for (key, value) in [
                (b"trait_bonus_attack".as_slice(), attack),
                (b"trait_bonus_defense", defense),
                (b"trait_bonus_planning", planning),
                (b"trait_bonus_logistics", logistics),
                (b"trait_bonus_maneuvering", maneuvering),
                (b"trait_bonus_coordination", coordination),
            ] {
                if value == 0 {
                    continue;
                }
                newline(&mut text);
                text.push_str(&self.char_native_text(
                    key,
                    &[("VAL", Argument::Int32(value))],
                    scopes,
                    false,
                )?);
            }
        }
        if definition.proximity_offset != 0 {
            let defines = self.char_defines()?;
            let integer = |name| {
                defines
                    .int32("NMilitary", name)
                    .map_err(|why| why.to_string())
            };
            let distance = definition
                .proximity_offset
                .wrapping_add(integer("GENERAL_PROXIMITY_DEFAULT")?);
            let key: &[u8] = if distance < 1 {
                b"LEADER_PROXIMITY_FRONTLINE_TITLE".as_slice()
            } else if distance <= integer("GENERAL_PROXIMITY_CLOSE")? {
                b"LEADER_PROXIMITY_CLOSE_TITLE"
            } else if distance <= integer("GENERAL_PROXIMITY_MEDIUM")? {
                b"LEADER_PROXIMITY_MEDIUM_TITLE"
            } else {
                b"LEADER_PROXIMITY_FAR_TITLE"
            };
            let name = self.char_native_text(key, &[], scopes, false)?;
            newline(&mut text);
            text.push_str(&self.char_native_text(
                b"trait_bonus_leader_default_proximity_offset",
                &[("NAME", Argument::Text(&name))],
                scopes,
                false,
            )?);
        }
        for (key, value) in &definition.experience_factors {
            let name = self.char_current_country_key(key.as_bytes(), scopes)?;
            newline(&mut text);
            text.push_str(&self.char_native_text(
                b"trait_xp_factor",
                &[
                    ("NAME", Argument::Text(&name)),
                    ("VAL", Argument::Fixed5(*value)),
                ],
                scopes,
                false,
            )?);
        }
        newline(&mut text);
        Ok(text)
    }

    pub(in crate::effect_description) fn char_unit_trait_description(
        &self,
        name: &[u8],
        scopes: Scopes,
    ) -> Result<Option<String>, String> {
        let name = std::str::from_utf8(name).map_err(|_| "unit trait name is not UTF-8")?;
        let catalogue = self.char_unit_catalogue()?;
        let Some(definition) = catalogue.traits.get(name) else {
            return Ok(None);
        };
        self.char_unit_trait_body(definition, scopes).map(Some)
    }

    pub(in crate::effect_description) fn char_trait(
        &self,
        item: &Item,
        scopes: Scopes,
        remove: bool,
    ) -> Result<String, String> {
        let Some(character) = self.effect_character_target(operand(item, "character"), scopes)?
        else {
            return Ok(String::new());
        };
        let token = trait_operand(item);
        let name = std::str::from_utf8(token).map_err(|_| "trait operand is not UTF-8")?;
        let units = self.char_unit_catalogue()?;
        let unit = units.traits.get(name);
        let countries = self.char_country_traits()?;
        let country = countries.get(name).map_err(|why| why.to_string())?;
        if unit.is_none() && country.is_none() {
            return Ok(String::new());
        }
        let title = if let Some(unit) = unit {
            self.char_current_country_key(unit.name.as_bytes(), scopes)?
        } else {
            self.localised(country.unwrap().token.as_bytes(), &[], scopes, false)?
        };
        let mut body = String::new();
        if remove {
            let source = match unit {
                Some(unit) => &unit.blocks[0],
                None => &country.unwrap().modifiers,
            };
            body = self.char_trait_modifiers(source, "", "\n  -")?;
        } else {
            if let Some(country) = country {
                body = self.char_trait_modifiers(&country.modifiers, "", "\n  -")?;
            }
        }
        if let Some(country) = country
            && !country.equipment_bonuses.is_empty()
        {
            newline(&mut body);
            body.push_str(&self.char_equipment_bonuses(
                &country.equipment_bonuses,
                "  -",
                scopes,
            )?);
        }
        if !remove && let Some(unit) = unit {
            newline(&mut body);
            body.push_str(
                &self
                    .char_unit_trait_description(unit.name.as_bytes(), scopes)?
                    .unwrap_or_default(),
            );
        }
        let modifier = if body.is_empty() {
            String::new()
        } else {
            format!("  -{body}\n")
        };
        if remove || self.effect_character(scopes.this)?.is_none() {
            let name = self.effect_character_name(&character)?;
            self.char_native_text(
                if remove {
                    b"EFFECT_REMOVE_TRAIT"
                } else {
                    b"EFFECT_ADD_TRAIT"
                },
                &[
                    ("NAME", Argument::Text(&name)),
                    ("TRAITNAME", Argument::Text(&title)),
                    ("TRAITMODIFIER", Argument::Text(&modifier)),
                ],
                scopes,
                false,
            )
        } else {
            self.char_native_text(
                b"EFFECT_ADD_UNIT_LEADER_TRAIT",
                &[
                    ("TRAITNAME", Argument::Text(&title)),
                    ("TRAITMODIFIER", Argument::Text(&modifier)),
                ],
                scopes,
                false,
            )
        }
    }

    pub(in crate::effect_description) fn char_unit_trait(
        &self,
        item: &Item,
        scopes: Scopes,
        remove: bool,
        timed: bool,
    ) -> Result<String, String> {
        let token = if timed {
            operand(item, "trait").unwrap_or_default()
        } else {
            item.value.scalar().unwrap_or_default()
        };
        let name = std::str::from_utf8(token).map_err(|_| "unit trait operand is not UTF-8")?;
        let database = self.char_unit_catalogue()?;
        let Some(definition) = database.traits.get(name) else {
            return Ok(String::new());
        };
        let name = self.char_current_country_key(definition.name.as_bytes(), scopes)?;
        let body = self.char_trait_modifiers(&definition.blocks[0], "", "\n      - ")?;
        let body = if body.is_empty() {
            body
        } else {
            format!("      - {body}\n")
        };
        let mut arguments = vec![
            ("TRAITNAME", Argument::Text(&name)),
            ("TRAITMODIFIER", Argument::Text(&body)),
        ];
        if timed {
            let days = operand(item, "days")
                .map(content_reader_parser::fixed5::decimal_i32)
                .unwrap_or(0);
            arguments.push(("NUMDAYS", Argument::Int32(days)));
        }
        self.char_native_text(
            if timed {
                b"EFFECT_ADD_UNIT_LEADER_TRAIT_WITH_DURATION"
            } else if remove {
                b"EFFECT_REMOVE_UNIT_LEADER_TRAIT"
            } else {
                b"EFFECT_ADD_UNIT_LEADER_TRAIT"
            },
            &arguments,
            scopes,
            false,
        )
    }

    pub(in crate::effect_description) fn char_country_trait(
        &self,
        item: &Item,
        scopes: Scopes,
        remove: bool,
    ) -> Result<String, String> {
        let mut character = None;
        if let Some(current) = self.effect_character(scopes.this)?
            && self
                .char_country_leader_scope(&current, operand(item, "ideology"))?
                .is_some()
        {
            character = Some(current);
        }
        if character.is_none()
            && let Scope::Country(country) | Scope::CountryLeader { country, .. } = scopes.this
            && country > 0
        {
            character = self.char_ruling_leader(country)?;
        }
        let Some(character) = character else {
            return Ok(String::new());
        };
        let source = item
            .value
            .scalar()
            .map(|_| item)
            .or_else(|| field(children(item), "trait"));
        if source.is_none_or(|source| source.value_keyword().is_none()) {
            return Ok(String::new());
        }
        let name = trait_operand(item);
        let traits = self.char_country_traits()?;
        let Some(trait_) = traits
            .by_token(&content_reader_parser::tokens::Token::c_str(name))
            .map_err(|why| why.to_string())?
        else {
            return Ok(String::new());
        };
        let title = self.localised(trait_.token.as_bytes(), &[], scopes, false)?;
        let mut trait_text = format!("\n    \u{11}H{title}\u{11}!\n");
        trait_text.push_str("  - ");
        trait_text.push_str(
            &self
                .char_country_trait_description(trait_.token.as_bytes(), scopes)?
                .unwrap_or_default(),
        );
        trait_text.push('\n');
        let leader = self.char_country_leader_display_name(&character, scopes)?;
        self.char_native_text(
            if remove {
                b"EFFECT_REMOVE_COUNTRY_LEADER_TRAIT"
            } else {
                b"EFFECT_ADD_COUNTRY_LEADER_TRAIT"
            },
            &[
                ("LEADER", Argument::Text(&leader)),
                ("TRAIT", Argument::Text(&trait_text)),
            ],
            scopes,
            true,
        )
    }

    pub(in crate::effect_description) fn char_swap_traits(
        &self,
        item: &Item,
        scopes: Scopes,
        character_only: bool,
    ) -> Result<String, String> {
        let traits = self.char_country_traits()?;
        let lookup = |key| -> Result<_, String> {
            let Some(source) =
                field(children(item), key).filter(|source| source.value_keyword().is_some())
            else {
                return Ok(None);
            };
            traits
                .by_token(&content_reader_parser::tokens::Token::c_str(
                    source.value.scalar().unwrap_or_default(),
                ))
                .map_err(|why| why.to_string())
        };
        let (Some(old), Some(new)) = (lookup("remove")?, lookup("add")?) else {
            return Ok(if character_only {
                String::new()
            } else {
                "ERROR: TRAITS ARE MISSING\n".into()
            });
        };
        let character = if character_only {
            let Some(character) = self.effect_character(scopes.this)? else {
                return Ok(String::new());
            };
            self.char_country_leader_scope(&character, operand(item, "ideology"))?
                .map(|_| character)
        } else {
            let country = self.current_country(scopes)?;
            self.char_ruling_leader(country)?
        };
        let Some(character) = character else {
            return Ok(String::new());
        };
        let leader = self.char_country_leader_display_name(&character, scopes)?;
        let remove = self.localised(old.token.as_bytes(), &[], scopes, false)?;
        let add = self.localised(new.token.as_bytes(), &[], scopes, false)?;
        let arguments = [
            ("LEADER", Argument::Text(&leader)),
            ("REMOVE", Argument::Text(&remove)),
            ("ADD", Argument::Text(&add)),
        ];
        let mut text = self.char_native_text(
            if old.token == new.token {
                b"EFFECT_SWAP_RULER_TRAITS_MODIFY"
            } else {
                b"EFFECT_SWAP_RULER_TRAITS_REPLACE"
            },
            &arguments,
            scopes,
            !character_only,
        )?;
        text.push('\n');
        let mut delta = Modifiers::default();
        for (source, sign) in [(&new.modifiers, 1_i64), (&old.modifiers, -1)] {
            delta.unheld.absorb(&source.unheld);
            for value in &source.values {
                if let Some(existing) = delta
                    .values
                    .iter_mut()
                    .find(|existing| existing.name == value.name)
                {
                    existing.value = existing.value.wrapping_add(value.value.wrapping_mul(sign));
                } else {
                    let mut value = value.clone();
                    value.value = value.value.wrapping_mul(sign);
                    delta.values.push(value);
                }
            }
        }
        text.push_str(&self.char_trait_modifiers(&delta, "", "\n")?);
        text.push('\n');
        Ok(text)
    }

    pub(in crate::effect_description) fn char_random_trait(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let values = children(item);
        if values.is_empty() {
            return Ok(String::new());
        }
        let Some(leader) = self.effect_unit_leader(scopes)? else {
            return Ok(String::new());
        };
        let database = self.char_unit_catalogue()?;
        let held: Vec<u32> = match &leader {
            UnitLeader::Referenced(row) => row.trait_definitions.clone(),
            UnitLeader::Direct(row) => {
                let low = (u128::from(row.id) << 32 | u128::from(row.reference_type)) << 32;
                each(
                    self.character_table::<tables::UnitLeaderTraits>(TRAIT_ORDINAL)?
                        .span(low, low | u128::from(u32::MAX)),
                )
                .map(|trait_| trait_.map(|trait_| trait_.definition_ordinal as u32))
                .collect::<Result<_, _>>()?
            }
        };
        let mut candidates = Vec::new();
        for item in values {
            let name = item.value.read_str().map_err(|why| why.to_string())?;
            let ordinal = database.traits.first_ordinal(name.as_bytes())?.unwrap_or(0) as u32;
            if held.contains(&ordinal) {
                continue;
            }
            candidates.push(name);
        }
        if candidates.is_empty() {
            return Ok(String::new());
        }
        let graph = self.graph.borrow();
        let scope = &graph.scopes[scopes.cursor.ok_or("random trait scope has no cursor")?];
        let index = scope.random_draw()? as usize % candidates.len();
        let Some(definition) = database.traits.get(candidates[index]) else {
            return Ok(String::new());
        };
        let title = self.char_current_country_key(definition.name.as_bytes(), scopes)?;
        let body = self.char_trait_modifiers(&definition.blocks[0], "", "\n      - ")?;
        let body = if body.is_empty() {
            body
        } else {
            format!("      - {body}\n")
        };
        self.char_native_text(
            b"EFFECT_ADD_UNIT_LEADER_TRAIT",
            &[
                ("TRAITNAME", Argument::Text(&title)),
                ("TRAITMODIFIER", Argument::Text(&body)),
            ],
            scopes,
            false,
        )
    }

    pub(in crate::effect_description) fn char_max_trait(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let value = self.scoped_value(item.value.scalar().unwrap_or_default(), scopes)?;
        let rounded = value.wrapping_add(if value < 0 { -50_000 } else { 50_000 }) / 100_000;
        let mut text = self.char_native_text(
            b"EFFECT_ADD_MAX_TRAIT",
            &[("NUM", Argument::Int32(rounded as i32))],
            scopes,
            false,
        )?;
        text.push('\n');
        Ok(text)
    }
}
