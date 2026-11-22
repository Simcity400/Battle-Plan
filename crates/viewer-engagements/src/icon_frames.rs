
use crate::engagement_progress::sides as engagement_progress_sides;
use replay_data::rows::tables;
use replay_frame::frame::Tables;
use std::{collections::BTreeMap, io};
use {crate::engaged::Continuity, crate::engaged::Engaged};

pub const CARRIED: &str = "frame carried from the last seen hour, assuming the camera kept the icon alive (the camera and the icon pool are not recorded; a province's combat icons are erased when it leaves the view)";

pub const LAND_NO_HISTORY: &str =
    "frame index not from history (the client land icon pool is not recorded)";

pub const NAVAL_NO_HISTORY: &str =
    "frame index not from history (the client naval icon pool is not recorded)";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kept {
    Frame(u32),
    Fresh,
    Unobserved,
    Undecided,
    Unindexed,
}

impl Kept {
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IconInput {
    pub alive: Option<bool>,
    pub seen: Option<bool>,
    pub frame: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IconHour {
    pub combats: BTreeMap<u64, IconInput>,
    pub complete: bool,
}

pub fn engaged_seen(flag: Engaged) -> Option<bool> {
    match flag {
        Engaged::Yes => Some(true),
        Engaged::No => Some(false),
        Engaged::Ambiguous(_) | Engaged::Unknown => None,
    }
}

pub fn land_hour<F: Tables + ?Sized>(frame: &F, flag: &dyn Fn(u64) -> Engaged) -> io::Result<IconHour> {
    let sides = engagement_progress_sides(frame)?;
    let combats = viewer_foundation::fold_tables::available::<tables::LandCombats, F>(frame)?;
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

#[derive(Clone, Debug, Default)]
pub struct IconFold {
    live: BTreeMap<u64, Kept>,
}

impl IconFold {
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

    pub fn lose(&mut self, changed: &mut Vec<(u64, Option<Kept>)>) {
        changed.clear();
        for (&combat, kept) in &mut self.live {
            if *kept != Kept::Unobserved {
                *kept = Kept::Unobserved;
                changed.push((combat, Some(Kept::Unobserved)));
            }
        }
    }

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

    #[test]
    fn an_unseen_icon_keeps_its_instance_last_seen_frame() {
        let mut fold = IconFold::default();
        let mut changed = Vec::new();
        let mut step = |fold: &mut IconFold, combats, continuity| {
            fold.advance(&hour(combats), continuity, &mut changed);
            fold.live().clone()
        };
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
        let live = step(
            &mut fold,
            &[
                (1, Some(true), Some(false), None),
                (2, Some(true), Some(false), None),
            ],
            Continuity::Next,
        );
        assert_eq!(live[&2], Kept::Fresh);
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
