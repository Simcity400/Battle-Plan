
use crate::map::visibility::Relations;
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
                "engaged flag ambiguous (the recording cannot prove whether a refresh ran: an other-side removal that may be one-sided or unrecorded, or a controller change)"
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
pub(crate) struct Lists {
    pub(crate) recorded: bool,
    pub(crate) units: bool,
}

impl Lists {
    fn any(self) -> bool {
        self.recorded || self.units
    }
}

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
    pub links: &'a BTreeMap<u32, u32>,
    pub relations: Result<&'a Relations, &'static str>,
    pub controllers: Option<Controllers>,
    pub sides: BTreeMap<(u64, u8), Side>,
    pub complete: bool,
    pub removal: Removal,
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
    links: BTreeMap<u32, u32>,
    relations: Result<Relations, &'static str>,
    controllers: Option<Controllers>,
    sides: BTreeMap<(u64, u8), Kept>,
    complete: bool,
}

fn gained(before: &[u64], now: &[u64]) -> bool {
    now.iter().any(|id| before.binary_search(id).is_err())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct EverySide {
    proven: bool,
    possible: bool,
}

impl Previous {
    fn every_side(&self, sample: &Sample<'_>) -> EverySide {
        EverySide {
            proven: self.viewer != sample.viewer,
            possible: match (&self.controllers, &sample.controllers) {
                (Some(was), Some(now)) => now.may_have_joined(was, sample.viewer),
                _ => true,
            },
        }
    }

    fn refresh(
        &self,
        sample: &Sample<'_>,
        key: (u64, u8),
        side: &Side,
        every: EverySide,
    ) -> Option<Refresh> {
        let (mut proven, mut possible) = match self.own_refresh(sample, key, side) {
            Some(own) => own,
            None if every.proven => (
                Lists::default(),
                Lists {
                    recorded: true,
                    units: true,
                },
            ),
            None => return None,
        };
        proven.recorded |= every.proven;
        possible.recorded |= every.possible;
        Some(Refresh::Hour { proven, possible })
    }

    fn own_refresh(
        &self,
        sample: &Sample<'_>,
        key: (u64, u8),
        side: &Side,
    ) -> Option<(Lists, Lists)> {
        let Some(before) = self.sides.get(&key) else {
            return self.complete.then_some((
                Lists {
                    recorded: true,
                    units: false,
                },
                Lists::default(),
            ));
        };
        let (Some(was), Some(now)) = (before.units.as_deref(), side.units.as_deref()) else {
            return None;
        };
        let (added, may_have_added) = before.added.refresh(&side.added);
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
        possible.recorded |= may_have_added;
        Some((proven, possible))
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
        let before = next.map(|p| (&p.links, p.relations.as_ref().map_err(|reason| *reason)));
        for (key, side) in &sample.sides {
            let refresh = match (continuity, next, every) {
                (Continuity::Load, ..) => Some(Refresh::Load),
                (_, Some(previous), Some(every)) => previous.refresh(&sample, *key, side, every),
                _ => None,
            };
            let same = side.unit_lists == side.lists;
            let mut tested = [[None; 2]; 2];
            let evaluate = |lists: Tested, when: usize| {
                let lists = if same { Tested::Recorded } else { lists };
                *tested[lists as usize][when].get_or_insert_with(|| {
                    let snapshot = if when == 0 {
                        Some((sample.links, sample.relations))
                    } else {
                        before
                    };
                    let (primary, secondary) = match lists {
                        Tested::Recorded => side.lists.as_ref(),
                        Tested::Units => side.unit_lists.as_ref(),
                    }?;
                    let (links, relations) = snapshot?;
                    test(primary, secondary, |raw| {
                        let slot = *links.get(&raw)?;
                        if slot == sample.viewer {
                            return Some((true, true));
                        }
                        Some((false, relations.ok()?.shared(slot, sample.viewer)))
                    })
                })
            };
            let held_before = self.held.get(key).copied().unwrap_or(Engaged::Unknown);
            held.insert(*key, step(held_before, refresh, evaluate));
        }
        let mut earlier = previous.map(|previous| previous.sides).unwrap_or_default();
        self.previous = Some(Previous {
            viewer: sample.viewer,
            links: sample.links.clone(),
            relations: sample.relations.cloned(),
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
