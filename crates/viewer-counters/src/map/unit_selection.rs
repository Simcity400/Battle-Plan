//! Unit selection belongs to one viewer, never to the recording.

use std::{collections::BTreeSet, sync::Arc};
use viewer_foundation::map::counter_hovers::unit_hits::Hit;
#[cfg(test)]
use viewer_foundation::map::counter_hovers::unit_hits::Kind;
use viewer_foundation::unit_handle::Unit;

use replay_data::{BranchId, rows::tables};
use replay_frame::frame::DecodedFrame;

use viewer_foundation::map::perspective::Country;

/// The game checks selection before owner and
/// coarse grouping. Splitting keeps each selected unit on its own plate.
pub fn joins(selected: &BTreeSet<Unit>, first: Unit, candidate: Unit, split: bool) -> bool {
    let first = selected.contains(&first);
    let candidate = selected.contains(&candidate);
    first == candidate && !(split && first)
}

#[derive(Default)]
pub struct Selection {
    context: Option<(BranchId, Country, u32)>,
    units: Arc<BTreeSet<Unit>>,
    ordered: Arc<Vec<Unit>>,
}

impl Selection {
    pub fn units(&self) -> &Arc<BTreeSet<Unit>> {
        &self.units
    }

    /// Native tooltip dispatch and first-member queries follow insertion order.
    pub fn ordered_units(&self) -> &Arc<Vec<Unit>> {
        &self.ordered
    }

    pub fn clear(&mut self) -> bool {
        if self.units.is_empty() && self.ordered.is_empty() {
            return false;
        }
        self.units = Arc::default();
        self.ordered = Arc::default();
        true
    }

    /// A normal click replaces selection; Shift-click toggles each member of
    /// the clicked plate. Clicking the background clears unless Shift is held.
    pub fn click(&mut self, hit: Option<&Hit>, shift: bool) -> bool {
        let hit = hit.filter(|hit| hit.ours);
        let before = Arc::clone(&self.units);
        let before_order = Arc::clone(&self.ordered);
        if let Some(hit) = hit {
            let units = Arc::make_mut(&mut self.units);
            let ordered = Arc::make_mut(&mut self.ordered);
            if !shift {
                units.clear();
                ordered.clear();
            }
            for unit in hit.units() {
                if shift && units.remove(&unit) {
                    ordered.retain(|selected| *selected != unit);
                } else if units.insert(unit) {
                    ordered.push(unit);
                }
            }
        } else if !shift {
            self.clear();
        }
        let changed = *before != *self.units || *before_order != *self.ordered;
        if !changed {
            // Keep held tooltip identities for a click which changed neither view.
            self.units = before;
            self.ordered = before_order;
        }
        changed
    }

    /// A new branch, country or viewer clears the selection; it reads no rows.
    pub fn context(&mut self, branch: BranchId, country: &Country, viewer: u32) -> bool {
        let context = (branch, country.clone(), viewer);
        if self.context.as_ref() == Some(&context) {
            return false;
        }
        self.context = Some(context);
        self.clear()
    }

    /// Selection survives roster, logical and expeditionary ownership changes.
    /// Complete censuses can retire missing handles; an absent or partial class
    /// census cannot prove disappearance, so it preserves that class. Ownership
    /// admits a new click but does not invalidate an already-selected handle.
    pub fn observe(
        &mut self,
        branch: BranchId,
        country: &Country,
        viewer: u32,
        frame: &DecodedFrame,
    ) -> bool {
        let mut changed = self.context(branch, country, viewer);
        if self.units.is_empty() {
            return changed;
        }
        let divisions = frame.complete::<tables::Divisions>().ok();
        let guns = frame.complete::<tables::RailwayGunCounterInputs>().ok();
        let forces = frame.complete::<tables::TaskForces>().ok();
        let present =
            |unit: &Unit| match unit {
                Unit::Division(handle) => divisions
                    .is_none_or(|rows| rows.binary_search_by_key(handle, |row| row.id).is_ok()),
                Unit::RailwayGun(handle) => {
                    guns.is_none_or(|rows| rows.binary_search_by_key(handle, |row| row.gun).is_ok())
                }
                Unit::TaskForce(handle) => forces
                    .is_none_or(|rows| rows.binary_search_by_key(handle, |row| row.id).is_ok()),
            };
        if self.units.iter().any(|unit| !present(unit)) {
            Arc::make_mut(&mut self.units).retain(present);
            Arc::make_mut(&mut self.ordered).retain(present);
            changed = true;
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{
        rows::CountryLinkRow, rows::DivisionRow, rows::RailwayGunCounterInputRow, rows::TableRows,
        rows::TaskForceRow,
    };
    use viewer_foundation::map::test_support::{test_frame, test_put as put};

    fn hit(kind: Kind, ids: &[u64]) -> Hit {
        Hit {
            ours: true,
            port: false,
            rect: [1.0, 2.0, 3.0, 4.0],
            kind,
            members: ids.into(),
            layer: 0,
            plate: None,
        }
    }

    fn ownership_frame(owners: &[(u64, u32, u32, u32)]) -> DecodedFrame {
        let mut frame = test_frame(42);
        put(
            &mut frame,
            TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 1,
                    canonical_slot: 2,
                },
                CountryLinkRow {
                    id: 101,
                    canonical_slot: 1,
                },
                CountryLinkRow {
                    id: 102,
                    canonical_slot: 1,
                },
                CountryLinkRow {
                    id: 201,
                    canonical_slot: 2,
                },
            ]),
        );
        put(
            &mut frame,
            TableRows::Divisions(
                owners
                    .iter()
                    .map(|&(id, roster, logical, expeditionary)| DivisionRow {
                        id,
                        roster_owner: roster,
                        logical_owner: logical,
                        expeditionary_owner: expeditionary,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        put(
            &mut frame,
            TableRows::RailwayGunCounterInputs(
                owners
                    .iter()
                    .map(
                        |&(gun, roster, logical, expeditionary)| RailwayGunCounterInputRow {
                            gun,
                            roster_owner: roster,
                            logical_owner: logical,
                            expeditionary_owner: expeditionary,
                            ..Default::default()
                        },
                    )
                    .collect(),
            ),
        );
        put(
            &mut frame,
            TableRows::TaskForces(
                owners
                    .iter()
                    .map(|&(id, roster, logical, expeditionary)| TaskForceRow {
                        id,
                        roster_owner: roster,
                        logical_owner: logical,
                        expeditionary_owner: expeditionary,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        frame
    }

    #[test]
    fn click_replaces_and_shift_toggles_typed_members() {
        let mut selection = Selection::default();
        let division = hit(Kind::Division, &[7, 8]);
        let gun = hit(Kind::RailwayGun, &[7]);
        assert!(selection.click(Some(&division), false));
        assert!(selection.click(Some(&gun), true));
        assert_eq!(
            selection.units().iter().copied().collect::<Vec<_>>(),
            [Unit::Division(7), Unit::Division(8), Unit::RailwayGun(7)]
        );
        assert_eq!(
            selection.ordered_units().as_slice(),
            [Unit::Division(7), Unit::Division(8), Unit::RailwayGun(7)]
        );
        assert!(selection.click(Some(&division), true));
        assert_eq!(
            selection.units().iter().copied().collect::<Vec<_>>(),
            [Unit::RailwayGun(7)]
        );
        assert_eq!(selection.ordered_units().as_slice(), [Unit::RailwayGun(7)]);
        assert!(!selection.click(None, true));
        assert!(selection.click(None, false));
        assert!(selection.units().is_empty());
        assert!(selection.ordered_units().is_empty());
    }

    #[test]
    fn normal_click_reordering_changes_tooltip_identity_without_changing_membership() {
        let mut selection = Selection::default();
        assert!(selection.click(Some(&hit(Kind::Division, &[8, 7, 8])), false));
        assert_eq!(
            selection.ordered_units().as_slice(),
            [Unit::Division(8), Unit::Division(7)]
        );
        let held = Arc::clone(selection.ordered_units());
        let members = Arc::clone(selection.units());
        assert!(!selection.click(Some(&hit(Kind::Division, &[8, 7])), false));
        assert!(Arc::ptr_eq(&held, selection.ordered_units()));
        assert!(Arc::ptr_eq(&members, selection.units()));
        assert!(selection.click(Some(&hit(Kind::Division, &[7, 8])), false));
        assert_eq!(**selection.units(), *members);
        assert!(!Arc::ptr_eq(&held, selection.ordered_units()));
        assert_eq!(held.as_slice(), [Unit::Division(8), Unit::Division(7)]);
        assert_eq!(
            selection.ordered_units().as_slice(),
            [Unit::Division(7), Unit::Division(8)]
        );
    }

    #[test]
    fn viewers_keep_separate_orders_and_nonown_click_clears_both_views() {
        let mut first = Selection::default();
        let mut second = Selection::default();
        first.click(Some(&hit(Kind::Division, &[8, 7])), false);
        second.click(Some(&hit(Kind::Division, &[7, 8])), false);
        let held = Arc::clone(first.ordered_units());
        assert!(!first.click(None, true));
        assert!(Arc::ptr_eq(&held, first.ordered_units()));
        let mut foreign = hit(Kind::Division, &[1]);
        foreign.ours = false;
        assert!(first.click(Some(&foreign), false));
        assert!(first.ordered_units().is_empty());
        assert!(first.units().is_empty());
        assert_eq!(
            second.ordered_units().as_slice(),
            [Unit::Division(7), Unit::Division(8)]
        );
        assert_eq!(held.as_slice(), [Unit::Division(8), Unit::Division(7)]);
    }

    #[test]
    fn shift_readdition_appends_and_retirement_keeps_survivor_order() {
        let mut frame = test_frame(42);
        let mut selection = Selection::default();
        selection.observe(frame.branch, &Country::Recorded, 1, &frame);
        selection.click(Some(&hit(Kind::Division, &[8, 9, 7])), false);
        // Two toggles preserve membership but move the removed/readded member last.
        assert!(selection.click(Some(&hit(Kind::Division, &[9, 9])), true));
        assert_eq!(
            selection.ordered_units().as_slice(),
            [Unit::Division(8), Unit::Division(7), Unit::Division(9)]
        );
        selection.click(Some(&hit(Kind::RailwayGun, &[7])), true);
        put(
            &mut frame,
            TableRows::Divisions(vec![
                DivisionRow {
                    id: 7,
                    ..Default::default()
                },
                DivisionRow {
                    id: 9,
                    ..Default::default()
                },
            ]),
        );
        assert!(selection.observe(frame.branch, &Country::Recorded, 1, &frame));
        assert_eq!(
            selection.ordered_units().as_slice(),
            [Unit::Division(7), Unit::Division(9), Unit::RailwayGun(7)]
        );
        assert_eq!(
            selection
                .ordered_units()
                .iter()
                .copied()
                .collect::<BTreeSet<_>>(),
            **selection.units()
        );
        assert!(selection.observe(frame.branch, &Country::Recorded, 2, &frame));
        assert!(selection.ordered_units().is_empty());
        assert!(selection.units().is_empty());
    }

    #[test]
    fn missing_censuses_preserve_selection_and_complete_censuses_retire_only_their_class() {
        let mut frame = test_frame(42);
        let mut selection = Selection::default();
        let country = Country::Recorded;
        selection.observe(frame.branch, &country, 1, &frame);
        selection.click(Some(&hit(Kind::Division, &[7, 8])), false);
        selection.click(Some(&hit(Kind::RailwayGun, &[7])), true);
        assert!(!selection.observe(frame.branch, &country, 1, &frame));
        put(
            &mut frame,
            TableRows::Divisions(vec![DivisionRow {
                id: 8,
                ..Default::default()
            }]),
        );
        assert!(selection.observe(frame.branch, &country, 1, &frame));
        assert_eq!(
            selection.units().iter().copied().collect::<Vec<_>>(),
            [Unit::Division(8), Unit::RailwayGun(7)]
        );
        assert_eq!(
            selection.ordered_units().as_slice(),
            [Unit::Division(8), Unit::RailwayGun(7)]
        );
        assert!(selection.observe(frame.branch, &Country::Tag("other".into()), 1, &frame));
        assert!(selection.units().is_empty());
        assert!(selection.ordered_units().is_empty());
    }

    #[test]
    fn ownership_transfer_preserves_selected_full_handles_in_every_class() {
        const TRANSFERRED: u64 = 0x1_0000_0007;
        const ALIASED: u64 = 0x2_0000_0008;
        const STEADY: u64 = 0x3_0000_0009;
        let mut frame = ownership_frame(&[
            (TRANSFERRED, 101, 101, 0),
            (ALIASED, 101, 101, 0),
            (STEADY, 101, 101, 0),
        ]);
        let mut selection = Selection::default();
        selection.observe(frame.branch, &Country::Recorded, 1, &frame);
        for kind in [Kind::Division, Kind::TaskForce, Kind::RailwayGun] {
            selection.click(Some(&hit(kind, &[STEADY, TRANSFERRED, ALIASED])), true);
        }
        let held = Arc::clone(selection.ordered_units());
        let members = Arc::clone(selection.units());

        // A definite roster transfer and foreign logical/expeditionary artwork
        // retain already-selected identities, including their full upper bits.
        frame = ownership_frame(&[
            (TRANSFERRED, 201, 101, 101),
            (ALIASED, 102, 201, 201),
            (STEADY, 101, 101, 0),
        ]);
        assert!(!selection.observe(frame.branch, &Country::Recorded, 1, &frame));
        assert!(Arc::ptr_eq(&held, selection.ordered_units()));
        assert!(Arc::ptr_eq(&members, selection.units()));
        assert_eq!(
            selection
                .ordered_units()
                .iter()
                .copied()
                .collect::<BTreeSet<_>>(),
            **selection.units()
        );
        assert_eq!(held.len(), 9);

        // Further changes to raw/canonical ownership leave the same selection.
        frame = ownership_frame(&[
            (TRANSFERRED, 1, 201, 201),
            (ALIASED, 201, 201, 201),
            (STEADY, 101, 101, 0),
        ]);
        assert!(!selection.observe(frame.branch, &Country::Recorded, 1, &frame));
        assert!(Arc::ptr_eq(&held, selection.ordered_units()));
        assert!(Arc::ptr_eq(&members, selection.units()));

        // Only disappearance from complete class censuses retires the handle,
        // retaining the survivor order and the held tooltip's previous order.
        frame = ownership_frame(&[(ALIASED, 201, 201, 201), (STEADY, 101, 101, 0)]);
        assert!(selection.observe(frame.branch, &Country::Recorded, 1, &frame));
        assert_eq!(
            selection.ordered_units().as_slice(),
            [
                Unit::Division(STEADY),
                Unit::Division(ALIASED),
                Unit::TaskForce(STEADY),
                Unit::TaskForce(ALIASED),
                Unit::RailwayGun(STEADY),
                Unit::RailwayGun(ALIASED),
            ]
        );
        assert_eq!(held.len(), 9);
        assert_eq!(
            selection
                .ordered_units()
                .iter()
                .copied()
                .collect::<BTreeSet<_>>(),
            **selection.units()
        );
    }

    #[test]
    fn missing_owner_links_and_partial_censuses_preserve_selection() {
        use replay_data::rows::TableId;
        use replay_frame::frame::TableView;

        let mut frame = ownership_frame(&[(7, 999, 201, 201), (8, 101, 101, 0)]);
        let mut selection = Selection::default();
        selection.observe(frame.branch, &Country::Recorded, 1, &frame);
        for kind in [Kind::Division, Kind::RailwayGun, Kind::TaskForce] {
            selection.click(Some(&hit(kind, &[8, 7])), true);
        }
        let held = Arc::clone(selection.ordered_units());
        assert!(!selection.observe(frame.branch, &Country::Recorded, 1, &frame));
        assert!(Arc::ptr_eq(&held, selection.ordered_units()));

        frame = ownership_frame(&[(7, 201, 101, 101), (8, 101, 101, 0)]);
        let links = frame.tables[TableId::CountryLinks.index()].clone();
        for unknown in [
            TableView::Unavailable {
                reason: "CountryLinks read failed".into(),
            },
            TableView::Truncated {
                rows: TableRows::CountryLinks(vec![CountryLinkRow {
                    id: 101,
                    canonical_slot: 1,
                }]),
                missing: 1,
            },
        ] {
            frame.tables[TableId::CountryLinks.index()] = unknown.into();
            assert!(!selection.observe(frame.branch, &Country::Recorded, 1, &frame));
            assert!(Arc::ptr_eq(&held, selection.ordered_units()));
        }
        frame.tables[TableId::CountryLinks.index()] = links;
        frame.tables[TableId::Divisions.index()] = TableView::Truncated {
            rows: TableRows::Divisions(vec![DivisionRow {
                id: 7,
                roster_owner: 201,
                ..Default::default()
            }]),
            missing: 1,
        }
        .into();
        frame.tables[TableId::RailwayGunCounterInputs.index()] = TableView::Unavailable {
            reason: "railway gun census read failed".into(),
        }
        .into();
        frame.tables[TableId::TaskForces.index()] = TableView::Truncated {
            rows: TableRows::TaskForces(vec![TaskForceRow {
                id: 7,
                roster_owner: 201,
                ..Default::default()
            }]),
            missing: 1,
        }
        .into();
        assert!(!selection.observe(frame.branch, &Country::Recorded, 1, &frame));
        assert!(Arc::ptr_eq(&held, selection.ordered_units()));

        // A complete census can still prove disappearance without owner links.
        frame.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
            reason: "CountryLinks read failed".into(),
        }
        .into();
        put(&mut frame, TableRows::Divisions(Vec::new()));
        assert!(selection.observe(frame.branch, &Country::Recorded, 1, &frame));
        assert_eq!(
            selection.ordered_units().as_slice(),
            [
                Unit::RailwayGun(8),
                Unit::RailwayGun(7),
                Unit::TaskForce(8),
                Unit::TaskForce(7),
            ]
        );
    }

    #[test]
    fn selection_always_separates_unselected_members_and_only_splits_selected_members_when_requested()
     {
        let selected = BTreeSet::from([Unit::Division(1), Unit::Division(2)]);
        for split in [false, true] {
            assert!(!joins(
                &selected,
                Unit::Division(1),
                Unit::Division(3),
                split
            ));
            assert!(joins(
                &selected,
                Unit::Division(3),
                Unit::Division(4),
                split
            ));
            assert_eq!(
                joins(&selected, Unit::Division(1), Unit::Division(2), split),
                !split
            );
        }
        assert!(joins(
            &selected,
            Unit::RailwayGun(1),
            Unit::RailwayGun(2),
            true
        ));
    }

    #[test]
    fn a_partial_census_holds_members_and_a_branch_change_clears_them() {
        use replay_data::rows::TableId;
        use replay_frame::frame::TableView;
        let mut frame = test_frame(42);
        let mut selection = Selection::default();
        selection.observe(frame.branch, &Country::Recorded, 1, &frame);
        selection.click(Some(&hit(Kind::Division, &[7, 8])), false);
        frame.tables[TableId::Divisions.index()] = TableView::Truncated {
            rows: TableRows::Divisions(vec![DivisionRow {
                id: 8,
                ..Default::default()
            }]),
            missing: 1,
        }
        .into();
        assert!(!selection.observe(frame.branch, &Country::Recorded, 1, &frame));
        assert_eq!(selection.units().len(), 2);
        assert_eq!(
            selection.ordered_units().as_slice(),
            [Unit::Division(7), Unit::Division(8)]
        );
        assert!(selection.observe(frame.branch + 1, &Country::Recorded, 1, &frame));
        assert!(selection.units().is_empty());
        assert!(selection.ordered_units().is_empty());
        selection.click(Some(&hit(Kind::Division, &[7])), false);
        assert!(selection.observe(frame.branch + 1, &Country::Recorded, 2, &frame));
        assert!(selection.units().is_empty());
    }
}
