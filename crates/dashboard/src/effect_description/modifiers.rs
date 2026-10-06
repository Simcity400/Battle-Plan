use super::*;
use crate::{
    products::{
        identity,
        politics_content::{self, CountryLook, IdeaCountry, IdeaName},
    },
    triggers::{Scope, Scopes},
};
use installed_content::{localisation::crt_case_text, modifiers::Modifiers, scope::Scope as Parsed};
use replay::rows::{CachedModifier, ModifierDefinitionRow, tables};

mod advisors;
mod power_balance;

fn text_field<'a>(fields: &'a [Item], name: &str) -> Result<&'a str, String> {
    let bytes = field(fields, name)
        .and_then(|item| item.value.scalar())
        .unwrap_or_default();
    std::str::from_utf8(installed_content::parser::c_string(bytes))
        .map_err(|_| format!("modifier {name} is not UTF-8"))
}

fn idea_tokens(item: &Item) -> Result<Vec<&str>, String> {
    let tokens: Vec<_> = if let Some(value) = item.value.scalar() {
        vec![value]
    } else {
        children(item)
            .iter()
            .filter_map(|child| child.value.scalar())
            .collect()
    };
    tokens
        .into_iter()
        .map(|bytes| {
            std::str::from_utf8(installed_content::parser::c_string(bytes))
                .map_err(|_| "idea token is not UTF-8".to_owned())
        })
        .collect()
}

fn newline(text: &mut String) {
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
}

fn clean_modifiers(text: &str) -> String {
    trim_description(text)
        .replace('\n', ", ")
        .replace(":,", ":")
        .replace(": ,", ":")
        .replace("    ", " ")
}

impl Renderer<'_, '_, '_> {
    fn idea<'a>(&'a self, token: &str) -> Result<Option<&'a installed_content::ideas::Idea>, String> {
        self.context
            .content
            .ideas
            .ok_or_else(|| self.content_gap("ideas"))?
            .idea(token)
            .map_err(|why| why.to_string())
    }

    fn idea_reference(
        &self,
        operand: &str,
        scopes: Scopes,
    ) -> Result<Option<&installed_content::ideas::Idea>, String> {
        // 1403A85A0 treats token: as a literal; ':' or '.' chooses a variable.
        if let Some(token) = operand.strip_prefix("token:") {
            return self.idea(token);
        }
        if !operand.bytes().any(|byte| matches!(byte, b':' | b'.')) {
            return self.idea(operand);
        }
        // 14032A6D0 -> 1405382B0, signed /100000 then lexer token lookup.
        let token = (self.scoped_value(operand.as_bytes(), scopes)? / 100_000) as u32;
        let rows = self
            .context
            .world()?
            .frame()
            .complete::<tables::LexicalTokens>()
            .map_err(|why| why.to_string())?;
        let literal = rows.iter().find(|row| row.token == token).ok_or_else(|| {
            format!("idea variable references unrecorded lexer token {token} (14032A6D0)")
        })?;
        let literal = std::str::from_utf8(installed_content::parser::c_string(&literal.literal))
            .map_err(|_| "idea variable token is not UTF-8")?;
        self.idea(literal)
    }

    // CIdea::GetDisplayNameForCountry@140FBCF90, shared by all idea effects.
    fn idea_name(&self, idea: &installed_content::ideas::Idea, scopes: Scopes) -> Result<String, String> {
        let country = self.current_country(scopes)?;
        let names = self
            .context
            .content
            .name_lists
            .ok_or_else(|| self.content_gap("name lists"))?;
        let world = self.context.world()?;
        let appearance = world
            .frame()
            .complete::<tables::CountryAppearance>()
            .map_err(|why| why.to_string())?
            .iter()
            .find(|row| world.recorded().slot(row.country) == Some(country));
        let civil = identity::civil_war(world.frame(), country);
        let look = appearance
            .map(|row| -> Result<_, String> {
                Ok(CountryLook {
                    name_tag: std::str::from_utf8(&row.name_tag)
                        .map_err(|_| "idea country name_tag is not UTF-8")?,
                    cosmetic_tag: std::str::from_utf8(&row.cosmetic_tag)
                        .map_err(|_| "idea country cosmetic_tag is not UTF-8")?,
                    civil_war: civil.as_ref().copied().map_err(String::as_str),
                })
            })
            .transpose()?;
        let ideas = self
            .context
            .content
            .ideas
            .ok_or_else(|| self.content_gap("ideas"))?;
        match politics_content::idea_name(
            ideas,
            names,
            self.context.localisation,
            &idea.token,
            IdeaCountry {
                tag: self.tag(country)?,
                slot: country,
                look,
            },
        )? {
            IdeaName::Key(key) => self.localised(key.as_bytes(), &[], scopes, true),
            IdeaName::Generated(name) => Ok(name),
        }
    }

    // 140FB9FF0/140FBB140/140FBDA10/140FBC770, in the native fragment order.
    fn idea_details(
        &self,
        idea: &installed_content::ideas::Idea,
        scopes: Scopes,
    ) -> Result<String, String> {
        let mut text = String::new();
        for bonus in &idea.research_bonuses {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&self.research_bonus_line(bonus, scopes)?);
        }
        newline(&mut text);
        text.push_str(&self.char_equipment_bonuses(&idea.equipment_bonuses, "", scopes)?);
        newline(&mut text);
        for name in &idea.traits {
            let traits = self.char_country_traits()?;
            let Some(trait_) = traits.get(name).map_err(|why| why.to_string())? else {
                continue;
            };
            text.push_str(&self.localised(name.as_bytes(), &[], scopes, false)?);
            text.push('\n');
            let modifiers = self.modifier_description_with(
                &trait_.modifiers,
                "",
                ModifierListOptions {
                    separator: "\n    ",
                    ..Default::default()
                },
            )?;
            if !modifiers.is_empty() {
                text.push_str("    ");
                text.push_str(&modifiers);
                text.push_str("\n\n");
            }
        }
        let options = ModifierListOptions {
            category_mask: 0x00FF_FFFF,
            ..Default::default()
        };
        text.push_str(&self.modifier_list_with(&idea.modifiers, "", scopes, options)?);
        newline(&mut text);
        for target in &idea.targeted {
            // 140FBC770 resolves in a default-empty scope, copies the caller,
            // and only SetCountry(..., true) / builds a header for positive tags.
            let country = target
                .operand
                .as_deref()
                .map(|operand| self.evaluator()?.scoped_country(operand, Scopes::empty()))
                .transpose()?
                .flatten();
            let (selected, prefix) = if let Some(country) = country.filter(|country| *country > 0) {
                let selected = self
                    .evaluator()?
                    .enter_scope(&Parsed::Country(self.tag(country)?.to_owned()), scopes)?
                    .ok_or_else(|| format!("idea {} target names no country", idea.token))?;
                let definite = crt_case_text(&self.country_name(country, true)?, true, true);
                let prefix = self.char_native_text(
                    b"EFFECT_CONTEXT_COUNTRY",
                    &[
                        ("TAG", Argument::Text(self.tag(country)?)),
                        ("COUNTRY", Argument::Text(&definite)),
                    ],
                    selected,
                    false,
                )?;
                (selected, prefix)
            } else {
                (scopes, String::new())
            };
            text.push_str(&self.modifier_list_with(
                &target.modifiers,
                &prefix,
                selected,
                options,
            )?);
            text.push('\n');
        }
        if text == "\n" {
            text.clear();
        }
        Ok(text)
    }

    // 140FB9FF0/1413CA140: authored category key + "_research".
    fn research_bonus_line(
        &self,
        bonus: &installed_content::modifiers::Modifier,
        scopes: Scopes,
    ) -> Result<String, String> {
        let category = self.localised(
            format!("{}_research", bonus.key).as_bytes(), &[], scopes, false,
        )?;
        self.char_native_text(
            b"POLITICAL_RESEARCH_BONUS_ENTRY",
            &[("CATEGORY", Argument::Text(&category)), ("FACTOR", Argument::Fixed5(bonus.value))],
            scopes, false,
        )
    }

    // 140FBA570 uses the short wrapper only when the complete detail text is empty.
    fn idea_tooltip(
        &self,
        idea: &installed_content::ideas::Idea,
        scopes: Scopes,
    ) -> Result<String, String> {
        let name = self.idea_name(idea, scopes)?;
        let details = trim_description(&self.idea_details(idea, scopes)?).to_owned();
        if details.is_empty() {
            self.char_native_text(
                b"EFFECT_SHOW_IDEA_TOOLTIP",
                &[("NAME", Argument::Text(&name))],
                scopes,
                true,
            )
        } else {
            self.char_native_line(
                b"EFFECT_SHOW_IDEA_TOOLTIP_DETAILED",
                &[
                    ("NAME", Argument::Text(&name)),
                    ("DESC", Argument::Text(&details)),
                ],
                scopes,
                true,
            )
        }
    }

    // 140378970 concatenates four fragments, cleans them, then expands the wrapper.
    pub(super) fn timed_idea(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let fields = children(item);
        let token = text_field(fields, "idea")?;
        let Some(idea) = self.idea_reference(token, scopes)? else {
            return Ok(String::new());
        };
        let name = self.idea_name(idea, scopes)?;
        let description = clean_modifiers(&self.idea_details(idea, scopes)?);
        let duration = self.duration(fields, scopes)?;
        self.char_native_text(
            b"EFFECT_ADD_TIMED_IDEA",
            &[
                ("IDEA", Argument::Text(&name)),
                ("TIME_DESC", Argument::Text(&duration)),
                ("DESC", Argument::Text(&description)),
            ],
            scopes,
            true,
        )
    }

    fn idea_advisor(&self, token: &str) -> Result<bool, String> {
        // 1406ACA00: registered advisor tokens take precedence over CIdea.
        Ok(self.context.text.frame()
            .complete::<tables::AdvisorTokens>()
            .map_err(|why| why.to_string())?
            .iter()
            .any(|row| row.token.eq_ignore_ascii_case(token.as_bytes())))
    }

    pub(super) fn ideas_effect(
        &mut self,
        item: &Item,
        scopes: Scopes,
        remove: bool,
        show: bool,
    ) -> Result<String, String> {
        let tokens = idea_tokens(item)?;
        let mut names = Vec::new();
        for token in &tokens {
            if self.idea_advisor(token)? {
                let Some(character) = self.advisor_idea_character(token, scopes)? else {
                    // 140369290 returns empty for a missing association;
                    // 140394850's multi-token loop continues with other names.
                    if !show || tokens.len() == 1 {
                        return Ok(String::new());
                    }
                    // 14039504D still tries CIdea when the association misses.
                    let name = self.idea(token)?
                        .map(|idea| self.idea_name(idea, scopes)).transpose()?
                        .unwrap_or_default();
                    names.push(name);
                    continue;
                };
                let character = self.effect_character(Scope::Character(character))?
                    .ok_or_else(|| format!("Characters lacks advisor character {character}"))?;
                names.push(if tokens.len() == 1 {
                    self.advisor_idea_tooltip(character, token, scopes, remove, show)?
                } else {
                    let mut name = self.effect_character_name(character)?;
                    // 14039504D also tries the CIdea name, even for an advisor.
                    if show && let Some(idea) = self.idea(token)? {
                        name.push_str(&self.idea_name(idea, scopes)?);
                    }
                    name
                });
                continue;
            }
            let idea = if show {
                self.idea(token)?
            } else {
                self.idea_reference(token, scopes)?
            };
            if let Some(idea) = idea {
                names.push(if tokens.len() == 1 {
                    self.idea_tooltip(idea, scopes)?
                } else {
                    let name = self.idea_name(idea, scopes)?;
                    // Show calls 1405260E0 at both 140394FC1 and 14039504D.
                    if show { format!("{name}{name}") } else { name }
                });
            }
        }
        let mut name = names.join(", ");
        // 140369290 removes the literal ".\n" (14271CCC0), not indentation.
        if !show {
            name = name.replace(".\n", "");
        }
        if show && tokens.len() == 1 {
            return Ok(name);
        }
        self.char_native_text(
            if show {
                b"EFFECT_SHOW_IDEA_TOOLTIP"
            } else if remove {
                b"EFFECT_REMOVE_IDEA"
            } else {
                b"EFFECT_ADD_IDEA"
            },
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        )
    }

    pub(super) fn remove_ideas_with_trait(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        // 140389480 localises the authored trait and current country's name.
        let trait_ = self.localised(scalar_operand(item)?, &[], scopes, true)?;
        let country = self.country_name(self.current_country(scopes)?, false)?;
        self.char_native_text(
            b"EFFECT_REMOVE_IDEAS_WITH_TRAIT",
            &[
                ("COUNTRY", Argument::Text(&country)),
                ("TRAIT", Argument::Text(&trait_)),
            ],
            scopes,
            true,
        )
    }

    pub(super) fn modify_timed_idea(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let token = text_field(children(item), "idea")?;
        if self.idea_reference(token, scopes)?.is_none() {
            return Ok(String::new());
        }
        self.current_country(scopes)?;
        // 140385480 tests membership with 140B9D8F0, then 140B9C980 reads days.
        Err("timed idea description requires unrecorded CPoliticalStatus timed ideas (+0x68/count +0x74, entry idea +8, remaining days +0x10; 140B9D8F0/140B9C980, writer 140B980F0)".into())
    }

    pub(super) fn swap_ideas(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let fields = children(item);
        let add = text_field(fields, "add_idea")?;
        let remove = text_field(fields, "remove_idea")?;
        if self.idea_reference(add, scopes)?.is_none()
            || self.idea_reference(remove, scopes)?.is_none()
        {
            return Ok("ERROR: IDEAS ARE MISSING\n".to_owned()); // 142751718.
        }
        self.current_country(scopes)?;
        // 140396910 always reads old duration, even an untimed authored swap.
        Err("idea swap description requires unrecorded CPoliticalStatus timed ideas (+0x68/count +0x74, entry idea +8, remaining days +0x10; 140396910/14039DBB0/140B9C980, writer 140B980F0)".into())
    }

    pub(super) fn opinion_modifier(
        &self,
        item: &Item,
        scopes: Scopes,
        remove: bool,
        reverse: bool,
    ) -> Result<String, String> {
        let token = text_field(children(item), "modifier")?;
        let database = self
            .context
            .content
            .opinion_modifiers
            .ok_or_else(|| self.content_gap("opinion modifiers"))?;
        let Some(modifier) = database.find(token)? else {
            return Ok(String::new());
        };
        let target = self
            .country_target(item, scopes)?
            .unwrap_or(self.current_country(scopes)?);
        let trust = if remove {
            -i32::from(modifier.opinion)
        } else {
            i32::from(modifier.opinion)
        };
        let colour = if trust > 0 {
            'G'
        } else if trust < 0 {
            'R'
        } else {
            'Y'
        };
        let trust = format!(
            "\u{11}{colour}{}{trust}\u{11}!",
            if trust > 0 { "+" } else { "" }
        );
        let name = self.localised(modifier.name.as_bytes(), &[], scopes, false)?;
        let country = self.country_name(target, false)?;
        let key: &[u8] = match (remove, reverse, modifier.trade) {
            (true, _, true) => b"EFFECT_REMOVE_TRADE_OPINION",
            (true, _, false) => b"EFFECT_REMOVE_OPINION",
            (false, true, true) => b"EFFECT_REVERSE_ADD_TRADE_OPINION",
            (false, true, false) => b"EFFECT_REVERSE_ADD_OPINION",
            (false, false, true) => b"EFFECT_ADD_TRADE_OPINION",
            (false, false, false) => b"EFFECT_ADD_OPINION",
        };
        // 1403759C0/14038A4F0 append LF; days affects execution, not this text.
        self.char_native_line(
            key,
            &[
                ("TAG", Argument::Text(self.tag(target)?)),
                ("TARGET", Argument::Text(&country)),
                ("TRUST", Argument::Text(&trust)),
                ("NAME", Argument::Text(&name)),
            ],
            scopes,
            true,
        )
    }

    fn static_modifier(&self, token: &str) -> Result<(Option<String>, Modifiers), String> {
        self.context
            .content
            .static_modifiers
            .ok_or_else(|| self.content_gap("static modifiers"))?
            .get(&self.context.text.content().layers.layers, token)
    }

    pub(super) fn relation_modifier(
        &self,
        item: &Item,
        scopes: Scopes,
        remove: bool,
    ) -> Result<String, String> {
        let token = text_field(children(item), "modifier")?;
        let (name, modifiers) = self.static_modifier(token)?;
        let Some(name) = name else {
            return Ok(String::new());
        };
        let target = self
            .country_target(item, scopes)?
            .unwrap_or(self.current_country(scopes)?);
        let country = self.country_name(target, false)?;
        let name = self.localised(name.as_bytes(), &[], scopes, false)?;
        let description = self.modifier_description_with(&modifiers, "", Default::default())?;
        // 140376590/14038AB90: target TARGET, static NAME, raw DESC, scoped wrapper.
        self.char_native_text(
            if remove {
                b"EFFECT_REMOVE_RELATION_MODIFIER"
            } else {
                b"EFFECT_ADD_RELATION_MODIFIER"
            },
            &[
                ("TARGET", Argument::Text(&country)),
                ("NAME", Argument::Text(&name)),
                ("DESC", Argument::Text(&description)),
            ],
            scopes,
            true,
        )
    }

    pub(super) fn dynamic_modifier(
        &self,
        item: &Item,
        scopes: Scopes,
        remove: bool,
    ) -> Result<String, String> {
        let fields = children(item);
        let token = text_field(fields, "modifier")?;
        let database = self
            .context
            .content
            .dynamic_modifiers
            .ok_or_else(|| self.content_gap("dynamic modifiers"))?;
        let Some(modifier) = database.find(token)? else {
            return Ok(String::new());
        };
        let selected = if let Some(operand) = field(fields, "scope") {
            self.evaluator()?
                .scoped_scope(scalar_operand(operand)?, scopes)?
        } else {
            None
        };
        let selected_scopes = match selected {
            Some(Scope::Country(country)) => self.fresh_country_state_scope(country, 0)?,
            Some(Scope::State(state)) => self.fresh_country_state_scope(0, state)?,
            _ => scopes,
        };
        let mut name = self.localised(modifier.name.as_bytes(), &[], selected_scopes, true)?;
        if let Some(selected) = selected {
            let selected_name = match selected {
                Scope::Country(country) => Some(self.country_name(country, false)?),
                Scope::State(state) => Some(self.state_name(state)?),
                _ => None,
            };
            if let Some(selected_name) = selected_name {
                name.push_str(" (");
                name.push_str(&selected_name);
                name.push(')');
            }
        }
        let mut description =
            self.modifier_list_with(&modifier.modifiers, "", selected_scopes, Default::default())?;
        newline(&mut description);
        // 14054DAD0 formats each scoped entry in authored order.
        let mut scoped_lines = Vec::new();
        for (key, operand) in &modifier.scoped_modifiers {
            let value = self.scoped_value(operand, selected_scopes)?;
            let mut modifiers = Modifiers::default();
            modifiers.values.push(installed_content::modifiers::Modifier {
                key: key.clone(),
                name: key.to_ascii_lowercase(),
                text: String::from_utf8_lossy(operand).into_owned(),
                value,
            });
            let line =
                self.modifier_list_with(&modifiers, "", selected_scopes, Default::default())?;
            if !line.is_empty() {
                scoped_lines.push(line);
            }
        }
        description.push_str(&scoped_lines.join("\n"));
        let days = if remove {
            -1
        } else {
            field(fields, "days")
                .map(|days| self.scoped_value(scalar_operand(days)?, scopes))
                .transpose()?
                .map(crate::triggers::fixed_round)
                .unwrap_or(-1) as i32
        };
        let key: &[u8] = if remove {
            b"EFFECT_REMOVE_DYMAMIC_MODIFIER"
        } else if days < 0 {
            b"EFFECT_ADD_DYMAMIC_MODIFIER"
        } else {
            b"EFFECT_ADD_DYMAMIC_MODIFIER_WITH_DAYS"
        };
        self.char_native_text(
            key,
            &[
                ("NAME", Argument::Text(&name)),
                ("DAYS", Argument::Int32(days)),
                ("DESC", Argument::Text(&description)),
            ],
            scopes,
            true,
        )
    }

    pub(super) fn province_modifier(
        &self,
        item: &Item,
        scopes: Scopes,
        remove: bool,
    ) -> Result<String, String> {
        let fields = field(children(item), "static_modifiers")
            .map(children)
            .unwrap_or_default();
        let tokens: Vec<_> = fields
            .iter()
            .filter_map(|item| item.value.scalar())
            .collect();
        let mut list = String::new();
        for (index, token) in tokens.iter().enumerate() {
            let token = std::str::from_utf8(installed_content::parser::c_string(token))
                .map_err(|_| "province modifier is not UTF-8")?;
            let (name, modifiers) = self.static_modifier(token)?;
            let name = match name {
                Some(name) => self.localised(name.as_bytes(), &[], scopes, false)?,
                None => String::new(),
            };
            let name = if tokens.len() > 1 && index == 0 {
                format!("\n{name}")
            } else {
                name
            };
            let description = "\n    ".to_owned()
                + &self.modifier_description_with(
                    &modifiers,
                    "",
                    ModifierListOptions {
                        separator: "\n    ",
                        ..Default::default()
                    },
                )?;
            list.push_str(&self.char_native_text(
                b"EFFECT_PROVINCE_MODIFIER_ENTRY",
                &[
                    ("NAME", Argument::Text(&name)),
                    ("MODIFIERS", Argument::Text(&description)),
                ],
                scopes,
                false,
            )?);
            list.push('\n');
        }
        // 140314F00/140316F70 compute days/province list but omit them from own text.
        let description = self.char_native_text(
            if remove {
                b"EFFECT_REMOVE_PROVINCE_MODIFIER"
            } else {
                b"EFFECT_ADD_PROVINCE_MODIFIER"
            },
            &[("MODIFIERLIST", Argument::Text(&list))],
            scopes,
            false,
        )?;
        Ok(if tokens.len() > 1 {
            format!("\n{description}")
        } else {
            description
        })
    }

    pub(super) fn state_modifier(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let fields = field(children(item), "modifier")
            .map(children)
            .unwrap_or_default();
        let modifiers = installed_content::modifiers::values(fields);
        let mut text = String::new();
        let frame = self.context.world()?.frame();
        let definitions = frame
            .complete::<tables::ModifierDefinitions>()
            .map_err(|why| why.to_string())?;
        let tokens = frame
            .complete::<tables::LexicalTokens>()
            .map_err(|why| why.to_string())?;
        for modifier in modifiers.listed() {
            let definition = definitions
                .iter()
                .find(|definition| {
                    tokens.iter().any(|token| {
                        token.token == definition.token
                            && token.literal.eq_ignore_ascii_case(modifier.name.as_bytes())
                    })
                })
                .ok_or_else(|| {
                    format!("state modifier {} has no recorded definition", modifier.key)
                })?;
            if definition.feature_selector != 0 {
                let features = frame
                    .complete::<tables::SimulationFeatures>()
                    .map_err(|why| why.to_string())?;
                let [features] = features else {
                    return Err("SimulationFeatures has no unique feature mask".into());
                };
                if features.dlc_mask & definition.feature_selector == 0 {
                    continue;
                }
            }
            let argument = crate::scripted_text::stored_text(
                self.context.localisation,
                self.context.text.recorded_language(),
                &definition.argument,
                "state modifier argument",
            )?;
            let name = self
                .context
                .localisation
                .localise_with(&definition.name_key, &[("ARG", Argument::Text(&argument))])
                .map_err(|why| why.to_string())?;
            let effective = if definition.flags & engine::modifier_definition::CLAMP_MAXIMUM != 0 {
                modifier.value.clamp(0, 100_000)
            } else {
                modifier.value
            };
            let value = crate::modifier_text::value_text(
                definition,
                effective,
                2,
                false,
                false,
                |key, _| {
                    self.context
                        .localisation
                        .localise(key)
                        .map_err(|why| why.to_string())
                },
            )?;
            // 140314A70 -> 14054C770 passes colour byte 1 and includes zero.
            text.push_str("\n\u{11}\u{1}");
            text.push_str(&name);
            text.push_str(": ");
            text.push_str(&value);
            text.push_str("\u{11}!");
        }
        self.char_native_text(
            b"EFFECT_ADD_STATE_MODIFIER",
            &[("MODIFIERS", Argument::Text(&text))],
            scopes,
            false,
        )
    }

    fn duration(&self, fields: &[Item], scopes: Scopes) -> Result<String, String> {
        let mut text = String::new();
        // 1403A21E0 floors each stored component separately, then casts to i32.
        for (name, key, trailing) in [
            ("years", b"FREQUENCY_YEAR".as_slice(), true),
            ("months", b"FREQUENCY_MONTH".as_slice(), true),
            ("days", b"FREQUENCY_DAY".as_slice(), false),
        ] {
            let Some(item) = field(fields, name) else {
                continue;
            };
            let raw = self.scoped_value(item.value.scalar().unwrap_or_default(), scopes)?;
            let whole = raw.div_euclid(100_000) as i32;
            if whole <= 0 {
                continue;
            }
            text.push_str(&self.localised(
                b"EFFECT_TIMED_IDEA_TIME_VALUE",
                &[("VALUE", Argument::Int32(whole))],
                scopes,
                false,
            )?);
            text.push(' ');
            text.push_str(&self.localised(key, &[], scopes, false)?);
            if trailing {
                text.push(' ');
            }
        }
        Ok(text)
    }

    pub(super) fn modifier_list(
        &self,
        modifiers: &Modifiers,
        prefix: &str,
        scopes: Scopes,
    ) -> Result<String, String> {
        self.modifier_list_with(modifiers, prefix, scopes, ModifierListOptions::default())
    }

    /// 14054E520 expands 14054E050's raw list in the supplied event scope.
    pub(super) fn modifier_list_with(
        &self,
        modifiers: &Modifiers,
        prefix: &str,
        scopes: Scopes,
        options: ModifierListOptions<'_>,
    ) -> Result<String, String> {
        self.expanded(
            &self.modifier_description_with(modifiers, prefix, options)?,
            scopes,
        )
    }

    /// 14054E050: raw text; its direct callers choose when to expand it.
    pub(super) fn modifier_description_with(
        &self,
        modifiers: &Modifiers,
        prefix: &str,
        options: ModifierListOptions<'_>,
    ) -> Result<String, String> {
        if let Some(why) = modifiers.unheld.cause() {
            return Err(why.to_owned());
        }
        let frame = self.context.world()?.frame();
        let definitions = frame
            .complete::<tables::ModifierDefinitions>()
            .map_err(|why| why.to_string())?;
        let tokens = frame
            .complete::<tables::LexicalTokens>()
            .map_err(|why| why.to_string())?;
        let mut values = modifiers
            .listed()
            .map(|modifier| {
                let definition = definitions
                    .iter()
                    .find(|definition| {
                        tokens.iter().any(|token| {
                            token.token == definition.token
                                && token.literal.eq_ignore_ascii_case(modifier.name.as_bytes())
                        })
                    })
                    .ok_or_else(|| {
                        format!("idea modifier {} has no recorded definition", modifier.key)
                    })?;
                Ok((modifier.value, definition))
            })
            .collect::<Result<Vec<_>, String>>()?;
        values.sort_by_key(|(_, definition)| definition.index);
        self.modifier_description_values(&values, &modifiers.tooltips(), prefix, options)
    }

    /// 14128C8E0 -> 14054E050: stored definition IDs and numeric-array order.
    pub(super) fn cached_modifier_description_with(
        &self,
        entries: &[CachedModifier],
        prefix: &str,
        options: ModifierListOptions<'_>,
    ) -> Result<String, String> {
        if entries.is_empty() {
            return self.modifier_description_values(&[], &[], prefix, options);
        }
        let definitions = self.context.text.frame()
            .complete::<tables::ModifierDefinitions>()
            .map_err(|why| why.to_string())?;
        let values = entries.iter().map(|entry| {
            let definition = definitions.iter()
                .find(|definition| definition.index == entry.definition)
                .ok_or_else(|| format!("advisor cached modifier {} has no recorded definition", entry.definition))?;
            Ok((entry.value, definition))
        }).collect::<Result<Vec<_>, String>>()?;
        self.modifier_description_values(&values, &[], prefix, options)
    }

    fn modifier_description_values(
        &self,
        values: &[(i64, &ModifierDefinitionRow)],
        tooltips: &[String],
        prefix: &str,
        options: ModifierListOptions<'_>,
    ) -> Result<String, String> {
        if !options.outer_colour.is_ascii() {
            return Err(format!(
                "modifier outer colour byte {:02x} is not UTF-8 (14054E0CC)",
                options.outer_colour
            ));
        }
        let frame = self.context.text.frame();
        let mut out = String::new();
        if options.outer_colour != 0 {
            // 14054E0CC: these bytes precede the first line's separator check.
            out.push('\u{11}');
            out.push(char::from(options.outer_colour));
        }
        for &(raw_value, definition) in values {
            // 14054F743/14054F74F: separate include/exclude intersections.
            if definition.category_mask & options.category_mask == 0
                || definition.category_mask & !options.excluded_category_mask == 0
            {
                continue;
            }
            // Getter 140551820 clamps before 14054E198's wrapping IMUL and
            // signed /100000; FormatLine 14054F6D0 tests the scaled zero.
            let effective = if definition.flags & engine::modifier_definition::CLAMP_MAXIMUM != 0 {
                raw_value.clamp(0, 100_000)
            } else {
                raw_value
            };
            let value = effective.wrapping_mul(options.scale) / 100_000;
            if value == 0 && definition.flags & 0x100 == 0 {
                continue;
            }
            if definition.feature_selector != 0 {
                let features = frame
                    .complete::<tables::SimulationFeatures>()
                    .map_err(|why| why.to_string())?;
                let [features] = features else {
                    return Err("SimulationFeatures has no unique feature mask".into());
                };
                if features.dlc_mask & definition.feature_selector == 0 {
                    continue;
                }
            }
            let argument = crate::scripted_text::stored_text(
                self.context.localisation,
                self.context.text.recorded_language(),
                &definition.argument,
                "idea modifier argument",
            )?;
            let label = self
                .context
                .localisation
                .localise_with(&definition.name_key, &[("ARG", Argument::Text(&argument))])
                .map_err(|why| why.to_string())?;
            let value = crate::modifier_text::value_text(
                definition,
                value,
                options.precision,
                options.force_plus,
                options.invert,
                |key, _| {
                    self.context
                        .localisation
                        .localise(key)
                        .map_err(|why| why.to_string())
                },
            )?;
            if !out.is_empty() {
                // 14054F81D appends the separator as a NUL-terminated string.
                out.push_str(options.separator.split('\0').next().unwrap_or_default());
            }
            out.push_str(prefix);
            out.push_str(&label);
            out.push_str(": ");
            out.push_str(&value);
        }
        for tooltip in tooltips {
            if tooltip.is_empty() {
                continue;
            }
            // 14054E296/14054E2AF: LF only if needed, no numeric prefix.
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(
                &self
                    .context
                    .localisation
                    .localise(tooltip.as_bytes())
                    .map_err(|why| why.to_string())?,
            );
        }
        if options.outer_colour != 0 {
            // 14054E3DE appends the reset even when there were no lines.
            out.push_str("\u{11}!");
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests;
