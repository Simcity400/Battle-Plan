
use super::*;
use crate::character_scope::{Tables, UnitLeader};
use replay_data::rows::{
    CharacterRow, CountryCharacterRow, LexicalTokenRow, UnitLeaderRow, tables,
};

const CHARACTER: u64 = tables::Characters::ID.full_mask();
const LEADER_LINK: u64 = mask::<UnitLeaderRow>(&["character_id"]);
const LEADER_NAME: u64 = mask::<UnitLeaderRow>(&["inherited_text_b"]);
pub(super) const ROSTER: u64 = mask::<CountryCharacterRow>(&["character"]);

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

    fn character_lexical(&self) -> Result<Rowset<'f, LexicalTokenRow>, String> {
        self.complete::<tables::LexicalTokens>(0)
    }

    fn character_table<T: TableSchema>(&self, mask: u64) -> Result<Rowset<'f, T::Row>, String> {
        self.complete::<T>(mask)
    }

    fn character_rows(&self) -> Result<Rowset<'f, CharacterRow>, String> {
        self.complete::<tables::Characters>(CHARACTER)
    }

    fn character_row(&self, id: u32) -> Result<Option<Cow<'f, CharacterRow>>, String> {
        keyed(&self.character_rows()?, u128::from(id))
    }

    fn character_leaders(&self, mask: u64) -> Result<Rowset<'f, UnitLeaderRow>, String> {
        self.complete::<tables::UnitLeaders>(mask)
    }

    pub(super) fn effect_character(
        &self,
        scope: Scope,
    ) -> Result<Option<Cow<'f, CharacterRow>>, String> {
        let id = match scope {
            Scope::Character(id) => id,
            Scope::Leader(reference)
            | Scope::CountryLeader {
                leader: reference, ..
            } => {
                let leaders = self.character_leaders(LEADER_LINK)?;
                let Some(row) = keyed(&leaders, u128::from(reference))? else {
                    return Ok(None);
                };
                row.character_id
            }
            _ => return Ok(None),
        };
        self.character_row(id)
    }

    pub(super) fn effect_character_target(
        &self,
        operand: Option<&[u8]>,
        scopes: Scopes,
    ) -> Result<Option<Cow<'f, CharacterRow>>, String> {
        if let Some(operand) = operand {
            if let Some(selected) = self
                .evaluator()?
                .scoped_scope(operand, scopes)
                .map_err(|why| why.to_string())?
                && let Some(character) = self.effect_character(selected)?
            {
                return Ok(Some(character));
            }
            let named_country = match scopes.root {
                Scope::Country(country) | Scope::CountryLeader { country, .. } if country > 0 => {
                    Some(country)
                }
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
                if let Some(token) = world.token_of(operand).map_err(|why| why.to_string())? {
                    let characters = self.character_rows()?;
                    let active = self.character_table::<tables::CountryCharacters>(ROSTER)?;
                    for roster in [Some(active), None] {
                        let roster = match roster {
                            Some(rows) => rows,
                            None => self
                                .character_table::<tables::CountryRetiredCharacters>(ROSTER)?,
                        };
                        let mut selected: Option<(u32, Cow<'f, CharacterRow>)> = None;
                        for row in each(roster) {
                            let row = row?;
                            let owner = world.slot(row.country).ok_or_else(|| {
                                format!(
                                    "country links lacks character roster country {}",
                                    row.country
                                )
                            })?;
                            if owner != country {
                                continue;
                            }
                            let Some(id) = row.character else {
                                continue;
                            };
                            let character = keyed(&characters, u128::from(id))?.ok_or_else(|| {
                                format!("country characters references missing character {id}")
                            })?;
                            if character.name_token_id as u32 == token
                                && selected
                                    .as_ref()
                                    .is_none_or(|(ordinal, _)| row.ordinal < *ordinal)
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

    pub(super) fn effect_character_name(&self, row: &CharacterRow) -> Result<String, String> {
        crate::scripted_text::stored_text(
            self.context.localisation,
            self.context.text.recorded_language(),
            &row.name,
            &format!("character {} name", row.id),
        )
    }

    pub(super) fn effect_unit_leader(
        &self,
        scopes: Scopes,
    ) -> Result<Option<UnitLeader<'f>>, String> {
        let Some(character) = self.effect_character(scopes.this)? else {
            return Ok(None);
        };
        Tables::read_in(self.tables())
            .unit_leader(character.id)
            .map_err(|why| why.to_string())
    }

    pub(super) fn effect_leader_name(
        &self,
        row: replay_frame::frame::Leader<'_>,
    ) -> Result<String, String> {
        crate::scripted_text::stored_text(
            self.context.localisation,
            self.context.text.recorded_language(),
            row.inherited_text_b,
            &format!("unit leader {} name", row.id),
        )
    }

    pub(super) fn effect_resolved_leader_name(
        &self,
        leader: UnitLeader<'f>,
    ) -> Result<String, String> {
        match leader {
            UnitLeader::Direct(row) => self.effect_leader_name((&*row).into()),
            UnitLeader::Referenced(row) => {
                if let replay_data::rows::Recorded::Value(Some(name)) = &row.name {
                    return crate::scripted_text::stored_text(
                        self.context.localisation,
                        self.context.text.recorded_language(),
                        name,
                        &format!("character {} referenced unit leader name", row.character),
                    );
                }
                let leaders = self.character_leaders(LEADER_NAME)?;
                let named = keyed(&leaders, u128::from(row.reference))?
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
                self.effect_leader_name((&*named).into())
            }
        }
    }

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
            .transpose()
            .map_err(|why| why.to_string())?
            .flatten()
        else {
            return Ok(String::new());
        };
        let name = self.effect_character_name(&character)?;
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

    pub(in crate::effect_description) fn character_retire(
        &self,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Some(character) = self.effect_character(scopes.this)? else {
            return Ok(String::new());
        };
        let name = self.effect_character_name(&character)?;
        let country = if character.country as i32 <= 0 {
            String::new()
        } else {
            let slot = self
                .context
                .world()?
                .slot(character.country)
                .ok_or_else(|| {
                    format!("country links lacks character country {}", character.country)
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
        let codename = self.effect_operative_codename(&leader)?;
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

    pub(super) fn effect_operative_codename(
        &self,
        leader: &UnitLeader<'f>,
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
