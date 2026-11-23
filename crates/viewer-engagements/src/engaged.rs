
use viewer_foundation::visibility::{Relations, ZeroVisibility};
use replay_data::rows::TableId;
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io};

#[derive(Clone)]
pub struct HourSides {
    pub sides: BTreeMap<(u64, u8), Side>,
    pub complete: bool,
    pub removal: Removal,
}

pub trait Sides: Sync {
    fn tables(&self) -> &'static [TableId];
    fn combats(&self, frame: &DecodedFrame) -> io::Result<Vec<u64>>;
    fn sides(&self, frame: &DecodedFrame) -> io::Result<HourSides>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engaged {
    Yes,
    No,
    Ambiguous(Ambiguity),
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Ambiguity {
    Scope,
    Order,
}

impl Ambiguity {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Scope => {
                "engaged flag ambiguous (the recording cannot prove whether a refresh ran: engagement refresh witnesses were unavailable or not recorded, an other-side removal may be one-sided or unrecorded, or a controller change target could not be resolved)"
            }
            Self::Order => {
                "engaged flag ambiguous (the recording cannot order a refresh against a membership or relation change between two samples)"
            }
        }
    }
}

impl Engaged {
    fn of(value: Option<bool>) -> Self {
        match value {
            Some(true) => Self::Yes,
            Some(false) => Self::No,
            None => Self::Unknown,
        }
    }

    pub fn either(self, other: Self) -> Self {
        use Engaged::*;
        match (self, other) {
            (Yes, _) | (_, Yes) => Yes,
            (Unknown, _) | (_, Unknown) => Unknown,
            (Ambiguous(a), Ambiguous(b)) => Ambiguous(a.min(b)),
            (Ambiguous(a), No) | (No, Ambiguous(a)) => Ambiguous(a),
            (No, No) => No,
        }
    }

    fn agreed(evaluated: &[Self], held: Option<Self>) -> Self {
        if evaluated.contains(&Self::Unknown) || held == Some(Self::Unknown) {
            return Self::Unknown;
        }
        let Some(&first) = evaluated.first() else {
            return held.unwrap_or(Self::Unknown);
        };
        if evaluated.iter().any(|value| *value != first) {
            return Self::Ambiguous(Ambiguity::Order);
        }
        match held {
            Some(held) if held != first => Self::Ambiguous(Ambiguity::Scope),
            _ => first,
        }
    }
}

#[cfg(test)]
pub(crate) fn test(
    primary: &[u32],
    secondary: &[u32],
    resolve: impl Fn(u32) -> Option<(bool, bool)>,
) -> Option<bool> {
    let mut unknown = false;
    for (lists, pick) in [(primary, 1), (secondary, 0)] {
        for raw in lists {
            match resolve(*raw) {
                Some(answers) if [answers.0, answers.1][pick] => return Some(true),
                Some(_) => {}
                None => unknown = true,
            }
        }
    }
    (!unknown).then_some(false)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Removal {
    BothSidesUnlessRoundRetreat,
    BothSides,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lists {
    pub recorded: bool,
    pub units: bool,
}

impl Lists {
    pub fn any(self) -> bool {
        self.recorded || self.units
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SideWitness {
    pub lists: Lists,
    pub recorded_viewers: Vec<u32>,
    pub units_viewers: Vec<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum RefreshWitness {
    #[default]
    Unavailable,
    Complete {
        sides: BTreeMap<(u64, u8), SideWitness>,
        controller_targets: Vec<u32>,
        local_viewers: Vec<u32>,
        cache_written_viewers: Vec<u32>,
    },
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refresh {
    Load,
    Hour { proven: Lists, possible: Lists },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tested {
    Recorded,
    Units,
}

#[cfg(test)]
pub(crate) fn step(
    held: Engaged,
    refresh: Option<Refresh>,
    mut evaluate: impl FnMut(Tested, usize) -> Option<bool>,
) -> Engaged {
    match refresh {
        None => Engaged::Unknown,
        Some(Refresh::Load) => Engaged::of(evaluate(Tested::Recorded, 0)),
        Some(Refresh::Hour { proven, possible }) => {
            if !proven.any() && !possible.any() {
                return held;
            }
            let mut evaluated = [Engaged::Unknown; 4];
            let mut len = 0;
            for (tested, lists) in [
                (proven.recorded || possible.recorded, Tested::Recorded),
                (proven.units || possible.units, Tested::Units),
            ] {
                if tested {
                    for when in 0..2 {
                        evaluated[len] = Engaged::of(evaluate(lists, when));
                        len += 1;
                    }
                }
            }
            Engaged::agreed(&evaluated[..len], (!proven.any()).then_some(held))
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Added {
    #[default]
    NoTrigger,
    Unavailable,
    Observed(Vec<u64>),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Side {
    pub units: Option<Vec<u64>>,
    pub added: Added,
    pub lists: Option<(Vec<u32>, Vec<u32>)>,
    pub unit_lists: Option<(Vec<u32>, Vec<u32>)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Controllers {
    pub players: BTreeMap<i32, (u32, Option<u32>)>,
    pub counts: BTreeMap<u32, i64>,
}

impl Controllers {
    fn may_have_joined(&self, was: &Self, viewer: u32) -> bool {
        let count = |rosters: &Self| rosters.counts.get(&viewer).copied().unwrap_or(0);
        count(self) > count(was)
            || self.players.iter().any(|(key, now)| {
                now.1.is_none_or(|slot| slot == viewer) && was.players.get(key) != Some(now)
            })
    }
}

pub struct Sample<'a> {
    pub viewer: Option<u32>,
    pub viewer_raw: Option<u32>,
    pub alternate_viewer: Option<u32>,
    pub alternate_viewer_raw: Option<u32>,
    pub recording: bool,
    pub links: &'a BTreeMap<u32, u32>,
    pub relations: Result<&'a Relations, &'static str>,
    pub zero_visibility: &'a ZeroVisibility,
    pub controllers: Option<Controllers>,
    pub refreshes: RefreshWitness,
    pub sides: BTreeMap<(u64, u8), Side>,
    pub complete: bool,
    pub removal: Removal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ViewerRelation {
    Same,
    Different,
    Unknown,
}

fn raw_relation_in(links: &BTreeMap<u32, u32>, a: u32, b: u32) -> ViewerRelation {
    if a == b {
        return ViewerRelation::Same;
    }
    if a == 0 || b == 0 {
        return ViewerRelation::Different;
    }
    match (links.get(&a), links.get(&b)) {
        (Some(sa), Some(sb)) if sa == sb => ViewerRelation::Same,
        (Some(_), Some(_)) => ViewerRelation::Different,
        _ => ViewerRelation::Unknown,
    }
}

fn candidate_relation(
    before_links: &BTreeMap<u32, u32>,
    before: CandidateViewer,
    now_links: &BTreeMap<u32, u32>,
    now: CandidateViewer,
) -> ViewerRelation {
    match (before, now) {
        (CandidateViewer::RawLocal(a), CandidateViewer::RawLocal(b)) => match (
            raw_relation_in(before_links, a, b),
            raw_relation_in(now_links, a, b),
        ) {
            (ViewerRelation::Same, ViewerRelation::Same) => ViewerRelation::Same,
            (ViewerRelation::Different, ViewerRelation::Different) => ViewerRelation::Different,
            _ => ViewerRelation::Unknown,
        },
        (CandidateViewer::Slot(a), CandidateViewer::Slot(b)) => {
            if a == b {
                ViewerRelation::Same
            } else {
                ViewerRelation::Different
            }
        }
        (CandidateViewer::RawLocal(raw), CandidateViewer::Slot(slot))
        | (CandidateViewer::Slot(slot), CandidateViewer::RawLocal(raw)) => {
            if raw == 0 {
                return ViewerRelation::Different;
            }
            match (
                before_links.get(&raw).copied(),
                now_links.get(&raw).copied(),
            ) {
                (Some(a), Some(b)) if a == slot && b == slot => ViewerRelation::Same,
                (Some(a), Some(b)) if a != slot && b != slot => ViewerRelation::Different,
                _ => ViewerRelation::Unknown,
            }
        }
    }
}

impl Sample<'_> {
    fn primary_viewer(&self) -> Option<CandidateViewer> {
        if let Some(raw) = self.viewer_raw {
            return Some(CandidateViewer::RawLocal(raw));
        }
        self.viewer.map(CandidateViewer::Slot)
    }

    fn secondary_viewer(
        &self,
        before_links: Option<&BTreeMap<u32, u32>>,
    ) -> Option<CandidateViewer> {
        let secondary = match (self.alternate_viewer_raw, self.alternate_viewer) {
            (Some(raw), _) => CandidateViewer::RawLocal(raw),
            (None, Some(slot)) => CandidateViewer::Slot(slot),
            (None, None) => return None,
        };
        if let Some(primary) = self.primary_viewer() {
            let relation = match (primary, secondary) {
                (CandidateViewer::RawLocal(a), CandidateViewer::RawLocal(b)) => match before_links {
                    Some(before) => match (
                        raw_relation_in(before, a, b),
                        raw_relation_in(self.links, a, b),
                    ) {
                        (ViewerRelation::Same, ViewerRelation::Same) => ViewerRelation::Same,
                        (ViewerRelation::Different, ViewerRelation::Different) => {
                            ViewerRelation::Different
                        }
                        _ => ViewerRelation::Unknown,
                    },
                    None => raw_relation_in(self.links, a, b),
                },
                _ => candidate_relation(
                    before_links.unwrap_or(self.links),
                    primary,
                    self.links,
                    secondary,
                ),
            };
            if relation == ViewerRelation::Same {
                return None;
            }
        }
        Some(secondary)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Continuity {
    Load,
    Next,
    Gap,
}

enum Trigger {
    Untracked,
    Observed(Vec<u64>),
    Unavailable(Option<Vec<u64>>),
}

impl Trigger {
    fn refresh(&self, now: &Added) -> (bool, bool) {
        match (self, now) {
            (_, Added::NoTrigger) => (false, false),
            (_, Added::Unavailable) => (false, true),
            (Self::Observed(was), Added::Observed(now)) => (gained(was, now), false),
            (Self::Unavailable(Some(was)), Added::Observed(now)) => (false, gained(was, now)),
            (Self::Unavailable(None) | Self::Untracked, Added::Observed(now)) => {
                (false, !now.is_empty())
            }
        }
    }
}

struct Kept {
    units: Option<Vec<u64>>,
    added: Trigger,
}

struct Previous {
    manager_viewers: Option<Vec<CandidateViewer>>,
    links: BTreeMap<u32, u32>,
    relations: Result<Relations, &'static str>,
    zero_visibility: ZeroVisibility,
    controllers: Option<Controllers>,
    sides: BTreeMap<(u64, u8), Kept>,
    complete: bool,
}

fn gained(before: &[u64], now: &[u64]) -> bool {
    now.iter().any(|id| before.binary_search(id).is_err())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct EverySide {
    other_proven: bool,
    local_proven: bool,
    possible: bool,
}

impl EverySide {
    fn proven(self) -> bool {
        self.other_proven || self.local_proven
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CandidateViewer {
    Slot(u32),
    RawLocal(u32),
}

fn push_unique<T: Copy + PartialEq>(list: &mut Vec<T>, item: T) {
    if !list.contains(&item) {
        list.push(item);
    }
}

fn push_viewer_aliases(
    before_links: &BTreeMap<u32, u32>,
    now_links: &BTreeMap<u32, u32>,
    viewer: CandidateViewer,
    recording: bool,
    out: &mut Vec<CandidateViewer>,
) -> bool {
    match viewer {
        CandidateViewer::RawLocal(0) => {
            push_unique(out, CandidateViewer::RawLocal(0));
            true
        }
        CandidateViewer::RawLocal(raw) => {
            let (Some(&before_slot), Some(&now_slot)) =
                (before_links.get(&raw), now_links.get(&raw))
            else {
                return false;
            };
            push_unique(out, CandidateViewer::RawLocal(raw));
            for (links, slot) in [(before_links, before_slot), (now_links, now_slot)] {
                for (&other, &other_slot) in links {
                    if other != 0 && other_slot == slot {
                        push_unique(out, CandidateViewer::RawLocal(other));
                    }
                }
            }
            true
        }
        CandidateViewer::Slot(slot) => {
            if recording && (before_links.is_empty() || now_links.is_empty()) {
                return false;
            }
            push_unique(out, CandidateViewer::Slot(slot));
            if recording {
                for links in [before_links, now_links] {
                    for (&other, &other_slot) in links {
                        if other != 0 && other_slot == slot {
                            push_unique(out, CandidateViewer::RawLocal(other));
                        }
                    }
                }
            }
            true
        }
    }
}

impl Previous {
    fn every_side(&self, sample: &Sample<'_>) -> EverySide {
        let mut other_proven = false;
        let mut local_proven = false;
        let mut possible = false;
        let primary = sample.primary_viewer();
        let secondary = sample.secondary_viewer(Some(&self.links));
        if !(sample.recording && matches!(sample.refreshes, RefreshWitness::Complete { .. })) {
            match (self.manager_viewers.as_deref(), primary) {
                (Some(known), Some(pv)) if !known.is_empty() => {
                    let mut all_same = true;
                    let mut all_different = true;
                    for &prev in known {
                        for curr in std::iter::once(pv).chain(secondary) {
                            match candidate_relation(&self.links, prev, sample.links, curr) {
                                ViewerRelation::Same => all_different = false,
                                ViewerRelation::Different => all_same = false,
                                ViewerRelation::Unknown => {
                                    all_same = false;
                                    all_different = false;
                                }
                            }
                        }
                    }
                    if all_different {
                        other_proven = true;
                    } else if !all_same {
                        possible = true;
                    }
                }
                _ => {
                    possible = true;
                }
            }
        }
        let alternate = sample
            .alternate_viewer
            .filter(|&alt| Some(alt) != sample.viewer);
        match &sample.refreshes {
            RefreshWitness::Complete {
                controller_targets,
                local_viewers,
                cache_written_viewers,
                ..
            } => {
                local_proven = !local_viewers.is_empty() || !cache_written_viewers.is_empty();
                for &raw in controller_targets {
                    let Some(viewer) = sample.viewer else {
                        possible = true;
                        continue;
                    };
                    let now = sample.links.get(&raw).copied();
                    let before = self.links.get(&raw).copied();
                    if let Some(alt) = alternate {
                        if now == Some(viewer)
                            || before == Some(viewer)
                            || now == Some(alt)
                            || before == Some(alt)
                            || now.is_none()
                            || before.is_none()
                        {
                            possible = true;
                        }
                    } else if now == Some(viewer) && before == Some(viewer) {
                        other_proven = true;
                    } else if now == Some(viewer)
                        || before == Some(viewer)
                        || now.is_none()
                        || before.is_none()
                    {
                        possible = true;
                    }
                }
            }
            RefreshWitness::Unavailable => {
                let candidate_may_have_joined =
                    |was: &Controllers, now: &Controllers, candidate: CandidateViewer| match candidate
                    {
                        CandidateViewer::Slot(slot) => now.may_have_joined(was, slot),
                        CandidateViewer::RawLocal(0) => now.may_have_joined(was, 0),
                        CandidateViewer::RawLocal(raw) => {
                            match (self.links.get(&raw).copied(), sample.links.get(&raw).copied()) {
                                (Some(a), Some(b)) => {
                                    now.may_have_joined(was, a)
                                        || (a != b && now.may_have_joined(was, b))
                                }
                                _ => true,
                            }
                        }
                    };
                possible |= match (&self.controllers, &sample.controllers, primary) {
                    (Some(was), Some(now), Some(pv)) => {
                        candidate_may_have_joined(was, now, pv)
                            || secondary.is_some_and(|alt| candidate_may_have_joined(was, now, alt))
                    }
                    _ => true,
                };
            }
        }
        EverySide {
            other_proven,
            local_proven,
            possible,
        }
    }

    fn refresh(
        &self,
        sample: &Sample<'_>,
        key: (u64, u8),
        side: &Side,
        every: EverySide,
    ) -> Option<(Lists, Lists, bool, bool, bool)> {
        let (own_proven, own_possible, other_lost_units) = match self.own_refresh(sample, key, side)
        {
            Some(own) => own,
            None if every.proven() => {
                if matches!(sample.refreshes, RefreshWitness::Complete { .. }) {
                    (Lists::default(), Lists::default(), false)
                } else {
                    (
                        Lists::default(),
                        Lists {
                            recorded: true,
                            units: true,
                        },
                        false,
                    )
                }
            }
            None => return None,
        };
        let own_recorded = own_proven.recorded || own_possible.recorded;
        let non_local_recorded = own_recorded || every.other_proven || every.possible;
        let proven = Lists {
            recorded: own_proven.recorded || every.proven(),
            units: own_proven.units,
        };
        let possible = Lists {
            recorded: own_possible.recorded || every.possible,
            units: own_possible.units,
        };
        Some((proven, possible, own_recorded, non_local_recorded, other_lost_units))
    }

    fn own_refresh(
        &self,
        sample: &Sample<'_>,
        key: (u64, u8),
        side: &Side,
    ) -> Option<(Lists, Lists, bool)> {
        let witnessed = match &sample.refreshes {
            RefreshWitness::Complete { sides, .. } => {
                Some(sides.get(&key).map_or(Lists::default(), |witness| witness.lists))
            }
            RefreshWitness::Unavailable => None,
        };
        let Some(before) = self.sides.get(&key) else {
            let extra = witnessed.unwrap_or_default();
            if self.complete {
                return Some((
                    Lists {
                        recorded: true,
                        units: extra.units,
                    },
                    Lists::default(),
                    false,
                ));
            }
            if extra.any() {
                return Some((extra, Lists::default(), false));
            }
            return None;
        };
        if let Some(witnessed) = witnessed {
            let mut proven = witnessed;
            let mut other_lost_units = false;
            if let (Some(was), Some(now)) = (before.units.as_deref(), side.units.as_deref()) {
                let (added, _) = before.added.refresh(&side.added);
                proven.recorded |= gained(was, now) || added;
                proven.units |= gained(now, was);
                if sample.removal == Removal::BothSides {
                    let other = (key.0, 1 - key.1);
                    let other_lost = match (
                        self.sides
                            .get(&other)
                            .and_then(|kept| kept.units.as_deref()),
                        sample
                            .sides
                            .get(&other)
                            .and_then(|side| side.units.as_deref()),
                    ) {
                        (Some(was), Some(now)) => Some(gained(now, was)),
                        _ => None,
                    };
                    if other_lost == Some(true) {
                        proven.units = true;
                        other_lost_units = true;
                    }
                }
            }
            return Some((proven, Lists::default(), other_lost_units));
        }
        let (Some(was), Some(now)) = (before.units.as_deref(), side.units.as_deref()) else {
            return None;
        };
        let (added, _) = before.added.refresh(&side.added);
        let proven = Lists {
            recorded: gained(was, now) || added,
            units: gained(now, was),
        };
        let other = (key.0, 1 - key.1);
        let other_lost = match (
            self.sides
                .get(&other)
                .and_then(|kept| kept.units.as_deref()),
            sample
                .sides
                .get(&other)
                .and_then(|side| side.units.as_deref()),
        ) {
            (Some(was), Some(now)) => Some(gained(now, was)),
            _ => None,
        };
        let (proven, mut possible) = match (sample.removal, other_lost) {
            (_, Some(false)) => (proven, Lists::default()),
            (Removal::BothSides, Some(true)) => (
                Lists {
                    units: true,
                    ..proven
                },
                Lists::default(),
            ),
            (Removal::BothSidesUnlessRoundRetreat, Some(true)) | (_, None) => (
                proven,
                Lists {
                    recorded: false,
                    units: true,
                },
            ),
        };
        possible.recorded = true;
        if side.unit_lists != side.lists {
            possible.units = true;
        }
        Some((proven, possible, other_lost == Some(true)))
    }
}

#[derive(Default)]
pub struct History {
    previous: Option<Previous>,
    held: BTreeMap<(u64, u8), Engaged>,
}

impl History {
    pub fn advance_reporting(
        &mut self,
        sample: Sample<'_>,
        continuity: Continuity,
        changed: &mut Vec<((u64, u8), Engaged)>,
        ended: &mut Vec<(u64, u8)>,
    ) {
        let before = self.replace(sample, continuity);
        changed.clear();
        ended.clear();
        changed.extend(
            self.held
                .iter()
                .filter(|(key, value)| before.get(key) != Some(value))
                .map(|(key, value)| (*key, *value)),
        );
        ended.extend(before.keys().filter(|key| !self.held.contains_key(key)));
    }

    pub fn advance(&mut self, sample: Sample<'_>, continuity: Continuity) {
        self.replace(sample, continuity);
    }

    fn replace(
        &mut self,
        sample: Sample<'_>,
        continuity: Continuity,
    ) -> BTreeMap<(u64, u8), Engaged> {
        let previous = self
            .previous
            .take()
            .filter(|_| continuity != Continuity::Load);
        let next = previous.as_ref().filter(|_| continuity == Continuity::Next);
        let every = next.map(|previous| previous.every_side(&sample));
        let mut held = BTreeMap::new();
        let (side_witnesses, local_viewers, cache_written_viewers): (
            Option<&BTreeMap<(u64, u8), SideWitness>>,
            &[u32],
            &[u32],
        ) = match &sample.refreshes {
            RefreshWitness::Complete {
                sides,
                local_viewers,
                cache_written_viewers,
                ..
            } => (
                Some(sides),
                local_viewers.as_slice(),
                cache_written_viewers.as_slice(),
            ),
            RefreshWitness::Unavailable => (None, &[], &[]),
        };
        let primary_viewer = sample.primary_viewer();
        let secondary_viewer = sample.secondary_viewer(next.map(|p| &p.links));
        let before = next.map(|p| {
            (
                &p.links,
                p.relations.as_ref().map_err(|reason| *reason),
                &p.zero_visibility,
            )
        });
        let mut evaluated = Vec::new();
        let mut viewers = Vec::new();
        for (key, side) in &sample.sides {
            let eval = |lists: Tested, when: usize, viewer: CandidateViewer| {
                let snapshot = if when == 0 {
                    Some((sample.links, sample.relations, sample.zero_visibility))
                } else {
                    before
                };
                let (primary, secondary) = match lists {
                    Tested::Recorded => side.lists.as_ref(),
                    Tested::Units => side.unit_lists.as_ref(),
                }?;
                let (links, relations, zero_visibility) = snapshot?;
                match viewer {
                    CandidateViewer::RawLocal(viewer_raw) => {
                        let mut unknown = false;
                        let target_slot = if viewer_raw == 0 {
                            Some(0)
                        } else {
                            links.get(&viewer_raw).copied()
                        };
                        for &raw in primary {
                            let owner = if raw == 0 {
                                0
                            } else if let Some(&slot) = links.get(&raw) {
                                slot
                            } else {
                                unknown = true;
                                continue;
                            };
                            if owner == 0 {
                                if viewer_raw == 0 {
                                    return Some(true);
                                }
                                let Some(target) = target_slot else {
                                    unknown = true;
                                    continue;
                                };
                                match zero_visibility.reverse_shared(target) {
                                    Ok(true) => return Some(true),
                                    Ok(false) => {}
                                    Err(_) => unknown = true,
                                }
                            } else {
                                let Some(target) = target_slot else {
                                    unknown = true;
                                    continue;
                                };
                                if target == 0 {
                                    match zero_visibility.forward_shared(owner) {
                                        Ok(true) => return Some(true),
                                        Ok(false) => {}
                                        Err(_) => unknown = true,
                                    }
                                } else if owner == target {
                                    return Some(true);
                                } else {
                                    match relations.ok() {
                                        Some(relations) if relations.shared(owner, target) => {
                                            return Some(true);
                                        }
                                        Some(_) => {}
                                        None => unknown = true,
                                    }
                                }
                            }
                        }
                        for &raw in secondary {
                            if raw == viewer_raw {
                                return Some(true);
                            }
                            if raw != 0 && viewer_raw != 0 {
                                match (links.get(&raw).copied(), target_slot) {
                                    (Some(owner), Some(target)) if owner == target => {
                                        return Some(true);
                                    }
                                    (Some(_), Some(_)) => {}
                                    _ => unknown = true,
                                }
                            }
                        }
                        (!unknown).then_some(false)
                    }
                    CandidateViewer::Slot(slot) => {
                        let mut unknown = false;
                        for &raw in primary {
                            let owner = if raw == 0 {
                                0
                            } else if let Some(&member) = links.get(&raw) {
                                member
                            } else {
                                unknown = true;
                                continue;
                            };
                            if owner == 0 {
                                match zero_visibility.reverse_shared(slot) {
                                    Ok(true) => return Some(true),
                                    Ok(false) => {}
                                    Err(_) => unknown = true,
                                }
                            } else if slot == 0 {
                                match zero_visibility.forward_shared(owner) {
                                    Ok(true) => return Some(true),
                                    Ok(false) => {}
                                    Err(_) => unknown = true,
                                }
                            } else if owner == slot {
                                return Some(true);
                            } else {
                                match relations.ok() {
                                    Some(relations) if relations.shared(owner, slot) => {
                                        return Some(true);
                                    }
                                    Some(_) => {}
                                    None => unknown = true,
                                }
                            }
                        }
                        for &raw in secondary {
                            if raw == 0 {
                                continue;
                            }
                            let Some(&member) = links.get(&raw) else {
                                unknown = true;
                                continue;
                            };
                            if member == slot {
                                return Some(true);
                            }
                        }
                        (!unknown).then_some(false)
                    }
                }
            };
            let held_before = self.held.get(key).copied().unwrap_or(Engaged::Unknown);
            let value = match (continuity, next, every) {
                (Continuity::Load, ..) => primary_viewer.map_or(Engaged::Unknown, |viewer| {
                    Engaged::of(eval(Tested::Recorded, 0, viewer))
                }),
                (_, Some(previous), Some(every)) => {
                    match previous.refresh(&sample, *key, side, every) {
                        None => Engaged::Unknown,
                        Some((
                            proven,
                            possible,
                            own_recorded,
                            non_local_recorded,
                            other_lost_units,
                        )) => {
                            if !proven.any() && !possible.any() {
                                held_before
                            } else {
                                evaluated.clear();
                                if proven.recorded || possible.recorded {
                                    viewers.clear();
                                    let mut missing_viewer = false;
                                    if sample.recording
                                        && let Some(sides) = side_witnesses
                                    {
                                        for &raw in
                                            local_viewers.iter().chain(cache_written_viewers)
                                        {
                                            push_unique(&mut viewers, CandidateViewer::RawLocal(raw));
                                        }
                                        if own_recorded {
                                            let side_rec = sides
                                                .get(key)
                                                .map_or(&[][..], |w| w.recorded_viewers.as_slice());
                                            if !side_rec.is_empty() {
                                                for &raw in side_rec {
                                                    push_unique(
                                                        &mut viewers,
                                                        CandidateViewer::RawLocal(raw),
                                                    );
                                                }
                                            } else if let Some(pv) = primary_viewer {
                                                push_unique(&mut viewers, pv);
                                                if let Some(alt) = secondary_viewer {
                                                    push_unique(&mut viewers, alt);
                                                }
                                            } else {
                                                missing_viewer = true;
                                            }
                                        }
                                    } else {
                                        for &raw in
                                            local_viewers.iter().chain(cache_written_viewers)
                                        {
                                            push_unique(&mut viewers, CandidateViewer::RawLocal(raw));
                                        }
                                        if non_local_recorded
                                            || (local_viewers.is_empty()
                                                && cache_written_viewers.is_empty())
                                        {
                                            if let Some(pv) = primary_viewer {
                                                push_unique(&mut viewers, pv);
                                                if let Some(alt) = secondary_viewer {
                                                    push_unique(&mut viewers, alt);
                                                }
                                            } else {
                                                missing_viewer = true;
                                            }
                                        }
                                    }
                                    if missing_viewer || viewers.is_empty() {
                                        evaluated.push(Engaged::Unknown);
                                    }
                                    for &viewer in &viewers {
                                        for when in 0..2 {
                                            evaluated.push(Engaged::of(eval(
                                                Tested::Recorded,
                                                when,
                                                viewer,
                                            )));
                                        }
                                    }
                                }
                                if proven.units || possible.units {
                                    viewers.clear();
                                    let mut missing_viewer = false;
                                    if sample.recording
                                        && let Some(sides) = side_witnesses
                                    {
                                        if let Some(witness) = sides.get(key) {
                                            for &raw in &witness.units_viewers {
                                                push_unique(
                                                    &mut viewers,
                                                    CandidateViewer::RawLocal(raw),
                                                );
                                            }
                                        }
                                        if other_lost_units
                                            && sample.removal == Removal::BothSides
                                            && let Some(other) = sides.get(&(key.0, 1 - key.1))
                                        {
                                            for &raw in &other.units_viewers {
                                                push_unique(
                                                    &mut viewers,
                                                    CandidateViewer::RawLocal(raw),
                                                );
                                            }
                                        }
                                        if viewers.is_empty() {
                                            if let Some(pv) = primary_viewer {
                                                push_unique(&mut viewers, pv);
                                                if let Some(alt) = secondary_viewer {
                                                    push_unique(&mut viewers, alt);
                                                }
                                            } else {
                                                missing_viewer = true;
                                            }
                                        }
                                    } else {
                                        if let Some(pv) = primary_viewer {
                                            push_unique(&mut viewers, pv);
                                            if let Some(alt) = secondary_viewer {
                                                push_unique(&mut viewers, alt);
                                            }
                                        } else {
                                            missing_viewer = true;
                                        }
                                        for &raw in
                                            local_viewers.iter().chain(cache_written_viewers)
                                        {
                                            push_unique(
                                                &mut viewers,
                                                CandidateViewer::RawLocal(raw),
                                            );
                                        }
                                    }
                                    if missing_viewer || viewers.is_empty() {
                                        evaluated.push(Engaged::Unknown);
                                    }
                                    for &viewer in &viewers {
                                        for when in 0..2 {
                                            evaluated.push(Engaged::of(eval(
                                                Tested::Units,
                                                when,
                                                viewer,
                                            )));
                                        }
                                    }
                                }
                                Engaged::agreed(&evaluated, (!proven.any()).then_some(held_before))
                            }
                        }
                    }
                }
                _ => Engaged::Unknown,
            };
            held.insert(*key, value);
        }
        let complete_recording =
            sample.recording && matches!(sample.refreshes, RefreshWitness::Complete { .. });
        let cache_written = || {
            let mut known = Vec::new();
            for &raw in cache_written_viewers {
                push_unique(&mut known, CandidateViewer::RawLocal(raw));
            }
            known
        };
        let manager_viewers = match continuity {
            Continuity::Load => primary_viewer.map(|v| vec![v]),
            Continuity::Gap => {
                if complete_recording && !cache_written_viewers.is_empty() {
                    Some(cache_written())
                } else {
                    (!sample.recording)
                        .then_some(primary_viewer)
                        .flatten()
                        .map(|v| vec![v])
                }
            }
            Continuity::Next => next.and_then(|prev_state| {
                if complete_recording {
                    if !cache_written_viewers.is_empty() {
                        return Some(cache_written());
                    }
                    return match (prev_state.manager_viewers.as_deref(), primary_viewer) {
                        (Some(prev), Some(pv)) => {
                            let mut known = Vec::new();
                            for &cached in prev {
                                if std::iter::once(pv).chain(secondary_viewer).any(|tick_viewer| {
                                    candidate_relation(
                                        &prev_state.links,
                                        cached,
                                        sample.links,
                                        tick_viewer,
                                    ) != ViewerRelation::Different
                                }) {
                                    push_unique(&mut known, cached);
                                }
                            }
                            (!known.is_empty()).then_some(known)
                        }
                        (Some(prev), None) => Some(prev.to_vec()),
                        (None, Some(pv)) => {
                            let mut known = Vec::new();
                            for tick_viewer in std::iter::once(pv).chain(secondary_viewer) {
                                if !push_viewer_aliases(
                                    &prev_state.links,
                                    sample.links,
                                    tick_viewer,
                                    sample.recording,
                                    &mut known,
                                ) {
                                    return None;
                                }
                            }
                            Some(known)
                        }
                        (None, None) => None,
                    };
                }
                match (prev_state.manager_viewers.as_deref(), primary_viewer) {
                    (Some(prev), Some(pv)) => {
                        let mut known = Vec::new();
                        for &cached in prev {
                            for tick_viewer in std::iter::once(pv).chain(secondary_viewer) {
                                match candidate_relation(
                                    &prev_state.links,
                                    cached,
                                    sample.links,
                                    tick_viewer,
                                ) {
                                    ViewerRelation::Same => push_unique(&mut known, cached),
                                    ViewerRelation::Different => {
                                        push_unique(&mut known, tick_viewer);
                                    }
                                    ViewerRelation::Unknown => {
                                        push_unique(&mut known, cached);
                                        push_unique(&mut known, tick_viewer);
                                    }
                                }
                            }
                        }
                        (!known.is_empty()).then_some(known)
                    }
                    (None, Some(pv)) => {
                        let mut known = Vec::new();
                        for tick_viewer in std::iter::once(pv).chain(secondary_viewer) {
                            if !push_viewer_aliases(
                                &prev_state.links,
                                sample.links,
                                tick_viewer,
                                sample.recording,
                                &mut known,
                            ) {
                                return None;
                            }
                        }
                        Some(known)
                    }
                    _ => None,
                }
            }),
        };
        let mut earlier = previous.map(|previous| previous.sides).unwrap_or_default();
        self.previous = Some(Previous {
            manager_viewers,
            links: sample.links.clone(),
            relations: sample.relations.cloned(),
            zero_visibility: sample.zero_visibility.clone(),
            controllers: sample.controllers,
            sides: sample
                .sides
                .into_iter()
                .map(|(key, side)| {
                    let added = match side.added {
                        Added::NoTrigger => Trigger::Untracked,
                        Added::Observed(ids) => Trigger::Observed(ids),
                        Added::Unavailable => {
                            Trigger::Unavailable(earlier.remove(&key).and_then(|kept| {
                                match kept.added {
                                    Trigger::Observed(ids) => Some(ids),
                                    Trigger::Unavailable(ids) => ids,
                                    Trigger::Untracked => None,
                                }
                            }))
                        }
                    };
                    (
                        key,
                        Kept {
                            units: side.units,
                            added,
                        },
                    )
                })
                .collect(),
            complete: sample.complete,
        });
        std::mem::replace(&mut self.held, held)
    }

    pub fn sides(&self) -> impl Iterator<Item = ((u64, u8), Engaged)> + '_ {
        self.held.iter().map(|(key, value)| (*key, *value))
    }

    pub(crate) fn side(&self, combat: u64, side: u8) -> Engaged {
        self.held
            .get(&(combat, side))
            .copied()
            .unwrap_or(Engaged::Unknown)
    }

    pub fn combat(&self, combat: u64) -> Engaged {
        self.side(combat, 0).either(self.side(combat, 1))
    }
}

#[cfg(test)]
#[path = "engaged/tests.rs"]
mod tests;
