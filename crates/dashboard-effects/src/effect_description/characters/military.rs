use super::*;
use replay_data::rows::CountryCharacterRow;

#[cfg(test)]
mod tests;

fn creation_key(category: i32) -> Option<&'static [u8]> {
    match category {
        0 => Some(b"EFFECT_CREATE_FM_LEADER"),
        1 => Some(b"EFFECT_CREATE_CC_LEADER"),
        2 => Some(b"EFFECT_CREATE_NAVY_LEADER"),
        3 => Some(b"EFFECT_CREATE_OPERATIVE_LEADER"),
        _ => None,
    }
}

impl<'r, 'c, 'f> Renderer<'r, 'c, 'f> {
    fn military_roster_character(
        &self,
        roster: Rowset<'f, CountryCharacterRow>,
        country: u32,
        token: i32,
    ) -> Result<Option<Cow<'f, CharacterRow>>, String> {
        let world = self.context.world()?;
        let characters = self.character_rows()?;
        let mut selected: Option<(u32, Option<Cow<'f, CharacterRow>>)> = None;
        for row in each(roster) {
            let row = row?;
            if world.slot(row.country).ok_or_else(|| {
                format!(
                    "country links lacks character roster country {}",
                    row.country
                )
            })? != country
            {
                continue;
            }
            let character = row
                .character
                .map(|id| {
                    keyed(&characters, u128::from(id))?.ok_or_else(|| {
                        format!("country character roster references missing character {id}")
                    })
                })
                .transpose()?;
            if character.as_ref().map_or(0, |row| row.name_token_id) == token
                && selected
                    .as_ref()
                    .is_none_or(|(ordinal, _)| row.ordinal < *ordinal)
            {
                selected = Some((row.ordinal, character));
            }
        }
        Ok(selected.and_then(|(_, character)| character))
    }

    pub(in crate::effect_description) fn char_add_military_role(
        &self,
        item: &Item,
        scopes: Scopes,
        category: i32,
    ) -> Result<String, String> {
        let operand = field(children(item), "character").and_then(|item| item.value.scalar());
        let Some(character) = self.effect_character_target(operand, scopes)? else {
            return Ok(String::new());
        };
        let name = self.effect_character_name(&character)?;
        self.char_native_text(
            creation_key(category).expect("registered military role category"),
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        )
    }

    pub(in crate::effect_description) fn char_create_legacy_military(
        &self,
        item: &Item,
        scopes: Scopes,
        category: i32,
    ) -> Result<String, String> {
        let mut name = field(children(item), "name")
            .map(|item| item.value.read_str().map_err(|why| why.to_string()))
            .transpose()?
            .unwrap_or_default()
            .to_owned();
        if name.is_empty() {
            let country = self.current_country(scopes)?;
            let roster = self.character_table::<tables::CountryCharacters>(ROSTER)?;
            let id = field(children(item), "id")
                .and_then(|item| item.value.scalar())
                .and_then(|value| content_reader_parser::crt::scan_i32(value).stored())
                .unwrap_or(0);
            let mut character = self.military_roster_character(roster, country, id)?;
            if character.is_none() {
                let token = format!("{}_", self.tag(country)?);
                let characters = self.character_rows()?;
                let world = self.context.world()?;
                let mut selected: Option<(u32, Cow<'f, CharacterRow>)> = None;
                for row in each(roster) {
                    let row = row?;
                    if world
                        .slot(row.country)
                        .ok_or("character roster country is not recorded")?
                        != country
                    {
                        continue;
                    }
                    let Some(id) = row.character else { continue };
                    let candidate = keyed(&characters, u128::from(id))?.ok_or_else(|| {
                        format!("country characters references missing character {id}")
                    })?;
                    if candidate.token == token.as_bytes()
                        && selected
                            .as_ref()
                            .is_none_or(|(ordinal, _)| row.ordinal < *ordinal)
                    {
                        selected = Some((row.ordinal, candidate));
                    }
                }
                character = selected.map(|(_, character)| character);
            }
            if let Some(character) = character {
                name = self.effect_character_name(&character)?;
            }
        }
        self.char_native_text(
            creation_key(category).expect("registered legacy military category"),
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        )
    }

    pub(in crate::effect_description) fn char_create_operative(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let name_key = field(children(item), "name")
            .and_then(|item| item.value.read_string())
            .unwrap_or_default();
        let name = self.localised(name_key, &[], scopes, false)?;
        self.char_native_text(
            b"EFFECT_CREATE_OPERATIVE_LEADER",
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        )
    }

    pub(in crate::effect_description) fn char_show_unit_leaders(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let operand = content_reader_parser::parser::c_string(
            item.value
                .scalar()
                .ok_or("unit leader tooltip token is not scalar")?,
        );
        let country = self.current_country(scopes)?;
        let Some(token) = self
            .context
            .world()?
            .token_of(operand)
            .map_err(|why| why.to_string())?
        else {
            return Ok(String::new());
        };
        let mut character = self.military_roster_character(
            self.character_table::<tables::CountryCharacters>(ROSTER)?,
            country,
            token as i32,
        )?;
        if character.is_none() {
            character = self.military_roster_character(
                self.character_table::<tables::CountryRetiredCharacters>(ROSTER)?,
                country,
                token as i32,
            )?;
        }
        let Some(character) = character.filter(|character| !character.token.is_empty()) else {
            return Ok(String::new());
        };
        let Some(leader) = Tables::read_in(self.tables())
            .unit_leader(character.id)
            .map_err(|why| why.to_string())?
        else {
            return Ok(String::new());
        };
        let Some(key) = creation_key(leader.category()) else {
            return Ok(String::new());
        };
        let name = self.effect_resolved_leader_name(leader)?;
        let mut text =
            self.char_native_text(key, &[("NAME", Argument::Text(&name))], scopes, false)?;
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        Ok(text)
    }

    pub(in crate::effect_description) fn char_change_rank(
        &self,
        scopes: Scopes,
        promote: bool,
    ) -> Result<String, String> {
        let Some(leader) = self.effect_unit_leader(scopes)? else {
            return Ok(String::new());
        };
        if leader.category() != i32::from(promote) {
            return Ok(String::new());
        }
        self.char_native_line(
            if promote {
                b"EFFECT_PROMOTE_LEADER"
            } else {
                b"EFFECT_DEMOTE_LEADER"
            },
            &[],
            scopes,
            false,
        )
    }

    pub(in crate::effect_description) fn char_remove_unit_leader(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let id = item
            .value
            .scalar()
            .and_then(|value| content_reader_parser::crt::scan_i32(value).stored())
            .unwrap_or(0);
        let leader = if id > 0 {
            let Some(character) = self.effect_character(Scope::Character(id as u32))? else {
                return Ok(String::new());
            };
            Tables::read_in(self.tables())
                .unit_leader(character.id)
                .map_err(|why| why.to_string())?
        } else {
            self.effect_unit_leader(scopes)?
        };
        let Some(leader) = leader else {
            return Ok(String::new());
        };
        let key: &[u8] = match leader.category() {
            0 => b"EFFECT_REMOVE_FM_LEADER",
            1 => b"EFFECT_REMOVE_CC_LEADER",
            2 => b"EFFECT_REMOVE_NAVY_LEADER",
            3 => {
                let codename = self.effect_operative_codename(&leader)?;
                let name = self.effect_resolved_leader_name(leader)?;
                return self.char_native_text(
                    b"EFFECT_REMOVE_OPERATIVE_LEADER",
                    &[
                        ("CODENAME", Argument::Text(&codename)),
                        ("NAME", Argument::Text(&name)),
                    ],
                    scopes,
                    false,
                );
            }
            _ => return Ok(String::new()),
        };
        let name = self.effect_resolved_leader_name(leader)?;
        self.char_native_text(key, &[("NAME", Argument::Text(&name))], scopes, false)
    }

    pub(in crate::effect_description) fn char_remove_military_role(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let operand = if matches!(
            scopes.this,
            Scope::Character(_) | Scope::Leader(_) | Scope::CountryLeader { .. }
        ) {
            None
        } else {
            item.value
                .scalar()
                .or_else(|| field(children(item), "character").and_then(|item| item.value.scalar()))
        };
        let Some(character) = self.effect_character_target(operand, scopes)? else {
            return Ok(String::new());
        };
        let Some(leader) = Tables::read_in(self.tables())
            .unit_leader(character.id)
            .map_err(|why| why.to_string())?
        else {
            return Ok(String::new());
        };
        let title = match leader.category() {
            0 => self.char_native_text(b"fm_leader", &[], scopes, false)?,
            1 => self.char_native_text(b"cc_leader", &[], scopes, false)?,
            2 => self.char_native_text(b"navy_leader", &[], scopes, false)?,
            3 => {
                let key = format!(
                    "operative_skill_{}",
                    leader.skill().map_err(|why| why.to_string())?
                );
                self.localised(
                    if self.context.localisation.get(&key).is_some() {
                        key.as_bytes()
                    } else {
                        b"OPERATIVE"
                    },
                    &[],
                    scopes,
                    false,
                )?
            }
            _ => String::new(),
        };
        let name = self.effect_character_name(&character)?;
        self.char_native_text(
            b"EFFECT_REMOVE_UNIT_LEADER_ROLE",
            &[
                ("NAME", Argument::Text(&name)),
                ("TITLE", Argument::Text(&title)),
            ],
            scopes,
            true,
        )
    }

    pub(in crate::effect_description) fn char_random_trait_from_unit(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let operand = field(children(item), "character").and_then(|item| item.value.scalar());
        let Some(character) = self.effect_character_target(operand, scopes)? else {
            return Ok(String::new());
        };
        let name = self.effect_character_name(&character)?;
        self.char_native_text(
            b"EFFECT_ADD_RANDOM_UNIT_TRAIT",
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        )
    }

    pub(in crate::effect_description) fn char_divisional_xp(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Scope::Unit(reference) = scopes.this else {
            return Ok(String::new());
        };
        if reference == 0 {
            return Ok(String::new());
        }
        self.context
            .world()?
            .unit(reference)
            .map_err(|why| why.to_string())?;
        let amount = self.scoped_value(item.value.scalar().unwrap_or_default(), scopes)?;
        self.char_native_text(
            b"ADD_DIVISIONAL_COMMANDER_XP",
            &[("VAL", Argument::Fixed5(amount))],
            scopes,
            false,
        )
    }

    pub(in crate::effect_description) fn char_history_entry(
        &self,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Scope::Unit(reference) = scopes.this else {
            return Ok(String::new());
        };
        if reference == 0 {
            return Ok(String::new());
        }
        self.context
            .world()?
            .unit(reference)
            .map_err(|why| why.to_string())?;
        let name = self.unit_name(reference)?;
        self.char_native_text(
            b"EFFECT_ADD_HISTORY_ENTRY_UNIT_EFFECT",
            &[("UNIT", Argument::Text(&name))],
            scopes,
            false,
        )
    }

    pub(in crate::effect_description) fn char_raid_history_entry(
        &self,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Scope::Raid(reference) = scopes.this else {
            return Ok(String::new());
        };
        if reference == 0 {
            return Ok(String::new());
        }
        let world = self.context.world()?;
        let raid = world.raid(reference).map_err(|why| why.to_string())?;
        let army_resolved = raid.unit != 0
            && match world.tables().complete::<tables::Divisions>(0) {
                Ok(rows) => rows.contains(u128::from(raid.unit)),
                Err(why) => {
                    world
                        .raid_wing_name(raid.air_wing)
                        .map_err(|_| why.to_string())?;
                    false
                }
            };
        let name = if army_resolved {
            self.unit_name(raid.unit)?
        } else if raid.air_wing != 0 {
            self.unit_name(raid.air_wing)?
        } else {
            String::new()
        };
        self.char_native_line(
            b"EFFECT_ADD_RAID_HISTORY_ENTRY_UNIT_EFFECT",
            &[("NAME", Argument::Text(&name))],
            scopes,
            false,
        )
    }

    pub(in crate::effect_description) fn char_unit_medal(
        &self,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Scope::Unit(reference) = scopes.this else {
            return Ok(String::new());
        };
        if reference == 0 {
            return Ok(String::new());
        }
        self.context
            .world()?
            .unit(reference)
            .map_err(|why| why.to_string())?;
        let name = self.unit_name(reference)?;
        self.char_native_text(
            b"ADD_UNIT_MEDAL_DIV_EFFECT",
            &[("DIV", Argument::Text(&name))],
            scopes,
            false,
        )
    }
}
