use super::*;
use installed_content::mio_traits::TraitName;

impl Renderer<'_, '_, '_> {
    // CTraitTooltipEffect 1402FF8F0 / CUnlockTraitEffect 1402FFE10 call
    // 1413978B0; EFFECT_MIO_UNLOCK_TRAIT@1427315B0 and
    // EFFECT_MIO_COMPLETE_TRAIT@14272FCB8 substitute TRAIT, then expand.
    pub(in crate::effect_description) fn char_mio_trait(
        &mut self,
        item: &Item,
        scopes: Scopes,
        complete: bool,
    ) -> Result<String, String> {
        // ReadValue 141385770 or ReadKey 140301200 -> 141398390.
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
        let token = std::str::from_utf8(installed_content::parser::c_string(operand))
            .map_err(|_| "MIO trait token is not UTF-8")?;
        let world = self.context.world()?;
        let mut definition = None;
        let mut name = token.to_owned(); // LexerToken_CopyAssign 14015A1E0 keeps authored bytes.
        // 141397DB0 -> 140529390: missing MIO/ordinary trait returns the raw
        // lexer spelling, never localises it. Native setters clear other arms;
        // the shared graph retains each full handle.
        if let Scope::IndustrialOrganisation(reference) = scopes.this
            && reference != 0
        {
            let row = world.industrial_organisation(reference)?;
            let organisation = world
                .token(row.token)?
                .ok_or("LexicalTokens lacks industrial organisation template")?;
            let organisation = std::str::from_utf8(organisation)
                .map_err(|_| "industrial organisation template is not UTF-8")?;
            let catalogue = self
                .character_content()?
                .mio_traits
                .as_ref()
                .map_err(Clone::clone)?;
            if let Some(trait_) = catalogue.trait_definition(organisation, token)? {
                // GetLocalizedName 140A405B0 uses recorded lexer spellings
                // and owner country +0x120; the name itself stays plain.
                let owner = if row.owner == 0 {
                    0
                } else {
                    world
                        .slot(row.owner)
                        .ok_or("CountryLinks lacks MIO owner")?
                };
                let spelling = world
                    .token_of(token.as_bytes())?
                    .map(|id| world.token(id))
                    .transpose()?
                    .flatten()
                    .ok_or("LexicalTokens lacks MIO trait spelling")?;
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
        // Factories 1402F9CF0/1402F9E20 default +0xA0 show_modifiers to yes.
        let show = installed_content::parser::read_bool_field(children(item), "show_modifiers", true);
        if show && let Some(trait_) = definition {
            let class = if complete {
                "NIndustrialOrganisation::CUnlockTraitEffect"
            } else {
                "NIndustrialOrganisation::CTraitTooltipEffect"
            };
            // 1413978B0 appends one complete parenthetical to this header.
            let Some(body) = self.char_mio_modifiers(&trait_, scopes, class)? else {
                return Ok(String::new());
            };
            text.push_str(" ("); // 14270C624.
            text.push_str(&body);
            text.push(')'); // 14270BE94; even when the body is empty.
        }
        // 1413978B0: LF only for a nonempty result lacking terminal LF.
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        Ok(text)
    }

    // 140A3FB60(...,1): comma separator@14270EC00; no equipment-type or
    // organisation-name headings. 140634360 removes zero bonuses in stored
    // order. This raw body is appended AFTER the wrapper's scope expansion.
    fn char_mio_modifiers(
        &mut self,
        trait_: &TraitName,
        scopes: Scopes,
        class: &str,
    ) -> Result<Option<String>, String> {
        let mut parts = Vec::new();
        let mut refused = false;
        for bonus in trait_.bonuses.iter().filter(|bonus| bonus.value != 0) {
            if let Some(index) = installed_content::equipment::stat_index(bonus.name.as_bytes()) {
                let (_, label, positive) = installed_content::equipment::STAT_METADATA[index];
                let difference = format!("{label}_MOD_DIFF");
                let key: &[u8] = if self.context.localisation.get(&difference).is_some() {
                    difference.as_bytes()
                } else if positive {
                    b"STAT_MODIFIER_DIFF"
                } else {
                    b"STAT_MODIFIER_DIFF_NEG"
                };
                let label = self.char_native_text(label.as_bytes(), &[], scopes, false)?;
                let value = self.char_native_text(
                    key,
                    &[("VALUE", Argument::Fixed5(bonus.value))],
                    scopes,
                    false,
                )?;
                parts.push(format!("{label}{value}")); // 140630A10 -> 1413D0370.
            } else if installed_content::mio_traits::is_production_stat(&bonus.name) {
                // 140630A10 converts fixed5 to Q15. 141483BF0 chooses colour
                // and sign from Q15, then 142259470/1424B5FF0 use snprintf
                // rounding at precision 2; the shared fixed5 formatter differs.
                self.refuse(class, format!(
                    "MIO production bonus {} needs shared Q15 percentage formatter (140630A10/141483A20/141483BF0/142259470/1424B5FF0)",
                    bonus.name,
                ));
                refused = true;
            }
        }
        let mut text = parts.join(", ");
        let kept = if let Some(why) = &trait_.modifier_inheritance_refusal {
            self.refuse(class, why.clone());
            refused = true;
            false
        } else {
            match self.char_trait_modifier_kept(&trait_.organisation_modifiers) {
                Ok(kept) => kept,
                Err(why) => {
                    self.refuse(class, why);
                    refused = true;
                    false
                }
            }
        };
        if kept {
            match self.modifier_description_with(
                &trait_.organisation_modifiers,
                "",
                ModifierListOptions {
                    category_mask: u32::MAX,
                    separator: ", ",
                    ..Default::default()
                },
            ) {
                Ok(body) => {
                    // 140A3FEFE adds a separator before FormatDescription even if
                    // native visibility later makes its result empty.
                    if !text.is_empty() {
                        text.push_str(", ");
                    }
                    text.push_str(&body);
                }
                Err(why) => {
                    self.refuse(class, why);
                    refused = true;
                }
            }
        }
        if refused {
            return Ok(None);
        }
        Ok(Some(text.trim_matches([' ', '\t', '\n', '\r']).to_owned())) // 1424B7400; four bytes@142B7B148.
    }
}

#[cfg(test)]
mod tests;
