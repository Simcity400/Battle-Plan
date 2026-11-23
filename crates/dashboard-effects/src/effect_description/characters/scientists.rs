
use super::*;
use crate::scripted_text::{EventObject, EventScope};
use content_reader_parser::tokens::Token;

#[cfg(test)]
mod tests;

impl<'r, 'c, 'f> Renderer<'r, 'c, 'f> {
    pub(in crate::effect_description) fn char_scientist(
        &self,
        scopes: Scopes,
    ) -> Result<Option<Cow<'f, CharacterRow>>, String> {
        let Some(character) = self.effect_character(scopes.this)? else {
            return Ok(None);
        };
        if self
            .character_table::<tables::Scientists>(0)?
            .contains(u128::from(character.id))
        {
            return Ok(Some(character));
        }
        for row in each(self.character_table::<tables::CountryCharacters>(ROSTER)?) {
            if row?.character == Some(character.id) {
                return Ok(None);
            }
        }
        Err(format!(
            "unrostered character {} scientist holder is unrecorded",
            character.id
        ))
    }

    pub(in crate::effect_description) fn char_specialization_name(
        &self,
        operand: &[u8],
        country: u32,
        scopes: Scopes,
    ) -> Result<Option<String>, String> {
        let catalog = self
            .character_content()?
            .special_projects
            .as_ref()
            .map_err(Clone::clone)?;
        let token = Token::c_str(operand);
        let Some(name) = catalog.specialization(&token)? else {
            return Ok(None);
        };
        let key = if country as i32 > 0 {
            let slot = self
                .context
                .world()?
                .slot(country)
                .ok_or_else(|| format!("country links lacks scientist country {country}"))?;
            let candidate = format!("{}_{}", self.tag(slot)?, name);
            if self.context.localisation.get(&candidate).is_some() {
                candidate
            } else {
                name.to_owned()
            }
        } else {
            name.to_owned()
        };
        self.localised(key.as_bytes(), &[], scopes, false).map(Some)
    }

    pub(in crate::effect_description) fn char_add_scientist_xp(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        self.char_scientist_change(item, scopes, false)
    }

    pub(in crate::effect_description) fn char_add_scientist_level(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        self.char_scientist_change(item, scopes, true)
    }

    fn char_scientist_change(
        &self,
        item: &Item,
        scopes: Scopes,
        levels: bool,
    ) -> Result<String, String> {
        let Some(character) = self.char_scientist(scopes)? else {
            return Ok(String::new());
        };
        let specialization = field(children(item), "specialization")
            .and_then(|field| field.value.scalar())
            .unwrap_or_default();
        let Some(specialization) =
            self.char_specialization_name(specialization, character.country, scopes)?
        else {
            return Ok(String::new());
        };
        let value = field(children(item), if levels { "level" } else { "experience" })
            .and_then(|field| field.value.scalar())
            .map(|operand| self.scoped_value(operand, scopes))
            .transpose()?
            .unwrap_or_default();
        let text = self.char_native_text(
            if levels {
                b"SCIENTIST_ADD_LEVEL_SPECIALIZATION_EFFECT"
            } else {
                b"SCIENTIST_ADD_XP_SPECIALIZATION_EFFECT"
            },
            &[
                (
                    if levels { "LEVELS" } else { "XP" },
                    Argument::Fixed5(value),
                ),
                ("SPECIALIZATION", Argument::Text(&specialization)),
            ],
            scopes,
            false,
        )?;
        if levels {
            Ok(text)
        } else {
            Ok(format!("{} {text}", self.effect_character_name(&character)?))
        }
    }

    pub(in crate::effect_description) fn char_injure_scientist(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Some(character) = self.char_scientist(scopes)? else {
            return Ok(String::new());
        };
        let days = content_reader_parser::fixed5::decimal_i32(item.value.scalar().unwrap_or_default());
        let at = {
            let mut graph = self.graph.borrow_mut();
            let at = graph.scopes.len();
            graph
                .scopes
                .push(EventScope::of(EventObject::Character(character.id)));
            at
        };
        let selected = self
            .evaluator()?
            .event_scopes(at)
            .map_err(|why| why.to_string())?;
        self.char_native_line(
            b"SPECIAL_PROJECT_INJURE_SCIENTIST_FOR_DAYS",
            &[("DAYS", Argument::Int32(days))],
            selected,
            true,
        )
    }

    pub(in crate::effect_description) fn char_add_scientist_trait(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        if item.kind != content_reader_script::native::Kind::Bare || item.number() {
            return Ok(String::new());
        }
        let token = item.value.c_str().map_err(|why| why.to_string())?;
        let traits = self
            .character_content()?
            .scientist_traits
            .as_ref()
            .map_err(Clone::clone)?;
        let Some(trait_) = traits.get(token)? else {
            return Ok(String::new());
        };
        if self.char_scientist(scopes)?.is_none() {
            return Ok(String::new());
        }
        if let Some(why) = &trait_.refusal {
            return Err(why.clone());
        }
        let key = trait_.display_key()?;
        let name = self.localised(key.as_bytes(), &[], scopes, false)?;
        let mut text = self.char_native_line(
            b"SCIENTIST_ADD_TRAIT_EFFECT",
            &[("TRAIT_NAME", Argument::Text(&name))],
            scopes,
            false,
        )?;
        let modifiers = self.modifier_list_with(
            &trait_.modifiers,
            "",
            scopes,
            ModifierListOptions {
                category_mask: u32::MAX,
                ..Default::default()
            },
        )?;
        text.push_str(&indent_nonempty_lines(&modifiers, 2));
        Ok(text)
    }

    pub(in crate::effect_description) fn char_generate_scientist(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let mut names = Vec::new();
        if let Some(list) = field(children(item), "traits") {
            for entry in list.value.items().unwrap_or_default() {
                let token = entry.value.c_str().map_err(|why| why.to_string())?;
                let traits = self
                    .character_content()?
                    .scientist_traits
                    .as_ref()
                    .map_err(Clone::clone)?;
                let name = if let Some(trait_) = traits.get(token)? {
                    let key = trait_.display_key()?;
                    self.localised(key.as_bytes(), &[], scopes, false)?
                } else {
                    String::new()
                };
                names.push(self.char_native_text(
                    b"EFFECT_GENERATE_SCIENTIST_CHARACTER_TRAIT",
                    &[("TRAIT_NAME", Argument::Text(&name))],
                    scopes,
                    false,
                )?);
            }
        }
        let joined = names.join(", ");
        let traits = if joined.is_empty() {
            joined
        } else {
            format!("({joined})")
        };
        self.char_native_text(
            b"EFFECT_GENERATE_SCIENTIST_CHARACTER",
            &[("TRAITS", Argument::Text(&traits))],
            scopes,
            true,
        )
    }

    pub(in crate::effect_description) fn char_add_scientist_role(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let operand = field(children(item), "character").and_then(|field| field.value.scalar());
        let Some(character) = self.effect_character_target(operand, scopes)? else {
            return Ok(String::new());
        };
        let name = self.effect_character_name(&character)?;
        self.char_native_text(
            b"EFFECT_ADD_SCIENTIST_ROLE",
            &[("CHARACTER", Argument::Text(&name))],
            scopes,
            true,
        )
    }

    pub(in crate::effect_description) fn char_remove_scientist_role(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let operand = item
            .value
            .scalar()
            .or_else(|| field(children(item), "character").and_then(|field| field.value.scalar()));
        let Some(character) = self.effect_character_target(operand, scopes)? else {
            return Ok(String::new());
        };
        let name = self.effect_character_name(&character)?;
        self.char_native_text(
            b"EFFECT_REMOVE_SCIENTIST_ROLE",
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        )
    }
}
