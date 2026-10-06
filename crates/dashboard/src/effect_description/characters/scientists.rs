//! Scientist-effect describers and their content-backed display names.

use super::*;
use crate::scripted_text::{EventObject, EventScope};
use installed_content::tokens::Token;

#[cfg(test)]
mod tests;

impl<'r, 'c, 'f> Renderer<'r, 'c, 'f> {
    // 1404909E0 requires CCharacter+0xC0. The active roster is kept in step
    // with that holder by ProcessCharacterStatus 1410DAE50 -> 1410DAC10.
    // An unrostered character's holder is not captured, so absence of a
    // Scientists row alone cannot prove the native null-pointer gate.
    fn char_scientist(&self, scopes: Scopes) -> Result<Option<&'f CharacterRow>, String> {
        let Some(character) = self.effect_character(scopes.this)? else {
            return Ok(None);
        };
        if self
            .character_table::<tables::Scientists>()?
            .iter()
            .any(|row| row.character == character.id)
        {
            return Ok(Some(character));
        }
        if self
            .character_table::<tables::CountryCharacters>()?
            .iter()
            .any(|row| row.character == Some(character.id))
        {
            return Ok(None);
        }
        Err(format!(
            "unrostered character {} scientist holder (CCharacter+0xC0, 1404909E0) is unrecorded",
            character.id
        ))
    }

    // LocalizeProjectSpecializationName 140AABD40 -> 14146B5D0 -> 14146B1D0:
    // TAG_token when that localisation exists, otherwise the token. No scope
    // expansion occurs in this name or the XP/level wrapper.
    fn char_specialization_name(
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
                .ok_or_else(|| format!("CountryLinks lacks scientist country {country}"))?;
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

    // Own 140493CF0; XP@1427A99D8, SPECIALIZATION@1427A98C0,
    // SCIENTIST_ADD_XP_SPECIALIZATION_EFFECT@1427A99E0. The character name
    // (+0x28, 140612C90) and one space precede the wrapper, with no LF.
    pub(in crate::effect_description) fn char_add_scientist_xp(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        self.char_scientist_change(item, scopes, false)
    }

    // Own 140493630; LEVELS@1427A98D0,
    // SCIENTIST_ADD_LEVEL_SPECIALIZATION_EFFECT@1427A98D8. Unlike XP this
    // body emits only the wrapper, with neither character name nor LF.
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
            Ok(format!("{} {text}", self.effect_character_name(character)?))
        }
    }

    // Own 140494B30; DAYS@14275179C and
    // SPECIAL_PROJECT_INJURE_SCIENTIST_FOR_DAYS@1427A9C18. Native expansion
    // uses a fresh default scope set to the character (140528800/14052DAF0),
    // so ROOT, FROM and PREV are that character, not the calling event.
    pub(in crate::effect_description) fn char_injure_scientist(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Some(character) = self.char_scientist(scopes)? else {
            return Ok(String::new());
        };
        let days = installed_content::fixed5::decimal_i32(item.value.scalar().unwrap_or_default()); // Reader 140496070 -> 1424B0080.
        let at = {
            let mut graph = self.graph.borrow_mut();
            let at = graph.scopes.len();
            graph
                .scopes
                .push(EventScope::of(EventObject::Character(character.id)));
            at
        };
        let selected = self.evaluator()?.event_scopes(at)?;
        self.char_native_line(
            b"SPECIAL_PROJECT_INJURE_SCIENTIST_FOR_DAYS",
            &[("DAYS", Argument::Int32(days))],
            selected,
            true,
        )
    }

    // Own 1404938C0; SCIENTIST_ADD_TRAIT_EFFECT@1427A9B00 and
    // TRAIT_NAME@1427557B0. Reader 140495DC0 marks only a found trait valid.
    // Name 140A92180 is localised without expansion; modifiers +0x48 use
    // FormatScopedModifierDescription 14054E520 then two spaces (1424B7850).
    pub(in crate::effect_description) fn char_add_scientist_trait(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        // 140495DC0 looks up the held token ID, unlike ReadTokenId. Quoted
        // and numeric values have the lexer's reserved 0xF/0xC identities.
        if item.kind != installed_content::parser::native::Kind::Bare || item.number() {
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
        let modifiers = self.char_scoped_modifiers(&trait_.modifiers, "", scopes, u32::MAX)?;
        text.push_str(&indent_nonempty_lines(&modifiers, 2));
        Ok(text)
    }

    // Own 1403811F0; EFFECT_GENERATE_SCIENTIST_CHARACTER@1427557F8,
    // TRAITS@1427557EC. 1403396F0 -> 140326D70 preserves authored trait
    // order, joins the wrappers with ", " and wraps nonempty output in ().
    // Portrait, gender and starting skills are execution inputs only.
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

    // Own 140377B00: CHARACTER@142755440 and
    // EFFECT_ADD_SCIENTIST_ROLE@142755450; ResolveCharacter 140536B10,
    // then current character fallback, no scientist-role existence gate.
    pub(in crate::effect_description) fn char_add_scientist_role(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let operand = field(children(item), "character").and_then(|field| field.value.scalar());
        let Some(character) = self.effect_character_target(operand, scopes)? else {
            return Ok(String::new());
        };
        let name = self.effect_character_name(character)?;
        self.char_native_text(
            b"EFFECT_ADD_SCIENTIST_ROLE",
            &[("CHARACTER", Argument::Text(&name))],
            scopes,
            true,
        )
    }

    // Own 14038B6E0: NAME@142727E7C,
    // EFFECT_REMOVE_SCIENTIST_ROLE@1427555B8; no LF or scientist gate.
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
        let name = self.effect_character_name(character)?;
        self.char_native_text(
            b"EFFECT_REMOVE_SCIENTIST_ROLE",
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        )
    }
}
