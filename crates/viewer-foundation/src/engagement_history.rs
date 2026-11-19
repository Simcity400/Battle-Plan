//! Shared engagement and retained-icon history for combat windows and counter bindings.
//! The host owns decoding, scheduling and cancellation of the recording pass.

use crate::{
    engaged::{self, Continuity, Engaged, Sides},
    engagement_progress::LandSides,
    icon_frames::{self, IconFold, IconHour, Kept},
    map::{tables::rows, visibility::Relations},
    naval_check::{self, Summary},
    naval_icon_frames::naval_icon_hour,
    naval_sides::NavalSides,
};
use replay_data::{BranchId, Hour, rows::NavalProvinceCombatRow, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::BTreeMap,
    io,
    sync::{Condvar, Mutex, atomic::AtomicBool, atomic::Ordering},
    time::{Duration, Instant},
};

pub const LAND: usize = 0;
pub const NAVAL: usize = 1;
/// Indexed by kind.
pub static SOURCES: [&dyn Sides; 2] = [&LandSides, &NavalSides];

/// Combat icon kinds whose unseen frame the index carries
/// ([`icon_frames`]).
pub const LAND_ICON: usize = 0;
pub const NAVAL_ICON: usize = 1;
pub const ICON_KINDS: usize = 2;

/// The engaged source whose flag an icon kind's seen test reads.
pub fn icon_source(kind: usize) -> usize {
    if kind == LAND_ICON { LAND } else { NAVAL }
}

/// One icon kind's hour. Both use their own kind's engaged flag as the seen
/// test. Naval frames update only when either side is engaged
/// ([`naval_icon_hour`]).
pub fn icon_hour<F: replay_frame::frame::Tables + ?Sized>(
    kind: usize,
    frame: &F,
    flag: &dyn Fn(u64) -> Engaged,
) -> io::Result<IconHour> {
    if kind == LAND_ICON {
        icon_frames::land_hour(frame, flag)
    } else {
        naval_icon_hour(frame, flag)
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
/// [`Kept::Unindexed`] for every unseen one. `flags` are the hour's engaged
/// flags of the icon's own kind ([`combats`]).
pub fn kept_frames(
    frame: &DecodedFrame,
    kind: usize,
    viewer: u32,
    flags: &BTreeMap<u64, Engaged>,
    index: Option<&History>,
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
    let hour = icon_hour(kind, frame, &flag)?;
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
pub type IconPoints = BTreeMap<u64, Vec<(Hour, Option<Kept>)>>;

/// Where an hour's flags came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provenance {
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
    pub fn limitation(self) -> Option<&'static str> {
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
pub fn combats(
    frame: &DecodedFrame,
    kind: usize,
    viewer: u32,
    links: &BTreeMap<u32, u32>,
    relations: Result<&Relations, &'static str>,
    index: Option<&History>,
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

/// The hour's flag of every land and naval combat for one viewer, with where
/// each kind's came from.
#[derive(Clone, Debug)]
pub struct CombatFlags {
    pub land: BTreeMap<u64, Engaged>,
    pub naval: BTreeMap<u64, Engaged>,
    pub land_provenance: Provenance,
    pub naval_provenance: Provenance,
}

impl Default for CombatFlags {
    fn default() -> Self {
        Self {
            land: BTreeMap::new(),
            naval: BTreeMap::new(),
            land_provenance: Provenance::Unindexed,
            naval_provenance: Provenance::Unindexed,
        }
    }
}

impl CombatFlags {
    /// Both kinds' [`combats`]; an undecided naval flag of the recording's
    /// own player takes its recorded byte ([`naval_check::settle_recorded`]).
    pub fn read(
        frame: &DecodedFrame,
        viewer: u32,
        links: &BTreeMap<u32, u32>,
        relations: Result<&Relations, &'static str>,
        index: Option<&History>,
    ) -> io::Result<Self> {
        let (mut naval, naval_provenance) = combats(frame, NAVAL, viewer, links, relations, index)?;
        naval_check::settle_recorded(frame, links, viewer, &mut naval)?;
        let (land, land_provenance) = combats(frame, LAND, viewer, links, relations, index)?;
        Ok(Self {
            land,
            naval,
            land_provenance,
            naval_provenance,
        })
    }

    /// The engaged state of one province combat entry, by its recorded class;
    /// a combat the hour does not
    /// hold is unknown.
    pub fn entry(&self, combat: u64, kind: u8) -> Engaged {
        let flags = if kind == NavalProvinceCombatRow::NAVAL {
            &self.naval
        } else {
            &self.land
        };
        flags.get(&combat).copied().unwrap_or(Engaged::Unknown)
    }

    /// Whether at least `count` entries of a province's combat vector are
    /// engaged, walking it in order and stopping at the threshold. Progress
    /// size needs two engaged entries; the battle picture needs one. `row`
    /// is the province's row, `complete` whether an absent row is an empty
    /// vector; `None` when an undecided flag or an unknown vector could
    /// change the answer.
    pub fn engaged_entries(
        &self,
        row: Option<&NavalProvinceCombatRow>,
        complete: bool,
        count: usize,
    ) -> Option<bool> {
        let Some(row) = row else {
            return complete.then_some(count == 0);
        };
        let (mut engaged, mut undecided) = (0, 0);
        for (&combat, &kind) in row.combats.iter().zip(&row.kinds) {
            if engaged >= count {
                break;
            }
            match self.entry(combat, kind) {
                Engaged::Yes => engaged += 1,
                Engaged::No => {}
                Engaged::Ambiguous(_) | Engaged::Unknown => undecided += 1,
            }
        }
        if engaged >= count {
            Some(true)
        } else {
            (engaged + undecided < count).then_some(false)
        }
    }
}

/// Per combat side: the hours its flag took a new value, ascending.
pub type Changes = BTreeMap<(u64, u8), Vec<(Hour, Engaged)>>;

/// The combats a fold changed at each hour it covers: the hours ascending, each
/// naming where its combats start in one run of ids. A reader advancing from
/// one hour to another reads the combats between them and leaves the rest.
#[derive(Debug, Default)]
pub struct Touched {
    hours: Vec<(Hour, u32)>,
    ids: Vec<u64>,
}

impl Touched {
    /// The combats `ids` changed at `hour`, later than every hour recorded
    /// before it; an hour that changed none takes no entry.
    fn record(&mut self, hour: Hour, ids: impl IntoIterator<Item = u64>) {
        let at = u32::try_from(self.ids.len()).expect("the engaged index's hour log holds under four billion changes");
        self.ids.extend(ids);
        if self.ids.len() > at as usize {
            self.hours.push((hour, at));
        }
    }

    /// The combats whose value changed after `from` and at or before `to`, each
    /// once, ascending.
    fn between(&self, from: Hour, to: Hour) -> Vec<u64> {
        let bound = |hour| self.hours.get(self.hours.partition_point(|&(held, _)| held <= hour)).map_or(self.ids.len(), |&(_, at)| at as usize);
        let mut ids = self.ids[bound(from)..bound(to)].to_vec();
        ids.sort_unstable();
        ids.dedup();
        ids
    }
}

/// One viewer's folded history of a branch: per kind the flags' change points,
/// per icon kind the kept frames', and for both the combats each hour changed,
/// appended together so an advance over hours can never miss one.
#[derive(Debug, Default)]
pub struct Folded {
    points: Vec<Changes>,
    icons: Vec<IconPoints>,
    touched: Vec<Touched>,
    icon_touched: Vec<Touched>,
}

impl Folded {
    fn new() -> Self {
        Self {
            points: vec![BTreeMap::new(); SOURCES.len()],
            icons: vec![BTreeMap::new(); ICON_KINDS],
            touched: (0..SOURCES.len()).map(|_| Touched::default()).collect(),
            icon_touched: (0..ICON_KINDS).map(|_| Touched::default()).collect(),
        }
    }

    /// Per kind and side: the hours its flag took a new value, ascending.
    pub fn points(&self) -> &[Changes] {
        &self.points
    }

    /// Per icon kind and combat: the hours its kept frame changed.
    pub fn icons(&self) -> &[IconPoints] {
        &self.icons
    }

    /// `changed` sides of `kind` took new values at `hour`, and the combats of
    /// `ended` are over: the change points and the hour's combats together.
    pub fn record(&mut self, kind: usize, hour: Hour, changed: &[((u64, u8), Engaged)], ended: &[(u64, u8)]) {
        let points = &mut self.points[kind];
        let mut touched: Vec<u64> = Vec::with_capacity(changed.len());
        for &(key, value) in changed {
            let changes = points.entry(key).or_default();
            if changes.last().is_none_or(|(_, last)| *last != value) {
                changes.push((hour, value));
                touched.push(key.0);
            }
        }
        self.touched[kind].record(hour, touched);
        // An ended combat takes no more points: keep only what it holds.
        for key in ended {
            if let Some(changes) = points.get_mut(key) {
                changes.shrink_to_fit();
            }
        }
    }

    /// `changed` icons of `kind` took new kept frames at `hour`.
    pub fn record_icons(&mut self, kind: usize, hour: Hour, changed: &[(u64, Option<Kept>)]) {
        for &(combat, kept) in changed {
            self.icons[kind].entry(combat).or_default().push((hour, kept));
        }
        self.icon_touched[kind].record(hour, changed.iter().map(|&(combat, _)| combat));
    }
}

/// The folded change points of one branch.
#[derive(Default)]
pub struct Covered {
    pub first: Hour,
    pub last: Hour,
    /// The perspective's viewer's history.
    pub perspective: Folded,
    /// The same for the recording's own player, whose history follows its
    /// player changes and whose icons take their frames from its own flags.
    pub recording: Folded,
    /// The viewer per hour where it changed, ascending; a lookup for another
    /// viewer is not covered.
    pub viewers: Vec<(Hour, u32)>,
    /// The recording's player per hour where it changed, ascending; `None`
    /// where the recording does not name one.
    pub players: Vec<(Hour, Option<u32>)>,
}

/// The value in effect at `hour` of an ascending change list.
pub fn at<T: Copy>(changes: &[(Hour, T)], hour: Hour) -> Option<T> {
    let at = changes.partition_point(|(from, _)| *from <= hour);
    Some(changes.get(at.checked_sub(1)?)?.1)
}

impl Covered {
    pub fn new(hour: Hour) -> Self {
        Self { first: hour, last: hour, perspective: Folded::new(), recording: Folded::new(), viewers: Vec::new(), players: Vec::new() }
    }

    /// The hour's changes of one track: the recording player's when `recording`.
    pub fn fold(&mut self, recording: bool) -> &mut Folded {
        if recording { &mut self.recording } else { &mut self.perspective }
    }

    /// Both tracks, the perspective's first.
    pub fn folds(&self) -> [&Folded; 2] {
        [&self.perspective, &self.recording]
    }

    fn viewer(&self, hour: Hour) -> Option<u32> {
        at(&self.viewers, hour)
    }

    /// Whether the viewer the pass resolved, or the recording's player, changed
    /// after `from` and at or before `to`: the values held at `from` then came
    /// from another track and none of them carries over.
    fn one_track(&self, from: Hour, to: Hour) -> bool {
        let steady = |mut hours: Box<dyn Iterator<Item = Hour> + '_>| !hours.any(|hour| hour > from && hour <= to);
        steady(Box::new(self.viewers.iter().map(|&(hour, _)| hour))) && steady(Box::new(self.players.iter().map(|&(hour, _)| hour)))
    }

    /// The perspective's viewer at `hour`, or why a lookup for `viewer`
    /// cannot use the perspective's history.
    fn answers_viewer(&self, hour: Hour, viewer: u32) -> Result<(), Provenance> {
        match self.viewer(hour) {
            Some(folded) if folded == viewer => Ok(()),
            Some(_) => Err(Provenance::OtherViewer),
            None => Err(Provenance::Unread),
        }
    }

    /// The change points that answer `viewer` at `hour`: the recording
    /// player's own history when `viewer` is that player, which is the
    /// client's flag itself, else the perspective's when it resolved to
    /// `viewer`.
    fn points_for(&self, hour: Hour, viewer: u32) -> Result<&Folded, Provenance> {
        if self.recording_player(hour, viewer) {
            return Ok(&self.recording);
        }
        self.answers_viewer(hour, viewer)?;
        Ok(&self.perspective)
    }

    /// Whether `viewer` is the recording's own player at `hour`, whose history
    /// is the client's flag itself.
    pub fn recording_player(&self, hour: Hour, viewer: u32) -> bool {
        at(&self.players, hour).flatten() == Some(viewer)
    }
}

/// Where `hour`'s flags for `viewer` come from, over the folds published so far.
fn provenance_in(
    branches: &BTreeMap<BranchId, Covered>,
    branch: BranchId,
    hour: Hour,
    viewer: u32,
) -> Provenance {
    let Some(covered) = branches
        .get(&branch)
        .filter(|covered| (covered.first..=covered.last).contains(&hour))
    else {
        return Provenance::Pending;
    };
    match covered.points_for(hour, viewer) {
        Ok(_) => Provenance::Indexed,
        Err(provenance) => provenance,
    }
}

#[derive(Default)]
pub struct History {
    pub branches: Mutex<BTreeMap<BranchId, Covered>>,
    /// Signalled after each hour the fold publishes under `branches`: what a reader that must see
    /// a given hour waits on ([`History::reached`]).
    folded: Condvar,
    /// The byte check's published summary; the pass folds it privately.
    pub check: Mutex<Summary>,
    pub cancelled: AtomicBool,
}

impl History {
    /// The recorded naval engaged bytes checked against the recording
    /// player's derivation over the hours folded so far: a small summary.
    pub fn byte_check(&self) -> Summary {
        self.check.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Whether the pass has folded this hour.
    pub fn covers(&self, branch: BranchId, hour: Hour) -> bool {
        let branches = self.branches.lock().unwrap_or_else(|p| p.into_inner());
        branches
            .get(&branch)
            .is_some_and(|covered| (covered.first..=covered.last).contains(&hour))
    }

    /// Whether [`combats`] answers this hour from the index for `viewer`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn answers(&self, frame: &DecodedFrame, viewer: u32) -> bool {
        SOURCES.iter().enumerate().all(|(kind, source)| {
            source.combats(frame).is_ok_and(|combats| {
                self.lookup(kind, frame.branch, frame.hour, viewer, &combats)
                    .is_ok()
            })
        })
    }

    /// The flag of each combat at a covered hour, for `viewer`: the recording
    /// player's history when `viewer` is that player there, else the
    /// perspective's when the pass resolved `viewer`; otherwise why the index
    /// cannot answer.
    pub fn lookup(
        &self,
        kind: usize,
        branch: BranchId,
        hour: Hour,
        viewer: u32,
        combats: &[u64],
    ) -> Result<BTreeMap<u64, Engaged>, Provenance> {
        let branches = self.branches.lock().unwrap_or_else(|p| p.into_inner());
        let covered = branches
            .get(&branch)
            .filter(|covered| (covered.first..=covered.last).contains(&hour))
            .ok_or(Provenance::Pending)?;
        // Every kind in SOURCES has its points from the first folded hour.
        let points = &covered.points_for(hour, viewer)?.points()[kind];
        let side = |combat: u64, side: u8| {
            points
                .get(&(combat, side))
                .and_then(|points| at(points, hour))
                .unwrap_or(Engaged::Unknown)
        };
        Ok(combats
            .iter()
            .map(|&combat| (combat, side(combat, 0).either(side(combat, 1))))
            .collect())
    }

    /// What each live icon of `kind` keeps at a covered hour, for the viewer
    /// the pass resolved there; otherwise why the index cannot answer.
    pub fn lookup_icons(
        &self,
        kind: usize,
        branch: BranchId,
        hour: Hour,
        viewer: u32,
        combats: &[u64],
    ) -> Result<BTreeMap<u64, Kept>, Provenance> {
        let branches = self.branches.lock().unwrap_or_else(|p| p.into_inner());
        let covered = branches
            .get(&branch)
            .filter(|covered| (covered.first..=covered.last).contains(&hour))
            .ok_or(Provenance::Pending)?;
        // The client's icons follow its own flags: the recording player's icon
        // history answers a viewer who is that player there, as `points_for`.
        let points = if covered.recording_player(hour, viewer) {
            &covered.recording.icons()[kind]
        } else {
            covered.answers_viewer(hour, viewer)?;
            &covered.perspective.icons()[kind]
        };
        Ok(combats
            .iter()
            .filter_map(|&combat| {
                let points = points.get(&combat)?;
                let at = points.partition_point(|(from, _)| *from <= hour);
                Some((combat, points.get(at.checked_sub(1)?)?.1?))
            })
            .collect())
    }

    /// Where a lookup at `hour` for `viewer` takes its values from, read
    /// without naming a combat: what [`Self::lookup`] and
    /// [`Self::lookup_icons`] report there.
    pub fn provenance(&self, branch: BranchId, hour: Hour, viewer: u32) -> Provenance {
        let branches = self.branches.lock().unwrap_or_else(|p| p.into_inner());
        provenance_in(&branches, branch, hour, viewer)
    }

    /// The fold has published an hour: woken for every [`Self::reached`].
    pub fn folded(&self) {
        self.folded.notify_all();
    }

    /// [`Self::provenance`] once the fold has reached `hour`, waited for within `limit`: what a
    /// frame that is saved asks, so its combat markers are the markers of its hour and not of
    /// however far the background fold had got when it was drawn. Every provenance but
    /// [`Provenance::Pending`] is a decision time cannot change and is answered at once; `Pending`
    /// is answered when the fold is cancelled or `limit` runs out.
    pub fn reached(&self, branch: BranchId, hour: Hour, viewer: u32, limit: Duration) -> Provenance {
        let until = Instant::now() + limit;
        let mut branches = self.branches.lock().unwrap_or_else(|p| p.into_inner());
        loop {
            let provenance = provenance_in(&branches, branch, hour, viewer);
            if provenance != Provenance::Pending || self.cancelled.load(Ordering::Acquire) {
                return provenance;
            }
            let Some(left) = until
                .checked_duration_since(Instant::now())
                .filter(|left| !left.is_zero())
            else {
                return Provenance::Pending;
            };
            branches = self
                .folded
                .wait_timeout(branches, left)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
    }

    /// The combats of `kind` whose flag changed after `from` and at or before
    /// `to` for `viewer`; `None` when the two hours are not answered from one
    /// folded track, so every combat's flag is read again.
    pub fn flags_changed(&self, kind: usize, branch: BranchId, from: Hour, to: Hour, viewer: u32) -> Option<Vec<u64>> {
        self.advance(branch, from, to, viewer, |fold| &fold.touched[kind])
    }

    /// The same for the kept frames of icon kind `kind`
    /// ([`Self::lookup_icons`]).
    pub fn icons_changed(&self, kind: usize, branch: BranchId, from: Hour, to: Hour, viewer: u32) -> Option<Vec<u64>> {
        self.advance(branch, from, to, viewer, |fold| &fold.icon_touched[kind])
    }

    fn advance(&self, branch: BranchId, from: Hour, to: Hour, viewer: u32, log: impl Fn(&Folded) -> &Touched) -> Option<Vec<u64>> {
        let branches = self.branches.lock().unwrap_or_else(|p| p.into_inner());
        let covered = branches.get(&branch)?;
        if from > to || from < covered.first || to > covered.last || !covered.one_track(from, to) {
            return None;
        }
        Some(log(covered.points_for(to, viewer).ok()?).between(from, to))
    }
}
