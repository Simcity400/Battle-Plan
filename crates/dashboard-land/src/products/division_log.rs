
use dashboard_base::{products::{LoadContext, Product, log_page::{self as logs, LogCell, LogPageParams, LogPageReply, LogPageRow}}, queries::country_ids::Names};
use super::divisions::Content;
use dashboard_base::Source;
use dashboard_query::{
    Query,
    log::{Cell, FacetFilter},
};
use replay_data::{
    codec::Decoder,
    rows::{DivisionDeletionDisposition as Disposition, DivisionEventRow, EquipmentDestructionCause as Cause, NameOperands, Recorded, TableId, TagHolderRow},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[cfg(test)]
mod tests;

pub const LOG: &str = "division-events";

mod column {
    pub const DISPOSITION: usize = 0;
    pub const CAUSE: usize = 1;
    pub const SLOT: usize = 3;
    pub const NATIVE_HOUR: usize = 4;
    pub const EXACT: usize = 5;
    pub const AFTER: usize = 6;
    pub const PROVINCE: usize = 7;
    pub const PLACE_HOUR: usize = 8;
    pub const TEMPLATE_HOUR: usize = 10;
    pub const NAME: usize = 11;
    pub const NAMING: usize = 12;
    pub const ICON_UNIT: usize = 14;
    pub const ICON_CUSTOM: usize = 15;
    pub const ICON_NAME: usize = 16;
    pub const KILLER_SLOT: usize = 19;
    pub const KILLER_DIVISION: usize = 20;
    pub const TEMPLATE_ID: usize = 21;
    pub const MATCHED_DEATHS: usize = 22;
    pub const TEMPLATE_LITERAL: usize = 23;
    pub const TEMPLATE_KEY: usize = 24;
    pub const TEMPLATE_COUNTRY: usize = 25;
}

pub const COLUMNS: [&str; 22] = [
    "division",
    "kind",
    "status",
    "hour",
    "by",
    "afterHour",
    "tag",
    "name",
    "nameMissing",
    "icon",
    "iconMissing",
    "presentationHour",
    "template",
    "templateName",
    "templateMissing",
    "province",
    "placeHour",
    "placeAtEvent",
    "killerTag",
    "killerDivision",
    "matchedDeaths",
    "readHour",
];

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

fn branch(text: &str) -> io::Result<u64> {
    text.parse().map_err(|_| invalid("Invalid branch"))
}

fn number(cells: &[Cell], column: usize) -> Option<i64> {
    match cells.get(column) {
        Some(Cell::Number(value)) => Some(*value),
        _ => None,
    }
}

fn bytes(cells: &[Cell], column: usize) -> Option<&[u8]> {
    match cells.get(column) {
        Some(Cell::Text(text)) => Some(text.as_bytes()),
        _ => None,
    }
}

fn opaque<T: replay_data::value::Value>(cells: &[Cell], column: usize) -> Recorded<T> {
    match cells.get(column) {
        Some(Cell::List(bytes)) => T::decode(&mut Decoder::new(bytes)).map_or(Recorded::NotRecorded, Recorded::Value),
        _ => Recorded::NotRecorded,
    }
}

fn number_cell(value: Option<i64>) -> LogCell {
    value.map_or(LogCell::Missing, |value| LogCell::Number { value: value.to_string() })
}

fn text_cell(value: Option<String>) -> LogCell {
    value.map_or(LogCell::Missing, |value| LogCell::Text { value })
}

pub fn status(kind: u8, disposition: Option<i64>, cause: Option<i64>) -> String {
    if kind == DivisionEventRow::CREATED {
        return "first_observed".into();
    }
    if let Some(cause) = cause.and_then(|cause| u8::try_from(cause).ok()).and_then(|cause| Cause::try_from(cause).ok()) {
        return cause.as_str().into();
    }
    let disposition = match disposition {
        Some(0) => Disposition::Loss,
        Some(1) => Disposition::Returned,
        Some(2) => Disposition::Merged,
        Some(3) => Disposition::HqTransferred,
        Some(4) => Disposition::VolunteerTransferred,
        Some(5) => Disposition::ExileTransferred,
        _ => return "unknown".into(),
    };
    disposition.as_str().into()
}

fn slot(value: Option<i64>) -> Option<u32> {
    value.map(|slot| slot as u32).filter(|&slot| slot != TagHolderRow::NONE)
}

pub struct Mapped {
    pub cells: Vec<LogCell>,
    pub template: Option<(Vec<u8>, Option<Vec<u8>>, i32)>,
    pub icon: Option<(u32, i32, Vec<u8>)>,
    pub name: (Recorded<Option<Vec<u8>>>, Recorded<Option<NameOperands>>),
    pub ambiguous: bool,
}

const NAME: usize = 7;
const ICON: usize = 9;
const TEMPLATE_NAME: usize = 13;

pub fn map(hour: i32, key: u128, cells: &[Cell], names: &Names) -> Mapped {
    let kind = (key >> 96) as u8;
    let division = key as u64;
    let exact = number(cells, column::EXACT) == Some(1);
    let tag = slot(number(cells, column::SLOT)).and_then(|slot| names.slot_tag(slot, hour)).map(str::to_owned);
    let killer = slot(number(cells, column::KILLER_SLOT)).and_then(|slot| names.slot_tag(slot, hour)).map(str::to_owned);
    let template = number(cells, column::TEMPLATE_ID).map(|id| id as u32);
    let literal = bytes(cells, column::TEMPLATE_LITERAL).map(<[u8]>::to_vec);
    let key_text = bytes(cells, column::TEMPLATE_KEY).map(<[u8]>::to_vec);
    let country = number(cells, column::TEMPLATE_COUNTRY).map(|country| country as i32);
    let icon = match (number(cells, column::ICON_UNIT), number(cells, column::ICON_CUSTOM), bytes(cells, column::ICON_NAME)) {
        (Some(unit), Some(custom), Some(name)) => Some((unit as u32, custom as i32, name.to_vec())),
        _ => None,
    };
    let matched = number(cells, column::MATCHED_DEATHS);
    let name = (opaque(cells, column::NAME), opaque(cells, column::NAMING));
    let cells = vec![
        LogCell::Text { value: division.to_string() },
        LogCell::Number { value: kind.to_string() },
        LogCell::Text { value: status(kind, number(cells, column::DISPOSITION), number(cells, column::CAUSE)) },
        number_cell(number(cells, column::NATIVE_HOUR).or(Some(i64::from(hour)))),
        LogCell::Number { value: u8::from(!exact).to_string() },
        number_cell(if kind == DivisionEventRow::CREATED { number(cells, column::AFTER) } else { None }),
        text_cell(tag),
        LogCell::Missing,
        LogCell::Text { value: "Division name not recorded at this observation".into() },
        LogCell::Missing,
        LogCell::Text { value: "Division template artwork not recorded at this observation".into() },
        number_cell(number(cells, column::TEMPLATE_HOUR)),
        text_cell(template.map(|id| id.to_string())),
        LogCell::Missing,
        LogCell::Text { value: "Template name not recorded at this observation".into() },
        number_cell(number(cells, column::PROVINCE)),
        number_cell(number(cells, column::PLACE_HOUR)),
        LogCell::Number { value: u8::from(kind == DivisionEventRow::CREATED || exact).to_string() },
        text_cell(killer),
        text_cell(number(cells, column::KILLER_DIVISION).map(|id| id as u64).filter(|&id| id != 0).map(|id| id.to_string())),
        number_cell(matched),
        LogCell::Number { value: hour.to_string() },
    ];
    Mapped {
        cells,
        template: literal.zip(country).map(|(literal, country)| (literal, key_text, country)),
        icon,
        name,
        ambiguous: matched.is_some_and(|count| count > 1),
    }
}

pub struct DivisionLog;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DivisionLogParams {
    pub branch_id: String,
    #[serde(default)]
    pub facets: Vec<FacetFilter>,
    pub from: Option<f64>,
    pub to: f64,
    pub cursor: Option<String>,
    pub page_size: u32,
}

#[derive(Default)]
struct Refused {
    counts: BTreeMap<String, usize>,
    notes: Vec<String>,
    grouped: bool,
}

impl Refused {
    fn named(&mut self, made: Result<String, String>, value: &mut LogCell, missing: &mut LogCell) {
        match made {
            Ok(text) => {
                *value = LogCell::Text { value: text };
                *missing = LogCell::Missing;
            }
            Err(reason) => {
                *self.counts.entry(reason.clone()).or_default() += 1;
                *missing = LogCell::Text { value: reason };
            }
        }
    }

    fn lines(mut self) -> impl Iterator<Item = String> {
        self.notes.sort_unstable();
        self.notes.dedup();
        self.notes.into_iter().chain(self.counts.into_iter().map(|(reason, count)| format!("{reason} ({count})")))
    }
}

impl Product for DivisionLog {
    const KEY: &'static str = "division-log";
    const CONTENT_DEPENDENT: bool = true;
    type Params = DivisionLogParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![
        branchId,
        facets,
        from: Start,
        to: End,
        cursor,
        pageSize,
        "facets[].countries[]",
        "facets[].values[]",
        "facets[].membership[].country",
        "facets[].membership[].firstHour": Start,
        "facets[].membership[].lastHour": End,
        "facets[].unobserved[].firstHour": Start,
        "facets[].unobserved[].lastHour": End
    ];
    type Reply = LogPageReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![DivisionLogParams::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        let kinds = params.facets.iter().find(|facet| facet.name == "kind").ok_or("A division log page names the kinds it shows")?;
        if kinds.values.is_empty() || kinds.values.iter().any(|&kind| !(0..=2).contains(&kind)) {
            return Err("A division log page shows created, lost or transferred divisions");
        }
        Ok(())
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let query = dashboard_base::queries::query(source, branch(&params.branch_id)?)?;
        let asked = LogPageParams {
            branch_id: params.branch_id.clone(),
            log: LOG.to_owned(),
            facets: params.facets.clone(),
            from: params.from,
            to: params.to,
            cursor: params.cursor.clone(),
            page_size: params.page_size,
            columns: None,
        };
        let names = Names::new(&query.holders);
        let tagged = params.facets.iter().any(|filter| !filter.countries.is_empty());
        let (page, rows, missing) = logs::page(&query, &asked, |paged, tag| (paged.row.clone(), paged.ordinal, paged.own, tag.cloned()))?;
        let art = Content::load(source, context);
        let mut refused = Refused::default();
        let mut ambiguous = 0usize;
        let mut reply_rows = Vec::with_capacity(rows.len());
        for (row, ordinal, own, tag) in rows {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let mut mapped = map(row.hour, row.key, &row.cells, &names);
            ambiguous += usize::from(mapped.ambiguous);
            named(&art, (&query, &names, source), row.hour, &mut mapped, &mut refused)?;
            reply_rows.push(LogPageRow {
                hour: row.hour,
                key: row.key.to_string(),
                ordinal: ordinal.to_string(),
                country: tag,
                own_ordinal: tagged.then(|| own.to_string()),
                cells: mapped.cells,
            });
        }
        let mut warnings: Vec<String> = art.unavailable().map(str::to_owned).into_iter().collect();
        warnings.extend(refused.lines());
        if ambiguous > 0 {
            warnings.push(format!("Ambiguous destroyed divisions interval; killer not attributed ({ambiguous})"));
        }
        Ok(LogPageReply {
            branch_id: params.branch_id.clone(),
            log: LOG.to_owned(),
            columns: COLUMNS.iter().map(|column| (*column).to_owned()).collect(),
            first_ordinal: page.first.to_string(),
            total: page.total().to_string(),
            cursor: page.cursor().map(|cursor| cursor.to_string()),
            rows: reply_rows,
            missing,
            warnings,
        })
    }
}

fn named(art: &Content, (query, names, source): (&Query<'_>, &Names, &Source), hour: i32, mapped: &mut Mapped, refused: &mut Refused) -> io::Result<()> {
    if art.unavailable().is_some() {
        return Ok(());
    }
    let cells = &mut mapped.cells;
    let (name, naming) = &mapped.name;
    if !matches!(name, Recorded::NotRecorded) || !matches!(naming, Recorded::NotRecorded) {
        let division = match &cells[0] {
            LogCell::Text { value } => value.parse().unwrap_or(0),
            _ => 0,
        };
        let (value, rest) = cells.split_at_mut(NAME + 1);
        let made = art.division_name_of(division, name, naming);
        if !refused.grouped {
            refused.grouped = true;
            refused.notes.extend(art.group_note_lines());
        }
        refused.named(made, &mut value[NAME], &mut rest[0]);
    }
    if let Some((unit, custom, unit_name)) = &mapped.icon {
        let (value, rest) = cells.split_at_mut(ICON + 1);
        refused.named(art.icon(*unit, *custom, unit_name), &mut value[ICON], &mut rest[0]);
    }
    if let Some((literal, key, country)) = &mapped.template {
        let tag = match (key, query.timeline.at(hour)) {
            (Some(_), Some(stretch)) => {
                let links = dashboard_base::queries::links(source, stretch.run)?;
                names.tag_at(&links, *country as u32, hour).map(str::to_owned)
            }
            _ => None,
        };
        let (value, rest) = cells.split_at_mut(TEMPLATE_NAME + 1);
        refused.named(art.template_name_of(literal, key.as_deref(), tag.as_deref()), &mut value[TEMPLATE_NAME], &mut rest[0]);
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DivisionLogTemplate {
    pub id: String,
    pub name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DivisionLogGap {
    pub first_hour: i32,
    pub last_hour: i32,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DivisionLogManifestReply {
    #[doc = " Whether the branch read its divisions at any hour."]
    pub available: bool,
    #[doc = " Whether it read their removals at any hour."]
    pub losses_available: bool,
    pub gaps: Vec<DivisionLogGap>,
    pub templates: Vec<DivisionLogTemplate>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(DivisionLogManifestReply);

pub struct DivisionLogManifest;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeenTemplate {
    pub id: u32,
    pub hour: i32,
    pub literal: Vec<u8>,
    pub key: Option<Vec<u8>>,
    pub country: Option<i32>,
}

pub fn run_templates(recording: &replay_recording::columnar_frames::Recording, run: u32) -> io::Result<Vec<SeenTemplate>> {
    use dashboard_query::{cost::storage, read};
    let held = recording.run(run)?;
    let (first, last) = (held.first_hour, held.last_hour());
    let spans = storage(|| replay_lanes::lanes::logged_hours(recording, run, TableId::DivisionEvents, first, last))?;
    let mut seen: BTreeMap<u32, SeenTemplate> = BTreeMap::new();
    if spans.is_empty() {
        return Ok(Vec::new());
    }
    let columns = [column::TEMPLATE_ID, column::TEMPLATE_LITERAL, column::TEMPLATE_KEY, column::TEMPLATE_COUNTRY];
    let mut reader = storage(|| replay_recording::cells::TableReader::new(recording, run, TableId::DivisionEvents, Some(&columns)))?;
    for (from, to) in spans {
        let start = if from > first { from - 1 } else { from };
        let mut before: HashSet<u128> = HashSet::new();
        storage(|| reader.go_to(recording, read::ordinal(recording, run, start)?))?;
        for hour in start..=to {
            if hour > start {
                storage(|| reader.step(recording))?;
            }
            let view = reader.view();
            let mut now = HashSet::with_capacity(before.len());
            for slot in view.rows() {
                let Some(key) = view.key(slot) else { continue };
                now.insert(key);
                if hour < from || before.contains(&key) || (key >> 96) as u8 > DivisionEventRow::TRANSFER {
                    continue;
                }
                let Some(id) = view.number(column::TEMPLATE_ID, slot).map(|id| id as u32) else { continue };
                if seen.contains_key(&id) {
                    continue;
                }
                let text = |column: usize| read::text(&view, column, slot).map(<[u8]>::to_vec);
                let country = view.number(column::TEMPLATE_COUNTRY, slot).map(|country| country as i32);
                seen.insert(id, SeenTemplate { id, hour, literal: text(column::TEMPLATE_LITERAL).unwrap_or_default(), key: text(column::TEMPLATE_KEY), country });
            }
            before = now;
        }
    }
    Ok(seen.into_values().collect())
}

pub fn unread(read: &[(i32, i32)], first: i32, last: i32) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    let mut at = i64::from(first);
    for &(a, b) in read {
        if i64::from(b) < at || a > last {
            continue;
        }
        if i64::from(a) > at {
            out.push((at as i32, a - 1));
        }
        at = at.max(i64::from(b) + 1);
    }
    if at <= i64::from(last) {
        out.push((at as i32, last));
    }
    out
}

impl Product for DivisionLogManifest {
    const KEY: &'static str = "division-log-manifest";
    const CONTENT_DEPENDENT: bool = true;
    type Params = dashboard_base::products::BranchParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId];
    type Reply = DivisionLogManifestReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![DivisionLogTemplate::decl(), DivisionLogGap::decl(), DivisionLogManifestReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let query = dashboard_base::queries::query(source, branch(&params.branch_id)?)?;
        let recording = query.replay.recording();
        let names = Names::new(&query.holders);
        let art = Content::load(source, context);
        let mut reply = DivisionLogManifestReply { available: false, losses_available: false, gaps: Vec::new(), templates: Vec::new(), warnings: Vec::new() };
        let mut templates: BTreeMap<u32, (i32, Option<String>)> = BTreeMap::new();
        let mut refused = Refused::default();
        for stretch in &query.timeline.stretches {
            dashboard_text::metadata::check_cancel(cancelled)?;
            for (table, reason, held) in [
                (TableId::Divisions, "Divisions not read; divisions first seen then are dated at the next read", &mut reply.available),
                (TableId::DivisionEquipmentDeletions, "division equipment deletions unavailable; removal causes and totals incomplete", &mut reply.losses_available),
            ] {
                let read = dashboard_query::cost::storage(|| replay_lanes::lanes::reads(recording, stretch.run, table, stretch.first, stretch.last))?;
                *held |= !read.is_empty();
                reply.gaps.extend(unread(&read, stretch.first, stretch.last).into_iter().map(|(first_hour, last_hour)| DivisionLogGap { first_hour, last_hour, reason: reason.to_owned() }));
            }
            let links = dashboard_base::queries::links(source, stretch.run)?;
            for seen in crate::products::division_log::cached_division_templates(source, stretch.run)?.iter() {
                if seen.hour < stretch.first || seen.hour > stretch.last || templates.get(&seen.id).is_some_and(|held| held.0 <= seen.hour) {
                    continue;
                }
                let name = if art.unavailable().is_some() {
                    None
                } else {
                    let tag = seen.country.and_then(|country| names.tag_at(&links, country as u32, seen.hour));
                    let mut value = LogCell::Missing;
                    let mut missing = LogCell::Missing;
                    refused.named(art.template_name_of(&seen.literal, seen.key.as_deref(), tag), &mut value, &mut missing);
                    match value {
                        LogCell::Text { value } => Some(value),
                        _ => None,
                    }
                };
                templates.insert(seen.id, (seen.hour, name));
            }
        }
        reply.gaps.sort_by_key(|gap| (gap.first_hour, gap.reason.clone()));
        reply.templates = templates.into_iter().map(|(id, (_, name))| DivisionLogTemplate { id: id.to_string(), name }).collect();
        reply.warnings.extend(art.unavailable().map(str::to_owned));
        reply.warnings.extend(refused.lines());
        Ok(reply)
    }
}

enum RunKey {}

type Runs = dashboard_base::queries::RunCache<RunKey, Vec<SeenTemplate>>;

pub(crate) fn forget(source: &dashboard_base::Source) {
    Runs::forget(source);
}

pub fn cached_division_templates(source: &dashboard_base::Source, run: u32) -> io::Result<std::sync::Arc<Vec<SeenTemplate>>> {
    Runs::held(&source.state::<Runs>(), run, || run_templates(source.replay_ref().recording(), run))
}
