//! The combat "engaged" flag (`CCombatant+0xBA`) for any viewing country.
//! The core is common to land and naval combatants; only land feeds it so far.
//!
//! `UpdateCombatSharedVisionFlag@1413CEBE0` is its only writer. A side is
//! engaged when a country in its `+0x38` list is the viewer or shares vision
//! with it (`HasFullCountryVisibility`: war veto, then faction, then subject or
//! overlord either way), or else when a `+0x50` roster owner is the viewer.
//! The flag is refreshed, never on a diplomacy change, by:
//!
//! - `CCombatant::AddUnit` (1413CC2B0) on the side a unit joins, a present unit
//!   re-added included: it appends the unit's owners to the lists (1413CC0D0);
//! - `RemoveUnitFromCombat` (1413CE250) on **both** sides: it calls `RemoveUnit`
//!   (vtable +0x80, 1413CE340) on side `+0x28` then `+0x30`, and each rebuilds
//!   its lists from its remaining units alone, whether or not the unit was on
//!   it. Its callers are `CUnit::CancelMovement` (140BEBF70), the check when a
//!   unit enters a province (140BA9FD0) and the remove-from-every-combat path
//!   (140BF4C40: deletion, ship transfer, exile transfer, teleport, route
//!   assignment);
//! - the land round retreat, the one removal of one side only: 14129DB50 and
//!   `CLandCombat::CheckRoundTermination` through 1413CE640 reach 1413CE670,
//!   which calls `RemoveUnit` on the retreating side alone;
//! - `AddTransferToGroup` (naval) and border-war creation on their side;
//! - every side (1413CEBC0) at load; when the local player's country moves,
//!   which `CCombatManager`'s hourly check (140BA9890) compares against its
//!   cached one; and whenever `CChangeCountryControllerCommand::Execute`
//!   (14113E4B0) gives a controller the local player's country, the local
//!   player or another one, through 140BABC00.
//!
//! The recording keeps each side's hourly units and lists, not the flag, so it
//! is replayed from them with its lag. [`Previous::refresh`] alone classifies
//! each side's hour. An hour proves a side's rebuild when its own units
//! changed, an [`Side::added`] id appeared, its combat started or the viewer
//! moved, and allows one it cannot prove when the other side lost a unit under
//! a kind whose removals may be one-sided ([`Removal`]) or the controller
//! rosters show a controller that may have joined the viewer's country
//! ([`Controllers`]). Each rebuild the hour allows is evaluated with the lists
//! it would have tested (the recorded lists after a join or an every-side
//! refresh, the unit-derived lists after a removal) under both the previous and
//! the current sample's relations, each with its own sample's country links (a
//! refresh resolves the list references at its instant, and a reference may
//! change country between samples), and a side that may not have rebuilt keeps
//! its held value as a candidate: when the candidates agree that is the flag,
//! an unresolved one makes it [`Engaged::Unknown`], and otherwise it is
//! [`Engaged::Ambiguous`] with its [`Ambiguity`].
//!
//! Known misses: a rebuild that leaves the unit set unchanged (a present unit
//! re-added), a controller change the hourly rosters do not show (one given
//! the country it already has, or one that joins and leaves between samples),
//! and countries that enter a list without a rebuild (air support, convoy and
//! transfer owners) until the next one.

use crate::map::visibility::Relations;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Engaged {
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
pub(crate) enum Ambiguity {
    /// A refresh the samples allow but cannot prove disagrees with the held
    /// flag: an other-side removal that may have been a one-sided round
    /// retreat (1413CE670) or is unrecorded, or a controller change the
    /// rosters cannot place.
    Scope,
    /// Refreshes between two samples tested different lists or relations,
    /// and the samples cannot order them.
    Order,
}

impl Ambiguity {
    /// The marker's reason, one per cause.
    pub(crate) fn reason(self) -> &'static str {
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
    pub(crate) fn either(self, other: Self) -> Self {
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

/// How a kind's unit removal refreshes the other side of the combat. A kind
/// whose every removal is `RemoveUnitFromCombat` (1413CE250) adds a variant
/// that proves the other side's rebuild.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Removal {
    /// Land: `RemoveUnitFromCombat` rebuilds both sides from their remaining
    /// units, but a round retreat (1413CE670) rebuilds only the retreating
    /// side, and the recording cannot tell the two apart, so the other side
    /// may have rebuilt.
    BothSidesUnlessRoundRetreat,
}

/// Which lists a rebuild in the hour tested.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Lists {
    /// The recorded lists: a join appends to them (1413CC0D0).
    pub(crate) recorded: bool,
    /// Rebuilt from the remaining units (1413CE340): a removal.
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
    /// The recorded `+0x38` and `+0x50` lists.
    Recorded,
    /// The lists rebuilt from the side's remaining units.
    Units,
}

/// Advances one side's flag by one sampled hour. `evaluate(lists, when)` is
/// [`test`] of the side's `lists` under this hour's links and relations
/// (`when` 0) or the previous sample's (1). As the game refreshes the byte only on its
/// refresh events (1413CEBE0) and a side keeps it otherwise, only the
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

/// One combat side's recorded inputs for one hour.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Side {
    /// The side's unit handles, sorted; `None` when the hour cannot tell.
    pub(crate) units: Option<Vec<u64>>,
    /// Sorted ids whose appearance this hour proves a rebuild that tested the
    /// recorded lists (naval convoy battle slots from `AddTransferToGroup`);
    /// `None` for a kind without them.
    pub(crate) added: Option<Vec<u64>>,
    /// Raw references of the `+0x38` and `+0x50` lists; `None` when unrecorded.
    pub(crate) lists: Option<(Vec<u32>, Vec<u32>)>,
    /// The lists a removal rebuild tests, built from the remaining units'
    /// owners (1413CE340); `None` when unrecorded. A kind whose lists come
    /// from its units alone passes the recorded lists.
    pub(crate) unit_lists: Option<(Vec<u32>, Vec<u32>)>,
}

/// The hour's controller rosters, the recorded evidence of a controller
/// change: `CChangeCountryControllerCommand::Execute` (14113E4B0) writes the
/// player's country (`CHuman+0x70`, the `Players` row's) and refreshes every
/// combat when that country is the local player's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Controllers {
    /// Per player key: its raw country and that country's canonical slot
    /// (`None` when the links do not hold it).
    pub(crate) players: BTreeMap<i32, (u32, Option<u32>)>,
    /// `HumanControllers` counts per canonical slot, summed over its references.
    pub(crate) counts: BTreeMap<u32, i64>,
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
pub(crate) struct Sample<'a> {
    pub(crate) viewer: u32,
    /// Raw country reference to canonical slot.
    pub(crate) links: &'a BTreeMap<u32, u32>,
    pub(crate) relations: Result<&'a Relations, &'static str>,
    /// `None` when either roster table is not whole this hour.
    pub(crate) controllers: Option<Controllers>,
    /// Keyed by (combat, side).
    pub(crate) sides: BTreeMap<(u64, u8), Side>,
    /// Every combat is present: a side missing from the previous sample
    /// started this hour.
    pub(crate) complete: bool,
    /// The kind's removal rule.
    pub(crate) removal: Removal,
}

/// How a sample follows the previous one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Continuity {
    /// A load: the first recorded hour, or a resume the caller treats as one.
    Load,
    /// The next recorded hour after the previous sample.
    Next,
    /// Hours went unobserved: sides stay unknown until they rebuild.
    Gap,
}

/// A side as the next sample compares it.
struct Kept {
    units: Option<Vec<u64>>,
    added: Option<Vec<u64>>,
}

struct Previous {
    viewer: u32,
    /// The sample's raw reference to canonical slot links, with its
    /// relations: a refresh under these relations resolved its list
    /// references at that instant (140BA56A0/140BA6240 from 1413CEBE0), and a
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

/// An every-side refresh (1413CEBC0) between two samples. It tests each
/// side's recorded lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct EverySide {
    proven: bool,
    possible: bool,
}

impl Previous {
    /// The every-side refresh between this sample and `sample`. A viewer
    /// change proves one: 140BA9890 refreshes when the local player's country
    /// differs from its cached one, and 14113E4B0 already does when it moves
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
        let added = match (before.added.as_deref(), side.added.as_deref()) {
            (_, None) => false,
            (Some(was), Some(now)) => gained(was, now),
            (None, Some(now)) => !now.is_empty(),
        };
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
        Some(match (sample.removal, other_lost) {
            (_, Some(false)) => (proven, Lists::default()),
            (Removal::BothSidesUnlessRoundRetreat, Some(true)) | (_, None) => (
                proven,
                Lists {
                    recorded: false,
                    units: true,
                },
            ),
        })
    }
}

/// The flag per combat side, folded over consecutive samples.
#[derive(Default)]
pub(crate) struct History {
    previous: Option<Previous>,
    held: BTreeMap<(u64, u8), Engaged>,
}

impl History {
    /// Advances by one sample and reports what moved: `changed` gets every
    /// side that is new or took another value, `ended` every side the sample
    /// no longer holds. Both are cleared first.
    pub(crate) fn advance_reporting(
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

    pub(crate) fn advance(&mut self, sample: Sample<'_>, continuity: Continuity) {
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
        self.previous = Some(Previous {
            viewer: sample.viewer,
            links: sample.links.clone(),
            relations: sample.relations.cloned(),
            controllers: sample.controllers,
            sides: sample
                .sides
                .into_iter()
                .map(|(key, side)| {
                    (
                        key,
                        Kept {
                            units: side.units,
                            added: side.added,
                        },
                    )
                })
                .collect(),
            complete: sample.complete,
        });
        std::mem::replace(&mut self.held, held)
    }

    /// Every side of the last sample with its flag.
    pub(crate) fn sides(&self) -> impl Iterator<Item = ((u64, u8), Engaged)> + '_ {
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
    pub(crate) fn combat(&self, combat: u64) -> Engaged {
        self.side(combat, 0).either(self.side(combat, 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve(viewer: u32, shares: &'static [u32]) -> impl Fn(u32) -> Option<(bool, bool)> {
        move |raw| (raw < 100).then(|| (raw == viewer, raw == viewer || shares.contains(&raw)))
    }

    #[test]
    fn primary_members_share_vision_and_secondary_owners_must_be_the_viewer() {
        let r = resolve(1, &[2]);
        assert_eq!(test(&[5, 2], &[], &r), Some(true));
        assert_eq!(test(&[1], &[], &r), Some(true));
        assert_eq!(test(&[5], &[1], &r), Some(true));
        assert_eq!(test(&[5], &[2], &r), Some(false));
        assert_eq!(test(&[], &[], &r), Some(false));
        // An unresolved reference blocks only a negative answer.
        assert_eq!(test(&[500], &[], &r), None);
        assert_eq!(test(&[500, 2], &[], &r), Some(true));
    }

    const RECORDED: Lists = Lists {
        recorded: true,
        units: false,
    };
    const UNITS: Lists = Lists {
        recorded: false,
        units: true,
    };
    const NONE: Lists = Lists {
        recorded: false,
        units: false,
    };

    /// One side's hour as fixed test answers per list kind, [now, before].
    #[derive(Clone, Copy)]
    struct SideHour {
        refresh: Option<Refresh>,
        recorded: [Option<bool>; 2],
        units: [Option<bool>; 2],
    }

    fn hour(refresh: Option<Refresh>, now: Option<bool>, before: Option<bool>) -> SideHour {
        SideHour {
            refresh,
            recorded: [now, before],
            units: [now, before],
        }
    }

    /// [`super::step`] over fixed answers, counting the evaluations it asks.
    fn step(held: Engaged, hour: SideHour) -> Engaged {
        super::step(held, hour.refresh, |lists, when| match lists {
            Tested::Recorded => hour.recorded[when],
            Tested::Units => hour.units[when],
        })
    }

    /// A held side evaluates nothing, a load one list under one relation.
    #[test]
    fn only_the_candidates_a_refresh_allows_are_evaluated() {
        let count = |refresh| {
            let mut calls = 0;
            super::step(Engaged::No, refresh, |_, _| {
                calls += 1;
                Some(true)
            });
            calls
        };
        assert_eq!(count(rebuilt(NONE, NONE)), 0);
        assert_eq!(count(None), 0);
        assert_eq!(count(Some(Refresh::Load)), 1);
        assert_eq!(count(rebuilt(RECORDED, NONE)), 2);
        assert_eq!(count(rebuilt(RECORDED, UNITS)), 4);
    }

    fn rebuilt(proven: Lists, possible: Lists) -> Option<Refresh> {
        Some(Refresh::Hour { proven, possible })
    }

    #[test]
    fn a_rebuild_holds_until_the_next_and_a_split_hour_is_ambiguous() {
        let load = step(
            Engaged::Unknown,
            hour(Some(Refresh::Load), Some(false), None),
        );
        assert_eq!(load, Engaged::No);
        // A relation change alone does not rebuild.
        let held = step(load, hour(rebuilt(NONE, NONE), Some(true), Some(false)));
        assert_eq!(held, Engaged::No);
        let joined = step(held, hour(rebuilt(RECORDED, NONE), Some(true), Some(true)));
        assert_eq!(joined, Engaged::Yes);
        let split = step(
            joined,
            hour(rebuilt(RECORDED, NONE), Some(false), Some(true)),
        );
        assert_eq!(split, Engaged::Ambiguous(Ambiguity::Order));
        assert_eq!(
            step(split, hour(rebuilt(NONE, NONE), Some(false), Some(false))),
            Engaged::Ambiguous(Ambiguity::Order)
        );
        assert_eq!(
            step(Engaged::Yes, hour(None, Some(true), Some(true))),
            Engaged::Unknown
        );
        assert_eq!(
            step(
                Engaged::Yes,
                hour(rebuilt(RECORDED, NONE), Some(true), None)
            ),
            Engaged::Unknown
        );
    }

    /// A rebuild the hour only allows keeps the held value as a candidate; a
    /// join and a removal in one hour test both list kinds.
    #[test]
    fn possible_rebuilds_and_both_list_kinds_must_agree() {
        let possible = |held, now, before| step(held, hour(rebuilt(NONE, UNITS), now, before));
        assert_eq!(possible(Engaged::Yes, Some(true), Some(true)), Engaged::Yes);
        assert_eq!(
            possible(Engaged::No, Some(true), Some(true)),
            Engaged::Ambiguous(Ambiguity::Scope)
        );
        assert_eq!(
            possible(Engaged::Unknown, Some(true), Some(true)),
            Engaged::Unknown
        );
        assert_eq!(
            possible(Engaged::Ambiguous(Ambiguity::Order), Some(true), Some(true)),
            Engaged::Ambiguous(Ambiguity::Scope)
        );
        // A proven rebuild drops the held value.
        let proven = step(
            Engaged::No,
            hour(rebuilt(UNITS, UNITS), Some(true), Some(true)),
        );
        assert_eq!(proven, Engaged::Yes);
        let kinds = |recorded, units| SideHour {
            refresh: rebuilt(RECORDED, UNITS),
            recorded,
            units,
        };
        assert_eq!(
            step(Engaged::No, kinds([Some(true); 2], [Some(true); 2])),
            Engaged::Yes
        );
        assert_eq!(
            step(Engaged::No, kinds([Some(true); 2], [Some(false); 2])),
            Engaged::Ambiguous(Ambiguity::Order)
        );
        assert_eq!(
            step(Engaged::No, kinds([Some(true); 2], [None, Some(true)])),
            Engaged::Unknown
        );
    }

    #[test]
    fn either_side_engages_the_combat() {
        use Engaged::*;
        assert_eq!(No.either(Yes), Yes);
        assert_eq!(Unknown.either(Yes), Yes);
        assert_eq!(Ambiguous(Ambiguity::Order).either(Unknown), Unknown);
        assert_eq!(
            No.either(Ambiguous(Ambiguity::Order)),
            Ambiguous(Ambiguity::Order)
        );
        // Two ambiguous sides report one cause, whichever side it is on.
        assert_eq!(
            Ambiguous(Ambiguity::Order).either(Ambiguous(Ambiguity::Scope)),
            Ambiguous(Ambiguity::Scope)
        );
        assert_eq!(No.either(No), No);
    }

    fn relations(allied: bool) -> Relations {
        Relations::factions_for_tests(if allied { &[(1, 2)] } else { &[][..] })
    }

    fn side(units: &[u64], primary: &[u32]) -> Side {
        let lists = Some((primary.to_vec(), vec![]));
        Side {
            units: Some(units.to_vec()),
            added: None,
            unit_lists: lists.clone(),
            lists,
        }
    }

    fn sample<'a>(
        links: &'a BTreeMap<u32, u32>,
        relations: &'a Relations,
        units: &[u64],
        primary: &[u32],
    ) -> Sample<'a> {
        defended(links, relations, units, primary, &[9])
    }

    /// The defending side (country 5) is foreign to the viewer throughout.
    fn defended<'a>(
        links: &'a BTreeMap<u32, u32>,
        relations: &'a Relations,
        units: &[u64],
        primary: &[u32],
        defenders: &[u64],
    ) -> Sample<'a> {
        Sample {
            viewer: 1,
            links,
            relations: Ok(relations),
            controllers: Some(Controllers::default()),
            sides: BTreeMap::from([
                ((62, 0), side(units, primary)),
                ((62, 1), side(defenders, &[5])),
            ]),
            complete: true,
            removal: Removal::BothSidesUnlessRoundRetreat,
        }
    }

    /// RemoveUnitFromCombat (1413CE250) rebuilds both sides, so a unit
    /// leaving the defenders may let the attackers pick up a faction joined
    /// earlier. Land cannot tell it from a round retreat (1413CE670), which
    /// rebuilds only the retreating side, so the attackers are ambiguous when
    /// the two disagree and decided when they agree.
    #[test]
    fn a_land_removal_may_rebuild_the_other_side() {
        let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
        let (apart, allied) = (relations(false), relations(true));
        let mut history = History::default();
        let next = Continuity::Next;
        history.advance(
            defended(&links, &apart, &[7], &[2], &[8, 9]),
            Continuity::Load,
        );
        // The faction join alone rebuilds nothing.
        history.advance(defended(&links, &allied, &[7], &[2], &[8, 9]), next);
        assert_eq!(history.side(62, 0), Engaged::No);
        history.advance(defended(&links, &allied, &[7], &[2], &[9]), next);
        assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Scope));
        // The defenders lost the unit themselves: rebuilt either way.
        assert_eq!(history.side(62, 1), Engaged::No);
        // A join decides the attackers again; a later defender removal whose
        // rebuild agrees with the held value keeps it.
        history.advance(defended(&links, &allied, &[6, 7], &[2], &[9]), next);
        assert_eq!(history.side(62, 0), Engaged::Yes);
        history.advance(defended(&links, &allied, &[6, 7], &[2], &[]), next);
        assert_eq!(history.side(62, 0), Engaged::Yes);
    }

    /// An id in [`Side::added`] that the previous sample lacked proves a
    /// rebuild of the recorded lists, with no unit change.
    #[test]
    fn a_new_added_id_proves_a_rebuild() {
        let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
        let (apart, allied) = (relations(false), relations(true));
        fn with<'a>(mut sample: Sample<'a>, added: &[u64]) -> Sample<'a> {
            sample.sides.get_mut(&(62, 0)).unwrap().added = Some(added.to_vec());
            sample
        }
        let mut history = History::default();
        history.advance(
            with(sample(&links, &apart, &[7], &[2]), &[3]),
            Continuity::Load,
        );
        history.advance(
            with(sample(&links, &allied, &[7], &[2]), &[3]),
            Continuity::Next,
        );
        assert_eq!(history.side(62, 0), Engaged::No);
        history.advance(
            with(sample(&links, &allied, &[7], &[2]), &[3, 4]),
            Continuity::Next,
        );
        assert_eq!(history.side(62, 0), Engaged::Yes);
    }

    /// A viewer change inside unobserved hours is not placed at the hour the
    /// samples resume: the sides stay unknown until they rebuild.
    #[test]
    fn a_viewer_change_across_a_gap_rebuilds_nothing() {
        let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
        let allied = relations(true);
        let mut history = History::default();
        history.advance(sample(&links, &allied, &[7], &[2]), Continuity::Load);
        let moved = |viewer| Sample {
            viewer,
            ..sample(&links, &allied, &[7], &[2])
        };
        history.advance(moved(2), Continuity::Gap);
        assert_eq!(history.side(62, 0), Engaged::Unknown);
        let mut next = History::default();
        next.advance(sample(&links, &allied, &[7], &[2]), Continuity::Load);
        next.advance(moved(2), Continuity::Next);
        assert_eq!(next.side(62, 0), Engaged::Yes);
    }

    /// A viewer change between samples refreshes every side (140BA9890, or
    /// 14113E4B0 through 140BABC00) at its own moment, never as a load. The
    /// side lists the old viewer (1); the new viewer (2) shares vision with it
    /// only while allied. Viewer change then alliance: the refresh saw no
    /// alliance and the alliance refreshes nothing, so native keeps No.
    /// Alliance then viewer change: the refresh sees it, Yes. In one interval
    /// the samples are the same for both orders.
    #[test]
    fn a_viewer_change_is_ordered_against_both_samples_relations() {
        let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
        let (apart, allied) = (relations(false), relations(true));
        fn at<'a>(
            links: &'a BTreeMap<u32, u32>,
            viewer: u32,
            relations: &'a Relations,
        ) -> Sample<'a> {
            Sample {
                viewer,
                ..sample(links, relations, &[7], &[1])
            }
        }
        let run = |hours: &[(u32, &Relations)]| {
            let mut history = History::default();
            history.advance(at(&links, 1, &apart), Continuity::Load);
            assert_eq!(history.side(62, 0), Engaged::Yes);
            for &(viewer, relations) in hours {
                history.advance(at(&links, viewer, relations), Continuity::Next);
            }
            history.side(62, 0)
        };
        // Apart samples: the viewer changed first.
        assert_eq!(run(&[(2, &apart), (2, &allied)]), Engaged::No);
        // Apart samples: the alliance came first.
        assert_eq!(run(&[(1, &allied), (2, &allied)]), Engaged::Yes);
        // One interval holds both, in an order the samples cannot tell.
        assert_eq!(run(&[(2, &allied)]), Engaged::Ambiguous(Ambiguity::Order));
        // And the other way: the alliance ends as the viewer changes.
        let mut ending = History::default();
        ending.advance(at(&links, 1, &allied), Continuity::Load);
        ending.advance(at(&links, 2, &apart), Continuity::Next);
        assert_eq!(ending.side(62, 0), Engaged::Ambiguous(Ambiguity::Order));
    }

    fn rosters(links: &BTreeMap<u32, u32>, players: &[(i32, u32)], count: i64) -> Controllers {
        Controllers {
            players: players
                .iter()
                .map(|&(key, raw)| (key, (raw, links.get(&raw).copied())))
                .collect(),
            counts: BTreeMap::from([(1, count)]),
        }
    }

    /// 14113E4B0 refreshes every combat through 140BABC00 when it gives any
    /// controller the local player's country. The hourly rosters can only
    /// allow that: a stale flag that the refresh would change is ambiguous,
    /// never asserted, and a roster change elsewhere refreshes nothing.
    #[test]
    fn a_controller_joining_the_viewer_may_refresh_a_stale_flag() {
        let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
        let (apart, allied) = (relations(false), relations(true));
        fn at<'a>(
            links: &'a BTreeMap<u32, u32>,
            relations: &'a Relations,
            controllers: Option<Controllers>,
        ) -> Sample<'a> {
            Sample {
                controllers,
                ..sample(links, relations, &[7], &[2])
            }
        }
        let away = Some(rosters(&links, &[(40, 5)], 1));
        let run = |relations: &Relations, controllers: Option<Controllers>| {
            let mut history = History::default();
            history.advance(at(&links, &apart, away.clone()), Continuity::Load);
            // The alliance alone refreshes nothing: the flag goes stale.
            history.advance(at(&links, relations, away.clone()), Continuity::Next);
            history.advance(at(&links, relations, controllers), Continuity::Next);
            history.side(62, 0)
        };
        let joined = Some(rosters(&links, &[(40, 1)], 2));
        assert_eq!(run(&allied, away.clone()), Engaged::No);
        assert_eq!(
            run(&allied, joined.clone()),
            Engaged::Ambiguous(Ambiguity::Scope)
        );
        // A new player key, or only a higher count, allows it too.
        for rosters in [
            rosters(&links, &[(40, 5), (41, 1)], 1),
            rosters(&links, &[(40, 5)], 2),
        ] {
            assert_eq!(
                run(&allied, Some(rosters)),
                Engaged::Ambiguous(Ambiguity::Scope)
            );
        }
        // Rosters that are not whole cannot rule it out.
        assert_eq!(run(&allied, None), Engaged::Ambiguous(Ambiguity::Scope));
        // A refresh that agrees with the held flag keeps it.
        assert_eq!(run(&apart, joined), Engaged::No);
        // A controller leaving for another country refreshes nothing.
        assert_eq!(
            run(&allied, Some(rosters(&links, &[(40, 2)], 1))),
            Engaged::No
        );
    }

    /// Raw reference 7 moves from country 2 (the viewer's ally) to country 3
    /// between two samples in which a unit joins. A refresh before the move
    /// resolved 7 as 2 (Yes), one after it as 3 (No): the samples cannot order
    /// them, so the flag is ambiguous, never the current links' No for both.
    /// A reference the previous links lack leaves that candidate unknown.
    #[test]
    fn each_candidate_resolves_references_through_its_own_sample_links() {
        let allied = relations(true);
        let before = BTreeMap::from([(1, 1), (5, 5), (7, 2)]);
        let after = BTreeMap::from([(1, 1), (5, 5), (7, 3)]);
        let mut history = History::default();
        history.advance(sample(&before, &allied, &[7], &[7]), Continuity::Load);
        assert_eq!(history.side(62, 0), Engaged::Yes);
        history.advance(sample(&after, &allied, &[7, 8], &[7]), Continuity::Next);
        assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Order));
        // The same links in both samples keep one answer.
        let mut steady = History::default();
        steady.advance(sample(&after, &allied, &[7], &[7]), Continuity::Load);
        steady.advance(sample(&after, &allied, &[7, 8], &[7]), Continuity::Next);
        assert_eq!(steady.side(62, 0), Engaged::No);
        // A reference the earlier links cannot place stays unresolved there.
        let unlinked = BTreeMap::from([(1, 1), (5, 5)]);
        let mut unplaced = History::default();
        unplaced.advance(sample(&unlinked, &allied, &[7], &[7]), Continuity::Load);
        unplaced.advance(sample(&after, &allied, &[7, 8], &[7]), Continuity::Next);
        assert_eq!(unplaced.side(62, 0), Engaged::Unknown);
    }

    /// Joining a faction does not engage a combat until a unit joins or
    /// leaves it; a combat that starts after an earlier sample rebuilds.
    #[test]
    fn history_lags_diplomacy_until_the_unit_set_changes() {
        let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
        let (apart, allied) = (relations(false), relations(true));
        let mut history = History::default();
        history.advance(sample(&links, &apart, &[7], &[2]), Continuity::Load);
        assert_eq!(history.combat(62), Engaged::No);
        history.advance(sample(&links, &allied, &[7], &[2]), Continuity::Next);
        assert_eq!(history.combat(62), Engaged::No);
        history.advance(sample(&links, &allied, &[7, 8], &[2]), Continuity::Next);
        assert_eq!(history.combat(62), Engaged::Yes);
        // A unit change in the hour the alliance ends cannot be ordered.
        history.advance(sample(&links, &apart, &[7], &[2]), Continuity::Next);
        assert_eq!(history.combat(62), Engaged::Ambiguous(Ambiguity::Order));
        history.advance(sample(&links, &apart, &[8], &[2]), Continuity::Next);
        assert_eq!(history.combat(62), Engaged::No);
        // After unobserved hours only a proven rebuild decides.
        history.advance(sample(&links, &apart, &[8], &[2]), Continuity::Gap);
        assert_eq!(history.combat(62), Engaged::Unknown);

        let mut started = History::default();
        let empty = Sample {
            sides: BTreeMap::new(),
            ..sample(&links, &allied, &[], &[])
        };
        started.advance(empty, Continuity::Load);
        started.advance(sample(&links, &allied, &[7], &[2]), Continuity::Next);
        assert_eq!(started.combat(62), Engaged::Yes);
    }
}
