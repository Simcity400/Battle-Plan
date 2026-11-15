//! The frame a combat map icon keeps while its combat is unseen.
//!
//! Land and naval icons set their frame only for a combat the viewer sees.
//! The unseen path leaves it unchanged, showing its last seen update.
//!
//! An icon lives while both sides' country lists are nonempty and the combat
//! continues. Its per-type pool reuses released icons; release clears the
//! combat reference and position, preserving the frame. A new instance shows
//! its pool slot's last frame, which the recording does not hold.
//!
//! The same producers also erase every icon of a province that leaves the
//! view or the combat cutoff,
//! and the icon manager releases icons that fail projection.
//! The camera is not recorded. The viewer
//! assumes a camera that kept watching: an instance lives while its combat
//! lasts with both sides non-empty, and every frame carried from an earlier
//! hour is counted under that assumption ([`CARRIED`]).
//!
//! [`IconFold`] folds that per hour; the engaged index carries it over the
//! recording-wide icon index, so a seek shows the
//! same kept frame.
//!
//! When history establishes no frame, the viewer uses the
//! child's content-authored initial frame, with a named limitation. For land,
//! the icon defaults to frame 1 and reads its authored initial frame. This
//! is a presentation fallback, not an observation of the recording client's pool slot. The
//! limitation keeps the history state's reason, including a pending index.
//! Naval instead sets its button to frame 1 explicitly, overriding the
//! authored frame.

use crate::engagement_progress::sides as engagement_progress_sides;
use replay_data::rows::tables;
use replay_frame::frame::Tables;
use std::{collections::BTreeMap, io};
use {crate::engaged::Continuity, crate::engaged::Engaged};

/// The limitation every frame carried from an earlier hour reports.
pub const CARRIED: &str = "frame carried from the last seen hour, assuming the camera kept the icon alive (the camera and the icon pool are not recorded; a province's combat icons are erased when it leaves the view)";

/// The limitation for an unseen land icon drawn at its authored initial frame.
pub const LAND_NO_HISTORY: &str =
    "frame index not from history (the client land icon pool is not recorded)";

/// The limitation for an unseen naval icon drawn at its native binding's initial frame.
pub const NAVAL_NO_HISTORY: &str =
    "frame index not from history (the client naval icon pool is not recorded)";

/// What an unseen combat icon's frame is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kept {
    /// The zero-based frame this instance's last seen hour set.
    Frame(u32),
    /// A live instance no seen hour has set: its pool slot's frame.
    Fresh,
    /// The instance predates the branch's first folded hour, or follows
    /// hours the pass could not read.
    Unobserved,
    /// An hour of this instance could not decide whether the viewer saw the
    /// combat, the frame it set, or whether the instance lived on.
    Undecided,
    /// The index does not cover this hour for this viewer.
    Unindexed,
}

impl Kept {
    /// The carried-frame assumption or why the initial-frame fallback is needed.
    pub fn limitation(self, initial: &str) -> String {
        let reason = match self {
            Self::Frame(_) => return CARRIED.to_owned(),
            Self::Fresh => "live instance has no seen hour",
            Self::Unobserved => "instance predates observed history or follows unread hours",
            Self::Undecided => "an icon hour could not decide visibility, frame or continuity",
            Self::Unindexed => {
                "engaged index does not cover this viewer/hour (index pending or absent)"
            }
        };
        format!("{initial}: {reason}")
    }
}

/// One combat's icon inputs for one hour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IconInput {
    /// Both sides' country lists are non-empty; `None` when unrecorded.
    pub alive: Option<bool>,
    /// The viewer sees the combat; `None` when undecided.
    pub seen: Option<bool>,
    /// The zero-based frame a seen update sets; `None` when unknown.
    pub frame: Option<u32>,
}

/// One hour's icon inputs of one combat kind.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IconHour {
    pub combats: BTreeMap<u64, IconInput>,
    /// The combat table is whole: a combat missing from it has ended.
    pub complete: bool,
}

/// `seen` as the land marker reads the engaged flag.
pub fn engaged_seen(flag: Engaged) -> Option<bool> {
    match flag {
        Engaged::Yes => Some(true),
        Engaged::No => Some(false),
        Engaged::Ambiguous(_) | Engaged::Unknown => None,
    }
}

/// The land icons' hour: `alive` from both `LandCombatSides` primary lists
/// for `alive`, the engaged `flag` for `seen`, and the frame for a seen combat (frame 1
/// or 2 by the combat's indicator byte, zero-based here).
pub fn land_hour<F: Tables + ?Sized>(frame: &F, flag: &dyn Fn(u64) -> Engaged) -> io::Result<IconHour> {
    let sides = engagement_progress_sides(frame)?;
    let combats = crate::fold_tables::available::<tables::LandCombats, F>(frame)?;
    Ok(IconHour {
        combats: combats
            .rows()
            .iter()
            .map(|combat| {
                let row = sides.get(&combat.id);
                let input = IconInput {
                    alive: row.map(|row| {
                        !row.attacker_primary.is_empty() && !row.defender_primary.is_empty()
                    }),
                    seen: engaged_seen(flag(combat.id)),
                    frame: row.map(|row| u32::from(row.indicator != 0)),
                };
                (combat.id, input)
            })
            .collect(),
        complete: combats.is_complete(),
    })
}

/// The live icon instances of one combat kind and what each keeps.
#[derive(Clone, Debug, Default)]
pub struct IconFold {
    live: BTreeMap<u64, Kept>,
}

impl IconFold {
    /// Folds one hour. `changed` receives each combat whose state changed,
    /// `None` for an instance that ended.
    pub fn advance(
        &mut self,
        hour: &IconHour,
        continuity: Continuity,
        changed: &mut Vec<(u64, Option<Kept>)>,
    ) {
        changed.clear();
        let joined = continuity != Continuity::Next;
        let mut next = BTreeMap::new();
        for (&combat, input) in &hour.combats {
            let held = if joined {
                Some(Kept::Unobserved)
            } else {
                self.live.get(&combat).copied()
            };
            let instance = match input.alive {
                Some(false) => None,
                Some(true) => Some(held.unwrap_or(Kept::Fresh)),
                // A side may have emptied and refilled.
                None => Some(Kept::Undecided),
            };
            let Some(kept) = instance else {
                continue;
            };
            let kept = match input.seen {
                Some(true) => input.frame.map_or(Kept::Undecided, Kept::Frame),
                Some(false) => kept,
                None => Kept::Undecided,
            };
            next.insert(combat, kept);
        }
        if !hour.complete {
            // A combat the table may lack lives on unobserved.
            for &combat in self.live.keys() {
                next.entry(combat).or_insert(Kept::Undecided);
            }
        }
        for (&combat, &kept) in &next {
            if self.live.get(&combat) != Some(&kept) {
                changed.push((combat, Some(kept)));
            }
        }
        for &combat in self.live.keys() {
            if !next.contains_key(&combat) {
                changed.push((combat, None));
            }
        }
        self.live = next;
    }

    /// An hour the pass could not read: every live instance's history
    /// becomes unobserved.
    pub fn lose(&mut self, changed: &mut Vec<(u64, Option<Kept>)>) {
        changed.clear();
        for (&combat, kept) in &mut self.live {
            if *kept != Kept::Unobserved {
                *kept = Kept::Unobserved;
                changed.push((combat, Some(Kept::Unobserved)));
            }
        }
    }

    /// Every live instance after the last hour.
    pub fn live(&self) -> &BTreeMap<u64, Kept> {
        &self.live
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_frame_limitations_keep_each_history_states_reason() {
        for (kept, reason) in [
            (Kept::Fresh, "live instance has no seen hour"),
            (
                Kept::Unobserved,
                "instance predates observed history or follows unread hours",
            ),
            (
                Kept::Undecided,
                "an icon hour could not decide visibility, frame or continuity",
            ),
            (
                Kept::Unindexed,
                "engaged index does not cover this viewer/hour (index pending or absent)",
            ),
        ] {
            assert_eq!(
                kept.limitation(LAND_NO_HISTORY),
                format!("{LAND_NO_HISTORY}: {reason}")
            );
            assert_eq!(
                kept.limitation(NAVAL_NO_HISTORY),
                format!("{NAVAL_NO_HISTORY}: {reason}")
            );
        }
        assert_eq!(Kept::Frame(1).limitation(LAND_NO_HISTORY), CARRIED);
    }

    /// (combat, alive, seen, frame).
    type Row = (u64, Option<bool>, Option<bool>, Option<u32>);

    fn hour(combats: &[Row]) -> IconHour {
        IconHour {
            combats: combats
                .iter()
                .map(|&(combat, alive, seen, frame)| (combat, IconInput { alive, seen, frame }))
                .collect(),
            complete: true,
        }
    }

    /// Seen then unseen keeps the seen frame; a new instance, a side
    /// emptied and refilled, and a joined branch each require the authored fallback.
    #[test]
    fn an_unseen_icon_keeps_its_instance_last_seen_frame() {
        let mut fold = IconFold::default();
        let mut changed = Vec::new();
        let mut step = |fold: &mut IconFold, combats, continuity| {
            fold.advance(&hour(combats), continuity, &mut changed);
            fold.live().clone()
        };
        // Joined a running game: unobserved until seen.
        let live = step(
            &mut fold,
            &[(1, Some(true), Some(false), Some(0))],
            Continuity::Gap,
        );
        assert_eq!(live[&1], Kept::Unobserved);
        let live = step(
            &mut fold,
            &[(1, Some(true), Some(true), Some(1))],
            Continuity::Next,
        );
        assert_eq!(live[&1], Kept::Frame(1));
        let live = step(
            &mut fold,
            &[(1, Some(true), Some(false), Some(0))],
            Continuity::Next,
        );
        assert_eq!(live[&1], Kept::Frame(1));
        // A new combat created while unseen: its pool slot is unknown.
        let live = step(
            &mut fold,
            &[
                (1, Some(true), Some(false), None),
                (2, Some(true), Some(false), None),
            ],
            Continuity::Next,
        );
        assert_eq!(live[&2], Kept::Fresh);
        // A side empties: no icon; refilled: a new instance.
        let live = step(
            &mut fold,
            &[(1, Some(false), Some(false), None)],
            Continuity::Next,
        );
        assert!(!live.contains_key(&1));
        let live = step(
            &mut fold,
            &[(1, Some(true), Some(false), None)],
            Continuity::Next,
        );
        assert_eq!(live[&1], Kept::Fresh);
        // An undecided hour needs the authored fallback until the next seen one.
        let live = step(&mut fold, &[(1, Some(true), None, None)], Continuity::Next);
        assert_eq!(live[&1], Kept::Undecided);
        let live = step(&mut fold, &[(1, None, Some(false), None)], Continuity::Next);
        assert_eq!(live[&1], Kept::Undecided);
        let live = step(
            &mut fold,
            &[(1, Some(true), Some(true), Some(0))],
            Continuity::Next,
        );
        assert_eq!(live[&1], Kept::Frame(0));
    }
}
