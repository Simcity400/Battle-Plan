use super::{
    CareerFacts, CareerMedal, Content, Countries, DecoratedDivisions, DecoratedRank, Facts, Levels, Measures, MedalIndex,
    Refusals, attributed, describe, manpower_of, rank_order, status_of,
};
use crate::products::{army::ArmyAttribution, division_log};
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
    identity::{Country, Holder},
    log::{
        Reader,
        occurrence::{Ask, Direction, Index, Occurrence, Order, PlacedGap, Selection},
    },
    series::{MissingSpan, ranked::GroupSelection},
};
use dashboard_text::metadata::check_cancel;
use replay_data::rows::{DivisionEventRow, DivisionHistoryRow, TableId, tables};
use replay_lanes::lanes::{self, Cell, Keys as LaneKeys};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, PoisonError, atomic::AtomicBool},
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
    pub selected: Option<String>,
    pub choices: Vec<ServiceDivision>,
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
    History { key: u128, read: i32 },
    Battle { combat: u128, start: i32, province: Option<u32>, defending: bool },
    Event(ServiceEntry),
}

#[derive(Default)]
struct Division {
    seen: Vec<(i32, i32)>,
    removals: Vec<(i32, i32, String)>,
}

impl Division {
    fn seen_at(&self, hour: i32) -> Option<i32> {
        let end = self.seen.partition_point(|&(first, _)| first <= hour);
        Some(self.seen.get(end.checked_sub(1)?)?.1.min(hour))
    }
}

struct Tail {
    divisions: BTreeMap<u64, Division>,
    entries: Index<Entry>,
    changes: Vec<i32>,
}

const TITLE: &str = "Division service record";
const CAREER: u16 = 0;
const AWARD: u16 = 1;

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
    lanes::at_each(recording, run, TableId::LandCombats, Some(&columns), &asks, &mut |hour, combat, row| {
        check_cancel(cancelled)?;
        let (Some((view, slot)), lanes::Reading::Cells(battle)) = (row, battles.reading(combat, hour)) else { return Ok(()) };
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
                    entries.push(occurrence(
                        entries.len(),
                        division,
                        hour,
                        hour,
                        CAREER,
                        Entry::Battle { combat, start: start as i32, province, defending },
                    ));
                }
            }
        }
        Ok(())
    })
}

fn scan(query: &Query<'_>, cancelled: &AtomicBool) -> io::Result<Tail> {
    let timeline = &query.timeline;
    let recording = query.replay.recording();
    let mut divisions = BTreeMap::<u64, Division>::new();
    let mut entries = Vec::new();
    let mut changes = BTreeSet::new();
    let mut gaps: Vec<_> = timeline
        .unheld_within(timeline.first, timeline.last)
        .into_iter()
        .map(|gap| MissingSpan { first_hour: gap.first, last_hour: gap.last, reason: gap.reason })
        .collect();
    let reader = Reader::new(
        query,
        dashboard_declare::log("division-events")
            .ok_or_else(|| dashboard_core::invalid("Division events log is not declared"))?,
    )?;
    let mut subject_gaps = HashMap::<u128, Vec<MissingSpan>>::new();
    let mut versions = HashMap::<u128, (i32, usize)>::new();
    let mut participated = BTreeSet::new();
    for stretch in &timeline.stretches {
        battle_entries(query, stretch, &mut entries, &mut participated, cancelled)?;
        let (first, last, run) = (stretch.first, stretch.last, stretch.run);
        changes.insert(first);
        changes.insert(last.saturating_add(1));
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
        for table in [
            TableId::Divisions,
            TableId::DivisionStatus,
            TableId::DivisionLandInputs,
            TableId::CountryLinks,
            TableId::Countries,
            TableId::DivisionTemplates,
            TableId::DivisionTemplateIcons,
            TableId::LexicalTokens,
            TableId::DivisionHistory,
        ] {
            check_cancel(cancelled)?;
            let columns = DecoratedDivisions::columns(table).unwrap_or_else(|| (0..table.field_count()).collect());
            let history = lanes::history(recording, run, table, &columns, LaneKeys::All, first, last)?;
            changes.extend(history.statuses.iter().map(|status| status.first));
            if table == TableId::DivisionHistory {
                for (&id, roster) in &divisions {
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
            }
            for (key, steps) in &history.keys {
                check_cancel(cancelled)?;
                changes.extend(steps.iter().map(|step| step.hour));
                if table == TableId::Divisions {
                    let at = history
                        .columns
                        .binary_search(&dashboard_declare::field(table, "reveal_skip")?)
                        .map_err(|_| dashboard_core::invalid("Missing division visibility"))?;
                    let division = divisions.entry(*key as u64).or_default();
                    for (i, step) in steps.iter().enumerate() {
                        if step.cells.as_ref().is_some_and(|cells| cells[at] == Cell::Number(0)) {
                            let until = steps.get(i + 1).map_or(last, |next| next.hour - 1);
                            division.seen.push((step.hour, until));
                        }
                    }
                }
                if table != TableId::DivisionHistory {
                    continue;
                }
                let list = ((key >> 32) & 3) as u8;
                if list == DivisionHistoryRow::SUMMARY {
                    continue;
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
                        entries.push(occurrence(
                            entries.len(),
                            division,
                            date,
                            observed,
                            if award { AWARD } else { CAREER },
                            Entry::History { key: *key, read: step.hour },
                        ));
                    }
                }
            }
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
            let tag = |column| {
                division_log::slot(num(column)).and_then(|slot| query.holders.slot_tag(slot, row.hour)).map(str::to_owned)
            };
            let status = division_log::status(kind, num(c::DISPOSITION), num(c::CAUSE));
            if kind != DivisionEventRow::CREATED {
                divisions.entry(division).or_default().removals.push((row.hour, hour, status.clone()));
            }
            changes.insert(row.hour);
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
            entries.push(occurrence(entries.len(), division, hour, row.hour.max(hour), CAREER, Entry::Event(data)));
        }
    }
    for division in divisions.values_mut() {
        division.removals.sort_unstable();
    }
    changes.extend(query.holders.tags.values().flatten().flat_map(|run| [run.first, run.last.saturating_add(1)]));
    Ok(Tail {
        divisions,
        entries: Index::new(entries, gaps, &[])?.with_subject_gaps(subject_gaps),
        changes: changes.into_iter().filter(|&at| at <= timeline.last).collect(),
    })
}

#[derive(Default)]
struct Records(HashMap<u64, Arc<Tail>>);

struct Prepared {
    tail: Arc<Tail>,
    content: Content,
    selection: Option<Selection>,
}

fn prepare(
    source: &Source,
    query: &Query<'_>,
    load: &LoadContext,
    params: &ServiceRecordParams,
    cancelled: &AtomicBool,
) -> io::Result<Prepared> {
    let cache = source.state::<Records>();
    let mut records = cache.lock().unwrap_or_else(PoisonError::into_inner);
    let tail = match records.0.entry(query.timeline.id) {
        std::collections::hash_map::Entry::Occupied(entry) => Arc::clone(entry.get()),
        std::collections::hash_map::Entry::Vacant(entry) => Arc::clone(entry.insert(Arc::new(scan(query, cancelled)?))),
    };
    drop(records);
    let selection = dashboard_world::products::group_members::dated(
        source,
        query,
        load,
        params.countries.as_deref(),
        params.group.as_ref(),
        cancelled,
    )?;
    Ok(Prepared { tail, content: Content::load(source, load), selection })
}

struct Roster {
    choices: Vec<ServiceDivision>,
    leader: Option<Measures>,
    ambiguous: bool,
}

fn roster(
    query: &Query<'_>,
    tail: &Tail,
    content: &Content,
    selection: Option<&Selection>,
    hour: i32,
    cancelled: &AtomicBool,
) -> io::Result<Roster> {
    let mut samples = BTreeMap::<i32, Vec<u128>>::new();
    for (&id, roster) in &tail.divisions {
        if let Some(at) = roster.seen_at(hour) {
            samples.entry(at).or_default().push(u128::from(id));
        }
    }
    let mut choices = Vec::new();
    let mut leader: Option<Measures> = None;
    let mut ambiguous = selection.is_some_and(|set| {
        set.unknown.iter().any(|span| span.first_hour <= hour && hour <= span.last_hour)
            || set.unobserved.iter().any(|span| span.first_hour <= hour && hour <= span.last_hour)
    });
    let mut templates = HashMap::new();
    let selected_slots: Option<BTreeSet<_>> = selection.map(|set| {
        set.membership
            .iter()
            .filter(|span| span.first_hour <= hour && hour <= span.last_hour)
            .filter_map(|span| match span.country {
                Country::Slot(slot) => Some(slot),
                Country::Tag(_) => match query.holders.at(&span.country, hour).map(|run| run.holder) {
                    Some(Holder::Found { slot, .. }) => Some(slot),
                    _ => {
                        ambiguous = true;
                        None
                    }
                },
            })
            .collect()
    });
    let mut frames = at_hour::Frames::new(query)?;
    for (at, ids) in samples {
        check_cancel(cancelled)?;
        let reads = [
            Read::these(TableId::Divisions, ids.iter().copied()),
            Read::these(TableId::DivisionStatus, ids.iter().copied()),
            Read::these(TableId::DivisionLandInputs, ids.iter().copied()),
            Read::these(TableId::DivisionHistory, ids.iter().map(|id| id << 34)),
            Read::all(TableId::DivisionTemplates),
            Read::all(TableId::DivisionTemplateIcons),
            Read::all(TableId::CountryLinks),
        ];
        let frame = frames.frame_with(at, &reads, DecoratedDivisions::columns, TITLE)?;
        let countries = Countries::witnessed(&frame).0.named(&query.holders, at);
        for row in witnessed_rows::<tables::Divisions>(&frame).rows {
            let owner = attributed(row, ArmyAttribution::Owner);
            let slot = countries.slot(owner);
            if selected_slots.as_ref().is_some_and(|slots| !slot.is_some_and(|slot| slots.contains(&slot))) {
                ambiguous |= slot.is_none();
                continue;
            }
            let status = status_of(&frame, row.id);
            let summary = witnessed::<tables::DivisionHistory>(&frame, u128::from(row.id) << 34).row();
            let medals = summary.filter(|row| row.available != 0).map(|row| row.amount);
            let measures = Measures {
                id: row.id,
                medals,
                experience: status
                    .zip(manpower_of(&frame, row.id))
                    .and_then(|(status, manpower)| status.normalized_experience(manpower)),
                casualties_taken: status.and_then(|status| status.casualties_taken()),
                name: None,
            };
            let roster = &tail.divisions[&row.id];
            let end = roster.removals.partition_point(|&(observed, _, _)| observed <= hour);
            let removed = end.checked_sub(1).and_then(|i| roster.removals.get(i)).filter(|(observed, _, _)| *observed >= at);
            if at == hour && removed.is_none() {
                ambiguous |= medals.is_none();
                if medals.is_some_and(|n| n > 0)
                    && leader.as_ref().is_none_or(|held| rank_order(&measures, held, DecoratedRank::Medals, true).is_lt())
                {
                    leader = Some(measures);
                }
            }
            let (template, sprite) = templates
                .entry((at, row.template))
                .or_insert_with(|| {
                    let icon = witnessed::<tables::DivisionTemplateIcons>(&frame, u128::from(row.template)).row();
                    let sprite = icon.zip(content.cat.as_ref().ok()).and_then(|(icon, cat)| {
                        super::super::icon_key(icon.priority_unit, icon.custom_icon, &icon.priority_unit_name, |key| {
                            cat.sprites.find(key).map(|sprite| sprite.is_some())
                        })
                        .ok()
                    });
                    (content.template_name(&frame, row.template).ok(), sprite)
                })
                .clone();
            choices.push(ServiceDivision {
                division: row.id.to_string(),
                name: content.division_name(status).ok(),
                tag: countries.tag(owner),
                template,
                sprite,
                medals,
                last_seen: at,
                removed: removed.map(|(_, _, status)| status.clone()),
                removed_at: removed.map(|(_, date, _)| *date),
            });
        }
    }
    choices.sort_by(|a, b| {
        b.removed_at.cmp(&a.removed_at).then(b.medals.cmp(&a.medals)).then(a.name.cmp(&b.name)).then(a.division.cmp(&b.division))
    });
    Ok(Roster { choices, leader, ambiguous })
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
    let content = &prepared.content;
    let mut warnings = Refusals::default();
    let selection = prepared.selection.as_ref();
    let Roster { choices, leader, ambiguous } = roster(query, tail, content, selection, hour, cancelled)?;
    let selected = params.division.clone().or_else(|| leader.map(|top| top.id.to_string()));
    let choice = selected.as_ref().and_then(|id| choices.iter().find(|row| &row.division == id));
    let subject = choice.and_then(|row| row.division.parse::<u64>().ok());
    let show = match params.show {
        Some(ServiceEntries::Career) => Some(&[CAREER][..]),
        Some(ServiceEntries::Awards) => Some(&[AWARD][..]),
        _ => None,
    };
    let answer = tail.entries.ask(
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
    )?;
    let mut entries = Vec::new();
    let mut facts = None;
    let mut medals = Vec::new();
    if let (Some(id), Some(choice)) = (subject, choice) {
        let key = u128::from(id);
        let reads = [
            Read::these(TableId::DivisionStatus, [key]),
            Read::these(TableId::DivisionLandInputs, [key]),
            Read { table: TableId::DivisionHistory, keys: at_hour::Keys::Range(key << 34, (key << 34) | ((1 << 34) - 1)) },
            Read::all(TableId::LexicalTokens),
        ];
        let frame = at_hour::frame_with(query, choice.last_seen, &reads, DecoratedDivisions::columns, TITLE)?;
        let mut details = Facts { frame: &frame, content, medals: MedalIndex::default(), levels: Levels::read(content) };
        facts = Some(details.of(
            witnessed_rows::<tables::DivisionHistory>(&frame).rows,
            status_of(&frame, id),
            manpower_of(&frame, id),
            &mut warnings,
        ));
        let mut frames = at_hour::Frames::new(query)?;
        for shown in &answer.rows {
            check_cancel(cancelled)?;
            let event = tail.entries.get(shown.at);
            let mut reads = vec![Read::these(TableId::Divisions, [key]), Read::all(TableId::CountryLinks)];
            let read_at = match event.payload {
                Entry::Event(ref row) => {
                    entries.push(row.clone());
                    continue;
                }
                Entry::Battle { combat, .. } => {
                    reads.push(Read::these(TableId::LandCombatSides, [combat]));
                    event.observed
                }
                Entry::History { key, read } => {
                    reads.push(Read::these(TableId::DivisionHistory, [key]));
                    if event.kind == AWARD {
                        reads.push(Read::all(TableId::LexicalTokens));
                    }
                    read
                }
            };
            let frame = frames.frame_with(read_at, &reads, DecoratedDivisions::columns, TITLE)?;
            let countries = Countries::witnessed(&frame).0.named(&query.holders, read_at);
            let tag = witnessed::<tables::Divisions>(&frame, key)
                .row()
                .and_then(|row| countries.tag(attributed(row, ArmyAttribution::Owner)));
            match &event.payload {
                Entry::Event(_) => unreachable!(),
                Entry::Battle { combat, start, province, defending } => {
                    let other = witnessed::<tables::LandCombatSides>(&frame, *combat).row().and_then(|sides| {
                        let (primary, secondary) = if *defending {
                            (&sides.attacker_primary, &sides.attacker_secondary)
                        } else {
                            (&sides.defender_primary, &sides.defender_secondary)
                        };
                        let mut refs = primary.iter().chain(secondary);
                        let first = refs.next()?;
                        refs.all(|raw| raw == first).then(|| countries.tag(*first)).flatten()
                    });
                    entries.push(ServiceEntry {
                        key: format!("battle:{combat}:{start}"),
                        hour: event.date,
                        observed: event.observed,
                        kind: "battle".into(),
                        title: Some("Land battle".into()),
                        status: Some("Took part".into()),
                        tag,
                        other,
                        province: *province,
                        place_hour: province.map(|_| event.observed),
                        medal: None,
                        exact: true,
                    });
                }
                Entry::History { key, .. } => {
                    let entry = witnessed::<tables::DivisionHistory>(&frame, *key).row();
                    let award = event.kind == AWARD;
                    let medal = entry
                        .filter(|_| award)
                        .and_then(|row| {
                            details.medals.index(&frame, row.medal_token?, row.medal_definition, content, &mut warnings)
                        })
                        .map(|at| details.medals.medals[at].clone());
                    let title = if award {
                        medal.as_ref().and_then(|m| m.name.clone())
                    } else {
                        let place_frame = if entry.is_some_and(|row| !row.provinces.is_empty() && !matches!(row.kind, 12 | 13)) {
                            let mut reads: Vec<_> = dashboard_text::scripted_text::TABLES
                                .iter()
                                .copied()
                                .chain([TableId::Provinces, TableId::States])
                                .map(Read::all)
                                .collect();
                            reads.sort_by_key(|read| read.table);
                            reads.dedup_by_key(|read| read.table);
                            Some(frames.frame(read_at, &reads, TITLE)?)
                        } else {
                            None
                        };
                        entry.and_then(|row| {
                            warnings.keep(describe(place_frame.as_ref().unwrap_or(&frame), row, content, &countries)).0
                        })
                    };
                    let other = entry.filter(|row| row.target_country != 0).and_then(|row| countries.tag(row.target_country));
                    let province = entry.and_then(|row| row.provinces.first().copied());
                    entries.push(ServiceEntry {
                        key: format!("history:{key}"),
                        hour: event.date,
                        observed: read_at,
                        kind: if award { "award" } else { "career" }.into(),
                        title,
                        status: entry
                            .and_then(|row| match (award, row.kind) {
                                (true, _) => Some("Awarded"),
                                (false, 12 | 23) => Some("Took part"),
                                (false, 13 | 24) => Some("Cited"),
                                _ => None,
                            })
                            .map(str::to_owned),
                        tag,
                        other,
                        province,
                        place_hour: province.map(|_| event.date),
                        medal,
                        exact: true,
                    });
                }
            }
        }
        medals = details.medals.medals;
    }
    let membership_change = selection
        .into_iter()
        .flat_map(|set| {
            set.membership
                .iter()
                .map(|span| (span.first_hour, span.last_hour))
                .chain(set.unobserved.iter().map(|span| (span.first_hour, span.last_hour)))
                .chain(set.unknown.iter().map(|span| (span.first_hour, span.last_hour)))
        })
        .flat_map(|(first, last)| [first, last.saturating_add(1)])
        .filter(|&at| at > hour)
        .min();
    let next_change =
        [answer.next_change, tail.changes.get(tail.changes.partition_point(|&at| at <= hour)).copied(), membership_change]
            .into_iter()
            .flatten()
            .min();
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
        warnings.note("Some divisions' countries or medal totals were not recorded at this hour");
    }
    content.group_notes(&mut warnings);
    let reply = ServiceRecordTailReply {
        branch_id: params.branch_id.clone(),
        selected: choice.map(|row| row.division.clone()),
        choices,
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

pub struct DivisionServiceRecordTail;

impl Product for DivisionServiceRecordTail {
    const KEY: &'static str = "division-service-record-tail";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = ServiceRecordParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, hour: Read, from: Start, division, countries, "countries[]", group, offset, show, limit];
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
        reply(&prepare(source, &query, load, params, cancelled)?, &query, params, hour, cancelled)
    }
}

pub fn dated_panel(source: &Source, load: &LoadContext, asked: &serde_json::Value) -> Result<Box<dyn Panel>, String> {
    let mut request = asked.clone();
    request.as_object_mut().ok_or("Invalid service record panel")?.remove("panel");
    let params: ServiceRecordParams = dashboard_query::named::named(&request).map_err(|error| error.to_string())?;
    DivisionServiceRecordTail::validate(&params)?;
    let branch = params.branch_id.parse().map_err(|_| "Invalid branch")?;
    let query = dashboard_base::queries::query(source, branch).map_err(|error| error.to_string())?;
    let prepared = prepare(source, &query, load, &params, &AtomicBool::new(false)).map_err(|error| error.to_string())?;
    let (timeline, holders, replay) = (query.timeline, query.holders, source.replay());
    Ok(occurrences::occurrence_panel(source.tables(), move |hour, cancelled| {
        let query = Query { timeline: Arc::clone(&timeline), holders: Arc::clone(&holders), replay: &replay };
        reply(&prepared, &query, &params, occurrences::cutoff(&query, Some(hour))?, cancelled)
    }))
}
