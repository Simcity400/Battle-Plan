//! The recorded naval engaged byte as a check on its derivation.
//!
//! `NavalCombatSideDetails.engaged` is each side's `CCombatant+0xBA` as the
//! recording client held it: `UpdateCombatSharedVisionFlag@1413CEBE0` writes
//! vtable `+0xA0`). The engaged index ([`crate::engaged_index`]) folds a
//! history for that player beside the viewing perspective's, serves it
//! whenever the viewed country is that player, and compares it with the byte
//! ([`ByteCheck`]). It counts:
//!
//! - agreements and mismatches, side by side and hour by hour, naming the
//!   first mismatches' branch, hour, combat and side (a display sample; the
//!   counts stay exact however long the recording);
//! - side-hours the derivation leaves ambiguous or unknown: there the byte,
//!   and only for the recording's player, settles the flag
//!   ([`settle_recorded`]); no other viewer takes it;
//! - the coverage: hours folded, hours holding naval combat sides, hours with
//!   an incomplete naval census, hours whose player the recording does not
//!   name, unreadable hours and blocks, and sides without a recorded byte.
//!
//! LocalPlayer records the client's own playing and observer-view references
//! each hour. The same native choice as 1413CEBE0 resolves through CountryLinks;
//! multiplayer roster membership cannot identify this client's perspective.

use super::engaged::{self, Engaged, Side};
use crate::map::tables::rows;
use replay::{BranchId, Hour, frame::DecodedFrame, rows::tables};
use std::{collections::BTreeMap, io};

/// How many mismatches the published summary keeps and a line names.
const NAMED_MISMATCHES: usize = 5;

/// The client's stored viewpoint, including the native observer fallback.
pub(crate) fn recording_player(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
) -> io::Result<Option<u32>> {
    crate::map::allegiance::perspective(frame, links)
}

/// Each recorded naval combat's two stored bytes.
pub(crate) fn recorded_bytes(frame: &DecodedFrame) -> io::Result<BTreeMap<u64, [u8; 2]>> {
    Ok(rows::<tables::NavalCombatDetails>(frame)?
        .rows()
        .iter()
        .map(|row| (row.combat, row.sides.each_ref().map(|side| side.engaged)))
        .collect())
}

/// For the recording's own player, an undecided naval flag takes the
/// recorded bytes (`any_side_engaged@1413CD880` tests either side's byte for
/// nonzero); a decided flag and every other viewer keep the derivation.
pub(crate) fn settle_recorded(
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

/// An undecided flag of the recording's own player takes its combat's recorded
/// bytes (`any_side_engaged@1413CD880` tests either side's byte for nonzero);
/// a decided flag, or a combat without bytes, keeps the derivation.
pub(crate) fn settle(flag: Engaged, bytes: Option<&[u8; 2]>) -> Engaged {
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

/// One side-hour where the derivation decided against the recorded byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Mismatch {
    pub(crate) branch: BranchId,
    pub(crate) hour: Hour,
    pub(crate) combat: u64,
    pub(crate) side: u8,
    /// The derived flag; the byte is its opposite.
    pub(crate) derived: bool,
}

/// The counts the pass publishes: small and cheap to copy, whatever the
/// recording's length.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Summary {
    /// Side-hours whose derived Yes or No equals the recorded byte.
    pub(crate) agreements: u64,
    /// Side-hours whose derived Yes or No differs from it.
    pub(crate) mismatches: u64,
    /// The first [`NAMED_MISMATCHES`] of them.
    pub(crate) first: Vec<Mismatch>,
    /// Side-hours the derivation leaves ambiguous or unknown.
    pub(crate) undecided: u64,
    /// Side-hours whose combat has no recorded details row, so no byte.
    pub(crate) unrecorded: u64,
    /// Hours folded, decoded or not.
    pub(crate) hours: u64,
    /// Hours whose naval sides were read and held at least one.
    pub(crate) naval_hours: u64,
    /// Hours whose naval combat census is incomplete: sides beyond it went
    /// unseen.
    pub(crate) incomplete_hours: u64,
    /// Hours holding naval combat sides whose player the recording does not
    /// name: not compared.
    pub(crate) unidentified_hours: u64,
    /// Hours that did not decode, or whose sides could not be read.
    pub(crate) unreadable_hours: u64,
    /// Blocks that could not be read, whose hours were not folded.
    pub(crate) failed_blocks: u64,
    /// Every recorded hour is folded.
    pub(crate) complete: bool,
}

impl Summary {
    /// Every hour of the covered span was read, compared or proved free of
    /// naval combat.
    fn whole(&self) -> bool {
        self.unreadable_hours == 0
            && self.failed_blocks == 0
            && self.incomplete_hours == 0
            && self.unidentified_hours == 0
    }

    /// The diagnostics lines: a summary with its coverage, then the first
    /// mismatches by id.
    pub(crate) fn lines(&self) -> Vec<String> {
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

/// The pass's private check. Its memory is the [`Summary`] alone: exact
/// counts and at most [`NAMED_MISMATCHES`] named mismatches, whatever the
/// number of side-hours folded.
#[derive(Default)]
pub(crate) struct ByteCheck {
    pub(crate) summary: Summary,
}

impl ByteCheck {
    /// Compares one hour's naval sides, as the recording player's `history`
    /// holds them after the hour, with the recorded bytes.
    pub(crate) fn compare(
        &mut self,
        branch: BranchId,
        hour: Hour,
        frame: &DecodedFrame,
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
    use replay::{
        frame::{Metrics, TableView},
        rows::{CountryLinkRow, NavalCombatDetailsRow, PlayerRow, TableId, TableRows},
    };

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
            TableView::Complete(TableRows::LocalPlayer(vec![replay::rows::LocalPlayerRow {
                country: players.first().copied().unwrap_or(0),
                observer_country: 0,
            }]));
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
                TableView::Complete(TableRows::LocalPlayer(vec![replay::rows::LocalPlayerRow {
                    country,
                    observer_country,
                }]));
            assert_eq!(recording_player(&frame, &links).unwrap(), expected);
            frame.hour += 1;
        }
        *frame.tables[TableId::LocalPlayer.index()] = TableView::Unavailable {
            reason: "failed".into(),
        };
        assert_eq!(recording_player(&frame, &links).unwrap(), None);
    }

    /// An undecided flag takes the bytes for the recording player only; a
    /// decided one keeps the derivation even against them.
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

    /// The published summary names the first mismatches and counts the rest;
    /// a span with no naval side says so only when every hour was read.
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
