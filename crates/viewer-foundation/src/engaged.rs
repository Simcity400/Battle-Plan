//! The combat "engaged" flag for any viewing country.
//! The core is common to land and naval combatants; each kind feeds it its
//! sides (land in [`super::engagement_progress::LandSides`], naval in
//! [`super::naval_sides::NavalSides`]).
//!
//! A side is engaged when a primary country is the viewer or shares vision
//! with it (war veto, then faction, then subject or overlord either way), or
//! when a roster owner is the viewer. Diplomacy changes alone never refresh it.
//! Refresh events are:
//!
//! - A unit joining or being re-added: append its owners and refresh that side.
//! - Combat unit removal: rebuild both sides, attacker then defender, from
//!   remaining units, even if the removed unit was absent from a side. Movement
//!   cancellation, province entry, deletion, transfer, teleport and route
//!   assignment can trigger this removal.
//! - Land round retreat: rebuild only the retreating side.
//! - A naval transfer joining or a border war starting: refresh that side.
//! - Load, an hourly local-player country change, or assigning a controller
//!   the local player's country: refresh every side.
//!
//! The recording keeps each side's hourly units and lists, not the flag, so it
//! is replayed from them with its lag. [`Previous::refresh`] alone classifies
//! each side's hour. An hour proves a side's rebuild when its own units
//! changed, an [`Side::added`] id appeared that the previous hour observed
//! without, its combat started or the viewer moved, and allows one it cannot
//! prove when the other side lost a unit under a kind whose removals may be
//! one-sided ([`Removal`]), when a kind's [`Added`] ids are unavailable this
//! hour or were the hour before (a new id may then have come in either), or
//! when the controller rosters show a controller that may have joined the
//! viewer's country ([`Controllers`]). Each rebuild the hour allows is
//! evaluated with the lists it would have tested (the recorded lists after a
//! join or an every-side refresh, the unit-derived lists after a removal)
//! under both the previous and the current sample's relations, each with its
//! own sample's country links (a refresh resolves the list references at its
//! instant, and a reference may change country between samples), and a side
//! that may not have rebuilt keeps its held value as a candidate: when the
//! candidates agree that is the flag, an unresolved one makes it
//! [`Engaged::Unknown`], and otherwise it is [`Engaged::Ambiguous`] with its
//! [`Ambiguity`].
//!
//! Known misses: a rebuild that leaves the unit set unchanged (a present unit
//! re-added), a controller change the hourly rosters do not show (one given
//! the country it already has, or one that joins and leaves between samples),
//! and countries that enter a list without a rebuild (air support, convoy and
//! transfer owners) until the next one.

use crate::map::visibility::Relations;
use replay::{frame::DecodedFrame, rows::TableId};
use std::{collections::BTreeMap, io};

/// One hour's sides of one combat kind.
#[derive(Clone)]
pub struct HourSides {
    /// Keyed by (combat, side).
    pub sides: BTreeMap<(u64, u8), Side>,
    /// The combat table is whole: a combat missing from it is not fighting.
    pub complete: bool,
    /// The kind's removal rule.
    pub removal: Removal,
}

/// A combat kind whose sides carry the engaged flag.
pub trait Sides: Sync {
    /// The tables [`Sides::sides`] reads.
    fn tables(&self) -> &'static [TableId];
    /// The hour's combats, ascending: what an index lookup needs, without
    /// building the sides.
    fn combats(&self, frame: &DecodedFrame) -> io::Result<Vec<u64>>;
    fn sides(&self, frame: &DecodedFrame) -> io::Result<HourSides>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engaged {
    Yes,
    No,
    /// The values the samples allow disagree, for this reason.
    Ambiguous(Ambiguity),
    /// An input the answer depends on was not recorded.
    Unknown,
}

/// Why the samples allow more than one flag. A flag held through hours with
/// no refresh keeps the reason of the hour that made it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Ambiguity {
    /// A refresh the samples allow but cannot prove disagrees with the held
    /// flag: an other-side removal that may have been a one-sided round
    /// retreat or is unrecorded, or a controller change the
    /// rosters cannot place.
    Scope,
    /// Refreshes between two samples tested different lists or relations,
    /// and the samples cannot order them.
    Order,
}

impl Ambiguity {
    /// The marker's reason, one per cause.
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

    /// Either side engaged, as the land marker and window test both flags.
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

    /// One flag from the refreshes the hour allows (`evaluated`, each Yes, No
    /// or Unknown) and, when none is proven, the `held` flag. An unresolved
    /// candidate makes it unknown; evaluations that disagree are
    /// [`Ambiguity::Order`], and a held flag that differs from agreeing ones
    /// is [`Ambiguity::Scope`].
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

/// The native test for one side's lists. `resolve` gives, per raw country
/// reference, whether it is the viewer and whether it shares vision with it
/// (the viewer shares with itself); `None` is unresolved and can only block a
/// negative answer.
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

/// How a kind's unit removal refreshes the other side of the combat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Removal {
    /// Land: removing a combat unit rebuilds both sides from their remaining
    /// units, but a round retreat rebuilds only the retreating
    /// side, and the recording cannot tell the two apart, so the other side
    /// may have rebuilt.
    BothSidesUnlessRoundRetreat,
    /// Naval: every removal is `combat unit removal`, which
    /// rebuilds both sides, so a unit leaving one side proves the other's
    /// rebuild from its remaining units.
    BothSides,
}

/// Which lists a rebuild in the hour tested.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Lists {
    /// The recorded lists: a join appends to them.
    pub(crate) recorded: bool,
    /// Rebuilt from the remaining units: a removal.
    pub(crate) units: bool,
}

impl Lists {
    fn any(self) -> bool {
        self.recorded || self.units
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refresh {
    /// A load rebuilds every side; no earlier sample to order against. Only
    /// [`Continuity::Load`] makes it: a viewer change between samples is an
    /// every-side refresh ordered against the relation change like any other.
    Load,
    /// The rebuilds the hour proves and those it allows but cannot prove, by
    /// the lists each tested. With neither the side holds its flag.
    Hour { proven: Lists, possible: Lists },
}

/// The lists a rebuild tested.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tested {
    /// The recorded primary country and roster-owner lists.
    Recorded,
    /// The lists rebuilt from the side's remaining units.
    Units,
}

/// Advances one side's flag by one sampled hour. `evaluate(lists, when)` is
/// `test` of the side's `lists` under this hour's links and relations
/// (`when` 0) or the previous sample's (1). As the game refreshes the byte only on its
/// refresh events and a side keeps it otherwise, only the
/// candidates the refresh allows are evaluated, and a held side none.
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
            // At most both list kinds under both relations.
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

/// A kind's rebuild trigger beyond its unit joins and removals, for one side
/// and hour: ids whose appearance proves a rebuild that tested the recorded
/// lists. Only naval has one: `transfer joining`
/// gives each joining convoy member a fresh combat-wide battle slot and
/// refreshes the flag.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Added {
    /// The kind has no such trigger (land).
    #[default]
    NoTrigger,
    /// The recording's census of the ids failed this hour (the participant
    /// table is incomplete). This is not a native state: the game joined
    /// convoys or did not, and the recording cannot tell, so a rebuild of the
    /// recorded lists may have happened.
    Unavailable,
    /// The convoy members' battle slots present this hour, sorted. A slot the
    /// previous hour observed without proves a rebuild; one only an earlier
    /// observation lacked, with unavailable hours between, allows one.
    Observed(Vec<u64>),
}

/// One combat side's recorded inputs for one hour.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Side {
    /// The side's unit handles, sorted; `None` when the hour cannot tell.
    pub units: Option<Vec<u64>>,
    pub added: Added,
    /// Raw primary-country and roster-owner references; `None` when unrecorded.
    pub lists: Option<(Vec<u32>, Vec<u32>)>,
    /// The lists a removal rebuild tests, built from the remaining units'
    /// owners; `None` when unrecorded. A kind whose lists come
    /// from its units alone passes the recorded lists.
    pub unit_lists: Option<(Vec<u32>, Vec<u32>)>,
}

/// The hour's controller rosters, the recorded evidence of a controller
/// change. Assigning a controller writes its player's country and refreshes
/// every combat when that country is the local player's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Controllers {
    /// Per player key: its raw country and that country's canonical slot
    /// (`None` when the links do not hold it).
    pub players: BTreeMap<i32, (u32, Option<u32>)>,
    /// `HumanControllers` counts per canonical slot, summed over its references.
    pub counts: BTreeMap<u32, i64>,
}

impl Controllers {
    /// Whether a controller may have been given `viewer`'s country since
    /// `was`: a player now there, or unresolved, whose key or country is new,
    /// or a higher count. The rosters are hourly, so this cannot prove the
    /// command ran, only allow it.
    fn may_have_joined(&self, was: &Self, viewer: u32) -> bool {
        let count = |rosters: &Self| rosters.counts.get(&viewer).copied().unwrap_or(0);
        count(self) > count(was)
            || self.players.iter().any(|(key, now)| {
                now.1.is_none_or(|slot| slot == viewer) && was.players.get(key) != Some(now)
            })
    }
}

/// Every combat side of one sampled hour.
pub struct Sample<'a> {
    pub viewer: u32,
    /// Raw country reference to canonical slot.
    pub links: &'a BTreeMap<u32, u32>,
    pub relations: Result<&'a Relations, &'static str>,
    /// `None` when either roster table is not whole this hour.
    pub controllers: Option<Controllers>,
    /// Keyed by (combat, side).
    pub sides: BTreeMap<(u64, u8), Side>,
    /// Every combat is present: a side missing from the previous sample
    /// started this hour.
    pub complete: bool,
    /// The kind's removal rule.
    pub removal: Removal,
}

/// How a sample follows the previous one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Continuity {
    /// A load: the first recorded hour, or a resume the caller treats as one.
    Load,
    /// The next recorded hour after the previous sample.
    Next,
    /// Hours went unobserved: sides stay unknown until they rebuild.
    Gap,
}

/// A side's [`Added`] ids as the next sample compares them.
enum Trigger {
    /// The kind has none.
    Untracked,
    /// Observed this hour.
    Observed(Vec<u64>),
    /// Unavailable this hour; the last observation before it, if any.
    Unavailable(Option<Vec<u64>>),
}

impl Trigger {
    /// What `now` proves and allows of a rebuild of the recorded lists.
    fn refresh(&self, now: &Added) -> (bool, bool) {
        match (self, now) {
            (_, Added::NoTrigger) => (false, false),
            (_, Added::Unavailable) => (false, true),
            (Self::Observed(was), Added::Observed(now)) => (gained(was, now), false),
            // A new id may have come in the unavailable hours instead.
            (Self::Unavailable(Some(was)), Added::Observed(now)) => (false, gained(was, now)),
            (Self::Unavailable(None) | Self::Untracked, Added::Observed(now)) => {
                (false, !now.is_empty())
            }
        }
    }
}

/// A side as the next sample compares it.
struct Kept {
    units: Option<Vec<u64>>,
    added: Trigger,
}

struct Previous {
    viewer: u32,
    /// The sample's raw reference to canonical slot links, with its
    /// relations: a refresh under these relations resolved its list
    /// references at that instant, and a
    /// reference may have changed country since.
    links: BTreeMap<u32, u32>,
    relations: Result<Relations, &'static str>,
    controllers: Option<Controllers>,
    sides: BTreeMap<(u64, u8), Kept>,
    complete: bool,
}

/// Whether `now` holds an element `before` lacks; both sorted.
fn gained(before: &[u64], now: &[u64]) -> bool {
    now.iter().any(|id| before.binary_search(id).is_err())
}

/// An every-side refresh between two samples. It tests each
/// side's recorded lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct EverySide {
    proven: bool,
    possible: bool,
}

impl Previous {
    /// The every-side refresh between this sample and `sample`. A viewer
    /// change proves one: the game refreshes when the local player's country
    /// differs from its cached one, and the game already does when it moves
    /// the local player. The rosters allow one when a controller may have
    /// been given the viewer's country, or when they cannot rule that out.
    fn every_side(&self, sample: &Sample<'_>) -> EverySide {
        EverySide {
            proven: self.viewer != sample.viewer,
            possible: match (&self.controllers, &sample.controllers) {
                (Some(was), Some(now)) => now.may_have_joined(was, sample.viewer),
                _ => true,
            },
        }
    }

    /// What the hour proves or allows for side `key` of `sample`, with the
    /// hour's [`EverySide`] refresh; `None` when it cannot tell whether the
    /// side rebuilt. The one classification of a side's hour.
    fn refresh(
        &self,
        sample: &Sample<'_>,
        key: (u64, u8),
        side: &Side,
        every: EverySide,
    ) -> Option<Refresh> {
        let (mut proven, mut possible) = match self.own_refresh(sample, key, side) {
            Some(own) => own,
            // Its own rebuilds are unknown, but a proven every-side refresh
            // ran; any rebuild after it stays possible.
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

    /// The side's own rebuilds, as (proven, possible) lists.
    fn own_refresh(
        &self,
        sample: &Sample<'_>,
        key: (u64, u8),
        side: &Side,
    ) -> Option<(Lists, Lists)> {
        let Some(before) = self.sides.get(&key) else {
            // The combat started this hour, or the previous sample missed it.
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
        // A removal from the other side: `None` when either sample lacks it.
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

/// The flag per combat side, folded over consecutive samples.
#[derive(Default)]
pub struct History {
    previous: Option<Previous>,
    held: BTreeMap<(u64, u8), Engaged>,
}

impl History {
    /// Advances by one sample and reports what moved: `changed` gets every
    /// side that is new or took another value, `ended` every side the sample
    /// no longer holds. Both are cleared first.
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

    /// Advances by one sample; returns the flags it replaced.
    fn replace(
        &mut self,
        sample: Sample<'_>,
        continuity: Continuity,
    ) -> BTreeMap<(u64, u8), Engaged> {
        let previous = self
            .previous
            .take()
            .filter(|_| continuity != Continuity::Load);
        // After unobserved hours the last rebuild, a viewer change's
        // included, cannot be placed.
        let next = previous.as_ref().filter(|_| continuity == Continuity::Next);
        let every = next.map(|previous| previous.every_side(&sample));
        let mut held = BTreeMap::new();
        // The previous sample's identities and relations, taken together: a
        // reference its links lack stays unresolved for that candidate.
        let before = next.map(|p| (&p.links, p.relations.as_ref().map_err(|reason| *reason)));
        for (key, side) in &sample.sides {
            let refresh = match (continuity, next, every) {
                (Continuity::Load, ..) => Some(Refresh::Load),
                (_, Some(previous), Some(every)) => previous.refresh(&sample, *key, side, every),
                _ => None,
            };
            // A kind whose unit lists are its recorded lists tests them once.
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
        // An unavailable hour carries the last observation forward; a load starts without one.
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

    /// Every side of the last sample with its flag.
    pub fn sides(&self) -> impl Iterator<Item = ((u64, u8), Engaged)> + '_ {
        self.held.iter().map(|(key, value)| (*key, *value))
    }

    /// The side's flag after the last sample; a side not in it is unknown.
    pub(crate) fn side(&self, combat: u64, side: u8) -> Engaged {
        self.held
            .get(&(combat, side))
            .copied()
            .unwrap_or(Engaged::Unknown)
    }

    /// Either side of the combat engaged.
    pub fn combat(&self, combat: u64) -> Engaged {
        self.side(combat, 0).either(self.side(combat, 1))
    }
}

#[cfg(test)]
#[path = "engaged/tests.rs"]
mod tests;
