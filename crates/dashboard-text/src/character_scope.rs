//! The recorded operands of the `tag`, `skill` and `has_trait` triggers in
//! a character scope. The tactic evaluator and the shared trigger evaluator
//! ([`crate::triggers`]) both read a character through these tables: its unit
//! leader (UnitLeaders or CharacterOperatives), its retired records
//! (CountryRetiredCharacters) and its roles' traits (CharacterRoleTraits).

use crate::state_resources::Links;
use installed_content_core::{leader_traits::LeaderTraits, unit_leader::Traits};
use replay_data::{
    rows::CharacterOperativeRow, rows::CharacterRoleTraitRow, rows::CharacterRow,
    rows::CountryCharacterRow, rows::UnitLeaderRow, rows::UnitLeaderTraitRow, rows::tables,
};
use replay_frame::frame::DecodedFrame;

type Evaluation<T> = Result<T, String>;

/// The fields of each table [`Tables`] reads, by
/// [`replay_data::rows::TableId::field_name`]: the row keys besides, and nothing
/// else of their rows. An evaluation reading through [`Tables`] reads these
/// ([`crate::triggers::inputs::fields`]).
pub const FIELDS: [(replay_data::rows::TableId, &[&str]); 6] = {
    use replay_data::rows::TableId;
    [
        (TableId::Characters, &["country", "name_token_id"]),
        (
            TableId::UnitLeaders,
            &[
                "character_id",
                "category",
                "owner",
                "skill_status",
                "skill_level",
            ],
        ),
        (TableId::UnitLeaderTraits, &["definition_ordinal"]),
        (TableId::CharacterRoleTraits, &["tokens", "definitions"]),
        (
            TableId::CharacterOperatives,
            &[
                "resolved",
                "category",
                "owner",
                "skill_level",
                "trait_definitions",
            ],
        ),
        (TableId::CountryRetiredCharacters, &["character"]),
    ]
};

/// The recorded character tables a character scope's `has_trait`, `skill`
/// and `tag` read.
pub struct Tables<'a> {
    characters: Result<&'a [CharacterRow], String>,
    unit_leaders: Result<&'a [UnitLeaderRow], String>,
    unit_leader_traits: Result<&'a [UnitLeaderTraitRow], String>,
    role_traits: Result<&'a [CharacterRoleTraitRow], String>,
    operatives: Result<&'a [CharacterOperativeRow], String>,
    retired: Result<&'a [CountryCharacterRow], String>,
}

/// A character's direct unit leader (UnitLeaders), else its resolved
/// referenced leader (CharacterOperatives).
#[derive(Clone, Copy)]
pub enum UnitLeader<'a> {
    Direct(&'a UnitLeaderRow),
    Referenced(&'a CharacterOperativeRow),
}

impl UnitLeader<'_> {
    /// The leader category; 3 is an operative.
    pub fn category(self) -> i32 {
        match self {
            Self::Direct(row) => row.category,
            Self::Referenced(row) => row.category,
        }
    }

    /// The raw country reference.
    pub fn owner(self) -> u32 {
        match self {
            Self::Direct(row) => row.owner,
            Self::Referenced(row) => row.owner,
        }
    }

    /// The skill level, which requires a recorded skill object.
    pub fn skill(self) -> Evaluation<i32> {
        match self {
            Self::Direct(row) if row.skill_status == 0 => Err(format!(
                "unit leader {} has no skill object (UnitLeaders skill_status 0), which the skill read requires",
                row.id
            )),
            Self::Direct(row) => Ok(row.skill_level),
            Self::Referenced(row) => row.skill_level.ok_or_else(|| {
                format!(
                    "character {}'s referenced unit leader has no skill object, which the skill read requires",
                    row.character
                )
            }),
        }
    }
}

/// The name sources for a character's `has_trait`: unit-leader traits,
/// country-leader traits and the lexer for a scientist trait.
pub struct TraitNames<'a> {
    pub units: &'a Traits,
    pub country_leaders: Result<&'a LeaderTraits, String>,
    pub lexer: &'a dyn Links,
}

impl Default for Tables<'_> {
    /// Every table unavailable.
    fn default() -> Self {
        fn none<T>() -> Result<T, String> {
            Err("the table is not recorded here".to_owned())
        }
        Self {
            characters: none(),
            unit_leaders: none(),
            unit_leader_traits: none(),
            role_traits: none(),
            operatives: none(),
            retired: none(),
        }
    }
}

impl<'a> Tables<'a> {
    /// The hour's tables, each hourly (replay_data::cadence): their writers run at
    /// any hour.
    pub fn read(frame: &'a DecodedFrame) -> Self {
        fn table<'f, T: replay_data::rows::TableSchema>(
            frame: &'f DecodedFrame,
            name: &str,
        ) -> Result<&'f [T::Row], String> {
            frame
                .complete::<T>()
                .map_err(|error| format!("{name} is not complete this hour: {error}"))
        }
        Self {
            characters: table::<tables::Characters>(frame, "Characters"),
            unit_leaders: table::<tables::UnitLeaders>(frame, "UnitLeaders"),
            unit_leader_traits: table::<tables::UnitLeaderTraits>(frame, "UnitLeaderTraits"),
            role_traits: table::<tables::CharacterRoleTraits>(frame, "CharacterRoleTraits"),
            operatives: table::<tables::CharacterOperatives>(frame, "CharacterOperatives"),
            retired: table::<tables::CountryRetiredCharacters>(frame, "CountryRetiredCharacters"),
        }
    }

    fn character(&self, character: u32) -> Evaluation<&'a CharacterRow> {
        self.characters
            .as_ref()
            .map_err(Clone::clone)?
            .iter()
            .find(|row| row.id == character)
            .ok_or_else(|| format!("character {character} has no Characters row"))
    }

    /// The direct leader recorded by UnitLeaders' `character_id`, else
    /// the referenced leader recorded by CharacterOperatives when resolved;
    /// `None` for neither.
    pub fn unit_leader(&self, character: u32) -> Evaluation<Option<UnitLeader<'a>>> {
        let leaders = self.unit_leaders.as_ref().map_err(Clone::clone)?;
        let mut found = leaders.iter().filter(|row| row.character_id == character);
        if let Some(leader) = found.next() {
            if found.next().is_some() {
                return Err(format!(
                    "character {character} has more than one UnitLeaders row, and a character holds one direct unit leader"
                ));
            }
            return Ok(Some(UnitLeader::Direct(leader)));
        }
        Ok(self
            .operatives
            .as_ref()
            .map_err(Clone::clone)?
            .iter()
            .find(|row| row.character == character && row.resolved)
            .map(UnitLeader::Referenced))
    }

    /// `tag` in a character scope without carried combat: the country of
    /// the character's unit leader, else Characters.country. Zero is false;
    /// other references compare canonically with the tagged slot.
    pub fn tag(
        &self,
        character: u32,
        tagged: u32,
        canonical: impl Fn(u32) -> Evaluation<u32>,
    ) -> Evaluation<bool> {
        let country = match self.unit_leader(character)? {
            Some(leader) => leader.owner(),
            None => self.character(character)?.country,
        };
        Ok(country != 0 && canonical(country)? == tagged)
    }

    /// `skill` in a character scope without carried combat: the unit
    /// leader's skill level, or zero without a unit leader.
    pub fn skill(&self, character: u32) -> Evaluation<i32> {
        self.unit_leader(character)?
            .map_or(Ok(0), UnitLeader::skill)
    }

    /// Retired when Characters.country is signed-nonpositive, or when
    /// that country's CountryRetiredCharacters has the character's name
    /// token. A null retired record compares as token zero.
    pub fn retired(
        &self,
        character: u32,
        canonical: impl Fn(u32) -> Evaluation<u32>,
    ) -> Evaluation<bool> {
        let own = self.character(character)?;
        if own.country as i32 <= 0 {
            return Ok(true);
        }
        let country = canonical(own.country)?;
        for row in self.retired.as_ref().map_err(Clone::clone)?.iter() {
            if canonical(row.country)? != country {
                continue;
            }
            let token = match row.character {
                None => 0,
                Some(id) => self.character(id)?.name_token_id,
            };
            if token == own.name_token_id {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `has_trait` in a character scope: the character passes the gate
    /// only as an operative (leader category 3) or when not retired.
    pub fn has_trait(
        &self,
        character: u32,
        operand: &[u8],
        names: &TraitNames<'_>,
        canonical: impl Fn(u32) -> Evaluation<u32>,
    ) -> Evaluation<bool> {
        let leader = self.unit_leader(character)?;
        let operative = leader.is_some_and(|leader| leader.category() == 3);
        if !operative && self.retired(character, canonical)? {
            return Ok(false);
        }
        self.character_traits(character, operand, names, || {
            let Some(leader) = leader else {
                return Ok(None);
            };
            Ok(Some(match leader {
                UnitLeader::Direct(leader) => {
                    let assigned = self.unit_leader_traits.as_ref().map_err(Clone::clone)?;
                    let reference = (u64::from(leader.id) << 32) | u64::from(leader.reference_type);
                    assigned
                        .iter()
                        .filter(|row| row.leader_reference == reference)
                        .map(|row| row.definition_ordinal)
                        .collect()
                }
                UnitLeader::Referenced(leader) => leader
                    .trait_definitions
                    .iter()
                    .map(|&ordinal| ordinal as i32)
                    .collect(),
            }))
        })
    }

    /// Trait membership after the character gate. Unit-leader names use
    /// the first case-insensitive definition and compare its recorded
    /// `definition_ordinal` against `assigned` (`None` without a leader).
    /// Otherwise the first country-leader definition compares by ordinal
    /// against advisor and country-leader roles' CharacterRoleTraits;
    /// a later definition with the same name does not count. Other names
    /// compare by token against the scientist role's traits.
    pub fn character_traits(
        &self,
        character: u32,
        operand: &[u8],
        names: &TraitNames<'_>,
        assigned: impl FnOnce() -> Evaluation<Option<Vec<i32>>>,
    ) -> Evaluation<bool> {
        if let Some(ordinal) = names.units.first_ordinal(operand)?
            && let Some(assigned) = assigned()?
        {
            return Ok(assigned.contains(&ordinal));
        }
        let shown = String::from_utf8_lossy(operand);
        let country_leaders = names
            .country_leaders
            .as_ref()
            .map_err(|why| format!("the country-leader traits are unavailable: {why}"))?;
        let rows = self.role_traits.as_ref().map_err(Clone::clone)?;
        let own = || rows.iter().filter(move |row| row.character == character);
        let name = std::str::from_utf8(operand)
            .map_err(|_| format!("trait {shown} is not UTF-8, as a trait name is"))?;
        match country_leaders.first_ordinal(name) {
            Err(gap) => Err(format!(
                "whether {shown} is a country-leader trait, and which definition, is unknown: {gap}"
            )),
            // The first definition of the name, by identity: its database
            // ordinal, which a later definition of the name lacks.
            Ok(Some(ordinal)) => {
                let token = names.lexer.token_of(operand)?;
                holds_country_leader_trait(
                    own().filter(|row| row.kind != CharacterRoleTraitRow::SCIENTIST),
                    country_leaders,
                    name,
                    ordinal,
                    token,
                )
            }
            // The scientist trait database is keyed by the token.
            Ok(None) => Ok(names.lexer.token_of(operand)?.is_some_and(|token| {
                own().any(|row| {
                    row.kind == CharacterRoleTraitRow::SCIENTIST && row.tokens.contains(&token)
                })
            })),
        }
    }
}

/// Compare trait definition identities, recorded as ordinals.
pub(crate) fn holds_country_leader_trait<'a>(
    rows: impl Iterator<Item = &'a CharacterRoleTraitRow>,
    traits: &LeaderTraits,
    name: &str,
    ordinal: u32,
    token: Option<u32>,
) -> Evaluation<bool> {
    let mut held = false;
    for row in rows {
        for (&held_token, &definition) in row.tokens.iter().zip(&row.definitions) {
            if definition == ordinal {
                held = true;
            } else if Some(held_token) == token
                && !traits
                    .by_ordinal(definition)
                    .is_some_and(|found| found.token.eq_ignore_ascii_case(name))
            {
                return Err(format!(
                    "character {} holds trait {name}'s token as definition {definition}, which the content's country-leader traits do not give that name, so which definition the name lookup finds is unknown",
                    row.character
                ));
            }
        }
    }
    Ok(held)
}
