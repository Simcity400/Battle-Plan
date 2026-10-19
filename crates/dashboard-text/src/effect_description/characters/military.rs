use super::*;
use replay_data::rows::CountryCharacterRow;

#[cfg(test)]
mod tests;

// Select the description key from the leader category.
// Description text comes from the recorded content stack.
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
    // Both country rosters are ordered by the character's name token.
    // Equal tokens use the first entry.
    fn military_roster_character(
        &self,
        roster: &[CountryCharacterRow],
        country: u32,
        token: i32,
    ) -> Result<Option<&'f CharacterRow>, String> {
        let world = self.context.world()?;
        let characters = self.character_table::<tables::Characters>()?;
        let mut selected: Option<(u32, Option<&'f CharacterRow>)> = None;
        for row in roster {
            if world.slot(row.country).ok_or_else(|| {
                format!(
                    "CountryLinks lacks character roster country {}",
                    row.country
                )
            })? != country
            {
                continue;
            }
            let character = row
                .character
                .map(|id| {
                    characters.iter().find(|row| row.id == id).ok_or_else(|| {
                        format!("country character roster references missing character {id}")
                    })
                })
                .transpose()?;
            if character.map_or(0, |row| row.name_token_id) == token
                && selected.is_none_or(|(ordinal, _)| row.ordinal < ordinal)
            {
                selected = Some((row.ordinal, character));
            }
        }
        Ok(selected.and_then(|(_, character)| character))
    }

    // Resolve the named character, then fall back to the current character.
    // Only NAME is described: skill, traits and legacy_id belong to execution.
    // Expand the description without appending LF.
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
        let name = self.effect_character_name(character)?;
        self.char_native_text(
            creation_key(category).expect("registered military role category"),
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        )
    }

    // A nonempty authored name is literal. An empty name selects by id
    // (default 0), compared to the character's name token,
    // then by the exact stored token matching the country tag plus an underscore.
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
            let roster = self.character_table::<tables::CountryCharacters>()?;
            let id = field(children(item), "id")
                .and_then(|item| item.value.scalar())
                .and_then(|value| installed_content_core::crt::scan_i32(value).stored())
                .unwrap_or(0);
            let mut character = self.military_roster_character(roster, country, id)?;
            if character.is_none() {
                let token = format!("{}_", self.tag(country)?);
                let characters = self.character_table::<tables::Characters>()?;
                let world = self.context.world()?;
                let mut selected: Option<(u32, &'f CharacterRow)> = None;
                for row in roster {
                    if world
                        .slot(row.country)
                        .ok_or("character roster country is not recorded")?
                        != country
                    {
                        continue;
                    }
                    let Some(id) = row.character else { continue };
                    let candidate =
                        characters.iter().find(|row| row.id == id).ok_or_else(|| {
                            format!("CountryCharacters references missing character {id}")
                        })?;
                    if candidate.token == token.as_bytes()
                        && selected.is_none_or(|(ordinal, _)| row.ordinal < ordinal)
                    {
                        selected = Some((row.ordinal, candidate));
                    }
                }
                character = selected.map(|(_, character)| character);
            }
            if let Some(character) = character {
                name = self.effect_character_name(character)?;
            }
        }
        self.char_native_text(
            creation_key(category).expect("registered legacy military category"),
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        )
    }

    // Read the operative name and localise it without a scope before
    // substituting NAME. Expand the containing description afterward;
    // no explicit LF is appended.
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

    // Search the active roster before the retired roster.
    // A token-bearing character without a leader discards the whole description.
    // Use plain localisation with a terminal LF.
    pub(in crate::effect_description) fn char_show_unit_leaders(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let operand = installed_content_core::parser::c_string(
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
            self.character_table::<tables::CountryCharacters>()?,
            country,
            token as i32,
        )?;
        if character.is_none() {
            character = self.military_roster_character(
                self.character_table::<tables::CountryRetiredCharacters>()?,
                country,
                token as i32,
            )?;
        }
        let Some(character) = character.filter(|character| !character.token.is_empty()) else {
            return Ok(String::new());
        };
        let Some(leader) = Tables::read(self.context.text.frame())
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

    // Only corps-commander promotion and field-marshal demotion produce text.
    // No NAME argument or expansion; each successful line appends LF.
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

    // Accept exact yes or a positive integer. A positive runtime character ID
    // is resolved in the character manager; otherwise use the scope ability caster.
    // The leader name stays plain. Operatives additionally need their codename.
    pub(in crate::effect_description) fn char_remove_unit_leader(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let id = item
            .value
            .scalar()
            .and_then(|value| installed_content_core::crt::scan_i32(value).stored())
            .unwrap_or(0);
        let leader = if id > 0 {
            let Some(character) = self.effect_character(Scope::Character(id as u32))? else {
                return Ok(String::new());
            };
            Tables::read(self.context.text.frame())
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
                let codename = self.effect_operative_codename(leader)?;
                let name = self.effect_resolved_leader_name(leader)?;
                return self.char_native_text(
                    b"EFFECT_REMOVE_OPERATIVE_LEADER",
                    &[
                        ("CODENAME", Argument::Text(&codename)),
                        ("NAME", Argument::Text(&name)),
                    ],
                    scopes,
                    false,
                ); // No scope expansion or LF.
            }
            _ => return Ok(String::new()),
        };
        let name = self.effect_resolved_leader_name(leader)?;
        self.char_native_text(key, &[("NAME", Argument::Text(&name))], scopes, false)
    }

    // Categories 0/1/2 select fm_leader/cc_leader/navy_leader for TITLE.
    // Category 3 uses operative_skill_<stored skill level>, else OPERATIVE.
    // NAME is the character name; expand without appending LF.
    pub(in crate::effect_description) fn char_remove_military_role(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        // A character scope accepts exact yes only; outside it,
        // scalar and character-key forms both assign the selector.
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
        let Some(leader) = Tables::read(self.context.text.frame())
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
        let name = self.effect_character_name(character)?;
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

    // Resolve the named character, then the current character for NAME.
    // The describer draws no random trait and reads no unit.
    pub(in crate::effect_description) fn char_random_trait_from_unit(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let operand = field(children(item), "character").and_then(|item| item.value.scalar());
        let Some(character) = self.effect_character_target(operand, scopes)? else {
            return Ok(String::new());
        };
        let name = self.effect_character_name(character)?;
        self.char_native_text(
            b"EFFECT_ADD_RANDOM_UNIT_TRAIT",
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        )
    }

    // Army resolution only gates existence: no commander record, current XP
    // or name is read. Evaluate the amount, then localise
    // ADD_DIVISIONAL_COMMANDER_XP with fixed5 VAL.
    // Neither scope expansion nor a terminal LF is added.
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
        // A null Army is empty; missing Divisions cannot prove null.
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

    // Resolve the Army name for UNIT; key/subject/allow are execution-only.
    // No expansion or appended LF.
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

    // Use the raid's assigned Army first, then its air wing for NAME.
    // Use plain localisation with a final LF.
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
            && match world
                .frame()
                .complete::<replay_data::rows::tables::Divisions>()
            {
                Ok(rows) => rows.iter().any(|row| row.id == raid.unit),
                Err(why) => {
                    // The legacy active-raid witness proves Army resolution
                    // failed before choosing this exact wing. An unknown
                    // census alone cannot establish the native fallback.
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

    // Resolve the Army name for DIV. A null Army returns empty
    // without reading naming operands.
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
        // Missing Divisions cannot prove that Army resolution is null.
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
