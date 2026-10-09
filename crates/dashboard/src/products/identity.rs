use super::{country_groups, politics_content};

/// A recorded character's script token: its lexer id (`CCharacter` +0x18,
/// the row's `name_token_id`) resolved through LexicalTokens. The row's `token`
/// field is the saved name (+0x48), not the script token. None when the id is
/// negative or not in the dictionary.
pub fn character_script_token<'a>(
    character: &replay::rows::CharacterRow,
    lexical: &'a [replay::rows::LexicalTokenRow],
) -> Option<&'a str> {
    let token = u32::try_from(character.name_token_id).ok()?;
    let index = lexical.binary_search_by_key(&token, |row| row.token).ok()?;
    std::str::from_utf8(&lexical[index].literal).ok()
}

/// The template a recorded character holds (CCharacter+0x20), from its
/// Characters row's template columns, its tag (+0x8) a lexer token spelled
/// by LexicalTokens; `Err` names what the hour does not hold.
pub(crate) fn built_template(
    character: &replay::rows::CharacterRow,
    lexical: Option<&[replay::rows::LexicalTokenRow]>,
) -> Result<installed_content::characters::BuiltTemplate, String> {
    let id = character.id;
    if character.template_present == 0 {
        return Err(format!(
            "character {id} holds no template (CCharacter+0x20 is null)"
        ));
    }
    let token = character.template_tag;
    let lexical = lexical.ok_or_else(|| {
        format!("character {id}'s template tag token {token} is not spelled: LexicalTokens is incomplete")
    })?;
    let index = lexical
        .binary_search_by_key(&token, |row| row.token)
        .map_err(|_| {
            format!("character {id}'s template tag token {token} has no LexicalTokens row")
        })?;
    let tag = std::str::from_utf8(&lexical[index].literal)
        .map_err(|_| format!("character {id}'s template tag token {token} is not UTF-8"))?;
    Ok(installed_content::characters::BuiltTemplate {
        tag: tag.to_owned(),
        file: character.template_file.clone(),
        line: character.template_line,
        generated: character.template_generated != 0,
    })
}

/// Whether GetCountryIdeologyGroupIndex (140D2BD20) holds for a country at
/// the frame's hour, as CIdea::GetDisplayNameForCountry (140FBCF90) and
/// `has_civil_war` (1403D3670) call it on the country's CDiplomacyStatus
/// (CCountry+0xF88, whose +0x288 is the country itself): a country of its
/// civil-war list (CCountry+0x12F0, CountryCivilWars) exists (+0x484 > 0,
/// CountryCounterInputs.owned_state_count), or an existing enemy
/// (CDiplomacyStatus+0x98) holds it in its own civil-war list. Each loop
/// only asks whether one holds; the enemy's list is searched by canonical
/// slot (a lower bound over the sorted list, then equality). `Err` names the
/// input the hour lacks.
pub fn civil_war(frame: &replay::frame::DecodedFrame, slot: u32) -> Result<bool, String> {
    use replay::rows::tables;
    let hour = frame.hour;
    let countries = frame
        .complete::<tables::Countries>()
        .map_err(|error| error.to_string())?;
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(|error| error.to_string())?;
    let lists = frame
        .complete::<tables::CountryCivilWars>()
        .map_err(|error| error.to_string())?;
    let counters = frame
        .complete::<tables::CountryCounterInputs>()
        .map_err(|error| error.to_string())?;
    let raw_of = |slot: u32| {
        countries
            .iter()
            .find(|country| country.canonical_slot == slot)
            .map(|country| country.id)
            .ok_or_else(|| format!("country slot {slot} has no Countries row at hour {hour}"))
    };
    let exists = |slot: u32| -> Result<bool, String> {
        let raw = raw_of(slot)?;
        counters
            .iter()
            .find(|row| row.country == raw)
            .map(|row| row.owned_state_count > 0)
            .ok_or_else(|| {
                format!(
                    "the owned state count (CountryCounterInputs.owned_state_count, CCountry+0x484) of country {raw} is not recorded at hour {hour}"
                )
            })
    };
    // The country's own list (+0x288 +0x12F0), its raw owner as recorded.
    let own = raw_of(slot)?;
    for row in lists.iter().filter(|row| row.country == own) {
        if exists(linked(links, row.target, "CountryCivilWars", hour)?)? {
            return Ok(true);
        }
    }
    for enemy in enemies(frame, links, slot)? {
        if !exists(enemy)? {
            continue;
        }
        let raw = raw_of(enemy)?;
        for row in lists.iter().filter(|row| row.country == raw) {
            if linked(links, row.target, "CountryCivilWars", hour)? == slot {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// A raw country reference's canonical slot, or why the hour cannot say.
fn linked(
    links: &[replay::rows::CountryLinkRow],
    raw: u32,
    table: &str,
    hour: i32,
) -> Result<u32, String> {
    crate::metadata::link_slot(links, raw).ok_or_else(|| {
        format!("{table} names country {raw}, which CountryLinks does not hold at hour {hour}")
    })
}

/// A country's enemy vector (CDiplomacyStatus+0x98) as a set: CountryWars'
/// where the recorder read it, else the other country of each ongoing war
/// relation, which RebuildWartimeAllies (140D31E20), its only runtime
/// writer, keeps it to.
fn enemies(
    frame: &replay::frame::DecodedFrame,
    links: &[replay::rows::CountryLinkRow],
    slot: u32,
) -> Result<Vec<u32>, String> {
    use replay::rows::tables;
    let hour = frame.hour;
    let wars = frame
        .complete::<tables::CountryWars>()
        .map_err(|error| error.to_string())?;
    for row in wars {
        if row.enemies_read != 0 && linked(links, row.country, "CountryWars", hour)? == slot {
            return row
                .enemies
                .iter()
                .map(|&raw| linked(links, raw, "CountryWars", hour))
                .collect();
        }
    }
    let relations = frame
        .complete::<tables::WarRelations>()
        .map_err(|error| error.to_string())?;
    let mut out = Vec::new();
    for row in relations
        .iter()
        .filter(|row| country_groups::war_ongoing(row.start_hour, row.end_hour, hour))
    {
        let first = linked(links, row.first_country, "WarRelations", hour)?;
        let second = linked(links, row.second_country, "WarRelations", hour)?;
        if first == slot {
            out.push(second);
        } else if second == slot {
            out.push(first);
        }
    }
    Ok(out)
}

/// A country as idea naming needs it at one hour, owned for the scans that
/// keep it: its canonical slot and its appearance ([`Look`]), `None` when
/// the hour lacks its CountryAppearance row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct NamingCountry {
    pub slot: u32,
    pub look: Option<Look>,
}

/// A country's naming operands: dynamic-gated appearance, the independent
/// original-or-own getter, and whether 140D2BD20 holds for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Look {
    pub name_tag: String,
    pub original_or_own_tag: Result<String, String>,
    pub cosmetic_tag: String,
    /// [`civil_war`] for a dynamic country (`name_tag` set, so 140BA61F0
    /// holds); `Ok(false)` for any other, which 140FBCF90 never asks.
    pub civil_war: Result<bool, String>,
}

impl Look {
    /// The country's appearance at the frame's hour, `None` without its
    /// CountryAppearance row.
    pub fn read(
        frame: &replay::frame::DecodedFrame,
        row: &replay::rows::CountryRow,
    ) -> Option<Self> {
        let appearance = frame
            .complete::<replay::rows::tables::CountryAppearance>()
            .ok()?
            .iter()
            .find(|appearance| appearance.country == row.id)?;
        Self::read_with_appearance(frame, row, appearance)
    }

    fn read_with_appearance(
        frame: &replay::frame::DecodedFrame,
        row: &replay::rows::CountryRow,
        appearance: &replay::rows::CountryAppearanceRow,
    ) -> Option<Self> {
        let name_tag = std::str::from_utf8(&appearance.name_tag).ok()?;
        let cosmetic_tag = std::str::from_utf8(&appearance.cosmetic_tag).ok()?;
        Some(Self {
            name_tag: name_tag.to_owned(),
            original_or_own_tag: if row.canonical_slot == 0 {
                installed_content::country_names::reserved_zero_original_tag(frame)
            } else {
                installed_content::country_names::original_tag(row, appearance)
            }
            .map(str::to_owned),
            cosmetic_tag: cosmetic_tag.to_owned(),
            civil_war: if name_tag.is_empty() {
                Ok(false)
            } else {
                civil_war(frame, row.canonical_slot)
            },
        })
    }

    pub fn country_look(&self) -> politics_content::CountryLook<'_> {
        politics_content::CountryLook {
            name_tag: &self.name_tag,
            original_or_own_tag: self.original_or_own_tag.as_deref().map_err(String::as_str),
            cosmetic_tag: &self.cosmetic_tag,
            civil_war: self.civil_war.as_ref().copied().map_err(String::as_str),
        }
    }
}

impl NamingCountry {
    pub fn read(frame: &replay::frame::DecodedFrame, row: &replay::rows::CountryRow) -> Self {
        Self {
            slot: row.canonical_slot,
            look: Look::read(frame, row),
        }
    }

    /// The same naming getters with the caller's indexed appearance row.
    pub fn read_with_appearance(
        frame: &replay::frame::DecodedFrame,
        row: &replay::rows::CountryRow,
        appearance: Option<&replay::rows::CountryAppearanceRow>,
    ) -> Self {
        Self {
            slot: row.canonical_slot,
            look: appearance
                .filter(|appearance| appearance.country == row.id)
                .and_then(|appearance| Look::read_with_appearance(frame, row, appearance)),
        }
    }

    pub fn for_tag<'a>(&'a self, tag: &'a str) -> politics_content::IdeaCountry<'a> {
        politics_content::IdeaCountry {
            tag,
            slot: self.slot,
            look: self.look.as_ref().map(Look::country_look),
        }
    }
}
