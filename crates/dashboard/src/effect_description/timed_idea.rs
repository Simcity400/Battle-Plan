use super::*;
use crate::{
    products::{
        identity,
        politics_content::{self, CountryLook, IdeaCountry, IdeaName},
    },
    triggers::{Scope, Scopes},
};
use installed_content::{localisation::crt_case_text, modifiers::Modifiers, scope::Scope as Parsed};
use replay::rows::tables;

fn clean_modifiers(text: &str) -> String {
    trim_description(text)
        .replace('\n', ", ")
        .replace(":,", ":")
        .replace(": ,", ":")
        .replace("    ", " ")
}

impl Renderer<'_, '_, '_> {
    // 140378970 concatenates four fragments, cleans them, then expands the wrapper.
    pub(super) fn timed_idea(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let fields = children(item);
        let token = field(fields, "idea")
            .and_then(|item| item.value.read_string())
            .ok_or("timed idea has no literal reference (140368FD0)")?;
        let token = std::str::from_utf8(token).map_err(|_| "timed idea reference is not UTF-8")?;
        let ideas = self
            .context
            .content
            .ideas
            .ok_or_else(|| self.content_gap("ideas"))?;
        let names = self
            .context
            .content
            .name_lists
            .ok_or_else(|| self.content_gap("name lists"))?;
        let Some(idea) = ideas.idea(token).map_err(|why| why.to_string())? else {
            return Err(format!("idea {token} is absent (140368FD0)"));
        };
        if !idea.research_bonuses.is_empty() {
            return Err(format!(
                "idea {token} research bonus description is outside reached form (140FBD640)"
            ));
        }
        if !idea.equipment_bonuses.is_empty() {
            return Err(format!(
                "idea {token} equipment bonus description is outside reached form (140FBB140)"
            ));
        }
        if !idea.traits.is_empty() {
            return Err(format!(
                "idea {token} trait description is outside reached form (140FBDA10)"
            ));
        }
        let country = match scopes.this {
            Scope::Country(country) => country,
            _ => return Err("timed idea scope has no country".into()),
        };
        let world = self.context.world()?;
        let frame = world.frame();
        let appearance = frame
            .complete::<tables::CountryAppearance>()
            .map_err(|why| why.to_string())?
            .iter()
            .find(|row| world.recorded().slot(row.country) == Some(country));
        let civil = identity::civil_war(frame, country);
        let look = match appearance {
            Some(row) => Some(CountryLook {
                name_tag: std::str::from_utf8(&row.name_tag)
                    .map_err(|_| "idea country name_tag is not UTF-8")?,
                cosmetic_tag: std::str::from_utf8(&row.cosmetic_tag)
                    .map_err(|_| "idea country cosmetic_tag is not UTF-8")?,
                civil_war: civil.as_ref().copied().map_err(String::as_str),
            }),
            None => None,
        };
        let name = politics_content::idea_name(
            ideas,
            names,
            self.context.localisation,
            token,
            IdeaCountry {
                tag: self.tag(country)?,
                slot: country,
                look,
            },
        )?;
        let name = match name {
            IdeaName::Key(key) => self.localised(key.as_bytes(), &[], scopes, true)?,
            IdeaName::Generated(name) => name,
        };
        let mut modifiers = self.idea_modifiers(&idea.modifiers, "", scopes)?;
        if !modifiers.is_empty() {
            modifiers.push('\n');
        }
        for target in &idea.targeted {
            let tag = target.tag.as_deref().ok_or_else(|| {
                format!(
                    "idea {token} dynamic targeted modifier is outside reached form (140FBC770)"
                )
            })?;
            let country = self.context.world()?.recorded().resolve_tag(tag)?;
            let selected = self
                .evaluator()?
                .enter_scope(&Parsed::Country(tag.to_owned()), scopes)?
                .ok_or_else(|| format!("idea {token} target {tag} names no country"))?;
            let definite = crt_case_text(&self.country_name(country, true)?, true, true);
            let prefix = self.localised(
                b"EFFECT_CONTEXT_COUNTRY",
                &[
                    ("TAG", Argument::Text(self.tag(country)?)),
                    ("COUNTRY", Argument::Text(&definite)),
                ],
                selected,
                false,
            )?;
            modifiers.push_str(&self.idea_modifiers(&target.modifiers, &prefix, selected)?);
            modifiers.push('\n');
        }
        let description = clean_modifiers(&modifiers);
        let duration = self.duration(fields, scopes)?;
        self.localised(
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
            let raw = self
                .evaluator()?
                .scoped_value(item.value.scalar().unwrap_or_default(), scopes)?;
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

    fn idea_modifiers(
        &self,
        modifiers: &Modifiers,
        prefix: &str,
        scopes: Scopes,
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
                Ok((modifier, definition))
            })
            .collect::<Result<Vec<_>, String>>()?;
        values.sort_by_key(|(_, definition)| definition.index);
        let mut out = String::new();
        for (modifier, definition) in values {
            if definition.category_mask & 0xff_ffff == 0
                || (modifier.value == 0 && definition.flags & 0x100 == 0)
            {
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
            let label = self.localised(
                &definition.name_key,
                &[("ARG", Argument::Text(&argument))],
                scopes,
                false,
            )?;
            let value = if definition.flags & engine::modifier_definition::CLAMP_MAXIMUM != 0 {
                modifier.value.clamp(0, 100_000)
            } else {
                modifier.value
            };
            let value =
                crate::modifier_text::value_text(definition, value, 2, false, false, |key, _| {
                    self.localised(key, &[], scopes, false)
                })?;
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(prefix);
            out.push_str(&label);
            out.push_str(": ");
            out.push_str(&value);
        }
        for tooltip in modifiers.tooltips() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(prefix);
            out.push_str(&self.localised(tooltip.as_bytes(), &[], scopes, false)?);
        }
        self.expanded(&out, scopes)
    }
}
