//! Observed appearances and witnessed removals, never snapshot-inferred deaths.
//! Deletion clears expeditionary ownership; the capture preserves it. Recorded
//! dispositions distinguish reserve returns, merges and transfers. Destruction
//! and combat lifecycle supply killer context, joined only within the same drain.
//! First appearance has no creation date: its date is an upper bound.
use super::land_battles::LandBattleCountryFilter;
use dashboard_base::{
    catalogue,
    products::{
        BranchParams, Product,
        battles::{BattleWarning, BattleWindow},
    },
    source::Source,
};
use dashboard_core::config::LoadContext;
use dashboard_text::{
    catalogue::Catalogue,
    metadata::{LinkTags, check_cancel, slot_read},
    scripted_text::stored_text,
};
use replay::container::Replay;
use replay_data::{
    rows::DivisionDeletionDisposition as Disposition, rows::DivisionEquipmentDeletionRow,
    rows::DivisionRow, rows::DivisionStatusRow, rows::DivisionTemplateRow,
    rows::EquipmentDestructionCause as Cause, rows::NameOperands, rows::Recorded, rows::TableId,
    rows::tables,
};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};
use ts_rs::TS;
#[cfg(any(test, feature = "test-support"))]
use {dashboard_text::metadata::validate_campaign, replay_frame::frame::FrameDecoder};

pub mod block;
mod newest;
pub mod prepared;

pub struct DivisionLog;
pub struct DivisionLogManifest;
pub struct DivisionLogTotals;
/// The tables the division observer reads.
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

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
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

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DivisionLogReply {
    pub window: usize,
    pub interval: BattleWindow,
    pub rows: Vec<DivisionLogEntry>,
    pub warnings: Vec<BattleWarning>,
    pub content_warnings: Vec<String>,
}
impl dashboard_base::products::ReplyWarnings for DivisionLogReply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        &mut self.content_warnings
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DivisionLogTemplate {
    pub id: String,
    pub name: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DivisionLogManifestReply {
    pub windows: Vec<BattleWindow>,
    pub available: bool,
    pub losses_available: bool,
    pub templates: Vec<DivisionLogTemplate>,
    pub content_warnings: Vec<String>,
}
impl dashboard_base::products::ReplyWarnings for DivisionLogManifestReply {
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
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DivisionLogTotalsReply {
    pub created: Option<usize>,
    pub lost: Option<usize>,
    pub transferred: Option<usize>,
    pub warnings: Vec<String>,
}
impl dashboard_base::products::ReplyWarnings for DivisionLogTotalsReply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        &mut self.warnings
    }
}

/// The recorded naming operands of one division name.
type NamingOperands = (i32, i32, u8, u8, Vec<u8>, Vec<u8>);

// Only the operands consumed by the production presentation builders survive.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ArmyName {
    division: u64,
    name: Result<Option<Vec<u8>>, ()>,
    naming: Result<Option<NamingOperands>, ()>,
}
impl ArmyName {
    fn new(row: &DivisionStatusRow) -> Self {
        Self {
            division: row.division,
            name: match &row.name {
                Recorded::Value(value) => Ok(value.clone()),
                Recorded::NotRecorded => Err(()),
            },
            naming: match &row.naming {
                Recorded::Value(value) => Ok(value.as_ref().map(|v| {
                    (
                        v.kind,
                        v.order,
                        v.ordered,
                        v.programmatic,
                        v.custom.clone(),
                        v.group.clone(),
                    )
                })),
                Recorded::NotRecorded => Err(()),
            },
        }
    }
    fn matches(&self, row: &DivisionStatusRow) -> bool {
        let name = match (&self.name, &row.name) {
            (Ok(old), Recorded::Value(new)) => old == new,
            (Err(()), Recorded::NotRecorded) => true,
            _ => false,
        };
        name && match (&self.naming, &row.naming) {
            (
                Ok(Some((kind, order, ordered, programmatic, custom, group))),
                Recorded::Value(Some(new)),
            ) => {
                *kind == new.kind
                    && *order == new.order
                    && *ordered == new.ordered
                    && *programmatic == new.programmatic
                    && custom == &new.custom
                    && group == &new.group
            }
            (Ok(None), Recorded::Value(None)) | (Err(()), Recorded::NotRecorded) => true,
            _ => false,
        }
    }
    fn status(&self) -> DivisionStatusRow {
        DivisionStatusRow {
            division: self.division,
            name: match &self.name {
                Ok(value) => Recorded::Value(value.clone()),
                Err(()) => Recorded::NotRecorded,
            },
            naming: match &self.naming {
                Ok(value) => Recorded::Value(value.as_ref().map(
                    |(kind, order, ordered, programmatic, custom, group)| NameOperands {
                        kind: *kind,
                        order: *order,
                        ordered: *ordered,
                        programmatic: *programmatic,
                        custom: custom.clone(),
                        group: group.clone(),
                    },
                )),
                Err(()) => Recorded::NotRecorded,
            },
            ..Default::default()
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct TemplateName {
    literal: Vec<u8>,
    key: Result<Vec<u8>, ()>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Presentation {
    hour: i32,
    division: u64,
    province: u32,
    template_id: u64,
    status: Option<ArmyName>,
    template: Option<TemplateName>,
    template_hour: Option<i32>,
    icon: Option<(u32, i32, Vec<u8>)>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Event {
    row: DivisionLogEntry,
    presentation: Option<Presentation>,
}
// Rows hold unique ascending keys, so a large simultaneous deployment is looked
// up without indexing every hour.
struct Presentations<'a> {
    hour: i32,
    divisions: &'a [DivisionRow],
    names: &'a [DivisionStatusRow],
    templates: &'a [DivisionTemplateRow],
    icons: &'a [replay_data::rows::DivisionTemplateIconRow],
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
            names: frame
                .complete::<tables::DivisionStatus>()
                .unwrap_or_default(),
            templates: frame
                .complete::<tables::DivisionTemplates>()
                .unwrap_or_default(),
            icons: frame
                .complete::<tables::DivisionTemplateIcons>()
                .unwrap_or_default(),
        }
    }
    fn get(&self, id: u64, held: Option<ArmyName>) -> Option<Presentation> {
        let division = keyed(self.divisions, id, |row| row.id)?;
        let status = keyed(self.names, id, |row| row.division).map(|row| {
            held.filter(|name| name.matches(row))
                .unwrap_or_else(|| ArmyName::new(row))
        });
        let template =
            keyed(self.templates, division.template, |row| row.template).map(|row| TemplateName {
                literal: row.name.clone(),
                key: match &row.name_key {
                    Recorded::Value(value) => Ok(value.clone()),
                    Recorded::NotRecorded => Err(()),
                },
            });
        let icon = keyed(self.icons, division.template, |row| row.template).map(|row| {
            (
                row.priority_unit,
                row.custom_icon,
                row.priority_unit_name.clone(),
            )
        });
        Some(Presentation {
            hour: self.hour,
            division: division.id,
            province: division.province,
            template_id: division.template,
            status,
            template,
            template_hour: self.template_hour,
            icon,
        })
    }
}
/// Last complete census, retained through unavailable reads and branch switches.
/// It contains presentation operands, never raw table rows.
#[derive(Serialize, Deserialize)]
struct Census {
    rows: Vec<Presentation>,
    // All operands share the last complete census's supplying dates. Advancing
    // those dates does not require copying the unchanged naming operands.
    hour: i32,
    template_hour: Option<i32>,
}
impl Census {
    fn new(frame: &DecodedFrame, template_hour: Option<i32>, held: Option<Census>) -> Self {
        let current = Presentations::new(frame, template_hour);
        let mut held = held
            .map_or_else(Vec::new, |census| census.rows)
            .into_iter()
            .peekable();
        let rows = current
            .divisions
            .iter()
            .map(|division| {
                while held.next_if(|old| old.division < division.id).is_some() {}
                let name = held
                    .next_if(|old| old.division == division.id)
                    .and_then(|old| old.status);
                current
                    .get(division.id, name)
                    .expect("complete division census")
            })
            .collect();
        Self {
            rows,
            hour: frame.hour,
            template_hour,
        }
    }
    fn get(&self, id: u64) -> Option<Presentation> {
        keyed(&self.rows, id, |row| row.division).map(|row| Presentation {
            hour: self.hour,
            template_hour: self.template_hour,
            ..row.clone()
        })
    }
}
fn deletion_signature(row: &DivisionEquipmentDeletionRow) -> (u32, u32, u8, Option<u8>) {
    (
        row.country,
        row.expeditionary_owner,
        row.disposition as u8,
        row.cause.map(|cause| cause as u8),
    )
}
#[derive(Default, Serialize, Deserialize)]
pub struct Scan {
    /// The recording's format, which dates its daily tables
    /// ([`dashboard_text::metadata::slot_read`]).
    #[serde(skip)]
    format: replay_data::format::FormatVersion,
    events: Vec<Event>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_set")]
    seen: HashSet<u64>,
    /// The last census's divisions, ascending.
    live: Vec<u64>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    deletions: HashMap<u64, (u32, u32, u8, Option<u8>)>,
    links: LinkTags,
    previous_hour: Option<i32>,
    previous_census: Option<i32>,
    presentation: Option<Census>,
    template_hour: Option<i32>,
    available: bool,
    losses_available: bool,
    #[serde(with = "dashboard_core::map_pairs")]
    warnings: BTreeMap<(i32, String), usize>,
    counts: prepared::Counts,
    /// While a block's extract follows the block alone ([`block`]): each
    /// division absent from the last census, by the index of the event it
    /// opened and its id. The extract cannot know which of them earlier
    /// blocks saw; its fold keeps the events of those the carried scan never
    /// saw, as this scan's own record of seen divisions would.
    #[serde(skip)]
    candidates: Option<Vec<(usize, u64)>>,
    /// While following, a census met without a census before it in the
    /// block, which the carried scan may have had.
    #[serde(skip)]
    irregular: bool,
    #[cfg(test)]
    #[serde(skip)]
    work: dashboard_base::prepared::observer_tests::Work,
}
/// The warning of an hour whose country tables were not read completely.
const IDENTITY_UNAVAILABLE: &str =
    "Country identity unavailable; division events retain unknown countries";
impl Scan {
    fn warn(&mut self, hour: i32, reason: impl Into<String>) {
        *self.warnings.entry((hour, reason.into())).or_default() += 1;
    }
    /// Whether a division absent from the last census is seen for the
    /// first time on the branch, and so logged unless this census is the
    /// first available one (`baseline`). While a block's extract follows,
    /// every such division outside a baseline is a candidate its fold
    /// decides.
    fn first_seen(&mut self, id: u64, baseline: bool) -> bool {
        let next = self.events.len();
        match self.candidates.as_mut() {
            Some(candidates) => {
                if baseline {
                    self.irregular = true;
                    return false;
                }
                candidates.push((next, id));
                true
            }
            None => self.seen.insert(id) && !baseline,
        }
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
            template: presentation.map(|value| value.template_id.to_string()),
            template_name: None,
            template_missing: Some("Template name not recorded at this observation".into()),
            template_hour: presentation.and_then(|value| value.template_hour),
            province: presentation.map(|value| value.province),
            place_hour: presentation.map(|value| value.hour),
            place_at_event: kind == DivisionLogKind::Created,
            killer_tag: None,
            killer_hour: None,
            killer_division: None,
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    fn observe(&mut self, frame: &DecodedFrame) {
        self.observe_with_changes(frame, None);
    }
    fn observe_with_changes(
        &mut self,
        frame: &DecodedFrame,
        changes: Option<&dashboard_base::prepared::frame_changes::FrameChanges>,
    ) {
        let changed =
            |table: TableId| changes.is_none_or(|changes| changes.rows_changed[table.index()]);
        let divisions_changed = changed(TableId::Divisions);
        let presentation_changed = [
            TableId::Divisions,
            TableId::DivisionStatus,
            TableId::DivisionTemplates,
            TableId::DivisionTemplateIcons,
        ]
        .into_iter()
        .any(changed);
        #[cfg(test)]
        let first_event = self.events.len();
        let hour = frame.hour;
        if self
            .previous_hour
            .is_some_and(|last| last.checked_add(1) != Some(hour))
        {
            self.gap();
        }
        let labels = self.links.update(frame).is_ok();
        if !labels {
            self.warn(hour, IDENTITY_UNAVAILABLE);
        }
        if changes.map_or_else(
            || {
                slot_read(
                    self.format,
                    &frame.metrics,
                    TableId::DivisionTemplates,
                    hour,
                )
            },
            |changes| changes.slot_read[TableId::DivisionTemplates.index()],
        ) {
            self.template_hour = frame
                .complete::<tables::DivisionTemplates>()
                .is_ok()
                .then_some(hour);
        }
        let current_presentation = Presentations::new(frame, self.template_hour);
        let census = self.presentation.take();
        let live = if divisions_changed {
            std::mem::take(&mut self.live)
        } else {
            Vec::new()
        };
        let divisions = frame.complete::<tables::Divisions>().ok();
        if let Some(divisions) = divisions {
            let baseline = !self.available;
            self.available = true;
            if divisions_changed {
                #[cfg(test)]
                {
                    self.work.row_visits += divisions.len();
                }
                let mut held = live.iter().peekable();
                for division in divisions {
                    // Native reveal-skip copies are intel presentations, not divisions.
                    if division.reveal_skip != 0 {
                        continue;
                    }
                    // Every division of the last census was seen then.
                    while held.next_if(|id| **id < division.id).is_some() {}
                    if held.next_if(|id| **id == division.id).is_none()
                        && self.first_seen(division.id, baseline)
                    {
                        let presentation = current_presentation.get(division.id, None);
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
            }
        } else {
            self.warn(
                hour,
                "Divisions unavailable or incomplete; appearances unknown",
            );
            self.previous_census = None;
        }
        let deletion_rows = frame.complete::<tables::DivisionEquipmentDeletions>().ok();
        if changed(TableId::DivisionEquipmentDeletions) {
            let mut deaths = HashMap::<u64, Vec<_>>::new();
            for death in frame
                .complete::<tables::DestroyedDivisions>()
                .unwrap_or_default()
            {
                #[cfg(test)]
                {
                    self.work.row_visits += 1;
                }
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
                    #[cfg(test)]
                    {
                        self.work.row_visits += 1;
                    }
                    if self.deletions.get(&deletion.division_handle)
                        == Some(&deletion_signature(deletion))
                    {
                        continue;
                    }
                    let presentation = census
                        .as_ref()
                        .and_then(|previous| previous.get(deletion.division_handle))
                        .or_else(|| current_presentation.get(deletion.division_handle, None));
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
                    .map(|row| (row.division_handle, deletion_signature(row)))
                    .collect();
            }
        }
        if deletion_rows.is_none() {
            self.warn(
                hour,
                "DivisionEquipmentDeletions unavailable; removal causes and totals incomplete",
            );
        }
        if let Some(divisions) = divisions.filter(|_| divisions_changed) {
            #[cfg(test)]
            {
                self.work.row_visits +=
                    divisions.len() + live.len() + deletion_rows.map_or(0, <[_]>::len);
            }
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
        } else if divisions_changed {
            self.live = live;
        }
        if divisions.is_some() {
            self.previous_census = Some(hour);
            self.presentation = if presentation_changed || census.is_none() {
                #[cfg(test)]
                {
                    self.work.row_visits += current_presentation.divisions.len();
                }
                Some(Census::new(frame, self.template_hour, census))
            } else {
                census.map(|mut census| {
                    census.hour = hour;
                    census.template_hour = self.template_hour;
                    census
                })
            };
        } else {
            self.presentation = census;
        }
        self.previous_hour = Some(hour);
        #[cfg(test)]
        {
            self.work.emitted += self.events.len() - first_event;
        }
    }
}
fn disposition(row: &DivisionEquipmentDeletionRow) -> (DivisionLogKind, &'static str) {
    let kind = match row.disposition {
        Disposition::Loss | Disposition::Returned => DivisionLogKind::Lost,
        _ => DivisionLogKind::Transfer,
    };
    (kind, row.event_name())
}
#[cfg(any(test, feature = "test-support"))]
fn scan(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<Scan> {
    let mut scan = Scan {
        format: replay.format,
        ..Scan::default()
    };
    dashboard_core::blocks::scan_blocks(
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
    Result<Arc<installed_content_division_names::division_names::Database>, String>,
    Vec<String>,
);

#[derive(Default)]
struct Cache {
    #[cfg(any(test, feature = "test-support"))]
    branch: Option<(u64, Arc<Scan>)>,
    catalogue: Option<Arc<Catalogue>>,
    names: Option<CachedLegacyNames>,
    icons: HashMap<String, Result<String, String>>,
    icon_bytes: usize,
    #[cfg(any(test, feature = "test-support"))]
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
#[cfg(any(test, feature = "test-support"))]
struct Totals {
    key: String,
    reads: Vec<i32>,
    counts: Vec<[usize; 3]>,
}
fn kind_index(kind: DivisionLogKind) -> usize {
    match kind {
        DivisionLogKind::Created => 0,
        DivisionLogKind::Lost => 1,
        DivisionLogKind::Transfer => 2,
    }
}
struct Selection<'a> {
    params: &'a DivisionLogTotalsParams,
    tags: Option<HashSet<&'a str>>,
    templates: Option<HashSet<&'a str>>,
    membership: HashMap<&'a str, Vec<(i32, i32)>>,
}
impl<'a> Selection<'a> {
    fn new(params: &'a DivisionLogTotalsParams) -> Self {
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
        Self {
            params,
            tags,
            templates,
            membership,
        }
    }
    fn country_matches(&self, tag: Option<&str>, hour: i32) -> bool {
        match tag {
            Some(tag) => {
                self.tags.as_ref().is_none_or(|tags| tags.contains(tag))
                    && (self.params.countries.membership.is_none()
                        || self.membership.get(tag).is_some_and(|runs| {
                            runs.iter()
                                .any(|(first, last)| *first <= hour && *last >= hour)
                        }))
                    && !self
                        .params
                        .countries
                        .unobserved
                        .iter()
                        .any(|span| span.first_hour <= hour && span.last_hour >= hour)
            }
            None => self.tags.is_none() && self.params.countries.membership.is_none(),
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    fn matches(&self, row: &DivisionLogEntry) -> bool {
        row.hour >= self.params.first_hour
            && self.templates.as_ref().is_none_or(|templates| {
                row.template
                    .as_deref()
                    .is_some_and(|id| templates.contains(id))
            })
            && self.country_matches(row.tag.as_deref(), row.country_hour)
    }
}
fn totals_reply([created, lost, transferred]: [usize; 3]) -> io::Result<DivisionLogTotalsReply> {
    bounded(DivisionLogTotalsReply { created: Some(created), lost: Some(lost), transferred: Some(transferred),
        warnings: vec!["Totals count observed appearances and witnessed removals in the selected time; missed events are not zero.".into()] })
}
#[cfg(any(test, feature = "test-support"))]
fn totals_index(
    scan: &Scan,
    params: &DivisionLogTotalsParams,
    key: String,
    cancelled: &AtomicBool,
) -> io::Result<Totals> {
    let selection = Selection::new(params);
    let mut index = Totals {
        key,
        reads: Vec::new(),
        counts: vec![[0; 3]],
    };
    for event in &scan.events {
        check_cancel(cancelled)?;
        let row = &event.row;
        if !selection.matches(row) {
            continue;
        }
        let mut counts = *index.counts.last().expect("initial totals");
        counts[kind_index(row.kind)] += 1;
        index.reads.push(row.read_hour);
        index.counts.push(counts);
    }
    Ok(index)
}
#[cfg(any(test, feature = "test-support"))]
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
        .map_err(|_| dashboard_core::invalid("Invalid division log branch"))?;
    source
        .replay_ref()
        .branches
        .iter()
        .any(|branch| branch.id == id)
        .then_some(id)
        .ok_or_else(|| dashboard_core::invalid("Branch not in this recording"))
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
    dashboard_base::products::check_reply_budget(&reply).map_err(|_| {
        dashboard_core::invalid(
            "Division log reply exceeds its transport budget; no events were omitted",
        )
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
            match installed_content_division_names::division_names::read_directories::<
                installed_content_core::Diagnostic,
            >(
                &cat.content.layers,
                &installed_content_division_names::division_names::DIRECTORIES,
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
/// A division template's artwork as the division log draws it: its custom
/// icon, else its priority sub-unit's, as a PNG data URL kept per content binding.
pub fn template_icon(
    source: &Source,
    cat: &Arc<Catalogue>,
    row: &replay_data::rows::DivisionTemplateIconRow,
) -> Result<String, String> {
    icon(source, cat, row.priority_unit, row.custom_icon, &row.priority_unit_name)
}

/// The content stack's division name groups, read once per content binding,
/// and the notes their read made.
pub fn division_name_groups(
    source: &Source,
    cat: &Arc<Catalogue>,
) -> (
    Result<Arc<installed_content_division_names::division_names::Database>, String>,
    Vec<String>,
) {
    legacy_names(source, cat)
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
    if bytes <= dashboard_base::products::artwork::MAX_RETAINED_ARTWORK_BYTES {
        if cache.icon_bytes + bytes > dashboard_base::products::artwork::MAX_RETAINED_ARTWORK_BYTES
        {
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
        let name = dashboard_text::unit_names::army_name(
            &status.status(),
            &cat.localisation,
            &cat.recorded_language,
            || {
                let (database, _) = legacy.get_or_init(|| legacy_names(source, cat));
                database.as_ref().map(Arc::as_ref).map_err(Clone::clone)
            },
        )
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
        let name = if !template.literal.is_empty() {
            stored_text(
                &cat.localisation,
                &cat.recorded_language,
                &template.literal,
                "Division template name",
            )
        } else {
            template
                .key
                .as_ref()
                .map_err(|()| "Division template fallback key not recorded".to_owned())
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
impl Scan {
    fn reply(
        &self,
        source: &Source,
        params: &DivisionLogParams,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<DivisionLogReply> {
        let branch = branch(source, &params.branch_id)?;
        let interval = windows(source.replay_ref(), branch)
            .get(params.window)
            .cloned()
            .ok_or_else(|| dashboard_core::invalid("Division log window not in recording"))?;
        let mut content_warnings = Vec::new();
        let content = names(source, context, &mut content_warnings);
        let mut refused = BTreeMap::new();
        let first = self
            .events
            .partition_point(|event| event.row.read_hour < interval.first_hour);
        let last = self
            .events
            .partition_point(|event| event.row.read_hour <= interval.last_hour);
        let mut rows = Vec::new();
        for event in &self.events[first..last] {
            check_cancel(cancelled)?;
            rows.push(named(source, event, content.as_ref(), &mut refused, true));
        }
        content_warnings.extend(
            refused
                .into_iter()
                .map(|(reason, count)| format!("{reason} ({count})")),
        );
        let warnings = self
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
    fn manifest(
        &self,
        source: &Source,
        params: &BranchParams,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<DivisionLogManifestReply> {
        let branch = branch(source, &params.branch_id)?;
        let mut content_warnings = Vec::new();
        let content = names(source, context, &mut content_warnings);
        let mut templates = BTreeMap::new();
        let mut refused = BTreeMap::new();
        let mut latest = BTreeMap::new();
        for event in &self.events {
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
            available: self.available,
            losses_available: self.losses_available,
            templates: templates.into_values().collect(),
            content_warnings,
        })
    }
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
    #[cfg(any(test, feature = "test-support"))]
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = branch(source, &params.branch_id)?;
        let scan = cached_scan(source, branch, cancelled)?;
        scan.reply(source, params, context, cancelled)
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
    #[cfg(any(test, feature = "test-support"))]
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = branch(source, &params.branch_id)?;
        let scan = cached_scan(source, branch, cancelled)?;
        scan.manifest(source, params, context, cancelled)
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
    #[cfg(any(test, feature = "test-support"))]
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
        .map_err(|why| dashboard_core::invalid(why.to_string()))?;
        let state = source.state::<Cache>();
        let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
        if cache.totals.as_ref().is_none_or(|index| index.key != key) {
            cache.totals = Some(totals_index(&scan, params, key, cancelled)?);
        }
        let index = cache.totals.as_ref().expect("totals index");
        let [created, lost, transferred] = index.counts[index
            .reads
            .partition_point(|hour| *hour <= params.last_hour)];
        totals_reply([created, lost, transferred])
    }
}

#[cfg(test)]
#[path = "division_log/tests.rs"]
mod tests;
