use replay_data::rows::{CountryLinkRow, Recorded, TableId, tables};
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io};

#[derive(Clone, Debug, PartialEq)]
pub struct PeopleExile {
    pub tag: String,
    pub host: Option<String>,
    pub legitimacy: Option<String>,
    pub manpower: Option<i32>,
    pub cap: Option<i32>,
    pub leaders: Option<i32>,
    pub deployed_air: Option<i32>,
    pub read_hour: Option<i32>,
}

fn scalar<T: Copy>(value: &Recorded<Option<T>>) -> Option<T> {
    value.require("people operand").ok().copied().flatten()
}

fn recorded<T: Copy>(value: &Recorded<T>) -> Option<T> {
    value.require("people operand").ok().copied()
}

pub fn election_date(last: i32, interval: i32) -> i32 {
    use game_model::calendar::{HOUR_EPOCH, MONTH_LENGTHS};
    fn parts(hour: i32) -> (i32, i32, i32) {
        let days = hour.wrapping_sub(HOUR_EPOCH) / 24;
        let ordinal = days % 365;
        let mut month = 0;
        let mut prefix = 0;
        while month < 11 && ordinal >= prefix + MONTH_LENGTHS[month] {
            prefix += MONTH_LENGTHS[month];
            month += 1;
        }
        (days, month as i32, prefix)
    }
    fn replace_year(hour: i32, year: i32) -> i32 {
        let (days, _, _) = parts(hour);
        let ordinal = days % 365;
        let absolute = year.wrapping_abs().wrapping_mul(365).wrapping_add(ordinal);
        let shifted = if year < 0 { absolute.wrapping_neg() } else { absolute };
        hour.wrapping_add(shifted.wrapping_sub(days).wrapping_mul(24))
    }
    let (days, month, _) = parts(last);
    let combined = month.wrapping_add(interval);
    let mut years = combined / 12;
    let mut destination = combined;
    let mut hour = last;
    if years == 0 {
        if combined < 0 {
            hour = replace_year(hour, (days / 365).wrapping_sub(1));
            destination = destination.wrapping_add(12);
        }
    } else {
        hour = replace_year(hour, (days / 365).wrapping_add(years));
        if combined < 0 {
            destination = destination.wrapping_add(12);
            years = destination / 12;
        }
        destination = destination.wrapping_sub(years.wrapping_mul(12));
    }
    let (days, _, old_prefix) = parts(hour);
    let year = days / 365;
    let new_prefix: i32 = MONTH_LENGTHS[..destination.max(0) as usize].iter().sum();
    let ordinal = (days % 365).wrapping_sub(old_prefix).wrapping_add(new_prefix);
    let absolute = year.wrapping_abs().wrapping_mul(365).wrapping_add(ordinal);
    let shifted = if year < 0 { absolute.wrapping_neg() } else { absolute };
    hour.wrapping_add(shifted.wrapping_sub(days).wrapping_mul(24))
}

pub fn next_election(last: i32, interval: Option<i32>, cache: Option<Option<i32>>) -> Option<i32> {
    if engine_native_politics::cpolitical_status::needs_election_cache(last) {
        cache.flatten()
    } else {
        interval.map(|interval| election_date(last, interval))
    }
}

pub fn next_election_date(row: &replay_data::rows::PoliticsRow) -> Option<i32> {
    next_election(row.last_election, recorded(&row.election_interval), recorded(&row.next_election_cache))
}

pub(crate) struct Links<'a> {
    links: &'a [CountryLinkRow],
    tags: BTreeMap<u32, &'a [u8]>,
}

impl<'a> Links<'a> {
    pub(crate) fn new(frame: &'a DecodedFrame) -> io::Result<Self> {
        let countries = frame.complete::<tables::Countries>()?;
        let links = frame.complete::<tables::CountryLinks>()?;
        Ok(Self { links, tags: countries.iter().map(|row| (row.canonical_slot, row.tag.as_slice())).collect() })
    }

    pub(crate) fn slot(&self, raw: u32) -> Option<u32> {
        self.links.binary_search_by_key(&raw, |link| link.id).ok().map(|at| self.links[at].canonical_slot)
    }

    pub(crate) fn slot_tag(&self, slot: u32) -> Option<String> {
        self.tags.get(&slot).and_then(|tag| std::str::from_utf8(tag).ok()).filter(|tag| !tag.is_empty()).map(str::to_owned)
    }

    pub(crate) fn tag(&self, raw: u32) -> Option<String> {
        self.slot_tag(self.slot(raw)?)
    }
}

pub const EXILE_TABLES: [TableId; 5] = [TableId::Countries, TableId::CountryLinks, TableId::CountryStabilityInputs, TableId::SliceReads, TableId::CountryDeployedManpower];

pub struct ExileAt {
    pub country: u32,
    pub slot: Option<u32>,
    pub exile: PeopleExile,
}

pub fn exile_rows(frame: &DecodedFrame, include: impl Fn(u32, u32) -> bool) -> (Vec<ExileAt>, [u32; 5]) {
    let mut refusals = [0_u32; 5];
    let mut out = Vec::new();
    let (Ok(links), Ok(rows)) = (Links::new(frame), frame.complete::<tables::CountryStabilityInputs>()) else {
        return (out, refusals);
    };
    let deployed = frame.complete::<tables::CountryDeployedManpower>().ok();
    let reads = frame.complete::<tables::SliceReads>().ok();
    for row in rows {
        if scalar(&row.exile_host).is_none() {
            refusals[0] += 1;
        }
        let Some(host) = scalar(&row.exile_host).filter(|host| *host > 0) else { continue };
        if !include(row.country, host as u32) {
            continue;
        }
        for (index, available) in [
            scalar(&row.exile_legitimacy).is_some(),
            scalar(&row.exile_manpower).is_some(),
            scalar(&row.exile_manpower_cap).is_some(),
            scalar(&row.exile_leaders).is_some(),
        ]
        .into_iter()
        .enumerate()
        {
            if !available {
                refusals[index + 1] += 1;
            }
        }
        let deployed_air = links.slot(row.country).and_then(|slot| {
            let rows = deployed?;
            let at = rows.binary_search_by_key(&slot, |row| row.slot).ok()?;
            recorded(&rows[at].exile_air)
        });
        out.push(ExileAt { country: row.country, slot: links.slot(row.country), exile: PeopleExile {
            tag: links.tag(row.country).unwrap_or_else(|| format!("Country {}", row.country)),
            host: links.tag(host as u32),
            legitimacy: scalar(&row.exile_legitimacy).map(|value| value.to_string()),
            manpower: scalar(&row.exile_manpower),
            cap: scalar(&row.exile_manpower_cap),
            leaders: scalar(&row.exile_leaders),
            deployed_air,
            read_hour: reads.and_then(|reads| replay_data::sliced::read_hour(reads, TableId::CountryStabilityInputs, row.country)),
        } });
    }
    (out, refusals)
}

#[cfg(test)]
mod tests;
