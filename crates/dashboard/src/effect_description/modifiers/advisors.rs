//! Advisor-backed idea lookup and stored description (1410D69C0/14128C670).
use super::*;
use replay::rows::{AdvisorRoleRow, CharacterRoleTraitRow, CharacterRow};

impl<'r, 'c, 'f> Renderer<'r, 'c, 'f> {
    pub(super) fn advisor_idea_character(
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
        // 1410D69C0: exact +0x30 string comparison, in +0x40 array order.
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
        // +0x28 retired records, then each character's +0xB0 tree order.
        // 140F8A630 folds the name (140BBA1B0/14253D9F8), unlike the primary.
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

    // 14128D5C0: template research; 14128C820: equipment from current traits;
    // 14128FF90 rebuilds +0x260 from those traits' ordered targeted blocks.
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
                format!(
                    "advisor {token} research template is absent from recorded content (14128D5C0)"
                )
            })?;
        let mut before = String::new();
        for bonus in &template.research_bonuses {
            before.push_str(&self.research_bonus_line(bonus, scopes)?);
            before.push('\n'); // 1413CA140 appends LF unconditionally.
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
                    // 14128C8E0 evaluates in an empty scope, then installs only
                    // that country; it does not inherit caller ROOT/FROM/PREV.
                    let country = target
                        .operand
                        .as_deref()
                        .map(|operand| self.evaluator()?.scoped_country(operand, Scopes::empty()))
                        .transpose()?
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
        // 14128C670 -> 14128BE00 resolves the role again by folded token,
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
            .ok_or("AdvisorRoles.modifier_cache unavailable (CAdvisor+0x1B0)")?;
        // 14128C8E0 calls raw 14054E050: precision 2, full mask, unit scale.
        let numeric = self.cached_modifier_description_with(
            &cache.entries,
            "",
            ModifierListOptions {
                category_mask: u32::MAX,
                ..Default::default()
            },
        )?;
        let class = if show {
            "CShowIdeasTooltipEffect"
        } else if remove {
            "CRemoveIdeasEffect"
        } else {
            "CAddIdeasEffect"
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
        self.refuse(
            class,
            "advisor cached tooltip keys +0x218 unrecorded until format 99".into(),
        );
        // 14128BE00: trim, LF -> comma, colon cleanup, four/two spaces -> one.
        let details = clean_modifiers(&details).replace("  ", " ");
        let name = self.effect_character_name(character)?;
        // 14128C670 always uses the detailed wrapper and appends LF;
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
