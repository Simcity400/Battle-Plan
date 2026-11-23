
use super::{EventFeedItem, EventFeedKind, EventFeedRow, EventFeedStrike, Names, Notices, invalid};
use dashboard_base::{products::at_hour::{self, Read}, queries::country_ids::{self, RunLinks}};
use dashboard_world::products::scalars::nuclear_arsenal;
use dashboard_query::{
    Query,
    log::{Cell, Reader, Row},
    series::PAGE_LIMIT,
};
use replay_data::{
    codec::Decoder,
    event_feed_rows::EventFeedCountry,
    rows::{NuclearStrikePart, NuclearStrikeRow, TableId, tables},
    value::Value,
};
use std::{collections::BTreeMap, io, sync::atomic::AtomicBool};

pub(crate) const LOG: &str = "nuclear-strikes";

const LEVEL_HEALTH: f64 = 10_000_000.0;
const WAR_SUPPORT_PERCENT: f64 = 1_000.0;
const EQUIPMENT_UNIT: f64 = 100_000.0;

fn part_name(part: NuclearStrikePart) -> &'static str {
    match part {
        NuclearStrikePart::WarSupport => "war support",
        NuclearStrikePart::Buildings => "buildings",
        NuclearStrikePart::Units => "units",
        NuclearStrikePart::Equipment => "equipment",
    }
}

pub(crate) fn detail(strike: &NuclearStrikeRow) -> EventFeedStrike {
    let complete = |part| strike.complete(part);
    EventFeedStrike {
        province: strike.province,
        state: (strike.state != 0).then_some(strike.state),
        building_levels: complete(NuclearStrikePart::Buildings).then(|| strike.buildings.iter().map(|building| building.amount_raw as f64).sum::<f64>() / LEVEL_HEALTH),
        war_support: complete(NuclearStrikePart::WarSupport).then(|| strike.war_support.iter().map(|change| change.after_raw as f64 - change.before_raw as f64).sum::<f64>() / WAR_SUPPORT_PERCENT),
        units: complete(NuclearStrikePart::Units).then_some(strike.units.len() as u32),
        manpower_lost: (complete(NuclearStrikePart::Units) && complete(NuclearStrikePart::Equipment)).then(|| {
            let units = strike.units.iter().map(|unit| unit.manpower_before as f64 - unit.manpower_after as f64);
            let crews = strike.wings.iter().map(|wing| wing.manpower_before as f64 - wing.manpower_after as f64);
            units.chain(crews).sum()
        }),
        equipment_lost: complete(NuclearStrikePart::Equipment).then(|| strike.equipment.iter().map(|loss| loss.amount_raw as f64).sum::<f64>() / EQUIPMENT_UNIT),
        not_recorded: strike.refusals.iter().map(|refusal| format!("{}: {} ({}×)", part_name(refusal.part), String::from_utf8_lossy(&refusal.reason), refusal.count)).collect(),
    }
}

fn opaque<T: Value>(cell: &Cell, column: &str) -> io::Result<T> {
    match cell {
        Cell::List(bytes) => {
            let mut decoder = Decoder::new(bytes);
            let value = T::decode(&mut decoder)?;
            decoder.finish()?;
            Ok(value)
        }
        _ => Err(invalid(format!("{} is not recorded", TableId::NuclearStrikes.field_words(column)))),
    }
}

fn number(cell: &Cell, column: &str) -> io::Result<i64> {
    match cell {
        Cell::Number(value) => Ok(*value),
        _ => Err(invalid(format!("{} is not recorded", TableId::NuclearStrikes.field_words(column)))),
    }
}

pub(crate) struct Columns(BTreeMap<&'static str, usize>);

impl Columns {
    pub(crate) fn of(names: &[&'static str]) -> Self {
        Self(names.iter().enumerate().map(|(at, &name)| (name, at)).collect())
    }

    fn cell<'r>(&self, row: &'r Row, name: &str) -> io::Result<&'r Cell> {
        self.0.get(name).and_then(|&at| row.cells.get(at)).ok_or_else(|| invalid(format!("The nuclear strikes log has no column {name}")))
    }

    pub(crate) fn strike(&self, row: &Row) -> io::Result<NuclearStrikeRow> {
        let id = u64::try_from(row.key).map_err(|_| invalid("A nuclear strike's id is out of range"))?;
        let unsigned = |name: &str| number(self.cell(row, name)?, name).and_then(|value| u32::try_from(value).map_err(|_| invalid(format!("{} is out of range", TableId::NuclearStrikes.field_words(name)))));
        Ok(NuclearStrikeRow {
            id,
            hour: i32::try_from(number(self.cell(row, "hour")?, "hour")?).map_err(|_| invalid("hour of nuclear strikes is out of range"))?,
            launcher: opaque(self.cell(row, "launcher")?, "launcher")?,
            nuke_type: unsigned("nuke_type")?,
            province: unsigned("province")?,
            state: unsigned("state")?,
            owner: opaque(self.cell(row, "owner")?, "owner")?,
            controller: unsigned("controller")?,
            war_support: opaque(self.cell(row, "war_support")?, "war_support")?,
            buildings: opaque(self.cell(row, "buildings")?, "buildings")?,
            units: opaque(self.cell(row, "units")?, "units")?,
            equipment: opaque(self.cell(row, "equipment")?, "equipment")?,
            refusals: opaque(self.cell(row, "refusals")?, "refusals")?,
            wings: opaque(self.cell(row, "wings")?, "wings")?,
        })
    }
}

pub(crate) fn tag(links: &RunLinks, names: &Names, country: EventFeedCountry, hour: i32) -> Option<String> {
    if country.reference as i32 <= 0 {
        return None;
    }
    let slot = links.slot_at(country.reference, hour)?;
    if country.id != 0 && links.slot_at(country.id, hour) != Some(slot) {
        return None;
    }
    names.slot_tag(slot, hour).map(str::to_owned)
}

pub(crate) fn row(strike: &NuclearStrikeRow, read_hour: i32, participants: [Option<String>; 2], item: Option<EventFeedItem>) -> EventFeedRow {
    let kind = EventFeedKind::NuclearStrike;
    EventFeedRow {
        id: format!("nuclear:{}", strike.id),
        hour: strike.hour,
        read_hour: read_hour.max(strike.hour),
        name_hour: strike.hour,
        exact: true,
        category: kind.category(),
        kind,
        tags: participants.iter().flatten().cloned().collect(),
        participants: Some(participants),
        item,
        amount: None,
        cause: None,
        strike: Some(detail(strike)),
    }
}

fn items(query: &Query<'_>, types: &[u32], localise: &dyn Fn(&str) -> Option<String>, notices: &mut Notices) -> BTreeMap<u32, EventFeedItem> {
    let tokens: Vec<(u32, u32)> = types.iter().filter_map(|&ordinal| nuclear_arsenal::type_token(ordinal).map(|token| (ordinal, token))).collect();
    let mut out = BTreeMap::new();
    if tokens.is_empty() {
        return out;
    }
    let spelled: BTreeMap<u32, String> = at_hour::frame(query, query.timeline.last, &[Read::these(TableId::LexicalTokens, tokens.iter().map(|&(_, token)| u128::from(token)))], "Nuclear strikes")
        .and_then(|frame| frame.complete::<tables::LexicalTokens>().map(|rows| rows.iter().filter_map(|row| Some((row.token, String::from_utf8(row.literal.clone()).ok()?))).collect()))
        .unwrap_or_default();
    for (ordinal, token) in tokens {
        let spelling = spelled.get(&token).cloned();
        if spelling.is_none() {
            notices.sampled("nuclearType", "Nuclear type", format!("Nuclear type {ordinal}'s token {token} has no recorded spelling."), 1);
        }
        let name = spelling.as_deref().and_then(localise);
        out.insert(ordinal, EventFeedItem { token: spelling, name, ..Default::default() });
    }
    out
}

pub(crate) fn strike_rows(query: &Query<'_>, from: i32, to: i32, localise: &dyn Fn(&str) -> Option<String>, notices: &mut Notices, cancelled: &AtomicBool) -> io::Result<Vec<EventFeedRow>> {
    let declared = dashboard_declare::log(LOG).ok_or_else(|| invalid("The nuclear strikes log is not declared"))?;
    let reader = Reader::new(query, declared)?;
    let selection = reader.select(&[], query.timeline.first, to)?;
    let columns = Columns::of(&reader.columns());
    let mut strikes = Vec::new();
    let mut cursor = 0;
    loop {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let page = reader.page(&selection.patterns, selection.groups.len(), from, to, cursor, PAGE_LIMIT)?;
        for paged in &page.rows {
            match columns.strike(&paged.row) {
                Ok(strike) => strikes.push((paged.row.hour, strike)),
                Err(error) => notices.sampled("nuclearStrikeCoverage", "Nuclear strikes", format!("A strike read at {} is not shown: {error}", paged.row.hour), 1),
            }
        }
        match page.cursor() {
            Some(next) => cursor = next,
            None => break,
        }
    }
    let mut types: Vec<u32> = strikes.iter().map(|(_, strike)| strike.nuke_type).collect();
    types.sort_unstable();
    types.dedup();
    let items = items(query, &types, localise, notices);
    let names = Names::new(&query.holders);
    let recording = query.replay.recording();
    let mut links: BTreeMap<u32, RunLinks> = BTreeMap::new();
    let mut rows = Vec::with_capacity(strikes.len());
    for (read_hour, strike) in strikes {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let Some(stretch) = query.timeline.at(strike.hour) else {
            notices.sampled("nuclearStrikeCountry", "Nuclear strike country", format!("Strike {} at {} is dated at an hour the timeline does not hold; its countries are not named.", strike.id, strike.hour), 1);
            rows.push(row(&strike, read_hour, [None, None], items.get(&strike.nuke_type).cloned()));
            continue;
        };
        let held = match links.entry(stretch.run) {
            std::collections::btree_map::Entry::Occupied(held) => held.into_mut(),
            std::collections::btree_map::Entry::Vacant(slot) => slot.insert(country_ids::run_links(recording, stretch.run)?),
        };
        let participants = [strike.launcher, strike.owner].map(|country| tag(held, &names, country, strike.hour));
        for (role, named, expected) in [("launcher", &participants[0], true), ("target owner", &participants[1], strike.state != 0)] {
            if expected && named.is_none() {
                notices.sampled("nuclearStrikeCountry", "Nuclear strike country", format!("The {role} of strike {} at {} is named by no link and holder.", strike.id, strike.hour), 1);
            }
        }
        if !items.contains_key(&strike.nuke_type) {
            notices.sampled("nuclearType", "Nuclear type", format!("Nuclear type {} is not one the game names.", strike.nuke_type), 1);
        }
        rows.push(row(&strike, read_hour, participants, items.get(&strike.nuke_type).cloned()));
    }
    Ok(rows)
}
