//! The character and country identity operands names are chosen by: script
//! tokens and templates of recorded characters, and the tag, look and civil-war
//! state a country's ideas and advisors are named for.

use std::collections::HashMap;

/// A recorded character's script token: its name_token_id resolved through
/// LexicalTokens. The row's token field is the saved name, not the script
/// token. None when the id is negative or not in the dictionary.
pub fn character_script_token<'a>(
    character: &replay_data::rows::CharacterRow,
    lexical: impl Into<replay_frame::frame::Rows<'a, replay_data::rows::LexicalTokenRow>>,
) -> Option<&'a str> {
    let lexical = lexical.into();
    let token = u32::try_from(character.name_token_id).ok()?;
    let index = crate::triggers::refusal::reference(
        lexical.binary_search_by_key(&token, |row| row.token).ok(),
        token != 0,
        || {
            crate::triggers::refusal::missing_reference(
                replay_data::rows::TableId::LexicalTokens,
                None,
            )
        },
    )?;
    crate::triggers::refusal::reference(
        std::str::from_utf8(&lexical.get(index).unwrap().literal).ok(),
        token != 0,
        || {
            crate::triggers::refusal::missing_reference(
                replay_data::rows::TableId::LexicalTokens,
                None,
            )
        },
    )
}

/// The template a recorded character holds, from its Characters row's
/// template columns. Its tag token is spelled by LexicalTokens;
/// `Err` names what the hour does not hold.
pub fn built_template<'a>(
    character: &replay_data::rows::CharacterRow,
    lexical: Option<impl Into<replay_frame::frame::Rows<'a, replay_data::rows::LexicalTokenRow>>>,
) -> Result<installed_content_core::characters::BuiltTemplate, String> {
    let lexical = lexical.map(Into::into);
    let id = character.id;
    if character.template_present == 0 {
        return Err(format!(
            "character {id} holds no template (recorded template is null)"
        ));
    }
    let token = character.template_tag;
    let lexical = crate::triggers::refusal::reference(lexical, token != 0,
        || crate::triggers::refusal::missing_reference(replay_data::rows::TableId::LexicalTokens, None))
        .ok_or_else(|| format!("character {id}'s template tag token {token} is not spelled: LexicalTokens is incomplete"))?;
    let index = crate::triggers::refusal::reference(
        lexical.binary_search_by_key(&token, |row| row.token).ok(),
        token != 0,
        || {
            crate::triggers::refusal::missing_reference(
                replay_data::rows::TableId::LexicalTokens,
                None,
            )
        },
    )
    .ok_or_else(|| {
        format!("character {id}'s template tag token {token} has no LexicalTokens row")
    })?;
    let tag = std::str::from_utf8(&lexical.get(index).unwrap().literal)
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
pub fn civil_war(
    frame: &dyn replay_frame::frame::DecodedSource,
    slot: u32,
) -> Result<bool, String> {
    CivilWars::new(frame)?.holds(slot)
}

/// What [`civil_war`] reads of an hour, for every country: an hour changing
/// none of it since the hour before leaves every country's answer as it was.
pub fn civil_war_reads() -> Vec<crate::triggers::inputs::TableRead> {
    use crate::triggers::inputs::TableRead;
    use replay_data::rows::TableId;
    vec![
        TableRead::fields(TableId::Countries, &["canonical_slot"]),
        TableRead::fields(TableId::CountryLinks, &["canonical_slot"]),
        TableRead::fields(TableId::CountryCivilWars, &["target"]),
        TableRead::fields(TableId::CountryCounterInputs, &["owned_state_count"]),
        TableRead::fields(TableId::CountryWars, &["enemies_read", "enemies"]),
        TableRead::fields(TableId::WarRelations, &["start_hour", "end_hour"]).ending(),
    ]
}

/// The civil-war predicate's inputs at one hour, indexed once, so asking it
/// of every country costs each country only its own rows. Answers, and the
/// reason an answer is refused, are those of scanning the tables in stored
/// order: a reference that does not resolve is reported only where that
/// scan reaches it.
pub struct CivilWars<'a> {
    hour: i32,
    links: &'a [replay_data::rows::CountryLinkRow],
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
/// stored position, and the first whose country does not resolve.
struct Enemies<'a> {
    rows: &'a [replay_data::rows::CountryWarRow],
    by_slot: HashMap<u32, usize>,
    unlinked: Option<(usize, u32)>,
}

/// The other country of each ongoing war relation, by slot in stored order,
/// and the first raw country of one that does not resolve.
struct Relations {
    by_slot: HashMap<u32, Vec<u32>>,
    unlinked: Option<u32>,
}

impl<'a> CivilWars<'a> {
    pub fn new(frame: &'a dyn replay_frame::frame::DecodedSource) -> Result<Self, String> {
        use replay_data::rows::tables;
        let hour = frame.hour();
        let text = |error: std::io::Error| error.to_string();
        let countries = frame.complete::<tables::Countries>().map_err(text)?;
        let links = frame.complete::<tables::CountryLinks>().map_err(text)?;
        let lists = frame.complete::<tables::CountryCivilWars>().map_err(text)?;
        let counters = frame
            .complete::<tables::CountryCounterInputs>()
            .map_err(text)?;
        // Resolved without refusing: a refusal is reported only where a scan reaches it.
        let slot_of = |raw: u32| {
            links
                .binary_search_by_key(&raw, |link| link.id)
                .ok()
                .map(|index| links[index].canonical_slot)
        };
        let mut raw = HashMap::new();
        for country in countries {
            raw.entry(country.canonical_slot).or_insert(country.id);
        }
        let mut owned = HashMap::new();
        for row in counters {
            owned.entry(row.country).or_insert(row.owned_state_count);
        }
        let mut targets = HashMap::<u32, Vec<u32>>::new();
        for row in lists {
            targets.entry(row.country).or_default().push(row.target);
        }
        let wars = frame
            .complete::<tables::CountryWars>()
            .map_err(text)
            .map(|rows| {
                let mut wars = Enemies {
                    rows,
                    by_slot: HashMap::new(),
                    unlinked: None,
                };
                for (index, row) in rows.iter().enumerate() {
                    if row.enemies_read == 0 {
                        continue;
                    }
                    match slot_of(row.country) {
                        Some(slot) => {
                            wars.by_slot.entry(slot).or_insert(index);
                        }
                        None => {
                            wars.unlinked.get_or_insert((index, row.country));
                        }
                    }
                }
                wars
            });
        let relations = frame
            .complete::<tables::WarRelations>()
            .map_err(text)
            .map(|rows| {
                let mut relations = Relations {
                    by_slot: HashMap::new(),
                    unlinked: None,
                };
                for row in rows
                    .iter()
                    .filter(|row| crate::triggers::ongoing(row.start_hour, row.end_hour, hour))
                {
                    match (slot_of(row.first_country), slot_of(row.second_country)) {
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
                relations
            });
        Ok(Self {
            hour,
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
            if self.exists(self.linked(target, "CountryCivilWars")?)? {
                return Ok(true);
            }
        }
        for enemy in self.enemies(slot)? {
            if !self.exists(enemy)? {
                continue;
            }
            for &target in self.targets(self.raw_of(enemy)?) {
                if self.linked(target, "CountryCivilWars")? == slot {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn raw_of(&self, slot: u32) -> Result<u32, String> {
        self.raw.get(&slot).copied().ok_or_else(|| {
            format!("country slot {slot} has no Countries row at hour {}", self.hour)
        })
    }

    /// Whether `slot` owns a state, the game's existence check.
    pub fn exists(&self, slot: u32) -> Result<bool, String> {
        let raw = self.raw_of(slot)?;
        self.owned.get(&raw).map(|&count| count > 0).ok_or_else(|| {
            format!(
                "the owned state count (CountryCounterInputs.owned_state_count) of country {raw} is not recorded at hour {}",
                self.hour
            )
        })
    }

    fn targets(&self, raw: u32) -> &[u32] {
        self.lists.get(&raw).map(Vec::as_slice).unwrap_or_default()
    }

    fn linked(&self, raw: u32, table: &str) -> Result<u32, String> {
        linked(self.links, raw, table, self.hour)
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
            .filter(|&(at, _)| found.is_none_or(|found| at < found))
        {
            self.linked(raw, "CountryWars")?;
        }
        if let Some(index) = found {
            return wars.rows[index]
                .enemies
                .iter()
                .map(|&raw| self.linked(raw, "CountryWars"))
                .collect();
        }
        let relations = self.relations.as_ref().map_err(Clone::clone)?;
        if let Some(raw) = relations.unlinked {
            self.linked(raw, "WarRelations")?;
        }
        Ok(relations.by_slot.get(&slot).cloned().unwrap_or_default())
    }
}

/// A raw country reference's canonical slot, or why the hour cannot say.
fn linked(
    links: &[replay_data::rows::CountryLinkRow],
    raw: u32,
    table: &str,
    hour: i32,
) -> Result<u32, String> {
    crate::metadata::link_slot(links, raw).ok_or_else(|| {
        format!("{table} names country {raw}, which CountryLinks does not hold at hour {hour}")
    })
}

/// A country as idea naming needs it at one hour, owned for the scans that
/// keep it: its canonical slot and its appearance ([`Look`]), `None` when
/// the hour lacks its CountryAppearance row.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NamingCountry {
    pub slot: u32,
    pub look: Option<Look>,
}

/// A country's naming operands: dynamic-gated appearance, the independent
/// original-or-own getter, and whether it has a civil war.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Look {
    pub name_tag: String,
    pub original_or_own_tag: Result<String, String>,
    pub cosmetic_tag: String,
    /// [`civil_war`] for a dynamic country (name_tag set); Ok(false) for any
    /// other country, whose idea naming never asks it.
    pub civil_war: Result<bool, String>,
}

impl Look {
    /// The country's appearance at the frame's hour, `None` without its
    /// CountryAppearance row.
    pub fn read(
        frame: &dyn replay_frame::frame::DecodedSource,
        row: &replay_data::rows::CountryRow,
    ) -> Option<Self> {
        let appearances = crate::triggers::refusal::FrameInputs::trigger_table::<
            replay_data::rows::tables::CountryAppearance,
        >(frame)
        .ok()?;
        let appearance = crate::triggers::refusal::reference(
            appearances
                .iter()
                .find(|appearance| appearance.country == row.id),
            row.id != 0,
            || {
                crate::triggers::refusal::missing_reference(
                    replay_data::rows::TableId::CountryAppearance,
                    Some(crate::triggers::Store::Country(row.canonical_slot)),
                )
            },
        )?;
        Self::read_with_appearance(frame, row, appearance)
    }

    fn read_with_appearance(
        frame: &dyn replay_frame::frame::DecodedSource,
        row: &replay_data::rows::CountryRow,
        appearance: &replay_data::rows::CountryAppearanceRow,
    ) -> Option<Self> {
        Self::read_with(frame, row, appearance, &|slot| civil_war(frame, slot))
    }

    /// [`Self::read_with_appearance`], a dynamic country's civil war answered by `civil_war`.
    fn read_with(
        frame: &dyn replay_frame::frame::DecodedSource,
        row: &replay_data::rows::CountryRow,
        appearance: &replay_data::rows::CountryAppearanceRow,
        civil_war: &dyn Fn(u32) -> Result<bool, String>,
    ) -> Option<Self> {
        let name_tag = std::str::from_utf8(&appearance.name_tag).ok()?;
        let cosmetic_tag = std::str::from_utf8(&appearance.cosmetic_tag).ok()?;
        Some(Self {
            name_tag: name_tag.to_owned(),
            original_or_own_tag: if row.canonical_slot == 0 {
                installed_content_country_names::country_names::reserved_zero_original_tag(frame)
            } else {
                installed_content_country_names::country_names::original_tag(row, appearance)
            }
            .map(str::to_owned),
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
    pub fn read(
        frame: &dyn replay_frame::frame::DecodedSource,
        row: &replay_data::rows::CountryRow,
    ) -> Self {
        Self {
            slot: row.canonical_slot,
            look: Look::read(frame, row),
        }
    }

    /// The same naming getters with the caller's indexed appearance row.
    pub fn read_with_appearance(
        frame: &dyn replay_frame::frame::DecodedSource,
        row: &replay_data::rows::CountryRow,
        appearance: Option<&replay_data::rows::CountryAppearanceRow>,
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
    pub fn read_with_civil_war(
        frame: &dyn replay_frame::frame::DecodedSource,
        row: &replay_data::rows::CountryRow,
        appearance: Option<&replay_data::rows::CountryAppearanceRow>,
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
