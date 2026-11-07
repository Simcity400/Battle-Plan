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
use installed_content_localisation::localisation::Argument;
use facts::{Read, Roster};
use replay::campaign::Replay;
use replay_data::{rows::AirAceRow, rows::RowCodec, rows::TableId, rows::tables};
use replay_frame::frame::{DecodedFrame, Rowset, Tables};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, hash_map::Entry},
    io,
    mem::size_of,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;
#[cfg(test)]
use dashboard_text::metadata::validate_campaign;

pub struct AirAces;
pub struct AirAcePortraits;

#[cfg(test)]
mod columnar_tests;
pub mod emit;
pub mod facts;
pub mod prepared;
mod served;

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

/// One ace as a roster read gives it: its row, and what the read joins to it.
#[derive(Clone, Debug, PartialEq, replay_spool::Spooled)]
pub struct Sighting {
    row: AirAceRow,
    tag: Option<String>,
    variants: Option<Vec<u64>>,
    wing_name: Option<Vec<u8>>,
    killer_tag: Option<String>,
}

impl Sighting {
    /// The bytes it owns on the heap.
    pub(crate) fn heap(&self) -> usize {
        RowCodec::heap_bytes(&self.row)
            + self.tag.as_ref().map_or(0, String::capacity)
            + self
                .variants
                .as_ref()
                .map_or(0, |ids| ids.capacity() * size_of::<u64>())
            + self.wing_name.as_ref().map_or(0, Vec::capacity)
            + self.killer_tag.as_ref().map_or(0, String::capacity)
    }
}

/// [`Sighting`] borrowed from the read that gives it.
#[derive(Clone, Copy)]
pub(crate) struct Joined<'r> {
    row: &'r AirAceRow,
    tag: Option<&'r String>,
    variants: Option<&'r [u64]>,
    wing_name: Option<&'r Vec<u8>>,
    killer_tag: Option<&'r String>,
}

impl Joined<'_> {
    pub(crate) fn id(&self) -> u64 {
        self.row.id
    }

    pub(crate) fn is(&self, seen: &Sighting) -> bool {
        *self.row == seen.row
            && self.tag == seen.tag.as_ref()
            && self.variants == seen.variants.as_deref()
            && self.wing_name == seen.wing_name.as_ref()
            && self.killer_tag == seen.killer_tag.as_ref()
    }

    pub(crate) fn owned(&self) -> Sighting {
        Sighting {
            row: self.row.clone(),
            tag: self.tag.cloned(),
            variants: self.variants.map(<[u64]>::to_vec),
            wing_name: self.wing_name.cloned(),
            killer_tag: self.killer_tag.cloned(),
        }
    }
}

/// Whether a roster read holds an ace twice, which makes it a gap.
/// Whether a read holds an ace twice. Aces are keyed by id; a duplicate is a corrupt read.
pub(crate) fn duplicated(rows: Rowset<'_, AirAceRow>) -> bool {
    let mut ids = HashSet::with_capacity(rows.len());
    rows.keys().any(|key| !ids.insert(key))
}

/// Each ace of a roster read, in roster order, with what the read joins to it: its owner's and
/// killer's tags, its wing's variants by ordinal and the wing's stored name.
/// The ace roster's fields read to name a run's wing.
const ASSIGNED: u64 = replay_data::rows::mask::<AirAceRow>(&["assigned_wing"]);

/// `rows`, an ace roster read of `frame`, each with what the read joins to it, `tags_known`
/// whether the tags were brought to the hour's countries.
pub(crate) fn join<F: Tables + ?Sized>(
    frame: &F,
    rows: Rowset<'_, AirAceRow>,
    (tags, tags_known): (&LinkTags, bool),
    refusals: &mut Refusals,
    wing_refusals: &mut Refusals,
    mut each: impl FnMut(Joined<'_>),
) -> io::Result<()> {
    let mut assigned = HashSet::new();
    for row in rows.with_mask(ASSIGNED).iter() {
        let wing = row?.assigned_wing;
        if wing != 0 {
            assigned.insert(wing);
        }
    }
    let variant = replay_data::rows::mask::<replay_data::rows::WingEquipmentRow>(&["variant"]);
    let equipment = frame.complete_rows::<tables::WingEquipment>(variant).ok();
    let wings: HashSet<u64> = match frame.complete_rows::<tables::AirWings>(0) {
        Ok(held) => assigned
            .iter()
            .copied()
            .filter(|&id| held.contains(u128::from(id)))
            .collect(),
        Err(_) => HashSet::new(),
    };
    let mut by_wing: HashMap<u64, Vec<(u32, u64)>> = HashMap::new();
    if let Some(equipment) = &equipment {
        // An equipment row's wing is its key's high bits.
        for row in equipment.keyed(|key| assigned.contains(&((key >> 32) as u64))) {
            let row = row?;
            by_wing
                .entry(row.wing)
                .or_default()
                .push((row.ordinal, row.variant));
        }
    }
    let by_wing: HashMap<u64, Vec<u64>> = by_wing
        .into_iter()
        .map(|(wing, mut rows)| {
            rows.sort_unstable();
            (wing, rows.into_iter().map(|(_, id)| id).collect())
        })
        .collect();
    let names = dashboard_text::air_wing_names::recorded_names(frame, &assigned, wing_refusals);
    for row in rows.iter() {
        let row = row?;
        let row = &*row;
        let tag = tags_known.then(|| tags.get(row.country)).flatten();
        if tag.is_none() {
            refusals.note(
                "Ace owner country link has no resolved recorded tag",
                row.country.to_string(),
            );
        }
        let variants = if row.assigned_wing == 0 {
            Some(&[][..])
        } else if equipment.is_some() && wings.contains(&row.assigned_wing) {
            Some(
                by_wing
                    .get(&row.assigned_wing)
                    .map_or(&[][..], Vec::as_slice),
            )
        } else {
            None
        };
        let killer_tag = row.death.as_ref().and_then(|death| {
            tags_known
                .then(|| tags.get(death.killer_country))
                .flatten()
        });
        each(Joined {
            row,
            tag,
            variants,
            wing_name: names.get(&row.assigned_wing),
            killer_tag,
        });
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Facts {
    /// Saved apart from the continuation, which no reply reads it from.
    #[serde(skip)]
    row: AirAceRow,
    tag: Option<String>,
    variants: Option<Vec<u64>>,
    wing_name: Option<Vec<u8>>,
    last_alive: Option<i32>,
    dead_by: Option<i32>,
    killer_tag: Option<String>,
}

impl Facts {
    fn new(seen: Sighting, last_alive: Option<i32>, dead_by: Option<i32>) -> Self {
        Self {
            row: seen.row,
            tag: seen.tag,
            variants: seen.variants,
            wing_name: seen.wing_name,
            last_alive,
            dead_by,
            killer_tag: seen.killer_tag,
        }
    }

    fn same(&self, seen: &Sighting, last_alive: Option<i32>, dead_by: Option<i32>) -> bool {
        self.row == seen.row
            && self.tag == seen.tag
            && self.variants == seen.variants
            && self.wing_name == seen.wing_name
            && self.last_alive == last_alive
            && self.dead_by == dead_by
            && self.killer_tag == seen.killer_tag
    }
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

/// Which aces an hour's roster gave anew: none, every one, or, of a read that gave only what
/// changed, those it gave and those it no longer holds (indices into [`Scan::aces`]).
enum Touched {
    None,
    All,
    Changes { seen: Vec<usize>, gone: Vec<usize> },
}

#[derive(Default, Serialize, Deserialize)]
struct Scan {
    /// The recording's format, which dates its daily tables
    /// ([`dashboard_text::metadata::slot_read`]).
    #[serde(skip)]
    format: replay_data::format::FormatVersion,
    first: i32,
    last: i32,
    #[serde(skip)]
    windows: Vec<HourSpan>,
    unread: bool,
    last_frame: Option<i32>,
    read_through: Option<i32>,
    known: bool,
    aces: Vec<Ace>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    indices: HashMap<u64, usize>,
    active: BTreeSet<usize>,
    observed: Vec<HourSpan>,
    reads: Vec<i32>,
    variant_ids: BTreeSet<u64>,
    variants: VariantCatalogue,
    /// The row reads' tags; a native roster's are its operators'.
    #[serde(skip)]
    tags: LinkTags,
    warnings: BTreeSet<String>,
    refusals: Refusals,
    wing_refusals: Refusals,
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

/// Ends `ace`'s last run at `last`, dating the ace to the roster's last read, `read`.
fn close_ace(ace: &mut Ace, read: Option<i32>, last: i32) {
    if let Some(read) = read {
        ace.last_read = Some(read);
        if ace.runs.last().is_some_and(|run| run.facts.row.alive != 0) {
            ace.last_alive = Some(read);
        }
    }
    if let Some(run) = ace.runs.last_mut() {
        run.last = last;
    }
}

impl Scan {
    fn close(&mut self, last: i32) {
        for &index in &self.active {
            close_ace(&mut self.aces[index], self.read_through, last);
        }
    }

    /// What is known of the roster ends at `last`.
    fn end(&mut self, last: i32) {
        self.close(last);
        self.active.clear();
        self.known = false;
        self.read_through = None;
    }

    #[cfg(test)]
    fn observe(&mut self, frame: &DecodedFrame) {
        self.variants.observe(self.format, frame);
        self.observe_roster(frame)
            .expect("a row read gives every ace");
    }

    /// One hour of a row recording, whose every read gives every ace.
    fn observe_roster(&mut self, frame: &DecodedFrame) -> io::Result<()> {
        let roster = match frame.complete::<tables::AirAces>() {
            Err(_) => Roster::Unknown,
            Ok(rows) if slot_read(self.format, &frame.metrics, TableId::AirAces, frame.hour) => {
                if duplicated(Rowset::Decoded(rows)) {
                    Roster::Duplicate
                } else {
                    let mut seen = Vec::with_capacity(rows.len());
                    let known = self.tags.update(frame).is_ok();
                    join(
                        frame,
                        Rowset::Decoded(rows),
                        (&self.tags, known),
                        &mut self.refusals,
                        &mut self.wing_refusals,
                        |ace| seen.push(ace.owned()),
                    )?;
                    Roster::Read(Box::new(Read {
                        whole: true,
                        seen,
                        gone: Vec::new(),
                    }))
                }
            }
            Ok(_) => Roster::Unread,
        };
        self.hour(frame.hour, roster).map(drop)
    }

    /// Keeps one hour of the roster: which aces it gave anew.
    fn hour(&mut self, hour: i32, roster: Roster) -> io::Result<Touched> {
        let mut touched = Touched::None;
        if let Some(last) = self.last_frame
            && last.checked_add(1) != Some(hour)
        {
            self.end(last);
            touched = Touched::All;
        }
        match roster {
            Roster::Unread => {}
            Roster::Unknown => {
                self.end(self.last_frame.unwrap_or(hour.saturating_sub(1)));
                touched = Touched::All;
            }
            Roster::Duplicate => {
                self.end(hour.saturating_sub(1));
                self.refusals
                    .note("Duplicate ace reference; read is a gap", hour.to_string());
                self.last_frame = Some(hour);
                return Ok(Touched::All);
            }
            Roster::Same => {
                if !self.known {
                    return Err(dashboard_core::invalid(
                        "An ace roster read repeats a read not known",
                    ));
                }
                self.read_through = Some(hour);
                self.reads.push(hour);
            }
            Roster::Read(read) => {
                let Read { whole, seen, gone } = *read;
                touched = if whole {
                    self.read_whole(hour, seen);
                    Touched::All
                } else if self.known {
                    self.read_changes(hour, seen, &gone)?
                } else {
                    return Err(dashboard_core::invalid(
                        "An ace roster read changes a read not known",
                    ));
                };
                self.known = true;
                self.read_through = Some(hour);
                self.reads.push(hour);
            }
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
        Ok(touched)
    }

    /// A read giving every ace.
    fn read_whole(&mut self, hour: i32, seen: Vec<Sighting>) {
        self.close(hour.saturating_sub(1));
        let previous = self.known.then(|| self.reads.last().copied()).flatten();
        self.active.clear();
        self.aces.reserve(seen.len().saturating_sub(self.aces.len()));
        self.indices
            .reserve(seen.len().saturating_sub(self.indices.len()));
        for seen in seen {
            let index = self.sight(hour, seen, previous);
            self.active.insert(index);
        }
    }

    /// A read giving only the aces it gives otherwise than the read before, from which every
    /// other ace it holds goes on, and the aces it no longer holds.
    fn read_changes(
        &mut self,
        hour: i32,
        seen: Vec<Sighting>,
        gone: &[u64],
    ) -> io::Result<Touched> {
        let previous = self.reads.last().copied();
        let before = hour.saturating_sub(1);
        let mut left = Vec::with_capacity(gone.len());
        for id in gone {
            let index = *self
                .indices
                .get(id)
                .ok_or_else(|| dashboard_core::invalid("An ace left a roster it was never in"))?;
            if !self.active.remove(&index) {
                return Err(dashboard_core::invalid(
                    "An ace left a roster it was not in",
                ));
            }
            close_ace(&mut self.aces[index], self.read_through, before);
            left.push(index);
        }
        let mut given = Vec::with_capacity(seen.len());
        for seen in seen {
            if let Some(&index) = self.indices.get(&seen.row.id)
                && self.active.contains(&index)
            {
                close_ace(&mut self.aces[index], self.read_through, before);
            }
            let index = self.sight(hour, seen, previous);
            self.active.insert(index);
            given.push(index);
        }
        Ok(Touched::Changes {
            seen: given,
            gone: left,
        })
    }

    /// One ace a read at `hour` gave, the roster's read before it at `previous`: its index.
    fn sight(&mut self, hour: i32, seen: Sighting, previous: Option<i32>) -> usize {
        let index = match self.indices.entry(seen.row.id) {
            Entry::Occupied(entry) => *entry.get(),
            Entry::Vacant(entry) => {
                let index = self.aces.len();
                self.aces.push(Ace {
                    id: seen.row.id,
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
        self.variant_ids
            .extend(seen.variants.iter().flatten().copied());
        let ace = &mut self.aces[index];
        let continues = previous.is_some() && ace.last_read == previous;
        ace.last_read = Some(hour);
        if seen.row.alive != 0 {
            ace.last_alive = Some(hour);
            ace.dead_by = None;
        } else if ace.dead_by.is_none() && ace.last_alive.is_some() {
            ace.dead_by = Some(hour);
        }
        let last_alive = if seen.row.alive == 0 {
            ace.last_alive
        } else {
            None
        };
        if continues
            && ace
                .runs
                .last()
                .is_some_and(|run| run.facts.same(&seen, last_alive, ace.dead_by))
        {
            ace.runs.last_mut().expect("compared run").last = hour;
        } else {
            ace.runs.push(Run {
                first: hour,
                last: hour,
                facts: Facts::new(seen, last_alive, ace.dead_by),
            });
        }
        index
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
                let Ok(frames) = frames else {
                    result.unread = true;
                    result
                        .warnings
                        .insert("Unread replay block; ace history has a gap".into());
                    continue;
                };
                for frame in frames {
                    cancelled(flag)?;
                    let Ok(decoded) = Ok::<&DecodedFrame, std::io::Error>(&frame.decoded) else {
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

/// One scan per branch of the source revision.
#[cfg(test)]
#[derive(Default)]
struct Cache {
    branches: BTreeMap<u64, Arc<Scan>>,
}
#[cfg(test)]
fn cached(source: &Source, branch: u64, flag: &AtomicBool) -> io::Result<Arc<Scan>> {
    let state = source.state::<Cache>();
    let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(result) = state.branches.get(&branch) {
        return Ok(Arc::clone(result));
    }
    let result = Arc::new(scan(&source.replay(), branch, flag)?);
    state.branches.insert(branch, Arc::clone(&result));
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
                .and_then(|name| catalogue::plain_text(&name))
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
                let Ok(frames) = frames else {
                    unread = true;
                    continue;
                };
                for frame in frames {
                    cancelled(flag)?;
                    let Ok(decoded) = Ok::<&DecodedFrame, std::io::Error>(&frame.decoded) else {
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
    entries: HashMap<PortraitOperands, catalogue::Drawn<Result<String, String>>>,
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
            return found.get().clone();
        }
    }
    // The whole ladder is one operation: the visual mods' choice of sprite, or, when they cannot
    // draw it, the recording's own choice made again.
    let found = catalogue::draw(
        || {
            format!(
                "ace portrait {} {} {}",
                facts.tag.as_deref().unwrap_or(""),
                String::from_utf8_lossy(&facts.name),
                String::from_utf8_lossy(&facts.surname)
            )
        },
        cat,
        |cat| portrait(facts, cat),
    );
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    if state
        .catalogue
        .as_ref()
        .is_some_and(|previous| Arc::ptr_eq(previous, cat))
    {
        return state.entries.entry(key).or_insert(found).get().clone();
    }
    found.get().clone()
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
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
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
