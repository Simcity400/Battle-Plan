//! Every construction project in a branch, for the Construction log: each
//! building and railway line of every country's construction queue, when it was
//! started, the levels it built and how it left the queue.
//!
//! # What the recording holds
//!
//! `BuildingConstruction` and `RailwayConstruction` are one walk of each
//! country's construction vector, read once a day in their slot (and at every
//! full hour: branch start, load, gap, refresh). A line's `id` is its native
//! handle and stays with it for its life.
//!
//! - `created_hour` is the game's creation date of the line, in hours: a repair
//!   line created by bombing carries the hour the building's healthy level
//!   fell. It is when the project started, exact, even when first read days later.
//! - `cost` is fixed5 per level (a whole route for a railway); `produced` is the
//!   fixed5 progress toward the next level; `speed` is the line's progress per
//!   daily update as last computed; `active_factories` the factories assigned.
//! - `amount` is the levels still to build. A daily update adds `speed` to
//!   `produced`, subtracts each whole `cost`, decrements the remaining amount
//!   and raises the building's level. Completion requires `amount == 0` and
//!   no repair left, after which the queue drops the line.
//! - `repair_levels` is the levels still to repair. A line with repair work
//!   repairs instead, keeps `produced` at zero and sets `repair_levels` to
//!   level minus healthy level afterwards. Speed combines BASE_FACTORY_REPAIR
//!   with factory output, so a repair with no factories still progresses.
//! - For a railway, `amount` is the route segments still to build (a route of n
//!   provinces starts at n - 1), `cost` the whole route and `produced` =
//!   `cost` - `remaining_cost`.
//!
//! # How a project's life shows in the recording
//!
//! A line appears at the first read that lists it and is gone at the first read
//! that does not. `StateBuildings` and `ProvinceBuildings` are recorded every
//! hour, so the building a line targets is watched hour by hour while the line is
//! open.
//!
//! The daily construction update is the only thing that completes a level or
//! repair. It runs inside the hourly update entered at hour 23 of a day. The
//! recorder samples at that update's entry, so a completion first shows at an
//! hour that is a multiple of 24, three hours before the construction slot reads
//! the queue. Other things raise buildings too: a player's decision or event
//! choice (a command run between ticks) at any hour, and a focus reward, timed
//! decision, event or on_action at that same daily-update hour.
//!
//! A line repairs while it has levels to repair (the engine takes its repair
//! branch) and builds its remaining new levels otherwise, one or the other at each
//! daily update, so one line can repair, then build, and damage to its building
//! gives it repair work whatever it was doing. Each read's run says what it had
//! left, and each level built says which kind it was (`LevelsBuilt::repair`).
//!
//! ## The claim algorithm
//!
//! Every building line is settled once per stretch, from its last read to the
//! next read that succeeds (a read that failed extends the stretch), by one
//! routine (`settle`):
//!
//! 1. **Can the building be told?** If an hour of the stretch was not recorded, or
//!    another line open since the last read targets the same building (whatever
//!    either was doing), nothing is claimed: what its counts lost is
//!    `unaccounted`, and if it left, its ending is unknown (`NotRecorded`,
//!    `SharedTarget`; `NothingLeft` if it had nothing left). At its first read,
//!    the line's opening stretch runs from its creation hour to this read. Its
//!    earlier work is known to be none only if this queue had a successful read
//!    earlier in the branch, it was created after that read, every hour's building
//!    tables since were complete, and step 2 finds no repair or possible completion
//!    from its creation hour on (for a railway, over every province of its recorded,
//!    nonempty route). Otherwise its earlier work is unknown (`earlier_unknown`),
//!    never zero.
//! 2. **Sort the building's hourly changes.** A rise of the healthy level with the
//!    level unchanged at a daily update is a *repair*; a rise of the level at a
//!    daily update, of a building fully healthy just before, is a *possible
//!    completion* (the line builds only with nothing left to repair); a rise of
//!    level − healthy level at any hour is *damage*. Any other rise (at another
//!    hour, or of a damaged building's level) is never a line's. A line first
//!    listed at this read at the same building may have repaired or built there
//!    from the hour it was created, whatever it shows at its first read, so every
//!    change from that hour on is nobody's and a rise among them makes the stretch
//!    *doubtful*.
//! 3. **New levels.** While listed, as many as its count fell, no more than
//!    the possible completions in step 2, counting a multi-level rise as its count.
//!    A progress fall proves no extra completion: signed factory output can
//!    lower it, and the qword addition wraps. Sampled speed bounds nothing
//!    between reads: factories can be added and taken off at any hour.
//!    The levels are dated by their rises only when the possible completions are
//!    exactly as many as the completions; otherwise by the read (`hour` null).
//! 4. **Repairs.** A repair and damage are the line's while it is open: while
//!    listed, throughout (every repair is built and every damage `added`, even when
//!    neither read shows repair work); once gone, until its work was done, which is
//!    at the repair that left its building fully healthy if it had no new levels
//!    queued. Completion requires none left, and repair sets the remaining
//!    count to level minus healthy level, so damage undoes rises however many
//!    they were. Otherwise work ended before its last new level. Once gone,
//!    what was added is damage before its last claimed rise, or repairs beyond
//!    what it had left if more.
//! 5. **Counts.** While listed, a rise in its queued count is `added`. In a
//!    clear stretch, extra possible completions are `possible`:
//!    levels it may have both built and had added, dated by the read, widening
//!    both counts by that many. A fall beyond what it built was dropped, or is
//!    `unaccounted` when the stretch is doubtful.
//! 6. **Ending, once gone.** An observed daily completion decision or linked
//!    partner removal proves queue completion. Railway cancellation uses the same
//!    decision and retains its cancel operand. A complete observation of an
//!    unlinked line with no such result proves another removal path, dated only
//!    by the read. A linked line with no outcome stays unknown. Older prefixes
//!    and departures outside observation stay unknown. Final between-read level
//!    counts remain unaccounted: completion does not prove which levels commands
//!    may have dropped before that decision.
//!
//! A railway line's segments built while listed are the fall of its `amount`,
//! known only by the read that shows it. Building/route changes never establish
//! its ending; the native completion adapter owns that decision for both types.
//!
//! A project started and finished between two reads is never listed, and lines
//! of a country the links cannot resolve are left out (both counted in the
//! warnings).
//!
//! # Dating
//!
//! Nothing is shown before the read that establishes it: a project from its
//! first read (dated by its recorded `created_hour`), a level built or an ending
//! from the read that shows the line's remaining levels fall or the line gone
//! (`known_at`), dated by the hour its building rose. Factories and remaining
//! levels are runs: a read's values held over the following hours until the next
//! read that differs, a gap or a read that failed. A read is an hour scheduled by
//! the table's cadence or whose frame timed a walk of any table of its slot
//! (`metadata::slot_read`).
//!
//! # Windows
//!
//! The log is sent a window at a time, one per replay block of the branch, so no
//! reply grows with the campaign. A window lists every project with a row date in
//! it (its creation, a level's or its ending's date, as the widget dates its row),
//! and the output runs its tooltips read at its hours. An output read is held until
//! the next, so each project's reads are coded as runs its tooltip shows alike; a
//! run's last read and the window's reads date it. A line whose row is no longer
//! published (its final output lost) is read at every later read until the branch
//! ends: its reason is then the table's at each read, so that run never splits and
//! no later window carries it.

use super::{BranchParams, LoadContext, Product, battles::BattleWindow};
mod output;
use crate::{
    Source,
    catalogue::{self, Catalogue},
    metadata::{branch_bounds, link_slot, mapped_slot, slot_read, validate_campaign},
};
use engine::cbuilding_database::RAILWAY_TEMPLATE_TOKEN;
use installed_content::{ResolvedLayers, gfx::SpriteSet, gui::native};
pub use output::{ConstructionOutputRead, SpendingGap};
use output::{OutputScan, OutputUnknown};
use replay::{
    container::Replay,
    frame::{DecodedFrame, FrameDecoder, SharedView, TableView},
    rows::{BuildingConstructionRow, BuildingRow, RailwayConstructionRow, TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct ConstructionLog;
pub struct ConstructionLogManifest;

/// Longest side of a returned building icon.
const ICON_EDGE: u32 = 48;
/// The fixed5 scale of `cost`.
pub(crate) const COST_SCALE: u32 = 100_000;
/// The construction queue's line entry gives each building's `icon_frame`
/// (`picture`) and repair marker (`repair`). It uses production_building_line_entry;
/// the authored entry supplies the icon and marker declarations through the stack.
const LINE_ENTRY: &str = "production_building_line_entry";
const LINE_PICTURE: &str = "picture";
const LINE_REPAIR: &str = "repair";

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConstructionLogParams {
    pub branch_id: String,
    /// Index into [`ConstructionLogManifestReply::windows`].
    pub window: usize,
}

impl ConstructionLogParams {
    fn branch(&self) -> io::Result<u64> {
        self.branch_id
            .parse()
            .map_err(|_| crate::invalid("Invalid branch"))
    }
}

/// A building line or a railway line.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum ProjectKind {
    Building,
    Railway,
}

/// How a project left the queue.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum EndKind {
    /// Native completion (daily getter or linked removal) was observed at `hour`.
    Finished,
    /// Cancelled, or removed without an observed native completion decision.
    Removed,
    /// It left, but whether it was finished is not known (`unknown` says why).
    Unknown,
}

/// Why a project's ending is not known.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum UnknownEnd {
    /// An hour since its last read was not recorded, or recorded without the
    /// building tables that show its target.
    NotRecorded,
    /// Another line targeted the same building: one open since its last read, or
    /// one first listed at this read, created before a rise it would need.
    SharedTarget,
    /// Its last read held no levels left to build.
    NothingLeft,
    /// No native queue-completion outcome covered its departure.
    CompletionNotRecorded,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectEnd {
    pub kind: EndKind,
    /// The native outcome hour, when observed; otherwise only `known_at` bounds it.
    pub hour: Option<i32>,
    /// The read that showed the line gone: nothing about the ending shows before it.
    pub known_at: i32,
    pub unknown: Option<UnknownEnd>,
}

/// Levels (railway: segments) built between two reads of a line.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LevelsBuilt {
    /// The hour the building rose; null when only the read shows it (railways, and
    /// a building line whose levels could be any of several rises).
    pub hour: Option<i32>,
    /// The read that established it.
    pub known_at: i32,
    pub count: u32,
    /// Levels repaired (the healthy level rising back to the level), not built.
    pub repair: bool,
}

/// One read of a line, held over the following hours until the next read that
/// differs, a gap or a read that failed.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRun {
    pub first_hour: i32,
    pub last_hour: i32,
    /// Factories assigned (`active_factories`).
    pub factories: i32,
    /// Levels (railway: segments) still to build (`amount`).
    pub remaining: i32,
    /// Levels still to repair (`repair_levels`): while above zero the line repairs
    /// (the engine's repair branch), else it builds its remaining new levels.
    pub repair_levels: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ConstructionProject {
    /// The line's native handle, as a decimal string.
    pub id: String,
    pub kind: ProjectKind,
    /// The queue owner's tag at the project's first read.
    pub tag: String,
    /// Index into [`ConstructionLogManifestReply::types`].
    pub building: u32,
    /// It had levels to repair at its first read. What it does over time is per
    /// stretch: a run with `repair_levels` above zero repairs, one without builds
    /// its remaining new levels (a line can repair, then build).
    pub repair: bool,
    /// The target state (a railway: the state of its route's first province).
    pub state: Option<u32>,
    /// A railway: the state of its route's last province.
    pub to_state: Option<u32>,
    /// A building built in one province (a naval base, a bunker).
    pub province: Option<u32>,
    /// A railway: provinces on its route, and its levels before and after.
    pub route_provinces: Option<u32>,
    pub base_level: Option<i32>,
    pub target_level: Option<i32>,
    /// Fixed5 cost per level (railway: the whole route), as at the first read.
    pub cost_raw: String,
    /// When the game created the line.
    pub created_hour: i32,
    /// The first read that lists it: nothing about it shows before.
    pub first_read: i32,
    /// Work before its first read was not recorded; its counts are lower bounds.
    pub earlier_unknown: bool,
    pub runs: Vec<ProjectRun>,
    /// Levels (railway: segments) the line completed. A building line's are as many
    /// as the line itself shows it completed (its count fell, or a daily healthy
    /// rise under its repair), dated by the corresponding daily rise; by the
    /// read (`hour` null) when more such rises at different hours could be them.
    /// A railway's are dated by the read. A rise at
    /// another hour (a player's decision or event choice) is never among them.
    pub built: Vec<LevelsBuilt>,
    /// Levels that cannot be told built or dropped, dated by the read (`hour`
    /// null): what the line's remaining count lost between two reads when some
    /// hour between them was not recorded or another project was building at the
    /// same place, and what it still had to build when it left the queue
    /// unknowably (`EndKind::Unknown`).
    pub unaccounted: Vec<LevelsBuilt>,
    /// Extra possible completions of a listed, non-doubtful stretch: levels the
    /// line may have both built and had added, dated by the read (`hour` null).
    /// They widen both counts without changing the certain levels.
    pub possible: Vec<LevelsBuilt>,
    /// Levels added to the line between two reads (the player queued more, or new
    /// damage gave it repair work), dated by the read (`hour` null). While it stays
    /// listed: the new levels it completed beyond how far its count fell, and the
    /// damage its building took (or the rise of its repair count net of its
    /// repairs, if more). Once it left: the damage before its last claimed rise, or
    /// its repairs beyond what it had left if more; a new level is never added
    /// then. Only the line's own completions are `built`, so a building raised by
    /// something else adds nothing.
    pub added: Vec<LevelsBuilt>,
    /// Null while it is in the queue at the branch's last read.
    pub end: Option<ProjectEnd>,
    /// The first output read counting a repair advance (natural or by factories).
    pub first_repair_tick: Option<i32>,
}

/// Consecutive output reads of a project, from the read `from` to the read `to`,
/// that its tooltip shows alike.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ConstructionOutputRun {
    pub from: i32,
    pub to: i32,
    /// Fixed5 factory output since observation began; null when attribution is unknown.
    pub construction_raw: Option<String>,
    pub repair_raw: Option<String>,
    pub reason: Option<SpendingGap>,
    /// Output before observation began is not recorded.
    pub before_observation: bool,
    /// No row was published: the reason is the table's at each read (it was
    /// unavailable), else the line's final output is missing.
    pub absent: bool,
}

impl ConstructionOutputRun {
    fn shown_as(&self, other: &Self) -> bool {
        (
            &self.construction_raw,
            &self.repair_raw,
            self.reason,
            self.before_observation,
            self.absent,
        ) == (
            &other.construction_raw,
            &other.repair_raw,
            other.reason,
            other.before_observation,
            other.absent,
        )
    }
}

/// A project's output runs a window holds, by its handle.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectOutput {
    pub id: String,
    /// In hour order.
    pub runs: Vec<ConstructionOutputRun>,
}

/// A building type the projects name, with its name and icon from content.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct BuildingInfo {
    /// The recorded lexer id of the type.
    pub id: u32,
    /// Its script name; null when the recorded dictionary lacks it.
    pub token: Option<String>,
    pub name: Option<String>,
    /// A PNG data URL: the construction queue's picture of it.
    pub icon: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct StateName {
    pub id: u32,
    pub name: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ConstructionLogManifestReply {
    /// Units per factory output point of `cost_raw`.
    pub cost_scale: u32,
    /// The branch's blocks: the log's windows, in hour order.
    pub windows: Vec<BattleWindow>,
    /// Either construction queue was read in the branch.
    pub recorded: bool,
    pub types: Vec<BuildingInfo>,
    /// The construction queue's repair marker.
    pub repair_icon: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ConstructionLogReply {
    pub window: usize,
    pub interval: BattleWindow,
    /// Hours the output table was read from the last read before the window to
    /// the next window, ascending.
    pub output_reads: Vec<i32>,
    /// Those reads at which the table was unavailable, and why.
    pub output_unavailable: Vec<(i32, SpendingGap)>,
    /// Every project with a row date in the window, in the scan's order: by first
    /// read, building lines before railway lines, then by handle.
    pub projects: Vec<ConstructionProject>,
    /// For every project changing output in the window, its runs read there; for
    /// every project listed, its last run too, held from then on.
    pub outputs: Vec<ProjectOutput>,
    pub warnings: Vec<String>,
}

/// A building an open line is watched through: province scope, location and type.
type Key = (bool, u32, u32);
/// A building's level and healthy level.
type Levels = (i16, i16);
/// An hourly change between two recorded building values.
type Change = (i32, Levels, Levels);

/// A watched building since the last reset: its value at the last recorded hour
/// (none when unknown), each change since, and whether an hour was lost.
#[derive(Default)]
struct Track {
    value: Option<Levels>,
    changes: Vec<Change>,
    lost: bool,
}

/// The buildings one family's open lines are watched through.
#[derive(Default)]
struct Watch {
    tracks: BTreeMap<Key, Track>,
}

impl Watch {
    /// Follows every watched building into `hour`: a change is kept only between
    /// two known values; an incomplete table loses the hour.
    fn observe(
        &mut self,
        hour: i32,
        states: Option<&[BuildingRow]>,
        provinces: Option<&[BuildingRow]>,
    ) {
        for (key, track) in &mut self.tracks {
            match levels(if key.0 { provinces } else { states }, *key) {
                Some(now) => {
                    if let Some(was) = track.value
                        && was != now
                    {
                        track.changes.push((hour, was, now));
                    }
                    track.value = Some(now);
                }
                None => {
                    track.value = None;
                    track.lost = true;
                }
            }
        }
    }

    /// An hour the recording lacks: every watched building loses it.
    fn lose(&mut self) {
        for track in self.tracks.values_mut() {
            track.value = None;
            track.lost = true;
        }
    }

    /// Starts watching `key` from this hour's tables, unless already watched.
    fn watch(
        &mut self,
        key: Key,
        states: Option<&[BuildingRow]>,
        provinces: Option<&[BuildingRow]>,
    ) {
        self.tracks.entry(key).or_insert_with(|| {
            let value = levels(if key.0 { provinces } else { states }, key);
            Track {
                value,
                changes: Vec::new(),
                lost: value.is_none(),
            }
        });
    }

    /// After a read: forgets what was evaluated and every building no open line needs.
    fn reset(&mut self, needed: &BTreeSet<Key>) {
        self.tracks.retain(|key, _| needed.contains(key));
        for track in self.tracks.values_mut() {
            track.changes.clear();
            track.lost = track.value.is_none();
        }
    }
}

/// Every building whose levels changed since a queue's last successful read.
/// The two queues can fail separately, so their stretches share one hourly diff
/// and each clears its own changed buildings after a successful read.
#[derive(Default)]
struct Changes {
    /// The last hour's complete state and province building tables.
    states: Option<Vec<BuildingRow>>,
    provinces: Option<Vec<BuildingRow>>,
    changed: BTreeMap<Key, Vec<Change>>,
    /// Only province railway buildings can belong to a route's opening stretch.
    railway_changed: BTreeMap<Key, Vec<Change>>,
    lost: bool,
    railway_lost: bool,
}

impl Changes {
    fn observe(
        &mut self,
        hour: i32,
        states: Option<&[BuildingRow]>,
        provinces: Option<&[BuildingRow]>,
    ) {
        for (province, now) in [(false, states), (true, provinces)] {
            let before = if province {
                &mut self.provinces
            } else {
                &mut self.states
            };
            match (now, before.as_deref()) {
                (Some(now), Some(was)) => {
                    level_changes(
                        was,
                        now,
                        province,
                        hour,
                        &mut self.changed,
                        &mut self.railway_changed,
                    );
                }
                _ => {
                    self.lost = true;
                    self.railway_lost = true;
                }
            }
            match now {
                Some(now) => {
                    let kept = before.get_or_insert_with(Vec::new);
                    kept.clear();
                    kept.extend_from_slice(now);
                }
                None => *before = None,
            }
        }
    }

    /// An hour the recording lacks.
    fn lose(&mut self) {
        self.lost = true;
        self.railway_lost = true;
        self.states = None;
        self.provinces = None;
    }

    /// Clears this queue's stretch; the other queue keeps its own changes.
    fn reset(&mut self, railway: bool) {
        if railway {
            self.railway_changed.clear();
            self.railway_lost = false;
        } else {
            self.changed.clear();
            self.lost = false;
        }
    }

    /// Whether the building's level changed since the building queue's last read; None
    /// when an hour's record is missing. Healthy-only changes remain separate.
    fn changed_since_read(&self, key: Key) -> Option<bool> {
        if self.lost {
            return None;
        }
        Some(
            self.changed
                .get(&key)
                .is_some_and(|changes| changes.iter().any(|&(_, was, now)| was.0 != now.0)),
        )
    }

    /// Step 1: the opening stretch is empty only when step 2 finds no daily rise.
    fn earlier_unknown(
        &self,
        keys: &[Key],
        created: i32,
        since: Option<i32>,
        railway: bool,
    ) -> bool {
        if since.is_none_or(|since| created <= since) {
            return true;
        }
        if keys.is_empty() {
            return true;
        }
        let changed = if railway {
            &self.railway_changed
        } else {
            &self.changed
        };
        keys.iter().any(|&key| {
            let lost = if railway {
                self.railway_lost
            } else {
                self.changed_since_read(key).is_none()
            };
            if lost {
                return true;
            }
            let changes = changed.get(&key).map_or(&[][..], Vec::as_slice);
            let first = changes.partition_point(|(hour, _, _)| *hour < created);
            let stretch = Stretch::of(&changes[first..], None);
            !stretch.repairs.is_empty() || !stretch.levels.is_empty()
        })
    }
}

/// Changes between complete tables sorted by (location, type). A building absent
/// from a complete table has no levels; only changed buildings are kept.
fn level_changes(
    was: &[BuildingRow],
    now: &[BuildingRow],
    province: bool,
    hour: i32,
    changed: &mut BTreeMap<Key, Vec<Change>>,
    railway_changed: &mut BTreeMap<Key, Vec<Change>>,
) {
    let (mut a, mut b) = (was.iter().peekable(), now.iter().peekable());
    loop {
        let key = |row: &BuildingRow| (row.location, row.building_type);
        let levels = |row: &BuildingRow| (row.level, row.healthy_level);
        let (level_was, level_now, at) = match (a.peek(), b.peek()) {
            (None, None) => break,
            (Some(x), None) => (levels(x), (0, 0), key(x)),
            (None, Some(y)) => ((0, 0), levels(y), key(y)),
            (Some(x), Some(y)) if key(x) < key(y) => (levels(x), (0, 0), key(x)),
            (Some(x), Some(y)) if key(x) > key(y) => ((0, 0), levels(y), key(y)),
            (Some(x), Some(y)) => (levels(x), levels(y), key(x)),
        };
        if a.peek().is_some_and(|x| key(x) == at) {
            a.next();
        }
        if b.peek().is_some_and(|y| key(y) == at) {
            b.next();
        }
        if level_was != level_now {
            let key = (province, at.0, at.1);
            for changed in [
                Some(&mut *changed),
                (key == railway_key(key.1)).then_some(&mut *railway_changed),
            ]
            .into_iter()
            .flatten()
            {
                changed
                    .entry(key)
                    .or_default()
                    .push((hour, level_was, level_now));
            }
        }
    }
}

/// A building's levels in a complete table; a building the table does not list
/// has none. None when the table is not complete.
fn levels(rows: Option<&[BuildingRow]>, key: Key) -> Option<Levels> {
    let rows = rows?;
    Some(
        rows.binary_search_by_key(&(key.1, key.2), |row| (row.location, row.building_type))
            .map(|index| (rows[index].level, rows[index].healthy_level))
            .unwrap_or((0, 0)),
    )
}

/// A line open since its last read.
struct Open {
    project: usize,
    /// Whether its last run is still carried into this hour.
    carried: bool,
    remaining: i32,
    repair_levels: i32,
    /// The building it is watched through (a railway: its route provinces).
    keys: Vec<Key>,
}

#[derive(Default)]
struct Family {
    open: BTreeMap<u64, Open>,
    watch: Watch,
    reads: Vec<i32>,
}

impl Family {
    fn needed(&self) -> BTreeSet<Key> {
        self.open
            .values()
            .flat_map(|line| line.keys.iter().copied())
            .collect()
    }
}

#[derive(Clone, Debug, Default)]
pub struct ScannedLog {
    pub projects: Vec<ConstructionProject>,
    /// Each project's factory output observed since the recording began, dated by
    /// its publication: at every output read from its first until its line's last.
    pub outputs: Vec<Vec<ConstructionOutputRead>>,
    /// Hours the output table was read, each with why it was unavailable then.
    pub(crate) output_reads: Vec<(i32, Option<SpendingGap>)>,
    /// Lost output or output whose line never had a census owner.
    pub(crate) output_unknown: Vec<OutputUnknown>,
    pub unowned_lines: usize,
    pub reads: Vec<i32>,
    pub railway_reads: Vec<i32>,
    /// The branch's last hour, gaps recorded after its last frame included: a
    /// project still open whose reads stop before it was not recorded for the rest.
    pub last_hour: Option<i32>,
    /// Recorded lexer ids of the building types projects name, with their literal.
    pub types: Vec<(u32, Option<String>)>,
    /// Lines left out because the links do not resolve their country.
    pub unresolved: usize,
    /// Lines first read with a creation hour after their read.
    pub late_created: usize,
    /// Railway lines first read with no route: whether they finished cannot be told.
    pub empty_routes: usize,
    /// Distinct lexical dictionaries taken: an unchanged one is taken once, not
    /// read again each hour.
    pub dictionaries: usize,
}

struct Scan {
    log: ScannedLog,
    buildings: Family,
    railways: Family,
    changes: Changes,
    type_index: BTreeMap<u32, u32>,
    /// The latest complete lexical dictionary (shared, not copied), which names
    /// the building types: looked up only for the types projects name.
    dictionary: Option<SharedView>,
    /// Lines whose country the links do not resolve, each counted once.
    unresolved: BTreeSet<u64>,
    output: OutputScan,
}

impl Scan {
    fn type_of(&mut self, id: u32) -> u32 {
        let next = self.type_index.len() as u32;
        *self.type_index.entry(id).or_insert_with(|| {
            let literal = self.dictionary.as_ref().and_then(|view| literal(view, id));
            self.log.types.push((id, literal));
            next
        })
    }

    /// Takes this hour's dictionary when it is complete and not the one already
    /// held (an unchanged table is the same shared view hour after hour, so this
    /// costs a pointer comparison), and names any type an earlier dictionary could
    /// not.
    fn observe_dictionary(&mut self, view: &SharedView) {
        if !matches!(&**view, TableView::Complete(_))
            || self
                .dictionary
                .as_ref()
                .is_some_and(|held| SharedView::ptr_eq(held, view))
        {
            return;
        }
        self.dictionary = Some(view.clone());
        self.log.dictionaries += 1;
        for (id, token) in &mut self.log.types {
            if token.is_none() {
                *token = literal(view, *id);
            }
        }
    }
}

/// A lexer id's literal in a complete dictionary; none when it is absent or not
/// text.
fn literal(view: &SharedView, id: u32) -> Option<String> {
    let TableView::Complete(rows) = &**view else {
        return None;
    };
    let rows = rows.rows::<tables::LexicalTokens>()?;
    let index = rows.binary_search_by_key(&id, |row| row.token).ok()?;
    std::str::from_utf8(&rows[index].literal)
        .ok()
        .filter(|text| !text.contains('\0'))
        .map(str::to_owned)
}

/// The queue owner's tag at this hour, when the links resolve it.
fn owner_tag(frame: &DecodedFrame, raw: u32) -> Option<String> {
    let countries = frame.complete::<tables::Countries>().ok()?;
    let links = frame.complete::<tables::CountryLinks>().ok()?;
    let slot = link_slot(links, raw)?;
    countries
        .iter()
        .find(|country| {
            country.canonical_slot == slot && mapped_slot(country, links).ok() == Some(slot)
        })
        .map(|country| String::from_utf8_lossy(&country.tag).into_owned())
}

/// A province's state from this hour's complete Provinces table.
fn province_state(frame: &DecodedFrame, province: u32) -> Option<u32> {
    let rows = frame.complete::<tables::Provinces>().ok()?;
    rows.binary_search_by_key(&province, |row| row.id)
        .ok()
        .map(|index| rows[index].state)
        .filter(|state| *state != 0)
}

/// Extends the last run of an open line, or starts a new one at a read.
fn record_run(
    project: &mut ConstructionProject,
    line: &mut Open,
    hour: i32,
    factories: i32,
    remaining: i32,
    repair_levels: i32,
) {
    let same = project.runs.last().is_some_and(|run| {
        line.carried
            && run.factories == factories
            && run.remaining == remaining
            && run.repair_levels == repair_levels
    });
    line.carried = true;
    line.remaining = remaining;
    line.repair_levels = repair_levels;
    if same {
        if let Some(run) = project.runs.last_mut() {
            run.last_hour = hour;
        }
    } else {
        project.runs.push(ProjectRun {
            first_hour: hour,
            last_hour: hour,
            factories,
            remaining,
            repair_levels,
        });
    }
}

/// A rise of a line's building since its last read that the daily construction
/// update could have made: `count` levels at `hour`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rise {
    hour: i32,
    count: u32,
}

/// Hours in a game day. The daily construction update runs building, repair
/// and railway progress inside the hourly update entered at hour 23. The recorder
/// samples at that update's entry, so a completion or repair first shows at an
/// hour that is a multiple of 24.
const HOURS_PER_DAY: i32 = 24;

/// Whether a rise at `hour` can be the daily construction update's. A player's
/// decision or event choice (a command run between ticks) lands at any hour; a
/// focus reward, timed decision or on_action can land at the same daily-update hour.
fn daily(hour: i32) -> bool {
    hour.rem_euclid(HOURS_PER_DAY) == 0
}

/// What a building line's building did over a stretch, sorted into what the line
/// can have done (steps 1 and 2 of the module notes' claim algorithm).
#[derive(Debug, Default)]
struct Stretch {
    /// Daily rises of the healthy level with the level unchanged (repairs), each
    /// with whether it left the building fully healthy.
    repairs: Vec<(Rise, bool)>,
    /// Daily rises of the level of a building fully healthy just before: a line
    /// builds only with nothing left to repair; a positive repair_levels takes
    /// the repair branch.
    levels: Vec<Rise>,
    /// Damage: (hour, levels) of each rise of its level less its healthy level.
    damage: Vec<(i32, u32)>,
    /// A daily rise a line first listed at this read could have made: one from the
    /// hour it was created on, whatever it shows at its first read (it may have
    /// repaired or built before). Nothing from that hour on is the older line's.
    doubtful: bool,
}

impl Stretch {
    fn of(changes: &[Change], newcomer: Option<i32>) -> Self {
        let mut stretch = Stretch::default();
        for &(hour, was, now) in changes {
            let (level, healthy) = (i32::from(was.0), i32::from(was.1));
            let (now_level, now_healthy) = (i32::from(now.0), i32::from(now.1));
            let repaired = daily(hour) && now_level == level && now_healthy > healthy;
            let built = daily(hour) && now_level > level && healthy >= level;
            if newcomer.is_some_and(|since| hour >= since) {
                stretch.doubtful |= repaired || built;
                continue;
            }
            if repaired {
                let rise = Rise {
                    hour,
                    count: (now_healthy - healthy) as u32,
                };
                stretch.repairs.push((rise, now_healthy >= now_level));
            }
            if built {
                stretch.levels.push(Rise {
                    hour,
                    count: (now_level - level) as u32,
                });
            }
            let more = (now_level - now_healthy) - (level - healthy);
            if more > 0 {
                stretch.damage.push((hour, more as u32));
            }
        }
        stretch
    }
}

/// Levels built at the rises, dated by them.
fn claim(rises: &[Rise], known_at: i32, repair: bool) -> Vec<LevelsBuilt> {
    rises
        .iter()
        .map(|rise| LevelsBuilt {
            hour: Some(rise.hour),
            known_at,
            count: rise.count,
            repair,
        })
        .collect()
}

/// `count` levels of one kind known only by the read at `known_at`.
fn by_read(count: u32, known_at: i32, repair: bool) -> LevelsBuilt {
    LevelsBuilt {
        hour: None,
        known_at,
        count,
        repair,
    }
}

/// A line that left the queue unknowably may have built any of the `left` levels
/// its last read showed (all of them, in a gap): whatever this read did not show
/// built is unaccounted, never read as unbuilt.
fn leave_unaccounted(
    project: &mut ConstructionProject,
    end: &ProjectEnd,
    left: &[(bool, i32)],
    hour: i32,
) {
    if end.kind != EndKind::Unknown {
        return;
    }
    for &(repair, left) in left {
        let shown: u32 = project
            .built
            .iter()
            .filter(|levels| levels.known_at == hour && levels.repair == repair)
            .map(|levels| levels.count)
            .sum();
        let rest = u32::try_from(left).unwrap_or(0).saturating_sub(shown);
        if rest > 0 {
            project.unaccounted.push(by_read(rest, hour, repair));
        }
    }
}

fn ended(
    kind: EndKind,
    hour: Option<i32>,
    known_at: i32,
    unknown: Option<UnknownEnd>,
) -> ProjectEnd {
    ProjectEnd {
        kind,
        hour,
        known_at,
        unknown,
    }
}

/// The building a building line is watched through.
fn building_key(row: &BuildingConstructionRow) -> Key {
    (
        row.target_flags & engine::cbuilding_status::PROVINCE_FLAG != 0,
        row.target_location,
        row.building_type,
    )
}

/// A province's railway building, which the route is watched through.
fn railway_key(province: u32) -> Key {
    (true, province, RAILWAY_TEMPLATE_TOKEN)
}

/// How a building line's building was seen over a stretch.
enum Seen<'a> {
    /// An hour was not recorded, or the building tables were incomplete.
    Lost,
    /// Another line open since the last read targets the same building.
    Shared,
    Clear(&'a Track, Option<i32>),
}

/// Pushes a count of levels read at `known_at` onto `list` when positive.
fn push_read(list: &mut Vec<LevelsBuilt>, count: i64, known_at: i32, repair: bool) {
    if let Ok(count) = u32::try_from(count)
        && count > 0
    {
        list.push(by_read(count, known_at, repair));
    }
}

/// The module notes' claim algorithm for one building line over the stretch from
/// its last read to this read at `hour`, where `row` is this read's
/// value of it (none once it left the queue). Records what it built, what was added
/// to it and what cannot be told, and returns its ending.
fn settle(
    project: &mut ConstructionProject,
    line: &Open,
    row: Option<&BuildingConstructionRow>,
    seen: Seen<'_>,
    hour: i32,
) -> Option<ProjectEnd> {
    let (repair_left, new_left) = (line.repair_levels.max(0), line.remaining.max(0));
    let (repair_now, new_now) =
        row.map_or((0, 0), |row| (row.repair_levels.max(0), row.amount.max(0)));
    let left = [(true, repair_left), (false, new_left)];
    let stretch = match seen {
        Seen::Clear(track, newcomer) => Stretch::of(&track.changes, newcomer),
        Seen::Lost | Seen::Shared => {
            if row.is_some() {
                // What its counts lost cannot be checked against the building.
                for (repair, left, now) in
                    [(true, repair_left, repair_now), (false, new_left, new_now)]
                {
                    push_read(
                        &mut project.unaccounted,
                        i64::from(left - now),
                        hour,
                        repair,
                    );
                }
                return None;
            }
            let why = if repair_left == 0 && new_left == 0 {
                UnknownEnd::NothingLeft
            } else if matches!(seen, Seen::Lost) {
                UnknownEnd::NotRecorded
            } else {
                UnknownEnd::SharedTarget
            };
            let end = ended(EndKind::Unknown, None, hour, Some(why));
            leave_unaccounted(project, &end, &left, hour);
            return Some(end);
        }
    };
    if row.is_none() && repair_left == 0 && new_left == 0 {
        let end = ended(EndKind::Unknown, None, hour, Some(UnknownEnd::NothingLeft));
        return Some(end);
    }
    let candidates: u32 = stretch.levels.iter().map(|rise| rise.count).sum();
    let damaged = |until: Option<i32>| -> u32 {
        stretch
            .damage
            .iter()
            .filter(|(at, _)| until.is_none_or(|until| *at < until))
            .map(|(_, levels)| levels)
            .sum()
    };
    let dated = |completions: u32| {
        if candidates == completions {
            claim(&stretch.levels, hour, false)
        } else {
            vec![by_read(completions, hour, false)]
        }
    };
    if row.is_none() {
        // Disappearance and building changes cannot distinguish completion from
        // cancellation. The output adapter applies the observed native outcome.
        let end = ended(
            EndKind::Unknown,
            None,
            hour,
            Some(UnknownEnd::CompletionNotRecorded),
        );
        leave_unaccounted(project, &end, &left, hour);
        return Some(end);
    }
    // Signed output and qword wrapping can lower progress without a
    // completion. Bound claims only by the count's fall.
    let fell = u32::try_from(new_left - new_now).unwrap_or(0);
    let completions = fell.min(candidates);
    let built = if completions > 0 {
        dated(completions)
    } else {
        Vec::new()
    };
    project.built.extend(built);
    // Extra possible completions may have been built and had added. A fall beyond
    // what it built was dropped, or is unknown in a doubtful stretch.
    let net = i64::from(new_now) - i64::from(new_left) + i64::from(completions);
    push_read(&mut project.added, net, hour, false);
    if stretch.doubtful {
        push_read(&mut project.unaccounted, -net, hour, false);
    } else {
        push_read(
            &mut project.possible,
            i64::from(candidates - completions),
            hour,
            false,
        );
    }
    // Repairs: every daily healthy rise, and every damage (open throughout, the
    // line had the repair work), even when neither read shows any.
    let repairs: Vec<Rise> = stretch.repairs.iter().map(|(rise, _)| *rise).collect();
    let repaired: u32 = repairs.iter().map(|rise| rise.count).sum();
    project.built.extend(claim(&repairs, hour, true));
    let added = i64::from(damaged(None))
        .max(i64::from(repair_now) - i64::from(repair_left) + i64::from(repaired));
    push_read(&mut project.added, added, hour, true);
    if stretch.doubtful {
        let lost =
            i64::from(repair_left) + added.max(0) - i64::from(repaired) - i64::from(repair_now);
        push_read(&mut project.unaccounted, lost, hour, true);
    }
    None
}

/// One read of the building queue at `hour`.
fn read_buildings(
    scan: &mut Scan,
    frame: &DecodedFrame,
    rows: &[BuildingConstructionRow],
    states: Option<&[BuildingRow]>,
    provinces: Option<&[BuildingRow]>,
) {
    let hour = frame.hour;
    let present: BTreeMap<u64, &BuildingConstructionRow> =
        rows.iter().map(|row| (row.id, row)).collect();
    // Lines per building, open since the last read: a rise two of them could have
    // made is nobody's. Whatever a line was doing, damage can give it repair work,
    // so lines share a building whatever their kinds.
    let mut sharing: BTreeMap<Key, usize> = BTreeMap::new();
    for line in scan.buildings.open.values() {
        *sharing.entry(line.keys[0]).or_default() += 1;
    }
    // Lines first listed now at the same building: each may have repaired or built
    // there from the hour it was created (its recorded `created_hour`), never
    // before, whatever it shows at its first read.
    let mut newcomers: BTreeMap<Key, i32> = BTreeMap::new();
    for row in rows
        .iter()
        .filter(|row| !scan.buildings.open.contains_key(&row.id))
    {
        newcomers
            .entry(building_key(row))
            .and_modify(|since| *since = (*since).min(row.created_hour))
            .or_insert(row.created_hour);
    }
    let ids: Vec<u64> = scan.buildings.open.keys().copied().collect();
    for id in ids {
        let Some(line) = scan.buildings.open.get_mut(&id) else {
            continue;
        };
        let index = line.project;
        let project = &mut scan.log.projects[index];
        let key = line.keys[0];
        let seen = match scan.buildings.watch.tracks.get(&key) {
            Some(track) if !track.lost => {
                if sharing.get(&key).copied().unwrap_or(0) > 1 {
                    Seen::Shared
                } else {
                    Seen::Clear(track, newcomers.get(&key).copied())
                }
            }
            _ => Seen::Lost,
        };
        let row = present.get(&id).copied();
        let end = settle(project, line, row, seen, hour);
        match row {
            Some(row) => {
                record_run(
                    project,
                    line,
                    hour,
                    row.active_factories,
                    row.amount,
                    row.repair_levels,
                );
            }
            None => {
                project.end = end;
                scan.buildings.open.remove(&id);
            }
        }
    }
    for row in rows {
        if scan.buildings.open.contains_key(&row.id) {
            continue;
        }
        let Some(tag) = owner_tag(frame, row.country) else {
            scan.unresolved.insert(row.id);
            continue;
        };
        if row.created_hour > hour {
            scan.log.late_created += 1;
        }
        let key = building_key(row);
        let province = key.0.then_some(row.target_location);
        let building = scan.type_of(row.building_type);
        let earlier_unknown = scan.changes.earlier_unknown(
            &[key],
            row.created_hour,
            scan.buildings.reads.last().copied(),
            false,
        );
        let mut project = ConstructionProject {
            id: row.id.to_string(),
            kind: ProjectKind::Building,
            tag,
            building,
            repair: row.repair_levels > 0,
            state: (row.target_state != 0).then_some(row.target_state),
            to_state: None,
            province,
            route_provinces: None,
            base_level: None,
            target_level: None,
            cost_raw: row.cost.to_string(),
            created_hour: row.created_hour,
            first_read: hour,
            earlier_unknown,
            runs: Vec::new(),
            built: Vec::new(),
            unaccounted: Vec::new(),
            possible: Vec::new(),
            added: Vec::new(),
            end: None,
            first_repair_tick: None,
        };
        let mut line = Open {
            project: scan.log.projects.len(),
            carried: false,
            remaining: row.amount,
            repair_levels: row.repair_levels,
            keys: vec![key],
        };
        record_run(
            &mut project,
            &mut line,
            hour,
            row.active_factories,
            row.amount,
            row.repair_levels,
        );
        scan.log.projects.push(project);
        scan.log.outputs.push(Vec::new());
        scan.buildings.watch.watch(key, states, provinces);
        scan.buildings.open.insert(row.id, line);
    }
    let needed = scan.buildings.needed();
    scan.buildings.watch.reset(&needed);
    scan.buildings.reads.push(hour);
    scan.changes.reset(false);
}

/// One read of the railway queue at `hour`.
fn read_railways(
    scan: &mut Scan,
    frame: &DecodedFrame,
    rows: &[RailwayConstructionRow],
    states: Option<&[BuildingRow]>,
    provinces: Option<&[BuildingRow]>,
) {
    let hour = frame.hour;
    let present: BTreeMap<u64, &RailwayConstructionRow> =
        rows.iter().map(|row| (row.id, row)).collect();
    let ids: Vec<u64> = scan.railways.open.keys().copied().collect();
    for id in ids {
        let Some(line) = scan.railways.open.get_mut(&id) else {
            continue;
        };
        let index = line.project;
        let project = &mut scan.log.projects[index];
        match present.get(&id) {
            Some(row) => {
                let fell = line.remaining.saturating_sub(row.amount);
                if fell > 0 {
                    project.built.push(by_read(fell as u32, hour, false));
                }
                record_run(project, line, hour, row.active_factories, row.amount, 0);
            }
            None => {
                let end = ended(
                    EndKind::Unknown,
                    None,
                    hour,
                    Some(UnknownEnd::CompletionNotRecorded),
                );
                leave_unaccounted(project, &end, &[(false, line.remaining)], hour);
                project.end = Some(end);
                scan.railways.open.remove(&id);
            }
        }
    }
    for row in rows {
        if scan.railways.open.contains_key(&row.id) {
            continue;
        }
        let Some(tag) = owner_tag(frame, row.country) else {
            scan.unresolved.insert(row.id);
            continue;
        };
        if row.created_hour > hour {
            scan.log.late_created += 1;
        }
        if row.route.is_empty() {
            scan.log.empty_routes += 1;
        }
        let keys: Vec<Key> = row
            .route
            .iter()
            .map(|&province| railway_key(province))
            .collect();
        let mut project = ConstructionProject {
            id: row.id.to_string(),
            kind: ProjectKind::Railway,
            tag,
            building: scan.type_of(RAILWAY_TEMPLATE_TOKEN),
            repair: false,
            state: row
                .route
                .first()
                .and_then(|&province| province_state(frame, province)),
            to_state: row
                .route
                .last()
                .and_then(|&province| province_state(frame, province)),
            province: None,
            route_provinces: Some(row.route.len() as u32),
            base_level: Some(row.base_level),
            target_level: Some(row.target_level),
            cost_raw: row.cost.to_string(),
            created_hour: row.created_hour,
            first_read: hour,
            earlier_unknown: scan.changes.earlier_unknown(
                &keys,
                row.created_hour,
                scan.railways.reads.last().copied(),
                true,
            ),
            runs: Vec::new(),
            built: Vec::new(),
            unaccounted: Vec::new(),
            possible: Vec::new(),
            added: Vec::new(),
            end: None,
            first_repair_tick: None,
        };
        let mut line = Open {
            project: scan.log.projects.len(),
            carried: false,
            remaining: row.amount,
            repair_levels: 0,
            keys: keys.clone(),
        };
        record_run(
            &mut project,
            &mut line,
            hour,
            row.active_factories,
            row.amount,
            0,
        );
        scan.log.projects.push(project);
        scan.log.outputs.push(Vec::new());
        for key in keys {
            scan.railways.watch.watch(key, states, provinces);
        }
        scan.railways.open.insert(row.id, line);
    }
    let needed = scan.railways.needed();
    scan.railways.watch.reset(&needed);
    scan.railways.reads.push(hour);
    scan.changes.reset(true);
}

/// Scans one branch for every construction project.
pub fn scan(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<ScannedLog> {
    scan_with_reads(replay, branch, cancelled, |frame, table| {
        slot_read(&frame.metrics, table, frame.hour)
    })
}

fn scan_with_reads(
    replay: &Replay,
    branch: u64,
    cancelled: &AtomicBool,
    read_queue: impl Fn(&DecodedFrame, TableId) -> bool,
) -> io::Result<ScannedLog> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::Provinces,
        TableId::StateBuildings,
        TableId::ProvinceBuildings,
        TableId::BuildingConstruction,
        TableId::RailwayConstruction,
        TableId::ConstructionLineOutput,
        TableId::LexicalTokens,
    ];
    let cancel = || io::Error::new(io::ErrorKind::Interrupted, "Construction request cancelled");
    let mut scan = Scan {
        log: ScannedLog::default(),
        buildings: Family::default(),
        railways: Family::default(),
        changes: Changes::default(),
        type_index: BTreeMap::new(),
        dictionary: None,
        unresolved: BTreeSet::new(),
        output: OutputScan::default(),
    };
    let mut last: Option<i32> = None;
    crate::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                if cancelled.load(Ordering::Acquire) {
                    return Err(cancel());
                }
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
                    continue;
                };
                for frame in frames {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(cancel());
                    }
                    let Ok(decoded) = decoder.advance(&frame) else {
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        break;
                    }
                    let hour = frame.hour;
                    scan.observe_dictionary(&decoded.tables[TableId::LexicalTokens.index()]);
                    // An hour the recording lacks breaks every run and loses every watched building.
                    if last.and_then(|last| last.checked_add(1)) != Some(hour) {
                        if let Some(missing) = last.and_then(|last| last.checked_add(1)) {
                            scan.output.gap(&mut scan.log, missing);
                        }
                        scan.changes.lose();
                        for family in [&mut scan.buildings, &mut scan.railways] {
                            family.watch.lose();
                            for line in family.open.values_mut() {
                                line.carried = false;
                            }
                        }
                    }
                    last = Some(hour);
                    let states = decoded.complete::<tables::StateBuildings>().ok();
                    let provinces = decoded.complete::<tables::ProvinceBuildings>().ok();
                    scan.changes.observe(hour, states, provinces);
                    scan.buildings.watch.observe(hour, states, provinces);
                    scan.railways.watch.observe(hour, states, provinces);
                    let read = read_queue(decoded, TableId::BuildingConstruction);
                    match (read, decoded.complete::<tables::BuildingConstruction>()) {
                        (true, Ok(rows)) => {
                            read_buildings(&mut scan, decoded, rows, states, provinces)
                        }
                        // A read that failed: its values are unknown, so nothing is carried,
                        // until another successful queue read.
                        (true, Err(_)) => scan.buildings.open.values_mut().for_each(|line| {
                            line.carried = false;
                        }),
                        (false, _) => carry(&mut scan.log.projects, &scan.buildings.open, hour),
                    }
                    let read = read_queue(decoded, TableId::RailwayConstruction);
                    match (read, decoded.complete::<tables::RailwayConstruction>()) {
                        (true, Ok(rows)) => {
                            read_railways(&mut scan, decoded, rows, states, provinces)
                        }
                        (true, Err(_)) => scan.railways.open.values_mut().for_each(|line| {
                            line.carried = false;
                        }),
                        (false, _) => carry(&mut scan.log.projects, &scan.railways.open, hour),
                    }
                    if read_queue(decoded, TableId::ConstructionLineOutput) {
                        scan.output.observe(&mut scan.log, decoded);
                    }
                }
            }
            Ok(())
        },
    )?;
    scan.log.unresolved = scan.unresolved.len();
    scan.log.reads = scan.buildings.reads;
    scan.log.railway_reads = scan.railways.reads;
    // The branch's end: recorded gaps after its last frame are part of it.
    scan.log.last_hour = replay
        .branches
        .iter()
        .find(|start| start.id == branch)
        .map(|start| branch_bounds(replay, start).1)
        .or(last);
    if let Some(missing) = last.and_then(|last| last.checked_add(1))
        && scan.log.last_hour.is_some_and(|end| missing <= end)
    {
        scan.output.gap(&mut scan.log, missing);
    }
    Ok(scan.log)
}

/// Holds each open line's last read over an hour between reads.
fn carry(projects: &mut [ConstructionProject], open: &BTreeMap<u64, Open>, hour: i32) {
    for line in open.values().filter(|line| line.carried) {
        if let Some(run) = projects[line.project].runs.last_mut() {
            run.last_hour = hour;
        }
    }
}

/// Names and icons of building types, and state names, from content.
pub struct Named {
    /// By recorded lexer id: the type's name and icon.
    pub types: BTreeMap<u32, (Option<String>, Option<String>)>,
    pub repair_icon: Option<String>,
    pub states: BTreeMap<u32, Option<String>>,
    pub warnings: Vec<installed_content::Diagnostic>,
    /// A refused state read, warned live; the states corpus owns it offline.
    state_read_error: Option<String>,
    pub(crate) state_diagnostics: Vec<installed_content::states::Diagnostic>,
    pub(crate) declared_types: Vec<String>,
}

impl Named {
    fn live_warnings(&self) -> impl Iterator<Item = String> + '_ {
        self.warnings
            .iter()
            .map(|(_, message)| message.clone())
            .chain(self.state_read_error.clone())
            .chain(
                crate::scripted_text::live_state_warnings(&self.state_diagnostics)
                    .map(|why| format!("State names: {why}")),
            )
    }
}

/// The recorded content a log's names and art come from: the catalogue's layers,
/// cached GUIs, sprites, client decode state and localisation.
pub struct ContentView<'a> {
    pub layers: &'a ResolvedLayers,
    pub sprites: &'a SpriteSet,
    pub guis: Result<&'a native::Guis, String>,
    pub gui_decode: &'a Result<installed_content::image::native::Decode, String>,
    pub localise: &'a dyn Fn(&str) -> Option<String>,
}

struct LineSprite {
    sprite: installed_content::gfx::SpriteType,
    /// One-based GUI frame; the game overwrites the building picture's frame.
    frame: i32,
}

/// The queue entry's sprite for `element` and its authored icon frame.
fn line_sprite(content: &ContentView<'_>, element: &str) -> io::Result<LineSprite> {
    // The GUI registry's later definition wins.
    let entry = content
        .guis
        .as_ref()
        .map_err(crate::invalid)?
        .template(LINE_ENTRY)
        .filter(|entry| entry.kind == native::Kind::Container)
        .ok_or_else(|| crate::invalid(format!("no .gui file declares {LINE_ENTRY}")))?;
    let icon = entry
        .find(native::Kind::Icon, element)
        .ok_or_else(|| crate::invalid(format!("{LINE_ENTRY} declares no {element} icon")))?;
    if let Some(refusal) = entry.refusals().iter().chain(icon.refusals()).next() {
        return Err(crate::invalid(format!("{LINE_ENTRY}: {refusal}")));
    }
    let icon = icon.icon().expect("icon kind");
    let bytes = &icon.sprite;
    let name = std::str::from_utf8(bytes)
        .map_err(|_| crate::invalid(format!("{LINE_ENTRY}.{element} sprite is not UTF-8")))?;
    if name.is_empty() {
        return Err(crate::invalid(format!(
            "{LINE_ENTRY}.{element} has no sprite"
        )));
    }
    let sprite = catalogue::created_gui_sprite(content.sprites, name)?;
    Ok(LineSprite {
        sprite: sprite.clone(),
        frame: icon.frame,
    })
}

/// Names shown in a warning before the rest are counted.
const LISTED: usize = 12;

/// "N <what>: a, b, c and N more", or nothing for none.
fn counted(
    warnings: &mut Vec<installed_content::Diagnostic>,
    kind: installed_content::DiagnosticKind,
    what: &str,
    names: &[String],
) {
    if names.is_empty() {
        return;
    }
    let shown = names
        .iter()
        .take(LISTED)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let more = names.len().saturating_sub(LISTED);
    let rest = if more > 0 {
        format!(" and {more} more")
    } else {
        String::new()
    };
    warnings.push((kind, format!("{} {what}: {shown}{rest}.", names.len())));
}

/// Names and icons for the building types the projects name and names for their
/// states, all from the recorded content stack. Whatever content cannot give is
/// left null, so the widget shows the script name, the state's number or a
/// placeholder, and each kind of refusal is one counted warning naming the types.
pub fn named(
    content: &ContentView<'_>,
    types: &[(u32, Option<String>)],
    states: &BTreeSet<u32>,
    cancelled: &AtomicBool,
) -> io::Result<Named> {
    named_inventory(content, Some((types, states)), cancelled)
}

pub(crate) fn named_all(content: &ContentView<'_>, cancelled: &AtomicBool) -> io::Result<Named> {
    named_inventory(content, None, cancelled)
}

/// The building types and states a recording names.
type Inventory<'a> = (&'a [(u32, Option<String>)], &'a BTreeSet<u32>);

fn named_inventory(
    content: &ContentView<'_>,
    inventory: Option<Inventory<'_>>,
    cancelled: &AtomicBool,
) -> io::Result<Named> {
    use installed_content::DiagnosticKind::{Ambiguous, Fallback, Refused};
    let mut warnings = Vec::new();
    let layers = content.layers;
    let buildings = match installed_content::buildings::read(layers) {
        Ok(buildings) => Some(buildings),
        Err(error) => {
            warnings.push((
                Refused,
                format!("Building types are unavailable in the recorded content: {error}"),
            ));
            None
        }
    };
    let mut state_diagnostics = Vec::new();
    let mut state_read_error = None;
    let definitions = match crate::scripted_text::read_state_content(layers) {
        Ok(content) => {
            state_diagnostics = content.warnings;
            Some(content.states)
        }
        Err(error) => {
            state_read_error = Some(format!("State names are unavailable: {error}"));
            None
        }
    };
    let (declared_types, authored_types, authored_states) = if inventory.is_none() {
        let declared_types = buildings
            .as_ref()
            .map(|buildings| buildings.declaration_order.clone())
            .unwrap_or_default();
        let authored_types: Vec<_> = declared_types
            .iter()
            .enumerate()
            .map(|(id, token)| (id as u32, Some(token.clone())))
            .collect();
        let authored_states = definitions
            .as_ref()
            .map(|states| states.definitions().keys().copied().collect())
            .unwrap_or_default();
        (declared_types, authored_types, authored_states)
    } else {
        (Vec::new(), Vec::new(), BTreeSet::new())
    };
    let (types, states) = inventory.unwrap_or((&authored_types, &authored_states));
    let sprite = |element| line_sprite(content, element);
    let strip = match sprite(LINE_PICTURE) {
        Ok(strip) => Some(strip),
        Err(error) => {
            warnings.push((Refused, format!("Building icons are unavailable: {error}")));
            None
        }
    };
    let (mut undictionaried, mut undeclared, mut unnamed) = (Vec::new(), Vec::new(), Vec::new());
    let (mut frameless, mut unusable, mut ambiguous, mut unreadable) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut named_types = BTreeMap::new();
    for (id, token) in types {
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Construction request cancelled",
            ));
        }
        let Some(token) = token else {
            undictionaried.push(id.to_string());
            named_types.insert(*id, (None, None));
            continue;
        };
        let declaration =
            buildings
                .as_ref()
                .and_then(|buildings| match buildings.by_token(token) {
                    Ok(Some(building)) => Some(building),
                    Ok(None) => {
                        undeclared.push(token.clone());
                        None
                    }
                    Err(_) => {
                        ambiguous.push(token.clone());
                        None
                    }
                });
        // A building's name is its own key in the localisation.
        let key = declaration.map_or(token.as_str(), |building| building.name.as_str());
        let name = (content.localise)(key);
        if name.is_none() {
            unnamed.push(token.clone());
        }
        let frame = match declaration {
            Some(building) => {
                let frame = building.icon_frame;
                if frame < 0 {
                    unusable.push(token.clone());
                    None
                } else if frame == 0 {
                    frameless.push(token.clone());
                    None
                } else {
                    u32::try_from(frame).ok()
                }
            }
            _ => None,
        };
        let icon = match (&strip, frame) {
            (Some(strip), Some(frame)) => {
                let icon = content
                    .gui_decode
                    .as_ref()
                    .map_err(crate::invalid)
                    .and_then(|decode| {
                        catalogue::sprite_frame_png(
                            layers,
                            &strip.sprite,
                            *decode,
                            frame - 1,
                            ICON_EDGE,
                        )
                    });
                match icon {
                    Ok(icon) => Some(icon),
                    Err(error) => {
                        unreadable.push(format!("{token} (frame {frame}: {error})"));
                        None
                    }
                }
            }
            _ => None,
        };
        named_types.insert(*id, (name, icon));
    }
    counted(
        &mut warnings,
        Fallback,
        "building types are not in the recorded lexical dictionary; their titles are unavailable",
        &undictionaried,
    );
    counted(
        &mut warnings,
        Fallback,
        "building types are not declared in the recorded content's buildings; their icons are unavailable",
        &undeclared,
    );
    counted(
        &mut warnings,
        Fallback,
        "building types have no name in the recorded localisation; their titles are unavailable",
        &unnamed,
    );
    counted(
        &mut warnings,
        Fallback,
        "building types have icon_frame 0 (none declared), whose picture is not verified, so they show a placeholder",
        &frameless,
    );
    counted(
        &mut warnings,
        Refused,
        "building types give a negative icon_frame; it is refused and they show a placeholder",
        &unusable,
    );
    counted(
        &mut warnings,
        Ambiguous,
        "building types are declared more than once with differing fields; which declaration the game shows is not verified, so they show a placeholder",
        &ambiguous,
    );
    counted(
        &mut warnings,
        Refused,
        "building icons could not be read from the queue's building strip and show a placeholder",
        &unreadable,
    );
    let repair_icon = match sprite(LINE_REPAIR).and_then(|line| {
        let frame = u32::try_from(line.frame)
            .ok()
            .filter(|frame| *frame > 0 && *frame <= line.sprite.frames)
            .ok_or_else(|| {
                crate::invalid(format!(
                    "{LINE_ENTRY}.{LINE_REPAIR} ({}): frame {} is outside 1..={}",
                    line.sprite.name, line.frame, line.sprite.frames
                ))
            })?;
        // The game's marker is a frame-animated strip (vanilla blends its frames);
        // the widget shows the authored frame, cropped across the strip.
        let sprite = &line.sprite;
        let refused = match sprite.facts.kind.plain() {
            None => Some("has no frame strip"),
            Some(plain) if plain.frame_direction_y() => {
                Some("runs down its texture, which is not supported")
            }
            Some(_) => None,
        };
        if let Some(refused) = refused {
            return Err(crate::invalid(format!(
                "{LINE_ENTRY}.{LINE_REPAIR} ({}, {}) {refused}",
                sprite.name,
                sprite.facts.kind.keyword()
            )));
        }
        let texture = &sprite.facts.kind.plain().expect("frame strip").texture;
        let decode = *content.gui_decode.as_ref().map_err(crate::invalid)?;
        catalogue::gui_texture_frame_png(
            layers,
            texture,
            decode,
            sprite.frames,
            frame - 1,
            ICON_EDGE,
        )
    }) {
        Ok(icon) => Some(icon),
        Err(error) => {
            let why = format!(
                "The repair marker is unavailable, so repairs are marked in words only: {error}"
            );
            warnings.push((Refused, why));
            None
        }
    };
    let mut named_states = BTreeMap::new();
    let mut unnamed_states = Vec::new();
    for &id in states {
        let name = definitions
            .as_ref()
            .and_then(|definitions| definitions.get(id))
            // A key whose reading cannot be followed names nothing.
            .and_then(|state| state.name_key.as_deref().ok())
            .and_then(|key| (content.localise)(key));
        if definitions.is_some() && name.is_none() {
            unnamed_states.push(id.to_string());
        }
        named_states.insert(id, name);
    }
    counted(
        &mut warnings,
        Fallback,
        "states have no name in the recorded content; their titles are unavailable",
        &unnamed_states,
    );
    Ok(Named {
        types: named_types,
        repair_icon,
        states: named_states,
        warnings,
        state_diagnostics,
        state_read_error,
        declared_types,
    })
}

#[derive(Default)]
struct Cache {
    /// Names for the catalogue they were read from, per branch.
    named: Vec<(u64, Arc<Catalogue>, Arc<Named>)>,
}

/// The branches' scans. Its lock is held through a cold scan, so concurrent
/// requests for the branch (the log and spending widgets) share one scan
/// instead of each decoding the whole branch; naming keeps its own lock.
#[derive(Default)]
struct Scans {
    branches: Vec<(u64, Arc<ScannedLog>)>,
    /// Scans run for this source.
    run: usize,
}

/// How many scans this source has run (the coalescing test).
#[cfg(test)]
pub(crate) fn scans_run(source: &Source) -> usize {
    source
        .state::<Scans>()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .run
}

/// Branches whose scans stay cached.
const CACHED_BRANCHES: usize = 4;

/// A branch's scan, cached per source and shared by every construction product,
/// so the log and the spending read the queue once.
pub(crate) fn scanned(
    source: &Source,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Arc<ScannedLog>> {
    let state = source.state::<Scans>();
    let mut scans = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((_, log)) = scans.branches.iter().find(|(cached, _)| *cached == branch) {
        return Ok(Arc::clone(log));
    }
    scans.run += 1;
    let log = Arc::new(scan(&source.replay(), branch, cancelled)?);
    scans.branches.push((branch, Arc::clone(&log)));
    if scans.branches.len() > CACHED_BRANCHES {
        scans.branches.remove(0);
    }
    Ok(log)
}

/// Content's names and pictures for a branch's scan: every type its projects
/// name (in the scan's order, so a project's `building` indexes it), the repair
/// marker, and the names of `states`.
pub(crate) struct ContentNames {
    pub types: Vec<BuildingInfo>,
    pub repair_icon: Option<String>,
    names: Option<Arc<Named>>,
}

impl ContentNames {
    /// Every type the scan names, in its order, named and pictured by `names`.
    fn of(log: &ScannedLog, names: Option<Arc<Named>>) -> Self {
        let types = log
            .types
            .iter()
            .map(|(id, token)| {
                let (name, icon) = names
                    .as_ref()
                    .and_then(|names| names.types.get(id).cloned())
                    .unwrap_or((None, None));
                BuildingInfo {
                    id: *id,
                    token: token.clone(),
                    name,
                    icon,
                }
            })
            .collect();
        Self {
            types,
            repair_icon: names.as_ref().and_then(|names| names.repair_icon.clone()),
            names,
        }
    }

    pub(crate) fn states(&self, states: &BTreeSet<u32>) -> Vec<StateName> {
        states
            .iter()
            .map(|&id| StateName {
                id,
                name: self
                    .names
                    .as_ref()
                    .and_then(|names| names.states.get(&id).cloned().flatten()),
            })
            .collect()
    }
}

/// Every state a branch's projects name.
fn project_states(log: &ScannedLog) -> BTreeSet<u32> {
    log.projects
        .iter()
        .flat_map(|project| [project.state, project.to_state])
        .flatten()
        .collect()
}

/// Names and pictures from the recorded content, cached per branch and catalogue.
/// Without the game folders every type and state stays unnamed, and `warnings` says why.
pub(crate) fn content_names(
    source: &Source,
    context: &LoadContext,
    branch: u64,
    log: &ScannedLog,
    cancelled: &AtomicBool,
    warnings: &mut Vec<String>,
) -> io::Result<ContentNames> {
    let state = source.state::<Cache>();
    let names = match context
        .content()
        .and_then(|content| catalogue::catalogue(source, content))
    {
        Ok(catalogue) => {
            let cached = {
                let cache = state.lock().unwrap_or_else(|error| error.into_inner());
                cache
                    .named
                    .iter()
                    .find(|(cached, from, _)| *cached == branch && Arc::ptr_eq(from, &catalogue))
                    .map(|(_, _, named)| Arc::clone(named))
            };
            let named = match cached {
                Some(named) => named,
                None => {
                    let localise = |key: &str| catalogue.localised(key, catalogue::View::Plain);
                    let view = ContentView {
                        layers: &catalogue.content.layers,
                        sprites: &catalogue.sprites,
                        guis: catalogue.guis().map_err(|error| error.to_string()),
                        gui_decode: &catalogue.gui_decode,
                        localise: &localise,
                    };
                    let named =
                        Arc::new(named(&view, &log.types, &project_states(log), cancelled)?);
                    let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
                    cache.named.retain(|(cached, _, _)| *cached != branch);
                    cache
                        .named
                        .push((branch, Arc::clone(&catalogue), Arc::clone(&named)));
                    if cache.named.len() > CACHED_BRANCHES {
                        cache.named.remove(0);
                    }
                    named
                }
            };
            warnings.extend(catalogue.verification.iter().cloned());
            warnings.extend(catalogue.warnings.iter().cloned());
            warnings.extend(named.live_warnings());
            if named.types.values().any(|(_, icon)| icon.is_some()) {
                warnings.push("Widget artwork comes from the recorded content stack; visual assets are outside content hash coverage.".to_owned());
            }
            Some(named)
        }
        Err(error) => {
            warnings.push(format!("Building and state names are unavailable: {error}"));
            None
        }
    };
    Ok(ContentNames::of(log, names))
}

/// Names from one view of the content without the cache, as `content_names`
/// reads them from a catalogue: for content fixtures.
#[cfg(test)]
pub(crate) fn names_in(
    content: &ContentView<'_>,
    log: &ScannedLog,
    cancelled: &AtomicBool,
    warnings: &mut Vec<String>,
) -> io::Result<ContentNames> {
    let named = named(content, &log.types, &project_states(log), cancelled)?;
    warnings.extend(named.live_warnings());
    Ok(ContentNames::of(log, Some(Arc::new(named))))
}

/// What every construction product warns about a branch's scan.
pub(crate) fn scan_warnings(log: &ScannedLog, warnings: &mut Vec<String>) {
    if log.unresolved > 0 {
        warnings.push(format!(
            "{} construction lines name a country the recorded links do not resolve; they are left out.",
            log.unresolved
        ));
    }
    if log.empty_routes > 0 {
        warnings.push(format!(
            "{} railway lines were recorded without a route; whether they finished is not known.",
            log.empty_routes
        ));
    }
    if log.late_created > 0 {
        warnings.push(format!(
            "{} construction lines were created after the read that first lists them.",
            log.late_created
        ));
    }
    warnings.push(
        "A line created and removed between daily queue reads has no recorded owner or target; its output cannot be attributed.".to_owned(),
    );
    if log.unowned_lines > 0 {
        warnings.push(format!(
            "{} construction output lines have no recorded census owner or target; their projects cannot be attributed.",
            log.unowned_lines
        ));
    }
    let unknown: BTreeSet<_> = log
        .projects
        .iter()
        .zip(&log.outputs)
        .flat_map(|(project, output)| {
            output
                .iter()
                .filter_map(|read| read.reason.map(|reason| (reason, &project.id)))
        })
        .collect();
    let mut reasons = BTreeMap::new();
    for (reason, _) in unknown {
        *reasons.entry(reason).or_insert(0_usize) += 1;
    }
    for (reason, count) in reasons {
        warnings.push(format!(
            "{count} construction lines have unavailable factory output ({reason:?})."
        ));
    }
    if log.reads.is_empty() && log.railway_reads.is_empty() {
        warnings.push("No recorded construction queue in this branch.".to_owned());
    }
}

/// The branch's replay blocks: the log's windows, in hour order.
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

/// The window an hour falls in: the last starting at or before it (an hour before
/// the first falls in the first).
fn window_at(windows: &[BattleWindow], hour: i32) -> usize {
    windows
        .partition_point(|window| window.first_hour <= hour)
        .saturating_sub(1)
}

/// Every date a project's row can carry, as the widget dates it by its latest
/// change or by its start: a level's rise (else the read that showed it), its
/// ending's outcome (else its read) and its creation.
fn row_dates(project: &ConstructionProject) -> impl Iterator<Item = i32> + '_ {
    [&project.built, &project.unaccounted, &project.possible]
        .into_iter()
        .flatten()
        .map(|levels| levels.hour.unwrap_or(levels.known_at))
        .chain(
            project
                .end
                .as_ref()
                .map(|end| end.hour.unwrap_or(end.known_at)),
        )
        .chain([project.created_hour])
}

/// A project's output reads as runs its tooltip shows alike.
fn output_runs(reads: &[ConstructionOutputRead]) -> Vec<ConstructionOutputRun> {
    let mut runs: Vec<ConstructionOutputRun> = Vec::new();
    for read in reads {
        // Without a row, the reason is the table's at the read unless the line's
        // observation restarted (`OutputScan::observe`).
        let absent =
            read.since_hour.is_none() && read.reason != Some(SpendingGap::ObservationRestart);
        let run = ConstructionOutputRun {
            from: read.known_at,
            to: read.known_at,
            construction_raw: read.construction_raw.clone(),
            repair_raw: read.repair_raw.clone(),
            reason: read.reason.filter(|_| !absent),
            before_observation: read.since_hour.is_some() && !read.observed_since_creation,
            absent,
        };
        match runs.last_mut() {
            Some(last) if last.shown_as(&run) => last.to = run.to,
            _ => runs.push(run),
        }
    }
    runs
}

/// A branch's log cut into its windows, coded once rather than for each window.
struct Cut {
    log: Arc<ScannedLog>,
    windows: Vec<BattleWindow>,
    /// Per window, the projects with a row date in it, ascending.
    members: Vec<Vec<usize>>,
    /// Per window, the projects with an output run before their last read there,
    /// ascending.
    changing: Vec<Vec<usize>>,
    /// Per project, its output runs.
    runs: Vec<Vec<ConstructionOutputRun>>,
}

impl Cut {
    fn new(log: Arc<ScannedLog>, windows: Vec<BattleWindow>) -> Self {
        let mut members = vec![Vec::new(); windows.len()];
        let mut changing = vec![Vec::new(); windows.len()];
        let runs: Vec<_> = log
            .outputs
            .iter()
            .map(Vec::as_slice)
            .map(output_runs)
            .collect();
        if !windows.is_empty() {
            let at = |hour| window_at(&windows, hour);
            for (index, project) in log.projects.iter().enumerate() {
                let mut dated: BTreeSet<usize> = row_dates(project).map(at).collect();
                // A line created after the read that first lists it shows from that
                // read dated later: every window between holds it.
                dated.extend(at(project.first_read)..=at(project.created_hour));
                for window in dated {
                    members[window].push(index);
                }
                // A run is read from its first read until the next run's.
                if let [first, .., last] = runs[index].as_slice() {
                    for window in &mut changing[at(first.from)..=at(last.from - 1)] {
                        window.push(index);
                    }
                }
            }
        }
        Self {
            log,
            windows,
            members,
            changing,
            runs,
        }
    }

    fn reply(&self, window: usize) -> ConstructionLogReply {
        let interval = self.windows[window].clone();
        let first = interval.first_hour;
        let next = self.windows.get(window + 1).map(|next| next.first_hour);
        let before_next = |hour: i32| next.is_none_or(|next| hour < next);
        let members = &self.members[window];
        let listed: BTreeSet<usize> = members
            .iter()
            .chain(&self.changing[window])
            .copied()
            .collect();
        let outputs = listed
            .into_iter()
            .filter_map(|index| {
                let runs = &self.runs[index];
                let (last, earlier) = runs.split_last()?;
                let mut held: Vec<ConstructionOutputRun> = earlier
                    .iter()
                    .zip(&runs[1..])
                    .filter(|(run, then)| before_next(run.from) && then.from > first)
                    .map(|(run, _)| run.clone())
                    .collect();
                if members.binary_search(&index).is_ok() {
                    held.push(last.clone());
                }
                (!held.is_empty()).then(|| ProjectOutput {
                    id: self.log.projects[index].id.clone(),
                    runs: held,
                })
            })
            .collect();
        let reads = &self.log.output_reads;
        let start = reads
            .partition_point(|(hour, _)| *hour < first)
            .saturating_sub(1);
        let reads = &reads[start..reads.partition_point(|(hour, _)| before_next(*hour))];
        ConstructionLogReply {
            window,
            interval,
            output_reads: reads.iter().map(|(hour, _)| *hour).collect(),
            output_unavailable: reads
                .iter()
                .filter_map(|(hour, gap)| gap.map(|gap| (*hour, gap)))
                .collect(),
            projects: members
                .iter()
                .map(|&index| self.log.projects[index].clone())
                .collect(),
            outputs,
            warnings: Vec::new(),
        }
    }
}

/// The branches' cut logs.
#[derive(Default)]
struct Cuts {
    branches: Vec<(u64, Arc<Cut>)>,
}

/// A branch's log cut into its windows, cached per source. Its lock is held
/// through a cold cut, so the windows a widget asks for at once share it.
fn cut(source: &Source, branch: u64, cancelled: &AtomicBool) -> io::Result<Arc<Cut>> {
    let state = source.state::<Cuts>();
    let mut cuts = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((_, cut)) = cuts.branches.iter().find(|(cached, _)| *cached == branch) {
        return Ok(Arc::clone(cut));
    }
    let cut = Arc::new(Cut::new(
        scanned(source, branch, cancelled)?,
        windows(source.replay_ref(), branch),
    ));
    cuts.branches.push((branch, Arc::clone(&cut)));
    if cuts.branches.len() > CACHED_BRANCHES {
        cuts.branches.remove(0);
    }
    Ok(cut)
}

impl Product for ConstructionLogManifest {
    const KEY: &'static str = "construction-log-manifest";
    const CONTENT_DEPENDENT: bool = true;
    type Params = BranchParams;
    type Reply = ConstructionLogManifestReply;

    fn types() -> Vec<String> {
        vec![ConstructionLogManifestReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = params.branch();
        let log = scanned(source, branch, cancelled)?;
        let mut warnings = Vec::new();
        // Without the game folders the log still lists every project, by its type's token.
        let names = content_names(source, context, branch, &log, cancelled, &mut warnings)?;
        scan_warnings(&log, &mut warnings);
        let reply = ConstructionLogManifestReply {
            cost_scale: COST_SCALE,
            windows: windows(source.replay_ref(), branch),
            recorded: !(log.reads.is_empty() && log.railway_reads.is_empty()),
            types: names.types,
            repair_icon: names.repair_icon,
            warnings,
        };
        super::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

impl Product for ConstructionLog {
    const KEY: &'static str = "construction-log";
    const CONTENT_DEPENDENT: bool = false;
    type Params = ConstructionLogParams;
    type Reply = ConstructionLogReply;

    fn types() -> Vec<String> {
        vec![
            ConstructionLogParams::decl(),
            ProjectKind::decl(),
            EndKind::decl(),
            UnknownEnd::decl(),
            ProjectEnd::decl(),
            LevelsBuilt::decl(),
            ProjectRun::decl(),
            SpendingGap::decl(),
            ConstructionProject::decl(),
            ConstructionOutputRun::decl(),
            ProjectOutput::decl(),
            BuildingInfo::decl(),
            StateName::decl(),
            ConstructionLogReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map(|_| ())
            .map_err(|_| "Invalid branch")
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        _: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let cut = cut(source, params.branch()?, cancelled)?;
        if params.window >= cut.windows.len() {
            return Err(crate::invalid("Construction log window not in recording"));
        }
        let reply = cut.reply(params.window);
        super::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

#[cfg(test)]
pub(crate) mod tests;
