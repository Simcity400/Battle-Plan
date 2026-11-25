mod roster;
use super::{CareerFacts, CareerMedal, Content, Countries, DecoratedDivisions, Facts, Levels, MedalIndex, Refusals, attributed, describe};
use crate::products::{army::ArmyAttribution, division_log};
use dashboard_base::queries::country_ids::{RunLinks, run_links};
use dashboard_base::{
    Source,
    products::{
        LoadContext, Product,
        at_hour::{self, AtHour, Read},
        occurrences::{self, OccurrenceReply},
    },
};
use dashboard_follow::panel::Panel;
use dashboard_query::{
    Query,
    coverage::{witnessed, witnessed_rows},
    identity::Country,
    log::{
        Reader,
        occurrence::{Ask, Direction, Index, Occurrence, Order, PlacedGap, Selection},
    },
    series::{MissingSpan, ranked::GroupSelection},
};
use dashboard_text::metadata::check_cancel;
use replay_data::rows::{DivisionEventRow, DivisionHistoryRow, DivisionRow, TableId, TableRows, tables};
use replay_lanes::lanes::{self, Cell, Keys as LaneKeys};
use roster::{Roster, Scope};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap},
    io,
    sync::{Arc, Mutex, PoisonError, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceRecordParams {
    pub branch_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub hour: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub from: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub division: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub countries: Option<Vec<Country>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub group: Option<GroupSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub offset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub show: Option<ServiceEntries>,
    pub limit: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ServiceEntries {
    Both,
    Career,
    Awards,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ServiceDivision {
    pub division: String,
    pub name: Option<String>,
    pub tag: Option<String>,
    pub template: Option<String>,
    pub sprite: Option<String>,
    pub medals: Option<i32>,
    pub last_seen: i32,
    pub removed: Option<String>,
    pub removed_at: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ServiceEntry {
    pub key: String,
    pub hour: i32,
    pub observed: i32,
    pub kind: String,
    pub title: Option<String>,
    pub status: Option<String>,
    pub tag: Option<String>,
    pub other: Option<String>,
    pub province: Option<u32>,
    pub place_hour: Option<i32>,
    pub medal: Option<CareerMedal>,
    pub exact: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ServiceRecordTailReply {
    pub branch_id: String,
    pub selected: Option<ServiceDivision>,
    pub facts: Option<CareerFacts>,
    pub medals: Vec<CareerMedal>,
    pub total: String,
    pub offset: String,
    pub rows: Vec<ServiceEntry>,
    pub missing: Vec<PlacedGap>,
    pub next_change: Option<i32>,
    pub unavailable: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ServiceRecordTailReply);

impl OccurrenceReply for ServiceRecordTailReply {
    fn next_change(&self) -> Option<i32> {
        self.next_change
    }
    fn row_count(&self) -> usize {
        self.rows.len()
    }
}

enum Entry {
    History(usize),
    Battle { combat: u128, start: i32, province: Option<u32>, owner: Option<u32>, other: Option<u32> },
    Event(Box<ServiceEntry>),
}

#[derive(Clone)]
struct Summary {
    facts: CareerFacts,
    medals: Vec<CareerMedal>,
    notes: Refusals,
}

#[derive(Default)]
struct Rendered {
    rows: HashMap<u32, (ServiceEntry, Refusals)>,
    last_seen: HashMap<(u64, i32), Summary>,
}

struct Tail {
    roster: Roster,
    content: Content,
    rendered: Mutex<Rendered>,
    history: Vec<(ServiceEntry, Refusals)>,
    entries: Index<Entry>,
}

const TITLE: &str = "Division service record";
const CAREER: u16 = 0;
const AWARD: u16 = 1;
const PARTIAL_ROSTER: &str = "Some divisions' countries or medal totals were not recorded at this hour";

fn occurrence(id: usize, division: u64, date: i32, observed: i32, kind: u16, payload: Entry) -> Occurrence<Entry> {
    Occurrence {
        id: id as u128,
        date,
        observed,
        kind,
        roles: Box::new([]),
        subjects: Box::new([u128::from(division)]),
        states: Box::new([(observed, 1)]),
        sort_keys: Box::new([]),
        text: Box::default(),
        payload,
    }
}

fn battle_entries(
    query: &Query<'_>,
    stretch: &dashboard_query::timeline::Stretch,
    links: &RunLinks,
    entries: &mut Vec<Occurrence<Entry>>,
    participated: &mut BTreeSet<(u128, i32, u64)>,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    let recording = query.replay.recording();
    let (run, first, last) = (stretch.run, stretch.first, stretch.last);
    let start = dashboard_declare::field(TableId::LandBattles, "start_hour")?;
    let battles = lanes::history(recording, run, TableId::LandBattles, &[start], LaneKeys::All, first, last)?;
    let columns = ["province", "attackers", "defenders"].map(|field| dashboard_declare::field(TableId::LandCombats, field));
    let [province, attackers, defenders] = columns;
    let columns = [province?, attackers?, defenders?];
    let members = lanes::history(recording, run, TableId::LandCombats, &columns, LaneKeys::All, first, last)?;
    let asks: Vec<_> = [&battles, &members]
        .into_iter()
        .flat_map(|history| &history.keys)
        .flat_map(|(key, steps)| steps.iter().filter(|step| step.cells.is_some()).map(move |step| (step.hour, *key)))
        .collect();
    let mut joined = Vec::new();
    lanes::at_each(recording, run, TableId::LandCombats, Some(&columns), &asks, &mut |hour, combat, row| {
        check_cancel(cancelled)?;
        let (Some((view, slot)), lanes::Reading::Cells(battle)) = (row, battles.reading(combat, hour)) else {
            return Ok(());
        };
        let Cell::Number(start) = battle[0] else { return Ok(()) };
        let province = view.number(columns[0], slot).map(|id| id as u32);
        for (column, defending) in [(columns[1], false), (columns[2], true)] {
            let Some(bytes) = view.list(column, slot) else { continue };
            let chunks = bytes.chunks_exact(8);
            if !chunks.remainder().is_empty() {
                return Err(dashboard_core::invalid("Incomplete land combat participant"));
            }
            for bytes in chunks {
                let division = u64::from_le_bytes(bytes.try_into().expect("eight bytes"));
                if participated.insert((combat, start as i32, division)) {
                    joined.push((hour, combat, start as i32, division, province, defending));
                }
            }
        }
        Ok(())
    })?;
    let side_asks: Vec<_> = joined.iter().map(|&(at, combat, ..)| (at, combat)).collect();
    let fields = ["attacker_primary", "attacker_secondary", "defender_primary", "defender_secondary"];
    let table = TableId::LandCombatSides;
    let columns = at_hour::field_columns(table, &fields);
    let mask = at_hour::field_mask(table, &fields);
    let mut sides = HashMap::new();
    lanes::at_each(recording, run, table, Some(&columns), &side_asks, &mut |at, combat, read| {
        let Some((view, slot)) = read else { return Ok(()) };
        let TableRows::LandCombatSides(rows) =
            replay_frame::frame::cells_rows_of(table, &view, &[(combat, slot)], recording.format(), mask)?
        else {
            unreachable!()
        };
        let same = |primary: &[u32], secondary: &[u32]| {
            let mut refs = primary.iter().chain(secondary);
            let first = *refs.next()?;
            refs.all(|&r| r == first).then(|| links.slot_at(first, at)).flatten()
        };
        if let Some(row) = rows.first() {
            sides.insert(
                (at, combat),
                (same(&row.attacker_primary, &row.attacker_secondary), same(&row.defender_primary, &row.defender_secondary)),
            );
        }
        Ok(())
    })?;
    let owner_asks: Vec<_> = joined.iter().map(|&(at, _, _, id, ..)| (at, u128::from(id))).collect();
    let columns = at_hour::field_columns(TableId::Divisions, &["roster_owner", "expeditionary_owner"]);
    let mut owners = HashMap::new();
    lanes::at_each(recording, run, TableId::Divisions, Some(&columns), &owner_asks, &mut |at, key, read| {
        if let Some((view, slot)) = read
            && let (Some(roster), Some(expeditionary)) = (view.number(columns[0], slot), view.number(columns[1], slot))
        {
            let owner = replay_data::military_aid::owner(&DivisionRow {
                roster_owner: roster as u32,
                expeditionary_owner: expeditionary as u32,
                ..Default::default()
            });
            owners.insert((at, key as u64), links.slot_at(owner, at));
        }
        Ok(())
    })?;
    for (at, combat, start, id, province, defending) in joined {
        let owner = owners.get(&(at, id)).copied().flatten();
        let other = sides.get(&(at, combat)).and_then(|s| if defending { s.0 } else { s.1 });
        entries.push(occurrence(entries.len(), id, at, at, CAREER, Entry::Battle { combat, start, province, owner, other }));
    }
    Ok(())
}

fn scan(query: &Query<'_>, content: Content, cancelled: &AtomicBool) -> io::Result<Tail> {
    let timeline = &query.timeline;
    let recording = query.replay.recording();
    let mut roster = Roster::default();
    let divisions = &mut roster.divisions;
    let mut entries = Vec::new();
    let mut gaps: Vec<_> = timeline
        .unheld_within(timeline.first, timeline.last)
        .into_iter()
        .map(|gap| MissingSpan { first_hour: gap.first, last_hour: gap.last, reason: gap.reason })
        .collect();
    let reader = Reader::new(
        query,
        dashboard_declare::log("division-events").ok_or_else(|| dashboard_core::invalid("Division events log is not declared"))?,
    )?;
    let mut subject_gaps = HashMap::<u128, Vec<MissingSpan>>::new();
    let mut versions = HashMap::<u128, (i32, usize)>::new();
    let mut participated = BTreeSet::new();
    let mut history_reads = Vec::new();
    for stretch in &timeline.stretches {
        let links = run_links(recording, stretch.run)?;
        battle_entries(query, stretch, &links, &mut entries, &mut participated, cancelled)?;
        let (first, last, run) = (stretch.first, stretch.last, stretch.run);
        roster::scan(query, stretch, &links, divisions, &mut roster.templates, &content, cancelled)?;
        for (table, reason) in [
            (TableId::Divisions, "Division census incomplete"),
            (TableId::DivisionHistory, "Division history incomplete"),
            (TableId::LandCombats, "Land combat census incomplete"),
            (TableId::LandBattles, "Land battle identities incomplete"),
            (TableId::DivisionEquipmentDeletions, "Division removals incomplete"),
        ] {
            let complete = lanes::complete_reads(recording, run, table, first, last)?;
            gaps.extend(division_log::unread(&complete, first, last).into_iter().map(|(first_hour, last_hour)| MissingSpan {
                first_hour,
                last_hour,
                reason: reason.into(),
            }));
        }
        {
            let history = lanes::history(
                recording,
                run,
                TableId::DivisionHistory,
                &(0..TableId::DivisionHistory.field_count()).collect::<Vec<_>>(),
                LaneKeys::All,
                first,
                last,
            )?;
            let tokens = lanes::history(
                recording,
                run,
                TableId::LexicalTokens,
                &[dashboard_declare::field(TableId::LexicalTokens, "literal")?],
                LaneKeys::All,
                first,
                last,
            )?;
            for (&id, roster) in divisions.iter() {
                let seen = &roster.seen[roster.seen.partition_point(|&(_, end)| end < first)..];
                if seen.is_empty() {
                    continue;
                }
                for (start, end, reading) in history.readings(u128::from(id) << 34) {
                    let reason = match reading {
                        lanes::Reading::Cells(cells) if cells[0] == Cell::Number(0) => history
                            .text(cells[1])
                            .and_then(|text| std::str::from_utf8(text).ok())
                            .filter(|why| !why.is_empty())
                            .unwrap_or("This division's history was not recorded"),
                        lanes::Reading::Absent => "This division's history was not recorded",
                        _ => continue,
                    };
                    let at = seen.partition_point(|&(_, last)| last < start);
                    for &(a, b) in seen[at..].iter().take_while(|&&(a, _)| a <= end) {
                        subject_gaps.entry(u128::from(id)).or_default().push(MissingSpan {
                            first_hour: a.max(start),
                            last_hour: b.min(end),
                            reason: reason.into(),
                        });
                    }
                }
            }
            for (key, steps) in &history.keys {
                check_cancel(cancelled)?;
                let list = ((key >> 32) & 3) as u8;
                let division = divisions.entry((key >> 34) as u64).or_default();
                if list == DivisionHistoryRow::SUMMARY {
                    for step in steps {
                        let medal = step.cells.as_ref().and_then(|cells| match cells[2] {
                            Cell::Number(n) if cells[0] == Cell::Number(1) => Some(n as i32),
                            _ => None,
                        });
                        division.medals.put(step.hour, medal);
                        division.facts.push(step.hour);
                    }
                    continue;
                }
                if list == DivisionHistoryRow::AWARDS {
                    let mut previous = None;
                    for (i, step) in steps.iter().enumerate() {
                        let award = step.cells.as_ref().filter(|cells| cells[16] == Cell::Number(1)).map(|cells| (cells[8], cells[9]));
                        if award != previous || i == 0 {
                            division.facts.push(step.hour);
                            previous = award;
                        }
                        if let Some((Cell::Number(token), _)) = award {
                            let key = u128::from(token as u32);
                            division.tokens.insert(key);
                            let end = steps.get(i + 1).map_or(last, |next| next.hour - 1);
                            if let Ok(at) = tokens.keys.binary_search_by_key(&key, |row| row.0) {
                                let spellings = &tokens.keys[at].1;
                                let start = spellings.partition_point(|s| s.hour <= step.hour);
                                division.facts.extend(spellings[start..].iter().take_while(|s| s.hour <= end).map(|s| s.hour));
                            }
                            let start = tokens.statuses.partition_point(|s| s.first <= step.hour);
                            division.facts.extend(tokens.statuses[start..].iter().take_while(|s| s.first <= end).map(|s| s.first));
                        }
                    }
                }
                let division = (key >> 34) as u64;
                let award = list == DivisionHistoryRow::AWARDS;
                for step in steps {
                    let date = step.cells.as_ref().and_then(|cells| match cells[7] {
                        Cell::Number(date) if !award || cells[16] == Cell::Number(1) => Some(date as i32),
                        _ => None,
                    });
                    let old = versions.get(key).copied();
                    if step.cells.is_none()
                        && (divisions.get(&division).and_then(|roster| roster.seen_at(step.hour)) != Some(step.hour)
                            || !matches!(history.reading(u128::from(division) << 34, step.hour), lanes::Reading::Cells(summary) if summary[0] == Cell::Number(1))
                            || !matches!(history.reading(*key, step.hour), lanes::Reading::Absent))
                    {
                        continue;
                    }
                    if old.map(|(date, _)| date) == date {
                        continue;
                    }
                    if let Some((_, at)) = versions.remove(key) {
                        let event: &mut Occurrence<Entry> = &mut entries[at];
                        let mut states = event.states.to_vec();
                        if step.hour > event.observed {
                            states.push((step.hour, 0));
                        } else {
                            states[0].1 = 0;
                        }
                        event.states = states.into();
                    }
                    if let Some(date) = date {
                        let observed = step.hour.max(date);
                        versions.insert(*key, (date, entries.len()));
                        let index = history_reads.len();
                        history_reads.push((step.hour, *key, date, index));
                        entries.push(occurrence(
                            entries.len(),
                            division,
                            date,
                            observed,
                            if award { AWARD } else { CAREER },
                            Entry::History(index),
                        ));
                    }
                }
            }
        }
        for division in divisions.values_mut() {
            division.medals.put(last.saturating_add(1), None);
        }
        for row in reader.inserted(run, &[(first, last)])? {
            let kind = (row.key >> 96) as u8;
            if kind == DivisionEventRow::VANISHED {
                continue;
            }
            let division = row.key as u64;
            let num = |column| division_log::number(&row.cells, column);
            use division_log::column as c;
            let hour = num(c::NATIVE_HOUR).map_or(row.hour, |at| at as i32);
            let tag = |column| division_log::slot(num(column)).and_then(|slot| query.holders.slot_tag(slot, row.hour)).map(str::to_owned);
            let status = division_log::status(kind, num(c::DISPOSITION), num(c::CAUSE));
            if kind != DivisionEventRow::CREATED {
                divisions.entry(division).or_default().removals.push((row.hour, hour, status.clone()));
            }
            let data = ServiceEntry {
                key: format!("event:{}", row.key),
                hour,
                observed: row.hour,
                kind: if kind == DivisionEventRow::CREATED { "created" } else { "removed" }.into(),
                title: None,
                status: (kind != DivisionEventRow::CREATED).then_some(status),
                tag: tag(c::SLOT),
                other: tag(c::KILLER_SLOT),
                province: num(c::PROVINCE).map(|id| id as u32),
                place_hour: num(c::PLACE_HOUR).map(|at| at as i32),
                medal: None,
                exact: num(c::EXACT) == Some(1),
            };
            entries.push(occurrence(entries.len(), division, hour, row.hour.max(hour), CAREER, Entry::Event(Box::new(data))));
        }
    }
    let history = render_history(query, &content, history_reads, cancelled)?;
    roster.finish(query.timeline.last);
    Ok(Tail {
        roster,
        content,
        rendered: Mutex::default(),
        history,
        entries: Index::new(entries, gaps, &[])?.with_subject_gaps(subject_gaps),
    })
}

#[derive(Default)]
struct Records(HashMap<u64, Arc<Tail>>);

struct Prepared {
    tail: Arc<Tail>,
    selection: Option<Selection>,
}

fn prepare(
    source: &Source,
    query: &Query<'_>,
    load: &LoadContext,
    countries: Option<&[Country]>,
    group: Option<&GroupSelection>,
    cancelled: &AtomicBool,
) -> io::Result<Prepared> {
    let cache = source.state::<Records>();
    let mut records = cache.lock().unwrap_or_else(PoisonError::into_inner);
    let content = Content::load(source, load);
    let cached = records.0.get(&query.timeline.id).filter(|tail| match (&tail.content.cat, &content.cat) {
        (Ok(a), Ok(b)) => Arc::ptr_eq(a, b),
        (Err(a), Err(b)) => a == b,
        _ => false,
    });
    let tail = if let Some(tail) = cached {
        Arc::clone(tail)
    } else {
        let tail = Arc::new(scan(query, content, cancelled)?);
        records.0.insert(query.timeline.id, Arc::clone(&tail));
        tail
    };
    drop(records);
    let selection = dashboard_world::products::group_members::dated(source, query, load, countries, group, cancelled)?;
    Ok(Prepared { tail, selection })
}

fn render_history(
    query: &Query<'_>,
    content: &Content,
    mut reads: Vec<(i32, u128, i32, usize)>,
    cancelled: &AtomicBool,
) -> io::Result<Vec<(ServiceEntry, Refusals)>> {
    reads.sort_unstable();
    let mut frames = at_hour::Frames::new(query)?;
    let mut rendered = Vec::with_capacity(reads.len());
    for group in reads.chunk_by(|a, b| a.0 == b.0) {
        check_cancel(cancelled)?;
        let at = group[0].0;
        let award = |key: u128| (key >> 32) & 3 == u128::from(DivisionHistoryRow::AWARDS);
        let mut reads = vec![
            Read::these(TableId::Divisions, group.iter().map(|g| g.1 >> 34)),
            Read::these(TableId::DivisionHistory, group.iter().map(|g| g.1)),
            Read::all(TableId::CountryLinks),
        ];
        if group.iter().any(|g| award(g.1)) {
            reads.push(Read::all(TableId::LexicalTokens));
        }
        let frame = frames.frame_with(at, &reads, DecoratedDivisions::columns, TITLE)?;
        let provinces: BTreeSet<_> = witnessed_rows::<tables::DivisionHistory>(&frame)
            .rows
            .iter()
            .filter(|row| row.list != DivisionHistoryRow::AWARDS && row.sunk_ship.is_none())
            .flat_map(|row| row.provinces.iter().copied())
            .collect();
        let places = if !provinces.is_empty() && content.cat.is_ok() {
            Some(
                dashboard_world::products::place_names::read_hour(query, &mut frames, at, &provinces.into_iter().collect::<Vec<_>>(), &[])
                    .map_err(io::Error::other)?,
            )
        } else {
            None
        };
        let countries = Countries::witnessed(&frame).0.named(&query.holders, at);
        for &(at, history_key, date, index) in group {
            let mut notes = Refusals::default();
            let key = history_key >> 34;
            let tag =
                witnessed::<tables::Divisions>(&frame, key).row().and_then(|row| countries.tag(attributed(row, ArmyAttribution::Owner)));
            let mut medals = MedalIndex::default();
            let entry = witnessed::<tables::DivisionHistory>(&frame, history_key).row();
            let award = award(history_key);
            let medal = entry
                .filter(|_| award)
                .and_then(|row| medals.index(&frame, row.medal_token?, row.medal_definition, content, &mut notes))
                .map(|at| medals.medals[at].clone());
            let title = if award {
                medal.as_ref().and_then(|m| m.name.clone())
            } else {
                entry.and_then(|row| notes.keep(describe(places.as_ref().unwrap_or(&frame), row, content, &countries)).0)
            };
            let other = entry.filter(|row| row.target_country != 0).and_then(|row| countries.tag(row.target_country));
            let province = entry.and_then(|row| row.provinces.first().copied());
            let row = ServiceEntry {
                key: format!("history:{history_key}"),
                hour: date,
                observed: at,
                kind: if award { "award" } else { "career" }.into(),
                title,
                status: entry
                    .and_then(|row| match (award, row.kind) {
                        (true, _) => Some("Awarded"),
                        (false, DivisionHistoryRow::OPERATION | DivisionHistoryRow::NAVAL_COMBAT) => Some("Took part"),
                        (false, DivisionHistoryRow::OPERATION_EXCELLED | DivisionHistoryRow::NAVAL_COMBAT_EXCELLED) => Some("Cited"),
                        _ => None,
                    })
                    .map(str::to_owned),
                tag,
                other,
                province,
                place_hour: province.map(|_| date),
                medal,
                exact: true,
            };
            rendered.push((index, (row, notes)));
        }
    }
    rendered.sort_unstable_by_key(|r| r.0);
    Ok(rendered.into_iter().map(|(_, row)| row).collect())
}

fn summary(query: &Query<'_>, content: &Content, id: u64, at: i32, division: &roster::Division) -> io::Result<Summary> {
    let key = u128::from(id) << 34;
    let awards = key | (u128::from(DivisionHistoryRow::AWARDS) << 32);
    let reads = [
        Read { table: TableId::DivisionHistory, keys: at_hour::Keys::Ranges(vec![(key, key), (awards, awards | u128::from(u32::MAX))]) },
        Read::these(TableId::LexicalTokens, division.tokens.iter().copied()),
    ];
    let frame = at_hour::frame_with(
        query,
        at,
        &reads,
        |table| {
            (table == TableId::DivisionHistory).then(|| {
                at_hour::field_columns(table, &["available", "reason", "amount", "medal_token", "medal_definition", "medal_enabled"])
            })
        },
        TITLE,
    )?;
    let mut details = Facts { frame: &frame, content, medals: MedalIndex::default(), levels: Levels::read(content) };
    let mut notes = Refusals::default();
    let (experience, casualties) = division.measures.at(at).copied().unwrap_or_default();
    let facts = details.measured(witnessed_rows::<tables::DivisionHistory>(&frame).rows, experience, casualties, &mut notes);
    Ok(Summary { facts, medals: details.medals.medals, notes })
}

fn reply(
    prepared: &Prepared,
    query: &Query<'_>,
    params: &ServiceRecordParams,
    hour: i32,
    cancelled: &AtomicBool,
) -> io::Result<ServiceRecordTailReply> {
    let (offset, limit) = occurrences::window(params.offset.as_deref(), params.limit)?;
    let tail = &prepared.tail;
    let content = &tail.content;
    let mut warnings = Refusals::default();
    let scope = Scope::at(query, prepared.selection.as_ref(), hour);
    let pinned = params.division.as_ref().and_then(|id| id.parse::<u64>().ok());
    let (subject, mut ambiguous, leader_change) = match pinned {
        Some(id) => (Some(id), false, None),
        None => tail.roster.leader(&scope, hour),
    };
    let division = subject.and_then(|id| tail.roster.divisions.get(&id));
    let choice = subject.and_then(|id| tail.roster.choice(id, hour, &scope, &mut ambiguous));
    let subject = subject.filter(|_| choice.is_some());
    let show = match params.show {
        Some(ServiceEntries::Career) => Some(&[CAREER][..]),
        Some(ServiceEntries::Awards) => Some(&[AWARD][..]),
        _ => None,
    };
    let answer = tail.entries.ask_with_gap_end(
        &query.holders,
        &Ask {
            hour,
            from: params.from,
            to: None,
            selection: None,
            subject: Some(subject.map_or(u128::MAX, u128::from)),
            kinds: show,
            states: Some(&[1]),
            search: None,
            order: Order::Date(Direction::Descending),
            live_first: false,
            offset,
            limit,
            chips: &[],
        },
        subject.and(division).and_then(|d| d.removed(hour)).map_or(i32::MAX, |r| r.0),
    )?;
    let mut entries = Vec::new();
    let mut facts = None;
    let mut medals = Vec::new();
    if let (Some(id), Some(choice), Some(division)) = (subject, &choice, division) {
        let mut rendered = tail.rendered.lock().unwrap_or_else(PoisonError::into_inner);
        let summary = if choice.last_seen == hour && choice.removed_at.is_none() {
            summary(query, content, id, hour, division)?
        } else {
            let saved = match rendered.last_seen.entry((id, choice.last_seen)) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => entry.insert(summary(query, content, id, choice.last_seen, division)?),
            };
            saved.clone()
        };
        facts = Some(summary.facts);
        medals = summary.medals;
        warnings = summary.notes;
        for shown in &answer.rows {
            check_cancel(cancelled)?;
            let event = tail.entries.get(shown.at);
            let (row, notes) = match &event.payload {
                Entry::Event(row) => {
                    entries.push((**row).clone());
                    continue;
                }
                Entry::History(index) => &tail.history[*index],
                Entry::Battle { combat, start, province, owner, other } => rendered.rows.entry(shown.at).or_insert_with(|| {
                    let tag = |slot: Option<u32>| slot.and_then(|s| query.holders.slot_tag(s, event.observed)).map(str::to_owned);
                    (
                        ServiceEntry {
                            key: format!("battle:{combat}:{start}"),
                            hour: event.date,
                            observed: event.observed,
                            kind: "battle".into(),
                            title: Some("Land battle".into()),
                            status: Some("Took part".into()),
                            tag: tag(*owner),
                            other: tag(*other),
                            province: *province,
                            place_hour: province.map(|_| event.observed),
                            medal: None,
                            exact: true,
                        },
                        Refusals::default(),
                    )
                }),
            };
            entries.push(row.clone());
            for (reason, count) in &notes.0 {
                warnings.note_times(reason, *count);
            }
        }
    }
    let next_change = [answer.next_change, scope.next, leader_change, division.and_then(|d| d.next_change(hour, &tail.roster, &scope))]
        .into_iter()
        .flatten()
        .filter(|&at| at <= query.timeline.last)
        .min();
    let ambiguous = ambiguous || scope.unknown;
    let incomplete = ambiguous
        || query.timeline.at(hour).is_none()
        || answer.gaps.iter().any(|gap| {
            gap.last_hour == hour && matches!(gap.reason.as_str(), "Division census incomplete" | "Division history incomplete")
        });
    let unavailable = if params.division.is_some() && subject.is_none() {
        Some("The chosen division has not been observed in these countries through this hour.".into())
    } else if subject.is_none() && incomplete {
        Some("The most decorated division could not be determined from the recorded census and medals.".into())
    } else {
        None
    };
    if ambiguous {
        warnings.note(PARTIAL_ROSTER);
    }
    content.group_notes(&mut warnings);
    let reply = ServiceRecordTailReply {
        branch_id: params.branch_id.clone(),
        selected: choice,
        facts,
        medals,
        rows: entries,
        total: answer.total.to_string(),
        offset: offset.to_string(),
        missing: answer.gaps,
        next_change,
        unavailable,
        warnings: warnings.lines(),
    };
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceChoicesParams {
    pub branch_id: String,
    pub hour: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub countries: Option<Vec<Country>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub group: Option<GroupSelection>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ServiceChoicesReply {
    pub partial: Option<String>,
    pub choices: Vec<ServiceDivision>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ServiceChoicesReply);

pub struct DivisionServiceRecordChoices;

impl Product for DivisionServiceRecordChoices {
    const KEY: &'static str = "division-service-record-choices";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = ServiceChoicesParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, hour: Read, countries, "countries[]", group];
    type Reply = ServiceChoicesReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![ServiceChoicesParams::decl(), ServiceChoicesReply::decl(), ServiceDivision::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        params.countries.iter().flatten().try_for_each(dashboard_base::products::validate_country)
    }

    fn load(source: &Source, params: &Self::Params, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let hour = occurrences::cutoff(&query, Some(params.hour))?;
        let prepared = prepare(source, &query, load, params.countries.as_deref(), params.group.as_ref(), cancelled)?;
        let roster = &prepared.tail.roster;
        let scope = Scope::at(&query, prepared.selection.as_ref(), hour);
        let mut partial = scope.unknown;
        let mut choices: Vec<_> = roster.divisions.keys().filter_map(|&id| roster.choice(id, hour, &scope, &mut partial)).collect();
        choices.sort_by(|a, b| {
            b.removed_at.cmp(&a.removed_at).then(b.medals.cmp(&a.medals)).then(a.name.cmp(&b.name)).then(a.division.cmp(&b.division))
        });
        Ok(ServiceChoicesReply { choices, partial: partial.then(|| PARTIAL_ROSTER.to_owned()), warnings: Vec::new() })
    }
}

pub struct DivisionServiceRecordTail;

impl Product for DivisionServiceRecordTail {
    const KEY: &'static str = "division-service-record-tail";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = ServiceRecordParams;
    const FIELDS: dashboard_base::products::Fields =
        dashboard_base::fields![branchId, hour: Read, from: Start, division, countries, "countries[]", group, offset, show, limit];
    type Reply = ServiceRecordTailReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            ServiceRecordParams::decl(),
            ServiceEntries::decl(),
            ServiceDivision::decl(),
            ServiceEntry::decl(),
            ServiceRecordTailReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        if let Some(id) = &params.division {
            id.parse::<u64>().map_err(|_| "Invalid division")?;
        }
        params.countries.iter().flatten().try_for_each(dashboard_base::products::validate_country)
    }

    fn load(source: &Source, params: &Self::Params, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let hour = occurrences::cutoff(&query, params.hour)?;
        reply(
            &prepare(source, &query, load, params.countries.as_deref(), params.group.as_ref(), cancelled)?,
            &query,
            params,
            hour,
            cancelled,
        )
    }
}

pub fn dated_panel(source: &Source, load: &LoadContext, asked: &serde_json::Value) -> Result<Box<dyn Panel>, String> {
    let mut request = asked.clone();
    request.as_object_mut().ok_or("Invalid service record panel")?.remove("panel");
    let params: ServiceRecordParams = dashboard_query::named::named(&request).map_err(|error| error.to_string())?;
    DivisionServiceRecordTail::validate(&params)?;
    let branch = params.branch_id.parse().map_err(|_| "Invalid branch")?;
    let query = dashboard_base::queries::query(source, branch).map_err(|error| error.to_string())?;
    let prepared = prepare(source, &query, load, params.countries.as_deref(), params.group.as_ref(), &AtomicBool::new(false))
        .map_err(|error| error.to_string())?;
    let (timeline, holders, replay) = (query.timeline, query.holders, source.replay());
    Ok(occurrences::occurrence_panel(source.tables(), move |hour, cancelled| {
        let query = Query { timeline: Arc::clone(&timeline), holders: Arc::clone(&holders), replay: &replay };
        reply(&prepared, &query, &params, occurrences::cutoff(&query, Some(hour))?, cancelled)
    }))
}
