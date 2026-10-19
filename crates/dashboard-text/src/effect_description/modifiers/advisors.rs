//! Advisor-backed idea lookup and stored description.
use super::*;
use replay_data::{rows::AdvisorRoleRow, rows::CharacterRoleTraitRow, rows::CharacterRow};

impl<'r, 'c, 'f> Renderer<'r, 'c, 'f> {
    pub(in crate::effect_description) fn advisor_idea_character(
        &self,
        token: &str,
        scopes: Scopes,
    ) -> Result<Option<u32>, String> {
        let world = self.context.world()?;
        let country = self.current_country(scopes)?;
        let raw = world
            .raw_country(country)
            .ok_or_else(|| format!("Countries lacks advisor country {country}"))?;
        let frame = self.context.text.frame();
        // exact string comparison, in array order.
        let available = frame
            .complete::<tables::AvailableAdvisors>()
            .map_err(|why| why.to_string())?;
        if let Some(row) = available
            .iter()
            .filter(|row| row.country == raw && row.idea == token.as_bytes())
            .min_by_key(|row| row.ordinal)
        {
            return Ok(Some(row.character));
        }
        // retired records, then each character's tree order.
        // folds the name, unlike the primary.
        let mut retired: Vec<_> = frame
            .complete::<tables::CountryRetiredCharacters>()
            .map_err(|why| why.to_string())?
            .iter()
            .filter(|row| row.country == raw)
            .collect();
        retired.sort_by_key(|row| row.ordinal);
        if retired.is_empty() {
            return Ok(None);
        }
        let roles = frame
            .complete::<tables::AdvisorRoles>()
            .map_err(|why| why.to_string())?;
        for row in retired {
            let Some(character) = row.character else {
                return Err(format!(
                    "CountryRetiredCharacters advisor record {}:{} has no character",
                    raw, row.ordinal
                ));
            };
            if roles.iter().any(|role| {
                role.character == character && role.idea.eq_ignore_ascii_case(token.as_bytes())
            }) {
                return Ok(Some(character));
            }
        }
        Ok(None)
    }

    // template research: equipment from current traits;
    // rebuilds from those traits' ordered targeted blocks.
    fn advisor_idea_fragments(
        &self,
        character: &CharacterRow,
        role: &AdvisorRoleRow,
        scopes: Scopes,
    ) -> Result<(String, String), String> {
        let frame = self.context.text.frame();
        let token =
            std::str::from_utf8(&role.idea).map_err(|_| "advisor idea token is not UTF-8")?;
        let built =
            identity::built_template(character, frame.complete::<tables::LexicalTokens>().ok());
        let template = self
            .character_content()?
            .catalogue
            .as_ref()
            .map_err(Clone::clone)?
            .held_advisor(token, built.as_ref().map_err(String::as_str))?
            .ok_or_else(|| {
                format!("advisor {token} research template is absent from recorded content")
            })?;
        let mut before = String::new();
        for bonus in &template.research_bonuses {
            before.push_str(&self.research_bonus_line(bonus, scopes)?);
            before.push('\n'); // appends LF unconditionally.
        }
        let traits = frame
            .complete::<tables::CharacterRoleTraits>()
            .map_err(|why| why.to_string())?
            .iter()
            .find(|row| {
                row.character == character.id
                    && row.kind == CharacterRoleTraitRow::ADVISOR
                    && row.role == role.ordinal
            });
        let mut targets = String::new();
        if let Some(traits) = traits {
            let catalogue = self.char_country_traits()?;
            for &ordinal in &traits.definitions {
                let trait_ = catalogue.by_ordinal(ordinal).ok_or_else(|| {
                    format!("advisor trait definition {ordinal} is absent from recorded content")
                })?;
                before.push_str(&self.char_equipment_bonuses(
                    &trait_.equipment_bonuses,
                    "",
                    scopes,
                )?);
                for target in &trait_.targeted {
                    // evaluates in an empty scope, then installs only
                    // that country; it does not inherit caller ROOT/FROM/PREV.
                    let country = target
                        .operand
                        .as_deref()
                        .map(|operand| self.evaluator()?.scoped_country(operand, Scopes::empty()))
                        .transpose()
                        .map_err(|why| why.to_string())?
                        .flatten()
                        .unwrap_or(0);
                    let selected = self.fresh_country_state_scope(country, 0)?;
                    let prefix = if country > 0 {
                        let name = crt_case_text(&self.country_name(country, true)?, true, true);
                        self.char_native_text(
                            b"EFFECT_CONTEXT_COUNTRY",
                            &[
                                ("TAG", Argument::Text(self.tag(country)?)),
                                ("COUNTRY", Argument::Text(&name)),
                            ],
                            selected,
                            false,
                        )?
                    } else {
                        String::new()
                    };
                    newline(&mut targets);
                    targets.push_str(&self.modifier_list_with(
                        &target.modifiers,
                        &prefix,
                        selected,
                        Default::default(),
                    )?);
                }
            }
        }
        Ok((before, targets))
    }

    pub(super) fn advisor_idea_tooltip(
        &mut self,
        character: &CharacterRow,
        token: &str,
        scopes: Scopes,
        remove: bool,
        show: bool,
    ) -> Result<String, String> {
        // resolves the role again by folded token,
        // in tree order, even when availability selected a later role.
        let frame = self.context.text.frame();
        let role = frame
            .complete::<tables::AdvisorRoles>()
            .map_err(|why| why.to_string())?
            .iter()
            .filter(|row| {
                row.character == character.id && row.idea.eq_ignore_ascii_case(token.as_bytes())
            })
            .min_by_key(|row| row.ordinal)
            .ok_or_else(|| format!("AdvisorRoles lacks character {} idea {token}", character.id))?;
        let cache = role
            .modifier_cache
            .require("AdvisorRoles.modifier_cache")?
            .as_ref()
            .ok_or("AdvisorRoles.modifier_cache unavailable")?;
        let class = if show {
            "idea tooltip"
        } else if remove {
            "idea removal"
        } else {
            "idea addition"
        };
        // calls raw: stored numbers and occupied tooltip
        // slots, precision 2, full mask, unit scale. Older/unavailable keys
        // leave the numeric fragment readable and retain a counted refusal.
        let numeric = match role
            .tooltip_keys
            .require("AdvisorRoles.tooltip_keys")
            .and_then(|keys| {
                keys.as_ref()
                    .ok_or_else(|| "AdvisorRoles.tooltip_keys capture unavailable".to_owned())
            }) {
            Ok(_) => self.advisor_cache_description(role)?,
            Err(why) => {
                self.refuse(class, why);
                self.cached_modifier_description_with(
                    &cache.entries,
                    "",
                    ModifierListOptions {
                        category_mask: u32::MAX,
                        ..Default::default()
                    },
                )?
            }
        };
        let (mut details, targets) = match self.advisor_idea_fragments(character, role, scopes) {
            Ok(fragments) => fragments,
            Err(why) => {
                self.refuse(class, why);
                (String::new(), String::new())
            }
        };
        newline(&mut details);
        details.push_str(&numeric);
        newline(&mut details);
        details.push_str(&targets);
        // trim, LF -> comma, colon cleanup, four/two spaces -> one.
        let details = clean_modifiers(&details, true).replace("  ", " ");
        let name = self.effect_character_name(character)?;
        // always uses the detailed wrapper and appends LF;
        // scope expansion belongs to the add/remove caller, not this helper.
        self.char_native_line(
            b"EFFECT_SHOW_IDEA_TOOLTIP_DETAILED",
            &[
                ("NAME", Argument::Text(&name)),
                ("DESC", Argument::Text(&details)),
            ],
            scopes,
            false,
        )
    }
}

#[cfg(test)]
mod tests;
