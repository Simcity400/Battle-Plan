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
//! A combat kind registers its sides as a [`Sides`] source in [`SOURCES`]:
//! land and naval. The pass is one more decoder consumer: it reads
//! blocks through [`Replay::read_block_alone`] and never shares the
//! playback decoder.
//!
//! Whatever the perspective, fold the recording player's own flag history too.
//! Each player change rebuilds every side only for the new player. Tag switches
//! rebuild immediately; observer-view changes and controller assignments rebuild
//! on the next tick, so their history takes that change one sample later
//! ([`Pass::fold_hour`]). Combat icon frames follow this same player history.
//!
//! When the viewed country is the recording player, answer from that history;
//! otherwise use the perspective history, which does not rebuild for those player
//! changes. Check the naval history against recorded bytes on the pass thread
//! ([`naval_check`]), publishing [`History::byte_check`].
//! [`check_recorded_bytes`] uses the same fold for a dump.

use job_priority::SpawnCarried as _;
use replay::campaign::Replay;
use replay_data::{BranchId, Hour, rows::TableId, rows::tables};
use replay_frame::{branches::BranchReason, branches::BranchStart, frame::DecodedFrame};
use std::{
    collections::BTreeMap,
    io, mem,
    sync::{
        Arc, Mutex,
        atomic::Ordering,
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::Instant,
};
use viewer_foundation::engaged::HourSides;
use viewer_foundation::engagement_history::{
    Changes, Covered, History, ICON_KINDS, NAVAL, NAVAL_ICON, SOURCES, icon_hour, icon_source,
};
#[cfg(test)]
use viewer_foundation::engagement_history::{
    CombatFlags, LAND, Provenance, at, combats, kept_frames,
};
use viewer_foundation::profile_writer::profile_print;
use viewer_foundation::profile_writer::profiling;
use viewer_foundation::{
    engaged::{self, Continuity, Engaged},
    icon_frames::{IconFold, Kept},
    map::{
        perspective::{self, Country},
        tables::{Available, core, rows},
        visibility::Relations,
    },
    naval_check::{self, ByteCheck, Summary},
    worker_priority::lowest_priority,
};

/// The tables the icon kinds read besides the viewer's.
const ICON_TABLES: [TableId; 5] = [
    TableId::LandCombats,
    TableId::LandCombatSides,
    TableId::NavalCombats,
    TableId::NavalCombatShips,
    TableId::NavalCombatDetails,
];

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

    /// The recording's own player's inputs, when the recording names it
    /// ([`naval_check::recording_player`]), and whether its client observes.
    fn recording(frame: &DecodedFrame) -> io::Result<Option<(Self, bool)>> {
        let links: BTreeMap<u32, u32> = core::<tables::CountryLinks>(frame)?
            .iter()
            .map(|link| (link.id, link.canonical_slot))
            .collect();
        let Some(slot) = naval_check::recording_player(frame, &links)? else {
            return Ok(None);
        };
        // Use the client's playing-country reference when positive, otherwise
        // its observer-country reference.
        let observing = frame
            .complete::<tables::LocalPlayer>()
            .is_ok_and(|rows| rows.iter().all(|row| row.country as i32 <= 0));
        let relations = Relations::read(frame, &links);
        let controllers = controllers(frame, &links)?;
        Ok(Some((
            Self {
                slot,
                links,
                relations,
                controllers,
            },
            observing,
        )))
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

/// The index for one perspective country; dropping it stops the pass.
pub(crate) struct EngagedIndex {
    country: Country,
    pub(crate) shared: Arc<History>,
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
        let shared = Arc::new(History::default());
        let (snapshots, inbox) = mpsc::channel();
        let folding = Arc::clone(&replay);
        let pass = Pass::new(Arc::clone(&shared), country.clone());
        thread::Builder::new()
            .name("engaged index".into())
            .spawn_carried(move || {
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

    /// An index folded over `frames` (one branch, a load at `load`) on this
    /// thread, as the pass folds them.
    #[cfg(test)]
    pub(crate) fn folded(frames: &[DecodedFrame], country: Country, load: Option<Hour>) -> Self {
        let shared = Arc::new(History::default());
        let mut pass = Pass::new(Arc::clone(&shared), country.clone());
        for frame in frames {
            pass.fold_hour(frame.branch, frame.hour, Some(frame), load, false);
        }
        let (snapshots, _inbox) = mpsc::channel();
        Self {
            country,
            shared,
            snapshots,
            folding: Mutex::new(Arc::new(crate::playback::tests::fixture())),
        }
    }
}

/// Whether `newer` is `older` with more recorded, so what the pass folded
/// from `older` stands: the same campaign and format, every branch start of
/// `older` unchanged (its reason and hours decide how its first hour folds,
/// [`native_load`]), unchanged sessions/gaps, and the recorded block index
/// retained as a prefix. Source reuse follows the metadata-only append policy;
/// this check performs no compressed-byte reads or hashes.
fn extends(older: &Replay, newer: &Replay) -> bool {
    older.campaign_id == newer.campaign_id
        && older.format == newer.format
        && older.identity == newer.identity
        && newer.sessions.starts_with(&older.sessions)
        && newer.gaps.starts_with(&older.gaps)
        && older.branches.iter().all(|start| {
            newer
                .branches
                .iter()
                .find(|newer| newer.id == start.id)
                .is_some_and(|newer| newer == start)
        })
        && newer.index.starts_with(&older.index)
}

/// One viewer's histories of every kind and their per-hour change buffers.
#[derive(Default)]
struct Track {
    histories: Vec<engaged::History>,
    /// Per kind, reused each hour: the sides that took a new value, and those
    /// whose combat ended.
    changed: Vec<Vec<((u64, u8), Engaged)>>,
    ended: Vec<Vec<(u64, u8)>>,
}

impl Track {
    fn sized(&mut self) {
        self.histories.resize_with(SOURCES.len(), Default::default);
        self.changed.resize_with(SOURCES.len(), Vec::new);
        self.ended.resize_with(SOURCES.len(), Vec::new);
    }

    fn advance(&mut self, viewer: &Viewer, sides: Vec<HourSides>, continuity: Continuity) {
        self.sized();
        for (kind, sides) in sides.into_iter().enumerate() {
            self.histories[kind].advance_reporting(
                viewer.sample(sides),
                continuity,
                &mut self.changed[kind],
                &mut self.ended[kind],
            );
        }
    }

    /// An hour this viewer's flags were not folded for: every side it held
    /// becomes unknown until it rebuilds.
    fn forget(&mut self) {
        self.sized();
        for ((history, changed), ended) in self
            .histories
            .iter_mut()
            .zip(&mut self.changed)
            .zip(&mut self.ended)
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
    }

    /// Appends the hour's changes to `points`.
    fn record(&self, points: &mut [Changes], hour: Hour) {
        for ((points, changed), ended) in points.iter_mut().zip(&self.changed).zip(&self.ended) {
            record(points, hour, changed.iter().copied());
            // An ended combat takes no more points: keep only what it holds.
            for key in ended {
                if let Some(changes) = points.get_mut(key) {
                    changes.shrink_to_fit();
                }
            }
        }
    }
}

/// One branch's fold state on the pass thread.
#[derive(Default)]
struct Fold {
    /// The perspective's viewer.
    perspective: Track,
    /// The last folded hour, and whether its perspective folded.
    last: Option<(Hour, bool)>,
    /// The recording's own player.
    recording: Track,
    /// The last folded hour, and whether the recording player's folded.
    recording_last: Option<(Hour, bool)>,
    /// The recording client's view (its slot) at the last hour its history
    /// folded.
    client_view: Option<u32>,
    /// Per icon kind, for the perspective's viewer: the live instances, and
    /// those that changed this hour.
    icons: Vec<IconFold>,
    icon_changed: Vec<Vec<(u64, Option<Kept>)>>,
    /// The same for the recording's own player.
    recording_icons: Vec<IconFold>,
    recording_icon_changed: Vec<Vec<(u64, Option<Kept>)>>,
}

/// Advance one track's icon folds using its engaged histories. An unfolded
/// hour loses every icon. For the recording player, naval recorded bytes settle
/// an undecided flag and supply the icon frame, which is then retained.
/// The byte check continues to compare against the derivation.
fn advance_icons(
    icons: &mut [IconFold],
    changed: &mut [Vec<(u64, Option<Kept>)>],
    folded: Option<(&DecodedFrame, &[engaged::History])>,
    bytes: Option<&BTreeMap<u64, [u8; 2]>>,
    continuity: Continuity,
) {
    for (kind, (icons, changed)) in icons.iter_mut().zip(changed).enumerate() {
        let Some((frame, histories)) = folded else {
            icons.lose(changed);
            continue;
        };
        let history = &histories[icon_source(kind)];
        let bytes = bytes.filter(|_| kind == NAVAL_ICON);
        let flag = |combat: u64| {
            let flag = history.combat(combat);
            bytes.map_or(flag, |bytes| naval_check::settle(flag, bytes.get(&combat)))
        };
        // A frame decoded, so an icon table it cannot read is one the hour
        // holds nothing of: its icons are lost.
        match icon_hour(kind, frame, &flag) {
            Ok(hour) => icons.advance(&hour, continuity, changed),
            Err(_) => icons.lose(changed),
        }
    }
}

/// How an hour follows the last one a track folded.
fn continuity(
    last: Option<(Hour, bool)>,
    hour: Hour,
    load: Option<Hour>,
    restart: bool,
) -> Continuity {
    match last {
        _ if load == Some(hour) => Continuity::Load,
        // A history goes on with play of another load or session, whose rebuild was unobserved.
        _ if restart => Continuity::Gap,
        // A branch that joined a running game, or whose first hours were not
        // folded, starts after unobserved rebuilds.
        None => Continuity::Gap,
        Some((last, true)) if last.checked_add(1) == Some(hour) => Continuity::Next,
        Some(_) => Continuity::Gap,
    }
}

/// A load rebuilds every side when its loaded hour is the first sampled hour.
/// If the collector loses the load origin, or sampling starts after that origin,
/// unobserved refreshes leave those sides unknown. The fresh-save comparison
/// uses the same load-hour test. Other starts join a running game.
fn native_load(start: &BranchStart) -> Option<Hour> {
    (start.reason == BranchReason::LoadSignal && start.loaded_hour == Some(start.first_hour))
        .then_some(start.first_hour)
}

struct Pass {
    shared: Arc<History>,
    country: Country,
    folds: BTreeMap<BranchId, Fold>,
    /// The byte check, folded here and published as a summary.
    check: ByteCheck,
}

/// Every table the pass decodes.
fn pass_tables() -> Vec<TableId> {
    let mut tables: Vec<TableId> = VIEWER_TABLES.to_vec();
    for source in SOURCES {
        tables.extend_from_slice(source.tables());
    }
    tables.extend_from_slice(&ICON_TABLES);
    tables.sort_by_key(|table| table.index());
    tables.dedup();
    tables
}

/// Folds every hour of `replay` on this thread and returns the naval byte
/// check, as the index's pass does in the background: the headless dump's
/// count.
pub(crate) fn check_recorded_bytes(replay: &Replay) -> Summary {
    let mut pass = Pass::new(Arc::new(History::default()), Country::Recorded);
    pass.fold(replay, &pass_tables());
    pass.check.summary.complete = true;
    mem::take(&mut pass.check.summary)
}

impl Pass {
    fn new(shared: Arc<History>, country: Country) -> Self {
        Self {
            shared,
            country,
            folds: BTreeMap::new(),
            check: ByteCheck::default(),
        }
    }

    fn run(mut self, mut replay: Arc<Replay>, inbox: Receiver<Arc<Replay>>) {
        let tables = pass_tables();
        let mut opened = true;
        loop {
            let started = Instant::now();
            self.check.summary.complete = false;
            self.publish();
            let hours = self.fold(&replay, &tables);
            if self.cancelled() {
                return;
            }
            self.check.summary.complete = true;
            self.publish();
            // The open cost is always logged; each extension only when profiling.
            if opened || profiling() {
                let (sides, points, bytes) = self.size();
                profile_print(format!(
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

    /// Copies the check's small summary out under a brief lock.
    fn publish(&self) {
        self.shared
            .check
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone_from(&self.check.summary);
    }

    /// Folds every hour past each branch's last folded one; returns how many.
    fn fold(&mut self, replay: &Replay, tables: &[TableId]) -> usize {
        let _cost = replay_frame::cpu::scope("viewer", Some("engagement/strikes/text"));
        let mut blocks: Vec<_> = replay.index.iter().collect();
        blocks.sort_by_key(|block| (block.branch, block.first_hour));
        // Each history's first hour of each recorded segment: a native load, or a restart.
        let mut starts: BTreeMap<(BranchId, Hour), bool> = BTreeMap::new();
        for history in &replay.branches {
            for (at, piece) in replay.pieces(history.id).iter().enumerate() {
                let load = replay
                    .segments_recorded()
                    .iter()
                    .find(|start| start.id == piece.segment)
                    .and_then(native_load)
                    .filter(|hour| (piece.first_hour..=piece.last_hour).contains(hour));
                match load {
                    Some(hour) => starts.insert((history.id, hour), true),
                    None if at > 0 => starts.insert((history.id, piece.first_hour), false),
                    None => None,
                };
            }
        }
        let mut hours = 0;
        for block in blocks {
            if self.cancelled() {
                return hours;
            }
            let folded = self
                .folds
                .get(&block.branch)
                .and_then(|fold| fold.last.map(|(hour, _)| hour));
            if folded.is_some_and(|last| block.last_hour <= last) {
                continue;
            }
            let mut stopped = false;
            let read = replay.read_block_alone(
                block,
                tables,
                |_, decoded| {
                    if stopped || self.cancelled() {
                        stopped = true;
                        return Ok(());
                    }
                    if folded.is_some_and(|last| decoded.hour <= last) {
                        return Ok(());
                    }
                    let campaign = core::<tables::Campaign>(decoded)
                        .is_ok_and(|rows| rows.len() == 1 && rows[0].id == replay.campaign_id);
                    let start = starts.get(&(block.branch, decoded.hour)).copied();
                    let load = (start == Some(true)).then_some(decoded.hour);
                    self.fold_hour(
                        block.branch,
                        decoded.hour,
                        campaign.then_some(decoded),
                        load,
                        start == Some(false),
                    );
                    hours += 1;
                    Ok(())
                },
                || Ok(()),
            );
            if stopped {
                return hours;
            }
            if read.is_err() {
                self.failed_block(block.branch, block.last_hour);
            }
        }
        hours
    }

    /// A block that could not be read: both tracks follow unobserved hours,
    /// and the check counts it.
    fn failed_block(&mut self, branch: BranchId, last_hour: Hour) {
        let fold = self.folds.entry(branch).or_default();
        fold.last = Some((last_hour, false));
        fold.recording_last = Some((last_hour, false));
        self.check.summary.failed_blocks += 1;
        self.publish();
    }

    /// Folds one hour for the perspective's viewer, with its icons, and for
    /// the recording's own player, and checks the latter's naval flags
    /// against the recorded bytes. An undecodable hour, or one whose sides
    /// cannot be read, leaves every side unknown until it rebuilds and loses
    /// every icon. `load` is the hour the branch starts with a native load.
    ///
    /// The fold runs without the locks; each is held only to append or copy
    /// the hour's results.
    fn fold_hour(
        &mut self,
        branch: BranchId,
        hour: Hour,
        frame: Option<&DecodedFrame>,
        load: Option<Hour>,
        restart: bool,
    ) {
        let fold = self.folds.entry(branch).or_default();
        fold.icons.resize_with(ICON_KINDS, Default::default);
        fold.icon_changed.resize_with(ICON_KINDS, Vec::new);
        fold.recording_icons
            .resize_with(ICON_KINDS, Default::default);
        fold.recording_icon_changed
            .resize_with(ICON_KINDS, Vec::new);
        let perspective_continuity = continuity(fold.last, hour, load, restart);
        let recording_continuity = continuity(fold.recording_last, hour, load, restart);
        let summary = &mut self.check.summary;
        summary.hours += 1;
        // The sides do not depend on who views them.
        let sides = frame.zip(frame.and_then(|frame| {
            SOURCES
                .iter()
                .map(|source| source.sides(frame))
                .collect::<io::Result<Vec<_>>>()
                .ok()
        }));
        let viewer = sides
            .as_ref()
            .and_then(|(frame, _)| Viewer::read(frame, &self.country).ok());
        // The client's stored viewpoint comes from LocalPlayer. A change rebuilds
        // all sides for the new player only; its history records that refresh.
        // The perspective history does not rebuild for it.
        let player = sides
            .as_ref()
            .and_then(|(frame, _)| Viewer::recording(frame).ok().flatten());
        // Evaluate the view for which the client's flags were last rebuilt.
        // A tag switch or controller assigned the viewed country rebuilds at once.
        // An observer-view change or a controller assignment instead waits for the
        // next hourly check; evaluate that sample for the previous view, then move
        // to the new view in the next sample.
        //
        // Not recovered from these samples: a controller assigned the newly viewed
        // country in the same interval, or a side rebuilding after a view change in
        // the same command batch. Those refreshes can test the new view while this
        // fold still evaluates the previous one.
        let previous_view = fold
            .client_view
            .filter(|_| recording_continuity == Continuity::Next);
        // Each is (the evaluated viewer, the client's view).
        let player = player.map(|(mut viewer, observing)| {
            let view = viewer.slot;
            if let Some(slot) = previous_view
                && observing
            {
                viewer.slot = slot;
            }
            (viewer, view)
        });
        match &sides {
            None => summary.unreadable_hours += 1,
            Some((_, sides)) => {
                let naval = &sides[NAVAL];
                if !naval.complete {
                    summary.incomplete_hours += 1;
                }
                if !naval.sides.is_empty() {
                    summary.naval_hours += 1;
                    if player.is_none() {
                        summary.unidentified_hours += 1;
                    }
                }
            }
        }
        match (&sides, &player) {
            (Some((frame, sides)), Some((viewer, view))) => {
                fold.recording
                    .advance(viewer, sides.clone(), recording_continuity);
                fold.recording_last = Some((hour, true));
                fold.client_view = Some(*view);
                // An unreadable detail table settles nothing.
                let bytes = naval_check::recorded_bytes(frame).ok();
                advance_icons(
                    &mut fold.recording_icons,
                    &mut fold.recording_icon_changed,
                    Some((frame, &fold.recording.histories)),
                    bytes.as_ref(),
                    recording_continuity,
                );
                // The check reads the recording player's history after the hour.
                if self
                    .check
                    .compare(
                        branch,
                        hour,
                        frame,
                        &sides[NAVAL].sides,
                        &fold.recording.histories[NAVAL],
                    )
                    .is_err()
                {
                    self.check.summary.unreadable_hours += 1;
                }
            }
            _ => {
                fold.recording.forget();
                fold.recording_last = Some((hour, false));
                fold.client_view = None;
                advance_icons(
                    &mut fold.recording_icons,
                    &mut fold.recording_icon_changed,
                    None,
                    None,
                    recording_continuity,
                );
            }
        }
        let slot = viewer.as_ref().map(|viewer| viewer.slot);
        match (sides, viewer) {
            (Some((frame, sides)), Some(viewer)) => {
                fold.perspective
                    .advance(&viewer, sides, perspective_continuity);
                fold.last = Some((hour, true));
                advance_icons(
                    &mut fold.icons,
                    &mut fold.icon_changed,
                    Some((frame, &fold.perspective.histories)),
                    None,
                    perspective_continuity,
                );
            }
            _ => {
                fold.perspective.forget();
                fold.last = Some((hour, false));
                advance_icons(
                    &mut fold.icons,
                    &mut fold.icon_changed,
                    None,
                    None,
                    perspective_continuity,
                );
            }
        }
        {
            let mut branches = self
                .shared
                .branches
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            let covered = branches.entry(branch).or_insert_with(|| Covered::new(hour));
            covered.last = hour;
            if let Some(slot) = slot
                && covered.viewers.last().is_none_or(|(_, last)| *last != slot)
            {
                covered.viewers.push((hour, slot));
            }
            // A lookup for the client's view is the client's flag, stale or not.
            let player_slot = player.as_ref().map(|(_, view)| *view);
            if covered
                .players
                .last()
                .is_none_or(|(_, last)| *last != player_slot)
            {
                covered.players.push((hour, player_slot));
            }
            fold.perspective.record(&mut covered.points, hour);
            fold.recording.record(&mut covered.recording, hour);
            for (icons, changed) in [
                (&mut covered.icons, &fold.icon_changed),
                (&mut covered.recording_icons, &fold.recording_icon_changed),
            ] {
                for (points, changed) in icons.iter_mut().zip(changed) {
                    for &(combat, kept) in changed {
                        points.entry(combat).or_default().push((hour, kept));
                    }
                }
            }
        }
        self.publish();
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
            bytes += covered.viewers.capacity() * size_of::<(Hour, u32)>()
                + covered.players.capacity() * size_of::<(Hour, Option<u32>)>();
            for kind in covered.points.iter().chain(&covered.recording) {
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
mod tests;
