//! The naval hover source trees against the diagnostic oracle.
//!
//! A recording from a collector built with `naval_tree_oracle` holds, beside
//! NavalModifierTrees, one NavalModifierTreeSamples row per source occurrence
//! a live combat reached: the native tree read afresh at that hour through
//! the occurrence's own accumulator. Playback joins each occurrence to its
//! trees by recorded identities ([`replay::naval_trees`]); this check makes
//! that join from NavalCombatDetails and NavalCombatShips, as the viewer
//! does, and requires the identity the capture filed and the trees the
//! recording holds for it (carried or read) to equal the fresh read.
//! Recordings without samples report that no oracle was captured.

use replay::{
    frame::DecodedFrame,
    naval_trees::{self, Owner, SideSource, Trees},
    rows::{NavalModifierTreeSampleRow, tables},
};

/// Mismatches listed by name; all of them are counted.
const LISTED: usize = 40;

#[derive(Default)]
pub(super) struct Checker {
    hours: u64,
    /// Hours whose samples were not recorded (production builds), by reason.
    unsampled: std::collections::BTreeMap<String, u64>,
    /// Hours with samples whose trees or combat tables are incomplete.
    incomplete: u64,
    samples: u64,
    equal: u64,
    /// Samples whose playback join names another identity.
    identity_mismatches: u64,
    /// Samples whose recorded trees differ from the fresh read.
    tree_mismatches: u64,
    /// Samples whose occurrence playback cannot find.
    unjoined: u64,
    listed: Vec<String>,
}

impl Checker {
    pub(super) fn observe(&mut self, frame: &DecodedFrame) {
        self.hours += 1;
        let samples = match frame.complete::<tables::NavalModifierTreeSamples>() {
            Ok(samples) => samples,
            Err(error) => {
                *self.unsampled.entry(error.to_string()).or_default() += 1;
                self.incomplete += 1;
                return;
            }
        };
        if samples.is_empty() {
            *self
                .unsampled
                .entry("no diagnostic samples recorded".into())
                .or_default() += 1;
            return;
        }
        let (Ok(rows), Ok(details), Ok(ships)) = (
            frame.complete::<tables::NavalModifierTrees>(),
            frame.complete::<tables::NavalCombatDetails>(),
            frame.complete::<tables::NavalCombatShips>(),
        ) else {
            self.incomplete += 1;
            return;
        };
        let Some(trees) = Trees::new(rows) else {
            self.incomplete += 1;
            return;
        };
        for sample in samples {
            self.samples += 1;
            let joined = join(sample, details, ships);
            let Some(owner) = joined else {
                self.unjoined += 1;
                self.list(frame.hour, sample, "playback cannot find the occurrence");
                continue;
            };
            if (owner.kind as u8, owner.owner, owner.sub) != (sample.kind, sample.owner, sample.sub)
            {
                self.identity_mismatches += 1;
                self.list(
                    frame.hour,
                    sample,
                    &format!("playback joins {owner:?}, the capture filed the source elsewhere"),
                );
                continue;
            }
            let recorded = trees
                .row(owner)
                .map(|row| row.trees.as_slice())
                .unwrap_or_default();
            if recorded == sample.trees.as_slice() {
                self.equal += 1;
            } else {
                self.tree_mismatches += 1;
                self.list(
                    frame.hour,
                    sample,
                    &format!(
                        "recorded trees for keys {:?} differ from the fresh read for keys {:?}",
                        recorded.iter().map(|tree| tree.key).collect::<Vec<_>>(),
                        sample.trees.iter().map(|tree| tree.key).collect::<Vec<_>>()
                    ),
                );
            }
        }
    }

    fn list(&mut self, hour: i32, sample: &NavalModifierTreeSampleRow, what: &str) {
        if self.listed.len() < LISTED {
            self.listed.push(format!(
                "hour {hour}, combat {:#x} side {} slot {} source {} (scope {}): {what}",
                sample.combat, sample.side, sample.battle_slot, sample.ordinal, sample.scope
            ));
        }
    }

    pub(super) fn failures(&self) -> [u64; 2] {
        [
            self.identity_mismatches + self.tree_mismatches,
            self.incomplete + self.unjoined,
        ]
    }

    pub(super) fn print(&self) {
        println!(
            "NavalModifierTrees oracle: {} hours, {} samples, {} equal, {} identity mismatches, {} tree mismatches, {} unjoined, {} hours with incomplete tables",
            self.hours,
            self.samples,
            self.equal,
            self.identity_mismatches,
            self.tree_mismatches,
            self.unjoined,
            self.incomplete
        );
        for (reason, hours) in &self.unsampled {
            println!("  hours without samples ({reason}): {hours}");
        }
        for line in &self.listed {
            println!("  mismatch: {line}");
        }
    }

    #[cfg(test)]
    fn counts(&self) -> [u64; 5] {
        [
            self.samples,
            self.equal,
            self.identity_mismatches,
            self.tree_mismatches,
            self.unjoined,
        ]
    }
}

/// The identity playback joins for one sampled occurrence.
fn join(
    sample: &NavalModifierTreeSampleRow,
    details: &[replay::rows::NavalCombatDetailsRow],
    ships: &[replay::rows::NavalCombatShipRow],
) -> Option<Owner> {
    let side = usize::from(sample.side);
    if sample.battle_slot == u32::MAX {
        let detail = details
            .iter()
            .find(|detail| detail.combat == sample.combat)?;
        let source = SideSource::from_ordinal(sample.ordinal)?;
        return naval_trees::side_source(
            detail.combat,
            detail.sides.get(side)?,
            detail.sides.get(1 - side.min(1))?,
            source,
        );
    }
    let ship = ships.iter().find(|ship| {
        ship.combat == sample.combat
            && usize::from(ship.side) == side
            && ship.battle_slot == sample.battle_slot
    })?;
    let source = ship
        .weapon_context
        .sources
        .get(usize::from(sample.ordinal))?;
    naval_trees::ship_source(ship.ship, &ship.weapon_context, source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::{
        frame::{Metrics, TableView},
        naval_trees::Kind,
        rows::{
            ModifierBreakdownNode, NavalCombatDetailsRow, NavalCombatShipRow, NavalModifierKeyTree,
            NavalModifierTreeRow, NavalShipModifierSource, TableId, TableRows,
        },
    };

    fn trees(value: i64) -> Vec<NavalModifierKeyTree> {
        vec![NavalModifierKeyTree {
            key: 0x5b,
            nodes: vec![ModifierBreakdownNode {
                children: 0,
                value,
                name: b"Doctrine".to_vec(),
            }],
        }]
    }

    fn frame(recorded: i64, sampled: i64, kind: Kind) -> DecodedFrame {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 5,
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
            }),
            metrics: Metrics::default(),
        };
        let mut detail = NavalCombatDetailsRow {
            combat: 42,
            ..Default::default()
        };
        detail.sides[0].summary.countries = vec![7];
        let ship = NavalCombatShipRow {
            combat: 42,
            side: 0,
            battle_slot: 3,
            ship: 41,
            weapon_context: replay::rows::NavalShipWeaponContext {
                country: 7,
                sources: vec![NavalShipModifierSource {
                    scope: 0,
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..Default::default()
        };
        *frame.tables[TableId::NavalCombatDetails.index()] =
            TableView::Complete(TableRows::NavalCombatDetails(vec![detail]));
        *frame.tables[TableId::NavalCombatShips.index()] =
            TableView::Complete(TableRows::NavalCombatShips(vec![ship]));
        *frame.tables[TableId::NavalModifierTrees.index()] =
            TableView::Complete(TableRows::NavalModifierTrees(vec![NavalModifierTreeRow {
                kind: Kind::Country as u8,
                owner: 7,
                sub: 0,
                trees: trees(recorded),
            }]));
        *frame.tables[TableId::NavalModifierTreeSamples.index()] =
            TableView::Complete(TableRows::NavalModifierTreeSamples(vec![
                NavalModifierTreeSampleRow {
                    combat: 42,
                    side: 0,
                    battle_slot: 3,
                    ordinal: 0,
                    scope: 0,
                    kind: kind as u8,
                    owner: 7,
                    sub: 0,
                    trees: trees(sampled),
                },
                NavalModifierTreeSampleRow {
                    combat: 42,
                    side: 0,
                    battle_slot: u32::MAX,
                    ordinal: SideSource::Country as u16,
                    scope: u8::MAX,
                    kind: kind as u8,
                    owner: 7,
                    sub: 0,
                    trees: trees(sampled),
                },
            ]));
        frame
    }

    #[test]
    fn equal_joins_and_trees_pass_and_differences_are_named() {
        let mut checker = Checker::default();
        checker.observe(&frame(5_000, 5_000, Kind::Country));
        assert_eq!(checker.counts(), [2, 2, 0, 0, 0]);
        assert_eq!(checker.failures(), [0, 0]);
        // A carried tree that drifted from the fresh read is a mismatch.
        checker.observe(&frame(5_000, 6_000, Kind::Country));
        assert_eq!(checker.counts(), [4, 2, 0, 2, 0]);
        // A capture that filed the source under another kind is named.
        checker.observe(&frame(5_000, 5_000, Kind::Commander));
        assert_eq!(checker.counts(), [6, 2, 2, 2, 0]);
        assert_eq!(checker.failures(), [4, 0]);
        assert_eq!(checker.listed.len(), 4);
        let mut missing = frame(5_000, 5_000, Kind::Country);
        *missing.tables[TableId::NavalCombatShips.index()] = TableView::Unavailable {
            reason: "sampled occurrence has no ship input".into(),
        };
        checker.observe(&missing);
        assert_eq!(checker.failures(), [4, 1]);
        let mut unjoined = frame(5_000, 5_000, Kind::Country);
        *unjoined.tables[TableId::NavalCombatShips.index()] =
            TableView::Complete(TableRows::NavalCombatShips(Vec::new()));
        checker.observe(&unjoined);
        assert_eq!(checker.failures(), [4, 2]);
    }

    #[test]
    fn a_production_recording_reports_no_samples() {
        let mut frame = frame(5_000, 5_000, Kind::Country);
        *frame.tables[TableId::NavalModifierTreeSamples.index()] =
            TableView::Complete(TableRows::NavalModifierTreeSamples(Vec::new()));
        *frame.tables[TableId::NavalModifierTrees.index()] = TableView::Unavailable {
            reason: "no trees compared without diagnostic samples".into(),
        };
        let mut checker = Checker::default();
        checker.observe(&frame);
        assert_eq!(checker.counts(), [0; 5]);
        assert_eq!(checker.unsampled.values().sum::<u64>(), 1);
        assert_eq!(checker.failures(), [0, 0]);
    }
}
