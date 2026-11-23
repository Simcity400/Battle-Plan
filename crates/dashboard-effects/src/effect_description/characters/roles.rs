
use super::*;
use content_reader_key_names::characters::Authored;
use content_reader_script::modifiers::Modifiers;
use content_reader_script::localization_binding::{BoundLocalization, BoundValue, FormattedLocalization};
use replay_data::rows::{
    AdvisorRoleRow, CharacterRoleTraitRow, CountryLeaderRoleRow, FactionMemberRow, FactionRow,
    PartyLeaderRow, PoliticalPartyRow,
};

const LEADER_ROLE: u64 = tables::CountryLeaderRoles::ID.full_mask();
const PARTY_PICK: u64 = mask::<PoliticalPartyRow>(&["token", "is_ruling"]);
const PARTY_LEADER: u64 = tables::PartyLeaders::ID.full_mask();
const ROLE_TOKENS: u64 = mask::<CharacterRoleTraitRow>(&["tokens"]);
const ROLE_TRAITS: u64 = mask::<CharacterRoleTraitRow>(&["definitions"]);
const ADVISOR: u64 = mask::<AdvisorRoleRow>(&["idea", "modifier_cache", "tooltip_keys"]);
const MEMBER: u64 = mask::<FactionMemberRow>(&["faction", "ordinal"]);
const FACTION_NAME: u64 = mask::<FactionRow>(&["name", "template_token"]);
const ROLE_TOKENS_COLUMN: usize = replay_data::rows::column::<CharacterRoleTraitRow>("tokens");
const ROLE_DEFINITIONS_COLUMN: usize =
    replay_data::rows::column::<CharacterRoleTraitRow>("definitions");

#[cfg(test)]
mod tests;

fn operand<'a>(item: &'a Item, name: &str) -> Option<&'a [u8]> {
    field(children(item), name).and_then(|field| field.value.scalar())
}

fn boolean(item: &Item, name: &str) -> bool {
    operand(item, name)
        .and_then(content_reader_parser::parser::read_bool)
        .unwrap_or(false)
}

enum BoundResult {
    Text(String),
    Token(i32),
}

impl<'r, 'c, 'f> Renderer<'r, 'c, 'f> {
    pub(in crate::effect_description) fn bound_localisation(
        &mut self,
        bound: &BoundLocalization,
        scopes: Scopes,
        scoped: bool,
    ) -> Result<String, String> {
        if bound.is_empty() {
            return Ok(String::new());
        }
        match self.bound_node(bound, 0, scopes, scoped)? {
            BoundResult::Text(text) => Ok(text),
            BoundResult::Token(_) => Err(
                "bound localisation root returns a token integer, not the string variant".into(),
            ),
        }
    }

    fn bound_node(
        &mut self,
        bound: &BoundLocalization,
        index: usize,
        scopes: Scopes,
        scoped: bool,
    ) -> Result<BoundResult, String> {
        let node = bound
            .nodes
            .get(index)
            .ok_or("bound localisation node is absent")?;
        let mut arguments = Vec::with_capacity(node.arguments.len());
        let mut linked = Vec::new();
        for argument in &node.arguments {
            let value = match &argument.value {
                BoundValue::Localization(value) => {
                    self.bound_formatted(value, &[], scopes, scoped)?
                }
                BoundValue::Node(child) => self.bound_node(bound, *child, scopes, scoped)?,
            };
            let name = std::str::from_utf8(&argument.name)
                .map_err(|_| "bound argument name is not UTF-8")?;
            if let (BoundResult::Text(text), BoundValue::Localization(FormattedLocalization::Key(key))) = (&value, &argument.value)
                && let Some(token) = self.bound_technology(key)?
            {
                linked.push((arguments.len(), crate::effect_description::entity_links::mark(EntityKind::Technology, token, text)));
            }
            arguments.push((name, value));
        }
        let plain = arguments
            .iter()
            .map(|(name, value)| {
                (
                    *name,
                    match value {
                        BoundResult::Text(text) => Argument::Text(text),
                        BoundResult::Token(token) => Argument::Int32(*token),
                    },
                )
            })
            .collect::<Vec<_>>();
        if linked.is_empty() {
            return self.bound_formatted(&node.key, &plain, scopes, scoped);
        }
        let unmarked: Vec<Option<String>> = arguments
            .iter()
            .map(|(_, value)| match value {
                BoundResult::Text(text) => Some(crate::effect_description::entity_links::unmark(text)),
                BoundResult::Token(_) => None,
            })
            .collect();
        let unmarked: Vec<(&str, Argument<'_>)> = plain
            .iter()
            .zip(&unmarked)
            .map(|((name, argument), text)| (*name, text.as_deref().map_or(*argument, Argument::Text)))
            .collect();
        let text = self.bound_formatted(&node.key, &unmarked, scopes, scoped)?;
        let mut marked = plain.clone();
        for (at, name) in &linked {
            marked[*at].1 = Argument::Text(name);
        }
        match (self.bound_formatted(&node.key, &marked, scopes, scoped)?, &text) {
            (BoundResult::Text(with), BoundResult::Text(without))
                if crate::effect_description::entity_links::unmark(&with) == *without =>
            {
                Ok(BoundResult::Text(with))
            }
            _ => Err(
                "the game's formatting changes the linked technology names in this text, so their links are not reproduced"
                    .into(),
            ),
        }
    }

    fn bound_technology<'k>(&self, key: &'k [u8]) -> Result<Option<&'k str>, String> {
        if !self.context.entity_links || self.technology_catalog()?.get(key)?.is_none() {
            return Ok(None);
        }
        std::str::from_utf8(content_reader_parser::parser::c_string(key))
            .map(Some)
            .map_err(|_| "the bound technology key is not UTF-8".into())
    }

    fn bound_formatted(
        &mut self,
        value: &FormattedLocalization,
        arguments: &[(&str, Argument<'_>)],
        scopes: Scopes,
        scoped: bool,
    ) -> Result<BoundResult, String> {
        match value {
            FormattedLocalization::Key(key) => self
                .localised(key, arguments, scopes, scoped)
                .map(BoundResult::Text),
            FormattedLocalization::Token(token) => self
                .context
                .world()?
                .token_of(token)
                .map_err(|why| why.to_string())?
                .map(|token| BoundResult::Token(token as i32))
                .ok_or_else(|| {
                    format!(
                        "bound localisation token {} is absent from lexical tokens",
                        String::from_utf8_lossy(token)
                    )
                }),
            FormattedLocalization::Formatter { formatter, token } => {
                let token =
                    std::str::from_utf8(token).map_err(|_| "bound formatter token is not UTF-8")?;
                let text = match formatter.as_slice() {
                    b"idea_desc" if scoped => self.bound_idea_description(token, scopes)?,
                    b"tech_effect" if scoped => self.bound_technology_effect(token, None, scopes)?,
                    b"idea_desc" | b"tech_effect" => "NULL".to_owned(),
                    _ => {
                        return Err(format!(
                            "bound localisation formatter {} requires core registry callback",
                            String::from_utf8_lossy(formatter)
                        ));
                    }
                };
                Ok(BoundResult::Text(text))
            }
        }
    }

    fn char_token_text(&self, token: u32) -> Result<&'f [u8], String> {
        crate::scripted_text::spelled(&self.character_lexical()?, token)?
            .ok_or_else(|| format!("lexical tokens lacks token {token}"))
    }
    pub(super) fn char_country_leader_scope(
        &self,
        character: &CharacterRow,
        ideology: Option<&[u8]>,
    ) -> Result<Option<Cow<'f, CountryLeaderRoleRow>>, String> {
        let roles = self.character_table::<tables::CountryLeaderRoles>(LEADER_ROLE)?;
        let low = u128::from(character.id) << 32;
        let mut found = each(roles.span(low, low | u128::from(u32::MAX)));
        let Some(ideology) = ideology.filter(|ideology| !ideology.is_empty()) else {
            let first = found.next().transpose()?;
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
        let Some(token) = self
            .context
            .world()?
            .token_of(ideology)
            .map_err(|why| why.to_string())?
        else {
            return Ok(None);
        };
        for role in found {
            let role = role?;
            if role.ideology as u32 == token {
                return Ok(Some(role));
            }
        }
        Ok(None)
    }

    pub(super) fn char_ruling_leader(
        &self,
        country: u32,
    ) -> Result<Option<Cow<'f, CharacterRow>>, String> {
        self.char_party_leader(country, None)
    }

    fn char_party_leader(
        &self,
        country: u32,
        ideology: Option<&[u8]>,
    ) -> Result<Option<Cow<'f, CharacterRow>>, String> {
        let Some(leader) = self.char_party_leader_row(country, ideology)? else {
            return Ok(None);
        };
        let Some(character) = leader.character else {
            return Ok(None);
        };
        self.character_row(character)?
            .map(Some)
            .ok_or_else(|| format!("party leaders references missing character {character}"))
    }

    fn char_party_leader_row(
        &self,
        country: u32,
        ideology: Option<&[u8]>,
    ) -> Result<Option<Cow<'f, PartyLeaderRow>>, String> {
        let world = self.context.world()?;
        let parties = self.character_table::<tables::PoliticalParties>(PARTY_PICK)?;
        let mut selected = None;
        for party in parties.keyed(|key| world.slot((key >> 32) as u32) == Some(country)) {
            let party = party.map_err(built)?;
            if match ideology {
                Some(token) => party.token.eq_ignore_ascii_case(token),
                None => party.is_ruling != 0,
            } {
                if selected.is_some() {
                    return Err("country has multiple selected political parties".into());
                }
                selected = Some(party.id);
            }
        }
        let Some(party) = selected else {
            return Ok(None);
        };
        let leaders = self.character_table::<tables::PartyLeaders>(0)?;
        let Some(key) = leaders
            .keys()
            .filter(|&key| {
                world.slot((key >> 64) as u32) == Some(country) && (key >> 32) as u32 == party
            })
            .min_by_key(|&key| key as u32)
        else {
            return Ok(None);
        };
        if key as u32 != 0 {
            return Err("party leaders lacks first native vector entry".into());
        }
        keyed(&leaders.with_mask(PARTY_LEADER), key)
    }

    pub(super) fn char_country_leader_display_name(
        &self,
        character: &CharacterRow,
        scopes: Scopes,
    ) -> Result<String, String> {
        if character.name.is_empty() {
            self.localised(&character.token, &[], scopes, false)
        } else {
            self.effect_character_name(&character)
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
        let token = String::from_utf8_lossy(ideology).to_ascii_lowercase();
        let db = self.politics_ideologies()?;
        let group = db.group_of(&token).map_err(ToString::to_string)?;
        let ideology = match group {
            Some(group) => self.localised(group.token.as_bytes(), &[], scopes, false)?,
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

    pub(in crate::effect_description) fn char_create_country_leader(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let raw = operand(item, "name").unwrap_or_default();
        let country = self.current_country(scopes)?;
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
            if let Some(token) = operand(item, "token")
                && let Some(token) = self
                    .context
                    .world()?
                    .token_of(token)
                    .map_err(|why| why.to_string())?
            {
                let world = self.context.world()?;
                let characters = self.character_rows()?;
                let mut character: Option<(u32, Cow<'f, CharacterRow>)> = None;
                for row in self
                    .character_table::<tables::CountryCharacters>(ROSTER)?
                    .keyed(|key| world.slot((key >> 32) as u32) == Some(country))
                {
                    let row = row.map_err(built)?;
                    let Some(id) = row.character else { continue };
                    let Some(found) = keyed(&characters, u128::from(id))? else {
                        continue;
                    };
                    if found.name_token_id as u32 == token
                        && character
                            .as_ref()
                            .is_none_or(|(ordinal, _)| row.ordinal < *ordinal)
                    {
                        character = Some((row.ordinal, found));
                    }
                }
                let character = character.map(|(_, character)| character);
                if let Some(character) = character {
                    name = self.effect_character_name(&character)?;
                }
            }
        }
        if name.as_bytes().get(3) == Some(&b'_') {
            name = String::from_utf8_lossy(raw).into_owned();
        }
        let role = content_reader_key_names::characters::country_leader_role("", children(item))?;
        self.char_role_header(
            b"EFFECT_ADD_COUNTRY_LEADER_ROLE_PROMOTE",
            &name,
            role.ideology.as_bytes(),
            &role.traits,
            scopes,
        )
    }

    pub(in crate::effect_description) fn char_add_country_leader_role(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Some(character) = self.effect_character_target(operand(item, "character"), scopes)?
        else {
            return Ok(String::new());
        };
        let role = content_reader_key_names::characters::country_leader_role(
            "",
            field(children(item), "country_leader")
                .map(children)
                .unwrap_or_default(),
        )?;
        let name = self.effect_character_name(&character)?;
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

    pub(in crate::effect_description) fn char_promote_character(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let character_scope = matches!(
            scopes.this,
            Scope::Character(_) | Scope::Leader(_) | Scope::CountryLeader { .. }
        );
        let selector = if character_scope {
            None
        } else {
            item.value.scalar().or_else(|| operand(item, "character"))
        };
        let ideology = if character_scope {
            item.value
                .scalar()
                .filter(|value| content_reader_parser::parser::c_string(value) != b"yes")
                .or_else(|| operand(item, "ideology"))
        } else {
            operand(item, "ideology")
        }
        .filter(|value| !value.is_empty());
        if let Some(character) = self.effect_character_target(selector, scopes)? {
            let Some(role) = self.char_country_leader_scope(&character, ideology)? else {
                return Ok(String::new());
            };
            let ideology = match ideology {
                Some(value) => value.to_vec(),
                None => self.char_token_text(role.ideology as u32)?.to_vec(),
            };
            let role_key = u128::from(character.id) << 40
                | u128::from(CharacterRoleTraitRow::COUNTRY_LEADER) << 32
                | u128::from(role.ideology as u32);
            let traits = self
                .character_table::<tables::CharacterRoleTraits>(ROLE_TOKENS)?
                .u32s(role_key, |row| &row.tokens, ROLE_TOKENS_COLUMN)
                .map_err(built)?
                .unwrap_or_default()
                .iter()
                .map(|token| {
                    self.char_token_text(*token)
                        .map(|text| String::from_utf8_lossy(text).into_owned())
                })
                .collect::<Result<Vec<_>, _>>()?;
            return self.char_role_header(
                b"EFFECT_ADD_COUNTRY_LEADER_ROLE_PROMOTE",
                &self.effect_character_name(&character)?,
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
        if !catalogue.template_has_unconditional_startup(&token) {
            return Err(format!(
                "character template {token} selected identity/presence is unrecorded"
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
            Some(Authored::Asset(key)) => self.localised(key.as_bytes(), &[], scopes, false)?,
            Some(Authored::Ambiguous(_)) => {
                return Err(format!(
                    "character template {token} selected identity is unrecorded: names differ across instances"
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

    pub(in crate::effect_description) fn char_retire_character(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Some(character) = self.effect_character_target(item.value.scalar(), scopes)? else {
            return Ok(String::new());
        };
        let name = self.effect_character_name(&character)?;
        let country = if character.country as i32 > 0 {
            self.country_name(
                self.context
                    .world()?
                    .slot(character.country)
                    .ok_or("Character country is absent from country links")?,
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
                    .character_row(id)?
                    .ok_or_else(|| format!("party leaders references missing character {id}"))?;
                self.char_country_leader_display_name(&character, scopes)?
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
        let name = self.effect_character_name(&character)?;
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

    pub(in crate::effect_description) fn char_remove_advisor_role(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Some(character) = self.effect_character_target(operand(item, "character"), scopes)?
        else {
            return Ok(String::new());
        };
        let name = self.effect_character_name(&character)?;
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

    pub(in crate::effect_description) fn char_add_advisor_role(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Some(character) = self.effect_character_target(operand(item, "character"), scopes)?
        else {
            return Ok(String::new());
        };
        let role = content_reader_key_names::characters::advisor_role(
            "",
            field(children(item), "advisor")
                .map(children)
                .unwrap_or_default(),
        )?;
        let name = self.effect_character_name(&character)?;
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

    pub(in crate::effect_description) fn char_activate_advisor(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let idea = std::str::from_utf8(content_reader_parser::parser::c_string(
            item.value.scalar().unwrap_or_default(),
        ))
        .map_err(|_| "advisor idea token is not UTF-8")?;
        let Some(character_id) = self.advisor_idea_character(idea, scopes)? else {
            return Ok(String::new());
        };
        let low = u128::from(character_id) << 32;
        let mut matching = Vec::new();
        for role in each(
            self.character_table::<tables::AdvisorRoles>(ADVISOR)?
                .span(low, low | u128::from(u32::MAX)),
        ) {
            let role = role?;
            if role.idea.eq_ignore_ascii_case(idea.as_bytes()) {
                matching.push(role);
            }
        }
        let advisor = matching
            .iter()
            .min_by_key(|role| role.ordinal)
            .ok_or_else(|| format!("advisor roles lacks character {character_id} idea {idea}"))?;
        let mut exact = matching
            .iter()
            .filter(|role| role.idea == idea.as_bytes());
        let first_exact = exact.next();
        if exact.next().is_some() || first_exact.is_some_and(|role| role.ordinal != advisor.ordinal)
        {
            return Err(format!(
                "advisor {character_id} / {idea} has ambiguous role identity after character-only lookup"
            ));
        }
        let cache = advisor
            .modifier_cache
            .require("advisor modifier cache")?
            .as_ref()
            .ok_or("advisor modifier cache capture unavailable")?;
        let character = self
            .effect_character(Scope::Character(advisor.character))?
            .ok_or("advisor roles references missing character")?;
        let description = if matches!(
            advisor.tooltip_keys,
            replay_data::rows::Recorded::NotRecorded
        ) {
            let catalogue = self
                .character_content()?
                .catalogue
                .as_ref()
                .map_err(Clone::clone)?;
            let idea_token = std::str::from_utf8(&advisor.idea)
                .map_err(|_| "advisor idea token is not UTF-8")?;
            let built = crate::identity::built_template_in(&character, self.character_lexical().ok());
            let role = catalogue
                .held_advisor(idea_token, built.as_ref().map_err(String::as_str))?
                .ok_or_else(|| format!("advisor role {idea_token} is absent from content"))?;
            let mut own_blocks = vec![&role.modifiers];
            while let Some(block) = own_blocks.pop() {
                if let Some(why) = block.unheld.cause() {
                    return Err(why.to_owned());
                }
                if !block.tooltips().is_empty() {
                    return Err(
                        "advisor own-block custom tooltip presence depends on refresh history"
                            .into(),
                    );
                }
                own_blocks.extend(&block.children);
            }
            let mut modifiers = Modifiers::default();
            let mut tooltip_key: Option<String> = None;
            let role_key = u128::from(advisor.character) << 40
                | u128::from(CharacterRoleTraitRow::ADVISOR) << 32
                | u128::from(advisor.ordinal);
            if let Some(definitions) = self
                .character_table::<tables::CharacterRoleTraits>(ROLE_TRAITS)?
                .u32s(role_key, |row| &row.definitions, ROLE_DEFINITIONS_COLUMN)
                .map_err(crate::effect_description::built)?
            {
                let traits = self.char_country_traits()?;
                for &definition in definitions {
                    let trait_ = traits.by_ordinal(definition).ok_or_else(|| {
                        format!("advisor trait definition {definition} is absent from content")
                    })?;
                    if let Some(why) = trait_.modifiers.unheld.cause() {
                        return Err(why.to_owned());
                    }
                    for key in trait_.modifiers.tooltips() {
                        if tooltip_key
                            .as_ref()
                            .is_some_and(|previous| previous != &key)
                        {
                            return Err(
                                "advisor cached tooltip keys/slot order not recorded".into()
                            );
                        }
                        if tooltip_key.is_none() {
                            modifiers = trait_.modifiers.clone();
                            tooltip_key = Some(key);
                        }
                    }
                }
            }
            modifiers.values.clear();
            modifiers.hidden.clear();
            modifiers.children.clear();
            let options = ModifierListOptions {
                category_mask: u32::MAX,
                ..Default::default()
            };
            let mut description =
                self.cached_modifier_description_with(&cache.entries, "", options)?;
            if tooltip_key.is_some() {
                let tooltip = self.modifier_description_with(&modifiers, "", options)?;
                if !tooltip.is_empty() {
                    if !description.is_empty() && !description.ends_with('\n') {
                        description.push('\n');
                    }
                    description.push_str(&tooltip);
                }
            }
            description
        } else {
            self.advisor_cache_description(advisor)?
        };
        let name = self.effect_character_name(&character)?;
        self.char_native_text(
            if item.keyword_is("deactivate_advisor") {
                b"EFFECT_DEACTIVATE_ADVISOR"
            } else {
                b"EFFECT_ASSIGN_ADVISOR"
            },
            &[
                ("NAME", Argument::Text(&name)),
                ("DESC", Argument::Text(&description)),
            ],
            scopes,
            true,
        )
    }

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
            name = self.effect_character_name(&character)?;
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

    pub(in crate::effect_description) fn char_recruit_character(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Some(token) = self
            .context
            .world()?
            .token_of(item.value.scalar().unwrap_or_default())
            .map_err(|why| why.to_string())?
        else {
            return Ok(String::new());
        };
        let mut character = None;
        for row in each(self.character_rows()?) {
            let row = row?;
            if row.name_token_id as u32 == token {
                if character.is_some() {
                    return Err("character registry token resolves multiple recorded rows".into());
                }
                character = Some(row);
            }
        }
        let Some(character) = character else {
            return Ok(String::new());
        };
        self.char_native_line(
            b"EFFECT_RECRUIT_CHARACTER",
            &[(
                "NAME",
                Argument::Text(&self.effect_character_name(&character)?),
            )],
            scopes,
            true,
        )
    }

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

    pub(in crate::effect_description) fn char_set_faction_leader(
        &mut self,
        scopes: Scopes,
    ) -> Result<String, String> {
        let (Scope::Country(country) | Scope::CountryLeader { country, .. }) = scopes.this else {
            return Ok(String::new());
        };
        if country == 0 {
            return Ok(String::new());
        }
        let world = self.context.world()?;
        let members = self.character_table::<tables::FactionMembers>(MEMBER)?;
        let Some(member) = members
            .keyed(|key| world.slot(key as u32) == Some(country))
            .next()
            .transpose()
            .map_err(built)?
        else {
            return Ok(String::new());
        };
        let mut first: Option<Cow<'f, FactionMemberRow>> = None;
        for other in each(members) {
            let other = other?;
            if other.faction == member.faction
                && first.as_ref().is_none_or(|first| other.ordinal < first.ordinal)
            {
                first = Some(other);
            }
        }
        let Some(first) = first else {
            return Ok(String::new());
        };
        if first.ordinal != 0 {
            return Err("faction members lacks first native member".into());
        }
        if world.slot(first.country) == Some(country) {
            return Ok(String::new());
        }
        let faction = keyed(
            &self.character_table::<tables::Factions>(FACTION_NAME)?,
            u128::from(member.faction),
        )?
        .ok_or("Factions lacks member's faction")?;
        let name = if !faction.name.is_empty() {
            crate::scripted_text::stored_text(
                self.context.localisation,
                self.context.text.recorded_language(),
                &faction.name,
                "faction name",
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
                .ok_or_else(|| format!("faction template {token} is absent from recorded content"))?
                .clone();
            self.bound_localisation(&bound, scopes, false)?
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
