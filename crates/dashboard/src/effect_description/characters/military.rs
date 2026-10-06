use super::*;
use replay::rows::CountryCharacterRow;

#[cfg(test)]
mod tests;

// 140383E60 / 140395390: CUnitLeader category +0xE7C. The keys and
// NAME@142727E7C are executable facts; their text comes from the stack.
fn creation_key(category: i32) -> Option<&'static [u8]> {
    match category {
        0 => Some(b"EFFECT_CREATE_FM_LEADER"),        // 142751508
        1 => Some(b"EFFECT_CREATE_CC_LEADER"),        // 142751520
        2 => Some(b"EFFECT_CREATE_NAVY_LEADER"),      // 142751538
        3 => Some(b"EFFECT_CREATE_OPERATIVE_LEADER"), // 142751558
        _ => None,
    }
}

impl<'r, 'c, 'f> Renderer<'r, 'c, 'f> {
    // 1410D1130 -> 141A883F0: both country rosters are ordered by the
    // character's name token (+0x18). Equal tokens use the first entry.
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

    // Own 14036C790/14036CC50/14036C9F0. ResolveCharacter 140536B10,
    // current fallback 140529310; GetNameStringAt28 140612C90. Only NAME
    // is described: skill/traits/legacy_id belong to execution. Expanded, no LF.
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

    // Own 140383E60; ReadEffect 1403B1010 -> CUnitLeaderData 1403B6540.
    // Nonempty authored name +0x180 is literal. Empty name selects by id
    // (default 0, constructor 14032CB10), compared to character +0x18,
    // then exact +0x48 token == tag + '_' (1403B8BF0 / 1410D6860).
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
                .and_then(|value| installed_content::crt::scan_i32(value).stored())
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

    // Own 14037DC50. Reader 1403A9BB0 allocates a country-0 operative;
    // 1403B14C0 -> 140C0C950 reads name into +0x20. Finalizer 1403A4070
    // calls 140C0BB50 -> 140C0BC70, plain-localising it into +0x40 before
    // this NAME substitution. Description expansion follows; no explicit LF.
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

    // Own 140395390, scalar reader 1403ABD40. 1410D6BF0(...,1) checks
    // active then retired roster. 140F8FED0 tests +0x58, the length of the
    // recorded token string at +0x48. A token-bearing character without a
    // leader discards the whole description. Plain localisation, terminal LF.
    pub(in crate::effect_description) fn char_show_unit_leaders(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let operand = installed_content::parser::c_string(
            item.value
                .scalar()
                .ok_or("unit leader tooltip token is not scalar")?,
        );
        let country = self.current_country(scopes)?;
        let Some(token) = self.context.world()?.token_of(operand)? else {
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
        let Some(leader) = Tables::read(self.context.text.frame()).unit_leader(character.id)?
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

    // Own 1402EC5A0 / 1402EC390: only CC promotion / FM demotion;
    // no NAME argument or expansion. Each successful line appends LF.
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

    // Own 1402ECF90. Reader 1402F0540 accepts exact yes or a positive
    // integer. 1402E4180 resolves a positive runtime character ID in the
    // character manager; otherwise the scope ability caster. +0x40 NAME
    // is plain. Operatives additionally need CODENAME +0x1030 (140C06040).
    pub(in crate::effect_description) fn char_remove_unit_leader(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let id = item
            .value
            .scalar()
            .and_then(|value| installed_content::crt::scan_i32(value).stored())
            .unwrap_or(0);
        let leader = if id > 0 {
            let Some(character) = self.effect_character(Scope::Character(id as u32))? else {
                return Ok(String::new());
            };
            Tables::read(self.context.text.frame()).unit_leader(character.id)?
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
                ); // 1402ECF90: no scope expansion or LF.
            }
            _ => return Ok(String::new()),
        };
        let name = self.effect_resolved_leader_name(leader)?;
        self.char_native_text(key, &[("NAME", Argument::Text(&name))], scopes, false)
    }

    // Own 140389830. TITLE@142734318 uses GetLocalizedRoleTitle 140C0A7A0:
    // categories 0/1/2 -> fm_leader/cc_leader/navy_leader; category 3 uses
    // operative_skill_<stored skill level>, else OPERATIVE when absent.
    // NAME is the character's +0x28, not the leader's +0x40. Expanded, no LF.
    pub(in crate::effect_description) fn char_remove_military_role(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        // 1403AAC00: character scope accepts exact yes only; outside it,
        // scalar and character-key forms both assign the +0x58 selector.
        let operand = if matches!(scopes.this, Scope::Character(_) | Scope::Leader(_)) {
            None
        } else {
            item.value
                .scalar()
                .or_else(|| field(children(item), "character").and_then(|item| item.value.scalar()))
        };
        let Some(character) = self.effect_character_target(operand, scopes)? else {
            return Ok(String::new());
        };
        let Some(leader) = Tables::read(self.context.text.frame()).unit_leader(character.id)?
        else {
            return Ok(String::new());
        };
        let title = match leader.category() {
            0 => self.char_native_text(b"fm_leader", &[], scopes, false)?,
            1 => self.char_native_text(b"cc_leader", &[], scopes, false)?,
            2 => self.char_native_text(b"navy_leader", &[], scopes, false)?,
            3 => {
                let key = format!("operative_skill_{}", leader.skill()?);
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

    // Own 14045D5A0: ResolveCharacter 140536B10 then current 140529310;
    // NAME is +0x28. The describer draws no random trait and reads no unit.
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

    // Own 14045D0D0: ResolveArmy 140529260 only gates Army existence;
    // no commander record, current XP or name is read. Evaluate +0x58 then
    // localise ADD_DIVISIONAL_COMMANDER_XP with Fixed5 VAL@14272F830.
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
        // Native null is empty (14045D0D0); missing Divisions cannot prove null.
        self.context.world()?.unit(reference)?;
        let amount = self.scoped_value(item.value.scalar().unwrap_or_default(), scopes)?;
        self.char_native_text(
            b"ADD_DIVISIONAL_COMMANDER_XP",
            &[("VAL", Argument::Fixed5(amount))],
            scopes,
            false,
        )
    }

    // Format 98 records the selected naming operands. Lead D2's format-98
    // follow-up PR will use them in the shared CArmy formatter (140C72F00).
    fn char_recorded_unit_name(&self, reference: u64) -> Result<String, String> {
        // A UNIT absent from Divisions (possibly non-Army) conservatively
        // refuses; native ResolveArmy null gives empty (14045D0D0).
        self.context.world()?.unit(reference)?;
        let row = self
            .character_table::<tables::DivisionStatus>()?
            .iter()
            .find(|row| row.division == reference)
            .ok_or("DivisionStatus lacks UNIT naming row")?;
        row.naming
            .require("Army name (Army+0x340 -> CNameGroupMember+0x60, 140C72F00)")?
            .as_ref()
            .ok_or(
                "Army name (Army+0x340 -> CNameGroupMember+0x60, 140C72F00) capture unavailable",
            )?;
        Err("shared UNIT name formatter (140C72F00) awaits lead D2's format-98 follow-up PR to use DivisionStatus.naming".into())
    }

    // Own 14045D2E0: ResolveArmy, slot-13 name, UNIT@142776CA8;
    // key/subject/allow are execution-only. No expansion or appended LF.
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
        let name = self.char_recorded_unit_name(reference)?;
        self.char_native_text(
            b"EFFECT_ADD_HISTORY_ENTRY_UNIT_EFFECT",
            &[("UNIT", Argument::Text(&name))],
            scopes,
            false,
        )
    }

    // Own 1404A2FB0 -> 141454480: the raid's +0xC8 assigned Army first,
    // then +0xD0 air wing. NAME@142727E7C, plain localisation, final LF.
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
        let raid = self.context.world()?.raid(reference)?;
        let name = if raid.unit != 0 {
            // A missing Divisions row cannot prove native ResolveArmy null.
            self.char_recorded_unit_name(raid.unit)?
        } else if raid.air_wing != 0 {
            return Err(
                "shared wing naming hook (141454480 -> 140F4B8F0, CAirWing+0x998) is unavailable"
                    .into(),
            );
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

    // Own 14045D8B0: ResolveArmy then GetName 140C72F00, DIV@142797880.
    // Null Army returns native empty without reading naming operands.
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
        // Missing Divisions cannot prove ResolveArmy null (140529260/14045D8B0).
        let name = self.char_recorded_unit_name(reference)?;
        self.char_native_text(
            b"ADD_UNIT_MEDAL_DIV_EFFECT",
            &[("DIV", Argument::Text(&name))],
            scopes,
            false,
        ) // DIV@142797880.
    }
}
