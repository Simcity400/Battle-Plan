//! Every country's ace roster, dated by successful AirAces reads. Aces have no
//! creation/death timestamp: marking one dead stores alive, cause and killers,
//! never a date. First sightings are upper bounds; death is
//! (last alive read, first dead read], or unknown for an ace first seen dead.
//!
//! Ace generation chooses weighted definitions sharing an aircraft mask and
//! stores the selected definition. The recorded selected modifier is distinct
//! from the compatibility mask and the aircraft assigned to the wing.
//!
//! Prepared manifests, recording-block windows and portraits share the roster
//! observer. Windows have no player gate or truncation;
//! only the transport reply budget may refuse it.
//! The ace reader and air writer retain the full reference, owner-roster link,
//! roster position, aircraft mask, wing, name fields, alive and handled flags,
//! death cause, and dead-only killer reference and country.

use dashboard_base::products::{
    MAX_PRODUCT_REPLY_BYTES, Product,
    country_groups::HourSpan,
    equipment::{self, StockVariantName, VariantCatalogue},
};
use dashboard_base::{catalogue, source::Source};
use dashboard_core::config::LoadContext;
use dashboard_core::transport::json_len;
use dashboard_text::{
    catalogue::{Catalogue, Refusals, View},
    metadata::{LinkTags, branch_bounds, slot_read},
};
use installed_content::localisation::Argument;
use replay::{
    container::Replay,
    frame::{DecodedFrame, SharedView, TableView, ViewIdentity},
    rows::{AirAceRow, TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;
#[cfg(test)]
use {
    dashboard_text::metadata::validate_campaign, replay::frame::FrameDecoder,
    std::collections::VecDeque,
};

pub struct AirAces;
pub struct AirAcePortraits;

pub mod prepared;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum AirAcesView {
    #[default]
    Manifest,
    Window,
    Catalogue,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AirAcesParams {
    pub branch_id: String,
    #[serde(default)]
    pub view: AirAcesView,
    #[serde(default)]
    pub window: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AirAceDeath {
    pub last_alive: Option<i32>,
    pub by_hour: Option<i32>,
    pub killer_ace: Option<String>,
    pub killer_tag: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AirAceRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub tag: Option<String>,
    pub name: Option<usize>,
    pub callsign: Option<usize>,
    pub wing: Option<String>,
    pub wing_name: Option<usize>,
    pub aircraft: Option<Vec<String>>,
    pub alive: bool,
    pub death: Option<AirAceDeath>,
}

#[derive(Clone, Debug, Serialize, TS, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AirAceEntry {
    pub id: String,
    pub first_seen: Option<i32>,
    pub runs: Vec<AirAceRun>,
}

#[derive(Clone, Debug, Serialize, TS, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AirAceIdentity {
    pub id: String,
    pub first_seen: i32,
}

#[derive(Clone, Debug, Serialize, TS, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AirAcesManifestReply {
    pub branch_id: String,
    pub first_hour: i32,
    pub last_hour: i32,
    pub available: bool,
    pub unavailable: Option<String>,
    pub observed: Vec<HourSpan>,
    /// Successful roster reads across the branch.
    /// Equal facts compress; consumers date each row to its last read <= hour.
    pub read_hours: Vec<i32>,
    pub aces: Vec<AirAceIdentity>,
    /// Recording-block bounds, including blocks that could not be read.
    pub windows: Vec<HourSpan>,
    pub variants: BTreeMap<String, Vec<StockVariantName>>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, TS, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AirAcesWindowReply {
    pub branch_id: String,
    pub window: usize,
    pub first_hour: i32,
    pub last_hour: i32,
    pub aces: Vec<AirAceEntry>,
    /// Run name, callsign and wingName fields index this window's exact text.
    pub texts: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, TS, Deserialize)]
#[serde(untagged)]
pub enum AirAcesReply {
    Manifest(AirAcesManifestReply),
    Window(AirAcesWindowReply),
}

/// The roster at one hour, with historical dates pending past preparation.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AirAcesRosterReply {
    pub manifest: AirAcesManifestReply,
    pub roster: AirAcesWindowReply,
    pub read_hour: Option<i32>,
    pub preparing: Vec<String>,
}

#[derive(Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AirAcesRosterParams {
    pub branch_id: String,
    #[ts(type = "\"roster\"")]
    pub view: String,
    pub hour: i32,
}

impl dashboard_base::products::ReplyWarnings for AirAcesReply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        match self {
            Self::Manifest(reply) => &mut reply.warnings,
            Self::Window(reply) => &mut reply.warnings,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AirAcePortraitParams {
    pub branch_id: String,
    pub hour: i32,
    pub ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, TS, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AirAcePortrait {
    pub id: String,
    pub image: Option<String>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS, Deserialize)]
pub struct AirAcePortraitReply {
    pub portraits: Vec<AirAcePortrait>,
    pub warnings: Vec<String>,
}

// Keep the first decoded roster instead of copying its strings once per ace.
// Later changes own only their changed row, never another entire roster.
#[derive(Clone, Debug)]
enum RecordedAce {
    Initial { table: SharedView, index: usize },
    Changed(Box<AirAceRow>),
}
impl std::ops::Deref for RecordedAce {
    type Target = AirAceRow;
    fn deref(&self) -> &AirAceRow {
        match self {
            Self::Initial { table, index } => {
                let TableView::Complete(rows) = &**table else {
                    unreachable!("retained complete ace roster")
                };
                &rows.rows::<tables::AirAces>().expect("retained ace rows")[*index]
            }
            Self::Changed(row) => row,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Facts {
    #[serde(skip, default = "unrestored_ace")]
    row: RecordedAce,
    tag: Option<String>,
    variants: Option<Vec<u64>>,
    wing_name: Option<Vec<u8>>,
    last_alive: Option<i32>,
    dead_by: Option<i32>,
    killer_tag: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Run {
    first: i32,
    last: i32,
    facts: Facts,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Ace {
    id: u64,
    first_seen: i32,
    last_read: Option<i32>,
    last_alive: Option<i32>,
    dead_by: Option<i32>,
    runs: Vec<Run>,
}
#[derive(Default, Serialize, Deserialize)]
struct Scan {
    /// The recording's format, which dates its daily tables
    /// ([`dashboard_text::metadata::slot_read`]).
    #[serde(skip)]
    format: replay::format::FormatVersion,
    first: i32,
    last: i32,
    #[serde(skip)]
    windows: Vec<HourSpan>,
    unread: bool,
    last_frame: Option<i32>,
    read_through: Option<i32>,
    #[serde(skip)]
    country_keys: Option<dashboard_text::metadata::CountryKeys>,
    #[cfg(test)]
    #[serde(skip)]
    work: dashboard_base::prepared::observer_tests::Work,
    known: bool,
    aces: Vec<Ace>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    indices: HashMap<u64, usize>,
    active: Vec<usize>,
    observed: Vec<HourSpan>,
    reads: Vec<i32>,
    #[serde(skip)]
    from: Vec<ViewIdentity>,
    variant_ids: BTreeSet<u64>,
    variants: VariantCatalogue,
    tags: LinkTags,
    warnings: BTreeSet<String>,
    refusals: Refusals,
    wing_refusals: Refusals,
}

fn unrestored_ace() -> RecordedAce {
    RecordedAce::Changed(Box::default())
}

fn bounded<T: Serialize>(reply: T, kind: &str) -> io::Result<T> {
    json_len(&reply, MAX_PRODUCT_REPLY_BYTES).ok_or_else(|| {
        dashboard_core::invalid(format!(
            "Ace {kind} reply exceeds MAX_PRODUCT_REPLY_BYTES; no aces or runs were truncated"
        ))
    })?;
    Ok(reply)
}
fn cancelled(flag: &AtomicBool) -> io::Result<()> {
    if flag.load(Ordering::Acquire) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Ace request cancelled",
        ))
    } else {
        Ok(())
    }
}

impl Scan {
    fn close(&mut self, last: i32) {
        for &index in &self.active {
            #[cfg(test)]
            {
                self.work.history_visits += 1;
            }
            let ace = &mut self.aces[index];
            if let Some(read) = self.read_through {
                ace.last_read = Some(read);
                if ace.runs.last().is_some_and(|run| run.facts.row.alive != 0) {
                    ace.last_alive = Some(read);
                }
            }
            if let Some(run) = self.aces[index].runs.last_mut() {
                run.last = last;
            }
        }
    }
    #[cfg(test)]
    fn observe(&mut self, frame: &DecodedFrame) {
        self.variants.observe(self.format, frame);
        self.observe_roster(frame, None);
    }
    fn observe_roster(
        &mut self,
        frame: &DecodedFrame,
        changes: Option<&dashboard_base::prepared::frame_changes::FrameChanges>,
    ) -> bool {
        let mut changed = false;
        let hour = frame.hour;
        if self
            .last_frame
            .is_some_and(|last| last.checked_add(1) != Some(hour))
        {
            self.close(self.last_frame.unwrap());
            self.active.clear();
            self.known = false;
            self.read_through = None;
            changed = true;
        }
        let rows = frame.complete::<tables::AirAces>();
        let read = changes.map_or_else(
            || slot_read(self.format, &frame.metrics, TableId::AirAces, hour),
            |changes| changes.slot_read[TableId::AirAces.index()],
        );
        match rows {
            Err(_) => {
                self.close(self.last_frame.unwrap_or(hour.saturating_sub(1)));
                self.active.clear();
                self.known = false;
                self.read_through = None;
                changed = true;
            }
            Ok(rows) if read => {
                let same_roster = self.known
                    && self.from.first().is_some_and(|identity| {
                        identity.is(&frame.tables[TableId::AirAces.index()])
                    });
                let mut ids = HashSet::new();
                if !same_roster
                    && rows.iter().any(|row| {
                        #[cfg(test)]
                        {
                            self.work.row_visits += 1;
                        }
                        !ids.insert(row.id)
                    })
                {
                    self.close(hour.saturating_sub(1));
                    self.active.clear();
                    self.known = false;
                    self.refusals
                        .note("Duplicate ace reference; read is a gap", hour.to_string());
                    self.read_through = None;
                    self.last_frame = Some(hour);
                    return true;
                }
                let inputs = [
                    TableId::AirAces,
                    TableId::Countries,
                    TableId::CountryLinks,
                    TableId::AirWings,
                    TableId::WingEquipment,
                    TableId::Raids,
                    TableId::RaidTooltipOperands,
                ];
                let country_same = changes
                    .is_some_and(|changes| !changes.rows_changed[TableId::Countries.index()])
                    || self.country_keys.as_ref().is_some_and(|keys| {
                        #[cfg(test)]
                        let before = keys.row_visits();
                        let same = frame
                            .complete::<tables::Countries>()
                            .is_ok_and(|rows| keys.same(rows));
                        #[cfg(test)]
                        {
                            self.work.row_visits += keys.row_visits() - before;
                        }
                        same
                    });
                if self.known
                    && self.from.len() == inputs.len()
                    && self.from.iter().zip(inputs).all(|(identity, table)| {
                        (table == TableId::Countries && country_same)
                            || changes.map_or_else(
                                || identity.is(&frame.tables[table.index()]),
                                |changes| !changes.rows_changed[table.index()],
                            )
                    })
                {
                    self.read_through = Some(hour);
                } else {
                    changed = true;
                    self.close(hour.saturating_sub(1));
                    self.country_keys = frame
                        .complete::<tables::Countries>()
                        .ok()
                        .map(dashboard_text::metadata::CountryKeys::read);
                    #[cfg(test)]
                    {
                        self.work.row_visits += rows.len();
                    }
                    let previous_read = self.known.then(|| self.reads.last().copied()).flatten();
                    self.active.clear();
                    #[cfg(test)]
                    {
                        self.work.row_visits += frame
                            .complete::<tables::Countries>()
                            .map_or(0, |rows| rows.len())
                            + frame
                                .complete::<tables::CountryLinks>()
                                .map_or(0, |rows| rows.len());
                    }
                    let tags_known = self.tags.update(frame).is_ok();
                    let equipment = frame.complete::<tables::WingEquipment>().ok();
                    let wings = frame.complete::<tables::AirWings>().ok();
                    let wing_ids: HashSet<u64> = wings
                        .into_iter()
                        .flatten()
                        .map(|row| {
                            #[cfg(test)]
                            {
                                self.work.row_visits += 1;
                            }
                            row.id
                        })
                        .collect();
                    let mut by_wing: HashMap<u64, Vec<(u32, u64)>> = HashMap::new();
                    for row in equipment.into_iter().flatten() {
                        #[cfg(test)]
                        {
                            self.work.row_visits += 1;
                        }
                        by_wing
                            .entry(row.wing)
                            .or_default()
                            .push((row.ordinal, row.variant));
                    }
                    for rows in by_wing.values_mut() {
                        rows.sort_unstable();
                    }
                    let by_wing: HashMap<u64, Vec<u64>> = by_wing
                        .into_iter()
                        .map(|(wing, rows)| (wing, rows.into_iter().map(|(_, id)| id).collect()))
                        .collect();
                    let assigned_wings = rows
                        .iter()
                        .map(|row| row.assigned_wing)
                        .filter(|wing| *wing != 0)
                        .collect();
                    let names = dashboard_text::air_wing_names::recorded_names(
                        frame,
                        &assigned_wings,
                        &mut self.wing_refusals,
                    );
                    self.aces
                        .reserve(rows.len().saturating_sub(self.aces.len()));
                    self.indices
                        .reserve(rows.len().saturating_sub(self.indices.len()));
                    self.active.reserve(rows.len());
                    let initial = self.aces.is_empty();
                    for (position, row) in rows.iter().enumerate() {
                        let index = match self.indices.entry(row.id) {
                            std::collections::hash_map::Entry::Occupied(entry) => *entry.get(),
                            std::collections::hash_map::Entry::Vacant(entry) => {
                                let index = self.aces.len();
                                self.aces.push(Ace {
                                    id: row.id,
                                    first_seen: hour,
                                    last_read: None,
                                    last_alive: None,
                                    dead_by: None,
                                    runs: Vec::with_capacity(1),
                                });
                                entry.insert(index);
                                index
                            }
                        };
                        let ace = &mut self.aces[index];
                        let continues = previous_read.is_some() && ace.last_read == previous_read;
                        ace.last_read = Some(hour);
                        if row.alive != 0 {
                            ace.last_alive = Some(hour);
                            ace.dead_by = None;
                        } else if ace.dead_by.is_none() && ace.last_alive.is_some() {
                            ace.dead_by = Some(hour);
                        }
                        let tag = tags_known.then(|| self.tags.get(row.country)).flatten();
                        if tag.is_none() {
                            self.refusals.note(
                                "Ace owner country link has no resolved recorded tag",
                                row.country.to_string(),
                            );
                        }
                        let variants = if row.assigned_wing == 0 {
                            Some(&[][..])
                        } else if equipment.is_some() && wing_ids.contains(&row.assigned_wing) {
                            Some(
                                by_wing
                                    .get(&row.assigned_wing)
                                    .map_or(&[][..], Vec::as_slice),
                            )
                        } else {
                            None
                        };
                        self.variant_ids
                            .extend(variants.into_iter().flatten().copied());
                        let killer_tag = row.death.as_ref().and_then(|death| {
                            tags_known
                                .then(|| self.tags.get(death.killer_country))
                                .flatten()
                        });
                        let wing_name = names.get(&row.assigned_wing);
                        let last_alive = if row.alive == 0 { ace.last_alive } else { None };
                        if continues
                            && ace.runs.last().is_some_and(|run| {
                                let facts = &run.facts;
                                *facts.row == *row
                                    && facts.tag.as_ref() == tag
                                    && facts.variants.as_deref() == variants
                                    && facts.wing_name.as_ref() == wing_name
                                    && facts.last_alive == last_alive
                                    && facts.dead_by == ace.dead_by
                                    && facts.killer_tag.as_ref() == killer_tag
                            })
                        {
                            ace.runs.last_mut().unwrap().last = hour;
                        } else {
                            let facts = Facts {
                                row: if initial {
                                    RecordedAce::Initial {
                                        table: frame.tables[TableId::AirAces.index()].clone(),
                                        index: position,
                                    }
                                } else {
                                    RecordedAce::Changed(Box::new(row.clone()))
                                },
                                tag: tag.cloned(),
                                variants: variants.map(<[u64]>::to_vec),
                                wing_name: wing_name.cloned(),
                                last_alive,
                                dead_by: ace.dead_by,
                                killer_tag: killer_tag.cloned(),
                            };
                            ace.runs.push(Run {
                                first: hour,
                                last: hour,
                                facts,
                            });
                        }
                        self.active.push(index);
                    }
                    self.from = inputs
                        .iter()
                        .map(|table| frame.tables[table.index()].identity())
                        .collect();
                }
                self.known = true;
                self.read_through = Some(hour);
                self.reads.push(hour);
            }
            Ok(_) => {}
        }
        if self.known {
            match self.observed.last_mut() {
                Some(span) if span.last_hour.checked_add(1) == Some(hour) => span.last_hour = hour,
                _ => self.observed.push(HourSpan {
                    first_hour: hour,
                    last_hour: hour,
                }),
            }
        }
        self.last_frame = Some(hour);
        changed
    }
    #[cfg(test)]
    fn at(&self, id: u64, hour: i32) -> Option<&Run> {
        let ace = &self.aces[*self.indices.get(&id)?];
        let at = ace
            .runs
            .partition_point(|run| run.first <= hour)
            .checked_sub(1)?;
        let run = &ace.runs[at];
        (hour <= run.last).then_some(run)
    }
}

#[cfg(test)]
fn scan(replay: &Replay, branch_id: u64, flag: &AtomicBool) -> io::Result<Scan> {
    let branch = replay
        .branches
        .iter()
        .find(|branch| branch.id == branch_id)
        .ok_or_else(|| dashboard_core::invalid("Branch not in this recording"))?;
    let (first, last) = branch_bounds(replay, branch);
    let mut result = Scan {
        format: replay.format,
        first,
        last,
        windows: replay
            .index
            .iter()
            .filter(|block| block.branch == branch_id)
            .map(|block| HourSpan {
                first_hour: block.first_hour,
                last_hour: block.last_hour,
            })
            .collect(),
        ..Default::default()
    };
    let mut selected = vec![
        TableId::Campaign,
        TableId::AirAces,
        TableId::AirWings,
        TableId::WingEquipment,
        TableId::Raids,
        TableId::RaidTooltipOperands,
    ];
    selected.extend(VariantCatalogue::TABLES);
    selected.sort_unstable_by_key(|table| table.index());
    selected.dedup();
    dashboard_core::blocks::scan_blocks(
        replay,
        replay
            .index
            .iter()
            .filter(|block| block.branch == branch_id),
        &selected,
        flag,
        |blocks| {
            for (_, frames) in blocks {
                cancelled(flag)?;
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
                    result.unread = true;
                    result
                        .warnings
                        .insert("Unread replay block; ace history has a gap".into());
                    continue;
                };
                for frame in frames {
                    cancelled(flag)?;
                    let Ok(decoded) = decoder.advance(&frame) else {
                        result.unread = true;
                        result
                            .warnings
                            .insert("Unread replay frame; ace history has a gap".into());
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        result.unread = true;
                        result
                            .warnings
                            .insert("Campaign identity mismatch; ace history has a gap".into());
                        break;
                    }
                    result.observe(decoded);
                }
            }
            Ok(())
        },
    )?;
    if let Some(last) = result.last_frame {
        result.close(last);
    }
    Ok(result)
}

#[cfg(test)]
#[derive(Default)]
struct Cache {
    branches: VecDeque<(u64, Arc<Scan>)>,
}
#[cfg(test)]
fn cached(source: &Source, branch: u64, flag: &AtomicBool) -> io::Result<Arc<Scan>> {
    let state = source.state::<Cache>();
    let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, result)) = state.branches.iter().find(|(id, _)| *id == branch) {
        return Ok(Arc::clone(result));
    }
    let result = Arc::new(scan(&source.replay(), branch, flag)?);
    state.branches.push_back((branch, Arc::clone(&result)));
    // Retention only: evicted branches remain fully queryable by rescanning.
    while state.branches.len() > 2 {
        state.branches.pop_front();
    }
    Ok(result)
}

fn text(bytes: &[u8], identity: &str, refusals: &mut Refusals) -> Option<String> {
    catalogue::display_text(bytes)
        .map_err(|reason| refusals.note(reason, identity))
        .ok()
}
fn ace_name(row: &AirAceRow, cat: Option<&Catalogue>, refusals: &mut Refusals) -> Option<String> {
    // A nonempty callsign selects ACE_FULL_NAME_WITH_CALLSIGN in the game.
    // The separate columns use its plain ACE_FULL_NAME(NAME, SUR) template here
    // and retain callsign in its own column.
    let Some(cat) = cat else {
        refusals.note(
            "ACE_FULL_NAME content unavailable; name order unknown",
            row.id.to_string(),
        );
        return None;
    };
    if cat.localisation.get("ACE_FULL_NAME").is_none() {
        refusals.note(
            "ACE_FULL_NAME absent from recorded content",
            row.id.to_string(),
        );
        return None;
    }
    let identity = row.id.to_string();
    let first = text(&row.name, &identity, refusals)?;
    let surname = text(&row.surname, &identity, refusals)?;
    cat.localised_with(
        "ACE_FULL_NAME",
        &[
            ("NAME", Argument::Text(&first)),
            ("SUR", Argument::Text(&surname)),
        ],
        View::Plain,
    )
}

fn manifest(
    scan: &Scan,
    params: &AirAcesParams,
    cat: Option<&Catalogue>,
    flag: &AtomicBool,
) -> io::Result<AirAcesManifestReply> {
    let mut aces = Vec::with_capacity(scan.aces.len());
    for ace in &scan.aces {
        cancelled(flag)?;
        aces.push(AirAceIdentity {
            id: ace.id.to_string(),
            first_seen: ace.first_seen,
        });
    }
    let variants = scan
        .variant_ids
        .iter()
        .copied()
        .map(|id| {
            (
                id.to_string(),
                scan.variants
                    .names(id, cat.map(|c| c as &dyn equipment::Texts)),
            )
        })
        .collect();
    let mut warnings = scan.warnings.clone();
    warnings.extend(scan.refusals.counts("ace fields/reads"));
    warnings.extend(scan.wing_refusals.counts("ace wing names"));
    Ok(AirAcesManifestReply {
        branch_id: params.branch_id.clone(),
        first_hour: scan.first,
        last_hour: scan.last,
        available: !scan.observed.is_empty(),
        unavailable: scan.observed.is_empty().then(|| {
            if scan.unread {
                "AirAces availability could not be checked".into()
            } else {
                "AirAces was unavailable, incomplete or never read in this recording.".into()
            }
        }),
        observed: scan.observed.clone(),
        read_hours: scan.reads.clone(),
        aces,
        windows: scan.windows.clone(),
        variants,
        warnings: warnings.into_iter().collect(),
    })
}

#[derive(Default)]
struct Texts {
    indices: HashMap<String, usize>,
    values: Vec<String>,
}
impl Texts {
    fn intern(&mut self, value: Option<String>) -> Option<usize> {
        let value = value?;
        if let Some(index) = self.indices.get(&value) {
            return Some(*index);
        }
        let index = self.values.len();
        self.values.push(value.clone());
        self.indices.insert(value, index);
        Some(index)
    }
}

fn window_reply(
    scan: &Scan,
    params: &AirAcesParams,
    cat: Option<&Catalogue>,
    flag: &AtomicBool,
) -> io::Result<AirAcesWindowReply> {
    let window = params
        .window
        .ok_or_else(|| dashboard_core::invalid("Ace window index required"))?;
    let span = scan
        .windows
        .get(window)
        .ok_or_else(|| dashboard_core::invalid("Ace window not in this recording"))?;
    let mut refusals = Refusals::default();
    let mut wing_refusals = Refusals::default();
    let mut texts = Texts::default();
    let mut aces = Vec::new();
    for ace in &scan.aces {
        cancelled(flag)?;
        let first = ace.runs.partition_point(|run| run.last < span.first_hour);
        let last = ace.runs.partition_point(|run| run.first <= span.last_hour);
        let mut runs = Vec::with_capacity(last - first);
        for raw in &ace.runs[first..last] {
            let facts = &raw.facts;
            let row = &facts.row;
            let death = (row.alive == 0).then(|| AirAceDeath {
                last_alive: facts.last_alive,
                by_hour: facts.dead_by,
                killer_ace: row
                    .death
                    .as_ref()
                    .map(|death| death.killer_ace)
                    .filter(|id| *id != 0)
                    .map(|id| id.to_string()),
                killer_tag: facts.killer_tag.clone(),
            });
            let wing_name = facts.wing_name.as_ref().and_then(|bytes| match cat {
                Some(cat) => dashboard_text::scripted_text::stored_text(
                    &cat.localisation,
                    &cat.recorded_language,
                    bytes,
                    "Air wing name",
                )
                .map_err(|why| wing_refusals.note(why, row.assigned_wing.to_string()))
                .ok(),
                None => text(bytes, &row.assigned_wing.to_string(), &mut wing_refusals),
            });
            runs.push(AirAceRun {
                // Preserve the supplying read before a block boundary. The end is
                // clipped so appending later years cannot change this window.
                first_hour: raw.first,
                last_hour: raw.last.min(span.last_hour),
                tag: facts.tag.clone(),
                name: texts.intern(ace_name(row, cat, &mut refusals)),
                callsign: texts.intern(text(&row.callsign, &row.id.to_string(), &mut refusals)),
                wing: (row.assigned_wing != 0).then(|| row.assigned_wing.to_string()),
                wing_name: texts.intern(wing_name),
                aircraft: facts
                    .variants
                    .as_ref()
                    .map(|ids| ids.iter().map(u64::to_string).collect()),
                alive: row.alive != 0,
                death,
            });
        }
        if !runs.is_empty() {
            aces.push(AirAceEntry {
                id: ace.id.to_string(),
                first_seen: Some(ace.first_seen),
                runs,
            });
        }
    }
    let mut warnings = BTreeSet::new();
    warnings.extend(refusals.counts("ace fields/reads"));
    warnings.extend(wing_refusals.counts("ace wing names"));
    Ok(AirAcesWindowReply {
        branch_id: params.branch_id.clone(),
        window,
        first_hour: span.first_hour,
        last_hour: span.last_hour,
        aces,
        texts: texts.values,
        warnings: warnings.into_iter().collect(),
    })
}

// Like Raids' catalogue view, stop at the first positive recording evidence.
// No joins, content names, portraits or history materialization for the picker.
#[cfg(test)]
fn recording_available(
    replay: &Replay,
    branch: u64,
    flag: &AtomicBool,
) -> io::Result<Option<bool>> {
    let selected = [TableId::Campaign, TableId::AirAces];
    let mut unread = false;
    dashboard_core::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        flag,
        |blocks| {
            for (_, frames) in blocks {
                cancelled(flag)?;
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
                    unread = true;
                    continue;
                };
                for frame in frames {
                    cancelled(flag)?;
                    let Ok(decoded) = decoder.advance(&frame) else {
                        unread = true;
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        unread = true;
                        break;
                    }
                    if slot_read(
                        replay.format,
                        &decoded.metrics,
                        TableId::AirAces,
                        decoded.hour,
                    ) && let Ok(rows) = decoded.complete::<tables::AirAces>()
                    {
                        let mut ids = HashSet::new();
                        if rows.iter().all(|row| ids.insert(row.id)) {
                            return Ok(Some(true));
                        }
                    }
                }
            }
            Ok((!unread).then_some(false))
        },
    )
}

// The portrait ladder uses a named literal first, then the retained country/gender pool.
// Only an empty country pool builds culture sprites, stopping at the
// first missing sprite or the game's own 256-entry bound; global pool last.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct PortraitPool {
    cached: Option<Vec<u8>>,
    culture: Option<Vec<u8>>,
    fallback: Option<Vec<u8>>,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct PortraitSelection {
    gender: u8,
    pool_index: i32,
    portrait: Option<PortraitPool>,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct PortraitOperands {
    tag: Option<String>,
    name: Vec<u8>,
    surname: Vec<u8>,
    fields: Result<Option<PortraitSelection>, String>,
}
impl From<&Facts> for PortraitOperands {
    fn from(facts: &Facts) -> Self {
        Self {
            tag: facts.tag.clone(),
            name: facts.row.name.clone(),
            surname: facts.row.surname.clone(),
            fields: facts
                .row
                .presentation
                .require("Ace presentation")
                .map(|fields| {
                    fields.as_ref().map(|fields| PortraitSelection {
                        gender: fields.gender,
                        pool_index: fields.pool_index,
                        portrait: fields.portrait.as_ref().map(|pool| PortraitPool {
                            cached: pool.cached.clone(),
                            culture: pool.culture.clone(),
                            fallback: pool.fallback.clone(),
                        }),
                    })
                }),
        }
    }
}
fn portrait(facts: &PortraitOperands, cat: &Catalogue) -> Result<String, String> {
    let tag = facts.tag.as_ref().ok_or("Ace country not recorded")?;
    let name = std::str::from_utf8(&facts.name).map_err(|_| "Ace name is not UTF-8")?;
    let surname = std::str::from_utf8(&facts.surname).map_err(|_| "Ace surname is not UTF-8")?;
    let key = format!("GFX_{tag}_ace_{name}_{surname}");
    let sprite = match cat.sprites.find(&key)? {
        Some(sprite) => sprite,
        None => {
            let fields = facts
                .fields
                .as_ref()
                .map_err(|reason| reason.clone())?
                .as_ref()
                .ok_or("Ace presentation capture failed")?;
            let pool = fields
                .portrait
                .as_ref()
                .ok_or("Ace portrait pool capture failed")?;
            let cached = pool.cached.as_ref().or(pool.fallback.as_ref());
            let mut culture_pool = Vec::new();
            if pool.cached.is_none() {
                let culture = pool
                    .culture
                    .as_ref()
                    .ok_or("Ace portrait culture not recorded")?;
                let culture = std::str::from_utf8(culture)
                    .map_err(|_| "Ace portrait culture is not UTF-8")?;
                let gender = if fields.gender == 0 { "m" } else { "f" };
                for index in 0..256 {
                    let key = format!("GFX_{culture}_ace_{gender}_{index}");
                    let Some(sprite) = cat.sprites.find(&key)? else {
                        break;
                    };
                    culture_pool.push(sprite);
                }
            }
            if culture_pool.is_empty() {
                let key = cached.ok_or("Native ace global portrait unavailable or pool empty")?;
                let key =
                    std::str::from_utf8(key).map_err(|_| "Ace portrait sprite key is not UTF-8")?;
                cat.sprites.find(key)?.ok_or_else(|| {
                    format!("Native ace portrait sprite {key} absent from recorded content")
                })?
            } else {
                // Signed division keeps its remainder sign. A negative remainder is an
                // out-of-bounds game access, never rem_euclid or abs here.
                let index = fields.pool_index % (culture_pool.len() as i32);
                culture_pool
                    .get(
                        usize::try_from(index)
                            .map_err(|_| "Ace portrait index has negative native remainder")?,
                    )
                    .copied()
                    .ok_or("Ace portrait index outside native pool")?
            }
        }
    };
    catalogue::sprite_frame_png(&cat.content.layers, sprite, cat.gui_decode.clone()?, 0, 96)
        .map_err(|error| error.to_string())
}

#[derive(Default)]
struct PortraitCache {
    catalogue: Option<Arc<Catalogue>>,
    entries: HashMap<PortraitOperands, Result<String, String>>,
}
fn cached_portrait(
    source: &Source,
    facts: &PortraitOperands,
    cat: &Arc<Catalogue>,
) -> Result<String, String> {
    let state = source.state::<PortraitCache>();
    let key = facts.clone();
    {
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        if !state
            .catalogue
            .as_ref()
            .is_some_and(|previous| Arc::ptr_eq(previous, cat))
        {
            *state = PortraitCache {
                catalogue: Some(Arc::clone(cat)),
                ..Default::default()
            };
        }
        if let Some(found) = state.entries.get(&key) {
            return found.clone();
        }
    }
    let found = portrait(facts, cat);
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    if state
        .catalogue
        .as_ref()
        .is_some_and(|previous| Arc::ptr_eq(previous, cat))
    {
        return state.entries.entry(key).or_insert(found).clone();
    }
    found
}

fn ace_warnings(result: &mut AirAcesReply, cat: Result<&Catalogue, &io::Error>) {
    let warnings = match result {
        AirAcesReply::Manifest(reply) => &mut reply.warnings,
        AirAcesReply::Window(reply) => &mut reply.warnings,
    };
    match cat {
        Err(why) => warnings.push(format!("Ace content unavailable: {why}")),
        Ok(cat) => {
            warnings.extend(cat.verification.iter().cloned());
            warnings.extend(cat.warnings.iter().cloned());
            warnings.extend(cat.refusal_lines());
        }
    }
}

impl Product for AirAces {
    const KEY: &'static str = "air-aces";
    const CONTENT_DEPENDENT: bool = true;
    type Params = AirAcesParams;
    type Reply = AirAcesReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid branch")?;
        match (params.view, params.window) {
            (AirAcesView::Window, None) => Err("Ace window index required"),
            (AirAcesView::Manifest | AirAcesView::Catalogue, Some(_)) => {
                Err("Ace window index only allowed for a window request")
            }
            _ => Ok(()),
        }
    }
    fn types() -> Vec<String> {
        vec![
            AirAcesParams::decl(),
            AirAcesView::decl(),
            AirAcesReply::decl(),
            AirAcesRosterReply::decl(),
            AirAcesRosterParams::decl(),
            AirAcesManifestReply::decl(),
            AirAcesWindowReply::decl(),
            AirAceIdentity::decl(),
            AirAceEntry::decl(),
            AirAceRun::decl(),
            AirAceDeath::decl(),
            HourSpan::decl(),
            StockVariantName::decl(),
        ]
    }
    #[cfg(test)]
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        flag: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        if matches!(params.view, AirAcesView::Catalogue) {
            let replay = source.replay();
            let branch = params
                .branch_id
                .parse::<u64>()
                .map_err(|_| dashboard_core::invalid("Invalid branch"))?;
            let branch = replay
                .branches
                .iter()
                .find(|b| b.id == branch)
                .ok_or_else(|| dashboard_core::invalid("Branch not in this recording"))?;
            let (first_hour, last_hour) = branch_bounds(&replay, branch);
            let available = recording_available(&replay, branch.id, flag)?;
            let result = AirAcesManifestReply {
                branch_id: params.branch_id.clone(),
                first_hour,
                last_hour,
                available: available == Some(true),
                unavailable: match available {
                    Some(true) => None,
                    Some(false) => Some(
                        "AirAces was unavailable, incomplete or never read in this recording."
                            .into(),
                    ),
                    None => Some("AirAces availability could not be checked".into()),
                },
                observed: Vec::new(),
                read_hours: Vec::new(),
                aces: Vec::new(),
                windows: Vec::new(),
                variants: BTreeMap::new(),
                warnings: Vec::new(),
            };
            return bounded(AirAcesReply::Manifest(result), "catalogue");
        }
        let scan = cached(
            source,
            params
                .branch_id
                .parse()
                .map_err(|_| dashboard_core::invalid("Invalid branch"))?,
            flag,
        )?;
        let cat = context
            .content()
            .and_then(|options| catalogue::catalogue(source, options));
        let mut result = match params.view {
            AirAcesView::Manifest => {
                AirAcesReply::Manifest(manifest(&scan, params, cat.as_deref().ok(), flag)?)
            }
            AirAcesView::Window => {
                AirAcesReply::Window(window_reply(&scan, params, cat.as_deref().ok(), flag)?)
            }
            AirAcesView::Catalogue => unreachable!("catalogue returned above"),
        };
        ace_warnings(&mut result, cat.as_deref());
        bounded(
            result,
            if matches!(params.view, AirAcesView::Window) {
                "window"
            } else {
                "manifest"
            },
        )
    }
}
fn portrait_reply(
    source: &Source,
    params: &AirAcePortraitParams,
    context: &LoadContext,
    flag: &AtomicBool,
    lookup: impl Fn(u64, i32) -> Option<PortraitOperands>,
) -> io::Result<AirAcePortraitReply> {
    let cat = context
        .content()
        .and_then(|options| catalogue::catalogue(source, options));
    let mut portraits = vec![];
    let mut refusals = Refusals::default();
    for id in &params.ids {
        cancelled(flag)?;
        let result = lookup(
            id.parse()
                .map_err(|_| dashboard_core::invalid("Invalid ace"))?,
            params.hour,
        )
        .ok_or_else(|| "Ace not recorded at this hour".to_owned())
        .and_then(|operands| {
            cat.as_ref()
                .map_err(|error| error.to_string())
                .and_then(|cat| cached_portrait(source, &operands, cat))
        });
        let (image, missing) = match result {
            Ok(image) => (Some(image), None),
            Err(reason) => {
                refusals.note(&reason, format!("Ace {id}"));
                (None, Some(reason))
            }
        };
        portraits.push(AirAcePortrait {
            id: id.clone(),
            image,
            missing,
        });
    }
    let mut warnings = cat.as_ref().map_or_else(
        |why| vec![format!("Ace portraits unavailable: {why}")],
        |cat| {
            cat.warnings
                .iter()
                .chain(cat.verification.iter())
                .cloned()
                .collect()
        },
    );
    warnings.extend(refusals.counts("ace portraits"));
    let result = AirAcePortraitReply {
        portraits,
        warnings,
    };
    bounded(result, "portraits")
}

impl Product for AirAcePortraits {
    const KEY: &'static str = "air-ace-portraits";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = AirAcePortraitParams;
    type Reply = AirAcePortraitReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err()
            || params.ids.is_empty()
            || params.ids.iter().any(|id| id.parse::<u64>().is_err())
        {
            Err("Invalid ace portrait request")
        } else {
            Ok(())
        }
    }
    fn types() -> Vec<String> {
        vec![
            AirAcePortraitParams::decl(),
            AirAcePortraitReply::decl(),
            AirAcePortrait::decl(),
        ]
    }
    #[cfg(test)]
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        flag: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let scan = cached(
            source,
            params
                .branch_id
                .parse()
                .map_err(|_| dashboard_core::invalid("Invalid branch"))?,
            flag,
        )?;
        portrait_reply(source, params, context, flag, |id, hour| {
            scan.at(id, hour)
                .map(|run| PortraitOperands::from(&run.facts))
        })
    }
}

#[cfg(test)]
mod tests;
