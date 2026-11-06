//! Country and date properties whose operands are recorded.

use super::*;
use replay_data::rows::PoliticalPartyRow;

// Property labels and their ideology-group bindings.
const GROUPS: [(&str, &str); 4] = [
    ("Communist", "communism"),
    ("Democratic", "democratic"),
    ("Fascist", "fascism"),
    ("Neutral", "neutrality"),
];

pub(super) fn leader_group(name: &str) -> Option<&'static str> {
    group(name, "Leader")
}

pub(super) fn party_group(name: &str) -> Option<&'static str> {
    group(name, "Party")
}

fn group(name: &str, suffix: &str) -> Option<&'static str> {
    GROUPS
        .iter()
        .find_map(|(label, token)| named(name, &format!("Get{label}{suffix}")).then_some(*token))
}

/// Which party a property reads.
#[derive(Clone, Copy)]
pub(super) enum Party<'a> {
    /// The ruling party, used by party support and pronouns.
    Ruling,
    /// The first party of the group a lexer token names, with ASCII A-Z folding.
    Token(&'a str),
    /// The first party whose group name is these exact bytes.
    Named(&'a str),
}

const POPULARITY: u64 = replay_data::rows::mask::<PoliticalPartyRow>(&["popularity"]);

/// The PartyLeaders key of `party`'s first leader: country, party and ordinal zero.
fn first_leader(party: &PoliticalPartyRow) -> u128 {
    u128::from(party.country) << 64 | u128::from(party.id) << 32
}

impl<'a> ScriptedText<'a> {
    /// Normalizes the ruling group's stored popularity by the sum of every
    /// party before trimmed fixed5 formatting, without a `%`.
    pub(super) fn party_support(&self, slot: u32) -> Result<String, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Text(TextQuery::PartySupport(slot)),
            || self.party_support_unrecorded(slot),
        )
    }

    pub(crate) fn party_support_unrecorded(&self, slot: u32) -> Result<String, String> {
        let ruling = self
            .party(slot, Party::Ruling)?
            .ok_or("GetPartySupport: ruling-party pointer is absent")?;
        let parties = self
            .frame
            .complete::<tables::PoliticalParties>(POPULARITY)
            .map_err(|e| e.to_string())?;
        let mut total = 0_i64;
        for party in each(parties) {
            let party = party?;
            let owner = self.scopes.slot(party.country).ok_or_else(|| {
                format!(
                    "GetPartySupport: country link {} is unresolved",
                    party.country
                )
            })?;
            if owner == slot {
                total = total.wrapping_add(party.popularity);
            }
        }
        let value = if total == 0 || !(i64::MIN + 2..=i64::MAX - 1).contains(&ruling.popularity) {
            0xffff_ffff
        } else {
            ruling
                .popularity
                .wrapping_mul(100_000)
                .checked_div(total)
                .ok_or("GetPartySupport: native signed quotient overflows")?
        };
        Ok(installed_content_core::fixed5::format_fixed5(
            value.wrapping_mul(100),
            true,
            5,
        ))
    }

    /// The stored country residual plus the stored available pool of each
    /// state in the controlled-state array, with signed 32-bit wrapping
    /// addition. A state with no provinces is excluded; a controlled state
    /// no recorded province names as its state is refused, since its count
    /// is not recorded.
    pub(super) fn manpower(&self, slot: u32) -> Result<i32, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Text(TextQuery::Manpower(slot)),
            || self.manpower_unrecorded(slot),
        )
    }

    pub(crate) fn manpower_unrecorded(&self, slot: u32) -> Result<i32, String> {
        let (countries, gap) = rows::<tables::Countries>(self.frame, COUNTRY);
        let mut country = None;
        for row in each(countries) {
            let row = row?;
            if row.canonical_slot == slot {
                country = Some(row);
                break;
            }
        }
        let country = country.ok_or_else(|| format!("GetManpower: Countries {gap}"))?;
        let states = self
            .frame
            .complete::<tables::States>(STATE)
            .map_err(|e| format!("GetManpower: {e}"))?;
        let provinces = self
            .frame
            .complete::<tables::Provinces>(PROVINCE)
            .map_err(|e| format!("GetManpower: {e}"))?;
        let mut populated = std::collections::BTreeSet::new();
        for province in each(provinces) {
            populated.insert(province?.state);
        }
        let mut total = country.manpower_residual;
        for state in each(states) {
            let state = state?;
            if state.controller == 0 {
                continue;
            }
            let controller = self.scopes.slot(state.controller).ok_or_else(|| {
                format!(
                    "GetManpower: state {}'s controller {} is unresolved",
                    state.id, state.controller
                )
            })?;
            if controller == slot {
                if !populated.contains(&state.id) {
                    return Err(format!(
                        "GetManpower: no province names state {} as its state, so whether it is in the controlled-state array (states without provinces are excluded) is unknown",
                        state.id
                    ));
                }
                total = total.wrapping_add(state.manpower_available);
            }
        }
        Ok(total)
    }

    /// A created agency's stored name, even if empty; otherwise the
    /// localisation key `AGENCY_NAME`. Every country holds an agency object,
    /// so a missing row is a gap, not "no agency".
    pub(super) fn agency(&self, slot: u32) -> Result<String, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Text(TextQuery::Agency(slot)),
            || self.agency_unrecorded(slot),
        )
    }

    pub(crate) fn agency_unrecorded(&self, slot: u32) -> Result<String, String> {
        let agency = self
            .country_row::<tables::IntelligenceAgencies>(slot, AGENCY)
            .map_err(|reason| format!("GetAgency: {reason}"))?
            .ok_or_else(|| {
                format!("GetAgency: country slot {slot} has no IntelligenceAgencies row")
            })?;
        if agency.created == 0 {
            return self.localise("AGENCY_NAME");
        }
        // The name is a user's literal or a historical name
        // localised before it was stored.
        self.stored_text(&agency.name, "GetAgency: the agency's name")
    }

    /// Copies the retained old name, definite name or adjective,
    /// or the current text when it is empty.
    pub(super) fn old_name(&self, slot: u32, entry: &str, suffix: &str) -> Result<String, String> {
        crate::triggers::inputs::query(
            || {
                crate::triggers::inputs::Key::Text(TextQuery::OldName(
                    slot,
                    entry.to_owned(),
                    suffix.to_owned(),
                ))
            },
            || self.old_name_unrecorded(slot, entry, suffix),
        )
    }

    pub(crate) fn old_name_unrecorded(
        &self,
        slot: u32,
        entry: &str,
        suffix: &str,
    ) -> Result<String, String> {
        let row = self
            .country_row::<tables::CountryAppearance>(slot, APPEARANCE)
            .map_err(|reason| format!("{entry}: {reason}"))?
            .ok_or_else(|| format!("{entry}: country slot {slot} has no CountryAppearance row"))?;
        let (old, what) = match suffix {
            "_DEF" => (&row.old_definite_name, "the old definite name"),
            "_ADJ" => (&row.old_adjective, "the old adjective"),
            _ => (&row.old_name, "the old name"),
        };
        let old = old
            .as_ref()
            .ok_or_else(|| format!("{entry}: {what} was unreadable or longer than 256 bytes"))?;
        if old.is_empty() {
            return self.names.name_with(slot, suffix);
        }
        self.stored_text(old, &format!("{entry}: {what}"))
    }

    pub(super) fn character_gender(&self, id: u32) -> Result<u32, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Text(TextQuery::CharacterGender(id)),
            || self.character_gender_unrecorded(id),
        )
    }

    pub(crate) fn character_gender_unrecorded(&self, id: u32) -> Result<u32, String> {
        let (characters, gap) = rows::<tables::Characters>(self.frame, CHARACTER);
        keyed(&characters, u128::from(id))?
            .map(|row| row.gender)
            .ok_or_else(|| format!("character {id}'s gender: Characters {gap}"))
    }

    // A missing party, leader or parent is gender 0.
    pub(super) fn country_pronoun(&self, slot: u32, name: &str) -> Result<String, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Text(TextQuery::CountryPronoun(slot, name.to_owned())),
            || self.country_pronoun_unrecorded(slot, name),
        )
    }

    pub(crate) fn country_pronoun_unrecorded(
        &self,
        slot: u32,
        name: &str,
    ) -> Result<String, String> {
        let mut female = false;
        if let Some(party) = self.party(slot, Party::Ruling)? {
            let (leaders, gap) = rows::<tables::PartyLeaders>(self.frame, PARTY_LEADER);
            match keyed(&leaders, first_leader(&party))? {
                Some(row) => {
                    if let Some(id) = row.character {
                        female = self.character_gender(id)? == 2;
                    }
                }
                None if !complete::<tables::PartyLeaders>(self.frame) => {
                    return Err(format!("{name}: PartyLeaders {gap}"));
                }
                None => {}
            }
        }
        self.pronoun(name, female)
    }

    // Pronoun localisation keys are shared by country and character properties.
    // A character is feminine exactly when gender == 2. The Cap calls capitalise the
    // localised first byte; they do not select different localisation keys.
    pub(super) fn pronoun(&self, name: &str, female: bool) -> Result<String, String> {
        let index = PRONOUNS
            .iter()
            .position(|entry| named(name, entry))
            .expect("pronoun property");
        const KEYS: [[&str; 2]; 5] = [
            ["EV_HE", "EV_SHE"],
            ["EV_HIM", "EV_HER"],
            ["EV_HIS", "EV_HER"],
            ["EV_HIS", "EV_HERS"],
            ["EV_HIMSELF", "EV_HERSELF"],
        ];
        let text = self.localise(KEYS[index / 2][usize::from(female)])?;
        if index % 2 == 0 {
            Ok(text)
        } else {
            // Uppercase the first byte using the "C" locale's toupper.
            Ok(crt_case_text(&text, true, true))
        }
    }

    /// The recorded party `which` names, `None` where the frame proves there
    /// is none.
    pub(super) fn party(
        &self,
        slot: u32,
        which: Party<'_>,
    ) -> Result<Option<Cow<'a, PoliticalPartyRow>>, String> {
        let ruling = if matches!(which, Party::Ruling) {
            let (politics, gap) = rows::<tables::Politics>(self.frame, POLITICS);
            let row = first(&politics, |key| self.scopes.slot(key as u32) == Some(slot))?
                .ok_or_else(|| format!("party of country {slot}: Politics {gap}"))?;
            let Some(id) = row.ruling_party else {
                return Ok(None);
            };
            Some(id)
        } else {
            None
        };
        let (parties, gap) = rows::<tables::PoliticalParties>(self.frame, PARTY);
        // The country is the key's high word, the party id its low word.
        let mut party = None;
        for row in parties.keyed(|key| self.scopes.slot((key >> 32) as u32) == Some(slot)) {
            let row = row.map_err(built)?;
            if match which {
                Party::Ruling => Some(row.id) == ruling,
                Party::Token(group) => row.token.eq_ignore_ascii_case(group.as_bytes()),
                Party::Named(group) => row.token == group.as_bytes(),
            } {
                party = Some(row);
                break;
            }
        }
        if party.is_none() && ruling.is_some() {
            return Err(format!(
                "party of country {slot}: ruling party has no PoliticalParties row: {gap}"
            ));
        }
        if party.is_some()
            || (complete::<tables::PoliticalParties>(self.frame)
                && (complete::<tables::CountryLinks>(self.frame)
                    || parties
                        .keys()
                        .all(|key| self.scopes.slot((key >> 32) as u32).is_some())))
        {
            Ok(party)
        } else {
            Err(format!("party of country {slot}: PoliticalParties {gap}"))
        }
    }

    /// The leader of the ruling party (`group` none) or of the group's first
    /// party.
    pub(super) fn party_leader(&self, slot: u32, group: Option<&str>) -> Result<String, String> {
        crate::triggers::inputs::query(
            || {
                crate::triggers::inputs::Key::Text(TextQuery::PartyLeader(
                    slot,
                    group.map(str::to_owned),
                ))
            },
            || self.party_leader_unrecorded(slot, group),
        )
    }

    pub(crate) fn party_leader_unrecorded(
        &self,
        slot: u32,
        group: Option<&str>,
    ) -> Result<String, String> {
        self.first_party_leader(slot, group.map_or(Party::Ruling, Party::Token))
    }

    /// The long name of the country's first party of exactly `group`, or
    /// empty text where the frame proves there is none.
    pub(super) fn named_party_name(
        &self,
        slot: u32,
        group: &str,
        tag: &str,
    ) -> Result<String, String> {
        crate::triggers::inputs::query(
            || {
                crate::triggers::inputs::Key::Text(TextQuery::NamedPartyName(
                    slot,
                    group.to_owned(),
                    tag.to_owned(),
                ))
            },
            || self.named_party_name_unrecorded(slot, group, tag),
        )
    }

    pub(crate) fn named_party_name_unrecorded(
        &self,
        slot: u32,
        group: &str,
        tag: &str,
    ) -> Result<String, String> {
        match self.party(slot, Party::Named(group))? {
            Some(party) => self.party_name(&party, tag, true),
            None => Ok(String::new()),
        }
    }

    /// The date text of the country's last election.
    pub(super) fn last_election(&self, slot: u32) -> Result<String, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Text(TextQuery::LastElection(slot)),
            || self.last_election_unrecorded(slot),
        )
    }

    pub(crate) fn last_election_unrecorded(&self, slot: u32) -> Result<String, String> {
        let (politics, gap) = rows::<tables::Politics>(self.frame, POLITICS);
        let row = first(&politics, |key| self.scopes.slot(key as u32) == Some(slot))?
            .ok_or_else(|| format!("GetLastElection: Politics {gap}"))?;
        self.date_text(row.last_election)
    }

    /// Selects vector entry zero. A null character gives empty text,
    /// rather than falling through to the next leader.
    fn first_party_leader(&self, slot: u32, which: Party<'_>) -> Result<String, String> {
        let Some(party) = self.party(slot, which)? else {
            return Ok(String::new());
        };
        let (leaders, gap) = rows::<tables::PartyLeaders>(self.frame, PARTY_LEADER);
        match keyed(&leaders, first_leader(&party))? {
            Some(row) => row
                .character
                .map_or_else(|| Ok(String::new()), |id| self.character_name(id)),
            None if complete::<tables::PartyLeaders>(self.frame) => Ok(String::new()),
            None => Err(format!(
                "party {}'s first leader: PartyLeaders {gap}",
                party.id
            )),
        }
    }

    /// The event scope's date properties use the sampled date.
    pub(super) fn date_property(&self, name: &str) -> Result<Option<String>, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Text(TextQuery::DateProperty(name.to_owned())),
            || self.date_property_unrecorded(name),
        )
    }

    pub(crate) fn date_property_unrecorded(&self, name: &str) -> Result<Option<String>, String> {
        date_property_at(self.frame.hour(), name, |key| self.localise(key))
    }

    pub(super) fn date_text(&self, hour: i32) -> Result<String, String> {
        date_text_at(hour, |key| self.localise(key))
    }
}

/// The event scope's date property `name` at `hour`, its months localised by `localise`; None
/// for a name that is no date property.
pub(super) fn date_property_at(
    hour: i32,
    name: &str,
    localise: impl Fn(&str) -> Result<String, String>,
) -> Result<Option<String>, String> {
    let date = date_parts(hour);
    Ok(Some(if named(name, "GetDateText") {
        date_text_at(hour, localise)?
    } else if named(name, "GetDate") {
        format!("{}.{}.{}.{}", date.0, date.1 + 1, date.2 + 1, date.3 + 1)
    } else if named(name, "GetYear") {
        date.0.to_string()
    } else if named(name, "GetMonth") {
        month(date.1, localise)?
    } else {
        return Ok(None);
    }))
}

// Month localisation keys.
fn month(
    month: usize,
    localise: impl Fn(&str) -> Result<String, String>,
) -> Result<String, String> {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    MONTHS
        .get(month)
        .map_or_else(|| Ok(String::new()), |&key| localise(key))
}

/// Preserves the leading space and uses hours 1..24, including
/// for GetLastElection. This is not a UI date formatter.
fn date_text_at(
    hour: i32,
    localise: impl Fn(&str) -> Result<String, String>,
) -> Result<String, String> {
    let (year, month_index, day, hour) = date_parts(hour);
    Ok(format!(
        " {}:00, {} {}, {}",
        hour + 1,
        day + 1,
        month(month_index, localise)?,
        year
    ))
}

// Uses the fixed 365-day calendar with signed arithmetic.
fn date_parts(raw: i32) -> (i32, usize, i32, i32) {
    let hours = raw.wrapping_sub(engine_native::cdate::HOUR_EPOCH);
    let mut day = hours / 24 % 365;
    let mut month = 0;
    while month < engine_native::cdate::MONTH_LENGTHS.len()
        && day >= engine_native::cdate::MONTH_LENGTHS[month]
    {
        day -= engine_native::cdate::MONTH_LENGTHS[month];
        month += 1;
    }
    (hours / 8760, month, day, hours % 24)
}
