//! The character and country identity operands names are chosen by: script
//! tokens and templates of recorded characters, and the tag, look and civil-war
//! state a country's ideas and advisors are named for.

use replay_data::rows::{
    CharacterRow, CountryAppearanceRow, CountryCivilWarRow, CountryCounterInputRow,
    CountryLinkRow, CountryRow, CountryWarRow, LexicalTokenRow, TableId, WarRelationRow, mask, tables,
};
use replay_frame::frame::{Rowset, Tables};
use std::{borrow::Cow, collections::HashMap};

/// A LexicalTokens row's spelling, borrowed where the frame holds it.
fn spelling<'a>(lexical: &Rowset<'a, LexicalTokenRow>, token: u32) -> Option<&'a [u8]> {
    lexical
        .text(u128::from(token), |row| &row.literal, LITERAL)
        .ok()
        .flatten()
}

const LITERAL: usize = replay_data::rows::column::<LexicalTokenRow>("literal");

/// The fields of a Characters row [`character_script_token_in`] and [`built_template_in`] read.
pub const CHARACTER_IDENTITY: u64 = mask::<CharacterRow>(&[
    "name_token_id",
    "token",
    "template_present",
    "template_tag",
    "template_file",
    "template_line",
    "template_generated",
]);

/// A recorded character's script token: its name_token_id resolved through
/// LexicalTokens. The row's token field is the saved name, not the script
/// token. None when the id is negative or not in the dictionary.
pub fn character_script_token<'a>(
    character: &CharacterRow,
    lexical: impl Into<replay_frame::frame::Rows<'a, LexicalTokenRow>>,
) -> Option<&'a str> {
    character_script_token_in(character, Rowset::Decoded(lexical.into().as_slice()))
}

/// [`character_script_token`] of any frame's LexicalTokens, a walk's views among them.
pub fn character_script_token_in<'a>(
    character: &CharacterRow,
    lexical: Rowset<'a, LexicalTokenRow>,
) -> Option<&'a str> {
    let token = u32::try_from(character.name_token_id).ok()?;
    let missing = || {
        crate::triggers::refusal::missing_reference(replay_data::rows::TableId::LexicalTokens, None)
    };
    let literal = crate::triggers::refusal::reference(spelling(&lexical, token), token != 0, missing)?;
    crate::triggers::refusal::reference(std::str::from_utf8(literal).ok(), token != 0, missing)
}

/// The template a recorded character holds, from its Characters row's
/// template columns. Its tag token is spelled by LexicalTokens;
/// `Err` names what the hour does not hold.
pub fn built_template<'a>(
    character: &CharacterRow,
    lexical: Option<impl Into<replay_frame::frame::Rows<'a, LexicalTokenRow>>>,
) -> Result<installed_content_core::characters::BuiltTemplate, String> {
    built_template_in(
        character,
        lexical.map(|lexical| Rowset::Decoded(lexical.into().as_slice())),
    )
}

/// [`built_template`] of any frame's LexicalTokens, a walk's views among them.
pub fn built_template_in(
    character: &CharacterRow,
    lexical: Option<Rowset<'_, LexicalTokenRow>>,
) -> Result<installed_content_core::characters::BuiltTemplate, String> {
    let id = character.id;
    if character.template_present == 0 {
        return Err(format!(
            "character {id} holds no template (recorded template is null)"
        ));
    }
    let token = character.template_tag;
    let lexical = crate::triggers::refusal::reference(lexical, token != 0,
        || crate::triggers::refusal::missing_reference(replay_data::rows::TableId::LexicalTokens, None))
        .ok_or_else(|| format!("character {id}'s template tag token {token} is not spelled: {} is incomplete", replay_data::rows::TableId::LexicalTokens.words()))?;
    let literal = crate::triggers::refusal::reference(
        spelling(&lexical, token),
        token != 0,
        || {
            crate::triggers::refusal::missing_reference(
                replay_data::rows::TableId::LexicalTokens,
                None,
            )
        },
    )
    .ok_or_else(|| {
        format!("character {id}'s template tag token {token} has no lexical tokens row")
    })?;
    let tag = std::str::from_utf8(literal)
        .map_err(|_| format!("character {id}'s template tag token {token} is not UTF-8"))?;
    Ok(installed_content_core::characters::BuiltTemplate {
        tag: tag.to_owned(),
        file: character.template_file.clone(),
        line: character.template_line,
        generated: character.template_generated != 0,
    })
}

/// Whether the country has a civil war at the frame's hour, as used by idea
/// naming and has_civil_war: a country in its own CountryCivilWars list has a
/// positive CountryCounterInputs.owned_state_count, or an existing enemy
/// holds it in its own civil-war list ([`CivilWars::holds`]). `Err` names
/// the input the hour lacks.
pub fn civil_war<F: Tables + ?Sized>(frame: &F, slot: u32) -> Result<bool, String> {
    CivilWars::new(frame)?.holds(slot)
}

/// What [`civil_war`] reads of an hour, for every country: an hour changing
/// none of it since the hour before leaves every country's answer as it was.
pub fn civil_war_reads() -> Vec<crate::triggers::inputs::TableRead> {
    use crate::triggers::inputs::TableRead;
    vec![
        TableRead::fields(TableId::Countries, &["canonical_slot"]),
        TableRead::fields(TableId::CountryLinks, &["canonical_slot"]),
        TableRead::fields(TableId::CountryCivilWars, &["target"]),
        TableRead::fields(TableId::CountryCounterInputs, &["owned_state_count"]),
        TableRead::fields(TableId::CountryWars, &["enemies_read", "enemies"]),
        TableRead::fields(TableId::WarRelations, &["start_hour", "end_hour"]).ending(),
    ]
}

/// The fields [`civil_war_reads`] names, by table.
const SLOT: u64 = mask::<CountryRow>(&["canonical_slot"]);
const LINK: u64 = mask::<CountryLinkRow>(&["canonical_slot"]);
const TARGET: u64 = mask::<CountryCivilWarRow>(&["target"]);
const OWNED: u64 = mask::<CountryCounterInputRow>(&["owned_state_count"]);
const READ: u64 = mask::<CountryWarRow>(&["enemies_read"]);
const ENEMIES: u64 = mask::<CountryWarRow>(&["enemies_read", "enemies"]);
const SPAN: u64 = mask::<WarRelationRow>(&["start_hour", "end_hour"]);

/// The civil-war predicate's inputs at one hour, indexed once, so asking it
/// of every country costs each country only its own rows. Answers, and the
/// reason an answer is refused, are those of scanning the tables in stored
/// order: a reference that does not resolve is reported only where that
/// scan reaches it.
pub struct CivilWars<'a> {
    links: Rowset<'a, CountryLinkRow>,
    /// Each canonical slot's first Countries row id.
    raw: HashMap<u32, u32>,
    /// Each raw country's first owned state count.
    owned: HashMap<u32, i32>,
    /// Each raw country's civil-war targets, in stored order.
    lists: HashMap<u32, Vec<u32>>,
    wars: Result<Enemies<'a>, String>,
    relations: Result<Relations, String>,
}

/// The CountryWars rows that read their enemy vector: each slot's first by
/// stored position with its key, and the first whose country does not resolve.
struct Enemies<'a> {
    rows: Rowset<'a, CountryWarRow>,
    by_slot: HashMap<u32, (usize, u128)>,
    unlinked: Option<(usize, u32)>,
}

/// The other country of each ongoing war relation, by slot in stored order,
/// and the first raw country of one that does not resolve.
struct Relations {
    by_slot: HashMap<u32, Vec<u32>>,
    unlinked: Option<u32>,
}

impl<'a> CivilWars<'a> {
    pub fn new<F: Tables + ?Sized>(frame: &'a F) -> Result<Self, String> {
        let hour = frame.at_hour();
        let text = |error: std::io::Error| error.to_string();
        let countries = frame.complete_rows::<tables::Countries>(SLOT).map_err(text)?;
        let links = frame.complete_rows::<tables::CountryLinks>(LINK).map_err(text)?;
        let lists = frame.complete_rows::<tables::CountryCivilWars>(TARGET).map_err(text)?;
        let counters = frame
            .complete_rows::<tables::CountryCounterInputs>(OWNED)
            .map_err(text)?;
        // Resolved without refusing: a refusal is reported only where a scan reaches it.
        let slot_of = |raw: u32| -> Result<Option<u32>, String> {
            Ok(links
                .get(u128::from(raw))
                .map_err(text)?
                .map(|link| link.canonical_slot))
        };
        let mut raw = HashMap::new();
        for country in countries.iter() {
            let country = country.map_err(text)?;
            raw.entry(country.canonical_slot).or_insert(country.id);
        }
        let mut owned = HashMap::new();
        for row in counters.iter() {
            let row = row.map_err(text)?;
            owned.entry(row.country).or_insert(row.owned_state_count);
        }
        let mut targets = HashMap::<u32, Vec<u32>>::new();
        for row in lists.iter() {
            let row = row.map_err(text)?;
            targets.entry(row.country).or_default().push(row.target);
        }
        let wars = frame
            .complete_rows::<tables::CountryWars>(ENEMIES)
            .map_err(text)
            .and_then(|rows| {
                let mut wars = Enemies {
                    rows,
                    by_slot: HashMap::new(),
                    unlinked: None,
                };
                for (index, row) in rows.with_mask(READ).iter().enumerate() {
                    let row = row.map_err(text)?;
                    if row.enemies_read == 0 {
                        continue;
                    }
                    match slot_of(row.country)? {
                        Some(slot) => {
                            wars.by_slot.entry(slot).or_insert((index, row.key()));
                        }
                        None => {
                            wars.unlinked.get_or_insert((index, row.country));
                        }
                    }
                }
                Ok(wars)
            });
        let relations = frame
            .complete_rows::<tables::WarRelations>(SPAN)
            .map_err(text)
            .and_then(|rows| {
                let mut relations = Relations {
                    by_slot: HashMap::new(),
                    unlinked: None,
                };
                for row in rows.iter() {
                    let row = row.map_err(text)?;
                    if !crate::triggers::ongoing(row.start_hour, row.end_hour, hour) {
                        continue;
                    }
                    match (slot_of(row.first_country)?, slot_of(row.second_country)?) {
                        (Some(first), Some(second)) => {
                            relations.by_slot.entry(first).or_default().push(second);
                            if second != first {
                                relations.by_slot.entry(second).or_default().push(first);
                            }
                        }
                        (None, _) => {
                            relations.unlinked.get_or_insert(row.first_country);
                        }
                        (Some(_), None) => {
                            relations.unlinked.get_or_insert(row.second_country);
                        }
                    }
                }
                Ok(relations)
            });
        Ok(Self {
            links,
            raw,
            owned,
            lists: targets,
            wars,
            relations,
        })
    }

    /// Whether `slot` has a civil war: a country its own list names exists,
    /// or an existing enemy lists it. Each list is searched only until one
    /// holds.
    pub fn holds(&self, slot: u32) -> Result<bool, String> {
        for &target in self.targets(self.raw_of(slot)?) {
            if self.exists(self.linked(target, TableId::CountryCivilWars)?)? {
                return Ok(true);
            }
        }
        for enemy in self.enemies(slot)? {
            if !self.exists(enemy)? {
                continue;
            }
            for &target in self.targets(self.raw_of(enemy)?) {
                if self.linked(target, TableId::CountryCivilWars)? == slot {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn raw_of(&self, slot: u32) -> Result<u32, String> {
        self.raw.get(&slot).copied().ok_or_else(|| "a country is not among the recorded countries at this hour".to_owned())
    }

    /// Whether `slot` owns a state, the game's existence check.
    pub fn exists(&self, slot: u32) -> Result<bool, String> {
        let raw = self.raw_of(slot)?;
        self.owned
            .get(&raw)
            .map(|&count| count > 0)
            .ok_or_else(|| "a country's owned state count is not recorded at this hour".to_owned())
    }

    fn targets(&self, raw: u32) -> &[u32] {
        self.lists.get(&raw).map(Vec::as_slice).unwrap_or_default()
    }

    fn linked(&self, raw: u32, table: TableId) -> Result<u32, String> {
        linked(&self.links, raw, table)
    }

    /// The country's enemy vector: its CountryWars row's where read, else the
    /// other country of each ongoing war relation. War-relation updates keep
    /// that vector synchronized.
    fn enemies(&self, slot: u32) -> Result<Vec<u32>, String> {
        let wars = self.wars.as_ref().map_err(Clone::clone)?;
        let found = wars.by_slot.get(&slot).copied();
        // A row before it whose country does not resolve stops the scan there.
        if let Some((_, raw)) = wars
            .unlinked
            .filter(|&(at, _)| found.is_none_or(|(found, _)| at < found))
        {
            self.linked(raw, TableId::CountryWars)?;
        }
        if let Some((_, key)) = found {
            let row = wars
                .rows
                .get(key)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("a country's {} are not held at this hour", TableId::CountryWars.words()))?;
            return row
                .enemies
                .iter()
                .map(|&raw| self.linked(raw, TableId::CountryWars))
                .collect();
        }
        let relations = self.relations.as_ref().map_err(Clone::clone)?;
        if let Some(raw) = relations.unlinked {
            self.linked(raw, TableId::WarRelations)?;
        }
        Ok(relations.by_slot.get(&slot).cloned().unwrap_or_default())
    }
}

/// A raw country reference's canonical slot, or why the hour cannot say.
fn linked(links: &Rowset<'_, CountryLinkRow>, raw: u32, table: TableId) -> Result<u32, String> {
    crate::metadata::link_slot_in(links, raw)
        .ok_or_else(|| format!("{} names a country that no recorded country link holds", table.words()))
}

/// A country as idea naming needs it at one hour, owned for the scans that
/// keep it: its canonical slot and its appearance ([`Look`]), `None` when
/// the hour lacks its CountryAppearance row.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[derive(replay_spool::Spooled)]
pub struct NamingCountry {
    pub slot: u32,
    pub look: Option<Look>,
}

/// A country's naming operands: dynamic-gated appearance, the independent
/// original-or-own getter, and whether it has a civil war.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[derive(replay_spool::Spooled)]
pub struct Look {
    pub name_tag: String,
    pub original_or_own_tag: Result<String, String>,
    pub cosmetic_tag: String,
    /// [`civil_war`] for a dynamic country (name_tag set); Ok(false) for any
    /// other country, whose idea naming never asks it.
    pub civil_war: Result<bool, String>,
}

/// The fields of a country's appearance its look reads ([`Look::read`]).
pub const LOOK: u64 = mask::<CountryAppearanceRow>(&[
    "name_tag",
    "cosmetic_tag",
    "original_tag",
    "original_tag_reference",
]);

impl Look {
    /// The country's appearance at the frame's hour, `None` without its
    /// CountryAppearance row.
    pub fn read<F: Tables + ?Sized>(frame: &F, row: &CountryRow) -> Option<Self> {
        let appearances =
            crate::triggers::refusal::complete_table::<F, tables::CountryAppearance>(frame, LOOK)
                .ok()?;
        let appearance = crate::triggers::refusal::reference(
            appearances.get(u128::from(row.id)).ok().flatten(),
            row.id != 0,
            || {
                crate::triggers::refusal::missing_reference(
                    replay_data::rows::TableId::CountryAppearance,
                    Some(crate::triggers::Store::Country(row.canonical_slot)),
                )
            },
        )?;
        Self::read_with_appearance(frame, row, &appearance)
    }

    fn read_with_appearance<F: Tables + ?Sized>(
        frame: &F,
        row: &CountryRow,
        appearance: &CountryAppearanceRow,
    ) -> Option<Self> {
        Self::read_with(frame, row, appearance, &|slot| civil_war(frame, slot))
    }

    /// [`Self::read_with_appearance`], a dynamic country's civil war answered by `civil_war`.
    fn read_with<F: Tables + ?Sized>(
        frame: &F,
        row: &CountryRow,
        appearance: &CountryAppearanceRow,
        civil_war: &dyn Fn(u32) -> Result<bool, String>,
    ) -> Option<Self> {
        let name_tag = std::str::from_utf8(&appearance.name_tag).ok()?;
        let cosmetic_tag = std::str::from_utf8(&appearance.cosmetic_tag).ok()?;
        Some(Self {
            name_tag: name_tag.to_owned(),
            original_or_own_tag: if row.canonical_slot == 0 {
                installed_content_country_names::country_names::reserved_zero_original_tag_in(frame)
                    .map(Cow::into_owned)
            } else {
                installed_content_country_names::country_names::original_tag(row, appearance)
                    .map(str::to_owned)
            },
            cosmetic_tag: cosmetic_tag.to_owned(),
            civil_war: if name_tag.is_empty() {
                Ok(false)
            } else {
                civil_war(row.canonical_slot)
            },
        })
    }

    pub fn country_look(&self) -> crate::idea_names::CountryLook<'_> {
        crate::idea_names::CountryLook {
            name_tag: &self.name_tag,
            original_or_own_tag: self.original_or_own_tag.as_deref().map_err(String::as_str),
            cosmetic_tag: &self.cosmetic_tag,
            civil_war: self.civil_war.as_ref().copied().map_err(String::as_str),
        }
    }
}

impl NamingCountry {
    #[cfg(any(test, feature = "test-support"))]
    pub fn read<F: Tables + ?Sized>(frame: &F, row: &CountryRow) -> Self {
        Self {
            slot: row.canonical_slot,
            look: Look::read(frame, row),
        }
    }

    /// The same naming getters with the caller's indexed appearance row.
    pub fn read_with_appearance<F: Tables + ?Sized>(
        frame: &F,
        row: &CountryRow,
        appearance: Option<&CountryAppearanceRow>,
    ) -> Self {
        Self {
            slot: row.canonical_slot,
            look: appearance
                .filter(|appearance| appearance.country == row.id)
                .and_then(|appearance| Look::read_with_appearance(frame, row, appearance)),
        }
    }

    /// [`Self::read_with_appearance`], a dynamic country's civil war answered by `civil_war`:
    /// [`CivilWars::holds`] of the frame's hour, indexed once for every country asked.
    pub fn read_with_civil_war<F: Tables + ?Sized>(
        frame: &F,
        row: &CountryRow,
        appearance: Option<&CountryAppearanceRow>,
        civil_war: &dyn Fn(u32) -> Result<bool, String>,
    ) -> Self {
        Self {
            slot: row.canonical_slot,
            look: appearance
                .filter(|appearance| appearance.country == row.id)
                .and_then(|appearance| Look::read_with(frame, row, appearance, civil_war)),
        }
    }

    pub fn for_tag<'a>(&'a self, tag: &'a str) -> crate::idea_names::IdeaCountry<'a> {
        crate::idea_names::IdeaCountry {
            tag,
            slot: self.slot,
            look: self.look.as_ref().map(Look::country_look),
        }
    }
}
