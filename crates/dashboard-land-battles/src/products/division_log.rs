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
    template_facts::{TemplateNaming, display_naming},
};
use replay::campaign::Replay;
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
use dashboard_text::metadata::validate_campaign;

pub mod columnar;
pub mod emit;
mod newest;
pub mod prepared;
pub mod served;

/// The prepared family publishing the division log.
pub const FAMILY: &str = "divisions";

/// The template read hour a walk holds before its first read of the templates, which only the
/// ordered scan knows: its presentations are dated with it until then, and the scan dates them.
const INHERITED: i32 = i32::MIN;

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
#[derive(replay_spool::Spooled)]
pub enum DivisionLogKind {
    Created,
    Lost,
    Transfer,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[derive(replay_spool::Spooled)]
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
    /// The first hour these totals count from when hours the recording lost lie in the range:
    /// totals restart after them, so they count from there to its last hour.
    pub since: Option<i32>,
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
#[derive(replay_spool::Spooled)]
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
/// The shared template display naming at the observation, or why it was unavailable.
#[derive(Clone, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
struct TemplateName(Result<TemplateNaming, String>);
#[derive(Clone, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
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
#[derive(replay_spool::Spooled)]
struct Event {
    row: DivisionLogEntry,
    presentation: Option<Presentation>,
}
// Rows hold unique ascending keys, so a large simultaneous deployment is looked
// up without indexing every hour.
struct Presentations<'a> {
    frame: &'a DecodedFrame,
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
            frame,
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
        let template = keyed(self.templates, division.template, |row| row.template)
            .map(|_| TemplateName(display_naming(self.frame, division.template)));
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
/// What a columnar walk leaves the ordered scan to date and name: the template read hour at
/// the last census, the hour the census last built its presentations, and the names a walk
/// that built them before its end gave the templates they name.
#[derive(Default, Serialize, Deserialize)]
struct Walked {
    census_template: Option<i32>,
    built: Option<i32>,
    names: HashMap<u64, Option<TemplateName>>,
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
    /// While hours a columnar walk projected are kept ([`columnar`]): what only earlier
    /// walks know.
    #[serde(default)]
    walked: Walked,
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
    /// first available one (`baseline`).
    fn first_seen(&mut self, id: u64, baseline: bool) -> bool {
        self.seen.insert(id) && !baseline
    }
    fn gap(&mut self) {
        self.previous_hour = None;
        self.previous_census = None;
        self.template_hour = None;
    }
    /// Hours from `first` on were lost from the recording: what it read before goes no further,
    /// and every total restarts after them.
    pub fn lost(&mut self, first: i32) -> io::Result<()> {
        self.gap();
        self.counts.restart(first)
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
        entry(&self.links, id, hour, kind, status, raw, presentation)
    }

    /// The hour a columnar walk projected ([`columnar`]), kept as
    /// [`Self::observe_with_changes`] keeps the hour it reads: a candidate the branch saw
    /// before, or met by its first census, is no first sighting, and the dates only earlier
    /// hours know are dated.
    pub fn observe_projected(&mut self, projected: columnar::Hour) -> io::Result<()> {
        let hour = projected.hour;
        if self
            .previous_hour
            .is_some_and(|last| last.checked_add(1) != Some(hour))
        {
            self.gap();
        }
        let census_template = self.walked.census_template;
        if let Some(complete) = projected.template_read {
            self.template_hour = complete.then_some(hour);
        }
        let template = self.template_hour;
        let baseline = !self.available;
        let first = self.events.len();
        for columnar::Projected {
            mut event,
            candidate,
            restored,
        } in projected.events
        {
            if let Some(id) = candidate {
                if !(self.seen.insert(id) && !baseline) {
                    continue;
                }
                event.row.after_hour = self
                    .previous_census
                    .filter(|last| last.checked_add(1) == Some(hour));
            }
            // A presentation of the hour dates its template read now; one of the last census,
            // as at that census.
            let census = event
                .presentation
                .as_ref()
                .is_some_and(|presentation| presentation.hour != hour);
            let read = if census { census_template } else { template };
            let date = |at: &mut Option<i32>| {
                if *at == Some(INHERITED) {
                    *at = read;
                }
            };
            date(&mut event.row.template_hour);
            if let Some(presentation) = &mut event.presentation {
                date(&mut presentation.template_hour);
                // Named from a restart's census its walk took for the hour before's, which an
                // earlier hour built: named as that build named it.
                if restored.is_some_and(|taken| self.walked.built != Some(taken))
                    && let Some(name) = self.walked.names.get(&presentation.template_id)
                {
                    presentation.template = name.clone();
                }
            }
            self.events.push(event);
        }
        if projected.census {
            self.available = true;
            self.previous_census = Some(hour);
            self.walked.census_template = self.template_hour;
        } else {
            self.previous_census = None;
        }
        if projected.built {
            self.walked.built = Some(hour);
        }
        if let Some(names) = projected.names {
            self.walked.names = names;
        }
        self.losses_available |= projected.losses;
        for (reason, count) in projected.warnings {
            *self.warnings.entry((hour, reason)).or_default() += count;
        }
        self.previous_hour = Some(hour);
        self.counts.observe(hour, &self.events[first..])
    }
}

/// One division log row of `id` at `hour`, its country through `links`.
fn entry(
    links: &LinkTags,
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
        tag: links.get(raw).cloned(),
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
impl Scan {
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
        // Template naming also reads the composition country's tag.
        let presentation_changed = [
            TableId::Divisions,
            TableId::DivisionStatus,
            TableId::DivisionTemplates,
            TableId::DivisionTemplateIcons,
            TableId::Countries,
            TableId::CountryLinks,
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
                    let decoded = match Ok::<&DecodedFrame, std::io::Error>(&frame.decoded) {
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
pub type DivisionNameGroups = (
    Result<Arc<installed_content_division_names::division_names::Database>, String>,
    Vec<String>,
);

#[derive(Default)]
struct Cache {
    #[cfg(any(test, feature = "test-support"))]
    branch: Option<(u64, Arc<Scan>)>,
    catalogue: Option<Arc<Catalogue>>,
    names: Option<DivisionNameGroups>,
    /// Each template icon drawn for this content, by its operands.
    icons: HashMap<IconOperands, catalogue::Drawn<Result<String, String>>>,
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
    bounded(DivisionLogTotalsReply { created: Some(created), lost: Some(lost), transferred: Some(transferred), since: None,
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
/// The catalogue division names are drawn from, and why it is unavailable.
pub type Names = (Option<Arc<Catalogue>>, Vec<String>);

/// [`Names`] as `context` gives them.
pub fn names_content(source: &Source, context: &LoadContext) -> Names {
    let mut warnings = Vec::new();
    let catalogue = names(source, context, &mut warnings);
    (catalogue, warnings)
}

/// The catalogue of `names`, selected for the source's name caches.
fn selected(source: &Source, names: &Names) -> Option<Arc<Catalogue>> {
    let catalogue = names.0.as_ref()?;
    let state = source.state::<Cache>();
    let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
    cache.select_content(catalogue);
    Some(Arc::clone(catalogue))
}
/// The content stack's division name groups, read once per content binding,
/// and the notes their read made.
pub fn division_name_groups(source: &Source, cat: &Arc<Catalogue>) -> DivisionNameGroups {
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
/// icon, else its priority sub-unit's, as an artwork URL kept per content binding.
pub fn template_icon(
    source: &Source,
    cat: &Arc<Catalogue>,
    row: &replay_data::rows::DivisionTemplateIconRow,
) -> Result<String, String> {
    icon(
        source,
        cat,
        row.priority_unit,
        row.custom_icon,
        &row.priority_unit_name,
    )
}

/// [`template_icon`] from a row's drawing operands.
pub fn template_icon_of(
    source: &Source,
    cat: &Arc<Catalogue>,
    priority_unit: u32,
    custom: i32,
    unit: &[u8],
) -> Result<String, String> {
    icon(source, cat, priority_unit, custom, unit)
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
/// What a template's icon is drawn from: its priority unit, custom icon index and the priority
/// unit's name, the operands of the sprite choice.
type IconOperands = (u32, i32, Vec<u8>);

fn icon(
    source: &Source,
    cat: &Arc<Catalogue>,
    priority_unit: u32,
    custom: i32,
    unit: &[u8],
) -> Result<String, String> {
    let state = source.state::<Cache>();
    let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
    cache.select_content(cat);
    let operands: IconOperands = (priority_unit, custom, unit.to_vec());
    if let Some(icon) = cache.icons.get(&operands) {
        return icon.get().clone();
    }
    // The choice between the custom template sprite and the unit's sprite is made against the
    // catalogue drawing it: when the shown one cannot draw its choice, the recording's own
    // choice is made again and drawn.
    let drawn = catalogue::draw(
        || format!("division template icon ({})", String::from_utf8_lossy(unit)),
        cat,
        |cat| template_sprite(cat, priority_unit, custom, unit),
    );
    let result = drawn.get().clone();
    cache.icons.insert(operands, drawn);
    result
}

/// A template's icon drawn through `cat`: the sprite [`icon_key`] chooses by what `cat`
/// declares, rendered at the division log's edge.
fn template_sprite(
    cat: &Catalogue,
    priority_unit: u32,
    custom: i32,
    unit: &[u8],
) -> Result<String, String> {
    let key = icon_key(priority_unit, custom, unit, |key| {
        cat.sprites.find(key).map(|sprite| sprite.is_some())
    })?;
    let sprite = cat
        .sprites
        .find(&key)?
        .ok_or_else(|| format!("Division sprite {key} is unavailable"))?;
    let decode = cat.gui_decode.clone()?;
    catalogue::sprite_frame_png(&cat.content.layers, sprite, decode, 0, 48)
        .map_err(|why| format!("Division sprite {key}: {why}"))
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
        let _cpu = ::replay_frame::cpu::scope("division log icon", None);
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
        let _cpu = ::replay_frame::cpu::scope("division log army name", None);
        let groups = std::cell::OnceCell::new();
        let name = dashboard_text::unit_names::army_name(
            &status.status(),
            &cat.localisation,
            &cat.recorded_language,
            || {
                let (database, _) = groups.get_or_init(|| division_name_groups(source, cat));
                database.as_ref().map(Arc::as_ref).map_err(Clone::clone)
            },
        )
        .and_then(|name| catalogue::plain_text(&name));
        if let Some((_, notes)) = groups.get() {
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
    if let Some(TemplateName(naming)) = &presentation.template {
        let _cpu = ::replay_frame::cpu::scope("division log template name", None);
        let name = naming
            .clone()
            .and_then(|naming| naming.localise(&cat.localisation))
            .and_then(|name| catalogue::plain_text(&name))
            .map_err(|why| format!("Template name unavailable: {why}"));
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
        self.reply_named(source, params, &names_content(source, context), cancelled)
    }
    /// [`Self::reply`] naming from `names`, resolved before.
    fn reply_named(
        &self,
        source: &Source,
        params: &DivisionLogParams,
        names: &Names,
        cancelled: &AtomicBool,
    ) -> io::Result<DivisionLogReply> {
        let branch = branch(source, &params.branch_id)?;
        let interval = windows(source.replay_ref(), branch)
            .get(params.window)
            .cloned()
            .ok_or_else(|| dashboard_core::invalid("Division log window not in recording"))?;
        let mut content_warnings = names.1.clone();
        let content = selected(source, names);
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
    #[cfg(any(test, feature = "test-support"))]
    fn manifest(
        &self,
        source: &Source,
        params: &BranchParams,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<DivisionLogManifestReply> {
        self.manifest_named(source, params, &names_content(source, context), cancelled)
    }
    /// [`Self::manifest`] naming from `names`, resolved before.
    fn manifest_named(
        &self,
        source: &Source,
        params: &BranchParams,
        names: &Names,
        cancelled: &AtomicBool,
    ) -> io::Result<DivisionLogManifestReply> {
        let branch = branch(source, &params.branch_id)?;
        let mut content_warnings = names.1.clone();
        let content = selected(source, names);
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
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
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
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
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
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
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

#[cfg(any(test, feature = "test-support"))]
impl Scan {
    pub fn state_census(
        &self,
        line: dashboard_base::prepared::state_census::Line,
    ) -> dashboard_base::prepared::state_census::Line {
        line.field("divisions_events", self.events.len())
            .json("divisions_events_json", &self.events)
            .field("divisions_seen", self.seen.len())
            .field("divisions_live", self.live.len())
            .field("divisions_deletions", self.deletions.len())
            .field("divisions_warnings", self.warnings.len())
            .json("divisions_counts_json", &self.counts)
            .json("divisions_presentation_json", &self.presentation)
    }
}

#[cfg(test)]
#[path = "division_log/tests.rs"]
mod tests;
