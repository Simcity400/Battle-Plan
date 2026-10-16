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
use replay::{
    BranchId, Hour,
    frame::DecodedFrame,
    rows::{NavalProvinceCombatRow, tables},
};
use std::{
    collections::BTreeMap,
    io,
    sync::{Mutex, atomic::AtomicBool},
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
pub fn icon_hour(
    kind: usize,
    frame: &DecodedFrame,
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

/// The folded change points of one branch.
#[derive(Default)]
pub struct Covered {
    pub first: Hour,
    pub last: Hour,
    /// Per kind and side: the hours its flag took a new value, ascending, for
    /// the perspective's viewer.
    pub points: Vec<Changes>,
    /// The same for the recording's own player, whose history follows its
    /// player changes.
    pub recording: Vec<Changes>,
    /// Per icon kind and combat: the hours its kept frame changed, for the
    /// perspective's viewer.
    pub icons: Vec<IconPoints>,
    /// The same for the recording's own player, whose icons take their frames
    /// from its own flag history.
    pub recording_icons: Vec<IconPoints>,
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
        Self {
            first: hour,
            last: hour,
            points: vec![BTreeMap::new(); SOURCES.len()],
            recording: vec![BTreeMap::new(); SOURCES.len()],
            icons: vec![BTreeMap::new(); ICON_KINDS],
            recording_icons: vec![BTreeMap::new(); ICON_KINDS],
            viewers: Vec::new(),
            players: Vec::new(),
        }
    }

    fn viewer(&self, hour: Hour) -> Option<u32> {
        at(&self.viewers, hour)
    }

    /// The perspective's viewer at `hour`, or why a lookup for `viewer`
    /// cannot use the perspective's history.
    fn perspective(&self, hour: Hour, viewer: u32) -> Result<(), Provenance> {
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
    fn points_for(&self, hour: Hour, viewer: u32) -> Result<&[Changes], Provenance> {
        if at(&self.players, hour).flatten() == Some(viewer) {
            return Ok(&self.recording);
        }
        self.perspective(hour, viewer)?;
        Ok(&self.points)
    }
}

#[derive(Default)]
pub struct History {
    pub branches: Mutex<BTreeMap<BranchId, Covered>>,
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
        let points = &covered.points_for(hour, viewer)?[kind];
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
        let points = if at(&covered.players, hour).flatten() == Some(viewer) {
            &covered.recording_icons[kind]
        } else {
            covered.perspective(hour, viewer)?;
            &covered.icons[kind]
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
}
