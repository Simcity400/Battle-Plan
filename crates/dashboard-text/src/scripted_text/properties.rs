
use super::*;
use replay_data::rows::PoliticalPartyRow;

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

#[derive(Clone, Copy)]
pub(super) enum Party<'a> {
    Ruling,
    Token(&'a str),
    Named(&'a str),
}

const POPULARITY: u64 = replay_data::rows::mask::<PoliticalPartyRow>(&["popularity"]);

fn first_leader(party: &PoliticalPartyRow) -> u128 {
    u128::from(party.country) << 64 | u128::from(party.id) << 32
}

impl<'a> ScriptedText<'a> {
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
        Ok(content_reader_parser::fixed5::format_fixed5(
            value.wrapping_mul(100),
            true,
            5,
        ))
    }

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
        let country = country.ok_or_else(|| format!("GetManpower: countries: {gap}"))?;
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
                format!("GetAgency: country slot {slot} has no intelligence agencies row")
            })?;
        if agency.created == 0 {
            return self.localise("AGENCY_NAME");
        }
        self.stored_text(&agency.name, "GetAgency: the agency's name")
    }

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
            .ok_or_else(|| format!("{entry}: country slot {slot} has no country appearance row"))?;
        let (old, what) = match suffix {
            "_DEF" => (&row.old_definite_name, "the old definite name"),
            "_ADJ" => (&row.old_adjective, "the old adjective"),
            _ => (&row.old_name, "the old name"),
        };
        let old = old
            .as_ref()
            .ok_or_else(|| format!("{entry}: {what} was unreadable or longer than 256 bytes"))?;
        if old.is_empty() {
            return self.names().name_with(slot, suffix);
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
            .ok_or_else(|| format!("character {id}'s gender: {}: {gap}", replay_data::rows::TableId::Characters.words()))
    }

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
                    return Err(format!("{name}: party leaders: {gap}"));
                }
                None => {}
            }
        }
        self.pronoun(name, female)
    }

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
            Ok(crt_case_text(&text, true, true))
        }
    }

    pub(super) fn party(
        &self,
        slot: u32,
        which: Party<'_>,
    ) -> Result<Option<Cow<'a, PoliticalPartyRow>>, String> {
        let ruling = if matches!(which, Party::Ruling) {
            let (politics, gap) = rows::<tables::Politics>(self.frame, POLITICS);
            let row = first(&politics, |key| self.scopes.slot(key as u32) == Some(slot))?
                .ok_or_else(|| format!("party of country {slot}: politics: {gap}"))?;
            let Some(id) = row.ruling_party else {
                return Ok(None);
            };
            Some(id)
        } else {
            None
        };
        let (parties, gap) = rows::<tables::PoliticalParties>(self.frame, PARTY);
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
                "party of country {slot}: ruling party has no political parties row: {gap}"
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
            Err(format!("party of country {slot}: political parties: {gap}"))
        }
    }

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

    pub(super) fn last_election(&self, slot: u32) -> Result<String, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Text(TextQuery::LastElection(slot)),
            || self.last_election_unrecorded(slot),
        )
    }

    pub(crate) fn last_election_unrecorded(&self, slot: u32) -> Result<String, String> {
        let (politics, gap) = rows::<tables::Politics>(self.frame, POLITICS);
        let row = first(&politics, |key| self.scopes.slot(key as u32) == Some(slot))?
            .ok_or_else(|| format!("GetLastElection: politics: {gap}"))?;
        self.date_text(row.last_election)
    }

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
                "party {}'s first leader: party leaders: {gap}",
                party.id
            )),
        }
    }

    pub(super) fn date_property(&self, name: &str) -> Result<Option<String>, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Text(TextQuery::DateProperty(name.to_owned())),
            || self.date_property_unrecorded(name),
        )
    }

    pub(crate) fn date_property_unrecorded(&self, name: &str) -> Result<Option<String>, String> {
        date_property_at(self.frame.hour(), name, |key| self.localise(key))
    }

    pub fn date_text(&self, hour: i32) -> Result<String, String> {
        date_text_at(hour, |key| self.localise(key))
    }
}

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

fn date_parts(raw: i32) -> (i32, usize, i32, i32) {
    let hours = raw.wrapping_sub(game_model::calendar::HOUR_EPOCH);
    let mut day = hours / 24 % 365;
    let mut month = 0;
    while month < game_model::calendar::MONTH_LENGTHS.len()
        && day >= game_model::calendar::MONTH_LENGTHS[month]
    {
        day -= game_model::calendar::MONTH_LENGTHS[month];
        month += 1;
    }
    (hours / 8760, month, day, hours % 24)
}
