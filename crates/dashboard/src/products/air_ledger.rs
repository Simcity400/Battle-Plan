//! Aircraft losses and applied bombing damage: the declared logs `air-losses` (AirLossEvents) and
//! `bombing-damage` (BuildingBombingDamage) folded into running totals at the reply. A point is
//! dated by the event's own game hour and ordered by the hour its drain was read; a range starts
//! at the game hour, which totals kept by read hour could not reproduce, so the events are read.
//! Bombing counts only the air-mission and air-naval callers of building damage. An hour the event
//! table or the links were not complete certifies nothing; a complete hour certifies no events.
//! Nothing walks hours: which hours were complete comes from the tables' status runs, the events
//! from the log's buckets that hold rows. A reply is one window of hours ([`super::windows`]), its
//! points running from the window's first hour and its openings each part's total before it, read
//! from the running totals the recorder derives (AirLossTotals, BombingTotals) at the edges of the
//! stretches before the window, so one window shows and ranks every holder's totals since the
//! first hour counted. Where those tables cannot say (withheld, a total past 64 bits), and for the
//! events read after a range's first hour but dated before it, the events are folded instead.

use super::{LoadContext, Product};
use crate::{Source, catalogue, ranking, span::HourSpan};
use dashboard_query::{
    Query,
    identity::Holder,
    log::{self, Cell, Row},
    read,
};
use replay::{columnar_frames::Recording, lanes::StatusRun};
use replay_columnar::{manifest::Status, summary};
use replay_data::rows::{AirLossTotalRow, BombingTotalRow, BuildingBombingDamageCause, TableId};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, Mutex, PoisonError, atomic::AtomicBool},
};
use ts_rs::TS;

#[cfg(test)]
mod reference;
#[cfg(test)]
mod tests;

pub struct MilitaryAirLedger;

pub const AIR_LOSSES: &str = "air-losses";
pub const BOMBING: &str = "bombing-damage";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum MilitaryAirFamily {
    AirLosses,
    Bombing,
}

impl MilitaryAirFamily {
    fn log(self) -> &'static str {
        match self {
            Self::AirLosses => AIR_LOSSES,
            Self::Bombing => BOMBING,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum MilitaryAirMetric {
    Dealt,
    Taken,
    Total,
}

/// A top N the holders of a window are ranked by, each by its total since the first hour counted.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MilitaryAirTop {
    pub count: usize,
    pub metric: MilitaryAirMetric,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MilitaryAirParams {
    pub branch_id: String,
    pub family: MilitaryAirFamily,
    #[serde(default)]
    pub regions: Vec<u32>,
    #[serde(default)]
    pub causes: Vec<u32>,
    /// A range starts at the events' own game hour.
    #[serde(default)]
    pub from_hour: Option<i32>,
    pub window: u32,
    #[serde(default)]
    pub top: Option<MilitaryAirTop>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct MilitaryAirCountry {
    pub tag: String,
    /// Canonical slot and raw country id of the holder; tag changes do not join holders.
    pub identity: String,
    pub present: Vec<HourSpan>,
    /// Null means no ranking filter. Empty means never in the requested top N within the window.
    pub membership: Option<Vec<HourSpan>>,
}

/// A total from the first hour counted through the hour before the window: a holder's (by its
/// identity) or the unattributed, on one side, of one cause and, of air losses, whether the
/// opponents were recorded; summed over the regions, buildings and states asked.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct MilitaryAirOpening {
    pub identity: Option<String>,
    pub dealt: bool,
    pub cause: u32,
    pub opponent_known: Option<bool>,
    pub total: String,
}

/// One running total a reply shows: a holder's, or the unattributed, on one side. Air losses keep
/// each cause and whether its opponents were recorded; bombing keeps each cause, building and
/// state, so a holder's breakdown opens from the reply it is in.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase")]
pub struct MilitaryAirPart {
    pub tag: Option<String>,
    pub identity: Option<String>,
    pub dealt: bool,
    pub cause: Option<u32>,
    /// Air losses: whether the opposite endpoint was resolved for every loss in it.
    pub opponent_known: Option<bool>,
    pub building_token: Option<String>,
    pub building_id: Option<u32>,
    pub state: Option<u32>,
    /// Each point is `[hour, readHour, totalRaw]`: the event's game hour, the hour its drain was
    /// read and the exact total through it since the window's first hour, in read order.
    #[ts(type = "Array<[number, number, string]>")]
    pub points: Vec<(i32, i32, String)>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MilitaryAirName {
    pub id: String,
    pub name: Option<String>,
    pub missing: Option<String>,
}

/// How many events of a window one reason left out or left unnamed.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct MilitaryAirNote {
    pub reason: String,
    pub events: usize,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MilitaryAirReply {
    pub family: MilitaryAirFamily,
    pub window: u32,
    pub first_hour: i32,
    pub last_hour: i32,
    /// The hour the totals count from: the range's first, else the timeline's first observed;
    /// none while no hour through the window's last is observed.
    pub counted_from: Option<i32>,
    pub available: bool,
    pub observed: Vec<HourSpan>,
    /// Reads holding a caller whose membership in aircraft bombing is unknown.
    pub incomplete: Vec<HourSpan>,
    pub countries: Vec<MilitaryAirCountry>,
    pub openings: Vec<MilitaryAirOpening>,
    pub parts: Vec<MilitaryAirPart>,
    pub regions: Vec<MilitaryAirName>,
    pub buildings: Vec<MilitaryAirName>,
    pub notes: Vec<MilitaryAirNote>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(MilitaryAirReply);

/// A holder as the reply names it: its tag and `"slot:id"`.
type Named = (String, String);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct LeafKey {
    country: Option<Named>,
    dealt: bool,
    opponent: Option<String>,
    region: Option<u32>,
    /// The building type token and its spelling.
    building: Option<(u32, Option<String>)>,
    state: Option<u32>,
    cause: u32,
}

/// One event as the fold reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    Air { id: u64, game_hour: i32, country: u32, region: u32, cause: u32, credited_enemy: bool, amount: i64, opponent: Option<u32> },
    Bombing { id: u64, game_hour: i32, attacker: Option<u32>, victim: u32, state: u32, building: u32, cause: BuildingBombingDamageCause, amount: i64 },
}

/// What a stretch of hours certified: the event table not complete (an unread interval), the
/// event table complete without the links (events not attributable, nothing certified), or both
/// complete (every event of the stretch read).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Held {
    Unread,
    Unjoined,
    Observed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    pub first: i32,
    pub last: i32,
    pub held: Held,
}

/// What the fold of one family's hours holds: the reply before naming and ranking.
#[derive(Default)]
pub(crate) struct Fold {
    observed: Vec<HourSpan>,
    incomplete: Vec<HourSpan>,
    leaves: BTreeMap<LeafKey, BTreeMap<(i32, i32), i128>>,
    warnings: BTreeMap<String, usize>,
    last_event: Option<u64>,
    /// The first hour of an unread interval since the last read with events: the collector may
    /// restart its event identities across it.
    unread_since: Option<i32>,
}

fn append_hour(spans: &mut Vec<HourSpan>, hour: i32) {
    match spans.last_mut() {
        Some(last) if last.last_hour.checked_add(1) == Some(hour) => last.last_hour = hour,
        _ => spans.push(HourSpan { first_hour: hour, last_hour: hour }),
    }
}

impl Fold {
    fn note(&mut self, reason: impl Into<String>) {
        *self.warnings.entry(reason.into()).or_default() += 1;
    }

    fn add(&mut self, key: LeafKey, hour: i32, read_hour: i32, amount: i64) -> io::Result<()> {
        let entry = self.leaves.entry(key).or_default().entry((read_hour, hour)).or_default();
        *entry = entry.checked_add(i128::from(amount)).ok_or_else(|| dashboard_core::invalid("military air event sum overflow"))?;
        Ok(())
    }

    /// The first read with events after an unread interval may restart the identities only when
    /// every event in it is dated within or after that interval; else they are earlier events
    /// recorded again.
    fn begin_read(&mut self, events: &str, first: Option<u64>, earliest: Option<i32>) -> io::Result<()> {
        let (Some(first), Some(earliest), Some(since)) = (first, earliest, self.unread_since) else { return Ok(()) };
        self.unread_since = None;
        if self.last_event.is_some_and(|last| first <= last) {
            if earliest < since {
                return Err(dashboard_core::invalid(format!(
                    "{events} events restart their ordinals after the unread interval from hour {since} but are dated from hour {earliest}, before it: earlier events recorded again, not new ones"
                )));
            }
            self.last_event = None;
        }
        Ok(())
    }

    fn event(&mut self, id: u64) -> io::Result<()> {
        if self.last_event.is_some_and(|last| id <= last) {
            return Err(dashboard_core::invalid("military air event ordinal repeated or moved backwards"));
        }
        self.last_event = Some(id);
        Ok(())
    }

    /// One stretch of `family`'s hours and, when it is observed, the events each of its hours
    /// read, ascending by hour: `resolve` names a raw country at an hour, `spell` a building token.
    pub(crate) fn span(&mut self, family: MilitaryAirFamily, span: Span, reads: &[(i32, Vec<Event>)], resolve: impl Fn(u32, i32) -> Option<Named>, spell: impl Fn(u32) -> Option<String>) -> io::Result<()> {
        match span.held {
            Held::Unread => {
                self.unread_since.get_or_insert(span.first);
                return Ok(());
            }
            Held::Unjoined => return Ok(()),
            Held::Observed => {}
        }
        match self.observed.last_mut() {
            Some(last) if last.last_hour.checked_add(1) == Some(span.first) => last.last_hour = span.last,
            _ => self.observed.push(HourSpan { first_hour: span.first, last_hour: span.last }),
        }
        for (hour, events) in reads {
            self.read(family, *hour, events, |raw| resolve(raw, *hour), &spell)?;
        }
        Ok(())
    }

    /// The events one observed hour's drain read, ascending by ordinal.
    fn read(&mut self, family: MilitaryAirFamily, hour: i32, events: &[Event], resolve: impl Fn(u32) -> Option<Named>, spell: impl Fn(u32) -> Option<String>) -> io::Result<()> {
        let first = events.first().map(|event| match event {
            Event::Air { id, .. } | Event::Bombing { id, .. } => *id,
        });
        let earliest = events
            .iter()
            .map(|event| match event {
                Event::Air { game_hour, .. } | Event::Bombing { game_hour, .. } => *game_hour,
            })
            .min();
        let name = |raw: Option<u32>| raw.filter(|raw| *raw != 0).and_then(&resolve);
        match family {
            MilitaryAirFamily::AirLosses => {
                self.begin_read("air loss", first, earliest)?;
                for event in events {
                    let Event::Air { id, game_hour, country, region, cause, credited_enemy, amount, opponent } = *event else { continue };
                    self.event(id)?;
                    if game_hour > hour {
                        return Err(dashboard_core::invalid("air event is dated after its supplying read"));
                    }
                    let country = name(Some(country));
                    if country.is_none() {
                        self.note("air event endpoint is not resolved by its recorded country links");
                    }
                    if !(1..=6).contains(&cause) {
                        self.note(format!("air loss cause {cause} has no proved display meaning; its exact amount is retained"));
                    }
                    let opponent = name(opponent).map(|named| named.0);
                    self.add(LeafKey { country, dealt: credited_enemy, opponent, region: Some(region), building: None, state: None, cause }, game_hour, hour, amount)?;
                }
            }
            MilitaryAirFamily::Bombing => {
                self.begin_read("bombing", first, earliest)?;
                if events.iter().any(|event| matches!(event, Event::Bombing { cause: BuildingBombingDamageCause::Unknown, .. })) {
                    append_hour(&mut self.incomplete, hour);
                }
                for event in events {
                    let Event::Bombing { id, game_hour, attacker, victim, state, building, cause, amount } = *event else { continue };
                    self.event(id)?;
                    if cause == BuildingBombingDamageCause::Unknown {
                        self.note("building damage has an unsupported caller; its contribution to aircraft bombing is not known");
                        continue;
                    }
                    if !matches!(cause, BuildingBombingDamageCause::AirMission | BuildingBombingDamageCause::AirNaval) {
                        continue;
                    }
                    if game_hour > hour {
                        return Err(dashboard_core::invalid("bombing event is dated after its supplying read"));
                    }
                    let token = spell(building).filter(|token| !token.is_empty());
                    if token.is_none() {
                        self.note("building type is not spelled by the recorded lexical dictionary");
                    }
                    for (dealt, raw) in [(true, attacker), (false, Some(victim))] {
                        let country = name(raw);
                        if country.is_none() {
                            self.note(if dealt { "bombing attacker not recorded" } else { "bombing victim not recorded" });
                        }
                        let key = LeafKey { country, dealt, opponent: None, region: None, building: Some((building, token.clone())), state: Some(state), cause: cause as u32 };
                        self.add(key, game_hour, hour, amount)?;
                    }
                }
            }
        }
        Ok(())
    }
}

/// Each canonical slot's holders over the timeline: tag, `"slot:id"` and hours.
pub(crate) fn slot_holders(query: &Query<'_>) -> HashMap<u32, Vec<(String, String, i32, i32)>> {
    let mut out: HashMap<u32, Vec<(String, String, i32, i32)>> = HashMap::new();
    for (tag, runs) in &query.holders.tags {
        for run in runs {
            if let Holder::Found { slot, id } = run.holder {
                out.entry(slot).or_default().push((tag.clone(), format!("{slot}:{id}"), run.first, run.last));
            }
        }
    }
    for runs in out.values_mut() {
        runs.sort_by_key(|run| run.2);
    }
    out
}

fn number(row: &Row, column: usize) -> Option<i64> {
    match row.cells.get(column)? {
        Cell::Number(value) => Some(*value),
        _ => None,
    }
}

/// An optional number cell: none when missing, refused (outer none) when not recorded.
fn optional(row: &Row, column: usize) -> Option<Option<u32>> {
    match row.cells.get(column)? {
        Cell::Missing => Some(None),
        Cell::Number(value) => Some(Some(*value as u32)),
        _ => None,
    }
}

/// The log's table columns the fold reads, by name.
struct Columns(Vec<usize>);

impl Columns {
    fn of(table: TableId, names: &[&str]) -> io::Result<Self> {
        names.iter().map(|name| dashboard_declare::field(table, name)).collect::<io::Result<Vec<_>>>().map(Self)
    }
}

/// The event a logged row of `family`'s table holds; none when a cell the fold needs is not
/// recorded.
fn event(family: MilitaryAirFamily, row: &Row, columns: &Columns) -> Option<Event> {
    let id = row.key as u64;
    let c = &columns.0;
    match family {
        MilitaryAirFamily::AirLosses => Some(Event::Air {
            id,
            game_hour: number(row, c[0])? as i32,
            country: number(row, c[1])? as u32,
            region: number(row, c[2])? as u32,
            cause: number(row, c[3])? as u32,
            credited_enemy: number(row, c[4])? != 0,
            amount: number(row, c[5])?,
            opponent: optional(row, c[6])?,
        }),
        MilitaryAirFamily::Bombing => Some(Event::Bombing {
            id,
            game_hour: number(row, c[0])? as i32,
            attacker: optional(row, c[1])?,
            victim: number(row, c[2])? as u32,
            state: number(row, c[3])? as u32,
            building: number(row, c[4])? as u32,
            cause: BuildingBombingDamageCause::try_from(number(row, c[5])? as u8).ok()?,
            amount: number(row, c[6])?,
        }),
    }
}

const AIR_COLUMNS: [&str; 7] = ["game_hour", "country", "region", "native_cause", "credited_enemy", "amount_raw", "opponent"];
const BOMBING_COLUMNS: [&str; 7] = ["game_hour", "attacker", "victim", "state", "building_type", "cause", "amount_raw"];

fn available(status: Status) -> bool {
    matches!(status, Status::Complete | Status::Truncated { .. })
}

/// A stretch's hours as the fold reads them, from the event table's and the links' status runs
/// over it: runs of one [`Held`], joined. An hour neither table's runs cover was not recorded.
pub(crate) fn spans(first: i32, last: i32, events: &[StatusRun], links: &[StatusRun]) -> Vec<Span> {
    let status_at = |runs: &[StatusRun], at: &mut usize, hour: i32| -> (Status, i32) {
        while *at < runs.len() && runs[*at].last < hour {
            *at += 1;
        }
        match runs.get(*at) {
            Some(run) if run.first <= hour => (run.status, run.last),
            Some(run) => (Status::NotRecorded, run.first - 1),
            None => (Status::NotRecorded, i32::MAX),
        }
    };
    let mut out: Vec<Span> = Vec::new();
    let (mut a, mut b) = (0, 0);
    let mut hour = first;
    while hour <= last {
        let (event, event_last) = status_at(events, &mut a, hour);
        let (link, link_last) = status_at(links, &mut b, hour);
        let end = event_last.min(link_last).min(last);
        let held = match (available(event), available(link)) {
            (false, _) => Held::Unread,
            (true, false) => Held::Unjoined,
            (true, true) => Held::Observed,
        };
        match out.last_mut() {
            Some(span) if span.held == held => span.last = end,
            _ => out.push(Span { first: hour, last: end, held }),
        }
        hour = match end.checked_add(1) {
            Some(next) => next,
            None => break,
        };
    }
    out
}

/// The canonical slot `runs` held at `hour`.
fn slot_at(runs: &read::Runs, hour: i32) -> Option<u32> {
    let at = runs.partition_point(|run| run.0 <= hour).checked_sub(1)?;
    let (_, last, value) = runs[at];
    (hour <= last).then_some(value? as u32)
}

/// The fold of `family`'s log over `first..=last` of `query`'s timeline, stretch by stretch: the
/// hours observed from the two tables' status runs; the events from the log's buckets that hold
/// rows within them, each row once; a raw country named by its canonical slot's holder at the read
/// hour, from the links' runs of the raw ids the events name.
fn scan(source: &Source, query: &Query<'_>, family: MilitaryAirFamily, (first, last): (i32, i32), cancelled: &AtomicBool) -> io::Result<Fold> {
    let log = dashboard_declare::log(family.log()).ok_or_else(|| dashboard_core::invalid(format!("No declared log {}", family.log())))?;
    let columns = Columns::of(log.table, if family == MilitaryAirFamily::AirLosses { &AIR_COLUMNS } else { &BOMBING_COLUMNS })?;
    let links_slot = dashboard_declare::field(TableId::CountryLinks, "canonical_slot")?;
    let holders = slot_holders(query);
    let recording = query.replay.recording();
    let reader = log::Reader::new(query, log)?;
    let mut fold = Fold::default();
    for stretch in query.timeline.stretches.iter().filter(|stretch| stretch.first <= last && stretch.last >= first) {
        crate::metadata::check_cancel(cancelled)?;
        let (from, to) = (stretch.first.max(first), stretch.last.min(last));
        let spans = spans(from, to, &read::statuses(recording, stretch.run, log.table, from, to)?, &read::statuses(recording, stretch.run, TableId::CountryLinks, from, to)?);
        let observed: Vec<(i32, i32)> = spans.iter().filter(|span| span.held == Held::Observed).map(|span| (span.first, span.last)).collect();
        let mut reads: Vec<(i32, Vec<Event>)> = Vec::new();
        for row in reader.inserted(stretch.run, &observed)? {
            let event = event(family, &row, &columns).ok_or_else(|| dashboard_core::invalid(format!("{} holds an event whose cells are not recorded", log.name)))?;
            match reads.last_mut() {
                Some((hour, events)) if *hour == row.hour => events.push(event),
                _ => reads.push((row.hour, vec![event])),
            }
        }
        crate::metadata::check_cancel(cancelled)?;
        let mut raws: BTreeSet<u128> = BTreeSet::new();
        let mut tokens: BTreeSet<u32> = BTreeSet::new();
        for (_, events) in &mut reads {
            events.sort_by_key(|event| match event {
                Event::Air { id, .. } | Event::Bombing { id, .. } => *id,
            });
            for event in events.iter() {
                let (one, other) = match *event {
                    Event::Air { country, opponent, .. } => (Some(country), opponent),
                    Event::Bombing { attacker, victim, building, .. } => {
                        tokens.insert(building);
                        (attacker, Some(victim))
                    }
                };
                raws.extend([one, other].into_iter().flatten().filter(|raw| *raw != 0).map(u128::from));
            }
        }
        let keys: Vec<u128> = raws.into_iter().collect();
        let links = if keys.is_empty() { HashMap::new() } else { read::runs(recording, stretch.run, TableId::CountryLinks, summary::Source::Cell(links_slot), Some(&keys), from, to)? };
        // Spelled at the stretch's last hour whatever the window, as the whole stretch is.
        let spelled = crate::answer::spellings(source, stretch.run, stretch.last, &tokens)?;
        let resolve = |raw: u32, hour: i32| {
            let slot = slot_at(links.get(&u128::from(raw))?, hour)?;
            holders.get(&slot)?.iter().find(|run| run.2 <= hour && hour <= run.3).map(|run| (run.0.clone(), run.1.clone()))
        };
        let mut at = 0;
        for span in spans {
            let end = at + reads[at..].partition_point(|(hour, _)| *hour <= span.last);
            fold.span(family, span, &reads[at..end], resolve, |token| spelled.get(&token).cloned())?;
            at = end;
        }
    }
    Ok(fold)
}

/// Every holder over the timeline, its hours cut to the observed ones.
fn countries(query: &Query<'_>, observed: &[HourSpan]) -> BTreeMap<Named, Vec<HourSpan>> {
    let mut out: BTreeMap<Named, Vec<HourSpan>> = BTreeMap::new();
    for (tag, runs) in &query.holders.tags {
        for run in runs {
            let Holder::Found { slot, id } = run.holder else { continue };
            for span in observed {
                let (first, last) = (span.first_hour.max(run.first), span.last_hour.min(run.last));
                if first <= last {
                    out.entry((tag.clone(), format!("{slot}:{id}"))).or_default().push(HourSpan { first_hour: first, last_hour: last });
                }
            }
        }
    }
    for spans in out.values_mut() {
        spans.sort_by_key(|span| span.first_hour);
        let mut joined: Vec<HourSpan> = Vec::with_capacity(spans.len());
        for span in spans.drain(..) {
            match joined.last_mut() {
                Some(last) if last.last_hour.checked_add(1) == Some(span.first_hour) => last.last_hour = span.last_hour,
                _ => joined.push(span),
            }
        }
        *spans = joined;
    }
    out
}

/// A leaf's points after `from` (by game hour): (game hour, read hour, total), in read order.
type Points = Vec<(i32, i32, i128)>;

fn points(deltas: &BTreeMap<(i32, i32), i128>, from: Option<i32>) -> io::Result<Points> {
    let mut total = 0_i128;
    deltas
        .iter()
        .filter(|((_, hour), _)| from.is_none_or(|from| *hour >= from))
        .map(|(&(read_hour, hour), &amount)| {
            total = total.checked_add(amount).ok_or_else(|| dashboard_core::invalid("military air prefix sum overflow"))?;
            Ok((hour, read_hour, total))
        })
        .collect()
}

/// The leaves a reply keeps: `params`' regions and causes, their points from its first hour.
fn selected<'f>(fold: &'f Fold, params: &MilitaryAirParams) -> io::Result<Vec<(&'f LeafKey, Points)>> {
    fold.leaves
        .iter()
        .filter(|(key, _)| (params.regions.is_empty() || key.region.is_some_and(|region| params.regions.contains(&region))) && (params.causes.is_empty() || params.causes.contains(&key.cause)))
        .map(|(key, deltas)| Ok((key, points(deltas, params.from_hour)?)))
        .collect()
}

/// `leaves` regrouped as the parts a reply shows: each part's points are its leaves' increments
/// summed at each (read, game) hour and run on, so every total read from them is the sum of its
/// leaves'.
fn parts(leaves: &[(&LeafKey, Points)], family: MilitaryAirFamily) -> io::Result<Vec<MilitaryAirPart>> {
    let air = family == MilitaryAirFamily::AirLosses;
    let mut grouped = BTreeMap::<MilitaryAirPart, BTreeMap<(i32, i32), i128>>::new();
    for (leaf, points) in leaves {
        let part = MilitaryAirPart {
            tag: leaf.country.as_ref().map(|country| country.0.clone()),
            identity: leaf.country.as_ref().map(|country| country.1.clone()),
            dealt: leaf.dealt,
            cause: Some(leaf.cause),
            opponent_known: air.then_some(leaf.opponent.is_some()),
            building_token: if air { None } else { leaf.building.as_ref().and_then(|building| building.1.clone()) },
            building_id: if air { None } else { leaf.building.as_ref().map(|building| building.0) },
            state: if air { None } else { leaf.state },
            points: Vec::new(),
        };
        let deltas = grouped.entry(part).or_default();
        let mut before = 0_i128;
        for &(hour, read_hour, total) in points {
            let delta = deltas.entry((read_hour, hour)).or_default();
            *delta = total.checked_sub(before).and_then(|step| delta.checked_add(step)).ok_or_else(|| dashboard_core::invalid("military air prefix sum overflow"))?;
            before = total;
        }
    }
    grouped
        .into_iter()
        .map(|(mut part, deltas)| {
            let mut total = 0_i128;
            part.points = deltas
                .into_iter()
                .map(|((read, hour), delta)| {
                    total = total.checked_add(delta).ok_or_else(|| dashboard_core::invalid("military air prefix sum overflow"))?;
                    Ok((hour, read, total.to_string()))
                })
                .collect::<io::Result<_>>()?;
            Ok(part)
        })
        .collect()
}

fn named(id: String, result: Result<String, String>) -> MilitaryAirName {
    match result {
        Ok(name) => MilitaryAirName { id, name: Some(name), missing: None },
        Err(reason) => MilitaryAirName { id, name: None, missing: Some(reason) },
    }
}

/// The names of every region and building the family's events name, in the reader's language.
fn names(source: &Source, context: &LoadContext, fold: &Fold, warnings: &mut Vec<String>) -> (Vec<MilitaryAirName>, Vec<MilitaryAirName>) {
    let regions: BTreeSet<u32> = fold.leaves.keys().filter_map(|key| key.region).collect();
    let buildings: BTreeSet<String> = fold.leaves.keys().filter_map(|key| key.building.as_ref().and_then(|building| building.1.clone())).collect();
    let cat = context.content().and_then(|options| catalogue::catalogue(source, options));
    let Ok(cat) = cat else {
        let reason = "Recorded content stack unavailable; choose matching game and user folders".to_owned();
        warnings.push(reason.clone());
        return (
            regions.into_iter().map(|id| named(id.to_string(), Err(reason.clone()))).collect(),
            buildings.into_iter().map(|id| named(id, Err(reason.clone()))).collect(),
        );
    };
    warnings.extend(cat.verification.iter().cloned());
    warnings.extend(cat.warnings.iter().cloned());
    let regions = if regions.is_empty() {
        Vec::new()
    } else {
        let content = dashboard_base::content::strategic_regions(&cat.content.layers).map_err(|error| error.to_string());
        if let Ok(content) = &content {
            warnings.extend(content.notes.iter().cloned());
        }
        regions.into_iter().map(|id| named(id.to_string(), content.as_ref().map_err(Clone::clone).and_then(|content| region_name(&cat, content, id)))).collect()
    };
    let buildings = if buildings.is_empty() {
        Vec::new()
    } else {
        let content = cat.content.layers.derived::<catalogue::CachedResult<installed_content_core::buildings::Buildings>>().get_or_init(|| installed_content_core::buildings::read(&cat.content.layers)).map_err(|error| error.to_string());
        buildings
            .into_iter()
            .map(|id| {
                let name = content.as_ref().map_err(Clone::clone).and_then(|content| building_name(&cat, content, &id));
                named(id, name)
            })
            .collect()
    };
    (regions, buildings)
}

pub(crate) fn region_name(cat: &catalogue::Catalogue, content: &installed_content_regions::regions::StrategicRegions, id: u32) -> Result<String, String> {
    let region = content.regions.get(&id).ok_or_else(|| format!("Region {id} absent from the recorded stack"))?;
    cat.localised(&region.name_key, catalogue::View::Plain).ok_or_else(|| format!("Region {id} name unsupported"))
}

fn building_name(cat: &catalogue::Catalogue, content: &installed_content_core::buildings::Buildings, token: &str) -> Result<String, String> {
    let building = content.by_token(token).map_err(|error| error.to_string())?.ok_or_else(|| format!("Building {token} absent from the recorded stack"))?;
    cat.localised(&building.name, catalogue::View::Plain).ok_or_else(|| format!("Building {token} name unsupported"))
}

fn included(dealt: bool, top: &MilitaryAirTop) -> bool {
    match top.metric {
        MilitaryAirMetric::Dealt => dealt,
        MilitaryAirMetric::Taken => !dealt,
        MilitaryAirMetric::Total => true,
    }
}

/// Each holder's increments by read hour from `from`, from the leaves.
fn leaf_deltas<'l>(leaves: &'l [(&LeafKey, Points)], from: i32, top: &MilitaryAirTop) -> io::Result<BTreeMap<&'l str, BTreeMap<i32, i128>>> {
    let mut deltas: BTreeMap<&str, BTreeMap<i32, i128>> = BTreeMap::new();
    for (leaf, points) in leaves {
        let Some((_, identity)) = leaf.country.as_ref() else { continue };
        if !included(leaf.dealt, top) {
            continue;
        }
        let mut before = 0_i128;
        for &(_, read_hour, total) in points {
            let delta = total.checked_sub(before).ok_or_else(|| dashboard_core::invalid("military air ranking delta overflow"))?;
            before = total;
            if read_hour < from {
                continue;
            }
            let value = deltas.entry(identity.as_str()).or_default().entry(read_hour).or_default();
            *value = value.checked_add(delta).ok_or_else(|| dashboard_core::invalid("military air ranking sum overflow"))?;
        }
    }
    Ok(deltas)
}

struct RankRun {
    span: HourSpan,
    value: i128,
}

/// Each tag's hours in the top `top.count` by its holder's total since `from` (its increments
/// `deltas` by read hour, a total carried in before the hours at `i32::MIN`), exact to the hour. A
/// holder ranks only while its total is above zero: a top N is never filled with zeros in tag order.
fn ranked(countries: &BTreeMap<Named, Vec<HourSpan>>, mut deltas: BTreeMap<&str, BTreeMap<i32, i128>>, from: i32, top: &MilitaryAirTop, cancelled: &AtomicBool) -> io::Result<BTreeMap<String, Vec<HourSpan>>> {
    for values in deltas.values_mut() {
        let mut total = 0_i128;
        for value in values.values_mut() {
            total = total.checked_add(*value).ok_or_else(|| dashboard_core::invalid("military air ranking prefix overflow"))?;
            *value = total;
        }
    }
    let mut series: BTreeMap<String, Vec<RankRun>> = BTreeMap::new();
    for ((tag, identity), spans) in countries {
        crate::metadata::check_cancel(cancelled)?;
        let values = deltas.get(identity.as_str());
        for span in spans {
            let first = span.first_hour.max(from);
            if first > span.last_hour {
                continue;
            }
            let mut changes = vec![(first, values.and_then(|values| values.range(..=first).next_back()).map_or(0, |(_, value)| *value))];
            if let Some(values) = values {
                changes.extend(values.range((std::ops::Bound::Excluded(first), std::ops::Bound::Included(span.last_hour))).map(|(&hour, &value)| (hour, value)));
            }
            for (index, &(first_hour, value)) in changes.iter().enumerate() {
                if value <= 0 {
                    continue;
                }
                let last_hour = changes.get(index + 1).map_or(span.last_hour, |(hour, _)| hour - 1);
                series.entry(tag.clone()).or_default().push(RankRun { span: HourSpan { first_hour, last_hour }, value });
            }
        }
    }
    for runs in series.values_mut() {
        runs.sort_by_key(|run| run.span.first_hour);
    }
    ranking::top_membership_exact(&series, top.count, |run| (run.span.first_hour, run.span.last_hour), |run| Some(run.value), true, cancelled)
}

/// `countries` as a reply names them, each with its hours in `membership` when a top N is asked.
fn ranked_countries(countries: &BTreeMap<Named, Vec<HourSpan>>, membership: Option<&BTreeMap<String, Vec<HourSpan>>>, cancelled: &AtomicBool) -> io::Result<Vec<MilitaryAirCountry>> {
    countries
        .iter()
        .map(|((tag, identity), present)| {
            let selected = membership
                .map(|membership| {
                    let mut selected = Vec::new();
                    ranking::for_each_piece(present, membership.get(tag).map_or(&[], Vec::as_slice), |span| (span.first_hour, span.last_hour), cancelled, |_, first_hour, last_hour| {
                        selected.push(HourSpan { first_hour, last_hour });
                        Ok(())
                    })?;
                    Ok::<_, io::Error>(selected)
                })
                .transpose()?;
            Ok(MilitaryAirCountry { tag: tag.clone(), identity: identity.clone(), present: present.clone(), membership: selected })
        })
        .collect()
}

/// What a window's openings are keyed by: the holder's identity (none: unattributed), the side, the
/// cause and, of air losses, whether the opponents were recorded.
pub(crate) type OpeningKey = (Option<String>, bool, u32, Option<bool>);
pub(crate) type Openings = BTreeMap<OpeningKey, i128>;

fn add_opening(out: &mut Openings, key: OpeningKey, amount: i128) -> io::Result<()> {
    let total = out.entry(key).or_default();
    *total = total.checked_add(amount).ok_or_else(|| dashboard_core::invalid("military air opening overflow"))?;
    Ok(())
}

/// The fold's selected leaves' totals as openings.
fn folded_openings(fold: &Fold, params: &MilitaryAirParams, out: &mut Openings) -> io::Result<()> {
    let air = params.family == MilitaryAirFamily::AirLosses;
    for (leaf, points) in selected(fold, params)? {
        let Some(&(_, _, total)) = points.last() else { continue };
        add_opening(out, (leaf.country.as_ref().map(|country| country.1.clone()), leaf.dealt, leaf.cause, air.then_some(leaf.opponent.is_some())), total)?;
    }
    Ok(())
}

fn holder_identity(slot: u32, id: u32) -> Option<String> {
    (slot != AirLossTotalRow::NONE).then(|| format!("{slot}:{id}"))
}

const _: () = assert!(AirLossTotalRow::NONE == BombingTotalRow::NONE);

/// The family's running totals of run `run` after `hour`, as openings under `params`' regions and
/// causes, and the most hours late a counted event was read; none where the table cannot say (not
/// complete, a total not recorded).
fn table_totals(recording: &Recording, run: u32, hour: i32, params: &MilitaryAirParams) -> io::Result<Option<(Openings, i32)>> {
    let air = params.family == MilitaryAirFamily::AirLosses;
    let table = if air { TableId::AirLossTotals } else { TableId::BombingTotals };
    let snapshot = read::at(recording, run, &[read::Read::all(table)], hour)?.pop().ok_or_else(|| dashboard_core::invalid("A table read returned nothing"))?;
    if snapshot.status != Status::Complete {
        return Ok(None);
    }
    let view = snapshot.view();
    let wanted = |region: Option<u32>, cause: u32| (params.regions.is_empty() || region.is_some_and(|region| params.regions.contains(&region))) && (params.causes.is_empty() || params.causes.contains(&cause));
    let (mut out, mut late) = (Openings::new(), 0_i32);
    for &(key, slot) in &snapshot.rows {
        let mut cells: Vec<(OpeningKey, usize)> = Vec::new();
        let width = if air {
            let (holder, id, region, cause) = ((key >> 96) as u32, (key >> 64) as u32, (key >> 32) as u32, key as u32);
            if wanted(Some(region), cause) {
                let identity = holder_identity(holder, id);
                for (column, (dealt, known)) in [(true, true), (true, false), (false, true), (false, false)].into_iter().enumerate() {
                    cells.push(((identity.clone(), dealt, cause, Some(known)), column));
                }
            }
            4
        } else {
            let (holder, id, cause) = ((key >> 40) as u32, (key >> 8) as u32, u32::from(key as u8));
            if wanted(None, cause) {
                let identity = holder_identity(holder, id);
                cells.extend([((identity.clone(), true, cause, None), 0), ((identity, false, cause, None), 1)]);
            }
            2
        };
        late = late.max(view.number(width, slot).unwrap_or(0) as i32);
        for (opening, column) in cells {
            let Some(total) = view.number(column, slot) else { return Ok(None) };
            if total != 0 {
                add_opening(&mut out, opening, i128::from(total))?;
            }
        }
    }
    Ok(Some((out, late)))
}

/// Each part's total from the first hour counted (`params`' range's, else the timeline's) through
/// the hour before `before`: per stretch, the recorder's running totals at its last hour less those
/// before its first; the reads after a range's first hour that may hold events dated before it
/// (within the most hours late any counted event was read) folded instead, and a stretch the tables
/// cannot say for folded whole. `tables` false folds every stretch, for proof.
pub(crate) fn openings(source: &Source, query: &Query<'_>, params: &MilitaryAirParams, before: i32, tables: bool, cancelled: &AtomicBool) -> io::Result<Openings> {
    let mut out = Openings::new();
    let (start, end) = (params.from_hour.unwrap_or(query.timeline.first), before.saturating_sub(1));
    if start > end {
        return Ok(out);
    }
    let recording = query.replay.recording();
    let stretches: Vec<_> = query.timeline.within(start, end).collect();
    for stretch in stretches {
        crate::metadata::check_cancel(cancelled)?;
        let (a, b) = (stretch.first, stretch.last);
        let run_first = recording.run(stretch.run)?.first_hour;
        let mut folded_through = b;
        if let Some((at_end, late)) = if tables { table_totals(recording, stretch.run, b, params)? } else { None } {
            let through = params.from_hour.map_or(a - 1, |from| from.saturating_add(late).saturating_sub(1).clamp(a - 1, b));
            let at_start = if through < run_first { Some((Openings::new(), 0)) } else { table_totals(recording, stretch.run, through, params)? };
            if let Some((at_start, _)) = at_start {
                for (key, total) in at_end {
                    add_opening(&mut out, key, total)?;
                }
                for (key, total) in at_start {
                    add_opening(&mut out, key, -total)?;
                }
                folded_through = through;
            }
        }
        if folded_through >= a {
            folded_openings(&scan(source, query, params.family, (a, folded_through), cancelled)?, params, &mut out)?;
        }
    }
    out.retain(|_, total| *total != 0);
    Ok(out)
}

/// Each run's observed stretches of each family's log, behind a slot held while they are read: a
/// request asking during the read waits for it instead of reading the status runs again.
#[derive(Default)]
struct Observed(HashMap<(u32, &'static str), Arc<Mutex<Option<Arc<Vec<(i32, i32)>>>>>>);

/// Forgets every run's observed stretches: the next window reads them cold.
pub(crate) fn forget(source: &Source) {
    *source.state::<Observed>().lock().unwrap_or_else(PoisonError::into_inner) = Observed::default();
}

/// The hours of run `run` the family's log and the links were both available, ascending and
/// joined: read once over the whole run from the two tables' status runs, then kept on `source`.
fn observed(source: &Source, query: &Query<'_>, family: MilitaryAirFamily, run: u32) -> io::Result<Arc<Vec<(i32, i32)>>> {
    let slot = Arc::clone(source.state::<Observed>().lock().unwrap_or_else(PoisonError::into_inner).0.entry((run, family.log())).or_default());
    let mut held = slot.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(found) = held.as_ref() {
        return Ok(Arc::clone(found));
    }
    let log = dashboard_declare::log(family.log()).ok_or_else(|| dashboard_core::invalid(format!("No declared log {}", family.log())))?;
    let recording = query.replay.recording();
    let directory = recording.run(run)?;
    let first = directory.first_hour;
    let last = first.saturating_add(i32::try_from(directory.ordinals).unwrap_or(i32::MAX)).saturating_sub(1);
    let made: Vec<(i32, i32)> = if last < first {
        Vec::new()
    } else {
        spans(first, last, &read::statuses(recording, run, log.table, first, last)?, &read::statuses(recording, run, TableId::CountryLinks, first, last)?)
            .into_iter()
            .filter(|span| span.held == Held::Observed)
            .map(|span| (span.first, span.last))
            .collect()
    };
    let made = Arc::new(made);
    *held = Some(Arc::clone(&made));
    Ok(made)
}

/// Keeps every run of `branch`'s timeline's observed stretches of both families on `source`: the
/// open warms them on a thread of its own, so a late window's reach back is a lookup.
pub(crate) fn warm_observed(source: &Source, branch: u64) -> io::Result<()> {
    let query = crate::answer::query(source, branch)?;
    let mut runs: Vec<u32> = Vec::new();
    for stretch in query.timeline.stretches.iter().rev() {
        if !runs.contains(&stretch.run) {
            runs.push(stretch.run);
        }
    }
    for family in [MilitaryAirFamily::AirLosses, MilitaryAirFamily::Bombing] {
        for &run in &runs {
            observed(source, &query, family, run)?;
        }
    }
    Ok(())
}

/// The runs and families whose observed stretches are kept on `source`, ascending.
#[cfg(test)]
pub(crate) fn kept_observed(source: &Source) -> Vec<(u32, &'static str, Vec<(i32, i32)>)> {
    let index = source.state::<Observed>();
    let index = index.lock().unwrap_or_else(PoisonError::into_inner);
    let mut kept: Vec<(u32, &'static str, Vec<(i32, i32)>)> =
        index.0.iter().filter_map(|(&(run, log), slot)| Some((run, log, slot.lock().unwrap_or_else(PoisonError::into_inner).as_deref()?.clone()))).collect();
    kept.sort_unstable();
    kept
}

/// The first hour of `first..=last` the family's log and the links were both available, looked up
/// in each run's observed stretches.
fn first_observed(source: &Source, query: &Query<'_>, family: MilitaryAirFamily, (first, last): (i32, i32)) -> io::Result<Option<i32>> {
    for stretch in query.timeline.within(first, last) {
        let spans = observed(source, query, family, stretch.run)?;
        let at = spans.partition_point(|&(_, end)| end < stretch.first);
        if let Some(&(start, _)) = spans.get(at).filter(|&&(start, _)| start <= stretch.last) {
            return Ok(Some(start.max(stretch.first)));
        }
    }
    Ok(None)
}

/// The first hour of the observed hours running unbroken up to `at`, an observed hour, looked for
/// no further back than `floor`: each run's observed stretch holding the hour before looked up,
/// across stretches that meet.
fn observed_since(source: &Source, query: &Query<'_>, family: MilitaryAirFamily, at: i32, floor: i32) -> io::Result<i32> {
    let mut since = at;
    let stretches: Vec<_> = query.timeline.within(floor, at.saturating_sub(1)).collect();
    for stretch in stretches.iter().rev() {
        if stretch.last.checked_add(1) != Some(since) {
            break;
        }
        let spans = observed(source, query, family, stretch.run)?;
        let held = spans.partition_point(|&(_, end)| end < stretch.last);
        let Some(&(start, _)) = spans.get(held).filter(|&&(start, _)| start <= stretch.last) else { return Ok(since) };
        since = start.max(stretch.first);
        if since > stretch.first {
            break;
        }
    }
    Ok(since)
}

/// The latest read of `from..=to` holding a bombing caller whose membership in aircraft bombing is
/// unknown, from the log's counts by cause: the last of about eight coarse bins holding one, then
/// the last six-hour bucket of it, read row by row.
fn latest_incomplete(query: &Query<'_>, (from, to): (i32, i32)) -> io::Result<Option<i32>> {
    let log = dashboard_declare::log(MilitaryAirFamily::Bombing.log()).ok_or_else(|| dashboard_core::invalid("No declared bombing log"))?;
    let reader = log::Reader::new(query, log)?;
    let unknown = log::FacetFilter { name: "cause".into(), values: vec![BuildingBombingDamageCause::Unknown as i64], tags: Vec::new() };
    let patterns = reader.select(&[unknown], from, to)?.patterns;
    let last_bin = |(from, to): (i32, i32), bin: i64| -> io::Result<Option<(i32, i32)>> {
        let latest = reader.bins(&patterns, from, to, bin)?.into_iter().filter_map(|bins| bins.last().map(|&(start, _)| start)).max();
        Ok(latest.map(|start| (i32::try_from(start).unwrap_or(i32::MIN).max(from), i32::try_from(start + bin - 1).unwrap_or(i32::MAX).min(to))))
    };
    let coarse = log::bin_for(i64::from(to) - i64::from(from) + 1, 8);
    let Some(within) = last_bin((from, to), coarse)? else { return Ok(None) };
    let Some(bucket) = last_bin(within, summary::bucket_hours(0))? else { return Ok(None) };
    let mut latest = None;
    for run in patterns.iter().map(|pattern| pattern.run).collect::<BTreeSet<_>>() {
        latest = latest.max(reader.rows(run, &patterns, &[bucket])?.into_iter().map(|(_, row)| row.hour).max());
    }
    Ok(latest)
}

/// `fold`'s observed hours over a window from `first` reaching back, where they run on from before
/// it, to where that observation began (never before `counted`, the first hour counted), and of
/// bombing the latest read in that reach holding a caller of unknown membership: so the window
/// alone says whether its holders' totals since the first hour counted are complete, and where
/// each holder's hours began.
fn reach_back(source: &Source, query: &Query<'_>, family: MilitaryAirFamily, fold: &mut Fold, first: i32, counted: Option<i32>) -> io::Result<()> {
    let (Some(counted), Some(span)) = (counted, fold.observed.first()) else { return Ok(()) };
    if span.first_hour != first || counted >= first {
        return Ok(());
    }
    let since = observed_since(source, query, family, first, counted)?;
    if since == first {
        return Ok(());
    }
    fold.observed[0].first_hour = since;
    if family == MilitaryAirFamily::Bombing
        && let Some(read) = latest_incomplete(query, (since, first - 1))?
    {
        fold.incomplete.insert(0, HourSpan { first_hour: read, last_hour: read });
        if fold.incomplete.get(1).is_some_and(|next| next.first_hour == read + 1) {
            let next = fold.incomplete.remove(1);
            fold.incomplete[0].last_hour = next.last_hour;
        }
    }
    Ok(())
}

/// `params.window`'s reply from `fold`, over `first..=last`, whose holders are `countries` and its
/// parts' totals before it `openings`; its holders ranked by `params`' top.
pub(crate) fn reply(fold: &Fold, (first, last): (i32, i32), countries: &BTreeMap<Named, Vec<HourSpan>>, params: &MilitaryAirParams, openings: &Openings, warnings: Vec<String>, cancelled: &AtomicBool) -> io::Result<MilitaryAirReply> {
    let leaves = selected(fold, params)?;
    let from = params.from_hour.unwrap_or_else(|| fold.observed.first().map_or(first, |span| span.first_hour)).max(first);
    let membership = params
        .top
        .as_ref()
        .map(|top| {
            let mut deltas = leaf_deltas(&leaves, from, top)?;
            for ((identity, dealt, ..), total) in openings {
                if let Some(identity) = identity.as_deref().filter(|_| included(*dealt, top)) {
                    let carried = deltas.entry(identity).or_default().entry(i32::MIN).or_default();
                    *carried = carried.checked_add(*total).ok_or_else(|| dashboard_core::invalid("military air ranking sum overflow"))?;
                }
            }
            ranked(countries, deltas, from, top, cancelled)
        })
        .transpose()?;
    Ok(MilitaryAirReply {
        family: params.family,
        window: params.window,
        first_hour: first,
        last_hour: last,
        counted_from: params.from_hour,
        available: !fold.observed.is_empty(),
        observed: fold.observed.clone(),
        incomplete: fold.incomplete.clone(),
        countries: ranked_countries(countries, membership.as_ref(), cancelled)?,
        openings: openings
            .iter()
            .map(|((identity, dealt, cause, opponent_known), total)| MilitaryAirOpening { identity: identity.clone(), dealt: *dealt, cause: *cause, opponent_known: *opponent_known, total: total.to_string() })
            .collect(),
        parts: parts(&leaves, params.family)?,
        regions: Vec::new(),
        buildings: Vec::new(),
        notes: fold.warnings.iter().map(|(reason, events)| MilitaryAirNote { reason: reason.clone(), events: *events }).collect(),
        warnings,
    })
}

/// `params.family`'s ledger over `first..=last`, a window of the timeline: its observed hours, and
/// so its holders' hours, reaching back to where they began since the first hour counted.
pub(crate) fn window_reply(source: &Source, query: &Query<'_>, params: &MilitaryAirParams, (first, last): (i32, i32), context: &LoadContext, cancelled: &AtomicBool) -> io::Result<MilitaryAirReply> {
    let mut fold = if first <= last { scan(source, query, params.family, (first, last), cancelled)? } else { Fold::default() };
    let counted_from = match params.from_hour {
        Some(from) => Some(from),
        None if first <= last => first_observed(source, query, params.family, (query.timeline.first, last))?,
        None => None,
    };
    reach_back(source, query, params.family, &mut fold, first, counted_from)?;
    let countries = countries(query, &fold.observed);
    let opened = if first <= last { openings(source, query, params, first, true, cancelled)? } else { Openings::new() };
    let mut warnings = Vec::new();
    let (regions, buildings) = names(source, context, &fold, &mut warnings);
    let mut reply = reply(&fold, (first, last), &countries, params, &opened, warnings, cancelled)?;
    reply.counted_from = counted_from;
    reply.regions = regions;
    reply.buildings = buildings;
    Ok(reply)
}

impl Product for MilitaryAirLedger {
    const KEY: &'static str = "military-air-ledger";
    const CONTENT_DEPENDENT: bool = true;
    type Params = MilitaryAirParams;
    type Reply = MilitaryAirReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        super::windows::hours(params.window).ok_or("Invalid window")?;
        if params.top.as_ref().is_some_and(|top| top.count == 0) {
            return Err("Top count must be positive");
        }
        Ok(())
    }

    fn types() -> Vec<String> {
        vec![
            MilitaryAirParams::decl(),
            MilitaryAirFamily::decl(),
            MilitaryAirMetric::decl(),
            MilitaryAirTop::decl(),
            MilitaryAirCountry::decl(),
            MilitaryAirOpening::decl(),
            MilitaryAirPart::decl(),
            MilitaryAirName::decl(),
            MilitaryAirNote::decl(),
            MilitaryAirReply::decl(),
        ]
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        crate::metadata::check_cancel(cancelled)?;
        let branch = params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let query = crate::answer::query(source, branch)?;
        let reply = window_reply(source, &query, params, super::windows::within(&query, params.window)?, context, cancelled)?;
        dashboard_base::products::check_reply_budget(&reply).map_err(|_| dashboard_core::invalid("Military air ledger reply exceeds transport budget; nothing was truncated"))?;
        Ok(reply)
    }
}
