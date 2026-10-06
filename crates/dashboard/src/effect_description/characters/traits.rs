//! Native character trait descriptions; modifier values use the core formatter.

use super::*;
use installed_content::{
    modifiers::{EquipmentBonus, Modifiers},
    unit_leader::{Adjuster, Leaders, SubUnit, TraitEffects},
};
use std::{borrow::Cow, collections::BTreeSet};

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
    // 140BBA630 -> 140BBA780: localise for the selected viewing country;
    // only a defined TAG_key replaces the literal catalogue key.
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
        let definition_tokens: BTreeSet<_> = self
            .character_table::<tables::ModifierDefinitions>()?
            .iter()
            .map(|definition| definition.token)
            .collect();
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
            // 1406042A0 finds the lexer's token before the modifier definition.
            &|name| match world.token_of(name) {
                Ok(token) => Some(token.is_some_and(|token| definition_tokens.contains(&token))),
                Err(why) => {
                    refusal.borrow_mut().get_or_insert(why);
                    None
                }
            },
        );
        if let Some(why) = refusal.into_inner() {
            return Err(why);
        }
        Ok(Cow::Owned(leaders))
    }

    // 1406042A0 -> 1424A7060/14022EC40: folded lexer lookup, then definition.
    fn char_trait_modifier_definition(
        &self,
        name: &str,
    ) -> Result<&replay::rows::ModifierDefinitionRow, String> {
        let definitions = self.character_table::<tables::ModifierDefinitions>()?;
        let token = self
            .context
            .world()?
            .token_of(name.as_bytes())?
            .ok_or_else(|| format!("trait modifier {name} has no recorded definition"))?;
        definitions
            .iter()
            .find(|definition| definition.token == token)
            .ok_or_else(|| format!("trait modifier {name} has no recorded definition"))
    }

    // IsEligible 140552BC0; both the getter and FormatLine test the feature.
    fn char_trait_modifier_eligible(
        &self,
        definition: &replay::rows::ModifierDefinitionRow,
    ) -> Result<bool, String> {
        if definition.feature_selector == 0 {
            return Ok(true);
        }
        let [features] = self.character_table::<tables::SimulationFeatures>()? else {
            return Err("SimulationFeatures has no unique feature mask".into());
        };
        Ok(features.dlc_mask & definition.feature_selector != 0)
    }

    // 1410079E0 keeps custom tooltip/child blocks or any eligible entry,
    // even a hidden/zero entry that FormatLine will not show.
    fn char_trait_modifier_kept(&self, source: &Modifiers) -> Result<bool, String> {
        if let Some(why) = source.unheld.cause() {
            return Err(why.to_owned());
        }
        if !source.tooltips().is_empty() || !source.children.is_empty() {
            return Ok(true);
        }
        for modifier in &source.values {
            if self.char_trait_modifier_eligible(
                self.char_trait_modifier_definition(&modifier.name)?,
            )? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    // Plain FormatDescription 14054E050 differs from the core's scoped
    // default. Until the core accepts plain/mask/separator/show-zero options,
    // refuse that call rather than reproducing its formatting rules here.
    // A wholly empty block needs no FormatLine or custom-tooltip call.
    pub(in crate::effect_description) fn char_trait_plain_modifiers(
        &self,
        source: &Modifiers,
    ) -> Result<(), String> {
        if let Some(why) = source.unheld.cause() {
            return Err(why.to_owned());
        }
        if source == &Modifiers::default() {
            return Ok(());
        }
        Err("native plain modifier description requires core plain/mask/separator/show-zero parameters (14054E050/14054F6D0)".into())
    }

    // FormatScopedModifierDescription 14054E520 expands the description.
    // Consume the shared formatter unchanged for its default caller options.
    pub(in crate::effect_description) fn char_scoped_modifiers(
        &self,
        source: &Modifiers,
        prefix: &str,
        scopes: Scopes,
        include: u32,
    ) -> Result<String, String> {
        if include != u32::MAX {
            return Err("scoped modifier description requires core category-mask parameter (14054E520)".into());
        }
        self.modifier_list(source, prefix, scopes)
    }

    fn char_trait_modifiers(&self, source: &Modifiers) -> Result<String, String> {
        self.char_trait_plain_modifiers(source)?;
        // Use modifier_list again when the core's native plain entry point lands.
        Ok(String::new())
    }

    // 1407118A0 -> 140711C80: authored equipment name, yellow controls,
    // ': \n'@1427D4220; stat rows are the native stored order, not sorted.
    fn char_equipment_bonuses(
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
                let Some(index) = installed_content::equipment::stat_index(value.name.as_bytes()) else {
                    continue; // 140625D00 reports and drops unknown stats.
                };
                if !first {
                    text.push('\n');
                }
                first = false;
                text.push_str(&" ".repeat(stripped.len() + 4));
                let (_, name, positive) = installed_content::equipment::STAT_METADATA[index];
                text.push_str(&self.char_native_text(name.as_bytes(), &[], scopes, false)?);
                // 1413D0370 uses a defined per-stat _MOD_DIFF, else the
                // native sign-polarity fallback (14309B650).
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
                    &[("VALUE", Argument::Fixed5(value.value))],
                    scopes,
                    false,
                )?);
            }
        }
        Ok(text)
    }

    // 140712850 resolves each targeted country in a fresh empty event scope.
    fn char_country_trait_body(
        &self,
        trait_: &installed_content::leader_traits::LeaderTrait,
        prefix: &str,
        scopes: Scopes,
        targeted: bool,
    ) -> Result<String, String> {
        let mut text = self.char_trait_modifiers(&trait_.modifiers)?;
        if targeted && !trait_.targeted.is_empty() {
            // 140712850 passes an empty scope, 24-bit mask and the caller's
            // separator to plain FormatDescription. Core parameters pending.
            return Err("native targeted trait modifiers require core category-mask/plain/separator/show-zero parameters (140712850/14054E050)".into());
        }
        newline(&mut text);
        let equipment_prefix = prefix.replace('\n', "");
        let mut equipment =
            self.char_equipment_bonuses(&trait_.equipment_bonuses, &equipment_prefix, scopes)?;
        if text.is_empty() && !equipment.is_empty() {
            // 140712180 removes only the first prefix's indentation when
            // equipment is first (prefix length minus its LF count).
            equipment.drain(..equipment_prefix.len());
        }
        text.push_str(&equipment);
        if text == "\n" {
            text.clear();
        }
        Ok(text)
    }

    // 140712180, body only; callers supply the trait's highlighted name.
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
        self.char_country_trait_body(trait_, prefix, scopes, true)
            .map(Some)
    }

    // 140FFFDD0 -> 140FFF670, fixed native key construction and line order.
    fn char_adjuster_lines(&self, adjuster: &Adjuster, scopes: Scopes) -> Result<String, String> {
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

    // 140AD9060 (indent 2, scale 1); custom_effect_tooltip is intentionally
    // outside this helper: only its tooltip-item callers invoke 140AD8CF0.
    fn char_unit_trait_body(
        &self,
        definition: &TraitEffects,
        scopes: Scopes,
    ) -> Result<String, String> {
        if let Some(why) = definition.unsupported.first() {
            return Err(why.clone());
        }
        if !definition.conditions.is_empty() {
            return Err(
                "unit trait modifier registry conditions remain unresolved (140ADCB60)".into(),
            );
        }
        let mut text = String::new();
        if !definition.override_tooltips.is_empty() {
            for key in &definition.override_tooltips {
                text.push_str(&self.line(key.as_bytes(), &[], scopes, false)?);
            }
            return Ok(text);
        }
        for (index, block) in definition.blocks.iter().enumerate() {
            text.push_str(&self.char_trait_modifiers(block)?);
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
                    SubUnit::Unknown => continue, // 1410079E0 reports/drops it.
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
            for (index, entry) in entries {
                let block = self.char_trait_modifiers(&entry.block)?;
                let lines = self.char_adjuster_lines(&entry.adjuster, scopes)?;
                newline(&mut text);
                let name = database
                    .name(index)
                    .ok_or("sub-unit database index has no name")??;
                text.push_str("\u{11}H");
                text.push_str(&self.char_current_country_key(name.as_bytes(), scopes)?);
                text.push_str("\u{11}!\n");
                text.push_str(&lines);
                if !block.is_empty() {
                    text.push_str("    ");
                    text.push_str(&block);
                }
            }
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

    // Own 1402EB210 / 1402EC760; NAME@142727E7C, TRAITNAME@142727BD0,
    // TRAITMODIFIER@142727BC0; prefixes '  -'@142727E78 and explicit LF.
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
            body = self.char_trait_modifiers(source)?;
        } else {
            if let Some(country) = country {
                body = self.char_trait_modifiers(&country.modifiers)?;
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
            let name = self.effect_character_name(character)?;
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

    // Own 1402EBC60 / 1402ED1D0 / 1402EADD0 use only the main modifier
    // (+0x370), separator '\n      - '@142727BB0. Duration NUMDAYS is i32.
    pub(in crate::effect_description) fn char_unit_trait(
        &self,
        item: &Item,
        scopes: Scopes,
        remove: bool,
        timed: bool,
    ) -> Result<String, String> {
        // Timed 1402F0FF0 reads trait/days; untimed 141385570 reads only the scalar.
        let token = if timed {
            operand(item, "trait").unwrap_or_default()
        } else {
            item.value.scalar().unwrap_or_default()
        };
        let name = std::str::from_utf8(token)
            .map_err(|_| "unit trait operand is not UTF-8")?;
        let database = self.char_unit_catalogue()?;
        let Some(definition) = database.traits.get(name) else {
            return Ok(String::new());
        };
        let name = self.char_current_country_key(definition.name.as_bytes(), scopes)?;
        let body = self.char_trait_modifiers(&definition.blocks[0])?;
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
                .map(installed_content::fixed5::decimal_i32)
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

    // Own 1403723B0 / 140388540; TRAIT@142750A08, LEADER@142750024;
    // name highlight is '\n    ' + H(name) + LF, body '  - ' + helper + LF.
    pub(in crate::effect_description) fn char_country_trait(
        &self,
        item: &Item,
        scopes: Scopes,
        remove: bool,
    ) -> Result<String, String> {
        let mut character = None;
        if let Some(current) = self.effect_character(scopes.this)?
            && self
                .char_country_leader_scope(current, operand(item, "ideology"))?
                .is_some()
        {
            character = Some(current);
        }
        if character.is_none()
            && let Scope::Country(country) = scopes.this
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
            .by_token(&installed_content::tokens::Token::c_str(name))
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
        let leader = self.char_country_leader_display_name(character, scopes)?;
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

    // Own 140397AB0 / 1402EDD50; 'ADD'@1427285D4, REMOVE@142751904.
    // AddFilteredModifierBlock 14054AD00/14054AE00 adds raw new minus old.
    // It does not copy source hidden sets or custom tooltips to the root.
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
                .by_token(&installed_content::tokens::Token::c_str(
                    source.value.scalar().unwrap_or_default(),
                ))
                .map_err(|why| why.to_string())
        };
        let (Some(old), Some(new)) = (lookup("remove")?, lookup("add")?) else {
            return Ok(if character_only {
                String::new()
            } else {
                "ERROR: TRAITS ARE MISSING\n".into()
            }); // literal@142751A88.
        };
        let character = if character_only {
            let Some(character) = self.effect_character(scopes.this)? else {
                return Ok(String::new());
            };
            self.char_country_leader_scope(character, operand(item, "ideology"))?
                .map(|_| character)
        } else {
            let country = self.current_country(scopes)?;
            self.char_ruling_leader(country)?
        };
        let Some(character) = character else {
            return Ok(String::new());
        };
        let leader = self.char_country_leader_display_name(character, scopes)?;
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
        text.push_str(&self.char_trait_modifiers(&delta)?);
        text.push('\n');
        Ok(text)
    }

    // Own 1402EA740 -> 1402EECF0: remove already held definition pointers,
    // draw on a copied event scope; the caller's seed words do not advance.
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
        let held: Vec<u32> = match leader {
            UnitLeader::Referenced(row) => row.trait_definitions.clone(),
            UnitLeader::Direct(row) => {
                let reference = (u64::from(row.id) << 32) | u64::from(row.reference_type);
                self.character_table::<tables::UnitLeaderTraits>()?
                    .iter()
                    .filter(|trait_| trait_.leader_reference == reference)
                    .map(|trait_| trait_.definition_ordinal as u32)
                    .collect()
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
        let index =
            crate::scripted_text::random::scoped_draw(scope.random_seed, scope.random_counter)
                as usize
                % candidates.len();
        let Some(definition) = database.traits.get(candidates[index]) else {
            return Ok(String::new());
        };
        let title = self.char_current_country_key(definition.name.as_bytes(), scopes)?;
        let body = self.char_trait_modifiers(&definition.blocks[0])?;
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

    // Own 1402EA3E0: fixed_round 1424D8760, NUM@142727638, explicit LF.
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
