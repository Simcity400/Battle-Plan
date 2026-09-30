//! The recorded operands of the triggers a character scope evaluates:
//! `tag` (`CTagTrigger::Evaluate@1403DCDF0`), `skill`
//! (`CSkillTrigger::GetValue@140475170`) and `has_trait`
//! (`CHasTraitTrigger::Evaluate@1403D7D20`). The tactic evaluator and the
//! shared trigger evaluator ([`crate::triggers`]) both read a character
//! through these tables: its unit leader (`CCharacter::GetUnitLeader@140F8A840`,
//! UnitLeaders or CharacterOperatives), its retired records
//! (CountryRetiredCharacters) and its roles' traits (CharacterRoleTraits).

use crate::state_resources::Links;
use installed_content::{leader_traits::LeaderTraits, unit_leader::Traits};
use replay::{
    frame::DecodedFrame,
    rows::{
        CharacterOperativeRow, CharacterRoleTraitRow, CharacterRow, CountryCharacterRow,
        UnitLeaderRow, UnitLeaderTraitRow, tables,
    },
};

type Evaluation<T> = Result<T, String>;

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

/// GetUnitLeader (140F8A840) of a character: the unit leader at
/// CCharacter+0xA8 (UnitLeaders), else the one its reference at +0xC8
/// resolves to (CharacterOperatives).
#[derive(Clone, Copy)]
pub enum UnitLeader<'a> {
    Direct(&'a UnitLeaderRow),
    Referenced(&'a CharacterOperativeRow),
}

impl UnitLeader<'_> {
    /// CUnitLeader+0xE7C; 3 is an operative.
    pub fn category(self) -> i32 {
        match self {
            Self::Direct(row) => row.category,
            Self::Referenced(row) => row.category,
        }
    }

    /// CUnitLeader+0x120, the raw country reference.
    pub fn owner(self) -> u32 {
        match self {
            Self::Direct(row) => row.owner,
            Self::Referenced(row) => row.owner,
        }
    }

    /// CUnitLeader::GetSkillLevel (140C0A4A0), which dereferences the skill
    /// object (+0xE60).
    pub fn skill(self) -> Evaluation<i32> {
        match self {
            Self::Direct(row) if row.skill_status == 0 => Err(format!(
                "unit leader {} has no skill object (UnitLeaders skill_status 0), which GetSkillLevel (140C0A4A0) dereferences",
                row.id
            )),
            Self::Direct(row) => Ok(row.skill_level),
            Self::Referenced(row) => row.skill_level.ok_or_else(|| {
                format!(
                    "character {}'s referenced unit leader has no skill object, which GetSkillLevel (140C0A4A0) dereferences",
                    row.character
                )
            }),
        }
    }
}

/// What a character's `has_trait` looks names up in: the unit-leader
/// traits (FindTraitByName 140ADB590), the country-leader traits
/// (FindCountryLeaderTraitByName 140712D30) and the lexer (1424A7060) for
/// a scientist trait.
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
    /// The hour's tables, each hourly (replay::cadence): their writers run at
    /// any hour.
    pub fn read(frame: &'a DecodedFrame) -> Self {
        fn table<'f, T: replay::rows::TableSchema>(
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

    /// GetUnitLeader (140F8A840): CCharacter+0xA8, whose leader UnitLeaders
    /// records (`character_id`, +0xF4C), else the reference at +0xC8
    /// (CharacterOperatives) when the registry holds its object; `None` for
    /// neither.
    pub fn unit_leader(&self, character: u32) -> Evaluation<Option<UnitLeader<'a>>> {
        let leaders = self.unit_leaders.as_ref().map_err(Clone::clone)?;
        let mut found = leaders.iter().filter(|row| row.character_id == character);
        if let Some(leader) = found.next() {
            if found.next().is_some() {
                return Err(format!(
                    "character {character} has more than one UnitLeaders row, and CCharacter+0xA8 holds one unit leader"
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

    /// CTagTrigger::Evaluate (1403DCDF0) in a character scope, whose combat
    /// (+0x60) SetCharacter (14052DAF0 -> 14052F3F0) cleared: the country of
    /// the character's unit leader (ability_caster_resolver 1405294F0 ->
    /// GetUnitLeader, CUnitLeader+0x120), else the character's own
    /// (CCharacter::country_reference 140A5BF80, +0x68, Characters.country),
    /// 0 being false, compared with the tagged slot as the same country
    /// (140BA60A0).
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

    /// CSkillTrigger's value (140475170) in a character scope: the scope has
    /// no combat, so the character's unit leader's GetSkillLevel (140C0A4A0:
    /// +0xE60 -> +0x1B8), 0 without a unit leader, compared by
    /// CIntTrigger::Evaluate (14138CC70).
    pub fn skill(&self, character: u32) -> Evaluation<i32> {
        self.unit_leader(character)?
            .map_or(Ok(0), UnitLeader::skill)
    }

    /// HasRetiredCharacter (14052BC80) -> IsRetired (140F8FAD0): retired
    /// when the country (+0x68, `Characters.country`) is at or below 0, or
    /// when a record of that country's retired array (CCountryCharacters+0x28,
    /// CountryRetiredCharacters) has the character's name token (+0x18;
    /// 1410D7A30 -> 1410D1130, whose comparator 141A883F0 takes a null
    /// record as token 0).
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

    /// CHasTraitTrigger::Evaluate (1403D7D20) in a character scope: the
    /// scope's character (character_resolver 140529310), which passes only
    /// as an operative (GetUnitLeader 140F8A840, category +0xE7C of 3) or
    /// when not retired (HasRetiredCharacter 14052BC80 reads the same scope
    /// character).
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

    /// 1403D7D20 past its character gate, for the character's unit leader's
    /// assigned trait definitions (`assigned`: `None` without a unit
    /// leader). FindTraitByName (140ADB590 -> 140AD3E30) takes the first
    /// definition of the operand's name in any case; a unit-leader trait
    /// with a unit leader is held when an assigned trait is that definition
    /// (1402EF030 compares the pointers; the recorded `definition_ordinal`,
    /// +0x8A4, names the definition, and a repeated name is another one).
    /// Otherwise a country-leader trait (FindCountryLeaderTraitByName
    /// 140712D30: the first definition of the name, a real one) is held when
    /// an advisor role's (+0xB0 -> CAdvisor+0xC8) or country-leader role's
    /// (+0x98 -> CCountryLeader+0x50) array holds that definition, compared
    /// by its database ordinal (+0x1A0, CharacterRoleTraits' `definitions`),
    /// so a later definition of the name does not count; any other name when
    /// the scientist's (+0xC0 -> CScientist+0x30) holds the scientist trait
    /// of its token (14022F320).
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
        let country_leaders = names.country_leaders.as_ref().map_err(|why| {
            format!("the country-leader traits (140712D30) are unavailable: {why}")
        })?;
        let rows = self.role_traits.as_ref().map_err(Clone::clone)?;
        let own = || rows.iter().filter(move |row| row.character == character);
        let name = std::str::from_utf8(operand)
            .map_err(|_| format!("trait {shown} is not UTF-8, as a trait name is"))?;
        match country_leaders.first_ordinal(name) {
            Err(gap) => Err(format!(
                "whether {shown} is a country-leader trait (140712D30), and which definition, is unknown: {gap}"
            )),
            // The first definition of the name, by identity: its database
            // ordinal (+0x1A0), which a later definition of the name lacks.
            Ok(Some(ordinal)) => {
                // A role holding the name's token at another ordinal holds a
                // later definition of the name; the content must agree, or
                // its reading counts definitions differently from 1407112F0.
                let token = names.lexer.token_of(operand)?;
                let mut held = false;
                for row in own().filter(|row| row.kind != CharacterRoleTraitRow::SCIENTIST) {
                    for (&held_token, &definition) in row.tokens.iter().zip(&row.definitions) {
                        if definition == ordinal {
                            held = true;
                        } else if Some(held_token) == token
                            && !country_leaders
                                .by_ordinal(definition)
                                .is_some_and(|found| found.token.eq_ignore_ascii_case(name))
                        {
                            return Err(format!(
                                "character {character} holds trait {shown}'s token as definition {definition}, which the content's country-leader traits do not give that name, so which definition 140712D30 finds is unknown"
                            ));
                        }
                    }
                }
                Ok(held)
            }
            // The scientist trait database is keyed by the token (14022F320).
            Ok(None) => Ok(names.lexer.token_of(operand)?.is_some_and(|token| {
                own().any(|row| {
                    row.kind == CharacterRoleTraitRow::SCIENTIST && row.tokens.contains(&token)
                })
            })),
        }
    }
}
