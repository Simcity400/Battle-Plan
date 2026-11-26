use super::{bytes, column, number, opaque, slot, status, unread};
use dashboard_query::log::Cell;
use crate::products::divisions::Content;
use dashboard_base::{
    Source,
    catalogue::Catalogue,
    products::{
        LoadContext, Product,
        occurrences::{self, OccurrenceCount, OccurrenceReply},
    },
    queries::country_ids::RunLinks,
};
use dashboard_follow::panel::Panel;
use dashboard_query::{
    Query,
    identity::{Country, Holders},
    log::{
        Reader, Row,
        occurrence::{Ask, Chip, Direction, Index, Occurrence, Order, PlacedGap, Role, Selection},
    },
    series::{MissingSpan, ranked::GroupSelection},
};
use dashboard_text::metadata::check_cancel;
use replay_data::rows::{DivisionEventRow, TableId};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, Mutex, OnceLock, PoisonError, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum DivisionEvents {
    Both,
    Created,
    Lost,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DivisionLogTailParams {
    pub branch_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub hour: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub from: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub to: Option<i32>,
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
    pub events: Option<DivisionEvents>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub templates: Option<Vec<u32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub expanded: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub totals: Option<bool>,
    pub limit: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DivisionTemplateChip {
    pub id: u32,
    pub name: Option<String>,
    pub first_hour: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DivisionEventDetail {
    pub division: String,
    pub kind: u8,
    pub status: String,
    pub exact: bool,
    pub after_hour: Option<i32>,
    pub name: Option<String>,
    pub name_missing: Option<String>,
    pub icon: Option<String>,
    pub icon_missing: Option<String>,
    pub template: Option<u32>,
    pub template_name: Option<String>,
    pub template_missing: Option<String>,
    pub province: Option<u32>,
    pub place_hour: Option<i32>,
    pub killer_slot: Option<u32>,
    pub killer_tag: Option<String>,
    pub killer_division: Option<String>,
    pub matched_deaths: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DivisionLogRow {
    pub key: String,
    pub hour: i32,
    pub observed: i32,
    pub slot: Option<u32>,
    pub tag: Option<String>,
    pub event: Option<DivisionEventDetail>,
    pub parent: Option<String>,
    pub members: usize,
    pub open: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DivisionLogTailReply {
    pub branch_id: String,
    pub available: bool,
    pub unavailable: Option<String>,
    pub total: String,
    pub offset: String,
    pub counts: Vec<OccurrenceCount>,
    pub census_incomplete: bool,
    pub templates: Vec<DivisionTemplateChip>,
    pub rows: Vec<DivisionLogRow>,
    pub missing: Vec<PlacedGap>,
    pub next_change: Option<i32>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(DivisionLogTailReply);

impl OccurrenceReply for DivisionLogTailReply {
    fn next_change(&self) -> Option<i32> {
        self.next_change
    }
    fn row_count(&self) -> usize {
        self.rows.len()
    }
}

struct Event {
    key: u128,
    observed: i32,
    hour: i32,
    kind: u8,
    slot: Option<u32>,
    template: Option<u32>,
    template_tag: Option<String>,
    recorded: Fields,
    detail: OnceLock<Box<DivisionEventDetail>>,
}

const NUMBERS: [usize; 11] = [
    column::DISPOSITION,
    column::CAUSE,
    column::EXACT,
    column::AFTER,
    column::PROVINCE,
    column::PLACE_HOUR,
    column::ICON_UNIT,
    column::ICON_CUSTOM,
    column::KILLER_SLOT,
    column::KILLER_DIVISION,
    column::MATCHED_DEATHS,
];

struct Fields {
    numbers: [Option<i64>; NUMBERS.len()],
    name: Option<Box<[u8]>>,
    naming: Option<Box<[u8]>>,
    icon_name: Option<Box<[u8]>>,
    template_literal: Option<Box<[u8]>>,
    template_key: Option<Box<[u8]>>,
}

impl Fields {
    fn new(cells: &[Cell]) -> Self {
        let list = |column| match cells.get(column) {
            Some(Cell::List(bytes)) => Some(bytes.as_slice().into()),
            _ => None,
        };
        let text = |column| bytes(cells, column).map(Box::from);
        Self {
            numbers: NUMBERS.map(|column| number(cells, column)),
            name: list(column::NAME),
            naming: list(column::NAMING),
            icon_name: text(column::ICON_NAME),
            template_literal: text(column::TEMPLATE_LITERAL),
            template_key: text(column::TEMPLATE_KEY),
        }
    }

    fn number(&self, column: usize) -> Option<i64> {
        NUMBERS.iter().position(|&held| held == column).and_then(|at| self.numbers[at])
    }
}

fn named(result: Result<String, String>) -> (Option<String>, Option<String>) {
    match result {
        Ok(text) => (Some(text), None),
        Err(reason) => (None, Some(reason)),
    }
}

impl Event {
    fn new(row: Row, holders: &Holders, links: &RunLinks) -> Self {
        let cells = &row.cells;
        let template_tag = number(cells, column::TEMPLATE_COUNTRY)
            .and_then(|id| links.slot_at(id as u32, row.hour))
            .and_then(|id| holders.slot_tag(id, row.hour))
            .map(str::to_owned);
        Self {
            key: row.key,
            observed: row.hour,
            hour: number(cells, column::NATIVE_HOUR).map_or(row.hour, |h| h as i32),
            kind: (row.key >> 96) as u8,
            slot: slot(number(cells, column::SLOT)),
            template: number(cells, column::TEMPLATE_ID).map(|id| id as u32),
            template_tag,
            recorded: Fields::new(cells),
            detail: OnceLock::new(),
        }
    }

    fn tag(&self, holders: &Holders) -> Option<String> {
        self.slot.and_then(|id| holders.slot_tag(id, self.observed)).map(str::to_owned)
    }

    fn group(&self) -> String {
        format!("group:{}:{}:{}", self.observed, self.hour, self.slot.map_or(String::new(), |slot| slot.to_string()))
    }

    fn template_name(&self, art: &Content) -> Result<String, String> {
        let literal = self.recorded.template_literal.as_deref().ok_or_else(|| "Template name not recorded at this observation".to_owned())?;
        art.template_name_of(literal, self.recorded.template_key.as_deref(), self.template_tag.as_deref())
    }

    fn detail(&self, art: &Content, holders: &Holders) -> &DivisionEventDetail {
        self.detail.get_or_init(|| {
            let recorded = &self.recorded;
            let num = |at| recorded.number(at);
            let kind = self.kind;
            let division = self.key as u64;
            let killer_slot = slot(num(column::KILLER_SLOT));
            let (name, name_missing) = named(art.division_name_of(division, &opaque(recorded.name.as_deref()), &opaque(recorded.naming.as_deref())));
            let icon = match (num(column::ICON_UNIT), num(column::ICON_CUSTOM), recorded.icon_name.as_deref()) {
                (Some(unit), Some(custom), Some(text)) => art.icon(unit as u32, custom as i32, text),
                _ => Err("Division template artwork not recorded at this observation".into()),
            };
            let (icon, icon_missing) = named(icon);
            let (template_name, template_missing) = named(self.template_name(art));
            Box::new(DivisionEventDetail {
                division: division.to_string(),
                kind,
                status: status(kind, num(column::DISPOSITION), num(column::CAUSE)),
                exact: num(column::EXACT) == Some(1),
                after_hour: num(column::AFTER).map(|hour| hour as i32),
                name,
                name_missing,
                icon,
                icon_missing,
                template: self.template,
                template_name,
                template_missing,
                province: num(column::PROVINCE).map(|id| id as u32),
                place_hour: num(column::PLACE_HOUR).map(|h| h as i32),
                killer_slot,
                killer_tag: killer_slot.and_then(|id| holders.slot_tag(id, self.observed)).map(str::to_owned),
                killer_division: num(column::KILLER_DIVISION).filter(|&id| id != 0).map(|id| (id as u64).to_string()),
                matched_deaths: num(column::MATCHED_DEATHS).map(|n| n as u32),
            })
        })
    }
}
#[derive(PartialEq)]
struct Filters {
    templates: Option<BTreeSet<u32>>,
    expanded: BTreeSet<String>,
    events: DivisionEvents,
}

enum Line {
    Single(usize),
    Header { first: usize, members: usize, open: bool },
    Child(usize),
}

impl Line {
    fn event(&self) -> usize {
        match self {
            Self::Single(at) | Self::Header { first: at, .. } | Self::Child(at) => *at,
        }
    }
}

struct Tail {
    art: Content,
    first_read: Option<i32>,
    has_vanished: bool,
    events: Vec<Event>,
    chips: Vec<DivisionTemplateChip>,
    gaps: Vec<MissingSpan>,
    removal_gaps: Vec<MissingSpan>,
    warnings: Vec<String>,
    view: Mutex<Option<(Filters, Arc<Index<Line>>)>>,
}

const REMOVAL_GAP: &str = "Division equipment deletions incomplete";

fn scan(source: &Source, query: &Query<'_>, art: Content, cancelled: &AtomicBool) -> io::Result<Tail> {
    let timeline = &query.timeline;
    let mut gaps: Vec<_> = timeline
        .unheld_within(timeline.first, timeline.last)
        .into_iter()
        .map(|gap| MissingSpan { first_hour: gap.first, last_hour: gap.last, reason: gap.reason })
        .collect();
    let mut first_read = None;
    let mut removal_gaps = Vec::new();
    let declared = dashboard_declare::log("division-events").ok_or_else(|| dashboard_core::invalid("Division events log is not declared"))?;
    let reader = Reader::new(query, declared)?;
    let mut events = Vec::new();
    let mut chips = BTreeMap::new();
    for stretch in &timeline.stretches {
        check_cancel(cancelled)?;
        let recording = query.replay.recording();
        for (table, reason, out) in
            [(TableId::Divisions, "Division census incomplete", &mut gaps), (TableId::DivisionEquipmentDeletions, REMOVAL_GAP, &mut removal_gaps)]
        {
            let read =
                dashboard_query::cost::storage(|| replay_lanes::lanes::complete_reads(recording, stretch.run, table, stretch.first, stretch.last))?;
            if let Some(&(hour, _)) = read.first() {
                first_read = Some(first_read.map_or(hour, |held: i32| held.min(hour)));
            }
            out.extend(unread(&read, stretch.first, stretch.last).into_iter().map(|(first_hour, last_hour)| MissingSpan {
                first_hour,
                last_hour,
                reason: reason.into(),
            }));
        }
        let links = dashboard_base::queries::links(source, stretch.run)?;
        for row in reader.inserted(stretch.run, &[(stretch.first, stretch.last)])? {
            check_cancel(cancelled)?;
            let event = Event::new(row, &query.holders, &links);
            if event.kind != DivisionEventRow::VANISHED
                && let Some(id) = event.template
            {
                let observed = event.observed;
                if chips.get(&id).is_none_or(|chip: &DivisionTemplateChip| observed < chip.first_hour) {
                    chips.insert(id, DivisionTemplateChip { id, first_hour: observed, name: event.template_name(&art).ok() });
                }
            }
            events.push(event);
        }
    }
    let mut chips: Vec<_> = chips.into_values().collect();
    chips.sort_by_key(|chip| (chip.first_hour, chip.id));
    let warnings = match art.unavailable() {
        Some(reason) => vec![reason.to_owned()],
        None => art.group_note_lines(),
    };
    let has_vanished = events.iter().any(|event| event.kind == DivisionEventRow::VANISHED);
    let first_read = first_read.into_iter().chain(events.iter().map(|event| event.observed)).min();
    Ok(Tail { art, first_read, has_vanished, events, chips, gaps, removal_gaps, warnings, view: Mutex::default() })
}

const SINGLE: u8 = 0;
const HEADER: u8 = 1;
const CHILD: u8 = 2;
const HIDDEN_CHILD: u8 = 3;
const VANISHED: u8 = 4;

impl Tail {
    fn index(&self, filters: Filters, cancelled: &AtomicBool) -> io::Result<Arc<Index<Line>>> {
        let mut kept = self.view.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((held, index)) = kept.as_ref()
            && held == &filters
        {
            return Ok(Arc::clone(index));
        }
        let mut groups: BTreeMap<(i32, i32, u32), Vec<usize>> = BTreeMap::new();
        let mut lines = Vec::new();
        let mut push = |line: Line, state| {
            let event = &self.events[line.event()];
            lines.push(Occurrence {
                id: u128::MAX - lines.len() as u128,
                date: event.hour,
                observed: event.observed,
                kind: u16::from(event.kind),
                roles: Box::new([Role { slot: event.slot, at: event.observed }]),
                subjects: Box::new([]),
                states: Box::new([(event.observed, state)]),
                sort_keys: Box::new([]),
                text: Box::default(),
                payload: line,
            });
        };
        for (at, event) in self.events.iter().enumerate() {
            check_cancel(cancelled)?;
            if filters.templates.as_ref().is_some_and(|ids| event.template.is_none_or(|id| !ids.contains(&id))) {
                continue;
            }
            if event.kind == DivisionEventRow::VANISHED {
                push(Line::Single(at), VANISHED);
                continue;
            }
            let created = event.kind == DivisionEventRow::CREATED;
            if (filters.events == DivisionEvents::Created && !created) || (filters.events == DivisionEvents::Lost && created) {
                continue;
            }
            if created && let Some(slot) = event.slot {
                groups.entry((event.hour, event.observed, slot)).or_default().push(at);
            } else {
                push(Line::Single(at), SINGLE);
            }
        }
        for members in groups.into_values() {
            if members.len() == 1 {
                push(Line::Single(members[0]), SINGLE);
                continue;
            }
            let open = filters.expanded.contains(&self.events[members[0]].group());
            push(Line::Header { first: members[0], members: members.len(), open }, HEADER);
            for at in members {
                push(Line::Child(at), if open { CHILD } else { HIDDEN_CHILD });
            }
        }
        let mut gaps = self.gaps.clone();
        if filters.events != DivisionEvents::Created {
            gaps.extend_from_slice(&self.removal_gaps);
        }
        let index = Arc::new(Index::new(lines, gaps, &[])?);
        *kept = Some((filters, Arc::clone(&index)));
        Ok(index)
    }
}

type Binding = Result<Arc<Catalogue>, String>;

#[derive(Default)]
struct Kept(HashMap<u64, (Binding, Arc<Tail>)>);

pub(crate) fn forget(source: &Source) {
    source.state::<Kept>().lock().unwrap_or_else(PoisonError::into_inner).0.clear();
}

#[cfg(feature = "test-support")]
pub fn is_cached(source: &Source, branch: u64) -> bool {
    source.state::<Kept>().lock().unwrap_or_else(PoisonError::into_inner).0.contains_key(&branch)
}

struct Prepared {
    tail: Arc<Tail>,
    index: Arc<Index<Line>>,
    selection: Option<Selection>,
}

fn prepare(source: &Source, query: &Query<'_>, load: &LoadContext, params: &DivisionLogTailParams, cancelled: &AtomicBool) -> io::Result<Prepared> {
    let branch = params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
    let art = Content::load(source, load);
    let cache = source.state::<Kept>();
    let tail = {
        let mut kept = cache.lock().unwrap_or_else(PoisonError::into_inner);
        let cached = kept.0.get(&branch).filter(|(binding, _)| match (binding, &art.cat) {
            (Ok(a), Ok(b)) => Arc::ptr_eq(a, b),
            (Err(a), Err(b)) => a == b,
            _ => false,
        });
        if let Some((_, tail)) = cached {
            Arc::clone(tail)
        } else {
            let binding = art.cat.clone();
            let tail = Arc::new(scan(source, query, art, cancelled)?);
            kept.0.insert(branch, (binding, Arc::clone(&tail)));
            tail
        }
    };
    let filters = Filters {
        templates: params.templates.as_ref().map(|ids| ids.iter().copied().collect()),
        expanded: params.expanded.iter().flatten().cloned().collect(),
        events: params.events.unwrap_or(DivisionEvents::Both),
    };
    let index = tail.index(filters, cancelled)?;
    let selection =
        dashboard_world::products::group_members::dated(source, query, load, params.countries.as_deref(), params.group.as_ref(), cancelled)?;
    Ok(Prepared { tail, index, selection })
}

fn reply(prepared: &Prepared, holders: &Holders, params: &DivisionLogTailParams, hour: i32) -> io::Result<DivisionLogTailReply> {
    let (offset, limit) = occurrences::window(params.offset.as_deref(), params.limit)?;
    let tail = &prepared.tail;
    let ask = Ask {
        hour,
        from: params.from,
        to: params.to,
        selection: prepared.selection.as_ref(),
        subject: None,
        kinds: None,
        states: Some(&[SINGLE, HEADER, CHILD]),
        search: None,
        order: Order::Date(Direction::Descending),
        live_first: false,
        offset,
        limit,
        chips: &[],
    };
    let answer = prepared.index.ask(holders, &ask)?;
    let mut warnings = tail.warnings.clone();
    let counts = if params.totals == Some(true) || tail.has_vanished {
        let chips = [DivisionEventRow::CREATED, DivisionEventRow::LOST, DivisionEventRow::TRANSFER, DivisionEventRow::VANISHED]
            .map(|kind| Chip { kinds: Some(vec![u16::from(kind)]), states: None });
        let counted = prepared
            .index
            .ask(holders, &Ask { offset: 0, limit: 1, states: Some(&[SINGLE, CHILD, HIDDEN_CHILD, VANISHED]), chips: &chips, ..ask })?;
        let vanished = counted.chips[3];
        if vanished > 0 {
            warnings.push(format!("{vanished} divisions left the census without a recorded removal."));
        }
        if params.totals == Some(true) { occurrences::counts(&counted, &["created", "lost", "transferred"]) } else { Vec::new() }
    } else {
        Vec::new()
    };
    let rows = answer
        .rows
        .iter()
        .map(|shown| {
            let line = &prepared.index.get(shown.at).payload;
            let event = &tail.events[line.event()];
            let (key, detail, parent, members, open) = match line {
                Line::Header { members, open, .. } => (event.group(), None, None, *members, *open),
                Line::Single(_) | Line::Child(_) => {
                    let parent = matches!(line, Line::Child(_)).then(|| event.group());
                    (event.key.to_string(), Some(event.detail(&tail.art, holders).clone()), parent, 0, false)
                }
            };
            DivisionLogRow {
                key,
                hour: event.hour,
                observed: event.observed,
                slot: event.slot,
                tag: event.tag(holders),
                event: detail,
                parent,
                members,
                open,
            }
        })
        .collect();
    let end = tail.chips.partition_point(|chip| chip.first_hour <= hour);
    let next_change = answer.next_change.into_iter().chain(tail.chips.get(end).map(|chip| chip.first_hour)).min();
    if answer.unattributed > 0 {
        warnings.push("Some divisions have no recorded owner for the country filter.".into());
    }
    let available = tail.first_read.is_some_and(|first| first <= hour);
    let reply = DivisionLogTailReply {
        available,
        unavailable: (!available).then(|| "Division census and removals were not recorded through this hour.".into()),
        branch_id: params.branch_id.clone(),
        total: answer.total.to_string(),
        offset: offset.to_string(),
        counts,
        census_incomplete: answer.gaps.iter().any(|gap| gap.reason != REMOVAL_GAP),
        templates: tail.chips[..end].to_vec(),
        rows,
        missing: answer.gaps,
        next_change,
        warnings,
    };
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
}

pub struct DivisionLogTail;

impl Product for DivisionLogTail {
    const KEY: &'static str = "division-log-tail";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = DivisionLogTailParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![
        branchId, hour: Read, from: Start, to: End, countries, "countries[]", group, offset, events, templates, "templates[]", expanded, "expanded[]", totals, limit
    ];
    type Reply = DivisionLogTailReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            DivisionLogTailParams::decl(),
            DivisionEvents::decl(),
            DivisionTemplateChip::decl(),
            DivisionEventDetail::decl(),
            DivisionLogRow::decl(),
            DivisionLogTailReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        params.countries.iter().flatten().try_for_each(dashboard_base::products::validate_country)
    }

    fn load(source: &Source, params: &Self::Params, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        check_cancel(cancelled)?;
        let branch = params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let hour = occurrences::cutoff(&query, params.hour)?;
        let prepared = prepare(source, &query, load, params, cancelled)?;
        reply(&prepared, &query.holders, params, hour)
    }
}

pub fn dated_panel(source: &Source, load: &LoadContext, asked: &serde_json::Value) -> Result<Box<dyn Panel>, String> {
    let mut request = asked.clone();
    request.as_object_mut().ok_or("Invalid division-log-tail panel")?.remove("panel");
    let params: DivisionLogTailParams =
        dashboard_query::named::named(&request).map_err(|error| format!("Invalid division-log-tail panel: {error}"))?;
    DivisionLogTail::validate(&params)?;
    let branch = params.branch_id.parse().map_err(|_| "Invalid branch")?;
    let query = dashboard_base::queries::query(source, branch).map_err(|error| error.to_string())?;
    let prepared = prepare(source, &query, load, &params, &AtomicBool::new(false)).map_err(|error| error.to_string())?;
    let (first, last, holders) = (query.timeline.first, query.timeline.last, query.holders);
    Ok(occurrences::occurrence_panel(source.tables(), move |hour, cancelled| {
        check_cancel(cancelled)?;
        if hour < first {
            return Err(dashboard_core::invalid("The hour asked is before the branch"));
        }
        reply(&prepared, &holders, &params, hour.min(last))
    }))
}
