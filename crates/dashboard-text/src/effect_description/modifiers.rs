use super::*;
use crate::{
    idea_names::{self, CountryLook, IdeaCountry, IdeaName},
    identity,
    triggers::{Scope, Scopes},
};
use installed_content_core::{modifiers::Modifiers, scope::Scope as Parsed};
use installed_content_localisation::localisation::crt_case_text;
use replay_data::{rows::CachedModifier, rows::ModifierDefinitionRow, rows::tables};

mod advisors;
mod power_balance;

fn text_field<'a>(fields: &'a [Item], name: &str) -> Result<&'a str, String> {
    let bytes = field(fields, name)
        .and_then(|item| item.value.scalar())
        .unwrap_or_default();
    std::str::from_utf8(installed_content_core::parser::c_string(bytes))
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
            std::str::from_utf8(installed_content_core::parser::c_string(bytes))
                .map_err(|_| "idea token is not UTF-8".to_owned())
        })
        .collect()
}

fn newline(text: &mut String) {
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
}

fn clean_modifiers(text: &str, remove_spaced_colon: bool) -> String {
    let text = trim_description(text)
        .replace('\n', ", ")
        .replace(":,", ":");
    let text = if remove_spaced_colon {
        text.replace(": ,", ":")
    } else {
        text
    };
    text.replace("    ", " ")
}

// floor into fixed5 first, wrapping that intermediate,
// then divide and narrow. This differs from integer floor near i64::MIN.
fn timed_component_integer(value: i64) -> i32 {
    let floored = if value % 100_000 == 0 {
        value
    } else {
        (value / 100_000 + (value >> 63)).wrapping_mul(100_000)
    };
    (floored / 100_000) as i32
}

impl<'r, 'c, 'f> Renderer<'r, 'c, 'f> {
    fn idea(&self, token: &str) -> Result<Option<&'c installed_content_core::ideas::Idea>, String> {
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
    ) -> Result<Option<&'c installed_content_core::ideas::Idea>, String> {
        // treats token: as a literal; ':' or '.' chooses a variable.
        if let Some(token) = operand.strip_prefix("token:") {
            return self.idea(token);
        }
        if !operand.bytes().any(|byte| matches!(byte, b':' | b'.')) {
            return self.idea(operand);
        }
        // signed /100000 then lexer token lookup.
        let token = (self.scoped_value(operand.as_bytes(), scopes)? / 100_000) as u32;
        let rows = self
            .context
            .world()?
            .frame()
            .complete::<tables::LexicalTokens>()
            .map_err(|why| why.to_string())?;
        let literal = rows
            .iter()
            .find(|row| row.token == token)
            .ok_or_else(|| format!("idea variable references unrecorded lexer token {token}"))?;
        let literal = std::str::from_utf8(installed_content_core::parser::c_string(&literal.literal))
            .map_err(|_| "idea variable token is not UTF-8")?;
        self.idea(literal)
    }

    // shared by all idea effects.
    fn idea_name(
        &self,
        idea: &installed_content_core::ideas::Idea,
        scopes: Scopes,
    ) -> Result<String, String> {
        // The caller supplies its country carrier, including zero for a
        // non-country scope; name expansion uses a fresh country-only scope.
        let country = match scopes.this {
            Scope::Country(country) | Scope::CountryLeader { country, .. } => country,
            _ => 0,
        };
        let naming_scope = self.fresh_country_state_scope(country, 0)?;
        if let Some(name) = idea.name.as_deref().filter(|name| !name.is_empty()) {
            return self.localised(name.as_bytes(), &[], naming_scope, true);
        }
        let names = self
            .context
            .content
            .name_lists
            .ok_or_else(|| self.content_gap("name lists"))?;
        let world = self.context.world()?;
        let civil = identity::civil_war(world.frame(), country);
        let original_or_own_tag =
            installed_content_country_names::country_names::original_or_own_tag(world.frame(), country);
        let look = if country == 0 {
            let rows = world
                .frame()
                .complete::<tables::ReservedCountryNaming>()
                .map_err(|why| why.to_string())?;
            let [row] = rows else {
                return Err("ReservedCountryNaming has no singleton row for country zero".into());
            };
            Some(CountryLook {
                name_tag: std::str::from_utf8(&row.name_tag)
                    .map_err(|_| "reserved idea name_tag is not UTF-8")?,
                original_or_own_tag: original_or_own_tag.as_deref().map_err(String::as_str),
                cosmetic_tag: std::str::from_utf8(&row.cosmetic_tag)
                    .map_err(|_| "reserved idea cosmetic_tag is not UTF-8")?,
                civil_war: Ok(false), // tests canonical zero, not a dynamic country.
            })
        } else {
            world
                .frame()
                .complete::<tables::CountryAppearance>()
                .map_err(|why| why.to_string())?
                .iter()
                .find(|row| world.recorded().slot(row.country) == Some(country))
                .map(|row| -> Result<_, String> {
                    Ok(CountryLook {
                        name_tag: std::str::from_utf8(&row.name_tag)
                            .map_err(|_| "idea country name_tag is not UTF-8")?,
                        original_or_own_tag: original_or_own_tag.as_deref().map_err(String::as_str),
                        cosmetic_tag: std::str::from_utf8(&row.cosmetic_tag)
                            .map_err(|_| "idea country cosmetic_tag is not UTF-8")?,
                        civil_war: civil.as_ref().copied().map_err(String::as_str),
                    })
                })
                .transpose()?
        };
        let ideas = self
            .context
            .content
            .ideas
            .ok_or_else(|| self.content_gap("ideas"))?;
        match idea_names::idea_name(
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
            IdeaName::Key(key) => self.localised(key.as_bytes(), &[], naming_scope, true),
            IdeaName::Generated(name) => Ok(name),
        }
    }

    // Preserve fragment order.
    fn idea_details(
        &self,
        idea: &installed_content_core::ideas::Idea,
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
            // resolves in a default-empty scope, copies the caller,
            // and only a positive country builds a header.
            let country = target
                .operand
                .as_deref()
                .map(|operand| self.evaluator()?.scoped_country(operand, Scopes::empty()))
                .transpose()
                .map_err(|why| why.to_string())?
                .flatten();
            let (selected, prefix) = if let Some(country) = country.filter(|country| *country > 0) {
                let selected = self
                    .evaluator()?
                    .enter_scope(&Parsed::Country(self.tag(country)?.to_owned()), scopes)
                    .map_err(|why| why.to_string())?
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

    // authored category key + "_research".
    fn research_bonus_line(
        &self,
        bonus: &installed_content_core::modifiers::Modifier,
        scopes: Scopes,
    ) -> Result<String, String> {
        let category = self.localised(
            format!("{}_research", bonus.key).as_bytes(),
            &[],
            scopes,
            false,
        )?;
        self.char_native_text(
            b"POLITICAL_RESEARCH_BONUS_ENTRY",
            &[
                ("CATEGORY", Argument::Text(&category)),
                ("FACTOR", Argument::Fixed5(bonus.value)),
            ],
            scopes,
            false,
        )
    }

    // uses the short wrapper only when the complete detail text is empty.
    fn idea_tooltip(
        &self,
        idea: &installed_content_core::ideas::Idea,
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

    // Bound idea_desc callback; a missing token is empty.
    pub(in crate::effect_description) fn bound_idea_description(
        &self,
        token: &str,
        scopes: Scopes,
    ) -> Result<String, String> {
        self.idea(token)?
            .map(|idea| self.idea_tooltip(idea, scopes))
            .transpose()
            .map(Option::unwrap_or_default)
    }

    // concatenates four fragments, cleans them, then expands the wrapper.
    pub(super) fn timed_idea(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let fields = children(item);
        let token = text_field(fields, "idea")?;
        let Some(idea) = self.idea_reference(token, scopes)? else {
            return Ok(String::new());
        };
        let name = self.idea_name(idea, scopes)?;
        let description = clean_modifiers(&self.idea_details(idea, scopes)?, true);
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
        // registered advisor tokens take precedence over idea.
        Ok(self
            .context
            .text
            .frame()
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
                    // returns empty for a missing association;
                    // The multi-token loop continues with other names.
                    if !show || tokens.len() == 1 {
                        return Ok(String::new());
                    }
                    // still tries idea when the association misses.
                    let name = self
                        .idea(token)?
                        .map(|idea| self.idea_name(idea, scopes))
                        .transpose()?
                        .unwrap_or_default();
                    names.push(name);
                    continue;
                };
                let character = self
                    .effect_character(Scope::Character(character))?
                    .ok_or_else(|| format!("Characters lacks advisor character {character}"))?;
                names.push(if tokens.len() == 1 {
                    self.advisor_idea_tooltip(character, token, scopes, remove, show)?
                } else {
                    let mut name = self.effect_character_name(character)?;
                    // also tries the idea name, even for an advisor.
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
                    // Show describes both the advisor name and the idea name.
                    if show { format!("{name}{name}") } else { name }
                });
            }
        }
        let mut name = names.join(", ");
        // removes the literal ".\n", not indentation.
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
        // localises the authored trait and current country's name.
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

    // returns the first entry's stored days, or -1 without one.
    // Keep membership separate: an entry itself may store -1.
    fn timed_idea_days(
        &self,
        idea: &installed_content_core::ideas::Idea,
        scopes: Scopes,
    ) -> Result<Option<i32>, String> {
        let country = self.current_country(scopes)?;
        let world = self.context.world()?;
        Ok(world
            .frame()
            .complete::<tables::TimedIdeas>()
            .map_err(|why| why.to_string())?
            .iter()
            .filter(|row| {
                world.slot(row.country) == Some(country)
                    && row.token.eq_ignore_ascii_case(idea.token.as_bytes())
            })
            .min_by_key(|row| row.ordinal)
            .map(|row| row.days))
    }

    pub(super) fn modify_timed_idea(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let token = text_field(children(item), "idea")?;
        let Some(idea) = self.idea_reference(token, scopes)? else {
            return Ok(String::new());
        };
        let Some(days) = self.timed_idea_days(idea, scopes)? else {
            return Ok(String::new()); // membership gate.
        };
        let mut added = 0_i32;
        // floors each component before native signed32 arithmetic.
        for (field_name, scale) in [("days", 1), ("years", 365), ("months", 30)] {
            if let Some(field) = field(children(item), field_name) {
                let raw = self.scoped_value(field.value.scalar().unwrap_or_default(), scopes)?;
                added = added.wrapping_add(timed_component_integer(raw).wrapping_mul(scale));
            }
        }
        let (year, month, day) = super::numeric::numeric_date_parts(
            self.context
                .text
                .frame()
                .hour
                .wrapping_add(added.wrapping_add(days).wrapping_mul(24)),
        );
        // localises the month key.
        let month = self.char_native_text(month.as_bytes(), &[], scopes, false)?;
        let date = format!("{day} {month}, {year}");
        let name = self.idea_name(idea, scopes)?;
        // cleans ':,' but, unlike add-timed, retains ':,'.
        let description = clean_modifiers(&self.idea_details(idea, scopes)?, false);
        let duration = self.duration(children(item), scopes)?;
        self.char_native_text(
            if added > 0 {
                b"EFFECT_MODIFY_TIMED_IDEA_INCREASE"
            } else {
                b"EFFECT_MODIFY_TIMED_IDEA_DECREASE"
            },
            &[
                ("IDEA", Argument::Text(&name)),
                ("DATE", Argument::Text(&date)),
                ("TIME_DESC", Argument::Text(&duration)),
                ("DESC", Argument::Text(&description)),
            ],
            scopes,
            true,
        )
    }

    pub(super) fn swap_ideas(&mut self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let fields = children(item);
        let add = text_field(fields, "add_idea")?;
        let remove = text_field(fields, "remove_idea")?;
        let add = self.idea_reference(add, scopes)?;
        let remove = self.idea_reference(remove, scopes)?;
        let (Some(add), Some(remove)) = (add, remove) else {
            return Ok("ERROR: IDEAS ARE MISSING\n".to_owned());
        };
        let old = self.timed_idea_days(remove, scopes)?.unwrap_or(-1);
        let mut days = old;
        // The negative add-days result describes removal alone.
        if let Some(days) = field(fields, "add_days") {
            let added = crate::triggers::fixed_round(
                self.scoped_value(days.value.scalar().unwrap_or_default(), scopes)?,
            ) as i32;
            let total = added.wrapping_add(if old == -1 { 0 } else { old });
            if total < 0 {
                let name = self.idea_name(remove, scopes)?;
                return self.char_native_line(
                    b"EFFECT_REMOVE_IDEA",
                    &[("NAME", Argument::Text(&name))],
                    scopes,
                    true,
                );
            }
            // add_days takes precedence even when days is also authored.
            return self.swap_idea_description(add, remove, old, total, scopes);
        } else if let Some(value) = field(fields, "days") {
            days = crate::triggers::fixed_round(
                self.scoped_value(value.value.scalar().unwrap_or_default(), scopes)?,
            ) as i32;
        }
        self.swap_idea_description(add, remove, old, days, scopes)
    }

    // compares display-name bytes, not idea tokens, before choosing
    // the duration wrapper. Its fragments stay plain; only the header expands.
    fn swap_idea_description(
        &mut self,
        add: &installed_content_core::ideas::Idea,
        remove: &installed_content_core::ideas::Idea,
        old: i32,
        days: i32,
        scopes: Scopes,
    ) -> Result<String, String> {
        let added_name = self.idea_name(add, scopes)?;
        let removed_name = self.idea_name(remove, scopes)?;
        let same = added_name == removed_name;
        let key: &[u8] = match (same, old == days, old == -1, days == -1) {
            (false, true, _, _) => b"EFFECT_SWAP_IDEAS_REPLACE",
            (false, false, true, _) => b"EFFECT_SWAP_IDEAS_REPLACE_AND_SET_DURATION",
            (false, false, _, true) => b"EFFECT_SWAP_IDEAS_REPLACE_AND_CLEAR_DURATION",
            (false, false, _, _) => b"EFFECT_SWAP_IDEAS_REPLACE_AND_ADD_DURATION",
            (true, true, _, _) => b"EFFECT_SWAP_IDEAS_MODIFY",
            (true, false, true, _) => b"EFFECT_SWAP_IDEAS_MODIFY_AND_SET_DURATION",
            (true, false, _, true) => b"EFFECT_SWAP_IDEAS_MODIFY_AND_CLEAR_DURATION",
            (true, false, _, _) => b"EFFECT_SWAP_IDEAS_MODIFY_AND_ADD_DURATION",
        };
        let mut args = vec![("REMOVE", Argument::Text(&removed_name))];
        if !same {
            args.push(("ADD", Argument::Text(&added_name)));
        }
        if old != days && days != -1 {
            args.push(("DAYS", Argument::Int32(days)));
            if old != -1 {
                args.push(("DAYS_MODIFIED", Argument::Int32(days.wrapping_sub(old))));
            }
        }
        let mut text = self.char_native_text(key, &args, scopes, true)?;
        text.push('\n'); // The header appends LF unconditionally.
        for fragment in [
            self.modifier_difference(&add.modifiers, &remove.modifiers, true),
            self.idea_targeted_difference(add, remove, scopes),
            self.idea_research_difference(add, remove, scopes),
            self.idea_rule_difference(add, remove, scopes),
            self.idea_equipment_difference(add, remove, scopes),
        ] {
            match fragment {
                Ok(fragment) => text.push_str(&fragment),
                Err(why) => self.refuse("idea swap", why),
            }
            newline(&mut text);
        }
        text.push('\n'); // terminal LF even after a fragment's LF.
        Ok(text)
    }

    // each raw contribution multiplies before division;
    // the new accumulator does not copy hidden flags or custom tooltip slots.
    fn modifier_difference(
        &self,
        add: &Modifiers,
        remove: &Modifiers,
        force_plus: bool,
    ) -> Result<String, String> {
        let mut combined = Modifiers::default();
        for (source, scale) in [(add, 100_000_i64), (remove, -100_000_i64)] {
            if let Some(why) = source.unheld.cause() {
                return Err(why.to_owned());
            }
            for value in &source.values {
                let contribution = value.value.wrapping_mul(scale) / 100_000;
                if let Some(stored) = combined
                    .values
                    .iter_mut()
                    .find(|stored| stored.name == value.name)
                {
                    stored.value = stored.value.wrapping_add(contribution);
                } else {
                    let mut value = value.clone();
                    value.value = contribution;
                    combined.values.push(value);
                }
            }
        }
        let options = ModifierListOptions {
            category_mask: u32::MAX,
            force_plus,
            ..Default::default()
        };
        if combined.values.is_empty() {
            self.modifier_description_values(&[], &[], "", options)
        } else {
            self.modifier_description_with(&combined, "", options)
        }
    }

    // first matching target per country, in country
    // index order. Each operand evaluates in a fresh default-empty scope.
    fn idea_targeted_difference(
        &self,
        add: &installed_content_core::ideas::Idea,
        remove: &installed_content_core::ideas::Idea,
        scopes: Scopes,
    ) -> Result<String, String> {
        if add.targeted.is_empty() && remove.targeted.is_empty() {
            return Ok(String::new());
        }
        let countries: std::collections::BTreeSet<_> = std::iter::once(0)
            .chain(
                self.context
                    .world()?
                    .country_slots()
                    .map_err(|why| why.to_string())?,
            )
            .collect();
        let empty = Modifiers::default();
        let mut text = String::new();
        for country in countries {
            // evaluates only the prefix up to the first match for
            // this country; a later variable may never be evaluated at all.
            let mut selected = [&empty, &empty];
            for (index, idea) in [add, remove].into_iter().enumerate() {
                for target in &idea.targeted {
                    let candidate = target
                        .operand
                        .as_deref()
                        .map(|operand| self.evaluator()?.scoped_country(operand, Scopes::empty()))
                        .transpose()
                        .map_err(|why| why.to_string())?
                        .flatten()
                        .unwrap_or(0);
                    if candidate == country {
                        selected[index] = &target.modifiers;
                        break;
                    }
                }
            }
            let details = self.modifier_difference(selected[0], selected[1], false)?;
            if details.is_empty() {
                continue;
            }
            let details = format!("    {}", details.replace('\n', "\n    "));
            let name = crt_case_text(&self.country_name(country, true)?, true, true);
            text.push_str(&self.char_native_text(
                b"EFFECT_SWAP_IDEAS_TARGETED_MODIFIERS",
                &[
                    ("COUNTRY", Argument::Text(&name)),
                    ("MODIFIER", Argument::Text(&details)),
                ],
                scopes,
                false,
            )?);
            text.push('\n');
        }
        Ok(text)
    }

    // new order, first old folded match, then remaining
    // old entries in their order. Matched values subtract with signed64 wrap.
    fn idea_research_difference(
        &self,
        add: &installed_content_core::ideas::Idea,
        remove: &installed_content_core::ideas::Idea,
        scopes: Scopes,
    ) -> Result<String, String> {
        let mut remaining: Vec<_> = remove.research_bonuses.iter().map(Some).collect();
        let mut text = String::new();
        for bonus in &add.research_bonuses {
            let mut difference = bonus.clone();
            if let Some(index) = remove
                .research_bonuses
                .iter()
                .position(|old| old.key.eq_ignore_ascii_case(&bonus.key))
            {
                difference.value = bonus
                    .value
                    .wrapping_sub(remove.research_bonuses[index].value);
                remaining[index] = None;
                if difference.value == 0 {
                    continue;
                }
            }
            text.push_str(&self.research_bonus_line(&difference, scopes)?);
            text.push('\n');
        }
        for old in remaining.into_iter().flatten() {
            let mut difference = old.clone();
            difference.value = old.value.wrapping_neg();
            text.push_str(&self.research_bonus_line(&difference, scopes)?);
            text.push('\n');
        }
        if !text.is_empty() {
            text.push('\n');
        }
        Ok(text)
    }

    // a changed rule restored to its native default contributes an empty line.
    fn idea_rule_difference(
        &self,
        add: &installed_content_core::ideas::Idea,
        remove: &installed_content_core::ideas::Idea,
        scopes: Scopes,
    ) -> Result<String, String> {
        let mut text = String::new();
        for (index, &(_, key, default, fulfilled)) in
            installed_content_core::power_balance::rules().iter().enumerate()
        {
            let value = add
                .rules
                .values
                .get(index)
                .copied()
                .ok_or("idea rule values are incomplete")?;
            let old = remove
                .rules
                .values
                .get(index)
                .copied()
                .ok_or("idea rule values are incomplete")?;
            if value == old {
                continue;
            }
            if value != default {
                let mut description = self.localised(
                    if value == fulfilled {
                        b"TRIGGER_FULLFILLED_PREFIX".as_slice()
                    } else {
                        b"TRIGGER_UNFULLFILLED_PREFIX".as_slice()
                    },
                    &[],
                    scopes,
                    false,
                )?;
                description.push(' ');
                description.push_str(&self.localised(key.as_bytes(), &[], scopes, false)?);
                let override_key = if add.rules.description.is_empty() {
                    add.rules
                        .overrides
                        .get(&index)
                        .map(String::as_str)
                        .unwrap_or_default()
                } else {
                    &add.rules.description
                };
                if !override_key.is_empty() {
                    description = self.localised(
                        override_key.as_bytes(),
                        &[("DESC", Argument::Text(&description))],
                        scopes,
                        false,
                    )?;
                }
                text.push_str(&description);
            }
            text.push('\n');
        }
        if !text.is_empty() {
            text.push('\n');
        }
        Ok(text)
    }

    // group equipment names in first-seen order
    // and add/subtract each stored stat; omits zero stats.
    fn idea_equipment_difference(
        &self,
        add: &installed_content_core::ideas::Idea,
        remove: &installed_content_core::ideas::Idea,
        scopes: Scopes,
    ) -> Result<String, String> {
        let mut groups: Vec<installed_content_core::modifiers::EquipmentBonus> = Vec::new();
        for (idea, subtract) in [(add, false), (remove, true)] {
            for bonus in &idea.equipment_bonuses {
                let index = groups
                    .iter()
                    .position(|group| group.equipment.eq_ignore_ascii_case(&bonus.equipment))
                    .unwrap_or_else(|| {
                        groups.push(installed_content_core::modifiers::EquipmentBonus {
                            values: Vec::new(),
                            ..bonus.clone()
                        });
                        groups.len() - 1
                    });
                for value in &bonus.values {
                    let Some(stat) = installed_content_core::equipment::stat_index(value.name.as_bytes())
                    else {
                        continue;
                    };
                    let contribution = if subtract {
                        value.value.wrapping_neg()
                    } else {
                        value.value
                    };
                    if let Some(stored) = groups[index].values.iter_mut().find(|stored| {
                        installed_content_core::equipment::stat_index(stored.name.as_bytes()) == Some(stat)
                    }) {
                        stored.value = stored.value.wrapping_add(contribution);
                    } else {
                        let mut value = value.clone();
                        value.value = contribution;
                        groups[index].values.push(value);
                    }
                }
            }
        }
        let mut text = String::new();
        for group in groups {
            let values: Vec<_> = group
                .values
                .iter()
                .filter(|value| value.value != 0)
                .collect();
            if values.is_empty() {
                continue;
            }
            text.push_str("\u{11}Y");
            text.push_str(&self.localised(group.equipment.as_bytes(), &[], scopes, false)?);
            text.push_str("\u{11}!: ");
            if values.len() > 1 {
                text.push('\n');
            }
            for value in &values {
                if values.len() > 1 {
                    text.push_str("    ");
                }
                let index = installed_content_core::equipment::stat_index(value.name.as_bytes())
                    .expect("filtered equipment stat");
                text.push_str(&self.char_equipment_stat_difference(index, value.value, scopes)?);
                text.push('\n');
            }
        }
        if !text.is_empty() {
            text.push('\n');
        }
        Ok(text)
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
        // append LF; days affects execution, not this text.
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

    pub(super) fn static_modifier(
        &self,
        token: &str,
    ) -> Result<(Option<String>, Modifiers), String> {
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
        // target TARGET, static NAME, raw DESC, scoped wrapper.
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
                .scoped_scope(scalar_operand(operand)?, scopes)
                .map_err(|why| why.to_string())?
        } else {
            None
        };
        let selected_scopes = match selected {
            Some(Scope::Country(country) | Scope::CountryLeader { country, .. }) => {
                self.fresh_country_state_scope(country, 0)?
            }
            Some(Scope::State(state)) => self.fresh_country_state_scope(0, state)?,
            _ => scopes,
        };
        let mut name = self.localised(modifier.name.as_bytes(), &[], selected_scopes, true)?;
        if let Some(selected) = selected {
            let selected_name = match selected {
                Scope::Country(country) | Scope::CountryLeader { country, .. } => {
                    Some(self.country_name(country, false)?)
                }
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
        // formats each scoped entry in authored order.
        let mut scoped_lines = Vec::new();
        for (key, operand) in &modifier.scoped_modifiers {
            let value = self.scoped_value(operand, selected_scopes)?;
            let mut modifiers = Modifiers::default();
            modifiers.values.push(installed_content_core::modifiers::Modifier {
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
            let token = std::str::from_utf8(installed_content_core::parser::c_string(token))
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
        // compute days/province list but omit them from own text.
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
        let modifiers = installed_content_core::modifiers::values(fields);
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
            if !feature_enabled(frame, definition)? {
                continue;
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
            let effective =
                if definition.flags & engine_native::modifier_definition::CLAMP_MAXIMUM != 0 {
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
            // passes colour byte 1 and includes zero.
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
        // floors each stored component separately, then casts to i32.
        for (name, key, trailing) in [
            ("years", b"FREQUENCY_YEAR".as_slice(), true),
            ("months", b"FREQUENCY_MONTH".as_slice(), true),
            ("days", b"FREQUENCY_DAY".as_slice(), false),
        ] {
            let Some(item) = field(fields, name) else {
                continue;
            };
            let raw = self.scoped_value(item.value.scalar().unwrap_or_default(), scopes)?;
            let whole = timed_component_integer(raw);
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

    /// Expands the raw modifier list in the supplied event scope.
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

    /// raw text; its direct callers choose when to expand it.
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

    /// stored definition IDs and numeric-array order.
    pub(in crate::effect_description) fn cached_modifier_description_with(
        &self,
        entries: &[CachedModifier],
        prefix: &str,
        options: ModifierListOptions<'_>,
    ) -> Result<String, String> {
        self.cached_modifier_description(entries, &[], prefix, options)
    }

    /// occupied tooltip slots are stored string bytes,
    /// independent of authored traits and the cache's refresh history.
    pub(in crate::effect_description) fn advisor_cache_description(
        &self,
        row: &replay_data::rows::AdvisorRoleRow,
    ) -> Result<String, String> {
        let cache = row
            .modifier_cache
            .require("AdvisorRoles.modifier_cache")?
            .as_ref()
            .ok_or("AdvisorRoles.modifier_cache unavailable")?;
        let keys = row
            .tooltip_keys
            .require("AdvisorRoles.tooltip_keys")?
            .as_ref()
            .ok_or("AdvisorRoles.tooltip_keys capture unavailable")?;
        let tooltips = keys
            .iter()
            .map(|key| {
                std::str::from_utf8(installed_content_core::parser::c_string(key))
                    .map(str::to_owned)
                    .map_err(|_| "AdvisorRoles tooltip key is not UTF-8".to_owned())
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.cached_modifier_description(
            &cache.entries,
            &tooltips,
            "",
            ModifierListOptions {
                category_mask: u32::MAX,
                ..Default::default()
            },
        )
    }

    fn cached_modifier_description(
        &self,
        entries: &[CachedModifier],
        tooltips: &[String],
        prefix: &str,
        options: ModifierListOptions<'_>,
    ) -> Result<String, String> {
        if entries.is_empty() {
            return self.modifier_description_values(&[], tooltips, prefix, options);
        }
        let definitions = self
            .context
            .text
            .frame()
            .complete::<tables::ModifierDefinitions>()
            .map_err(|why| why.to_string())?;
        let values = entries
            .iter()
            .map(|entry| {
                let definition = definitions
                    .iter()
                    .find(|definition| definition.index == entry.definition)
                    .ok_or_else(|| {
                        format!(
                            "advisor cached modifier {} has no recorded definition",
                            entry.definition
                        )
                    })?;
                Ok((entry.value, definition))
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.modifier_description_values(&values, tooltips, prefix, options)
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
                "modifier outer colour byte {:02x} is not UTF-8",
                options.outer_colour
            ));
        }
        let frame = self.context.text.frame();
        let mut out = String::new();
        if options.outer_colour != 0 {
            // these bytes precede the first line's separator check.
            out.push('\u{11}');
            out.push(char::from(options.outer_colour));
        }
        for &(raw_value, definition) in values {
            // separate include/exclude intersections.
            if definition.category_mask & options.category_mask == 0
                || definition.category_mask & !options.excluded_category_mask == 0
            {
                continue;
            }
            // Clamp before wrapping multiplication and
            // signed /100000; tests the scaled zero.
            let effective =
                if definition.flags & engine_native::modifier_definition::CLAMP_MAXIMUM != 0 {
                    raw_value.clamp(0, 100_000)
                } else {
                    raw_value
                };
            let value = effective.wrapping_mul(options.scale) / 100_000;
            if value == 0 && definition.flags & 0x100 == 0 {
                continue;
            }
            if !feature_enabled(frame, definition)? {
                continue;
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
                // appends the separator as a NUL-terminated string.
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
            // LF only if needed, no numeric prefix.
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
            // appends the reset even when there were no lines.
            out.push_str("\u{11}!");
        }
        Ok(out)
    }
}

/// Whether `definition`'s feature selector, if it has one, is in the
/// recorded DLC mask.
fn feature_enabled(
    frame: &replay_frame::frame::DecodedFrame,
    definition: &ModifierDefinitionRow,
) -> Result<bool, String> {
    if definition.feature_selector == 0 {
        return Ok(true);
    }
    let [features] = frame
        .complete::<tables::SimulationFeatures>()
        .map_err(|why| why.to_string())?
    else {
        return Err("SimulationFeatures has no unique feature mask".into());
    };
    Ok(features.dlc_mask & definition.feature_selector != 0)
}

#[cfg(test)]
mod tests;
