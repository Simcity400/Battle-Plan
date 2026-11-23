
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
    pub viewer: u32,
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

fn viewer_change(
    before_slot: u32,
    before_raw: Option<u32>,
    now_slot: u32,
    now_raw: Option<u32>,
) -> bool {
    match (before_raw, now_raw) {
        (Some(a), Some(b)) => !(a == b || (a != 0 && b != 0 && before_slot == now_slot)),
        _ => before_slot != now_slot,
    }
}

impl Sample<'_> {
    fn primary_viewer(&self) -> CandidateViewer {
        match self.viewer_raw {
            Some(raw) => CandidateViewer::RawLocal(raw),
            None => CandidateViewer::Slot(self.viewer),
        }
    }

    fn secondary_viewer(&self) -> Option<CandidateViewer> {
        let alt_slot = self.alternate_viewer?;
        if !viewer_change(self.viewer, self.viewer_raw, alt_slot, self.alternate_viewer_raw) {
            return None;
        }
        Some(match self.alternate_viewer_raw {
            Some(raw) => CandidateViewer::RawLocal(raw),
            None => CandidateViewer::Slot(alt_slot),
        })
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
    viewer: u32,
    viewer_raw: Option<u32>,
    alternate_viewer_refreshed: Option<CandidateViewer>,
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

impl Previous {
    fn every_side(&self, sample: &Sample<'_>) -> EverySide {
        let mut other_proven = false;
        let mut local_proven = false;
        let mut possible = false;
        if viewer_change(self.viewer, self.viewer_raw, sample.viewer, sample.viewer_raw)
            && !(sample.recording && matches!(sample.refreshes, RefreshWitness::Complete { .. }))
        {
            let already_refreshed = self.alternate_viewer_refreshed.is_some_and(|refreshed| {
                match (refreshed, sample.primary_viewer()) {
                    (CandidateViewer::RawLocal(a), CandidateViewer::RawLocal(b)) => {
                        a == b
                            || (a != 0
                                && b != 0
                                && sample.links.get(&a).copied() == Some(sample.viewer)
                                && self.links.get(&a).copied() == Some(sample.viewer))
                    }
                    (CandidateViewer::Slot(a), CandidateViewer::Slot(b)) => a == b,
                    _ => false,
                }
            });
            if already_refreshed {
                possible = true;
            } else {
                other_proven = true;
            }
        }
        let alternate = sample.alternate_viewer.filter(|&alt| alt != sample.viewer);
        match &sample.refreshes {
            RefreshWitness::Complete {
                controller_targets,
                local_viewers,
                ..
            } => {
                local_proven = !local_viewers.is_empty();
                for &raw in controller_targets {
                    let now = sample.links.get(&raw).copied();
                    let before = self.links.get(&raw).copied();
                    if let Some(alt) = alternate {
                        if now == Some(sample.viewer)
                            || before == Some(sample.viewer)
                            || now == Some(alt)
                            || before == Some(alt)
                            || now.is_none()
                            || before.is_none()
                        {
                            possible = true;
                        }
                    } else if now == Some(sample.viewer) && before == Some(sample.viewer) {
                        other_proven = true;
                    } else if now == Some(sample.viewer)
                        || before == Some(sample.viewer)
                        || now.is_none()
                        || before.is_none()
                    {
                        possible = true;
                    }
                }
            }
            RefreshWitness::Unavailable => {
                possible |= match (&self.controllers, &sample.controllers) {
                    (Some(was), Some(now)) => {
                        now.may_have_joined(was, sample.viewer)
                            || alternate.is_some_and(|alt| now.may_have_joined(was, alt))
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
        let (side_witnesses, local_viewers): (Option<&BTreeMap<(u64, u8), SideWitness>>, &[u32]) =
            match &sample.refreshes {
                RefreshWitness::Complete {
                    sides,
                    local_viewers,
                    ..
                } => (Some(sides), local_viewers.as_slice()),
                RefreshWitness::Unavailable => (None, &[]),
            };
        let primary_viewer = sample.primary_viewer();
        let secondary_viewer = sample.secondary_viewer();
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
                (Continuity::Load, ..) => Engaged::of(eval(Tested::Recorded, 0, primary_viewer)),
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
                                    if sample.recording
                                        && let Some(sides) = side_witnesses
                                    {
                                        for &raw in local_viewers {
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
                                            } else {
                                                push_unique(&mut viewers, primary_viewer);
                                                if let Some(alt) = secondary_viewer {
                                                    push_unique(&mut viewers, alt);
                                                }
                                            }
                                        }
                                    } else {
                                        for &raw in local_viewers {
                                            push_unique(&mut viewers, CandidateViewer::RawLocal(raw));
                                        }
                                        if non_local_recorded || local_viewers.is_empty() {
                                            push_unique(&mut viewers, primary_viewer);
                                            if let Some(alt) = secondary_viewer {
                                                push_unique(&mut viewers, alt);
                                            }
                                        }
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
                                            push_unique(&mut viewers, primary_viewer);
                                            if let Some(alt) = secondary_viewer {
                                                push_unique(&mut viewers, alt);
                                            }
                                        }
                                    } else {
                                        push_unique(&mut viewers, primary_viewer);
                                        if let Some(alt) = secondary_viewer {
                                            push_unique(&mut viewers, alt);
                                        }
                                        for &raw in local_viewers {
                                            push_unique(
                                                &mut viewers,
                                                CandidateViewer::RawLocal(raw),
                                            );
                                        }
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
        let alternate_viewer_refreshed = secondary_viewer.filter(|&alt| {
            every.is_some_and(|e| e.other_proven)
                || local_viewers.iter().any(|&raw| match alt {
                    CandidateViewer::RawLocal(alt_raw) => {
                        raw == alt_raw
                            || (raw != 0
                                && alt_raw != 0
                                && sample.links.get(&raw).is_some_and(|&slot| {
                                    sample.links.get(&alt_raw).copied() == Some(slot)
                                }))
                    }
                    CandidateViewer::Slot(alt_slot) => {
                        raw != 0 && sample.links.get(&raw).copied() == Some(alt_slot)
                    }
                })
        });
        let mut earlier = previous.map(|previous| previous.sides).unwrap_or_default();
        self.previous = Some(Previous {
            viewer: sample.viewer,
            viewer_raw: sample.viewer_raw,
            alternate_viewer_refreshed,
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
