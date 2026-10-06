//! Character operands and names shared by their native effect descriptions.

use super::*;
use crate::character_scope::{Tables, UnitLeader};
use replay::rows::{CharacterRow, TableSchema, UnitLeaderRow, tables};

mod content;
mod military;
mod mio;
mod operatives;
mod peace;
mod roles;
mod scientists;
mod traits;
pub(super) use content::CharacterContent;

#[cfg(test)]
mod tests;

impl<'r, 'c, 'f> Renderer<'r, 'c, 'f> {
    // Executable localisation names must exist in the recorded content stack.
    // Authored name keys keep each native caller's ordinary raw-key fallback.
    pub(super) fn char_native_text(
        &self,
        key: &[u8],
        args: &[(&str, Argument<'_>)],
        scopes: Scopes,
        expand: bool,
    ) -> Result<String, String> {
        let name = std::str::from_utf8(key)
            .map_err(|_| "native character localisation key is not UTF-8")?;
        if self.context.localisation.get(name).is_none() {
            return Err(format!(
                "native character localisation key {name} is not defined"
            ));
        }
        self.localised(key, args, scopes, expand)
    }

    pub(super) fn char_native_line(
        &self,
        key: &[u8],
        args: &[(&str, Argument<'_>)],
        scopes: Scopes,
        expand: bool,
    ) -> Result<String, String> {
        let mut text = self.char_native_text(key, args, scopes, expand)?;
        text.push('\n');
        Ok(text)
    }

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
    // active character of the token, then retired (1410D6BF0, include_retired=1).
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
                    let active = self.character_table::<tables::CountryCharacters>()?;
                    for roster in [Some(active), None] {
                        let roster = match roster {
                            Some(rows) => rows,
                            None => self.character_table::<tables::CountryRetiredCharacters>()?,
                        };
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

    // 140F8A840 may select a registry-only role. Text rows join the full
    // reference without inventing a direct CCharacter+0xA8 association.
    pub(super) fn effect_resolved_leader_name(
        &self,
        leader: UnitLeader<'f>,
    ) -> Result<String, String> {
        match leader {
            UnitLeader::Direct(row) => self.effect_leader_name(row),
            UnitLeader::Referenced(row) => {
                // Format 98 retains the referenced role's own +0x40 bytes
                // (140C16FB0); older recordings may join an exact direct handle.
                if let replay::rows::Recorded::Value(Some(name)) = &row.name {
                    return crate::scripted_text::stored_text(
                        self.context.localisation,
                        self.context.text.recorded_language(),
                        name,
                        &format!(
                            "character {} referenced unit leader name (+0x40)",
                            row.character
                        ),
                    );
                }
                let named = self
                    .character_table::<tables::UnitLeaders>()?
                    .iter()
                    .find(|leader| {
                        (u64::from(leader.id) << 32) | u64::from(leader.reference_type)
                            == row.reference
                    })
                    .ok_or_else(|| {
                        format!(
                            "character {} referenced unit leader name (+0x40, 140C16FB0) {}",
                            row.character,
                            if matches!(row.name, replay::rows::Recorded::NotRecorded) {
                                "not recorded"
                            } else {
                                "capture unavailable"
                            }
                        )
                    })?;
                self.effect_leader_name(named)
            }
        }
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
    // ReadEffect 1402F8640 reads country, not target; vf7 requires country >0.
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
        let operand = if item.value.scalar().is_some() {
            Some(item)
        } else {
            field(children(item), "country")
        };
        let Some(operand) = operand else {
            return Ok(String::new());
        };
        let Some(country) = self.country_target(operand, scopes)? else {
            return Ok(String::new());
        };
        let name = self.effect_resolved_leader_name(leader)?;
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
    // country. NAME is +0x40, CODENAME is 140C06040, and NATIONALITY
    // is ccountry_tag::adjective 140BA5560; only the containing text expands.
    pub(in crate::effect_description) fn character_add_nationality(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Some(leader) = self.effect_unit_leader(scopes)? else {
            return Ok(String::new());
        };
        if leader.category() != 3 {
            return Ok(String::new());
        }
        let Some(country) = self.country_target(item, scopes)? else {
            return Ok(String::new());
        };
        let nationality = self.context.text.country_adjective(country)?;
        let codename = self.effect_operative_codename(leader)?;
        let name = self.effect_resolved_leader_name(leader)?;
        self.char_native_text(
            b"EFFECT_ADD_NATIONALITY_TO_OPERATIVE",
            &[
                ("NAME", Argument::Text(&name)),
                ("CODENAME", Argument::Text(&codename)),
                ("NATIONALITY", Argument::Text(&nationality)),
            ],
            scopes,
            true,
        )
    }

    // 140C06040 returns the embedded name member's +0x60. Format 98
    // retains its selected operands; the shared name catalogue implements
    // 1409B5640/1409B57C0, without selecting or reserving another name.
    pub(super) fn effect_operative_codename(
        &self,
        leader: UnitLeader<'f>,
    ) -> Result<String, String> {
        let field = "operative codename (COperativeLeader+0x1030, 140C06040)";
        let recorded = match leader {
            UnitLeader::Direct(row) => &row.codename,
            UnitLeader::Referenced(row) => &row.codename,
        };
        let name = recorded
            .require(field)?
            .as_ref()
            .ok_or_else(|| format!("{field} capture unavailable"))?;
        if name.kind != 2 {
            return Err(format!("{field} has naming kind {}, expected 2", name.kind));
        }
        // The reader retains custom bytes only on the native stored-name
        // branch, including a programmatic override with a null group.
        if !name.custom.is_empty() {
            if name.programmatic != 0 {
                return crate::scripted_text::stored_text(
                    self.context.localisation,
                    self.context.text.recorded_language(),
                    &name.custom,
                    field,
                );
            }
            return String::from_utf8(name.custom.clone())
                .map_err(|_| format!("{field} custom name is not UTF-8"));
        }
        let group =
            std::str::from_utf8(&name.group).map_err(|_| format!("{field} group is not UTF-8"))?;
        self.character_content()?
            .name_groups
            .as_ref()
            .map_err(Clone::clone)?
            .name(group, name.order, name.ordered != 0)
    }
}
