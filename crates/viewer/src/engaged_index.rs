//! The combat engaged-flag index: a background pass that folds
//! [`engaged::History`] over every recorded hour, so the flag's native lag is
//! exact at any hour, a seek included.
//!
//! When a recording opens, one lowest-priority thread decodes only the tables
//! the derivation reads and keeps, per combat side, the hours its flag changes
//! value. Memory follows the number of combat sides and their changes, not the
//! hours. A newer snapshot of a growing recording continues from the last
//! folded hour ([`EngagedIndex::extend`]). Until the pass covers an hour, the
//! flag is what a load at that hour would rebuild, as the native does at load.
//!
//! Only a branch that starts with a load signal, sampled at its loaded hour,
//! starts rebuilt ([`native_load`]). A session start, an hour discontinuity,
//! or a load whose origin was lost or lies before the first sample joins a
//! running game: its sides stay unknown until each one rebuilds. (The
//! collector records the loaded hour as the first hour at a discontinuity
//! too, so that hour alone cannot tell a load from a discontinuity.)
//!
//! The pass folds each hour on its own thread and then appends that hour's
//! changes under a brief lock, which the UI thread also takes every frame.
//!
//! A combat kind registers its sides as a [`Sides`] source in [`SOURCES`];
//! only land does so far. The pass is one more decoder consumer: it reads
//! blocks through [`Replay::read_block_selected`] and never shares the
//! playback decoder.

use crate::{
    combat::{
        engaged::{self, Continuity, Engaged},
        icon_frames::{self, IconFold, IconHour, Kept},
    },
    map::{
        perspective::{self, Country},
        tables::{Available, core, rows},
        visibility::Relations,
    },
    worker_priority::lowest_priority,
};
use replay::{
    BranchId, Hour,
    branches::{BranchReason, BranchStart},
    container::{BlockIndex, Replay},
    frame::{DecodedFrame, FrameDecoder},
    rows::{TableId, tables},
};
use std::{
    collections::BTreeMap,
    io, mem,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::Instant,
};

/// One hour's sides of one combat kind.
pub(crate) struct HourSides {
    /// Keyed by (combat, side).
    pub(crate) sides: BTreeMap<(u64, u8), engaged::Side>,
    /// The combat table is whole: a combat missing from it is not fighting.
    pub(crate) complete: bool,
    /// The kind's removal rule.
    pub(crate) removal: engaged::Removal,
}

/// A combat kind whose sides carry the engaged flag.
pub(crate) trait Sides: Sync {
    /// The tables [`Sides::sides`] reads.
    fn tables(&self) -> &'static [TableId];
    /// The hour's combats, ascending: what an index lookup needs, without
    /// building the sides.
    fn combats(&self, frame: &DecodedFrame) -> io::Result<Vec<u64>>;
    fn sides(&self, frame: &DecodedFrame) -> io::Result<HourSides>;
}

pub(crate) const LAND: usize = 0;
/// Indexed by kind. Only land registers; naval would follow it.
static SOURCES: [&dyn Sides; 1] = [&crate::combat::progress::LandSides];

/// Combat icon kinds whose unseen frame the index carries
/// ([`icon_frames`]).
pub(crate) const LAND_ICON: usize = 0;
pub(crate) const NAVAL_ICON: usize = 1;
const ICON_KINDS: usize = 2;

/// The tables the icon kinds read besides the viewer's.
const ICON_TABLES: [TableId; 6] = [
    TableId::LandCombats,
    TableId::LandCombatSides,
    TableId::NavalCombats,
    TableId::NavalCombatParticipants,
    TableId::Ships,
    TableId::NavalCombatDetails,
];

/// One icon kind's hour for `viewer`. Land reads its seen test from the
/// engaged `flag`; naval from the members' sharing
/// ([`crate::map::counters::naval_icon_hour`]).
fn icon_hour(
    kind: usize,
    frame: &DecodedFrame,
    viewer: u32,
    links: &BTreeMap<u32, u32>,
    relations: Result<&Relations, &'static str>,
    flag: &dyn Fn(u64) -> Engaged,
) -> io::Result<IconHour> {
    if kind == LAND_ICON {
        icon_frames::land_hour(frame, flag)
    } else {
        let vision = perspective::Vision::relating(viewer, relations, links.values().copied());
        crate::map::counters::naval_icon_hour(frame, &vision, &|raw| links.get(&raw).copied())
    }
}

/// The ids an icon lookup answers for.
fn icon_combats(kind: usize, frame: &DecodedFrame) -> io::Result<Vec<u64>> {
    let mut combats: Vec<u64> = if kind == LAND_ICON {
        rows::<tables::LandCombats>(frame)?
            .rows()
            .iter()
            .map(|combat| combat.id)
            .collect()
    } else {
        rows::<tables::NavalCombats>(frame)?
            .rows()
            .iter()
            .map(|combat| combat.id)
            .collect()
    };
    combats.sort_unstable();
    combats.dedup();
    Ok(combats)
}

/// What each live combat icon of `kind` keeps for its frame at this hour
/// ([`icon_frames`]): the index's fold when it covers the hour. Otherwise
/// only this hour is known: a seen icon's own frame, and
/// [`Kept::Unindexed`] for every unseen one. `flags` are the hour's land
/// engaged flags ([`combats`]).
pub(crate) fn kept_frames(
    frame: &DecodedFrame,
    kind: usize,
    viewer: u32,
    links: &BTreeMap<u32, u32>,
    relations: Result<&Relations, &'static str>,
    flags: &BTreeMap<u64, Engaged>,
    index: Option<&EngagedIndex>,
) -> io::Result<(BTreeMap<u64, Kept>, Provenance)> {
    let provenance = match index {
        Some(index) => {
            let combats = icon_combats(kind, frame)?;
            match index.lookup_icons(kind, frame.branch, frame.hour, viewer, &combats) {
                Ok(found) => return Ok((found, Provenance::Indexed)),
                Err(missed) => missed,
            }
        }
        None => Provenance::Unindexed,
    };
    let flag = |combat: u64| flags.get(&combat).copied().unwrap_or(Engaged::Unknown);
    let hour = icon_hour(kind, frame, viewer, links, relations, &flag)?;
    let mut fold = IconFold::default();
    fold.advance(&hour, Continuity::Gap, &mut Vec::new());
    Ok((
        fold.live()
            .iter()
            .map(|(&combat, &kept)| {
                let kept = match kept {
                    Kept::Unobserved => Kept::Unindexed,
                    other => other,
                };
                (combat, kept)
            })
            .collect(),
        provenance,
    ))
}

/// Per combat: the hours its icon's kept frame changed, ascending; `None`
/// while it has no icon.
type IconPoints = BTreeMap<u64, Vec<(Hour, Option<Kept>)>>;

/// The tables every kind needs: the viewer and its relations.
const VIEWER_TABLES: [TableId; 9] = [
    TableId::Campaign,
    TableId::CountryLinks,
    TableId::Countries,
    TableId::LocalPlayer,
    TableId::Players,
    TableId::HumanControllers,
    TableId::FactionMembers,
    TableId::WarRelations,
    TableId::Subjects,
];

/// One hour's viewer inputs.
struct Viewer {
    slot: u32,
    links: BTreeMap<u32, u32>,
    relations: Result<Relations, &'static str>,
    controllers: Option<engaged::Controllers>,
}

/// The hour's controller rosters; `None` unless `Players` and
/// `HumanControllers` are both whole.
fn controllers(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
) -> io::Result<Option<engaged::Controllers>> {
    let (Available::Complete(players), Available::Complete(counts)) = (
        rows::<tables::Players>(frame)?,
        rows::<tables::HumanControllers>(frame)?,
    ) else {
        return Ok(None);
    };
    let mut out = engaged::Controllers {
        players: players
            .iter()
            .map(|player| {
                (
                    player.id,
                    (player.country, links.get(&player.country).copied()),
                )
            })
            .collect(),
        counts: BTreeMap::new(),
    };
    for row in counts {
        match links.get(&row.country) {
            Some(&slot) => *out.counts.entry(slot).or_default() += i64::from(row.count),
            // A count the links cannot place could be the viewer's.
            None => return Ok(None),
        }
    }
    Ok(Some(out))
}

impl Viewer {
    fn read(frame: &DecodedFrame, country: &Country) -> io::Result<Self> {
        let links: BTreeMap<u32, u32> = core::<tables::CountryLinks>(frame)?
            .iter()
            .map(|link| (link.id, link.canonical_slot))
            .collect();
        // `perspective::viewer` reads only the lowest slot (its fallback) and
        // the requested tag's, so only those two tags are made text.
        let countries = core::<tables::Countries>(frame)?;
        let mut tags = BTreeMap::new();
        if let Some(lowest) = countries.iter().min_by_key(|c| c.canonical_slot) {
            tags.insert(lowest.canonical_slot, String::new());
        }
        // The lowest slot among matching tags, as `perspective::viewer` picks.
        if let Country::Tag(tag) = country
            && let Some(found) = countries
                .iter()
                .filter(|c| c.tag.eq_ignore_ascii_case(tag.as_bytes()))
                .min_by_key(|c| c.canonical_slot)
        {
            tags.insert(
                found.canonical_slot,
                String::from_utf8_lossy(&found.tag).into_owned(),
            );
        }
        let (slot, _) = perspective::viewer(frame, &links, &tags, country)?;
        let relations = Relations::read(frame, &links);
        let controllers = controllers(frame, &links)?;
        Ok(Self {
            slot,
            links,
            relations,
            controllers,
        })
    }

    fn sample(&self, sides: HourSides) -> engaged::Sample<'_> {
        engaged::Sample {
            viewer: self.slot,
            links: &self.links,
            relations: self.relations.as_ref().map_err(|reason| *reason),
            controllers: self.controllers.clone(),
            sides: sides.sides,
            complete: sides.complete,
            removal: sides.removal,
        }
    }
}

/// Where an hour's flags came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Provenance {
    /// Folded over the branch's recorded hours: the flag's native history.
    Indexed,
    /// Rebuilt as a load at this hour: the pass has not folded it yet.
    Pending,
    /// Rebuilt as a load at this hour: the pass resolved another viewer here.
    OtherViewer,
    /// Rebuilt as a load at this hour: the pass read no viewer at or before
    /// it on this branch.
    Unread,
    /// Rebuilt as a load at this hour: built without the index for this
    /// viewing country (a dump, an export, or an hour built before that
    /// index started).
    Unindexed,
}

impl Provenance {
    /// The limitation a load-rebuilt flag reports, per combat; `None` for the
    /// native history.
    pub(crate) fn limitation(self) -> Option<&'static str> {
        match self {
            Self::Indexed => None,
            Self::Pending => Some(
                "engaged flag load-rebuilt; historical flag pending (the engaged index has not reached this hour)",
            ),
            Self::OtherViewer => Some(
                "engaged flag load-rebuilt; historical flag unavailable (the engaged index resolved another viewing country at this hour)",
            ),
            Self::Unread => Some(
                "engaged flag load-rebuilt; historical flag unavailable (the engaged index read no viewing country at or before this hour of the branch)",
            ),
            Self::Unindexed => Some(
                "engaged flag load-rebuilt; historical flag unavailable (built without the engaged index for this viewing country: a dump, an export, or an hour built before that index started)",
            ),
        }
    }
}

/// This hour's flag per combat of `kind` and where it came from: the index
/// when it covers the hour, else as a load at this hour rebuilds it. The flag
/// is kept either way; the provenance says which.
pub(crate) fn combats(
    frame: &DecodedFrame,
    kind: usize,
    viewer: u32,
    links: &BTreeMap<u32, u32>,
    relations: Result<&Relations, &'static str>,
    index: Option<&EngagedIndex>,
) -> io::Result<(BTreeMap<u64, Engaged>, Provenance)> {
    // A covered hour is answered from the index before any side is built.
    let combats = SOURCES[kind].combats(frame)?;
    let provenance = match index {
        Some(index) => match index.lookup(kind, frame.branch, frame.hour, viewer, &combats) {
            Ok(found) => return Ok((found, Provenance::Indexed)),
            Err(missed) => missed,
        },
        None => Provenance::Unindexed,
    };
    let sides = SOURCES[kind].sides(frame)?;
    let mut history = engaged::History::default();
    history.advance(
        engaged::Sample {
            viewer,
            links,
            relations,
            // A load refreshes every side whatever the controllers were.
            controllers: None,
            sides: sides.sides,
            complete: sides.complete,
            removal: sides.removal,
        },
        Continuity::Load,
    );
    Ok((
        combats
            .into_iter()
            .map(|combat| (combat, history.combat(combat)))
            .collect(),
        provenance,
    ))
}

/// Per combat side: the hours its flag took a new value, ascending.
type Changes = BTreeMap<(u64, u8), Vec<(Hour, Engaged)>>;

/// The folded change points of one branch.
#[derive(Default)]
struct Covered {
    first: Hour,
    last: Hour,
    /// Per kind and side: the hours its flag took a new value, ascending.
    points: Vec<Changes>,
    /// Per icon kind and combat: the hours its kept frame changed.
    icons: Vec<IconPoints>,
    /// The viewer per hour where it changed, ascending; a lookup for another
    /// viewer is not covered.
    viewers: Vec<(Hour, u32)>,
}

impl Covered {
    fn viewer(&self, hour: Hour) -> Option<u32> {
        let at = self.viewers.partition_point(|(from, _)| *from <= hour);
        Some(self.viewers.get(at.checked_sub(1)?)?.1)
    }
}

#[derive(Default)]
struct Shared {
    branches: Mutex<BTreeMap<BranchId, Covered>>,
    cancelled: AtomicBool,
}

/// The index for one perspective country; dropping it stops the pass.
pub(crate) struct EngagedIndex {
    country: Country,
    shared: Arc<Shared>,
    snapshots: Sender<Arc<Replay>>,
    /// The snapshot last handed to the pass, which a newer one must extend.
    /// The pass holds the same one while it waits, so this keeps no more.
    folding: Mutex<Arc<Replay>>,
}

impl Drop for EngagedIndex {
    fn drop(&mut self) {
        self.shared.cancelled.store(true, Ordering::Release);
    }
}

impl EngagedIndex {
    /// Starts the pass over `replay` for `country`.
    pub(crate) fn start(replay: Arc<Replay>, country: Country) -> io::Result<Self> {
        let shared = Arc::new(Shared::default());
        let (snapshots, inbox) = mpsc::channel();
        let folding = Arc::clone(&replay);
        let pass = Pass {
            shared: Arc::clone(&shared),
            country: country.clone(),
            folds: BTreeMap::new(),
        };
        thread::Builder::new()
            .name("engaged index".into())
            .spawn(move || {
                lowest_priority();
                pass.run(replay, inbox)
            })?;
        Ok(Self {
            country,
            shared,
            snapshots,
            folding: Mutex::new(folding),
        })
    }

    pub(crate) fn country(&self) -> &Country {
        &self.country
    }

    /// Continues the pass over `replay` when it is a newer snapshot of the
    /// same recording ([`extends`]); otherwise the caller starts a new index.
    pub(crate) fn extend(&self, replay: &Arc<Replay>) -> bool {
        let mut folding = self.folding.lock().unwrap_or_else(|p| p.into_inner());
        if !extends(&folding, replay) {
            return false;
        }
        *folding = Arc::clone(replay);
        // A stopped pass keeps what it folded; there is nothing to continue.
        let _ = self.snapshots.send(Arc::clone(replay));
        true
    }

    /// Whether the pass has folded this hour.
    pub(crate) fn covers(&self, branch: BranchId, hour: Hour) -> bool {
        let branches = self
            .shared
            .branches
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        branches
            .get(&branch)
            .is_some_and(|covered| (covered.first..=covered.last).contains(&hour))
    }

    /// An index folded over `frames` (one branch, a load at `load`) on this
    /// thread, as the pass folds them.
    #[cfg(test)]
    pub(crate) fn folded(frames: &[DecodedFrame], country: Country, load: Option<Hour>) -> Self {
        let shared = Arc::new(Shared::default());
        let mut pass = Pass {
            shared: Arc::clone(&shared),
            country: country.clone(),
            folds: BTreeMap::new(),
        };
        for frame in frames {
            pass.fold_hour(frame.branch, frame.hour, Some(frame), load);
        }
        let (snapshots, _inbox) = mpsc::channel();
        Self {
            country,
            shared,
            snapshots,
            folding: Mutex::new(Arc::new(crate::playback::tests::fixture())),
        }
    }

    /// Whether [`combats`] answers this hour from the index for `viewer`.
    #[cfg(test)]
    pub(crate) fn answers(&self, frame: &DecodedFrame, viewer: u32) -> bool {
        SOURCES.iter().enumerate().all(|(kind, source)| {
            source.combats(frame).is_ok_and(|combats| {
                self.lookup(kind, frame.branch, frame.hour, viewer, &combats)
                    .is_ok()
            })
        })
    }

    /// The flag of each combat at a covered hour, for the viewer the pass
    /// resolved there; otherwise why the index cannot answer.
    fn lookup(
        &self,
        kind: usize,
        branch: BranchId,
        hour: Hour,
        viewer: u32,
        combats: &[u64],
    ) -> Result<BTreeMap<u64, Engaged>, Provenance> {
        let branches = self
            .shared
            .branches
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let covered = branches
            .get(&branch)
            .filter(|covered| (covered.first..=covered.last).contains(&hour))
            .ok_or(Provenance::Pending)?;
        match covered.viewer(hour) {
            Some(folded) if folded == viewer => {}
            Some(_) => return Err(Provenance::OtherViewer),
            None => return Err(Provenance::Unread),
        }
        // Every kind in SOURCES has its points from the first folded hour.
        let points = &covered.points[kind];
        let side = |combat: u64, side: u8| {
            points
                .get(&(combat, side))
                .and_then(|points| {
                    let at = points.partition_point(|(from, _)| *from <= hour);
                    Some(points.get(at.checked_sub(1)?)?.1)
                })
                .unwrap_or(Engaged::Unknown)
        };
        Ok(combats
            .iter()
            .map(|&combat| (combat, side(combat, 0).either(side(combat, 1))))
            .collect())
    }

    /// What each live icon of `kind` keeps at a covered hour, for the viewer
    /// the pass resolved there; otherwise why the index cannot answer.
    fn lookup_icons(
        &self,
        kind: usize,
        branch: BranchId,
        hour: Hour,
        viewer: u32,
        combats: &[u64],
    ) -> Result<BTreeMap<u64, Kept>, Provenance> {
        let branches = self
            .shared
            .branches
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let covered = branches
            .get(&branch)
            .filter(|covered| (covered.first..=covered.last).contains(&hour))
            .ok_or(Provenance::Pending)?;
        match covered.viewer(hour) {
            Some(folded) if folded == viewer => {}
            Some(_) => return Err(Provenance::OtherViewer),
            None => return Err(Provenance::Unread),
        }
        let points = &covered.icons[kind];
        Ok(combats
            .iter()
            .filter_map(|&combat| {
                let points = points.get(&combat)?;
                let at = points.partition_point(|(from, _)| *from <= hour);
                Some((combat, points.get(at.checked_sub(1)?)?.1?))
            })
            .collect())
    }
}

/// Whether `newer` is `older` with more recorded, so what the pass folded
/// from `older` stands: the same campaign and format, every branch start of
/// `older` unchanged (its reason and hours decide how its first hour folds,
/// [`native_load`]), and every block of `older` at the same place with the
/// same bytes. Block positions and lengths alone do not identify content.
fn extends(older: &Replay, newer: &Replay) -> bool {
    fn bytes<'a>(replay: &'a Replay, block: &BlockIndex) -> Option<&'a [u8]> {
        let start = usize::try_from(block.offset).ok()?;
        let end = start.checked_add(usize::try_from(block.length).ok()?)?;
        replay.bytes().get(start..end)
    }
    older.campaign_id == newer.campaign_id
        && older.format == newer.format
        && older.branches.iter().all(|start| {
            newer
                .branches
                .iter()
                .find(|newer| newer.id == start.id)
                .is_some_and(|newer| newer == start)
        })
        && newer.index.starts_with(&older.index)
        && older
            .index
            .iter()
            .all(|block| bytes(older, block).is_some_and(|was| bytes(newer, block) == Some(was)))
}

/// One branch's fold state on the pass thread.
#[derive(Default)]
struct Fold {
    histories: Vec<engaged::History>,
    /// The last folded hour, and whether it decoded.
    last: Option<(Hour, bool)>,
    /// Per kind, reused each hour: the sides that took a new value, and those
    /// whose combat ended.
    changed: Vec<Vec<((u64, u8), Engaged)>>,
    ended: Vec<Vec<(u64, u8)>>,
    /// Per icon kind: the live instances, and those that changed this hour.
    icons: Vec<IconFold>,
    icon_changed: Vec<Vec<(u64, Option<Kept>)>>,
}

/// The hour a branch starts with a native load, which rebuilds every side
/// (1413CEBC0): a load signal whose loaded hour is its first sampled hour.
/// The collector drops the origin after an unreadable callback while still
/// seeing the new signal, so a first sample without that origin, or after it,
/// may follow unobserved hours whose refreshes (1413CEBE0) no current sample
/// recovers; those sides start unknown. The recorder's fresh-save oracle
/// (`Report::set_load_hour`) takes the same test. Other starts join a running
/// game.
fn native_load(start: &BranchStart) -> Option<Hour> {
    (start.reason == BranchReason::LoadSignal && start.loaded_hour == Some(start.first_hour))
        .then_some(start.first_hour)
}

struct Pass {
    shared: Arc<Shared>,
    country: Country,
    folds: BTreeMap<BranchId, Fold>,
}

impl Pass {
    fn run(mut self, mut replay: Arc<Replay>, inbox: Receiver<Arc<Replay>>) {
        let mut tables: Vec<TableId> = VIEWER_TABLES.to_vec();
        for source in SOURCES {
            tables.extend_from_slice(source.tables());
        }
        tables.extend_from_slice(&ICON_TABLES);
        tables.sort_by_key(|table| table.index());
        tables.dedup();
        let mut opened = true;
        loop {
            let started = Instant::now();
            let hours = self.fold(&replay, &tables);
            if self.cancelled() {
                return;
            }
            // The open cost is always logged; each extension only when profiling.
            if opened || crate::map::counter_move::profiling() {
                let (sides, points, bytes) = self.size();
                crate::map::counter_move::profile_print(format!(
                    "Engaged index: folded {hours} hours in {:.2} s; {sides} combat sides, {points} change points, about {} KiB",
                    started.elapsed().as_secs_f64(),
                    bytes / 1024
                ));
            }
            opened = false;
            // Wait for a newer snapshot; the window dropping the index ends it.
            match inbox.recv() {
                Ok(newer) => replay = newer,
                Err(_) => return,
            }
        }
    }

    fn cancelled(&self) -> bool {
        self.shared.cancelled.load(Ordering::Acquire)
    }

    /// Folds every hour past each branch's last folded one; returns how many.
    fn fold(&mut self, replay: &Replay, tables: &[TableId]) -> usize {
        let mut blocks: Vec<_> = replay.index.iter().collect();
        blocks.sort_by_key(|block| (block.branch, block.first_hour));
        let loads: BTreeMap<BranchId, Option<Hour>> = replay
            .branches
            .iter()
            .map(|start| (start.id, native_load(start)))
            .collect();
        let mut hours = 0;
        for block in blocks {
            if self.cancelled() {
                return hours;
            }
            let fold = self.folds.entry(block.branch).or_default();
            let folded = fold.last.map(|(hour, _)| hour);
            if folded.is_some_and(|last| block.last_hour <= last) {
                continue;
            }
            let Ok(frames) = replay.read_block_selected(block, tables) else {
                // The next folded hour follows unobserved ones.
                fold.last = Some((block.last_hour, false));
                continue;
            };
            let mut decoder = FrameDecoder::for_tables(tables);
            for frame in frames {
                if self.cancelled() {
                    return hours;
                }
                // Earlier hours still advance the decoder's baselines.
                let decoded = decoder.advance(&frame);
                if folded.is_some_and(|last| frame.hour <= last) {
                    continue;
                }
                let decoded = decoded.ok().filter(|decoded| {
                    core::<tables::Campaign>(decoded)
                        .is_ok_and(|rows| rows.len() == 1 && rows[0].id == replay.campaign_id)
                });
                let load = loads.get(&block.branch).copied().flatten();
                self.fold_hour(block.branch, frame.hour, decoded, load);
                hours += 1;
            }
        }
        hours
    }

    /// Folds one hour; an undecodable one leaves every side unknown until it
    /// rebuilds. `load` is the hour the branch starts with a native load.
    ///
    /// The fold runs without the lock; the lock is held only to append the
    /// hour's changes.
    fn fold_hour(
        &mut self,
        branch: BranchId,
        hour: Hour,
        frame: Option<&DecodedFrame>,
        load: Option<Hour>,
    ) {
        let fold = self.folds.entry(branch).or_default();
        fold.histories.resize_with(SOURCES.len(), Default::default);
        fold.changed.resize_with(SOURCES.len(), Vec::new);
        fold.ended.resize_with(SOURCES.len(), Vec::new);
        fold.icons.resize_with(ICON_KINDS, Default::default);
        fold.icon_changed.resize_with(ICON_KINDS, Vec::new);
        let continuity = match fold.last {
            None if load == Some(hour) => Continuity::Load,
            // A branch that joined a running game, or whose first hours
            // were not folded, starts after unobserved rebuilds.
            None => Continuity::Gap,
            Some((last, true)) if last.checked_add(1) == Some(hour) => Continuity::Next,
            Some(_) => Continuity::Gap,
        };
        let read = frame.and_then(|frame| {
            let viewer = Viewer::read(frame, &self.country).ok()?;
            let sides = SOURCES
                .iter()
                .map(|source| source.sides(frame))
                .collect::<io::Result<Vec<_>>>()
                .ok()?;
            Some((viewer, sides))
        });
        fold.last = Some((hour, read.is_some()));
        let slot = read.as_ref().map(|(viewer, _)| viewer.slot);
        match read {
            Some((viewer, sides)) => {
                for (kind, sides) in sides.into_iter().enumerate() {
                    fold.histories[kind].advance_reporting(
                        viewer.sample(sides),
                        continuity,
                        &mut fold.changed[kind],
                        &mut fold.ended[kind],
                    );
                }
                let land = &fold.histories[LAND];
                let flag = |combat: u64| land.combat(combat);
                for (kind, (icons, changed)) in fold
                    .icons
                    .iter_mut()
                    .zip(&mut fold.icon_changed)
                    .enumerate()
                {
                    // A frame decoded, so an icon table it cannot read is
                    // one the hour holds nothing of: its icons are lost.
                    match frame.map(|frame| {
                        icon_hour(
                            kind,
                            frame,
                            viewer.slot,
                            &viewer.links,
                            viewer.relations.as_ref().map_err(|reason| *reason),
                            &flag,
                        )
                    }) {
                        Some(Ok(hour)) => icons.advance(&hour, continuity, changed),
                        _ => icons.lose(changed),
                    }
                }
            }
            None => {
                for ((history, changed), ended) in fold
                    .histories
                    .iter_mut()
                    .zip(&mut fold.changed)
                    .zip(&mut fold.ended)
                {
                    let held = mem::take(history);
                    changed.clear();
                    ended.clear();
                    changed.extend(
                        held.sides()
                            .filter(|(_, value)| *value != Engaged::Unknown)
                            .map(|(key, _)| (key, Engaged::Unknown)),
                    );
                }
                for (icons, changed) in fold.icons.iter_mut().zip(&mut fold.icon_changed) {
                    icons.lose(changed);
                }
            }
        }
        let mut branches = self
            .shared
            .branches
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let covered = branches.entry(branch).or_insert_with(|| Covered {
            first: hour,
            last: hour,
            points: vec![BTreeMap::new(); SOURCES.len()],
            icons: vec![BTreeMap::new(); ICON_KINDS],
            viewers: Vec::new(),
        });
        covered.last = hour;
        if let Some(slot) = slot
            && covered.viewers.last().is_none_or(|(_, last)| *last != slot)
        {
            covered.viewers.push((hour, slot));
        }
        for ((points, changed), ended) in covered
            .points
            .iter_mut()
            .zip(&fold.changed)
            .zip(&fold.ended)
        {
            record(points, hour, changed.iter().copied());
            // An ended combat takes no more points: keep only what it holds.
            for key in ended {
                if let Some(changes) = points.get_mut(key) {
                    changes.shrink_to_fit();
                }
            }
        }
        for (points, changed) in covered.icons.iter_mut().zip(&fold.icon_changed) {
            for &(combat, kept) in changed {
                points.entry(combat).or_default().push((hour, kept));
            }
        }
    }

    /// Combat sides, change points and an estimate of their bytes.
    fn size(&self) -> (usize, usize, usize) {
        let branches = self
            .shared
            .branches
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let (mut sides, mut points, mut bytes) = (0, 0, 0);
        for covered in branches.values() {
            bytes += covered.viewers.capacity() * size_of::<(Hour, u32)>();
            for kind in &covered.points {
                sides += kind.len();
                for changes in kind.values() {
                    points += changes.len();
                    // Key, vector header and a B-tree slot's share.
                    bytes += size_of::<(u64, u8)>()
                        + size_of::<Vec<(Hour, Engaged)>>()
                        + 16
                        + changes.capacity() * size_of::<(Hour, Engaged)>();
                }
            }
        }
        (sides, points, bytes)
    }
}

/// Appends each side's value where it differs from its last point.
fn record(points: &mut Changes, hour: Hour, held: impl Iterator<Item = ((u64, u8), Engaged)>) {
    for (key, value) in held {
        let changes = points.entry(key).or_default();
        if changes.last().is_none_or(|(_, last)| *last != value) {
            changes.push((hour, value));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The index keeps one point per value change: 1,500 combats whose units
    /// keep changing but whose flags never do cost one point per side,
    /// however many hours fold.
    #[test]
    fn memory_follows_value_changes_not_hours() {
        const COMBATS: u64 = 1_500;
        let links: BTreeMap<u32, u32> = (1..=2 * COMBATS as u32).map(|c| (c, c)).collect();
        let relations = Relations::factions_for_tests(&[]);
        let mut history = engaged::History::default();
        let mut points = Changes::new();
        for hour in 0..240 {
            let sides = (0..COMBATS)
                .flat_map(|combat| {
                    [0, 1].map(|side| {
                        (
                            (combat, side),
                            engaged::Side {
                                // A unit joins every tenth hour: a rebuild.
                                units: Some(vec![combat, (hour / 10) as u64]),
                                added: None,
                                lists: Some((
                                    vec![2 * combat as u32 + u32::from(side) + 1],
                                    vec![],
                                )),
                                unit_lists: Some((
                                    vec![2 * combat as u32 + u32::from(side) + 1],
                                    vec![],
                                )),
                            },
                        )
                    })
                })
                .collect();
            let continuity = if hour == 0 {
                Continuity::Load
            } else {
                Continuity::Next
            };
            history.advance(
                engaged::Sample {
                    viewer: 0,
                    links: &links,
                    relations: Ok(&relations),
                    controllers: Some(Default::default()),
                    sides,
                    complete: true,
                    removal: engaged::Removal::BothSidesUnlessRoundRetreat,
                },
                continuity,
            );
            record(&mut points, hour, history.sides());
        }
        assert_eq!(points.len(), 2 * COMBATS as usize);
        assert!(
            points
                .values()
                .all(|changes| changes == &[(0, Engaged::No)])
        );
    }

    fn branch_start(reason: BranchReason, loaded_hour: Option<Hour>) -> BranchStart {
        BranchStart {
            id: 1,
            session: 1,
            loaded_hour,
            first_hour: 20,
            parent: replay::branches::Parentage::Unknown,
            reason,
        }
    }

    /// Production branch starts: the collector records the loaded hour as
    /// the first hour for a load signal and an hour discontinuity alike, and
    /// none for a session start. Only a load signal sampled at its loaded
    /// hour is a native load: the collector drops the origin after an
    /// unreadable callback (spooling `suppress_origin`) while still seeing
    /// the new signal, and a first sample after the loaded hour follows
    /// unobserved ones.
    #[test]
    fn only_a_load_signal_at_its_loaded_hour_is_a_native_load() {
        let start = branch_start;
        assert_eq!(
            native_load(&start(BranchReason::LoadSignal, Some(20))),
            Some(20)
        );
        assert_eq!(native_load(&start(BranchReason::LoadSignal, None)), None);
        assert_eq!(
            native_load(&start(BranchReason::LoadSignal, Some(19))),
            None
        );
        assert_eq!(
            native_load(&start(BranchReason::HourDiscontinuity, Some(20))),
            None
        );
        assert_eq!(native_load(&start(BranchReason::SessionStart, None)), None);
    }

    /// A load signal whose origin was lost or precedes the first sample does
    /// not rebuild its sides as historical: the pass folds that first hour as
    /// a gap, so the flag stays unknown until a side rebuilds, where a load
    /// sampled at its loaded hour rebuilds it (No: AAA shares vision with
    /// neither side).
    #[test]
    fn a_load_without_its_origin_folds_its_first_hour_as_a_gap() {
        let fold = |loaded_hour| {
            let start = branch_start(BranchReason::LoadSignal, loaded_hour);
            let mut frame = roster_frame(start.first_hour, Some(&[(40, 4)]), false);
            frame.branch = start.id;
            let shared = Arc::new(Shared::default());
            let country = Country::Tag("AAA".into());
            let mut pass = Pass {
                shared: Arc::clone(&shared),
                country: country.clone(),
                folds: BTreeMap::new(),
            };
            pass.fold_hour(frame.branch, frame.hour, Some(&frame), native_load(&start));
            let (snapshots, _inbox) = mpsc::channel();
            let index = EngagedIndex {
                country,
                shared,
                snapshots,
                folding: Mutex::new(Arc::new(crate::playback::tests::fixture())),
            };
            let viewer = Viewer::read(&frame, &index.country).unwrap();
            combats(
                &frame,
                LAND,
                viewer.slot,
                &viewer.links,
                viewer.relations.as_ref().map_err(|reason| *reason),
                Some(&index),
            )
            .unwrap()
        };
        let flag = |engaged| (BTreeMap::from([(62, engaged)]), Provenance::Indexed);
        assert_eq!(fold(Some(20)), flag(Engaged::No));
        assert_eq!(fold(None), flag(Engaged::Unknown));
        assert_eq!(fold(Some(19)), flag(Engaged::Unknown));
    }

    #[test]
    fn five_thousand_countries_resolve_and_fold() {
        use replay::{
            frame::TableView,
            rows::{CountryLinkRow, CountryRow, TableRows},
        };
        const COUNTRIES: u32 = 5_000;
        const COMBATS: u64 = 1_500;
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 0,
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "not captured".into(),
                }
                .into()
            }),
            metrics: Default::default(),
        };
        *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
            (0..COUNTRIES)
                .map(|slot| CountryRow {
                    id: slot + 1,
                    canonical_slot: slot,
                    tag: format!("C{slot:04}").into_bytes(),
                    ..Default::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(
                (0..COUNTRIES)
                    .map(|slot| CountryLinkRow {
                        id: slot + 1,
                        canonical_slot: slot,
                    })
                    .collect(),
            ));
        for table in [
            TableId::Players,
            TableId::HumanControllers,
            TableId::FactionMembers,
            TableId::WarRelations,
            TableId::Subjects,
        ] {
            *frame.tables[table.index()] = TableView::Complete(TableRows::empty(table));
        }
        let started = Instant::now();
        let viewer = Viewer::read(&frame, &Country::Tag("c4999".into())).unwrap();
        let read = started.elapsed();
        assert_eq!(viewer.slot, COUNTRIES - 1);
        assert_eq!(viewer.links.len(), COUNTRIES as usize);
        let mut history = engaged::History::default();
        let mut points = Changes::new();
        let (mut changed, mut ended) = (Vec::new(), Vec::new());
        let started = Instant::now();
        for hour in 0..48 {
            let sides = (0..COMBATS)
                .flat_map(|combat| {
                    [0_u8, 1].map(|side| {
                        (
                            (combat, side),
                            engaged::Side {
                                units: Some(vec![combat]),
                                added: None,
                                lists: Some((
                                    vec![(combat as u32 * 3 + u32::from(side)) % COUNTRIES + 1],
                                    vec![],
                                )),
                                unit_lists: Some((
                                    vec![(combat as u32 * 3 + u32::from(side)) % COUNTRIES + 1],
                                    vec![],
                                )),
                            },
                        )
                    })
                })
                .collect();
            let continuity = if hour == 0 {
                Continuity::Load
            } else {
                Continuity::Next
            };
            history.advance_reporting(
                engaged::Sample {
                    viewer: viewer.slot,
                    links: &viewer.links,
                    relations: viewer.relations.as_ref().map_err(|reason| *reason),
                    controllers: viewer.controllers.clone(),
                    sides,
                    complete: true,
                    removal: engaged::Removal::BothSidesUnlessRoundRetreat,
                },
                continuity,
                &mut changed,
                &mut ended,
            );
            record(&mut points, hour, changed.iter().copied());
            // After the first hour nothing changes, so nothing is appended
            // under the lock.
            assert_eq!(
                changed.len(),
                if hour == 0 { 2 * COMBATS as usize } else { 0 }
            );
            assert!(ended.is_empty());
        }
        println!(
            "5,000 countries: viewer read {:.3} ms; 48 hours of 1,500 combats folded in {:.3} ms",
            read.as_secs_f64() * 1e3,
            started.elapsed().as_secs_f64() * 1e3
        );
        assert_eq!(points.len(), 2 * COMBATS as usize);
        // No side names the viewer and none shares vision with it: one
        // point per side.
        assert!(points.values().all(|changes| changes.len() == 1));
    }

    #[test]
    fn a_side_whose_combat_ends_is_reported_once() {
        let links = BTreeMap::from([(1, 1)]);
        let relations = Relations::factions_for_tests(&[]);
        let side = || engaged::Side {
            units: Some(vec![1]),
            added: None,
            lists: Some((vec![1], vec![])),
            unit_lists: Some((vec![1], vec![])),
        };
        let mut history = engaged::History::default();
        let (mut changed, mut ended) = (Vec::new(), Vec::new());
        for (hour, sides, expect_changed, expect_ended) in [
            (0, vec![(7, 0), (7, 1)], 2, 0),
            (1, vec![(7, 0), (7, 1)], 0, 0),
            (2, vec![], 0, 2),
        ] {
            history.advance_reporting(
                engaged::Sample {
                    viewer: 1,
                    links: &links,
                    relations: Ok(&relations),
                    controllers: Some(Default::default()),
                    sides: sides.into_iter().map(|key| (key, side())).collect(),
                    complete: true,
                    removal: engaged::Removal::BothSidesUnlessRoundRetreat,
                },
                if hour == 0 {
                    Continuity::Load
                } else {
                    Continuity::Next
                },
                &mut changed,
                &mut ended,
            );
            assert_eq!((changed.len(), ended.len()), (expect_changed, expect_ended));
        }
    }

    #[test]
    fn a_lookup_takes_the_last_change_at_or_before_the_hour_for_the_folded_viewer() {
        let shared = Arc::new(Shared::default());
        let mut points = BTreeMap::new();
        record(
            &mut points,
            10,
            [((62, 0), Engaged::No), ((62, 1), Engaged::No)].into_iter(),
        );
        record(
            &mut points,
            11,
            [((62, 0), Engaged::No), ((62, 1), Engaged::No)].into_iter(),
        );
        record(
            &mut points,
            14,
            [((62, 0), Engaged::No), ((62, 1), Engaged::Yes)].into_iter(),
        );
        assert_eq!(points[&(62, 1)], [(10, Engaged::No), (14, Engaged::Yes)]);
        shared.branches.lock().unwrap().insert(
            3,
            Covered {
                first: 10,
                last: 20,
                points: vec![points],
                icons: vec![BTreeMap::new(); ICON_KINDS],
                viewers: vec![(10, 1)],
            },
        );
        let (snapshots, _inbox) = mpsc::channel();
        let index = EngagedIndex {
            country: Country::Recorded,
            shared,
            snapshots,
            folding: Mutex::new(Arc::new(crate::playback::tests::fixture())),
        };
        let at = |hour, viewer| index.lookup(LAND, 3, hour, viewer, &[62]);
        assert_eq!(at(13, 1), Ok(BTreeMap::from([(62, Engaged::No)])));
        assert_eq!(at(14, 1), Ok(BTreeMap::from([(62, Engaged::Yes)])));
        // Past the fold, another viewer or another branch is not covered.
        assert_eq!(at(21, 1), Err(Provenance::Pending));
        assert_eq!(at(14, 2), Err(Provenance::OtherViewer));
        assert_eq!(
            index.lookup(LAND, 4, 14, 1, &[62]),
            Err(Provenance::Pending)
        );
    }

    /// Countries 1..=4 (slot = id, tags AAA to DDD) and land combat 62:
    /// country 2 attacks country 3. `players` are (key, country), `None`
    /// leaving `Players` unrecorded; `allied` puts 1 and 2 in one faction.
    fn roster_frame(hour: Hour, players: Option<&[(i32, u32)]>, allied: bool) -> DecodedFrame {
        use replay::{
            frame::TableView,
            rows::{
                CountryLinkRow, CountryRow, FactionMemberRow, HumanControllerRow, LandCombatRow,
                LandCombatSideRow, PlayerRow, TableRows,
            },
        };
        let mut frame = DecodedFrame {
            branch: 1,
            hour,
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "not captured".into(),
                }
                .into()
            }),
            metrics: Default::default(),
        };
        let mut set = |table: TableId, rows: TableRows| {
            *frame.tables[table.index()] = TableView::Complete(rows);
        };
        set(
            TableId::Countries,
            TableRows::Countries(
                (1..=4)
                    .zip(["AAA", "BBB", "CCC", "DDD"])
                    .map(|(id, tag)| CountryRow {
                        id,
                        canonical_slot: id,
                        tag: tag.as_bytes().to_vec(),
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        set(
            TableId::CountryLinks,
            TableRows::CountryLinks(
                (1..=4)
                    .map(|id| CountryLinkRow {
                        id,
                        canonical_slot: id,
                    })
                    .collect(),
            ),
        );
        set(
            TableId::FactionMembers,
            TableRows::FactionMembers(if allied {
                vec![
                    FactionMemberRow {
                        country: 1,
                        faction: 9,
                        ordinal: 0,
                    },
                    FactionMemberRow {
                        country: 2,
                        faction: 9,
                        ordinal: 1,
                    },
                ]
            } else {
                vec![]
            }),
        );
        for table in [TableId::WarRelations, TableId::Subjects] {
            set(table, TableRows::empty(table));
        }
        if let Some(players) = players {
            set(
                TableId::Players,
                TableRows::Players(
                    players
                        .iter()
                        .map(|&(id, country)| PlayerRow {
                            id,
                            country,
                            ..Default::default()
                        })
                        .collect(),
                ),
            );
            let mut counts = BTreeMap::<u32, i8>::new();
            for &(_, country) in players {
                *counts.entry(country).or_default() += 1;
            }
            set(
                TableId::HumanControllers,
                TableRows::HumanControllers(
                    counts
                        .into_iter()
                        .map(|(country, count)| HumanControllerRow { country, count })
                        .collect(),
                ),
            );
        }
        set(
            TableId::LandCombats,
            TableRows::LandCombats(vec![LandCombatRow {
                id: 62,
                attackers: vec![7],
                defenders: vec![9],
                ..Default::default()
            }]),
        );
        set(
            TableId::LandCombatSides,
            TableRows::LandCombatSides(vec![LandCombatSideRow {
                combat: 62,
                attacker_primary: vec![2],
                defender_primary: vec![3],
                ..Default::default()
            }]),
        );
        frame
    }

    /// Folds `hours` for AAA from a load at hour 0 and answers the last one
    /// through [`combats`], as the window does.
    fn fold_for_aaa(hours: &[DecodedFrame]) -> (BTreeMap<u64, Engaged>, Provenance) {
        let shared = Arc::new(Shared::default());
        let country = Country::Tag("AAA".into());
        let mut pass = Pass {
            shared: Arc::clone(&shared),
            country: country.clone(),
            folds: BTreeMap::new(),
        };
        for frame in hours {
            pass.fold_hour(frame.branch, frame.hour, Some(frame), Some(0));
        }
        let (snapshots, _inbox) = mpsc::channel();
        let index = EngagedIndex {
            country,
            shared,
            snapshots,
            folding: Mutex::new(Arc::new(crate::playback::tests::fixture())),
        };
        let last = hours.last().unwrap();
        let viewer = Viewer::read(last, &index.country).unwrap();
        combats(
            last,
            LAND,
            viewer.slot,
            &viewer.links,
            viewer.relations.as_ref().map_err(|reason| *reason),
            Some(&index),
        )
        .unwrap()
    }

    /// Player 40 moving from DDD to AAA may be a controller change into the
    /// viewer's country (14113E4B0 then 140BABC00), which would refresh the
    /// flag the earlier alliance left stale: the rosters make it ambiguous,
    /// never Yes. Unrecorded rosters cannot rule it out either.
    #[test]
    fn a_controller_joining_the_viewer_reaches_the_index_as_a_possible_refresh() {
        let fold = |last: Option<&[(i32, u32)]>| {
            fold_for_aaa(&[
                roster_frame(0, Some(&[(40, 4)]), false),
                roster_frame(1, Some(&[(40, 4)]), true),
                roster_frame(2, last, true),
            ])
        };
        let flag = |engaged| (BTreeMap::from([(62, engaged)]), Provenance::Indexed);
        assert_eq!(fold(Some(&[(40, 4)])), flag(Engaged::No));
        let scope = Engaged::Ambiguous(engaged::Ambiguity::Scope);
        assert_eq!(fold(Some(&[(40, 1)])), flag(scope));
        assert_eq!(fold(None), flag(scope));
    }

    /// A flag the index does not answer is still the load rebuild, and says
    /// why: not folded yet, no index at all, or no viewer read on the branch.
    #[test]
    fn a_load_rebuilt_flag_reports_its_provenance() {
        let frames = [
            roster_frame(0, Some(&[(40, 4)]), false),
            roster_frame(1, Some(&[(40, 4)]), true),
        ];
        let viewer = Viewer::read(&frames[1], &Country::Tag("AAA".into())).unwrap();
        let rebuilt = |index: Option<&EngagedIndex>| {
            combats(
                &frames[1],
                LAND,
                viewer.slot,
                &viewer.links,
                viewer.relations.as_ref().map_err(|reason| *reason),
                index,
            )
            .unwrap()
        };
        let yes = BTreeMap::from([(62, Engaged::Yes)]);
        assert_eq!(rebuilt(None), (yes.clone(), Provenance::Unindexed));
        let shared = Arc::new(Shared::default());
        let mut pass = Pass {
            shared: Arc::clone(&shared),
            country: Country::Tag("AAA".into()),
            folds: BTreeMap::new(),
        };
        pass.fold_hour(1, 0, Some(&frames[0]), Some(0));
        let (snapshots, _inbox) = mpsc::channel();
        let index = EngagedIndex {
            country: Country::Tag("AAA".into()),
            shared,
            snapshots,
            folding: Mutex::new(Arc::new(crate::playback::tests::fixture())),
        };
        assert_eq!(rebuilt(Some(&index)), (yes.clone(), Provenance::Pending));
        // An unreadable first hour folds no viewer.
        let mut unread = frames[1].clone();
        unread.hour = 0;
        pass.fold_hour(2, 0, None, None);
        unread.branch = 2;
        assert_eq!(
            combats(
                &unread,
                LAND,
                viewer.slot,
                &viewer.links,
                viewer.relations.as_ref().map_err(|reason| *reason),
                Some(&index),
            )
            .unwrap(),
            (yes, Provenance::Unread)
        );
        for provenance in [
            Provenance::Pending,
            Provenance::OtherViewer,
            Provenance::Unread,
            Provenance::Unindexed,
        ] {
            assert!(
                provenance
                    .limitation()
                    .is_some_and(|line| line.starts_with("engaged flag load-rebuilt; "))
            );
        }
        assert_eq!(Provenance::Indexed.limitation(), None);
    }

    /// A newer snapshot keeps the folded index only when every folded block
    /// has the same bytes and every branch start is the same, not merely the
    /// same block positions.
    #[test]
    fn a_snapshot_extends_the_index_only_with_the_same_blocks_and_branch_starts() {
        use replay::spool::Record;
        let open =
            |records: Vec<Record>| Replay::from_records(b"campaign".to_vec(), &[records]).unwrap();
        let records = || crate::playback::tests::records(1, &[100, 101, 102]);
        let older = Arc::new(open(records()));
        let same = Arc::new(Replay::open(older.bytes().to_vec()).unwrap());
        // Equal metadata, one block byte changed.
        let mut bytes = older.bytes().to_vec();
        let block = &older.index[0];
        bytes[(block.offset + block.length - 1) as usize] ^= 1;
        let changed = Arc::new(Replay::open(bytes).unwrap());
        assert_eq!(
            (&changed.index, &changed.branches),
            (&older.index, &older.branches)
        );
        // The same blocks, but the branch now starts with a load signal.
        let mut loaded = records();
        for record in &mut loaded {
            if let Record::BranchStart(start) = record {
                start.reason = BranchReason::LoadSignal;
            }
        }
        let reloaded = Arc::new(open(loaded));
        assert_eq!(reloaded.index, older.index);
        let index = EngagedIndex::start(Arc::clone(&older), Country::Recorded).unwrap();
        assert!(!index.extend(&changed));
        assert!(!index.extend(&reloaded));
        assert!(index.extend(&same));
    }
}
