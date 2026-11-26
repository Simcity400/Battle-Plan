
use dashboard_world::products::region_names::region_name;
use dashboard_base::products::{LoadContext, Product};
use dashboard_base::{Source, catalogue, ranking, span::HourSpan};
use dashboard_query::{
    Query,
    identity::Holder,
    log::{self, Cell, Row},
    read,
};
use replay_recording::columnar_frames::{Recording, available, complete};
use replay_lanes::lanes::StatusRun;
use replay_columnar::{manifest::Status, summary};
use replay_data::rows::{AirLossTotalRow, BombingTotalRow, BuildingBombingDamageCause, TableId};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, Mutex, PoisonError, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub mod at;
pub mod breakdown;
pub mod tiles;
#[cfg(test)]
mod reference;
#[cfg(test)]
mod tests;

pub struct MilitaryAirLedger;

pub const AIR_LOSSES: &str = "air-losses";
pub const BOMBING: &str = "bombing-damage";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
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

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum MilitaryAirMetric {
    Dealt,
    Taken,
    Total,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MilitaryAirTop {
    pub count: usize,
    pub metric: MilitaryAirMetric,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MilitaryAirParams {
    pub branch_id: String,
    pub family: MilitaryAirFamily,
    #[serde(default)]
    pub regions: Vec<u32>,
    #[serde(default)]
    pub causes: Vec<u32>,
    #[doc = " A range starts at the events' own game hour."]
    #[serde(default)]
    pub from_hour: Option<i32>,
    pub window: u32,
    #[serde(default)]
    pub top: Option<MilitaryAirTop>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MilitaryAirCountry {
    pub tag: String,
    #[doc = " Canonical slot and raw country id of the holder; tag changes do not join holders."]
    pub identity: String,
    pub present: Vec<HourSpan>,
    #[doc = " Null means no ranking filter. Empty means never in the requested top N within the window."]
    pub membership: Option<Vec<HourSpan>>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MilitaryAirOpening {
    pub identity: Option<String>,
    pub dealt: bool,
    pub cause: u32,
    pub opponent_known: Option<bool>,
    pub total: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MilitaryAirPart {
    pub tag: Option<String>,
    pub identity: Option<String>,
    pub dealt: bool,
    pub cause: Option<u32>,
    #[doc = " Air losses: whether the opposite endpoint was resolved for every loss in it."]
    pub opponent_known: Option<bool>,
    pub building_token: Option<String>,
    pub building_id: Option<u32>,
    pub state: Option<u32>,
    #[doc = " Each point is `[hour, readHour, totalRaw]`: the event's game hour, the hour its drain was"]
    #[doc = " read and the exact total through it since the window's first hour, in read order."]
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "Array<[number, number, string]>"))]
    pub points: Vec<(i32, i32, String)>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MilitaryAirName {
    pub id: String,
    pub name: Option<String>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MilitaryAirCauseName {
    pub cause: u32,
    pub name: Option<String>,
    #[doc = " Why the cause has no name: its recorded id has no proved display meaning."]
    pub missing: Option<String>,
}

const AIR_LOSS_CAUSES: [&str; 6] = ["Aircraft combat", "Anti-air / raid effect", "Air accidents", "Airbase bombing", "Train anti-air", "Kamikaze anti-air"];

pub fn air_loss_cause(cause: u32) -> MilitaryAirCauseName {
    match cause.checked_sub(1).and_then(|at| AIR_LOSS_CAUSES.get(at as usize)) {
        Some(name) => MilitaryAirCauseName { cause, name: Some((*name).to_owned()), missing: None },
        None => MilitaryAirCauseName { cause, name: None, missing: Some(format!("Air loss cause {cause} has no proved display meaning")) },
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MilitaryAirNote {
    pub reason: String,
    pub events: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MilitaryAirReply {
    pub family: MilitaryAirFamily,
    pub window: u32,
    pub first_hour: i32,
    pub last_hour: i32,
    #[doc = " The hour the totals count from: the range's first, else the timeline's first observed;"]
    #[doc = " none while no hour through the window's last is observed."]
    pub counted_from: Option<i32>,
    pub available: bool,
    pub observed: Vec<HourSpan>,
    #[doc = " Reads that may miss events: read only in part, or holding a caller whose membership in aircraft bombing is unknown."]
    pub incomplete: Vec<HourSpan>,
    #[doc = " Hours whose events were read but whose country links were not, so no event can be credited to a country."]
    pub unjoined: Vec<HourSpan>,
    #[doc = " Air losses: every cause the window's events name, with its name or why it has none."]
    pub causes: Vec<MilitaryAirCauseName>,
    pub countries: Vec<MilitaryAirCountry>,
    pub openings: Vec<MilitaryAirOpening>,
    pub parts: Vec<MilitaryAirPart>,
    pub regions: Vec<MilitaryAirName>,
    pub buildings: Vec<MilitaryAirName>,
    pub notes: Vec<MilitaryAirNote>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(MilitaryAirReply);

type Named = (String, String);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct LeafKey {
    country: Option<Named>,
    dealt: bool,
    opponent: Option<String>,
    region: Option<u32>,
    building: Option<(u32, Option<String>)>,
    state: Option<u32>,
    cause: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    Air { id: u64, game_hour: i32, country: u32, region: u32, cause: u32, credited_enemy: bool, amount: i64, opponent: Option<u32> },
    Bombing { id: u64, game_hour: i32, attacker: Option<u32>, victim: u32, state: u32, building: u32, cause: BuildingBombingDamageCause, amount: i64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Held {
    Unread,
    Unjoined,
    InPart,
    Observed,
}

impl Held {
    fn read(self) -> bool {
        matches!(self, Self::InPart | Self::Observed)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    pub first: i32,
    pub last: i32,
    pub held: Held,
}

#[derive(Default)]
pub(crate) struct Fold {
    observed: Vec<HourSpan>,
    incomplete: Vec<HourSpan>,
    unjoined: Vec<HourSpan>,
    leaves: BTreeMap<LeafKey, BTreeMap<(i32, i32), i128>>,
    warnings: BTreeMap<String, usize>,
    last_event: Option<u64>,
    unread_since: Option<i32>,
}

fn append_hours(spans: &mut Vec<HourSpan>, first: i32, last: i32) {
    match spans.last_mut() {
        Some(held) if held.last_hour.saturating_add(1) >= first => held.last_hour = held.last_hour.max(last),
        _ => spans.push(HourSpan { first_hour: first, last_hour: last }),
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

    pub(crate) fn span(&mut self, family: MilitaryAirFamily, span: Span, reads: &[(i32, Vec<Event>)], resolve: impl Fn(u32, i32) -> Option<Named>, spell: impl Fn(u32) -> Option<String>) -> io::Result<()> {
        match span.held {
            Held::Unread => {
                self.unread_since.get_or_insert(span.first);
                return Ok(());
            }
            Held::Unjoined => {
                append_hours(&mut self.unjoined, span.first, span.last);
                return Ok(());
            }
            Held::InPart => append_hours(&mut self.incomplete, span.first, span.last),
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
                    append_hours(&mut self.incomplete, hour, hour);
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

fn optional(row: &Row, column: usize) -> Option<Option<u32>> {
    match row.cells.get(column)? {
        Cell::Missing => Some(None),
        Cell::Number(value) => Some(Some(*value as u32)),
        _ => None,
    }
}

struct Columns(Vec<usize>);

impl Columns {
    fn of(table: TableId, names: &[&str]) -> io::Result<Self> {
        names.iter().map(|name| dashboard_declare::field(table, name)).collect::<io::Result<Vec<_>>>().map(Self)
    }
}

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
            (true, true) if !complete(event) || !complete(link) => Held::InPart,
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

fn slot_at(runs: &read::Runs, hour: i32) -> Option<u32> {
    let at = runs.partition_point(|run| run.0 <= hour).checked_sub(1)?;
    let (_, last, value) = runs[at];
    (hour <= last).then_some(value? as u32)
}

fn scan(source: &Source, query: &Query<'_>, family: MilitaryAirFamily, (first, last): (i32, i32), cancelled: &AtomicBool) -> io::Result<Fold> {
    let log = dashboard_declare::log(family.log()).ok_or_else(|| dashboard_core::invalid(format!("No declared log {}", family.log())))?;
    let columns = Columns::of(log.table, if family == MilitaryAirFamily::AirLosses { &AIR_COLUMNS } else { &BOMBING_COLUMNS })?;
    let links_slot = dashboard_declare::field(TableId::CountryLinks, "canonical_slot")?;
    let holders = slot_holders(query);
    let recording = query.replay.recording();
    let reader = log::Reader::new(query, log)?;
    let mut fold = Fold::default();
    for stretch in query.timeline.stretches.iter().filter(|stretch| stretch.first <= last && stretch.last >= first) {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let (from, to) = (stretch.first.max(first), stretch.last.min(last));
        let spans = spans(from, to, &read::statuses(recording, stretch.run, log.table, from, to)?, &read::statuses(recording, stretch.run, TableId::CountryLinks, from, to)?);
        let observed: Vec<(i32, i32)> = spans.iter().filter(|span| span.held.read()).map(|span| (span.first, span.last)).collect();
        let mut reads: Vec<(i32, Vec<Event>)> = Vec::new();
        for row in reader.inserted(stretch.run, &observed)? {
            let event = event(family, &row, &columns).ok_or_else(|| dashboard_core::invalid(format!("{} holds an event whose cells are not recorded", log.name)))?;
            match reads.last_mut() {
                Some((hour, events)) if *hour == row.hour => events.push(event),
                _ => reads.push((row.hour, vec![event])),
            }
        }
        dashboard_text::metadata::check_cancel(cancelled)?;
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
        let spelled = dashboard_base::queries::spellings(source, stretch.run, stretch.last, &tokens)?;
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

fn selected<'f>(fold: &'f Fold, params: &MilitaryAirParams) -> io::Result<Vec<(&'f LeafKey, Points)>> {
    fold.leaves
        .iter()
        .filter(|(key, _)| (params.regions.is_empty() || key.region.is_some_and(|region| params.regions.contains(&region))) && (params.causes.is_empty() || params.causes.contains(&key.cause)))
        .map(|(key, deltas)| Ok((key, points(deltas, params.from_hour)?)))
        .collect()
}

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
        let content = cat.content.layers.derived::<catalogue::CachedResult<content_reader_key_names::buildings::Buildings>>().get_or_init(|| content_reader_key_names::buildings::read(&cat.content.layers)).map_err(|error| error.to_string());
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

fn building_name(cat: &catalogue::Catalogue, content: &content_reader_key_names::buildings::Buildings, token: &str) -> Result<String, String> {
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
        dashboard_text::metadata::check_cancel(cancelled)?;
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

pub(crate) type OpeningKey = (Option<String>, bool, u32, Option<bool>);
pub(crate) type Openings = BTreeMap<OpeningKey, i128>;

fn add_opening(out: &mut Openings, key: OpeningKey, amount: i128) -> io::Result<()> {
    let total = out.entry(key).or_default();
    *total = total.checked_add(amount).ok_or_else(|| dashboard_core::invalid("military air opening overflow"))?;
    Ok(())
}

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

pub(crate) fn openings(source: &Source, query: &Query<'_>, params: &MilitaryAirParams, before: i32, tables: bool, cancelled: &AtomicBool) -> io::Result<Openings> {
    let mut out = Openings::new();
    let (start, end) = (params.from_hour.unwrap_or(query.timeline.first), before.saturating_sub(1));
    if start > end {
        return Ok(out);
    }
    let recording = query.replay.recording();
    let stretches: Vec<_> = query.timeline.within(start, end).collect();
    for stretch in stretches {
        dashboard_text::metadata::check_cancel(cancelled)?;
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

#[derive(Default)]
struct Observed(HashMap<(u32, &'static str), Arc<Mutex<Option<Arc<RunReads>>>>>);

#[derive(Default)]
struct RunReads {
    read: Vec<(i32, i32)>,
    in_part: Vec<(i32, i32)>,
}

fn join(spans: &mut Vec<(i32, i32)>, first: i32, last: i32) {
    match spans.last_mut() {
        Some(held) if held.1.checked_add(1) == Some(first) => held.1 = last,
        _ => spans.push((first, last)),
    }
}

pub(crate) fn forget(source: &Source) {
    *source.state::<Observed>().lock().unwrap_or_else(PoisonError::into_inner) = Observed::default();
    tiles::forget(source);
}

fn observed(source: &Source, query: &Query<'_>, family: MilitaryAirFamily, run: u32) -> io::Result<Arc<RunReads>> {
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
    let mut made = RunReads::default();
    if first <= last {
        for span in spans(first, last, &read::statuses(recording, run, log.table, first, last)?, &read::statuses(recording, run, TableId::CountryLinks, first, last)?) {
            if span.held.read() {
                join(&mut made.read, span.first, span.last);
            }
            if span.held == Held::InPart {
                join(&mut made.in_part, span.first, span.last);
            }
        }
    }
    let made = Arc::new(made);
    *held = Some(Arc::clone(&made));
    Ok(made)
}

pub fn warm_observed(source: &Source, branch: u64) -> io::Result<()> {
    let query = dashboard_base::queries::query(source, branch)?;
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

#[cfg(any(test, feature = "test-support"))]
pub fn kept_observed(source: &Source) -> Vec<(u32, &'static str, Vec<(i32, i32)>)> {
    let index = source.state::<Observed>();
    let index = index.lock().unwrap_or_else(PoisonError::into_inner);
    let mut kept: Vec<(u32, &'static str, Vec<(i32, i32)>)> =
        index.0.iter().filter_map(|(&(run, log), slot)| Some((run, log, slot.lock().unwrap_or_else(PoisonError::into_inner).as_deref()?.read.clone()))).collect();
    kept.sort_unstable();
    kept
}

pub(crate) fn observed_at(source: &Source, query: &Query<'_>, family: MilitaryAirFamily, hour: i32) -> io::Result<bool> {
    let Some(stretch) = query.timeline.at(hour) else { return Ok(false) };
    let reads = observed(source, query, family, stretch.run)?;
    let at = reads.read.partition_point(|&(_, end)| end < hour);
    Ok(reads.read.get(at).is_some_and(|&(start, _)| start <= hour))
}

fn first_observed(source: &Source, query: &Query<'_>, family: MilitaryAirFamily, (first, last): (i32, i32)) -> io::Result<Option<i32>> {
    for stretch in query.timeline.within(first, last) {
        let reads = observed(source, query, family, stretch.run)?;
        let spans = &reads.read;
        let at = spans.partition_point(|&(_, end)| end < stretch.first);
        if let Some(&(start, _)) = spans.get(at).filter(|&&(start, _)| start <= stretch.last) {
            return Ok(Some(start.max(stretch.first)));
        }
    }
    Ok(None)
}

fn observed_since(source: &Source, query: &Query<'_>, family: MilitaryAirFamily, at: i32, floor: i32) -> io::Result<i32> {
    let mut since = at;
    let stretches: Vec<_> = query.timeline.within(floor, at.saturating_sub(1)).collect();
    for stretch in stretches.iter().rev() {
        if stretch.last.checked_add(1) != Some(since) {
            break;
        }
        let reads = observed(source, query, family, stretch.run)?;
        let spans = &reads.read;
        let held = spans.partition_point(|&(_, end)| end < stretch.last);
        let Some(&(start, _)) = spans.get(held).filter(|&&(start, _)| start <= stretch.last) else { return Ok(since) };
        since = start.max(stretch.first);
        if since > stretch.first {
            break;
        }
    }
    Ok(since)
}

fn latest_incomplete(query: &Query<'_>, (from, to): (i32, i32)) -> io::Result<Option<i32>> {
    let log = dashboard_declare::log(MilitaryAirFamily::Bombing.log()).ok_or_else(|| dashboard_core::invalid("No declared bombing log"))?;
    let reader = log::Reader::new(query, log)?;
    let unknown = log::FacetFilter { name: "cause".into(), values: vec![BuildingBombingDamageCause::Unknown as i64], ..Default::default() };
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

fn latest_in_part(source: &Source, query: &Query<'_>, family: MilitaryAirFamily, (from, to): (i32, i32)) -> io::Result<Option<i32>> {
    let stretches: Vec<_> = query.timeline.within(from, to).collect();
    for stretch in stretches.iter().rev() {
        let (first, last) = (stretch.first.max(from), stretch.last.min(to));
        let reads = observed(source, query, family, stretch.run)?;
        let at = reads.in_part.partition_point(|&(start, _)| start <= last);
        if let Some(&(_, end)) = at.checked_sub(1).map(|at| &reads.in_part[at]).filter(|&&(_, end)| end >= first) {
            return Ok(Some(end.min(last)));
        }
    }
    Ok(None)
}

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
    let unknown = if family == MilitaryAirFamily::Bombing { latest_incomplete(query, (since, first - 1))? } else { None };
    if let Some(read) = unknown.max(latest_in_part(source, query, family, (since, first - 1))?) {
        fold.incomplete.insert(0, HourSpan { first_hour: read, last_hour: read });
        if fold.incomplete.get(1).is_some_and(|next| next.first_hour == read + 1) {
            let next = fold.incomplete.remove(1);
            fold.incomplete[0].last_hour = next.last_hour;
        }
    }
    Ok(())
}

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
        unjoined: fold.unjoined.clone(),
        causes: if params.family == MilitaryAirFamily::AirLosses {
            let mut ids: Vec<u32> = leaves.iter().map(|(key, _)| key.cause).chain(openings.keys().map(|key| key.2)).collect();
            ids.sort_unstable();
            ids.dedup();
            ids.into_iter().map(air_loss_cause).collect()
        } else {
            Vec::new()
        },
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
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, family, regions, causes, fromHour: Start, window, top, "causes[]", "regions[]", "top.count"];
    type Reply = MilitaryAirReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        dashboard_world::products::windows::hours(params.window).ok_or("Invalid window")?;
        if params.top.as_ref().is_some_and(|top| top.count == 0) {
            return Err("Top count must be positive");
        }
        Ok(())
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            MilitaryAirParams::decl(),
            MilitaryAirCauseName::decl(),
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
        dashboard_text::metadata::check_cancel(cancelled)?;
        let branch = params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let reply = window_reply(source, &query, params, dashboard_world::products::windows::within(&query, params.window)?, context, cancelled)?;
        dashboard_base::products::check_reply_budget(&reply).map_err(|_| dashboard_core::invalid("Military air ledger reply exceeds transport budget; nothing was truncated"))?;
        Ok(reply)
    }
}
