
use super::engaged::{self, Engaged, Side};
use replay_data::{BranchId, Hour, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io};

const NAMED_MISMATCHES: usize = 5;

pub fn recording_player(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
) -> io::Result<Option<u32>> {
    viewer_foundation::allegiance::perspective(frame, links)
}

pub fn recorded_bytes<F: replay_frame::frame::Tables + ?Sized>(frame: &F) -> io::Result<BTreeMap<u64, [u8; 2]>> {
    Ok(viewer_foundation::fold_tables::available::<tables::NavalCombatDetails, F>(frame)?
        .rows()
        .iter()
        .map(|row| (row.combat, row.sides.each_ref().map(|side| side.engaged)))
        .collect())
}

pub fn settle_recorded(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
    viewer: u32,
    naval: &mut BTreeMap<u64, Engaged>,
) -> io::Result<()> {
    if !naval
        .values()
        .any(|flag| matches!(flag, Engaged::Ambiguous(_) | Engaged::Unknown))
        || recording_player(frame, links)? != Some(viewer)
    {
        return Ok(());
    }
    let bytes = recorded_bytes(frame)?;
    for (combat, flag) in naval.iter_mut() {
        *flag = settle(*flag, bytes.get(combat));
    }
    Ok(())
}

pub fn settle(flag: Engaged, bytes: Option<&[u8; 2]>) -> Engaged {
    match (flag, bytes) {
        (Engaged::Ambiguous(_) | Engaged::Unknown, Some(bytes)) => {
            if bytes.iter().any(|&byte| byte != 0) {
                Engaged::Yes
            } else {
                Engaged::No
            }
        }
        _ => flag,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mismatch {
    pub branch: BranchId,
    pub hour: Hour,
    pub combat: u64,
    pub side: u8,
    pub derived: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub agreements: u64,
    pub mismatches: u64,
    pub first: Vec<Mismatch>,
    pub undecided: u64,
    pub(crate) unrecorded: u64,
    pub hours: u64,
    pub naval_hours: u64,
    pub incomplete_hours: u64,
    pub unidentified_hours: u64,
    pub unreadable_hours: u64,
    pub failed_blocks: u64,
    pub complete: bool,
}

impl Summary {
    fn whole(&self) -> bool {
        self.unreadable_hours == 0
            && self.failed_blocks == 0
            && self.incomplete_hours == 0
            && self.unidentified_hours == 0
    }

    pub fn lines(&self) -> Vec<String> {
        if self.hours == 0 && self.failed_blocks == 0 {
            return Vec::new();
        }
        let so_far = if self.complete { "" } else { " (so far)" };
        if self.naval_hours == 0 && self.whole() {
            return vec![format!(
                "Naval engaged check{so_far}: no naval combat side in the {} folded hours.",
                self.hours
            )];
        }
        let mut lines = vec![format!(
            "Naval engaged check{so_far}: the recorded byte against the flag derived for the recording's player: {} side-hours agree, {} differ, {} undecided by the derivation (the byte decides those for that player only), {} sides without a recorded byte.",
            self.agreements, self.mismatches, self.undecided, self.unrecorded,
        )];
        lines.push(format!(
            "Naval engaged check coverage{so_far}: {} hours folded, {} with naval combat sides; not compared: {} hours whose player the recording does not name, {} unreadable hours, {} unreadable blocks; {} hours with an incomplete naval combat census.",
            self.hours,
            self.naval_hours,
            self.unidentified_hours,
            self.unreadable_hours,
            self.failed_blocks,
            self.incomplete_hours,
        ));
        for mismatch in &self.first {
            lines.push(format!(
                "Naval engaged mismatch: branch {} hour {} combat type={} id={} side {}: derived {}, recorded {}.",
                mismatch.branch,
                mismatch.hour,
                mismatch.combat as u32,
                mismatch.combat >> 32,
                mismatch.side,
                if mismatch.derived { "engaged" } else { "not engaged" },
                if mismatch.derived { "not engaged" } else { "engaged" },
            ));
        }
        if let Some(more) = (self.mismatches as usize).checked_sub(self.first.len())
            && more > 0
        {
            lines.push(format!("Naval engaged mismatch: {more} more not listed."));
        }
        lines
    }
}

#[derive(Default)]
pub struct ByteCheck {
    pub summary: Summary,
}

impl ByteCheck {
    pub fn compare<F: replay_frame::frame::Tables + ?Sized>(
        &mut self,
        branch: BranchId,
        hour: Hour,
        frame: &F,
        sides: &BTreeMap<(u64, u8), Side>,
        history: &engaged::History,
    ) -> io::Result<()> {
        let bytes = recorded_bytes(frame)?;
        let summary = &mut self.summary;
        for &(combat, side) in sides.keys() {
            let Some(byte) = bytes.get(&combat).map(|bytes| bytes[usize::from(side)]) else {
                summary.unrecorded += 1;
                continue;
            };
            let derived = match history.side(combat, side) {
                Engaged::Yes => true,
                Engaged::No => false,
                Engaged::Ambiguous(_) | Engaged::Unknown => {
                    summary.undecided += 1;
                    continue;
                }
            };
            if derived == (byte != 0) {
                summary.agreements += 1;
            } else {
                summary.mismatches += 1;
                if summary.first.len() < NAMED_MISMATCHES {
                    summary.first.push(Mismatch {
                        branch,
                        hour,
                        combat,
                        side,
                        derived,
                    });
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{
        rows::CountryLinkRow, rows::NavalCombatDetailsRow, rows::PlayerRow, rows::TableId,
        rows::TableRows,
    };
    use replay_frame::{frame::Metrics, frame::TableView};

    fn frame(players: &[u32], bytes: [u8; 2]) -> DecodedFrame {
        let mut frame = DecodedFrame {
            branch: 3,
            hour: 10,
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "not needed".into(),
                }
                .into()
            }),
            metrics: Metrics::default(),
        };
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(
                [1, 2, 5]
                    .map(|id| CountryLinkRow {
                        id,
                        canonical_slot: id,
                    })
                    .to_vec(),
            ));
        *frame.tables[TableId::Players.index()] = TableView::Complete(TableRows::Players(
            players
                .iter()
                .enumerate()
                .map(|(id, &country)| PlayerRow {
                    id: id as i32,
                    country,
                    ..Default::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::LocalPlayer.index()] =
            TableView::Complete(TableRows::LocalPlayer(vec![
                replay_data::rows::LocalPlayerRow {
                    country: players.first().copied().unwrap_or(0),
                    observer_country: 0,
                },
            ]));
        let mut detail = NavalCombatDetailsRow {
            combat: 62,
            ..Default::default()
        };
        detail.sides[0].engaged = bytes[0];
        detail.sides[1].engaged = bytes[1];
        *frame.tables[TableId::NavalCombatDetails.index()] =
            TableView::Complete(TableRows::NavalCombatDetails(vec![detail]));
        frame
    }

    #[test]
    fn mp_and_observer_viewpoints_are_named_independently_of_the_roster() {
        let links = BTreeMap::from([(101, 1), (102, 2), (105, 5)]);
        let mut frame = frame(&[1, 2, 5], [0; 2]);
        for (country, observer_country, expected) in [
            (105, 0, Some(5)),
            (102, 0, Some(2)),
            (0, 101, Some(1)),
            (0, 0, None),
            (109, 105, None),
        ] {
            *frame.tables[TableId::LocalPlayer.index()] =
                TableView::Complete(TableRows::LocalPlayer(vec![
                    replay_data::rows::LocalPlayerRow {
                        country,
                        observer_country,
                    },
                ]));
            assert_eq!(recording_player(&frame, &links).unwrap(), expected);
            frame.hour += 1;
        }
        *frame.tables[TableId::LocalPlayer.index()] = TableView::Unavailable {
            reason: "failed".into(),
        };
        assert_eq!(recording_player(&frame, &links).unwrap(), None);
    }

    #[test]
    fn the_byte_settles_only_the_recording_players_undecided_flags() {
        let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
        let frame = frame(&[1], [0, 1]);
        let settled = |viewer, flag| {
            let mut naval = BTreeMap::from([(62, flag)]);
            settle_recorded(&frame, &links, viewer, &mut naval).unwrap();
            naval[&62]
        };
        assert_eq!(settled(1, Engaged::Unknown), Engaged::Yes);
        assert_eq!(
            settled(1, Engaged::Ambiguous(engaged::Ambiguity::Scope)),
            Engaged::Yes
        );
        assert_eq!(settled(1, Engaged::No), Engaged::No);
        assert_eq!(settled(5, Engaged::Unknown), Engaged::Unknown);
    }

    #[test]
    fn the_summary_names_the_first_mismatches_and_its_coverage() {
        let mut summary = Summary {
            hours: 12,
            naval_hours: 10,
            agreements: 30,
            mismatches: 7,
            first: (0..NAMED_MISMATCHES as i32)
                .map(|hour| Mismatch {
                    branch: 3,
                    hour,
                    combat: 62,
                    side: 1,
                    derived: false,
                })
                .collect(),
            unreadable_hours: 2,
            complete: true,
            ..Default::default()
        };
        let lines = summary.lines();
        assert!(
            lines[0].contains("30 side-hours agree, 7 differ"),
            "{lines:?}"
        );
        assert!(lines[1].contains("12 hours folded, 10 with naval combat sides"));
        assert!(lines[1].contains("2 unreadable hours"));
        assert!(lines[2].contains("branch 3 hour 0 combat type=62 id=0 side 1"));
        assert_eq!(
            lines.last().unwrap(),
            "Naval engaged mismatch: 2 more not listed."
        );
        summary = Summary {
            hours: 4,
            complete: true,
            ..Default::default()
        };
        assert_eq!(
            summary.lines(),
            ["Naval engaged check: no naval combat side in the 4 folded hours."]
        );
        summary.failed_blocks = 1;
        assert!(summary.lines()[1].contains("1 unreadable blocks"));
    }
}
