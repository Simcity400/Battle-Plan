//! Character operands and names shared by their native effect descriptions.

use super::*;
use crate::character_scope::{Tables, UnitLeader};
use replay_data::{rows::CharacterRow, rows::TableSchema, rows::UnitLeaderRow, rows::tables};

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

    // A unit-leader scope carries the leader's character reference,
    // rather than the leader itself.
    pub(super) fn effect_character(
        &self,
        scope: Scope,
    ) -> Result<Option<&'f CharacterRow>, String> {
        let id = match scope {
            Scope::Character(id) => id,
            Scope::Leader(reference)
            | Scope::CountryLeader {
                leader: reference, ..
            } => {
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

    // Try the evaluated scope, named lookup, then the current character.
    // Named lookup selects the country's first active character of the token,
    // then searches the retired roster.
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
                Scope::Country(country) | Scope::CountryLeader { country, .. } if country > 0 => {
                    Some(country)
                }
                // Choose a ROOT with a positive state-owner tag,
                // but require that scope's own country to be positive.
                Scope::State(state) if state != NULL_STATE && state != 0 => {
                    let world = self.context.world()?;
                    let state = world
                        .states()
                        .get(&state)
                        .ok_or("ROOT state is not recorded")?;
                    if state.owner as i32 > 0 {
                        None
                    } else {
                        match scopes.this {
                            Scope::Country(country) | Scope::CountryLeader { country, .. } => {
                                Some(country)
                            }
                            _ => None,
                        }
                    }
                }
                _ => match scopes.this {
                    Scope::Country(country) | Scope::CountryLeader { country, .. } => Some(country),
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

    // Copy the stored character name verbatim, without a token fallback.
    // Nonempty text without its original key follows the shared recording-language
    // rule rather than being relocalised heuristically.
    pub(super) fn effect_character_name(&self, row: &CharacterRow) -> Result<String, String> {
        crate::scripted_text::stored_text(
            self.context.localisation,
            self.context.text.recorded_language(),
            &row.name,
            &format!("character {} name", row.id),
        )
    }

    // Use the character's direct role, else its reference role.
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
            &format!("unit leader {} name", row.id),
        )
    }

    // A registry-only role joins text rows by the full reference,
    // without inventing a direct character association.
    pub(super) fn effect_resolved_leader_name(
        &self,
        leader: UnitLeader<'f>,
    ) -> Result<String, String> {
        match leader {
            UnitLeader::Direct(row) => self.effect_leader_name(row),
            UnitLeader::Referenced(row) => {
                // Format 98 retains the referenced role's own name bytes;
                // older recordings may join an exact direct handle.
                if let replay_data::rows::Recorded::Value(Some(name)) = &row.name {
                    return crate::scripted_text::stored_text(
                        self.context.localisation,
                        self.context.text.recorded_language(),
                        name,
                        &format!("character {} referenced unit leader name", row.character),
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
                            "character {} referenced unit leader name {}",
                            row.character,
                            if matches!(row.name, replay_data::rows::Recorded::NotRecorded) {
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

    // Substitute NAME and COUNTRYDEF, then expand before the explicit LF.
    // A missing operand character falls back to the current character.
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

    // Use the character's country for COUNTRY and its name for NAME.
    // Do not expand the containing scope.
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

    // Accept army-leader categories below 2. Copy the leader name for NAME
    // and the definite country name for COUNTRY_NAME_DEF, without LF or expansion.
    // The block reads country, not target, and requires a positive country.
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

    // Require an operative ability caster and a positive country.
    // Use the leader name, codename and country adjective for the arguments;
    // only the containing text expands.
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

    // Format 98 retains the selected codename operands. The shared name
    // catalogue formats them without selecting or reserving another name.
    pub(super) fn effect_operative_codename(
        &self,
        leader: UnitLeader<'f>,
    ) -> Result<String, String> {
        let field = "operative codename";
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
