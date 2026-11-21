use super::*;
use replay_data::rows::{
    AdvisorRoleRow, AvailableAdvisorRow, CharacterRoleTraitRow, CharacterRow,
    CountryCharacterRow,
};

const AVAILABLE: u64 = mask::<AvailableAdvisorRow>(&["idea", "character"]);
const RETIRED: u64 = mask::<CountryCharacterRow>(&["character"]);
const ROLE_IDEA: u64 = mask::<AdvisorRoleRow>(&["idea"]);
const ROLE_CACHE: u64 = mask::<AdvisorRoleRow>(&["idea", "modifier_cache", "tooltip_keys"]);
const DEFINITIONS_COLUMN: usize =
    replay_data::rows::column::<CharacterRoleTraitRow>("definitions");

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
        let frame = self.tables();
        let (low, high) = (u128::from(raw) << 32, u128::from(raw) << 32 | u128::from(u32::MAX));
        let available = frame
            .complete::<tables::AvailableAdvisors>(AVAILABLE)
            .map_err(|why| why.to_string())?;
        if let Some(row) = find(available.span(low, high), |row| row.idea == token.as_bytes())? {
            return Ok(Some(row.character));
        }
        let retired = each(
            frame
                .complete::<tables::CountryRetiredCharacters>(RETIRED)
                .map_err(|why| why.to_string())?
                .span(low, high),
        )
        .collect::<Result<Vec<_>, _>>()?;
        if retired.is_empty() {
            return Ok(None);
        }
        let roles = frame
            .complete::<tables::AdvisorRoles>(ROLE_IDEA)
            .map_err(|why| why.to_string())?;
        for row in retired {
            let Some(character) = row.character else {
                return Err(format!(
                    "country retired characters advisor record {}:{} has no character",
                    raw, row.ordinal
                ));
            };
            let low = u128::from(character) << 32;
            if find(roles.span(low, low | u128::from(u32::MAX)), |role| {
                role.idea.eq_ignore_ascii_case(token.as_bytes())
            })?
            .is_some()
            {
                return Ok(Some(character));
            }
        }
        Ok(None)
    }

    fn advisor_idea_fragments(
        &self,
        character: &CharacterRow,
        role: &AdvisorRoleRow,
        scopes: Scopes,
    ) -> Result<(String, String), String> {
        let frame = self.tables();
        let token =
            std::str::from_utf8(&role.idea).map_err(|_| "advisor idea token is not UTF-8")?;
        let built =
            identity::built_template_in(character, frame.complete::<tables::LexicalTokens>(0).ok());
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
            before.push('\n');
        }
        let key = u128::from(character.id) << 40
            | u128::from(CharacterRoleTraitRow::ADVISOR) << 32
            | u128::from(role.ordinal);
        let traits = frame
            .complete::<tables::CharacterRoleTraits>(0)
            .map_err(|why| why.to_string())?
            .u32s(key, |row| &row.definitions, DEFINITIONS_COLUMN)
            .map_err(crate::effect_description::built)?;
        let mut targets = String::new();
        if let Some(traits) = traits {
            let catalogue = self.char_country_traits()?;
            for &ordinal in traits {
                let trait_ = catalogue.by_ordinal(ordinal).ok_or_else(|| {
                    format!("advisor trait definition {ordinal} is absent from recorded content")
                })?;
                before.push_str(&self.char_equipment_bonuses(
                    &trait_.equipment_bonuses,
                    "",
                    scopes,
                )?);
                for target in &trait_.targeted {
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
        let frame = self.tables();
        let low = u128::from(character.id) << 32;
        let role = find(
            frame
                .complete::<tables::AdvisorRoles>(ROLE_CACHE)
                .map_err(|why| why.to_string())?
                .span(low, low | u128::from(u32::MAX)),
            |row| row.idea.eq_ignore_ascii_case(token.as_bytes()),
        )?
        .ok_or_else(|| format!("advisor roles lacks character {} idea {token}", character.id))?;
        let cache = role
            .modifier_cache
            .require("modifier cache of advisor roles")?
            .as_ref()
            .ok_or("modifier cache of advisor roles unavailable")?;
        let class = if show {
            "idea tooltip"
        } else if remove {
            "idea removal"
        } else {
            "idea addition"
        };
        let numeric = match role
            .tooltip_keys
            .require("tooltip keys of advisor roles")
            .and_then(|keys| {
                keys.as_ref()
                    .ok_or_else(|| "tooltip keys of advisor roles capture unavailable".to_owned())
            }) {
            Ok(_) => self.advisor_cache_description(&role)?,
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
        let (mut details, targets) = match self.advisor_idea_fragments(character, &role, scopes) {
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
        let details = clean_modifiers(&details, true).replace("  ", " ");
        let name = self.effect_character_name(&character)?;
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
