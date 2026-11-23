
use replay_data::rows::{
    CharacterRow, CountryAppearanceRow, CountryCivilWarRow, CountryCounterInputRow,
    CountryLinkRow, CountryRow, CountryWarRow, LexicalTokenRow, TableId, WarRelationRow, mask, tables,
};
use replay_frame::frame::{Rowset, Tables};
use std::{borrow::Cow, collections::HashMap};

fn spelling<'a>(lexical: &Rowset<'a, LexicalTokenRow>, token: u32) -> Option<&'a [u8]> {
    lexical
        .text(u128::from(token), |row| &row.literal, LITERAL)
        .ok()
        .flatten()
}

const LITERAL: usize = replay_data::rows::column::<LexicalTokenRow>("literal");

pub const CHARACTER_IDENTITY: u64 = mask::<CharacterRow>(&[
    "name_token_id",
    "token",
    "template_present",
    "template_tag",
    "template_file",
    "template_line",
    "template_generated",
]);

pub fn character_script_token<'a>(
    character: &CharacterRow,
    lexical: impl Into<replay_frame::frame::Rows<'a, LexicalTokenRow>>,
) -> Option<&'a str> {
    character_script_token_in(character, Rowset::Decoded(lexical.into().as_slice()))
}

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

pub fn built_template<'a>(
    character: &CharacterRow,
    lexical: Option<impl Into<replay_frame::frame::Rows<'a, LexicalTokenRow>>>,
) -> Result<content_reader_key_names::characters::BuiltTemplate, String> {
    built_template_in(
        character,
        lexical.map(|lexical| Rowset::Decoded(lexical.into().as_slice())),
    )
}

pub fn built_template_in(
    character: &CharacterRow,
    lexical: Option<Rowset<'_, LexicalTokenRow>>,
) -> Result<content_reader_key_names::characters::BuiltTemplate, String> {
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
    Ok(content_reader_key_names::characters::BuiltTemplate {
        tag: tag.to_owned(),
        file: character.template_file.clone(),
        line: character.template_line,
        generated: character.template_generated != 0,
    })
}

pub fn civil_war<F: Tables + ?Sized>(frame: &F, slot: u32) -> Result<bool, String> {
    CivilWars::new(frame)?.holds(slot)
}

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

const SLOT: u64 = mask::<CountryRow>(&["canonical_slot"]);
const LINK: u64 = mask::<CountryLinkRow>(&["canonical_slot"]);
const TARGET: u64 = mask::<CountryCivilWarRow>(&["target"]);
const OWNED: u64 = mask::<CountryCounterInputRow>(&["owned_state_count"]);
const READ: u64 = mask::<CountryWarRow>(&["enemies_read"]);
const ENEMIES: u64 = mask::<CountryWarRow>(&["enemies_read", "enemies"]);
const SPAN: u64 = mask::<WarRelationRow>(&["start_hour", "end_hour"]);

pub struct CivilWars<'a> {
    links: Rowset<'a, CountryLinkRow>,
    raw: HashMap<u32, u32>,
    owned: HashMap<u32, i32>,
    lists: HashMap<u32, Vec<u32>>,
    wars: Result<Enemies<'a>, String>,
    relations: Result<Relations, String>,
}

struct Enemies<'a> {
    rows: Rowset<'a, CountryWarRow>,
    by_slot: HashMap<u32, (usize, u128)>,
    unlinked: Option<(usize, u32)>,
}

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

    fn enemies(&self, slot: u32) -> Result<Vec<u32>, String> {
        let wars = self.wars.as_ref().map_err(Clone::clone)?;
        let found = wars.by_slot.get(&slot).copied();
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

fn linked(links: &Rowset<'_, CountryLinkRow>, raw: u32, table: TableId) -> Result<u32, String> {
    crate::metadata::link_slot_in(links, raw)
        .ok_or_else(|| format!("{} names a country that no recorded country link holds", table.words()))
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NamingCountry {
    pub slot: u32,
    pub look: Option<Look>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct Look {
    pub name_tag: String,
    pub original_or_own_tag: Result<String, String>,
    pub cosmetic_tag: String,
    pub civil_war: Result<bool, String>,
}

pub const LOOK: u64 = mask::<CountryAppearanceRow>(&[
    "name_tag",
    "cosmetic_tag",
    "original_tag",
    "original_tag_reference",
]);

impl Look {
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
                content_reader_country_names::country_names::reserved_zero_original_tag_in(frame)
                    .map(Cow::into_owned)
            } else {
                content_reader_country_names::country_names::original_tag(row, appearance)
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
