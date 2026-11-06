//! The recorded operands of the `tag`, `skill` and `has_trait` triggers in
//! a character scope. The tactic evaluator and the shared trigger evaluator
//! ([`crate::triggers`]) both read a character through these tables: its unit
//! leader (UnitLeaders or CharacterOperatives), its retired records
//! (CountryRetiredCharacters) and its roles' traits (CharacterRoleTraits).

use crate::{state_resources::Links, triggers::Failure};
use installed_content_core::{leader_traits::LeaderTraits, unit_leader::Traits};
use replay_data::{
    rows::CharacterOperativeRow, rows::CharacterRoleTraitRow, rows::CharacterRow,
    rows::CountryCharacterRow, rows::TableSchema, rows::UnitLeaderRow, rows::UnitLeaderTraitRow,
    rows::mask, rows::tables,
};
use replay_frame::frame::{HourFrame, Rowset};
use std::{borrow::Cow, collections::BTreeMap, sync::OnceLock};

type Evaluation<T> = Result<T, Failure>;

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

/// The fields each table's rows are built with: [`FIELDS`], and for readers of the leader it
/// hands on, a direct leader's [`replay_frame::frame::Leader`] fields and a referenced one's
/// reference, name and codename.
const CHARACTER: u64 = mask::<CharacterRow>(&["country", "name_token_id"]);
const LEADER_OF: u64 = mask::<UnitLeaderRow>(&["character_id"]);
const LEADER: u64 = mask::<UnitLeaderRow>(replay_frame::frame::Leader::FIELDS);
const ASSIGNED: u64 = mask::<UnitLeaderTraitRow>(&["definition_ordinal"]);
const ROLE_TRAITS: u64 = mask::<CharacterRoleTraitRow>(&["tokens", "definitions"]);
const OPERATIVE: u64 = mask::<CharacterOperativeRow>(&[
    "resolved",
    "category",
    "owner",
    "skill_level",
    "trait_definitions",
    "reference",
    "name",
    "codename",
]);
const RETIRED: u64 = mask::<CountryCharacterRow>(&["character"]);

/// The recorded character tables a character scope's `has_trait`, `skill`
/// and `tag` read.
pub struct Tables<'a> {
    characters: Result<Rowset<'a, CharacterRow>, Failure>,
    unit_leaders: Result<Rowset<'a, UnitLeaderRow>, Failure>,
    /// Each character's first UnitLeaders key, and whether another row names it, built on
    /// first use.
    leaders_of: OnceLock<Result<BTreeMap<u32, (u128, bool)>, Failure>>,
    unit_leader_traits: Result<Rowset<'a, UnitLeaderTraitRow>, Failure>,
    role_traits: Result<Rowset<'a, CharacterRoleTraitRow>, Failure>,
    operatives: Result<Rowset<'a, CharacterOperativeRow>, Failure>,
    retired: Result<Rowset<'a, CountryCharacterRow>, Failure>,
}

/// A character's direct unit leader (UnitLeaders), else its resolved
/// referenced leader (CharacterOperatives).
#[derive(Clone)]
pub enum UnitLeader<'a> {
    /// The UnitLeaders row, holding the fields a [`replay_frame::frame::Leader`] holds.
    Direct(Cow<'a, UnitLeaderRow>),
    Referenced(Cow<'a, CharacterOperativeRow>),
}

impl UnitLeader<'_> {
    /// The leader category; 3 is an operative.
    pub fn category(&self) -> i32 {
        match self {
            Self::Direct(row) => row.category,
            Self::Referenced(row) => row.category,
        }
    }

    /// The raw country reference.
    pub fn owner(&self) -> u32 {
        match self {
            Self::Direct(row) => row.owner,
            Self::Referenced(row) => row.owner,
        }
    }

    /// The skill level, which requires a recorded skill object.
    pub fn skill(&self) -> Evaluation<i32> {
        match self {
            Self::Direct(row) if row.skill_status == 0 => Err(format!(
                "unit leader {} has no skill object (UnitLeaders skill_status 0), which the skill read requires",
                row.id
            ).into()),
            Self::Direct(row) => Ok(row.skill_level),
            Self::Referenced(row) => row.skill_level.ok_or_else(|| Failure::input(replay_data::rows::TableId::CharacterOperatives, "skill_level", Some(crate::triggers::Store::Character(row.character)), format!(
                    "character {}'s referenced unit leader has no skill object, which the skill read requires",
                    row.character
                ))),
        }
    }
}

/// `rows`' failure to build a row of `T`, refused where it is reached.
fn walked<T: TableSchema>(why: std::io::Error) -> Failure {
    crate::triggers::refusal::input_error(T::ID, why)
}

/// `table`, or why it is not read, reached.
fn held<T: Clone>(table: &Result<T, Failure>) -> Evaluation<T> {
    table.clone().map_err(|failure| {
        failure.reached();
        failure
    })
}
/// The name sources for a character's `has_trait`: unit-leader traits,
/// country-leader traits and the lexer for a scientist trait.
pub struct TraitNames<'a> {
    pub units: &'a Traits,
    pub country_leaders: Result<&'a LeaderTraits, Failure>,
    pub lexer: &'a dyn Links,
}

impl Default for Tables<'_> {
    /// Every table unavailable.
    fn default() -> Self {
        fn none<T>(table: replay_data::rows::TableId) -> Result<T, Failure> {
            Err(Failure::input(
                table,
                "",
                None,
                "the table is not recorded here",
            ))
        }
        Self {
            characters: none(replay_data::rows::TableId::Characters),
            unit_leaders: none(replay_data::rows::TableId::UnitLeaders),
            leaders_of: OnceLock::new(),
            unit_leader_traits: none(replay_data::rows::TableId::UnitLeaderTraits),
            role_traits: none(replay_data::rows::TableId::CharacterRoleTraits),
            operatives: none(replay_data::rows::TableId::CharacterOperatives),
            retired: none(replay_data::rows::TableId::CountryRetiredCharacters),
        }
    }
}

impl<'a> Tables<'a> {
    /// The hour's tables, each hourly (replay_data::cadence): their writers run at
    /// any hour.
    pub fn read(frame: &'a dyn replay_frame::frame::DecodedSource) -> Self {
        Self::read_in(HourFrame::Decoded(frame))
    }

    /// [`Self::read`] of any frame's tables, a walk's views among them.
    pub fn read_in(frame: HourFrame<'a>) -> Self {
        fn table<'f, T: TableSchema>(
            frame: HourFrame<'f>,
            mask: u64,
            name: &str,
        ) -> Result<Rowset<'f, T::Row>, Failure> {
            frame.complete::<T>(mask).map_err(|error| {
                Failure::input(
                    T::ID,
                    "",
                    None,
                    format!("{name} is not complete this hour: {error}"),
                )
            })
        }
        Self {
            characters: table::<tables::Characters>(frame, CHARACTER, "Characters"),
            unit_leaders: table::<tables::UnitLeaders>(frame, LEADER, "UnitLeaders"),
            leaders_of: OnceLock::new(),
            unit_leader_traits: table::<tables::UnitLeaderTraits>(
                frame,
                ASSIGNED,
                "UnitLeaderTraits",
            ),
            role_traits: table::<tables::CharacterRoleTraits>(
                frame,
                ROLE_TRAITS,
                "CharacterRoleTraits",
            ),
            operatives: table::<tables::CharacterOperatives>(
                frame,
                OPERATIVE,
                "CharacterOperatives",
            ),
            retired: table::<tables::CountryRetiredCharacters>(
                frame,
                RETIRED,
                "CountryRetiredCharacters",
            ),
        }
    }

    fn character(&self, character: u32) -> Evaluation<Cow<'a, CharacterRow>> {
        // Keyed by the character's id.
        held(&self.characters)?
            .get(u128::from(character))
            .map_err(walked::<tables::Characters>)?
            .ok_or_else(|| {
                Failure::input(
                    replay_data::rows::TableId::Characters,
                    "",
                    Some(crate::triggers::Store::Character(character)),
                    format!("character {character} has no Characters row"),
                )
            })
    }

    /// The direct leader recorded by UnitLeaders' `character_id`, else
    /// the referenced leader recorded by CharacterOperatives when resolved;
    /// `None` for neither.
    pub fn unit_leader(&self, character: u32) -> Evaluation<Option<UnitLeader<'a>>> {
        let leaders = held(&self.unit_leaders)?;
        let index = self
            .leaders_of
            .get_or_init(|| {
                let mut index = BTreeMap::new();
                for (key, row) in leaders.keys().zip(leaders.with_mask(LEADER_OF).iter()) {
                    let row = row.map_err(walked::<tables::UnitLeaders>)?;
                    index
                        .entry(row.character_id)
                        .and_modify(|(_, more): &mut (u128, bool)| *more = true)
                        .or_insert((key, false));
                }
                Ok(index)
            })
            .as_ref()
            .map_err(|failure| {
                failure.reached();
                failure.clone()
            })?;
        if let Some(&(key, more)) = index.get(&character) {
            if more {
                return Err(format!(
                    "character {character} has more than one UnitLeaders row, and a character holds one direct unit leader"
                ).into());
            }
            let leader = leaders
                .get(key)
                .map_err(walked::<tables::UnitLeaders>)?
                .ok_or_else(|| walked::<tables::UnitLeaders>(std::io::Error::other("an indexed key holds no row")))?;
            return Ok(Some(UnitLeader::Direct(leader)));
        }
        // CharacterOperatives is keyed by the character.
        Ok(held(&self.operatives)?
            .get(u128::from(character))
            .map_err(walked::<tables::CharacterOperatives>)?
            .filter(|row| row.resolved)
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
            .map_or(Ok(0), |leader| leader.skill())
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
        for row in held(&self.retired)?.iter() {
            let row = row.map_err(walked::<tables::CountryRetiredCharacters>)?;
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
        let operative = leader.as_ref().is_some_and(|leader| leader.category() == 3);
        if !operative && self.retired(character, canonical)? {
            return Ok(false);
        }
        self.character_traits(character, operand, names, || {
            let Some(leader) = leader else {
                return Ok(None);
            };
            Ok(Some(match leader {
                UnitLeader::Direct(leader) => {
                    let assigned = held(&self.unit_leader_traits)?;
                    // Keyed by (leader reference, ordinal): a leader's rows are one key range.
                    let reference = (u64::from(leader.id) << 32) | u64::from(leader.reference_type);
                    let low = u128::from(reference) << 32;
                    assigned
                        .range(low..=low | 0xffff_ffff)
                        .map(|row| {
                            row.map(|row| row.definition_ordinal)
                                .map_err(walked::<tables::UnitLeaderTraits>)
                        })
                        .collect::<Evaluation<Vec<i32>>>()?
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
        let country_leaders = names.country_leaders.as_ref().map_err(|why| {
            why.clone()
                .context(|why| format!("the country-leader traits are unavailable: {why}"))
        })?;
        // Keyed by (character, kind, role): a character's rows are one key range.
        let low = u128::from(character) << 40;
        let own = held(&self.role_traits)?
            .range(low..=low | 0xff_ffff_ffff)
            .map(|row| row.map_err(walked::<tables::CharacterRoleTraits>))
            .collect::<Evaluation<Vec<_>>>()?;
        let own = || own.iter().map(|row| &**row);
        let name = std::str::from_utf8(operand)
            .map_err(|_| format!("trait {shown} is not UTF-8, as a trait name is"))?;
        match country_leaders.first_ordinal(name) {
            Err(gap) => Err(format!(
                "whether {shown} is a country-leader trait, and which definition, is unknown: {gap}"
            )
            .into()),
            // The first definition of the name, by identity: its database
            // ordinal, which a later definition of the name lacks.
            Ok(Some(ordinal)) => {
                let token = names.lexer.token_input(operand)?;
                holds_country_leader_trait(
                    own().filter(|row| row.kind != CharacterRoleTraitRow::SCIENTIST),
                    country_leaders,
                    name,
                    ordinal,
                    token,
                )
            }
            // The scientist trait database is keyed by the token.
            Ok(None) => Ok(names.lexer.token_input(operand)?.is_some_and(|token| {
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
                ).into());
            }
        }
    }
    Ok(held)
}
