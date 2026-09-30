//! Country and date properties whose operands are recorded.

use super::*;
use replay::rows::PoliticalPartyRow;

// These are executable literals, not a content ideology catalogue:
// 141A7B350 cases 0x15..0x18 / 0x2A..0x2D, bytes 14271D898..14271D8D0.
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
    /// The ruling-party pointer (141A7B350 cases 10 and 15, the pronouns).
    Ruling,
    /// The first party of the group a lexer token names (141A7D890:
    /// FindTokenNoCaseOrDefault 14022EC40, so ASCII A-Z fold, then 140B980C0).
    Token(&'a str),
    /// The first party whose group name is these exact bytes (141A7DB70,
    /// memcmp 142574D50).
    Named(&'a str),
}

impl ScriptedText<'_> {
    /// 141A7B350 case 15 normalizes the ruling group's stored popularity by
    /// the sum of every party before FormatFixed5Trimmed, without a `%`.
    pub(super) fn party_support(&self, slot: u32) -> Result<String, String> {
        let ruling = self
            .party(slot, Party::Ruling)?
            .ok_or("GetPartySupport: ruling-party pointer is absent")?;
        let parties = self
            .frame
            .complete::<tables::PoliticalParties>()
            .map_err(|e| e.to_string())?;
        let mut total = 0_i64;
        for party in parties {
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
        Ok(installed_content::fixed5::format_fixed5(
            value.wrapping_mul(100),
            true,
            5,
        ))
    }

    /// 140CE7BD0: the stored country residual plus the stored available pool
    /// of each state in the controlled-state array, with native signed 32-bit
    /// wrapping addition. `CCountry::AddControlledState@1406C3890` leaves out
    /// a state with no provinces (CState+0x24); a controlled state no recorded
    /// province names as its state is refused, since its count is not
    /// recorded.
    pub(super) fn manpower(&self, slot: u32) -> Result<i32, String> {
        let (countries, gap) = rows::<tables::Countries>(self.frame);
        let country = countries
            .iter()
            .find(|row| row.canonical_slot == slot)
            .ok_or_else(|| format!("GetManpower: Countries {gap}"))?;
        let states = self
            .frame
            .complete::<tables::States>()
            .map_err(|e| format!("GetManpower: {e}"))?;
        let provinces = self
            .frame
            .complete::<tables::Provinces>()
            .map_err(|e| format!("GetManpower: {e}"))?;
        let populated: std::collections::BTreeSet<u32> =
            provinces.iter().map(|province| province.state).collect();
        let mut total = country.manpower_residual;
        for state in states {
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
                        "GetManpower: no province names state {} as its state, so whether it is in the controlled-state array (1406C3890 skips a state without provinces) is unknown",
                        state.id
                    ));
                }
                total = total.wrapping_add(state.manpower_available);
            }
        }
        Ok(total)
    }

    /// 141A7B350 case 0x19: a created agency's stored name (+0x80), even if
    /// empty; otherwise the executable's key `AGENCY_NAME`. Every country
    /// holds an agency object, so a missing row is a gap, not "no agency".
    pub(super) fn agency(&self, slot: u32) -> Result<String, String> {
        let agency = self
            .country_row::<tables::IntelligenceAgencies>(slot, |row| row.country)
            .map_err(|reason| format!("GetAgency: {reason}"))?
            .ok_or_else(|| {
                format!("GetAgency: country slot {slot} has no IntelligenceAgencies row")
            })?;
        if agency.created == 0 {
            return self.localise("AGENCY_NAME");
        }
        // The name is a user's literal or a historical name 140614FD0
        // localised before 140FC8480 or 141A14B60 stored it.
        self.stored_text(&agency.name, "GetAgency: the agency's name (+0x80)")
    }

    /// 141A7B350 cases 5-9 copy the retained old name (CCountry+0x90),
    /// definite name (+0xD0) or adjective (+0xB0), or the current text when
    /// it is empty.
    pub(super) fn old_name(&self, slot: u32, entry: &str, suffix: &str) -> Result<String, String> {
        let row = self
            .country_row::<tables::CountryAppearance>(slot, |row| row.country)
            .map_err(|reason| format!("{entry}: {reason}"))?
            .ok_or_else(|| format!("{entry}: country slot {slot} has no CountryAppearance row"))?;
        let (old, what) = match suffix {
            "_DEF" => (
                &row.old_definite_name,
                "the old definite name (CCountry+0xD0)",
            ),
            "_ADJ" => (&row.old_adjective, "the old adjective (CCountry+0xB0)"),
            _ => (&row.old_name, "the old name (CCountry+0x90)"),
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
        let (characters, gap) = rows::<tables::Characters>(self.frame);
        characters
            .iter()
            .find(|row| row.id == id)
            .map(|row| row.gender)
            .ok_or_else(|| format!("character {id}'s gender: Characters {gap}"))
    }

    // 141A7B350 -> 140FB5FB0: missing party, leader or parent is gender 0.
    pub(super) fn country_pronoun(&self, slot: u32, name: &str) -> Result<String, String> {
        let mut female = false;
        if let Some(party) = self.party(slot, Party::Ruling)? {
            let (leaders, gap) = rows::<tables::PartyLeaders>(self.frame);
            match leaders.iter().find(|row| {
                row.country == party.country && row.party == party.id && row.ordinal == 0
            }) {
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

    // Executable keys, 141A7B350 cases 27..36 and 141A72900 cases 1..10.
    // Character feminine predicate 140F8FAC0 is exactly gender == 2. The Cap calls capitalise the
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
            // SsoString_UppercaseFirstCharacter (1424B6FE0), the "C" locale's toupper.
            Ok(crt_case_text(&text, true, true))
        }
    }

    /// The recorded party `which` names, `None` where the frame proves there
    /// is none.
    pub(super) fn party(
        &self,
        slot: u32,
        which: Party<'_>,
    ) -> Result<Option<&PoliticalPartyRow>, String> {
        let ruling = if matches!(which, Party::Ruling) {
            let (politics, gap) = rows::<tables::Politics>(self.frame);
            let row = politics
                .iter()
                .find(|row| self.scopes.slot(row.country) == Some(slot))
                .ok_or_else(|| format!("party of country {slot}: Politics {gap}"))?;
            let Some(id) = row.ruling_party else {
                return Ok(None);
            };
            Some(id)
        } else {
            None
        };
        let (parties, gap) = rows::<tables::PoliticalParties>(self.frame);
        let party = parties.iter().find(|row| {
            self.scopes.slot(row.country) == Some(slot)
                && match which {
                    Party::Ruling => Some(row.id) == ruling,
                    Party::Token(group) => row.token.eq_ignore_ascii_case(group.as_bytes()),
                    Party::Named(group) => row.token == group.as_bytes(),
                }
        });
        if party.is_none() && ruling.is_some() {
            return Err(format!(
                "party of country {slot}: ruling party has no PoliticalParties row: {gap}"
            ));
        }
        if party.is_some()
            || (complete::<tables::PoliticalParties>(self.frame)
                && (complete::<tables::CountryLinks>(self.frame)
                    || parties
                        .iter()
                        .all(|p| self.scopes.slot(p.country).is_some())))
        {
            Ok(party)
        } else {
            Err(format!("party of country {slot}: PoliticalParties {gap}"))
        }
    }

    /// 14118E5A0 selects vector entry zero. A null CCharacter gives empty
    /// text (140FB5FD0), rather than falling through to the next leader.
    pub(super) fn party_leader(&self, slot: u32, which: Party<'_>) -> Result<String, String> {
        let Some(party) = self.party(slot, which)? else {
            return Ok(String::new());
        };
        let (leaders, gap) = rows::<tables::PartyLeaders>(self.frame);
        match leaders
            .iter()
            .find(|row| row.country == party.country && row.party == party.id && row.ordinal == 0)
        {
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

    /// 141A80CA0: the event scope's date properties use the sampled date.
    pub(super) fn date_property(&self, name: &str) -> Result<Option<String>, String> {
        let date = date_parts(self.frame.hour);
        Ok(Some(if named(name, "GetDateText") {
            self.date_text(self.frame.hour)?
        } else if named(name, "GetDate") {
            format!("{}.{}.{}.{}", date.0, date.1 + 1, date.2 + 1, date.3 + 1)
        } else if named(name, "GetYear") {
            date.0.to_string()
        } else if named(name, "GetMonth") {
            self.month(date.1)?
        } else {
            return Ok(None);
        }))
    }

    // The keys are literal executable operands in 1422542E0.
    fn month(&self, month: usize) -> Result<String, String> {
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
            .map_or_else(|| Ok(String::new()), |key| self.localise(key))
    }

    /// 140526B50 preserves the leading space and uses hours 1..24, including
    /// for GetLastElection. This is not a UI date formatter.
    pub(super) fn date_text(&self, hour: i32) -> Result<String, String> {
        let (year, month, day, hour) = date_parts(hour);
        Ok(format!(
            " {}:00, {} {}, {}",
            hour + 1,
            day + 1,
            self.month(month)?,
            year
        ))
    }
}

// GetMonthZeroBased 140177140 / GetDayOfMonthZeroBased 140176DB0 use the
// executable's fixed 365-day calendar (14306D210), with signed arithmetic.
fn date_parts(raw: i32) -> (i32, usize, i32, i32) {
    let hours = raw.wrapping_sub(engine::cdate::HOUR_EPOCH);
    let mut day = hours / 24 % 365;
    let mut month = 0;
    while month < engine::cdate::MONTH_LENGTHS.len() && day >= engine::cdate::MONTH_LENGTHS[month] {
        day -= engine::cdate::MONTH_LENGTHS[month];
        month += 1;
    }
    (hours / 8760, month, day, hours % 24)
}
