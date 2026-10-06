//! Country leader roles and advisors; their operands are effect_keys' native read.

use super::*;
use installed_content::gui::native::{BoundLocalization, BoundValue, FormattedLocalization};
use installed_content::{characters::Authored, modifiers::Modifiers};
use replay::rows::{CharacterRoleTraitRow, CountryLeaderRoleRow, PartyLeaderRow};

#[cfg(test)]
mod tests;

fn operand<'a>(item: &'a Item, name: &str) -> Option<&'a [u8]> {
    field(children(item), name).and_then(|field| field.value.scalar())
}

fn boolean(item: &Item, name: &str) -> bool {
    operand(item, name)
        .and_then(installed_content::parser::read_bool)
        .unwrap_or(false)
}

enum FactionArgument {
    Text(String),
    Token(i32),
}

impl<'r, 'c, 'f> Renderer<'r, 'c, 'f> {
    // 14238F3B0 evaluates the bound node's arguments in authored order,
    // then 142393810 calls the native localiser with that argument array.
    // This adapter consumes the existing parsed tree and core localiser.
    fn char_faction_bound_node(
        &self,
        bound: &BoundLocalization,
        index: usize,
        scopes: Scopes,
    ) -> Result<FactionArgument, String> {
        let node = bound
            .nodes
            .get(index)
            .ok_or("faction bound localisation node is absent")?;
        let mut arguments = Vec::with_capacity(node.arguments.len());
        for argument in &node.arguments {
            let value = match &argument.value {
                BoundValue::Localization(value) => {
                    self.char_faction_formatted(value, &[], scopes)?
                }
                BoundValue::Node(child) => self.char_faction_bound_node(bound, *child, scopes)?,
            };
            let name = std::str::from_utf8(&argument.name)
                .map_err(|_| "faction bound argument name is not UTF-8")?;
            arguments.push((name, value));
        }
        let arguments = arguments
            .iter()
            .map(|(name, value)| {
                (
                    *name,
                    match value {
                        FactionArgument::Text(text) => Argument::Text(text),
                        FactionArgument::Token(token) => Argument::Int32(*token),
                    },
                )
            })
            .collect::<Vec<_>>();
        self.char_faction_formatted(&node.key, &arguments, scopes)
    }

    fn char_faction_formatted(
        &self,
        value: &FormattedLocalization,
        arguments: &[(&str, Argument<'_>)],
        scopes: Scopes,
    ) -> Result<FactionArgument, String> {
        match value {
            FormattedLocalization::Key(key) => self
                .localised(key, arguments, scopes, false)
                .map(FactionArgument::Text),
            // 1423939C0 interns token: payloads; 142393810 returns the
            // integer variant, which the containing localiser formats.
            FormattedLocalization::Token(token) => self
                .context
                .world()?
                .token_of(token)?
                .map(|token| FactionArgument::Token(token as i32))
                .ok_or_else(|| {
                    format!(
                        "faction bound localisation token {} is absent from LexicalTokens",
                        String::from_utf8_lossy(token)
                    )
                }),
            FormattedLocalization::Formatter { formatter, .. } => Err(format!(
                "faction bound localisation formatter {} requires core registry callback (142393810)",
                String::from_utf8_lossy(formatter)
            )),
        }
    }

    fn char_token_text(&self, token: u32) -> Result<&'f [u8], String> {
        self.character_table::<tables::LexicalTokens>()?
            .iter()
            .find(|row| row.token == token)
            .map(|row| row.literal.as_slice())
            .ok_or_else(|| format!("LexicalTokens lacks token {token}"))
    }
    // 140F8A750 searches the signed ideology-token map; 140F8A820 selects
    // its sole entry, not the first of several. CIdeology+8 is the lexical
    // token: 140A37930 constructs it from AddOrFindDynamicToken.
    pub(super) fn char_country_leader_scope(
        &self,
        character: &CharacterRow,
        ideology: Option<&[u8]>,
    ) -> Result<Option<&'f CountryLeaderRoleRow>, String> {
        let roles = self.character_table::<tables::CountryLeaderRoles>()?;
        let mut found = roles.iter().filter(|role| role.character == character.id);
        let Some(ideology) = ideology.filter(|ideology| !ideology.is_empty()) else {
            let first = found.next();
            return Ok(if found.next().is_none() { first } else { None });
        };
        let literal = String::from_utf8_lossy(ideology).to_ascii_lowercase();
        if self
            .politics_ideologies()?
            .group_of(&literal)
            .map_err(ToString::to_string)?
            .is_none()
        {
            return Ok(None);
        }
        let Some(token) = self.context.world()?.token_of(ideology)? else {
            return Ok(None);
        };
        Ok(found.find(|role| role.ideology as u32 == token))
    }

    // Political status +0xD0 -> 14118E5A0: first entry of its party vector.
    pub(super) fn char_ruling_leader(
        &self,
        country: u32,
    ) -> Result<Option<&'f CharacterRow>, String> {
        self.char_party_leader(country, None)
    }

    fn char_party_leader(
        &self,
        country: u32,
        ideology: Option<&[u8]>,
    ) -> Result<Option<&'f CharacterRow>, String> {
        let Some(leader) = self.char_party_leader_row(country, ideology)? else {
            return Ok(None);
        };
        let Some(character) = leader.character else {
            return Ok(None);
        };
        self.character_table::<tables::Characters>()?
            .iter()
            .find(|row| row.id == character)
            .map(Some)
            .ok_or_else(|| format!("PartyLeaders references missing character {character}"))
    }

    fn char_party_leader_row(
        &self,
        country: u32,
        ideology: Option<&[u8]>,
    ) -> Result<Option<&'f PartyLeaderRow>, String> {
        let world = self.context.world()?;
        let parties = self.character_table::<tables::PoliticalParties>()?;
        let mut selected = parties.iter().filter(|party| {
            world.slot(party.country) == Some(country)
                && match ideology {
                    Some(token) => party.token.eq_ignore_ascii_case(token),
                    None => party.is_ruling != 0,
                }
        });
        let Some(party) = selected.next() else {
            return Ok(None);
        };
        if selected.next().is_some() {
            return Err("country has multiple selected political parties".into());
        }
        let leaders = self.character_table::<tables::PartyLeaders>()?;
        let Some(leader) = leaders
            .iter()
            .filter(|leader| {
                world.slot(leader.country) == Some(country) && leader.party == party.id
            })
            .min_by_key(|leader| leader.ordinal)
        else {
            return Ok(None);
        };
        if leader.ordinal != 0 {
            return Err("PartyLeaders lacks first native vector entry".into());
        }
        Ok(Some(leader))
    }

    // 140FB5FD0: +0x28, otherwise localise the original +0x48 string.
    pub(super) fn char_country_leader_display_name(
        &self,
        character: &CharacterRow,
        scopes: Scopes,
    ) -> Result<String, String> {
        if character.name.is_empty() {
            self.localised(&character.token, &[], scopes, false)
        } else {
            self.effect_character_name(character)
        }
    }

    fn char_role_header(
        &self,
        key: &[u8],
        name: &str,
        ideology: &[u8],
        traits: &[String],
        scopes: Scopes,
    ) -> Result<String, String> {
        // 14039B530: CIdeology+0x120 -> group GetLocalizedName (140616BC0).
        let token = String::from_utf8_lossy(ideology).to_ascii_lowercase();
        let db = self.politics_ideologies()?;
        let group = db.group_of(&token).map_err(ToString::to_string)?;
        let ideology = match group {
            Some(group) => self.localised(group.token.as_bytes(), &[], scopes, false)?,
            // Empty template-promote selector reaches the shared null
            // ideology: 140A39370 -> 140A394A0, whose constructor 14117BE20
            // sets +0x120 to the null group (14117C820). Its +0x18 name is
            // empty by 140A391F0; this native default needs no runtime input.
            None if token.is_empty() => self.localised(b"", &[], scopes, false)?,
            None => {
                return Err(format!(
                    "country-leader ideology {token} is absent from recorded content"
                ));
            }
        };
        let mut text = self.char_native_text(
            key,
            &[
                ("NAME", Argument::Text(name)),
                ("IDEOLOGY", Argument::Text(&ideology)),
            ],
            scopes,
            false,
        )?;
        text.push('\n');
        for token in traits {
            // 14271C710 four spaces; 14039B530 writes 0x11 H / 0x11 !;
            // 142727BB0 is the nested numeric separator "\n      - ".
            let Some(body) = self.char_country_trait_description_with_prefix(
                token.as_bytes(),
                scopes,
                "\n      - ",
            )?
            else {
                return Err(format!(
                    "country-leader trait {token} is absent from recorded content"
                ));
            };
            text.push_str("    \u{11}H");
            text.push_str(&self.localised(token.as_bytes(), &[], scopes, false)?);
            text.push_str("\u{11}!");
            if !body.is_empty() {
                text.push_str("\n      - ");
                text.push_str(&body);
            }
            text.push('\n');
        }
        Ok(text)
    }

    // CLegacyCreateCountryLeaderEffect own 140383B50, key 142750128.
    pub(in crate::effect_description) fn char_create_country_leader(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let raw = operand(item, "name").unwrap_or_default();
        let country = self.current_country(scopes)?;
        // 1403B8BF0: country prefix unless fourth byte is '_'; ASCII lower,
        // spaces become underscores, hyphens/apostrophes are removed.
        let key = if !raw.is_empty() && raw.get(3) != Some(&b'_') {
            let mut key = self.tag(country)?.as_bytes().to_vec();
            key.push(b'_');
            key.extend(
                raw.iter()
                    .filter(|byte| !matches!(byte, b'-' | b'\''))
                    .map(|byte| {
                        if *byte == b' ' {
                            b'_'
                        } else {
                            byte.to_ascii_lowercase()
                        }
                    }),
            );
            key
        } else {
            raw.to_vec()
        };
        let mut name = self.localised(&key, &[], scopes, false)?;
        if name.is_empty() {
            // 140383B50 calls 1410D6BF0 with include_retired=0; unlike
            // ResolveCharacter it has no current-character fallback.
            if let Some(token) = operand(item, "token")
                && let Some(token) = self.context.world()?.token_of(token)?
            {
                let world = self.context.world()?;
                let characters = self.character_table::<tables::Characters>()?;
                let character = self
                    .character_table::<tables::CountryCharacters>()?
                    .iter()
                    .filter(|row| world.slot(row.country) == Some(country))
                    .filter_map(|row| {
                        row.character.and_then(|id| {
                            characters
                                .iter()
                                .find(|character| {
                                    character.id == id && character.name_token_id as u32 == token
                                })
                                .map(|character| (row.ordinal, character))
                        })
                    })
                    .min_by_key(|(ordinal, _)| *ordinal)
                    .map(|(_, character)| character);
                if let Some(character) = character {
                    name = self.effect_character_name(character)?;
                }
            }
        }
        if name.as_bytes().get(3) == Some(&b'_') {
            name = String::from_utf8_lossy(raw).into_owned();
        }
        let role = installed_content::characters::country_leader_role("", children(item))?;
        self.char_role_header(
            b"EFFECT_ADD_COUNTRY_LEADER_ROLE_PROMOTE",
            &name,
            role.ideology.as_bytes(),
            &role.traits,
            scopes,
        )
    }

    // CAddCountryLeaderRoleEffect 140372110, reader 1403AEC00; key
    // EFFECT_ADD_COUNTRY_LEADER_ROLE[_PROMOTE] 142755278 / 142750128.
    pub(in crate::effect_description) fn char_add_country_leader_role(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Some(character) = self.effect_character_target(operand(item, "character"), scopes)?
        else {
            return Ok(String::new());
        };
        let role = installed_content::characters::country_leader_role(
            "",
            field(children(item), "country_leader")
                .map(children)
                .unwrap_or_default(),
        )?;
        let name = self.effect_character_name(character)?;
        self.char_role_header(
            if boolean(item, "promote_leader") {
                b"EFFECT_ADD_COUNTRY_LEADER_ROLE_PROMOTE"
            } else {
                b"EFFECT_ADD_COUNTRY_LEADER_ROLE"
            },
            &name,
            role.ideology.as_bytes(),
            &role.traits,
            scopes,
        )
    }

    // Own 140385DE0: recorded role before template fallback, traits keep
    // runtime order. Empty ideology is accepted only for one native role.
    pub(in crate::effect_description) fn char_promote_character(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        // 1403AA3C0: scope-kind 8 reads a scalar as ideology (exact yes
        // means empty), and its block accepts ideology but not character.
        let character_scope = matches!(scopes.this, Scope::Character(_) | Scope::Leader(_));
        let selector = if character_scope {
            None
        } else {
            item.value.scalar().or_else(|| operand(item, "character"))
        };
        let ideology = if character_scope {
            item.value
                .scalar()
                .filter(|value| installed_content::parser::c_string(value) != b"yes")
                .or_else(|| operand(item, "ideology"))
        } else {
            operand(item, "ideology")
        }
        .filter(|value| !value.is_empty());
        if let Some(character) = self.effect_character_target(selector, scopes)? {
            let Some(role) = self.char_country_leader_scope(character, ideology)? else {
                return Ok(String::new());
            };
            let ideology = match ideology {
                Some(value) => value.to_vec(),
                None => self.char_token_text(role.ideology as u32)?.to_vec(),
            };
            let traits = self
                .character_table::<tables::CharacterRoleTraits>()?
                .iter()
                .find(|row| {
                    row.character == character.id
                        && row.kind == CharacterRoleTraitRow::COUNTRY_LEADER
                        && row.role == role.ideology as u32
                })
                .map(|row| row.tokens.as_slice())
                .unwrap_or_default()
                .iter()
                .map(|token| {
                    self.char_token_text(*token)
                        .map(|text| String::from_utf8_lossy(text).into_owned())
                })
                .collect::<Result<Vec<_>, _>>()?;
            return self.char_role_header(
                b"EFFECT_ADD_COUNTRY_LEADER_ROLE_PROMOTE",
                &self.effect_character_name(character)?,
                &ideology,
                &traits,
                scopes,
            );
        }
        let Some(token) = selector.filter(|token| !token.is_empty()) else {
            return Ok(String::new());
        };
        let token = String::from_utf8_lossy(token).to_ascii_lowercase();
        let catalogue = self
            .character_content()?
            .catalogue
            .as_ref()
            .map_err(Clone::clone)?;
        let roles = catalogue.leader_roles(&token)?;
        // 1406ACC90 first clears both maps (1406AFB70), then registers
        // only startup-allowed templates. Agreement cannot prove presence;
        // absent authored entries may also fall through to generated +0x60.
        if !catalogue.template_has_unconditional_startup(&token) {
            return Err(format!(
                "character template {token} selected identity/presence (CCharacterTemplateDatabase+0x40/+0x60 tag entry+0x10, 1406B0920/1406ACC90) is unrecorded"
            ));
        }
        let Some(roles) = roles else {
            return Ok(String::new());
        };
        let role = match ideology {
            Some(ideology) => roles
                .iter()
                .find(|role| role.ideology.as_bytes().eq_ignore_ascii_case(ideology)),
            None if roles.len() == 1 => roles.first(),
            None => None,
        };
        let Some(role) = role else {
            return Ok(String::new());
        };
        let name = match catalogue.name(&token).map_err(ToString::to_string)? {
            // Template vf8 1413DC900 localises the authored +0x40 key
            // through 142230F20 and stores the result at +0x60. Promote
            // 140385DE0 copies that name without event-scope expansion.
            Some(Authored::Asset(key)) => self.localised(key.as_bytes(), &[], scopes, false)?,
            Some(Authored::Ambiguous(_)) => {
                return Err(format!(
                    "character template {token} selected identity (CCharacterTemplateDatabase+0x40/+0x60 tag entry+0x10, 1406B0920) is unrecorded: names differ across instances"
                ));
            }
            Some(Authored::Unknown(why)) => return Err(why.clone()),
            _ => String::new(),
        };
        self.char_role_header(
            b"EFFECT_ADD_COUNTRY_LEADER_ROLE_PROMOTE",
            &name,
            ideology.unwrap_or_default(),
            &role.traits,
            scopes,
        )
    }

    // Own 14038CDF0: ResolveCharacter, character country, expansion then LF.
    pub(in crate::effect_description) fn char_retire_character(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Some(character) = self.effect_character_target(item.value.scalar(), scopes)? else {
            return Ok(String::new());
        };
        let name = self.effect_character_name(character)?;
        let country = if character.country as i32 > 0 {
            self.country_name(
                self.context
                    .world()?
                    .slot(character.country)
                    .ok_or("Character country is absent from CountryLinks")?,
                false,
            )?
        } else {
            String::new()
        };
        self.char_native_line(
            b"EFFECT_RETIRE_CHARACTER",
            &[
                ("NAME", Argument::Text(&name)),
                ("COUNTRY", Argument::Text(&country)),
            ],
            scopes,
            true,
        )
    }

    // Own 14038D050/140382E90 (ruling); 14038D350/140383190
    // (ResolveIdeologySpecifier 1402EE8E0); all expand then append LF.
    pub(in crate::effect_description) fn char_remove_party_leader(
        &self,
        item: &Item,
        scopes: Scopes,
        ideology: bool,
        kill: bool,
    ) -> Result<String, String> {
        let country = self.current_country(scopes)?;
        let ideology = ideology
            .then(|| self.politics_ideology(item.value.scalar(), scopes))
            .transpose()?;
        let Some(leader) = self.char_party_leader_row(country, ideology.as_deref())? else {
            return Ok(String::new());
        };
        let name = match leader.character {
            Some(id) => {
                let character = self
                    .character_table::<tables::Characters>()?
                    .iter()
                    .find(|row| row.id == id)
                    .ok_or_else(|| format!("PartyLeaders references missing character {id}"))?;
                self.char_country_leader_display_name(character, scopes)?
            }
            None => String::new(),
        };
        self.char_native_line(
            if kill {
                b"EFFECT_KILL_COUNTRY_LEADER"
            } else {
                b"EFFECT_RETIRE_COUNTRY_LEADER"
            },
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        )
    }

    // Own 140388070: missing character/undefined ideology are native empty;
    // IDEOLOGY is the subgroup's group name (140616BC0), no LF.
    pub(in crate::effect_description) fn char_remove_country_leader_role(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Some(character) = self.effect_character_target(operand(item, "character"), scopes)?
        else {
            return Ok(String::new());
        };
        let token = String::from_utf8_lossy(operand(item, "ideology").unwrap_or_default())
            .to_ascii_lowercase();
        let db = self.politics_ideologies()?;
        let Some(group) = db.group_of(&token).map_err(ToString::to_string)? else {
            return Ok(String::new());
        };
        let ideology = self.localised(group.token.as_bytes(), &[], scopes, false)?;
        let name = self.effect_character_name(character)?;
        self.char_native_text(
            b"EFFECT_REMOVE_COUNTRY_LEADER_ROLE",
            &[
                ("NAME", Argument::Text(&name)),
                ("IDEOLOGY", Argument::Text(&ideology)),
            ],
            scopes,
            true,
        )
    }

    // Own 1403879E0: localise/expand SLOT first, then the containing line.
    pub(in crate::effect_description) fn char_remove_advisor_role(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Some(character) = self.effect_character_target(operand(item, "character"), scopes)?
        else {
            return Ok(String::new());
        };
        let name = self.effect_character_name(character)?;
        let slot = self.localised(operand(item, "slot").unwrap_or_default(), &[], scopes, true)?;
        self.char_native_text(
            b"EFFECT_REMOVE_ADVISOR_ROLE",
            &[
                ("NAME", Argument::Text(&name)),
                ("SLOT", Argument::Text(&slot)),
            ],
            scopes,
            true,
        )
    }

    // Own 140370790, reader 1403AEA80 -> CAdvisorTemplate 1413CA970.
    // A temporary accumulator sums each trait+0x98, then template+0xF8,
    // through AddFilteredModifierBlock 14054AD00; no trait names/bonuses.
    pub(in crate::effect_description) fn char_add_advisor_role(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Some(character) = self.effect_character_target(operand(item, "character"), scopes)?
        else {
            return Ok(String::new());
        };
        let role = installed_content::characters::advisor_role(
            "",
            field(children(item), "advisor")
                .map(children)
                .unwrap_or_default(),
        )?;
        let name = self.effect_character_name(character)?;
        let slot = self.localised(role.slot.as_bytes(), &[], scopes, true)?;
        let traits = self.char_country_traits()?;
        let mut modifiers = Modifiers::default();
        for source in role
            .traits
            .iter()
            .map(|name| {
                traits
                    .get(name)
                    .map_err(ToString::to_string)?
                    .ok_or_else(|| {
                        format!("country-leader trait {name} is absent from recorded content")
                    })
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(|trait_| &trait_.modifiers)
            .chain(std::iter::once(&role.modifiers))
        {
            if let Some(why) = source.unheld.cause() {
                return Err(why.to_owned());
            }
            for value in &source.values {
                // 14054AE00 multiplies in native signed 64-bit arithmetic
                // before dividing by fixed5, even at the unit scale.
                let contribution = value.value.wrapping_mul(100_000) / 100_000;
                if let Some(previous) = modifiers
                    .values
                    .iter_mut()
                    .find(|previous| previous.name == value.name)
                {
                    previous.value = previous.value.wrapping_add(contribution);
                } else {
                    let mut value = value.clone();
                    value.value = contribution;
                    modifiers.values.push(value)
                }
            }
        }
        // 140370790 calls plain FormatDescription 14054E050 with full
        // category mask, precision 2, then expands only its containing line.
        let modifiers = self.modifier_description_with(
            &modifiers,
            "",
            ModifierListOptions {
                category_mask: u32::MAX,
                ..Default::default()
            },
        )?;
        self.char_native_text(
            if boolean(item, "activate") {
                b"EFFECT_ADD_ADVISOR_ROLE_APPOINT"
            } else {
                b"EFFECT_ADD_ADVISOR_ROLE"
            },
            &[
                ("NAME", Argument::Text(&name)),
                ("SLOT", Argument::Text(&slot)),
                ("MODIFIER", Argument::Text(&modifiers)),
            ],
            scopes,
            true,
        )
    }

    // Own 14036F000/14037E3B0 -> 1410D69C0 searches the available
    // advisor roster before retired characters. Neither persistent roles nor
    // appointments proves that primary lookup. Cache rebuild 14128FF90 also
    // omits the own modifier that factory 141289CF0/load 14128E7D0 adds;
    // recorded current traits cannot distinguish those refresh histories.
    pub(in crate::effect_description) fn char_activate_advisor(
        &self,
        _item: &Item,
        _scopes: Scopes,
    ) -> Result<String, String> {
        Err("advisor lookup requires the unrecorded available-advisor roster (CCountryCharacters+0x40/+0x4C, 1410D69C0) and cached modifiers (CAdvisor+0x1A0, 14036F000/14037E3B0)".into())
    }

    // Own 14038F4D0's inverted guard is preserved: THIS character omits its
    // name; a resolved explicit character without a THIS character adds a
    // trailing space. SLOT@142765564 wraps nonempty text in "(" + ") ".
    pub(in crate::effect_description) fn char_set_can_be_fired(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let mut name = String::new();
        if self.effect_character(scopes.this)?.is_none()
            && let Some(character) =
                self.effect_character_target(operand(item, "character"), scopes)?
        {
            name = self.effect_character_name(character)?;
            name.push(' ');
        }
        let slot = match operand(item, "slot").filter(|value| !value.is_empty()) {
            Some(slot) => format!("({}) ", self.localised(slot, &[], scopes, true)?),
            None => String::new(),
        };
        self.char_native_text(
            if boolean(item, "value") {
                b"EFFECT_SET_CAN_BE_FIRED"
            } else {
                b"EFFECT_SET_CANNOT_BE_FIRED"
            },
            &[
                ("CHARACTER", Argument::Text(&name)),
                ("SLOT", Argument::Text(&slot)),
            ],
            scopes,
            true,
        )
    }

    // Own 140386C40: registry lookup by copied LexerToken id (reader
    // 1403AAA50), not country roster; character +0x28 NAME, expansion + LF.
    pub(in crate::effect_description) fn char_recruit_character(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Some(token) = self
            .context
            .world()?
            .token_of(item.value.scalar().unwrap_or_default())?
        else {
            return Ok(String::new());
        };
        let mut matching = self
            .character_table::<tables::Characters>()?
            .iter()
            .filter(|row| row.name_token_id as u32 == token);
        let Some(character) = matching.next() else {
            return Ok(String::new());
        };
        if matching.next().is_some() {
            return Err("character registry token resolves multiple recorded rows".into());
        }
        self.char_native_line(
            b"EFFECT_RECRUIT_CHARACTER",
            &[(
                "NAME",
                Argument::Text(&self.effect_character_name(character)?),
            )],
            scopes,
            true,
        )
    }

    // Own 140370540: template +0x58/+0x78/+0x98 are copied, no naming
    // lookup and no LF. EFFECT_ADD_ACE@142759E70.
    pub(in crate::effect_description) fn char_add_ace(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let name = String::from_utf8_lossy(operand(item, "name").unwrap_or_default());
        let surname = String::from_utf8_lossy(operand(item, "surname").unwrap_or_default());
        let callsign = String::from_utf8_lossy(operand(item, "callsign").unwrap_or_default());
        self.char_native_text(
            b"EFFECT_ADD_ACE",
            &[
                ("NAME", Argument::Text(&name)),
                ("SURNAME", Argument::Text(&surname)),
                ("CALLSIGN", Argument::Text(&callsign)),
            ],
            scopes,
            true,
        )
    }

    // Own 1404AEB90: positive country, faction and not its first member;
    // CFaction::display_name 140D77360, expand + LF.
    pub(in crate::effect_description) fn char_set_faction_leader(
        &self,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Scope::Country(country) = scopes.this else {
            return Ok(String::new());
        };
        if country == 0 {
            return Ok(String::new());
        }
        let world = self.context.world()?;
        let members = self.character_table::<tables::FactionMembers>()?;
        let Some(member) = members
            .iter()
            .find(|member| world.slot(member.country) == Some(country))
        else {
            return Ok(String::new());
        };
        let Some(first) = members
            .iter()
            .filter(|other| other.faction == member.faction)
            .min_by_key(|member| member.ordinal)
        else {
            return Ok(String::new());
        };
        if first.ordinal != 0 {
            return Err("FactionMembers lacks first native member".into());
        }
        if world.slot(first.country) == Some(country) {
            return Ok(String::new());
        }
        let faction = self
            .character_table::<tables::Factions>()?
            .iter()
            .find(|row| row.id == member.faction)
            .ok_or("Factions lacks member's faction")?;
        let name = if !faction.name.is_empty() {
            crate::scripted_text::stored_text(
                self.context.localisation,
                self.context.text.recorded_language(),
                &faction.name,
                "faction name (140D77360)",
            )?
        } else if let Some(token) = faction.template_token {
            let token = std::str::from_utf8(self.char_token_text(token)?)
                .map_err(|_| "faction template token is not UTF-8")?;
            let bound = self
                .character_content()?
                .factions
                .as_ref()
                .map_err(Clone::clone)?
                .get(token)?
                .ok_or_else(|| {
                    format!("faction template {token} is absent from recorded content")
                })?;
            if bound.is_empty() {
                String::new()
            } else {
                match self.char_faction_bound_node(bound, 0, scopes)? {
                    FactionArgument::Text(text) => text,
                    FactionArgument::Token(_) => return Err("faction bound root returns a token integer, not the native string variant (14238F090)".into()),
                }
            }
        } else {
            String::new()
        };
        self.char_native_line(
            b"EFFECT_SET_FACTION_LEADER",
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        )
    }
}
