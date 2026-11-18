//! The vision layer: what the viewing country sees, the values today's `perspective::Vision`
//! derives from a frame of rows, kept from the mirrors. The viewer, the relations, the unit
//! modifier, the intel lanes and the defines are reread whole when their small tables change;
//! the unit intel levels per unit; the province bytes per province from the ordered terms of
//! [`fold`]; the invasion sight per division at sea.
//!
//! Fog, intel, counters, unit models and the engagement layers read it. A table missing at the
//! head is what today's `Vision::read` makes of it: an undecided unit sight, a truncated census, a
//! relations gap or a visibility gap, by name; a gap step refuses the hour.

mod fold;
mod global;
mod worker;

use global::Global;

use super::{Budget, Carried, Gpu, Inputs, Layer, List, Num, Text, View};
use crate::playhead_host::mirror::{Mirror, Mirrors};
use playhead::Step;
use replay_columnar::manifest::Status;
use replay_data::rows::TableId;
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::Arc,
};
use viewer_foundation::map::{
    perspective::{self, Choice, FULL_VISIBILITY, LAND_FULL_LEVEL, LAND_SEEN_LEVEL, Parts, SHOWN_VISIBILITY, SharingUnknown, Sight, Vision as Seen},
    visibility::{self, FogContent},
};

/// The fog content the layer reads: province kinds and neighbours by id, and the `NIntel` defines.
pub struct Content {
    fog: Result<FogContent, String>,
    /// Per province id: 1 land, 2 sea, 0 neither.
    kind: Vec<u8>,
    /// Per province id: its native neighbours, `neighbors[start[id]..start[id + 1]]`; none for an
    /// id the topology does not hold.
    start: Vec<u32>,
    neighbors: Vec<u32>,
    held: Vec<bool>,
}

const LAND: u8 = 1;
const SEA: u8 = 2;

impl Content {
    pub fn new(fog: Result<FogContent, String>) -> Self {
        let mut content = Self { fog, kind: Vec::new(), start: vec![0], neighbors: Vec::new(), held: Vec::new() };
        if let Ok(fog) = &content.fog {
            let topology = &fog.topology;
            let largest = topology.land.iter().chain(&topology.sea).chain(topology.neighbors.keys()).copied().max().unwrap_or(0) as usize;
            content.kind = vec![0; largest + 1];
            for &id in &topology.land {
                content.kind[id as usize] = LAND;
            }
            for &id in &topology.sea {
                content.kind[id as usize] = SEA;
            }
            content.held = vec![false; largest + 1];
            content.start = Vec::with_capacity(largest + 2);
            content.start.push(0);
            for id in 0..=largest as u32 {
                if let Some(next) = topology.neighbors.get(&id) {
                    content.neighbors.extend(next);
                    content.held[id as usize] = true;
                }
                content.start.push(content.neighbors.len() as u32);
            }
        }
        content
    }

    fn land(&self, province: u32) -> bool {
        self.kind.get(province as usize) == Some(&LAND)
    }

    fn sea(&self, province: u32) -> bool {
        self.kind.get(province as usize) == Some(&SEA)
    }

    /// The province's neighbours, none when the topology does not hold it.
    fn neighbors(&self, province: u32) -> Option<&[u32]> {
        let at = province as usize;
        self.held.get(at).copied().unwrap_or(false).then(|| &self.neighbors[self.start[at] as usize..self.start[at + 1] as usize])
    }

    fn bytes(&self) -> usize {
        self.kind.capacity() + self.start.capacity() * 4 + self.neighbors.capacity() * 4 + self.held.capacity()
    }
}

/// The columns the layer reads.
#[derive(Clone, Copy)]
pub(super) struct Columns {
    link_slot: Num<u32>,
    country_slot: Num<u32>,
    country_tag: Text,
    player_country: Num<u32>,
    player_observer: Num<u32>,
    faction: Num<u64>,
    relation_links: Num<u8>,
    intel_viewer: Num<u32>,
    intel_level: Num<i8>,
    unit_modifier: Num<i64>,
    army: Num<i64>,
    air: Num<i64>,
    navy: Num<i64>,
    dlc_mask: Num<u32>,
    controller: Num<u32>,
    province_state: Num<u32>,
    state_owner: Num<u32>,
    state_controller: Num<u32>,
    network_modifier: Num<i64>,
    roster_owner: Num<u32>,
    division_owner: Num<u32>,
    division_province: Num<u32>,
    reveal_skip: Num<u8>,
    path: List,
    task_fleet: Num<u64>,
    task_province: Num<u32>,
    task_owner: Num<u32>,
    fleet_owner: Num<u32>,
    theatre_owner: Num<u32>,
    theatre_ordinal: Num<u32>,
    volunteers: Num<i32>,
    area_provinces: List,
    stationed_unit: Num<u64>,
    host: Num<u32>,
    radar_provinces: List,
    reserved_faction: Num<u64>,
    reverse_visibility: List,
    intel_sample: List,
    zero_war: List,
    zero_subject: List,
    forward_visibility: List,
}

/// A status as the layer reacts to it: a truncated table's missing count moves every hour and
/// changes nothing the layer reads.
fn kind(status: Option<Status>) -> Option<Status> {
    status.map(|status| match status {
        Status::Truncated { .. } => Status::Truncated { missing: 0 },
        status => status,
    })
}

/// The rows a table holds at the head when it is whole; none otherwise.
pub(super) fn complete(mirrors: &Mirrors, table: TableId) -> Option<&Mirror> {
    mirrors.get(table).filter(|mirror| mirror.complete())
}

/// The rows a table holds at the head, whole or truncated; none when unavailable, unrecorded or
/// in a gap.
pub(super) fn held(mirrors: &Mirrors, table: TableId) -> Option<&Mirror> {
    mirrors.get(table).filter(|mirror| matches!(mirror.status(), Some(Status::Complete | Status::Truncated { .. })))
}

/// The limitation notes vision's values make, in today's words and order, made on the fold's worker
/// and published with it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Notes {
    /// The viewing country and the fog switch.
    perspective: Vec<String>,
    fog: bool,
    /// With fog on: the undecided sight's and the visibility gap's lines.
    sight: Vec<String>,
    /// A fog is drawn: the fog of war's line is stated.
    fog_drawn: bool,
    /// With fog on: the perspective tables the hour truncated.
    truncated: Vec<String>,
}

impl Notes {
    /// The viewing country's line.
    pub fn perspective(&self) -> &[String] {
        &self.perspective
    }

    /// With fog on, the sight inputs' lines at `statuses`, then the undecided sight's and the gap's.
    pub fn sight(&self, statuses: &dyn viewer_foundation::map::tables::Statuses) -> Vec<String> {
        if !self.fog {
            return Vec::new();
        }
        let mut out = viewer_foundation::map::tables::Limitations::default();
        out.note_fog_on_sight(statuses);
        let mut lines = out.into_lines();
        lines.extend(self.sight.iter().cloned());
        lines
    }

    /// The fog of war's line over the intel texture's `fogged` cells, when a fog is drawn.
    pub fn fog_of_war(&self, fogged: usize) -> Option<String> {
        self.fog_drawn.then(|| {
            let mut out = viewer_foundation::map::tables::Limitations::default();
            out.note_fog_of_war(fogged, 0.0);
            out.into_lines().remove(0)
        })
    }

    /// Whether a fog is drawn, so the fog of war's line is owed.
    pub fn fog_drawn(&self) -> bool {
        self.fog_drawn
    }

    pub fn truncated(&self) -> &[String] {
        &self.truncated
    }
}

/// A whole vision's parts as the layer holds them, the province bytes as the fold's arrays (the
/// provinces held and every byte by id): made into today's type off the frame.
pub struct SeenCopy {
    bytes: Option<(Vec<u32>, Vec<u8>)>,
    parts: Parts,
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static MADE: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Visions made of a copy on this thread: the frame thread makes none.
#[cfg(any(test, feature = "test-support"))]
pub fn seen_made_here() -> u64 {
    MADE.with(std::cell::Cell::get)
}

impl SeenCopy {
    pub fn make(self) -> Seen {
        #[cfg(any(test, feature = "test-support"))]
        MADE.with(|made| made.set(made.get() + 1));
        let Self { bytes, mut parts } = self;
        if let Some((present, bytes)) = bytes {
            parts.visibility = present.into_iter().map(|id| (id, bytes[id as usize])).collect();
        }
        Seen::from_parts(parts)
    }
}

/// The vision layer's arrays.
pub struct Vision {
    content: Arc<Content>,
    pub(super) columns: Option<Columns>,
    choice: Choice,
    /// The whole parts as the worker last made them.
    global: Global,
    /// Per unit: its rows' ordinals and slots in UnitIntel, and the first level keyed to the
    /// viewer.
    unit_rows: HashMap<u64, Vec<(u32, u32)>>,
    levels: BTreeMap<u64, i8>,
    units: Dirty64,
    lanes_dirty: bool,
    /// CountryIntel rows whose air or navy lane moved: raw owner and target.
    lane_changes: Vec<(u32, u32)>,
    /// What the applies marked, read at the settle: UnitIntel's rows, all of its units (its status
    /// moved), and CountryIntel's slots whose air or navy lane was written.
    intel: Vec<Intel>,
    intel_all: bool,
    lane_slots: Vec<u32>,
    air_intel: Option<BTreeMap<u32, i64>>,
    navy_intel: Option<BTreeMap<u32, i64>>,
    truncated_country_intel: bool,
    province_controllers: BTreeMap<u32, u32>,
    fold: worker::Folder,
    statuses: HashMap<usize, Option<Status>>,
    gap: bool,
    pub(super) frame: u64,
    /// Units, provinces and terms the steps have marked.
    marked: [u64; 3],
    /// Nanoseconds on the frame handing steps to the worker.
    copying: u64,
    /// What a unit's sight reads besides its province's byte, its level and its sight province,
    /// and how many times it changed: a reader of unit sight rereads every unit only then.
    sight_key: Option<SightKey>,
    sight_epoch: u64,
    /// The perspective changed since the fold last started: it starts over at the view's change.
    rechoose: bool,
    /// Mirror rows its applies read: none.
    #[cfg(any(test, feature = "test-support"))]
    applied_reads: u64,
}

/// A UnitIntel row an apply marked: gone or come (key and slot), or written (slot).
#[derive(Clone, Copy, Debug)]
enum Intel {
    Gone(u128, u32),
    Came(u128, u32),
    Written(u32),
}

/// The whole parts a unit's sight reads ([`Vision::sees_land`], today's `sees_navy` and a task
/// force's priority).
#[derive(Debug, PartialEq)]
struct SightKey {
    ours: u32,
    fog: bool,
    undecided: Option<&'static str>,
    gap: bool,
    unit_modifier: i64,
    truncated_unit_intel: bool,
    shares: BTreeMap<u32, ()>,
    wars: BTreeMap<u32, ()>,
    relations_gap: Option<&'static str>,
}

/// Dirty 64-bit keys, each once, in the order first marked, each with the frame it was marked at.
#[derive(Debug, Default)]
struct Dirty64 {
    list: Vec<(u64, u64)>,
    marked: std::collections::HashSet<u64>,
}

impl Dirty64 {
    fn mark(&mut self, key: u64, frame: u64) -> bool {
        if !self.marked.insert(key) {
            return false;
        }
        self.list.push((key, frame));
        true
    }

    /// The frame the oldest key still dirty was marked at.
    fn since(&self) -> Option<u64> {
        self.list.first().map(|&(_, frame)| frame)
    }

    fn settle(&mut self, budget: &mut Budget, mut each: impl FnMut(u64)) {
        let mut done = 0;
        while done < self.list.len() {
            if done % 32 == 0 && !budget.left() {
                break;
            }
            let (key, _) = self.list[done];
            self.marked.remove(&key);
            done += 1;
            each(key);
        }
        self.list.drain(..done);
    }

    fn bytes(&self) -> usize {
        self.list.capacity() * 16 + self.marked.capacity() * 16
    }
}

impl Vision {
    pub fn new(content: Arc<Content>) -> Self {
        let fold = worker::Folder::new(Arc::clone(&content));
        Self {
            content,
            columns: None,
            choice: Choice::default(),
            global: Global::unread("not composed yet"),
            unit_rows: HashMap::new(),
            levels: BTreeMap::new(),
            units: Dirty64::default(),
            lanes_dirty: true,
            lane_changes: Vec::new(),
            intel: Vec::new(),
            intel_all: false,
            lane_slots: Vec::new(),
            air_intel: None,
            navy_intel: None,
            truncated_country_intel: false,
            province_controllers: BTreeMap::new(),
            fold,
            statuses: HashMap::new(),
            gap: false,
            frame: 0,
            marked: [0; 3],
            copying: 0,
            sight_key: None,
            sight_epoch: 0,
            rechoose: false,
            #[cfg(any(test, feature = "test-support"))]
            applied_reads: 0,
        }
    }

    pub fn fresh(&self) -> Self {
        Self { columns: self.columns, choice: self.choice.clone(), ..Self::new(Arc::clone(&self.content)) }
    }

    /// Why the hour has no vision, by name; none when it has one.
    pub fn refused(&self) -> Option<String> {
        if self.gap {
            return Some("the hour is a gap in the recording".into());
        }
        self.global.failed.clone()
    }

    /// The vision as today's type, made from the layer's arrays: what the proof compares and what
    /// a reader of today's type takes.
    pub fn vision(&self) -> Option<Seen> {
        self.refused().is_none().then(|| self.copy().make())
    }

    /// What [`Self::shown`] makes, copied at the cost of the layer's arrays, to be made into the
    /// vision elsewhere ([`SeenCopy::make`]): the province bytes as two arrays, not a map.
    pub fn shown_copy(&self) -> Result<SeenCopy, String> {
        if self.fold.published() == crate::layers::Published::Nothing {
            return Err("the vision layer has published nothing yet".into());
        }
        match self.refused() {
            Some(why) => Err(why),
            None => Ok(self.copy()),
        }
    }

    fn copy(&self) -> SeenCopy {
        let global = &self.global;
        let mut truncated = Vec::new();
        if global.truncated_unit_intel {
            truncated.push("UnitIntel");
        }
        if self.truncated_country_intel {
            truncated.push("CountryIntel");
        }
        let (bytes, visibility_gap, sight_provinces) = if global.fog {
            match self.fold.gap(&self.content) {
                Some(reason) => (None, Some(reason), BTreeMap::new()),
                None => (Some(self.fold.bytes_copy()), None, self.fold.sight()),
            }
        } else {
            (None, None, BTreeMap::new())
        };
        SeenCopy {
            bytes,
            parts: Parts {
                ours: global.ours,
                resolution: global.resolution.clone(),
                fog: global.fog,
                undecided: global.undecided,
                shares: global.shares.clone(),
                wars: global.wars.clone(),
                relations_gap: global.relations.as_ref().err().copied(),
                levels: self.levels.clone(),
                unit_modifier: global.unit_modifier,
                visibility: BTreeMap::new(),
                visibility_gap,
                sight_provinces,
                truncated,
                air_intel: self.air_intel.clone(),
                air_defines: global.air_defines.clone(),
                navy_intel: self.navy_intel.clone(),
                naval_defines: global.naval_defines.clone(),
                air_access: global.air_access.clone(),
                province_controllers: self.province_controllers.clone(),
            },
        }
    }

    /// Its limitation notes as its worker last made them; none while it refuses the hour.
    pub fn notes(&self) -> Option<&Notes> {
        self.refused().is_none().then(|| self.fold.notes())
    }

    /// The perspective it is drawn at.
    pub fn choice(&self) -> &Choice {
        &self.choice
    }

    pub fn marked(&self) -> [u64; 3] {
        self.marked
    }

    /// Microseconds the province terms' settles spent, by part: cutoffs, sources, provinces,
    /// divisions at sea.
    pub fn spent(&self) -> [u64; 4] {
        self.fold.spent().map(|ns| ns / 1000)
    }

    /// Microseconds the fold spent on the worker, and its folds, since the layer was made.
    pub fn worker(&self) -> (u64, u64) {
        let (ns, runs) = self.fold.worker();
        (ns / 1000, runs)
    }

    /// Microseconds on the frame handing steps to the worker since the layer was made.
    pub fn copying_us(&self) -> u64 {
        self.copying / 1000
    }

    /// Every term made again since the layer was made, by the part of the whole parts that moved.
    pub fn rebuilds(&self) -> [u64; 4] {
        self.fold.rebuilds()
    }

    pub fn content_bytes(&self) -> usize {
        self.content.bytes()
    }

    /// The newest hour its worker has made a product of, as received on the frame.
    #[cfg(any(test, feature = "test-support"))]
    pub fn made_through(&self) -> Option<replay_data::Hour> {
        self.fold.made_through()
    }

    fn columns(&self) -> Columns {
        self.columns.expect("the vision layer is declared before it composes")
    }

    fn mark_unit(&mut self, unit: u64) {
        if self.units.mark(unit, self.frame) {
            self.marked[0] += 1;
        }
    }

    /// Whether a fog is drawn: fog on, a vision, and no visibility gap.
    pub fn fog_drawn(&self) -> bool {
        self.refused().is_none() && self.global.fog && self.fold.gap(&self.content).is_none()
    }

    /// A province's byte as the intel texture reads it.
    pub fn intel_byte(&self, province: u32) -> u8 {
        self.fold.intel_byte(province)
    }

    /// Every province's byte as the terms make it, whether or not a gap withholds the fog: what
    /// stepping must equal when today's product draws none.
    pub fn terms_bytes(&self) -> BTreeMap<u32, u8> {
        self.fold.province_bytes()
    }

    /// A province's byte as today's `Vision::province_byte` gives it: 255 with fog off, none under
    /// a visibility gap or for a province the Provinces table does not hold.
    pub fn province_byte(&self, province: u32) -> Option<u8> {
        if !self.global.fog {
            return Some(u8::MAX);
        }
        if self.fold.gap(&self.content).is_some() {
            return None;
        }
        self.fold.province_byte(province)
    }

    /// The viewing country's canonical slot; none without a vision.
    pub fn ours(&self) -> Option<u32> {
        self.refused().is_none().then_some(self.global.ours)
    }

    /// How much of one land unit standing in `province` the viewer is shown, read in place:
    /// today's `Vision::sees_land`; none without a vision.
    pub fn sees_land(&self, unit: u64, owner: u32, province: u32) -> Option<Sight> {
        self.refused().is_none().then(|| {
            let shared = self.global.shares.contains_key(&owner);
            let at = self.fold.sight_of(unit).unwrap_or(province);
            self.sight_parts().land(owner, shared, self.levels.get(&unit).copied(), self.fold.province_byte(at))
        })
    }

    /// The whole parts a land unit's sight reads besides its own level and province byte.
    fn sight_parts(&self) -> SightParts {
        let global = &self.global;
        SightParts {
            ours: global.ours,
            fog: global.fog,
            undecided: global.undecided.is_some(),
            gap: global.fog && self.fold.gap(&self.content).is_some(),
            truncated_unit_intel: global.truncated_unit_intel,
            unit_modifier: global.unit_modifier,
        }
    }

    /// What a division's sight reads of the layer, whole: for a reader that keeps its own copy
    /// ([`SightCopy`]) on another thread.
    pub fn sight_copy(&self) -> SightCopy {
        SightCopy {
            keys: self.sight_keys(),
            levels: self.levels.clone(),
            bytes: self.fold.province_bytes(),
            sight: self.fold.sight(),
        }
    }

    /// What a division's sight reads of the layer as a whole, to tell a later change by.
    pub fn sight_keys(&self) -> SightKeys {
        SightKeys {
            refused: self.refused(),
            choice: self.choice.clone(),
            epoch: self.sight_epoch,
            parts: self.sight_parts(),
            shares: self.global.shares.clone(),
            relations_gap: self.global.relations.as_ref().err().copied(),
        }
    }

    /// What changed of a division's sight since `held` (the keys of the copy last made or
    /// changed), named by the changed lists: whole when anything but a unit's or a province's own
    /// part moved; none when nothing did.
    pub fn sight_change(&self, changed: &crate::layers::Changed, held: &mut Option<SightKeys>) -> Option<SightChange> {
        let keys = self.sight_keys();
        let whole = held.as_ref() != Some(&keys) || changed.all();
        let units: Vec<(u64, Option<i8>, Option<u32>)> = changed.of("unit").iter().map(|&unit| (unit, self.levels.get(&unit).copied(), self.fold.sight_of(unit))).collect();
        let provinces: Vec<(u32, Option<u8>)> = changed.of("province").iter().map(|&province| (province as u32, self.fold.province_byte(province as u32))).collect();
        if !whole && units.is_empty() && provinces.is_empty() {
            return None;
        }
        *held = Some(keys);
        Some(SightChange { whole: whole.then(|| Box::new(self.sight_copy())), units, provinces })
    }

    /// The province whose byte [`Self::sees_land`] reads for a land unit standing in `province`.
    /// It moves only with a change of the unit's sight, which the changed lists name ("unit").
    pub fn sight_province(&self, unit: u64, province: u32) -> u32 {
        self.fold.sight_of(unit).unwrap_or(province)
    }

    /// Moves on whenever what a unit's sight reads besides its province's byte, its level and its
    /// sight province changed (the viewer, the fog, the relations, the modifier, a gap), and at a
    /// compose; between two of its values the changed lists name every unit and province whose sight
    /// moved.
    pub fn sight_epoch(&self) -> u64 {
        self.sight_epoch
    }

    fn sight_key(&self) -> SightKey {
        let global = &self.global;
        SightKey {
            ours: global.ours,
            fog: global.fog,
            undecided: global.undecided,
            gap: self.fold.gap(&self.content).is_some(),
            unit_modifier: global.unit_modifier,
            truncated_unit_intel: global.truncated_unit_intel,
            shares: global.shares.clone(),
            wars: global.wars.clone(),
            relations_gap: global.relations.as_ref().err().copied(),
        }
    }

    /// Today's vision with only what the sight of `units` reads, made from the layer's arrays: their
    /// levels, the viewer, the fog, the relations and the modifier, but no province, so it answers
    /// `sees_navy`, `shares_vision_proven` and `at_war_proven` for those units and nothing that
    /// reads a province's byte. None without a vision.
    pub fn unit_vision(&self, units: impl Iterator<Item = u64>) -> Option<Seen> {
        if self.refused().is_some() {
            return None;
        }
        let global = &self.global;
        let levels = units.filter_map(|unit| Some((unit, *self.levels.get(&unit)?))).collect();
        Some(Seen::from_parts(Parts {
            ours: global.ours,
            resolution: global.resolution.clone(),
            fog: global.fog,
            undecided: global.undecided,
            shares: global.shares.clone(),
            wars: global.wars.clone(),
            relations_gap: global.relations.as_ref().err().copied(),
            levels,
            unit_modifier: global.unit_modifier,
            visibility: BTreeMap::new(),
            visibility_gap: if global.fog { self.fold.gap(&self.content) } else { None },
            sight_provinces: BTreeMap::new(),
            truncated: if global.truncated_unit_intel { vec!["UnitIntel"] } else { Vec::new() },
            air_intel: None,
            air_defines: global.air_defines.clone(),
            navy_intel: None,
            naval_defines: global.naval_defines.clone(),
            air_access: None,
            province_controllers: BTreeMap::new(),
        }))
    }

    /// Whether `owner` shares vision with the viewer, today's `Vision::shares_vision_proven`;
    /// none without a vision.
    pub fn shares_vision_proven(&self, owner: u32) -> Option<Result<bool, SharingUnknown>> {
        self.refused().is_none().then(|| {
            if self.global.shares.contains_key(&owner) {
                return Ok(true);
            }
            self.global.relations.as_ref().err().map_or(Ok(false), |&input| Err(SharingUnknown::Relations(input)))
        })
    }

    /// The marks of one step ([`Layer::apply`]).
    fn mark(&mut self, step: &Step) {
        let Some(columns) = self.columns else { return };
        self.fold.applied(step);
        self.gap = step.gap();
        if self.gap {
            return;
        }
        for table in [TableId::UnitIntel, TableId::CountryIntel] {
            let Some(changes) = step.table(table.index()) else { continue };
            if let Some((status, _)) = changes.status()
                && self.statuses.insert(changes.table(), kind(Some(status))) != Some(kind(Some(status)))
            {
                match table {
                    TableId::UnitIntel => self.intel_all = true,
                    _ => self.lanes_dirty = true,
                }
            }
            let (deleted, inserted) = (changes.deleted(), changes.inserted());
            if table == TableId::UnitIntel {
                let key = |keys: Option<&[u128]>, at: usize| keys.map_or(0, |keys| keys[at]);
                self.intel.extend(deleted.slots().iter().enumerate().map(|(at, &slot)| Intel::Gone(key(deleted.keys(), at), slot)));
                self.intel.extend(inserted.slots().iter().enumerate().map(|(at, &slot)| Intel::Came(key(inserted.keys(), at), slot)));
                for written in changes.columns().iter().filter(|written| [columns.intel_viewer.column, columns.intel_level.column].contains(&written.column())) {
                    self.intel.extend(written.slots().iter().map(|&slot| Intel::Written(slot)));
                }
            } else {
                for &key in deleted.keys().unwrap_or_default().iter().chain(inserted.keys().unwrap_or_default()) {
                    self.lane_changes.push(((key >> 32) as u32, key as u32));
                }
                for written in changes.columns().iter().filter(|written| [columns.air.column, columns.navy.column].contains(&written.column())) {
                    self.lane_slots.extend_from_slice(written.slots());
                }
            }
        }
    }

    /// Mirror rows its applies read since it was made.
    #[cfg(any(test, feature = "test-support"))]
    pub fn applied_reads(&self) -> u64 {
        self.applied_reads
    }

    /// The rows the applies marked, in order: UnitIntel's units and CountryIntel's lanes, a written
    /// slot's row as the mirror holds it now.
    fn take_marks(&mut self, mirrors: &Mirrors) {
        let rows = mirrors.get(TableId::UnitIntel);
        let mut intel = std::mem::take(&mut self.intel);
        for mark in intel.drain(..) {
            match mark {
                Intel::Gone(key, slot) => {
                    let unit = (key >> 32) as u64;
                    if let Some(rows) = self.unit_rows.get_mut(&unit) {
                        rows.retain(|&(ordinal, held)| !(held == slot && ordinal == key as u32));
                        if rows.is_empty() {
                            self.unit_rows.remove(&unit);
                        }
                    }
                    self.mark_unit(unit);
                }
                Intel::Came(key, slot) => {
                    let unit = (key >> 32) as u64;
                    self.unit_rows.entry(unit).or_default().push((key as u32, slot));
                    self.mark_unit(unit);
                }
                Intel::Written(slot) => {
                    if let Some(key) = rows.and_then(|rows| rows.key(slot)) {
                        self.mark_unit((key >> 32) as u64);
                    }
                }
            }
        }
        self.intel = intel;
        if std::mem::take(&mut self.intel_all) {
            let units: Vec<u64> = self.unit_rows.keys().copied().collect();
            units.into_iter().for_each(|unit| self.mark_unit(unit));
        }
        let lanes = mirrors.get(TableId::CountryIntel);
        if let Some(lanes) = lanes {
            self.lane_changes.extend(self.lane_slots.iter().filter_map(|&slot| lanes.key(slot)).map(|key| ((key >> 32) as u32, key as u32)));
        }
        self.lane_slots.clear();
    }

    /// One unit's first level keyed to the viewer, from its UnitIntel rows.
    fn settle_unit(&mut self, unit: u64, mirrors: &Mirrors) {
        let columns = self.columns();
        let ours = self.global.ours;
        let level = held(mirrors, TableId::UnitIntel).and_then(|rows| {
            let mut held = self.unit_rows.get(&unit)?.clone();
            held.sort_unstable();
            held.iter().find(|&&(_, slot)| rows.live(slot) && rows.number(columns.intel_viewer, slot).value() == Some(ours)).and_then(|&(_, slot)| rows.number(columns.intel_level, slot).value())
        });
        let old = match level {
            Some(level) => self.levels.insert(unit, level),
            None => self.levels.remove(&unit),
        };
        if old != level {
            self.fold.mark("unit", unit);
        }
    }

    /// The viewer's air and navy lanes from its references' CountryIntel rows, first reference
    /// first.
    fn read_lanes(&mut self, mirrors: &Mirrors) {
        let columns = self.columns();
        self.truncated_country_intel = matches!(mirrors.get(TableId::CountryIntel).and_then(Mirror::status), Some(Status::Truncated { .. }));
        let (air, navy) = match complete(mirrors, TableId::CountryIntel) {
            None => (None, None),
            Some(rows) => {
                let (mut air, mut navy) = (BTreeMap::new(), BTreeMap::new());
                for &raw in self.global.raws.get(&self.global.ours).into_iter().flatten() {
                    for (target, slot) in super::owned::owner_rows(rows, raw) {
                        if let Some(value) = rows.number(columns.air, slot).value() {
                            air.entry(target).or_insert(value);
                        }
                        if let Some(value) = rows.number(columns.navy, slot).value() {
                            navy.entry(target).or_insert(value);
                        }
                    }
                }
                (Some(air), Some(navy))
            }
        };
        if (&air, &navy) != (&self.air_intel, &self.navy_intel) {
            self.fold.touch_all();
        }
        (self.air_intel, self.navy_intel) = (air, navy);
    }

    /// The viewer's lanes for the targets whose rows moved: the first of its references with a row.
    fn read_lane_targets(&mut self, mirrors: &Mirrors) {
        let columns = self.columns();
        let mut targets: Vec<u32> = std::mem::take(&mut self.lane_changes).into_iter().filter(|(owner, _)| self.global.links.get(owner) == Some(&self.global.ours)).map(|(_, target)| target).collect();
        targets.sort_unstable();
        targets.dedup();
        let (Some(rows), Some(air), Some(navy)) = (complete(mirrors, TableId::CountryIntel), self.air_intel.as_mut(), self.navy_intel.as_mut()) else { return };
        let raws = self.global.raws.get(&self.global.ours).map_or(&[][..], Vec::as_slice);
        let mut moved = Vec::new();
        for target in targets {
            let slot = raws.iter().find_map(|&raw| rows.slot(u128::from(raw) << 32 | u128::from(target)));
            for (lane, column) in [(&mut *air, columns.air), (&mut *navy, columns.navy)] {
                let value = slot.and_then(|slot| rows.number(column, slot).value());
                let old = match value {
                    Some(value) => lane.insert(target, value),
                    None => lane.remove(&target),
                };
                if old != value {
                    moved.push(target);
                }
            }
        }
        for target in moved {
            self.fold.mark("lane", u64::from(target));
        }
    }

    /// The whole parts the worker made, taken in: what their change moves on the frame marked.
    fn adopt(&mut self, global: Global) {
        let ours = global.ours != self.global.ours || global.failed.is_some() != self.global.failed.is_some();
        let links = global.links != self.global.links;
        self.global = global;
        if ours {
            let units: Vec<u64> = self.unit_rows.keys().copied().collect();
            units.into_iter().for_each(|unit| self.mark_unit(unit));
            self.lanes_dirty = true;
        }
        if links {
            self.lanes_dirty = true;
        }
    }

    fn settle_dirty(&mut self, mirrors: &Mirrors, budget: &mut Budget) {
        self.frame += 1;
        self.take_marks(mirrors);
        let before = (self.fog_drawn(), self.global.ours);
        self.settle_parts(mirrors, budget);
        if before != (self.fog_drawn(), self.global.ours) {
            self.fold.touch_all();
        }
        // Every part of the key moves only with a change published as everything.
        if self.sight_key.is_none() || self.fold.changed.all() {
            let key = self.sight_key();
            if self.sight_key.as_ref() != Some(&key) {
                self.sight_key = Some(key);
                self.sight_epoch += 1;
            }
        }
    }

    fn settle_parts(&mut self, mirrors: &Mirrors, budget: &mut Budget) {
        self.fold.exchange(budget.unlimited_now());
        if self.gap {
            return;
        }
        if let Some(global) = self.fold.take_global() {
            self.adopt(global);
        }
        for (province, controller) in self.fold.take_controllers() {
            match controller {
                Some(slot) => self.province_controllers.insert(province, slot),
                None => self.province_controllers.remove(&province),
            };
            self.fold.mark("province", u64::from(province));
        }
        if self.global.failed.is_some() {
            return;
        }
        if self.lanes_dirty {
            self.lanes_dirty = false;
            self.lane_changes.clear();
            self.read_lanes(mirrors);
        } else if !self.lane_changes.is_empty() {
            self.read_lane_targets(mirrors);
        }
        let mut units = std::mem::take(&mut self.units);
        units.settle(budget, |unit| self.settle_unit(unit, mirrors));
        self.units = units;
    }

    /// The vision the layer shows as today's type, made now (every province's byte: not a read for
    /// every frame); loading by name while its worker has published nothing, and why the hour has
    /// no vision otherwise.
    pub fn shown(&self) -> Result<Seen, String> {
        self.shown_copy().map(SeenCopy::make)
    }
}

/// The layer while the map draws it: declared, composed and not refused. Its [`Vision::ours`] is
/// the viewing country the window's readers and its histories' continuity take.
pub fn drawn(side: &crate::playhead_host::MapSide) -> Option<&Vision> {
    if !side.draws("vision") {
        return None;
    }
    side.parts()?.1.iter().find_map(|layer| layer.any().downcast_ref::<Vision>())
}

/// The whole parts a land unit's sight reads besides its own level and province byte.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SightParts {
    ours: u32,
    fog: bool,
    undecided: bool,
    gap: bool,
    truncated_unit_intel: bool,
    unit_modifier: i64,
}

impl SightParts {
    /// Today's sight of a land unit of `owner`, with its level, whether its owner shares vision
    /// with the viewer, and the byte of the province its sight reads.
    fn land(self, owner: u32, shared: bool, level: Option<i8>, byte: Option<u8>) -> Sight {
        let known = shared || level.is_some() || !self.truncated_unit_intel;
        if !self.fog || owner == self.ours || self.undecided || self.gap || !known {
            return Sight::Full;
        }
        let level = if shared { 3 } else { level.unwrap_or(0) };
        let ladder = match level {
            level if level >= LAND_FULL_LEVEL => FULL_VISIBILITY,
            level if level >= LAND_SEEN_LEVEL => SHOWN_VISIBILITY,
            _ => 0,
        };
        let base = byte.map_or(ladder, |byte| ladder.max(u16::from(byte)));
        match visibility::unit_byte(base, self.unit_modifier) {
            byte if byte >= FULL_VISIBILITY => Sight::Full,
            byte if byte >= SHOWN_VISIBILITY => Sight::Partial,
            _ => Sight::None,
        }
    }
}

/// What a division's sight reads of the vision layer as a whole: why it has no vision, the
/// perspective and its sight epoch, the whole parts, who shares vision, and the relations' gap.
#[derive(Clone, Debug, PartialEq)]
pub struct SightKeys {
    refused: Option<String>,
    choice: Choice,
    epoch: u64,
    parts: SightParts,
    shares: BTreeMap<u32, ()>,
    relations_gap: Option<&'static str>,
}

/// A copy of what a division's sight reads of the vision layer, for a reader on another thread:
/// made whole ([`Vision::sight_copy`]) and kept from the layer's changes ([`Vision::sight_change`]).
/// It answers as the layer does.
#[derive(Clone, Debug)]
pub struct SightCopy {
    keys: SightKeys,
    levels: BTreeMap<u64, i8>,
    bytes: BTreeMap<u32, u8>,
    sight: BTreeMap<u64, u32>,
}

/// What changed of a division's sight: the whole copy when anything but a unit's or a province's
/// own part did; the units named (their level and sight province) and the provinces named (their
/// byte) always, which a reader marks.
pub struct SightChange {
    whole: Option<Box<SightCopy>>,
    units: Vec<(u64, Option<i8>, Option<u32>)>,
    provinces: Vec<(u32, Option<u8>)>,
}

impl SightChange {
    /// The whole copy, when anything but a unit's or a province's own part changed.
    pub fn whole(&self) -> Option<&SightCopy> {
        self.whole.as_deref()
    }

    pub fn units(&self) -> impl Iterator<Item = u64> + '_ {
        self.units.iter().map(|&(unit, _, _)| unit)
    }

    pub fn provinces(&self) -> impl Iterator<Item = u64> + '_ {
        self.provinces.iter().map(|&(province, _)| u64::from(province))
    }
}

impl SightCopy {
    /// The layer's change applied.
    pub fn apply(&mut self, change: &SightChange) {
        if let Some(whole) = &change.whole {
            *self = SightCopy::clone(whole);
            return;
        }
        for &(unit, level, sight) in &change.units {
            match level {
                Some(level) => self.levels.insert(unit, level),
                None => self.levels.remove(&unit),
            };
            match sight {
                Some(province) => self.sight.insert(unit, province),
                None => self.sight.remove(&unit),
            };
        }
        for &(province, byte) in &change.provinces {
            match byte {
                Some(byte) => self.bytes.insert(province, byte),
                None => self.bytes.remove(&province),
            };
        }
    }

    pub fn refused(&self) -> Option<String> {
        self.keys.refused.clone()
    }

    pub fn choice(&self) -> &Choice {
        &self.keys.choice
    }

    pub fn sight_epoch(&self) -> u64 {
        self.keys.epoch
    }

    pub fn ours(&self) -> Option<u32> {
        self.keys.refused.is_none().then_some(self.keys.parts.ours)
    }

    /// As [`Vision::sees_land`].
    pub fn sees_land(&self, unit: u64, owner: u32, province: u32) -> Option<Sight> {
        self.keys.refused.is_none().then(|| {
            let shared = self.keys.shares.contains_key(&owner);
            let at = self.sight.get(&unit).copied().unwrap_or(province);
            self.keys.parts.land(owner, shared, self.levels.get(&unit).copied(), self.bytes.get(&at).copied())
        })
    }

    /// As [`Vision::sight_province`].
    pub fn sight_province(&self, unit: u64, province: u32) -> u32 {
        self.sight.get(&unit).copied().unwrap_or(province)
    }

    /// As [`Vision::shares_vision_proven`].
    pub fn shares_vision_proven(&self, owner: u32) -> Option<Result<bool, SharingUnknown>> {
        self.keys.refused.is_none().then(|| {
            if self.keys.shares.contains_key(&owner) {
                return Ok(true);
            }
            self.keys.relations_gap.map_or(Ok(false), |input| Err(SharingUnknown::Relations(input)))
        })
    }

    pub fn bytes(&self) -> usize {
        (self.levels.len() + self.bytes.len() + self.sight.len()) * 24
    }
}

/// The vision layer of a loaded recording's content.
pub fn for_document(document: &crate::document::Document) -> Box<dyn Layer> {
    Box::new(Vision::new(Arc::new(Content::new(document.content.products.fog.as_ref().clone()))))
}

impl Layer for Vision {
    fn name(&self) -> &'static str {
        "vision"
    }

    fn placed(&mut self, _view: &crate::layers::View) -> Option<u64> {
        Some(0)
    }

    fn inputs(&mut self, view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        if view.perspective != self.choice {
            self.choice = view.perspective.clone();
            self.rechoose = true;
        }
        for table in perspective::VISION_TABLES {
            inputs.table(*table, true)?;
        }
        use TableId as T;
        self.columns = Some(Columns {
            link_slot: inputs.number(T::CountryLinks, "canonical_slot")?,
            country_slot: inputs.number(T::Countries, "canonical_slot")?,
            country_tag: inputs.text(T::Countries, "tag")?,
            player_country: inputs.number(T::LocalPlayer, "country")?,
            player_observer: inputs.number(T::LocalPlayer, "observer_country")?,
            faction: inputs.number(T::FactionMembers, "faction")?,
            relation_links: inputs.number(T::RelationLinks, "links")?,
            intel_viewer: inputs.number(T::UnitIntel, "viewer")?,
            intel_level: inputs.number(T::UnitIntel, "level")?,
            unit_modifier: inputs.number(T::CountryVisibility, "unit_modifier")?,
            army: inputs.number(T::CountryIntel, "army")?,
            air: inputs.number(T::CountryIntel, "air")?,
            navy: inputs.number(T::CountryIntel, "navy")?,
            dlc_mask: inputs.number(T::SimulationFeatures, "dlc_mask")?,
            controller: inputs.number(T::Provinces, "controller")?,
            province_state: inputs.number(T::Provinces, "state")?,
            state_owner: inputs.number(T::States, "owner")?,
            state_controller: inputs.number(T::States, "controller")?,
            network_modifier: inputs.number(T::StateVisibility, "network_modifier")?,
            roster_owner: inputs.number(T::Divisions, "roster_owner")?,
            division_owner: inputs.number(T::Divisions, "logical_owner")?,
            division_province: inputs.number(T::Divisions, "province")?,
            reveal_skip: inputs.number(T::Divisions, "reveal_skip")?,
            path: inputs.list(T::DivisionMovement, "path")?,
            task_fleet: inputs.number(T::TaskForces, "fleet")?,
            task_province: inputs.number(T::TaskForces, "province")?,
            task_owner: inputs.number(T::TaskForces, "logical_owner")?,
            fleet_owner: inputs.number(T::Fleets, "owner")?,
            theatre_owner: inputs.number(T::LandTheatres, "owner")?,
            theatre_ordinal: inputs.number(T::LandTheatres, "ordinal")?,
            volunteers: inputs.number(T::LandTheatres, "volunteers_theatre")?,
            area_provinces: inputs.list(T::TheatreAreas, "provinces")?,
            stationed_unit: inputs.number(T::StationedUnits, "unit")?,
            host: inputs.number(T::VolunteerHosts, "host")?,
            radar_provinces: inputs.list(T::RadarCoverage, "provinces")?,
            reserved_faction: inputs.number(T::ReservedCountryNaming, "faction")?,
            reverse_visibility: inputs.list(T::ReservedCountryNaming, "reverse_visibility")?,
            intel_sample: inputs.list(T::ReservedCountryNaming, "country_intel_sample")?,
            zero_war: inputs.list(T::CountryCounterInputs, "zero_war")?,
            zero_subject: inputs.list(T::CountryCounterInputs, "zero_subject")?,
            forward_visibility: inputs.list(T::CountryCounterInputs, "forward_visibility")?,
        });
        Ok(())
    }

    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        let (marked, copying, epoch) = (self.marked, self.copying, self.sight_epoch);
        let fold = std::mem::replace(&mut self.fold, worker::Folder::new(Arc::clone(&self.content)));
        *self = Self { columns: self.columns, choice: self.choice.clone(), copying, fold, sight_epoch: epoch + 1, ..Self::new(Arc::clone(&self.content)) };
        self.fold.reset(mirrors, &self.columns(), &self.choice);
        self.statuses = mirrors.iter().map(|mirror| (mirror.table().index(), kind(mirror.status()))).collect();
        if let Some(rows) = mirrors.get(TableId::UnitIntel) {
            for slot in rows.rows() {
                if let Some(key) = rows.key(slot) {
                    self.unit_rows.entry((key >> 32) as u64).or_default().push((key as u32, slot));
                }
            }
        }
        let units: Vec<u64> = self.unit_rows.keys().copied().collect();
        units.into_iter().for_each(|unit| self.mark_unit(unit));
        self.settle_dirty(mirrors, &mut Budget::unlimited());
        self.marked = marked;
        Ok(())
    }

    /// Marks only, of its two tables the frame reads: UnitIntel's rows gone, come and written,
    /// and CountryIntel's air and navy lanes; read at the settle.
    fn apply(&mut self, step: &Step, _mirrors: &Mirrors) {
        #[cfg(any(test, feature = "test-support"))]
        let reads = crate::playhead_host::mirror::probe::reads();
        self.mark(step);
        #[cfg(any(test, feature = "test-support"))]
        {
            self.applied_reads += crate::playhead_host::mirror::probe::reads() - reads;
        }
    }

    fn wants_ahead(&self) -> bool {
        true
    }

    /// A perspective change starts the fold over from the mirrors at the hour applied: the worker
    /// may hold steps folded past it, which it cannot undo. One copy of its tables, on the frame,
    /// at the user's change.
    fn viewed(&mut self, _view: &View, mirrors: &Mirrors) {
        if std::mem::take(&mut self.rechoose)
            && let Some(columns) = self.columns
        {
            self.fold.restart(mirrors, &columns, &self.choice);
        }
    }

    fn void_ahead(&mut self, mirrors: &Mirrors, _kept: Option<replay_data::Hour>) {
        if let Some(columns) = self.columns {
            self.fold.void(mirrors, &columns, &self.choice);
        }
    }

    fn ahead(&mut self, steps: &[Arc<Step>]) {
        let started = std::time::Instant::now();
        self.fold.hand(steps);
        self.copying += started.elapsed().as_nanos() as u64;
    }

    fn settle(&mut self, mirrors: &Mirrors, budget: &mut Budget) -> io::Result<()> {
        self.settle_dirty(mirrors, budget);
        [self.marked[1], self.marked[2]] = self.fold.marked();
        Ok(())
    }

    fn upload(&mut self, _gpu: &mut Gpu<'_>, _budget: &mut Budget) -> io::Result<()> {
        Ok(())
    }

    fn carried(&self) -> Carried {
        // The fold's steps are its worker's: published, not carried.
        let entities = self.units.list.len() + usize::from(self.lanes_dirty) + self.intel.len() + usize::from(self.intel_all) + self.lane_slots.len();
        Carried { entities, frames: self.units.since().map_or(0, |since| self.frame.saturating_sub(since)) }
    }

    fn published(&self) -> crate::layers::Published {
        self.fold.published()
    }

    fn held(&self) -> usize {
        self.unit_rows.capacity() * 40
            + self.unit_rows.values().map(|rows| rows.capacity() * 8).sum::<usize>()
            + self.levels.len() * 24
            + self.units.bytes()
            + self.intel.capacity() * std::mem::size_of::<Intel>()
            + self.lane_slots.capacity() * 4
            + self.air_intel.as_ref().map_or(0, |lane| lane.len() * 24)
            + self.navy_intel.as_ref().map_or(0, |lane| lane.len() * 24)
            + self.province_controllers.len() * 24
            + self.global.links.len() * 24
            + self.fold.held()
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }

    fn changed(&self) -> Option<&crate::layers::Changed> {
        Some(&self.fold.changed)
    }

    fn clear_changed(&mut self) {
        self.fold.changed.clear();
    }
}
