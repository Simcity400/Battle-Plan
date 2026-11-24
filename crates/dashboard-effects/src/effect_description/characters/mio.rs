use super::*;
use content_reader_military::mio_traits::TraitName;

impl Renderer<'_, '_, '_> {
    pub(in crate::effect_description) fn char_mio_trait(
        &mut self,
        item: &Item,
        scopes: Scopes,
        complete: bool,
    ) -> Result<String, String> {
        let operand = item
            .value
            .scalar()
            .or_else(|| {
                children(item)
                    .iter()
                    .rev()
                    .find(|field| field.keyword_is("trait"))
                    .and_then(|field| field.value.scalar())
            })
            .unwrap_or_default();
        let token = std::str::from_utf8(content_reader_parser::parser::c_string(operand))
            .map_err(|_| "MIO trait token is not UTF-8")?;
        let world = self.context.world()?;
        let mut definition = None;
        let mut name = token.to_owned();
        if let Scope::IndustrialOrganisation(reference) = scopes.this
            && reference != 0
        {
            let row = world
                .industrial_organisation(reference)
                .map_err(|why| why.to_string())?;
            let organisation = world
                .token(row.token)
                .map_err(|why| why.to_string())?
                .ok_or("lexical tokens lacks industrial organisation template")?;
            let organisation = std::str::from_utf8(organisation)
                .map_err(|_| "industrial organisation template is not UTF-8")?;
            let catalogue = self
                .character_content()?
                .mio_traits
                .as_ref()
                .map_err(Clone::clone)?;
            if let Some(trait_) = catalogue.trait_definition(organisation, token)? {
                let owner = if row.owner == 0 {
                    0
                } else {
                    world
                        .slot(row.owner)
                        .ok_or("country links lacks MIO owner")?
                };
                let spelling = world
                    .token_of(token.as_bytes())
                    .map_err(|why| why.to_string())?
                    .map(|id| world.token(id))
                    .transpose()
                    .map_err(|why| why.to_string())?
                    .flatten()
                    .ok_or("lexical tokens lacks MIO trait spelling")?;
                let spelling =
                    std::str::from_utf8(spelling).map_err(|_| "MIO trait spelling is not UTF-8")?;
                let candidates = trait_.name_candidates(
                    organisation,
                    spelling,
                    (owner > 0).then(|| self.tag(owner)).transpose()?,
                );
                let key = candidates
                    .iter()
                    .find(|key| self.context.localisation.get(key).is_some())
                    .unwrap_or_else(|| candidates.last().expect("base trait name"));
                name = self.localised(key.as_bytes(), &[], scopes, false)?;
                definition = Some(trait_.clone());
            }
        }
        let mut text = self.char_native_text(
            if complete {
                b"EFFECT_MIO_COMPLETE_TRAIT"
            } else {
                b"EFFECT_MIO_UNLOCK_TRAIT"
            },
            &[("TRAIT", Argument::Text(&name))],
            scopes,
            true,
        )?;
        let show = content_reader_parser::parser::read_bool_field(children(item), "show_modifiers", true);
        if show && let Some(trait_) = definition {
            let class = if complete {
                "MIO trait completion"
            } else {
                "MIO trait tooltip"
            };
            let body = self.char_mio_modifiers(&trait_, scopes, class);
            text.push_str(" (");
            text.push_str(&body);
            text.push(')');
        }
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        Ok(text)
    }

    fn char_mio_modifiers(&mut self, trait_: &TraitName, scopes: Scopes, class: &str) -> String {
        let mut text = self.char_mio_bonus_list(&trait_.bonuses, scopes, class);
        let modifiers = match trait_
            .resolved_organisation_modifiers(|name| self.char_trait_modifier_name_eligible(name))
        {
            Ok(modifiers) => modifiers,
            Err(reason) => {
                self.refuse(class, reason);
                return text;
            }
        };
        let kept = {
            match self.char_trait_modifier_kept(&modifiers) {
                Ok(kept) => kept,
                Err(why) => {
                    self.refuse(class, why);
                    false
                }
            }
        };
        if kept {
            match self.modifier_description_with(
                &modifiers,
                "",
                ModifierListOptions {
                    category_mask: u32::MAX,
                    separator: ", ",
                    ..Default::default()
                },
            ) {
                Ok(body) => {
                    if !text.is_empty() {
                        text.push_str(", ");
                    }
                    text.push_str(&body);
                }
                Err(why) => {
                    self.refuse(class, why);
                }
            }
        }
        text.trim_matches([' ', '\t', '\n', '\r']).to_owned()
    }

    fn char_mio_bonus_list(
        &mut self,
        bonuses: &[content_reader_military::mio_traits::Bonus],
        scopes: Scopes,
        class: &str,
    ) -> String {
        let mut parts = Vec::new();
        for bonus in bonuses.iter().filter(|bonus| bonus.value != 0) {
            match self.char_mio_bonus(bonus, scopes) {
                Ok(text) if !text.is_empty() => parts.push(text),
                Ok(_) => {}
                Err(why) => self.refuse(class, why),
            }
        }
        parts.join(", ")
    }

    fn char_mio_bonus(
        &self,
        bonus: &content_reader_military::mio_traits::Bonus,
        scopes: Scopes,
    ) -> Result<String, String> {
        if let Some(index) = content_reader_script::equipment_stats::stat_index(bonus.name.as_bytes()) {
            self.char_equipment_stat_difference(index, bonus.value, scopes)
        } else if content_reader_military::mio_traits::is_production_stat(&bonus.name) {
            let key = format!("BONUS_{}", bonus.name.to_ascii_uppercase());
            let label = self.char_native_text(key.as_bytes(), &[], scopes, false)?;
            let bad_positive = matches!(
                bonus.name.as_str(),
                "production_cost_factor"
                    | "production_resource_need_factor"
                    | "production_resource_penalty_factor"
            );
            Ok(format!(
                "{label}: {}",
                mio_production_percent(bonus.value, bad_positive)
            ))
        } else {
            Ok(String::new())
        }
    }

    pub(in crate::effect_description) fn char_mio_lines(
        &mut self,
        text: MioText<'_>,
        scopes: Scopes,
    ) -> Vec<Result<String, String>> {
        let class = "MIO window";
        let lines = |text: Result<String, String>| -> Vec<Result<String, String>> {
            match text {
                Ok(text) => text
                    .split('\n')
                    .map(|line| line.trim_matches([' ', '\t', '\r']))
                    .filter(|line| !line.is_empty())
                    .map(|line| Ok(line.to_owned()))
                    .collect(),
                Err(why) => vec![Err(why)],
            }
        };
        let listed = |renderer: &Self, modifiers: &content_reader_script::modifiers::Modifiers| {
            renderer.modifier_description_with(
                modifiers,
                "",
                ModifierListOptions {
                    category_mask: u32::MAX,
                    separator: "\n",
                    ..Default::default()
                },
            )
        };
        let out = match text {
            MioText::Bonuses(bonuses) => bonuses
                .iter()
                .filter(|bonus| bonus.value != 0)
                .map(|bonus| self.char_mio_bonus(bonus, scopes))
                .filter(|line| line.as_ref().map_or(true, |text| !text.is_empty()))
                .collect(),
            MioText::Modifiers(modifiers) => match self.char_trait_modifier_kept(modifiers) {
                Ok(true) => lines(listed(self, modifiers)),
                Ok(false) => Vec::new(),
                Err(why) => vec![Err(why)],
            },
            MioText::TraitModifiers(trait_) => {
                match trait_.resolved_organisation_modifiers(|name| {
                    self.char_trait_modifier_name_eligible(name)
                }) {
                    Ok(modifiers) => match self.char_trait_modifier_kept(&modifiers) {
                        Ok(true) => lines(listed(self, &modifiers)),
                        Ok(false) => Vec::new(),
                        Err(why) => vec![Err(why)],
                    },
                    Err(why) => vec![Err(why)],
                }
            }
        };
        for line in &out {
            if let Err(why) = line {
                self.refuse(class, why.clone());
            }
        }
        out
    }

    pub(in crate::effect_description) fn char_mio_policy_modifiers(
        &mut self,
        policy: &content_reader_military::industrial_organisations::policies::Policy,
        scopes: Scopes,
    ) -> String {
        let class = "MIO policy tooltip";
        let mut parts = Vec::new();
        for (group, bonuses) in &policy.groups {
            let name = match self.localised(group, &[], scopes, false) {
                Ok(name) => name,
                Err(why) => {
                    self.refuse(class, why);
                    continue;
                }
            };
            let body = self.char_mio_bonus_list(bonuses, scopes, class);
            parts.push(format!("\u{11}Y{name}\u{11}! {body}"));
        }
        let mut text = parts.join(", ");
        let kept = match self.char_trait_modifier_kept(&policy.organisation_modifiers) {
            Ok(kept) => kept,
            Err(why) => {
                self.refuse(class, why);
                false
            }
        };
        if kept {
            match self.modifier_description_with(
                &policy.organisation_modifiers,
                "",
                ModifierListOptions {
                    category_mask: u32::MAX,
                    separator: ", ",
                    ..Default::default()
                },
            ) {
                Ok(body) => {
                    if !text.is_empty() {
                        text.push_str(", ");
                    }
                    text.push_str(&body);
                }
                Err(why) => self.refuse(class, why),
            }
        }
        text.trim_matches([' ', '\t', '\n', '\r'])
            .replace('\n', ", ")
    }
}

fn mio_production_percent(value: i64, bad_positive: bool) -> String {
    let value = replay_data::fixed_point::fixed5_to_q15(value);
    let (sign, colour) = match value.signum() {
        1 => ('+', if bad_positive { 'R' } else { 'G' }),
        -1 => ('-', if bad_positive { 'G' } else { 'R' }),
        _ => (if bad_positive { '-' } else { '+' }, 'G'),
    };
    let percent = value.wrapping_abs().wrapping_mul(0x320000) >> 15;
    format!(
        "\u{11}{colour}{sign}{:.2}%\u{11}!",
        percent as f64 / 32768.0
    )
}

#[cfg(test)]
mod tests;
