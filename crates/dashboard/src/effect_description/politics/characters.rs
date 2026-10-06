//! Character operands and names shared by their native effect descriptions.

use super::*;
use crate::character_scope::{Tables, UnitLeader};
use replay::rows::{CharacterRow, TableSchema, UnitLeaderRow, tables};

#[cfg(test)]
mod tests;

impl<'r, 'c, 'f> Renderer<'r, 'c, 'f> {
    fn character_table<T: TableSchema>(&self) -> Result<&'f [T::Row], String> {
        self.context
            .text
            .frame()
            .complete::<T>()
            .map_err(|why| why.to_string())
    }

    // 140529310: scope +0x50 is a character reference. SetUnitLeader
    // (14052EFD0) installs the leader's character reference, not the leader.
    pub(super) fn effect_character(
        &self,
        scope: Scope,
    ) -> Result<Option<&'f CharacterRow>, String> {
        let id = match scope {
            Scope::Character(id) => id,
            Scope::Leader(reference) => {
                let Some(row) = self
                    .character_table::<tables::UnitLeaders>()?
                    .iter()
                    .find(|row| {
                        (u64::from(row.id) << 32) | u64::from(row.reference_type) == reference
                    })
                else {
                    return Ok(None);
                };
                row.character_id
            }
            _ => return Ok(None),
        };
        Ok(self
            .character_table::<tables::Characters>()?
            .iter()
            .find(|row| row.id == id))
    }

    // ResolveCharacter 140536B10: evaluated scope, named lookup 140536BB0,
    // then current character. The named lookup takes the country's first
    // active character of the token (1410D6BF0); its native roster is recorded.
    pub(super) fn effect_character_target(
        &self,
        operand: Option<&[u8]>,
        scopes: Scopes,
    ) -> Result<Option<&'f CharacterRow>, String> {
        if let Some(operand) = operand {
            if let Some(selected) = self.evaluator()?.scoped_scope(operand, scopes)?
                && let Some(character) = self.effect_character(selected)?
            {
                return Ok(Some(character));
            }
            let named_country = match scopes.root {
                Scope::Country(country) if country > 0 => Some(country),
                // 140536BB0 chooses a ROOT with a positive state-owner tag,
                // but then requires that scope's own +8 country to be positive.
                Scope::State(state) if state != NULL_STATE && state != 0 => {
                    let world = self.context.world()?;
                    let state = world
                        .states
                        .get(&state)
                        .ok_or("ROOT state is not recorded")?;
                    if state.owner as i32 > 0 {
                        None
                    } else {
                        match scopes.this {
                            Scope::Country(country) => Some(country),
                            _ => None,
                        }
                    }
                }
                _ => match scopes.this {
                    Scope::Country(country) => Some(country),
                    _ => None,
                },
            };
            if let Some(country) = named_country {
                let world = self.context.world()?;
                if let Some(token) = world.token_of(operand)? {
                    let characters = self.character_table::<tables::Characters>()?;
                    let roster = self.character_table::<tables::CountryCharacters>()?;
                    let mut selected: Option<(u32, &CharacterRow)> = None;
                    for row in roster {
                        let owner = world.slot(row.country).ok_or_else(|| {
                            format!(
                                "CountryLinks lacks character roster country {}",
                                row.country
                            )
                        })?;
                        if owner != country {
                            continue;
                        }
                        let Some(id) = row.character else {
                            continue;
                        };
                        let character =
                            characters.iter().find(|row| row.id == id).ok_or_else(|| {
                                format!("CountryCharacters references missing character {id}")
                            })?;
                        if character.name_token_id as u32 == token
                            && selected.is_none_or(|(ordinal, _)| row.ordinal < ordinal)
                        {
                            selected = Some((row.ordinal, character));
                        }
                    }
                    if let Some((_, character)) = selected {
                        return Ok(Some(character));
                    }
                }
            }
        }
        self.effect_character(scopes.this)
    }

    // GetNameStringAt28 140612C90 copies +0x28 verbatim; no token fallback.
    // Without the original key, nonempty stored text follows the shared
    // recording-language rule rather than being relocalised heuristically.
    pub(super) fn effect_character_name(&self, row: &CharacterRow) -> Result<String, String> {
        crate::scripted_text::stored_text(
            self.context.localisation,
            self.context.text.recorded_language(),
            &row.name,
            &format!("character {} name (+0x28)", row.id),
        )
    }

    // 1405294F0 / 140F8A840: the character's direct role, else reference role.
    pub(super) fn effect_unit_leader(
        &self,
        scopes: Scopes,
    ) -> Result<Option<UnitLeader<'f>>, String> {
        let Some(character) = self.effect_character(scopes.this)? else {
            return Ok(None);
        };
        Tables::read(self.context.text.frame()).unit_leader(character.id)
    }

    pub(super) fn effect_leader_name(&self, row: &UnitLeaderRow) -> Result<String, String> {
        crate::scripted_text::stored_text(
            self.context.localisation,
            self.context.text.recorded_language(),
            &row.inherited_text_b,
            &format!("unit leader {} name (+0x40)", row.id),
        )
    }

    // Own 140391DB0; reader 1403AB590. NAME@142727E7C, COUNTRYDEF@142764D78;
    // EFFECT_SET_LEADER_NATIONALITY@142764D88;
    // expansion precedes the explicit LF. Missing operand character falls back.
    pub(in crate::effect_description) fn character_set_nationality(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let character = field(children(item), "character").and_then(|item| item.value.scalar());
        let Some(character) = self.effect_character_target(character, scopes)? else {
            return Ok(String::new());
        };
        let country = item.value.scalar().or_else(|| {
            field(children(item), "target_country").and_then(|item| item.value.scalar())
        });
        let Some(country) = country
            .map(|operand| self.evaluator()?.scoped_country(operand, scopes))
            .transpose()?
            .flatten()
        else {
            return Ok(String::new());
        };
        let name = self.effect_character_name(character)?;
        let country = self.country_name(country, true)?;
        self.line(
            b"EFFECT_SET_LEADER_NATIONALITY",
            &[
                ("NAME", Argument::Text(&name)),
                ("COUNTRYDEF", Argument::Text(&country)),
            ],
            scopes,
            true,
        )
    }

    // Own 1402EDA60: NAME@142727E7C, COUNTRY@142728968, no scope expansion;
    // CCharacter::country_reference 140A5BF80 supplies the character's country.
    pub(in crate::effect_description) fn character_retire(
        &self,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Some(character) = self.effect_character(scopes.this)? else {
            return Ok(String::new());
        };
        let name = self.effect_character_name(character)?;
        let country = if character.country as i32 <= 0 {
            String::new()
        } else {
            let slot = self
                .context
                .world()?
                .slot(character.country)
                .ok_or_else(|| {
                    format!("CountryLinks lacks character country {}", character.country)
                })?;
            self.country_name(slot, false)?
        };
        self.line(
            b"EFFECT_RETIRE_CHARACTER",
            &[
                ("NAME", Argument::Text(&name)),
                ("COUNTRY", Argument::Text(&country)),
            ],
            scopes,
            false,
        )
    }

    // Own 1402F8130: ResolveArmyLeader 1405292D0 accepts categories <2.
    // NAME@142727E7C copies +0x40; COUNTRY_NAME_DEF@14272BA20; no LF/expansion.
    pub(in crate::effect_description) fn character_capture_by(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Some(leader) = self.effect_unit_leader(scopes)? else {
            return Ok(String::new());
        };
        if leader.category() as u32 >= 2 {
            return Ok(String::new());
        }
        let Some(country) = self.country_target(item, scopes)? else {
            return Ok(String::new());
        };
        let name = match leader {
            UnitLeader::Direct(row) => self.effect_leader_name(row)?,
            UnitLeader::Referenced(row) => {
                // Referenced roles (+0xC8) do not capture names themselves, but
                // another character's direct role can record the same handle.
                let Some(leader) =
                    self.character_table::<tables::UnitLeaders>()?
                        .iter()
                        .find(|leader| {
                            (u64::from(leader.id) << 32) | u64::from(leader.reference_type)
                                == row.reference
                        })
                else {
                    return Err(format!(
                        "character {} referenced army leader name (+0x40) is unrecorded",
                        row.character
                    ));
                };
                self.effect_leader_name(leader)?
            }
        };
        let country = self.country_name(country, true)?;
        self.localised(
            b"EFFECT_CAPTURE_GENERAL",
            &[
                ("NAME", Argument::Text(&name)),
                ("COUNTRY_NAME_DEF", Argument::Text(&country)),
            ],
            scopes,
            false,
        )
    }

    // Own 140374C60 gates on ResolveAbilityCaster/category 3 and a positive
    // country. The valid line also requires ptr_0x1030 140C06040 (CODENAME),
    // which neither UnitLeaders nor CharacterOperatives records.
    pub(in crate::effect_description) fn character_add_nationality(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Some(leader) = self.effect_unit_leader(scopes)? else {
            return Ok(String::new());
        };
        if leader.category() != 3 || self.country_target(item, scopes)?.is_none() {
            return Ok(String::new());
        }
        Err("operative codename (COperativeLeader+0x1030, 140C06040) is unrecorded".into())
    }
}
