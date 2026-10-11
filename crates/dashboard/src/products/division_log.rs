//! Observed appearances and witnessed removals, never snapshot-inferred deaths.
//! Deletion clears expeditionary ownership; the capture preserves it. Recorded
//! dispositions distinguish reserve returns, merges and transfers. Destruction
//! and combat lifecycle supply killer context, joined only within the same drain.
//! First appearance has no creation date: its date is an upper bound.
use super::{
    BranchParams, LoadContext, MAX_PRODUCT_REPLY_BYTES, Product,
    battles::{BattleWarning, BattleWindow},
    land_battles::LandBattleCountryFilter,
};
use crate::{
    Source,
    catalogue::{self, Catalogue},
    metadata::{LinkTags, check_cancel, slot_read, validate_campaign},
    scripted_text::stored_text,
    transport::json_len,
};
use replay::{
    container::Replay,
    frame::{DecodedFrame, FrameDecoder, SharedView, TableView},
    rows::{
        DivisionDeletionDisposition as Disposition, DivisionEquipmentDeletionRow, DivisionRow,
        DivisionStatusRow, DivisionTemplateRow, EquipmentDestructionCause as Cause, NameOperands,
        Recorded, TableId, TableSchema, tables,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};
use ts_rs::TS;

pub struct DivisionLog;
pub struct DivisionLogManifest;
pub struct DivisionLogTotals;
const TABLES: [TableId; 10] = [
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::Divisions,
    TableId::DivisionStatus,
    TableId::DivisionTemplates,
    TableId::DivisionEquipmentDeletions,
    TableId::DestroyedDivisions,
    TableId::LandEventCoverage,
    TableId::DivisionTemplateIcons,
];

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DivisionLogParams {
    pub branch_id: String,
    pub window: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum DivisionLogKind {
    Created,
    Lost,
    Transfer,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DivisionLogEntry {
    pub id: String,
    pub division: String,
    /// Available to playback only from this drain or census, even for an earlier death.
    pub read_hour: i32,
    pub hour: i32,
    pub by: bool,
    /// Last complete census excluding a newly observed handle, absent across gaps.
    pub after_hour: Option<i32>,
    pub kind: DivisionLogKind,
    pub status: String,
    pub tag: Option<String>,
    pub country_hour: i32,
    pub name: Option<String>,
    pub name_missing: Option<String>,
    pub icon: Option<String>,
    pub icon_missing: Option<String>,
    pub presentation_hour: Option<i32>,
    pub template: Option<String>,
    pub template_name: Option<String>,
    pub template_missing: Option<String>,
    pub template_hour: Option<i32>,
    pub province: Option<u32>,
    pub place_hour: Option<i32>,
    /// Otherwise the place is the division's last observed location.
    pub place_at_event: bool,
    pub killer_tag: Option<String>,
    pub killer_hour: Option<i32>,
    pub killer_division: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DivisionLogReply {
    pub window: usize,
    pub interval: BattleWindow,
    pub rows: Vec<DivisionLogEntry>,
    pub warnings: Vec<BattleWarning>,
    pub content_warnings: Vec<String>,
}
impl super::ReplyWarnings for DivisionLogReply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        &mut self.content_warnings
    }
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DivisionLogTemplate {
    pub id: String,
    pub name: Option<String>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DivisionLogManifestReply {
    pub windows: Vec<BattleWindow>,
    pub available: bool,
    pub losses_available: bool,
    pub templates: Vec<DivisionLogTemplate>,
    pub content_warnings: Vec<String>,
}
impl super::ReplyWarnings for DivisionLogManifestReply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        &mut self.content_warnings
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DivisionLogTotalsParams {
    pub branch_id: String,
    pub first_hour: i32,
    pub last_hour: i32,
    pub countries: LandBattleCountryFilter,
    pub templates: Option<Vec<String>>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DivisionLogTotalsReply {
    pub created: usize,
    pub lost: usize,
    pub transferred: usize,
    pub warnings: Vec<String>,
}
impl super::ReplyWarnings for DivisionLogTotalsReply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        &mut self.warnings
    }
}

#[derive(Clone)]
struct Presentation {
    hour: i32,
    division: DivisionRow,
    status: Option<DivisionStatusRow>,
    template: Option<DivisionTemplateRow>,
    template_hour: Option<i32>,
    icon: Option<(u32, i32, Vec<u8>)>,
}
struct Event {
    row: DivisionLogEntry,
    presentation: Option<Presentation>,
}
// Rows hold unique ascending keys, so a large simultaneous deployment is looked
// up without indexing every hour.
struct Presentations<'a> {
    hour: i32,
    divisions: &'a [DivisionRow],
    names: Names<'a>,
    templates: &'a [DivisionTemplateRow],
    icons: &'a [replay::rows::DivisionTemplateIconRow],
    template_hour: Option<i32>,
}
fn keyed<T>(rows: &[T], key: u64, of: impl Fn(&T) -> u64) -> Option<&T> {
    rows.binary_search_by_key(&key, of).ok().map(|at| &rows[at])
}
impl<'a> Presentations<'a> {
    fn new(frame: &'a DecodedFrame, template_hour: Option<i32>) -> Self {
        Self {
            hour: frame.hour,
            template_hour,
            divisions: frame.complete::<tables::Divisions>().unwrap_or_default(),
            names: Names::Rows(
                frame
                    .complete::<tables::DivisionStatus>()
                    .unwrap_or_default(),
            ),
            templates: frame
                .complete::<tables::DivisionTemplates>()
                .unwrap_or_default(),
            icons: frame
                .complete::<tables::DivisionTemplateIcons>()
                .unwrap_or_default(),
        }
    }
    fn get(&self, id: u64) -> Option<Presentation> {
        let division = keyed(self.divisions, id, |row| row.id)?.clone();
        let status = match self.names {
            Names::Rows(rows) => keyed(rows, id, |row| row.division)
                .map(|row| status(row.division, &row.name, &row.naming)),
            Names::Held(names) => keyed(names, id, |held| held.division)
                .map(|held| status(held.division, &held.name, &held.naming)),
        };
        let template = keyed(self.templates, division.template, |row| row.template).cloned();
        let icon = keyed(self.icons, division.template, |row| row.template).map(|row| {
            (
                row.priority_unit,
                row.custom_icon,
                row.priority_unit_name.clone(),
            )
        });
        Some(Presentation {
            hour: self.hour,
            division,
            status,
            template,
            template_hour: self.template_hour,
            icon,
        })
    }
}
/// A status row as a presentation keeps it: its division and name operands.
fn status(
    division: u64,
    name: &Recorded<Option<Vec<u8>>>,
    naming: &Recorded<Option<NameOperands>>,
) -> DivisionStatusRow {
    DivisionStatusRow {
        division,
        name: name.clone(),
        naming: naming.clone(),
        ..Default::default()
    }
}
enum Names<'a> {
    Rows(&'a [DivisionStatusRow]),
    Held(&'a [HeldName]),
}
struct HeldName {
    division: u64,
    name: Recorded<Option<Vec<u8>>>,
    naming: Recorded<Option<NameOperands>>,
}
/// The last census's presentation inputs. DivisionStatus changes every hour, so
/// only its names are kept, reused where unchanged: holding its rows would copy
/// them at every later hour.
struct Census {
    hour: i32,
    template_hour: Option<i32>,
    divisions: SharedView,
    templates: SharedView,
    icons: SharedView,
    names: Vec<HeldName>,
}
impl Census {
    fn new(frame: &DecodedFrame, template_hour: Option<i32>, held: Option<Census>) -> Self {
        let mut held = held
            .map_or_else(Vec::new, |census| census.names)
            .into_iter()
            .peekable();
        let names = frame
            .complete::<tables::DivisionStatus>()
            .unwrap_or_default()
            .iter()
            .map(|row| {
                while held.next_if(|name| name.division < row.division).is_some() {}
                held.next_if(|name| {
                    name.division == row.division
                        && name.name == row.name
                        && name.naming == row.naming
                })
                .unwrap_or_else(|| HeldName {
                    division: row.division,
                    name: row.name.clone(),
                    naming: row.naming.clone(),
                })
            })
            .collect();
        let view = |table: TableId| frame.tables[table.index()].clone();
        Self {
            hour: frame.hour,
            template_hour,
            divisions: view(TableId::Divisions),
            templates: view(TableId::DivisionTemplates),
            icons: view(TableId::DivisionTemplateIcons),
            names,
        }
    }
    fn presentations(&self) -> Presentations<'_> {
        fn rows<T: TableSchema>(view: &SharedView) -> &[T::Row] {
            match &**view {
                TableView::Complete(rows) => rows.rows::<T>().unwrap_or_default(),
                _ => &[],
            }
        }
        Presentations {
            hour: self.hour,
            divisions: rows::<tables::Divisions>(&self.divisions),
            names: Names::Held(&self.names),
            templates: rows::<tables::DivisionTemplates>(&self.templates),
            icons: rows::<tables::DivisionTemplateIcons>(&self.icons),
            template_hour: self.template_hour,
        }
    }
}
#[derive(Default)]
struct Scan {
    events: Vec<Event>,
    seen: HashSet<u64>,
    /// The last census's divisions, ascending.
    live: Vec<u64>,
    deletions: HashMap<u64, DivisionEquipmentDeletionRow>,
    links: LinkTags,
    previous_hour: Option<i32>,
    previous_census: Option<i32>,
    presentation: Option<Census>,
    template_hour: Option<i32>,
    available: bool,
    losses_available: bool,
    warnings: BTreeMap<(i32, String), usize>,
}
impl Scan {
    fn warn(&mut self, hour: i32, reason: impl Into<String>) {
        *self.warnings.entry((hour, reason.into())).or_default() += 1;
    }
    fn gap(&mut self) {
        self.previous_hour = None;
        self.previous_census = None;
        self.template_hour = None;
    }
    fn row(
        &self,
        id: u64,
        hour: i32,
        kind: DivisionLogKind,
        status: &str,
        raw: u32,
        presentation: Option<&Presentation>,
    ) -> DivisionLogEntry {
        DivisionLogEntry {
            id: format!("{id}:{}:{hour}", status),
            division: id.to_string(),
            read_hour: hour,
            hour,
            by: true,
            after_hour: None,
            kind,
            status: status.into(),
            tag: self.links.get(raw).cloned(),
            country_hour: hour,
            name: None,
            name_missing: Some("Division name not recorded at this observation".into()),
            icon: None,
            icon_missing: Some("Division template artwork not recorded at this observation".into()),
            presentation_hour: presentation.map(|value| value.hour),
            template: presentation.map(|value| value.division.template.to_string()),
            template_name: None,
            template_missing: Some("Template name not recorded at this observation".into()),
            template_hour: presentation.and_then(|value| value.template_hour),
            province: presentation.map(|value| value.division.province),
            place_hour: presentation.map(|value| value.hour),
            place_at_event: kind == DivisionLogKind::Created,
            killer_tag: None,
            killer_hour: None,
            killer_division: None,
        }
    }
    fn observe(&mut self, frame: &DecodedFrame) {
        let hour = frame.hour;
        if self
            .previous_hour
            .is_some_and(|last| last.checked_add(1) != Some(hour))
        {
            self.gap();
        }
        let labels = self.links.update(frame).is_ok();
        if !labels {
            self.warn(
                hour,
                "Country identity unavailable; division events retain unknown countries",
            );
        }
        if slot_read(&frame.metrics, TableId::DivisionTemplates, hour) {
            self.template_hour = frame
                .complete::<tables::DivisionTemplates>()
                .is_ok()
                .then_some(hour);
        }
        let current_presentation = Presentations::new(frame, self.template_hour);
        let census = self.presentation.take();
        let previous_presentation = census.as_ref().map(Census::presentations);
        let live = std::mem::take(&mut self.live);
        let divisions = frame.complete::<tables::Divisions>().ok();
        if let Some(divisions) = divisions {
            let baseline = !self.available;
            self.available = true;
            let mut held = live.iter().peekable();
            for division in divisions {
                // Native reveal-skip copies are intel presentations, not divisions.
                if division.reveal_skip != 0 {
                    continue;
                }
                // Every division of the last census was seen then.
                while held.next_if(|id| **id < division.id).is_some() {}
                if held.next_if(|id| **id == division.id).is_none()
                    && self.seen.insert(division.id)
                    && !baseline
                {
                    let presentation = current_presentation.get(division.id);
                    // Division owner is the positive expeditionary owner, otherwise
                    // the recorded roster owner.
                    let raw = if (division.expeditionary_owner as i32) > 0 {
                        division.expeditionary_owner
                    } else {
                        division.roster_owner
                    };
                    let mut row = self.row(
                        division.id,
                        hour,
                        DivisionLogKind::Created,
                        "first_observed",
                        raw,
                        presentation.as_ref(),
                    );
                    if !labels {
                        row.tag = None;
                    }
                    row.after_hour = self
                        .previous_census
                        .filter(|last| last.checked_add(1) == Some(hour));
                    self.events.push(Event { row, presentation });
                }
            }
        } else {
            self.warn(
                hour,
                "Divisions unavailable or incomplete; appearances unknown",
            );
            self.previous_census = None;
        }
        let deletion_rows = frame.complete::<tables::DivisionEquipmentDeletions>().ok();
        let mut deaths = HashMap::<u64, Vec<_>>::new();
        for death in frame
            .complete::<tables::DestroyedDivisions>()
            .unwrap_or_default()
        {
            if death.hour <= hour
                && self.previous_hour.is_some_and(|last| death.hour >= last)
                && frame
                    .complete::<tables::LandEventCoverage>()
                    .is_ok_and(|coverage| {
                        coverage.iter().any(|span| {
                            span.complete
                                && span.from_hour < death.hour
                                && span.to_hour >= death.hour
                        })
                    })
            {
                deaths.entry(death.division).or_default().push(death);
            }
        }
        if let Some(deletions) = deletion_rows {
            self.losses_available = true;
            for deletion in deletions {
                if self.deletions.get(&deletion.division_handle) == Some(deletion) {
                    continue;
                }
                let presentation = previous_presentation
                    .as_ref()
                    .and_then(|previous| previous.get(deletion.division_handle))
                    .or_else(|| current_presentation.get(deletion.division_handle));
                let (kind, status) = disposition(deletion);
                let raw = if (deletion.expeditionary_owner as i32) > 0 {
                    deletion.expeditionary_owner
                } else {
                    deletion.country
                };
                let mut row = self.row(
                    deletion.division_handle,
                    hour,
                    kind,
                    status,
                    raw,
                    presentation.as_ref(),
                );
                if !labels {
                    row.tag = None;
                }
                if matches!(
                    deletion.cause,
                    Some(Cause::DivisionDestroyed | Cause::DivisionOverrun)
                ) {
                    let matched = deaths
                        .get(&deletion.division_handle)
                        .map(Vec::as_slice)
                        .unwrap_or_default();
                    if let [death] = matched {
                        row.hour = death.hour;
                        row.by = false;
                        row.province = Some(death.province);
                        row.place_hour = Some(death.hour);
                        row.place_at_event = true;
                        // The victim/country snapshot names are still dated by their supplying read.
                        row.killer_tag = labels
                            .then(|| self.links.get(death.killer_country).cloned())
                            .flatten();
                        row.killer_hour = row.killer_tag.as_ref().map(|_| hour);
                        row.killer_division = death
                            .killer_division
                            .filter(|id| *id != 0)
                            .map(|id| id.to_string());
                    } else if matched.len() > 1 {
                        self.warn(
                            hour,
                            "Ambiguous DestroyedDivisions interval; killer not attributed",
                        );
                    }
                }
                self.events.push(Event { row, presentation });
            }
            self.deletions = deletions
                .iter()
                .map(|row| (row.division_handle, row.clone()))
                .collect();
        } else {
            self.warn(
                hour,
                "DivisionEquipmentDeletions unavailable; removal causes and totals incomplete",
            );
        }
        if let Some(divisions) = divisions {
            let current: Vec<_> = divisions
                .iter()
                .filter(|row| row.reveal_skip == 0)
                .map(|row| row.id)
                .collect();
            let witnessed: HashSet<_> = deletion_rows
                .into_iter()
                .flatten()
                .map(|row| row.division_handle)
                .collect();
            // Both are ascending, so one pass finds the divisions gone since.
            let mut remaining = current.iter().peekable();
            let missing = live
                .iter()
                .filter(|id| {
                    while remaining.next_if(|current| current < id).is_some() {}
                    remaining.peek() != Some(id) && !witnessed.contains(*id)
                })
                .count();
            if missing > 0 {
                *self
                    .warnings
                    .entry((
                        hour,
                        "Division disappeared without a removal witness; no loss inferred".into(),
                    ))
                    .or_default() += missing;
            }
            self.live = current;
            self.previous_census = Some(hour);
            self.presentation = Some(Census::new(frame, self.template_hour, census));
        } else {
            self.live = live;
            self.presentation = census;
        }
        self.previous_hour = Some(hour);
    }
}
fn disposition(row: &DivisionEquipmentDeletionRow) -> (DivisionLogKind, &'static str) {
    let kind = match row.disposition {
        Disposition::Loss | Disposition::Returned => DivisionLogKind::Lost,
        _ => DivisionLogKind::Transfer,
    };
    (kind, row.event_name())
}
fn scan(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<Scan> {
    let mut scan = Scan::default();
    crate::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &TABLES,
        cancelled,
        |blocks| {
            for (block, frames) in blocks {
                check_cancel(cancelled)?;
                let mut decoder = FrameDecoder::for_tables(&TABLES);
                let frames = match frames {
                    Ok(frames) => frames,
                    Err(error) => {
                        scan.gap();
                        scan.warn(block.first_hour, format!("Unread replay block: {error}"));
                        continue;
                    }
                };
                for frame in frames {
                    check_cancel(cancelled)?;
                    let decoded = match decoder.advance(&frame) {
                        Ok(frame) => frame,
                        Err(error) => {
                            scan.gap();
                            scan.warn(frame.hour, format!("Unread replay frame: {error}"));
                            break;
                        }
                    };
                    validate_campaign(decoded, replay)?;
                    scan.observe(decoded);
                }
            }
            Ok(())
        },
    )?;
    scan.events.sort_by_key(|event| event.row.read_hour);
    Ok(scan)
}
type CachedLegacyNames = (
    Result<Arc<installed_content::division_names::Database>, String>,
    Vec<String>,
);

#[derive(Default)]
struct Cache {
    branch: Option<(u64, Arc<Scan>)>,
    catalogue: Option<Arc<Catalogue>>,
    names: Option<CachedLegacyNames>,
    icons: HashMap<String, Result<String, String>>,
    icon_bytes: usize,
    totals: Option<Totals>,
}
impl Cache {
    fn select_content(&mut self, cat: &Arc<Catalogue>) {
        if self
            .catalogue
            .as_ref()
            .is_none_or(|previous| !Arc::ptr_eq(previous, cat))
        {
            self.names = None;
            self.icons.clear();
            self.icon_bytes = 0;
            self.catalogue = Some(Arc::clone(cat));
        }
    }
}
struct Totals {
    key: String,
    reads: Vec<i32>,
    counts: Vec<[usize; 3]>,
}
fn totals_index(
    scan: &Scan,
    params: &DivisionLogTotalsParams,
    key: String,
    cancelled: &AtomicBool,
) -> io::Result<Totals> {
    let tags: Option<HashSet<_>> = params
        .countries
        .tags
        .as_ref()
        .map(|tags| tags.iter().map(String::as_str).collect());
    let templates: Option<HashSet<_>> = params
        .templates
        .as_ref()
        .map(|templates| templates.iter().map(String::as_str).collect());
    let mut membership: HashMap<&str, Vec<(i32, i32)>> = HashMap::new();
    for run in params.countries.membership.iter().flatten() {
        membership
            .entry(&run.tag)
            .or_default()
            .push((run.first_hour, run.last_hour));
    }
    let mut index = Totals {
        key,
        reads: Vec::new(),
        counts: vec![[0; 3]],
    };
    for event in &scan.events {
        check_cancel(cancelled)?;
        let row = &event.row;
        if row.hour < params.first_hour
            || templates.as_ref().is_some_and(|templates| {
                !row.template
                    .as_deref()
                    .is_some_and(|id| templates.contains(id))
            })
        {
            continue;
        }
        let selected = match row.tag.as_deref() {
            Some(tag) => {
                tags.as_ref().is_none_or(|tags| tags.contains(tag))
                    && (params.countries.membership.is_none()
                        || membership.get(tag).is_some_and(|runs| {
                            runs.iter().any(|(first, last)| {
                                *first <= row.country_hour && *last >= row.country_hour
                            })
                        }))
                    && !params.countries.unobserved.iter().any(|span| {
                        span.first_hour <= row.country_hour && span.last_hour >= row.country_hour
                    })
            }
            None => tags.is_none() && params.countries.membership.is_none(),
        };
        if !selected {
            continue;
        }
        let mut counts = *index.counts.last().expect("initial totals");
        counts[match row.kind {
            DivisionLogKind::Created => 0,
            DivisionLogKind::Lost => 1,
            DivisionLogKind::Transfer => 2,
        }] += 1;
        index.reads.push(row.read_hour);
        index.counts.push(counts);
    }
    Ok(index)
}
fn cached_scan(source: &Source, branch: u64, cancelled: &AtomicBool) -> io::Result<Arc<Scan>> {
    let state = source.state::<Cache>();
    let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((id, scan)) = &cache.branch
        && *id == branch
    {
        return Ok(Arc::clone(scan));
    }
    let scan = Arc::new(scan(source.replay_ref(), branch, cancelled)?);
    cache.branch = Some((branch, Arc::clone(&scan)));
    Ok(scan)
}
fn branch(source: &Source, id: &str) -> io::Result<u64> {
    let id = id
        .parse::<u64>()
        .map_err(|_| crate::invalid("Invalid division log branch"))?;
    source
        .replay_ref()
        .branches
        .iter()
        .any(|branch| branch.id == id)
        .then_some(id)
        .ok_or_else(|| crate::invalid("Branch not in this recording"))
}
fn windows(replay: &Replay, branch: u64) -> Vec<BattleWindow> {
    replay
        .index
        .iter()
        .filter(|block| block.branch == branch)
        .map(|block| BattleWindow {
            first_hour: block.first_hour,
            last_hour: block.last_hour,
        })
        .collect()
}
fn bounded<T: Serialize>(reply: T) -> io::Result<T> {
    json_len(&reply, MAX_PRODUCT_REPLY_BYTES).ok_or_else(|| {
        crate::invalid("Division log reply exceeds its transport budget; no events were omitted")
    })?;
    Ok(reply)
}
fn names(
    source: &Source,
    context: &LoadContext,
    warnings: &mut Vec<String>,
) -> Option<Arc<Catalogue>> {
    let cat = match context
        .content()
        .and_then(|options| catalogue::catalogue(source, options))
    {
        Ok(cat) => cat,
        Err(error) => {
            warnings.push(format!("Division names unavailable: {error}"));
            return None;
        }
    };
    let state = source.state::<Cache>();
    let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
    cache.select_content(&cat);
    Some(cat)
}
fn legacy_names(source: &Source, cat: &Arc<Catalogue>) -> CachedLegacyNames {
    let state = source.state::<Cache>();
    let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
    cache.select_content(cat);
    cache
        .names
        .get_or_insert_with(|| {
            // The global database is reached only by the historical operand path.
            // Cached army-name bytes need no database.
            match installed_content::division_names::read_directories::<installed_content::Diagnostic>(
                &cat.content.layers,
                &installed_content::division_names::DIRECTORIES,
            ) {
                Ok((database, notes)) => (
                    Ok(Arc::new(database)),
                    notes.into_iter().map(|(_, note)| note).collect(),
                ),
                Err(error) => (
                    Err(format!("Division name groups unavailable: {error}")),
                    Vec::new(),
                ),
            }
        })
        .clone()
}
fn icon_key(
    priority_unit: u32,
    custom: i32,
    unit: &[u8],
    exists: impl Fn(&str) -> Result<bool, String>,
) -> Result<String, String> {
    // The template icon forwards its i32 choice and tests nonnegative indices.
    // The unit-type selector rejects a null/invalid priority definition before
    // any custom lookup, then falls back if absent.
    if priority_unit == 0 {
        return Err(
            "Division priority unit is null or invalid; native template artwork is absent".into(),
        );
    }
    if custom >= 0 {
        let key = format!("GFX_div_templ_{custom}_large");
        if exists(&key)? {
            return Ok(key);
        }
    }
    let unit = std::str::from_utf8(unit).map_err(|_| "Division priority unit name is not UTF-8")?;
    if unit.is_empty() {
        return Err("Division priority unit name is empty".into());
    }
    let key = format!("GFX_unit_{unit}_icon_medium");
    if exists(&key)? {
        Ok(key)
    } else {
        Err(format!("Division sprite {key} is unavailable"))
    }
}
fn icon(
    source: &Source,
    cat: &Arc<Catalogue>,
    priority_unit: u32,
    custom: i32,
    unit: &[u8],
) -> Result<String, String> {
    let key = icon_key(priority_unit, custom, unit, |key| {
        cat.sprites.find(key).map(|sprite| sprite.is_some())
    })?;
    let state = source.state::<Cache>();
    let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
    cache.select_content(cat);
    if let Some(icon) = cache.icons.get(&key) {
        return icon.clone();
    }
    let sprite = cat
        .sprites
        .find(&key)?
        .ok_or_else(|| format!("Division sprite {key} is unavailable"))?;
    let result =
        catalogue::sprite_frame_png(&cat.content.layers, sprite, cat.gui_decode.clone()?, 0, 48)
            .map_err(|why| format!("Division sprite {key}: {why}"));
    let bytes = key.len()
        + result
            .as_ref()
            .map_or_else(|reason| reason.len(), |value| value.len());
    if bytes <= super::artwork::MAX_RETAINED_ARTWORK_BYTES {
        if cache.icon_bytes + bytes > super::artwork::MAX_RETAINED_ARTWORK_BYTES {
            cache.icons.clear();
            cache.icon_bytes = 0;
        }
        cache.icon_bytes += bytes;
        cache.icons.insert(key, result.clone());
    }
    result
}
fn named(
    source: &Source,
    event: &Event,
    content: Option<&Arc<Catalogue>>,
    warnings: &mut BTreeMap<String, usize>,
    division_name: bool,
) -> DivisionLogEntry {
    let mut row = event.row.clone();
    let Some(presentation) = &event.presentation else {
        return row;
    };
    let Some(cat) = content else {
        return row;
    };
    if division_name && let Some((priority_unit, custom, unit)) = &presentation.icon {
        match icon(source, cat, *priority_unit, *custom, unit) {
            Ok(icon) => {
                row.icon = Some(icon);
                row.icon_missing = None;
            }
            Err(reason) => {
                *warnings.entry(reason.clone()).or_default() += 1;
                row.icon_missing = Some(reason);
            }
        }
    }
    if let Some(status) = &presentation.status
        && division_name
    {
        let legacy = std::cell::OnceCell::new();
        let name =
            crate::unit_names::army_name(status, &cat.localisation, &cat.recorded_language, || {
                let (database, _) = legacy.get_or_init(|| legacy_names(source, cat));
                database.as_ref().map(Arc::as_ref).map_err(Clone::clone)
            })
            .and_then(|name| catalogue::plain_text(&name));
        if let Some((_, notes)) = legacy.get() {
            for note in notes {
                warnings.entry(note.clone()).or_insert(1);
            }
        }
        match name {
            Ok(name) => {
                row.name = Some(name);
                row.name_missing = None;
            }
            Err(reason) => {
                *warnings.entry(reason.clone()).or_default() += 1;
                row.name_missing = Some(reason);
            }
        }
    }
    if let Some(template) = &presentation.template {
        // An explicit display name precedes the localisation key.
        let name = if !template.name.is_empty() {
            stored_text(
                &cat.localisation,
                &cat.recorded_language,
                &template.name,
                "Division template name",
            )
        } else {
            template
                .name_key
                .require("Division template fallback key")
                .and_then(|key| {
                    std::str::from_utf8(key)
                        .map_err(|_| "Division template key is not UTF-8".into())
                })
                .and_then(|key| {
                    cat.localised(key, catalogue::View::Plain)
                        .ok_or_else(|| format!("Division template {key} localisation unavailable"))
                })
        }
        .and_then(|name| catalogue::plain_text(&name));
        match name {
            Ok(name) => {
                row.template_name = Some(name);
                row.template_missing = None;
            }
            Err(reason) => {
                *warnings.entry(reason.clone()).or_default() += 1;
                row.template_missing = Some(reason);
            }
        }
    }
    row
}
impl Product for DivisionLog {
    const KEY: &'static str = "division-log";
    const CONTENT_DEPENDENT: bool = true;
    type Params = DivisionLogParams;
    type Reply = DivisionLogReply;
    fn types() -> Vec<String> {
        vec![
            DivisionLogParams::decl(),
            DivisionLogKind::decl(),
            DivisionLogEntry::decl(),
            DivisionLogReply::decl(),
        ]
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = branch(source, &params.branch_id)?;
        let interval = windows(source.replay_ref(), branch)
            .get(params.window)
            .cloned()
            .ok_or_else(|| crate::invalid("Division log window not in recording"))?;
        let scan = cached_scan(source, branch, cancelled)?;
        let mut content_warnings = Vec::new();
        let content = names(source, context, &mut content_warnings);
        let mut refused = BTreeMap::new();
        let first = scan
            .events
            .partition_point(|event| event.row.read_hour < interval.first_hour);
        let last = scan
            .events
            .partition_point(|event| event.row.read_hour <= interval.last_hour);
        let mut rows = Vec::new();
        for event in &scan.events[first..last] {
            check_cancel(cancelled)?;
            rows.push(named(source, event, content.as_ref(), &mut refused, true));
        }
        content_warnings.extend(
            refused
                .into_iter()
                .map(|(reason, count)| format!("{reason} ({count})")),
        );
        let warnings = scan
            .warnings
            .iter()
            .filter(|((hour, _), _)| *hour >= interval.first_hour && *hour <= interval.last_hour)
            .map(|((hour, reason), count)| BattleWarning {
                hour: *hour,
                reason: reason.clone(),
                count: *count,
            })
            .collect();
        bounded(DivisionLogReply {
            window: params.window,
            interval,
            rows,
            warnings,
            content_warnings,
        })
    }
}
impl Product for DivisionLogManifest {
    const KEY: &'static str = "division-log-manifest";
    const CONTENT_DEPENDENT: bool = true;
    type Params = BranchParams;
    type Reply = DivisionLogManifestReply;
    fn types() -> Vec<String> {
        vec![
            DivisionLogTemplate::decl(),
            DivisionLogManifestReply::decl(),
        ]
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = branch(source, &params.branch_id)?;
        let scan = cached_scan(source, branch, cancelled)?;
        let mut content_warnings = Vec::new();
        let content = names(source, context, &mut content_warnings);
        let mut templates = BTreeMap::new();
        let mut refused = BTreeMap::new();
        let mut latest = BTreeMap::new();
        for event in &scan.events {
            check_cancel(cancelled)?;
            if let Some(id) = &event.row.template {
                latest.insert(id, event);
            }
        }
        for (id, event) in latest {
            templates.insert(
                id.clone(),
                DivisionLogTemplate {
                    id: id.clone(),
                    name: named(source, event, content.as_ref(), &mut refused, false).template_name,
                },
            );
        }
        content_warnings.extend(
            refused
                .into_iter()
                .map(|(reason, count)| format!("{reason} ({count})")),
        );
        bounded(DivisionLogManifestReply {
            windows: windows(source.replay_ref(), branch),
            available: scan.available,
            losses_available: scan.losses_available,
            templates: templates.into_values().collect(),
            content_warnings,
        })
    }
}
impl Product for DivisionLogTotals {
    const KEY: &'static str = "division-log-totals";
    const CONTENT_DEPENDENT: bool = false;
    type Params = DivisionLogTotalsParams;
    type Reply = DivisionLogTotalsReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.first_hour > params.last_hour {
            return Err("Invalid division log totals range");
        }
        Ok(())
    }
    fn types() -> Vec<String> {
        vec![
            DivisionLogTotalsParams::decl(),
            DivisionLogTotalsReply::decl(),
        ]
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        _: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let scan = cached_scan(source, branch(source, &params.branch_id)?, cancelled)?;
        let key = serde_json::to_string(&(
            &params.branch_id,
            params.first_hour,
            &params.countries,
            &params.templates,
        ))
        .map_err(|why| crate::invalid(why.to_string()))?;
        let state = source.state::<Cache>();
        let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
        if cache.totals.as_ref().is_none_or(|index| index.key != key) {
            cache.totals = Some(totals_index(&scan, params, key, cancelled)?);
        }
        let index = cache.totals.as_ref().expect("totals index");
        let [created, lost, transferred] = index.counts[index
            .reads
            .partition_point(|hour| *hour <= params.last_hour)];
        bounded(DivisionLogTotalsReply { created, lost, transferred,
            warnings: vec!["Totals count observed appearances and witnessed removals in the selected time; missed events are not zero.".into()] })
    }
}

#[cfg(test)]
#[path = "division_log/tests.rs"]
mod tests;
