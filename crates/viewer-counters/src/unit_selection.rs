
use std::{collections::BTreeSet, sync::Arc};
use viewer_native_gui::counter_hovers::unit_hits::Hit;
#[cfg(test)]
use viewer_native_gui::counter_hovers::unit_hits::Kind;
use viewer_runtime::unit_handle::Unit;

use replay_data::{BranchId, rows::tables};
use replay_frame::frame::DecodedFrame;

#[derive(Default)]
pub struct Selection {
    context: Option<BranchId>,
    units: Arc<BTreeSet<Unit>>,
    ordered: Arc<Vec<Unit>>,
}

impl Selection {
    pub fn units(&self) -> &Arc<BTreeSet<Unit>> {
        &self.units
    }

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
            self.units = before;
            self.ordered = before_order;
        }
        changed
    }

    pub fn select_box(&mut self, units: impl IntoIterator<Item = Unit>, shift: bool) -> bool {
        let before = (Arc::clone(&self.units), Arc::clone(&self.ordered));
        if !shift {
            self.units = Arc::default();
            self.ordered = Arc::default();
        }
        for unit in units {
            if !self.units.contains(&unit) {
                Arc::make_mut(&mut self.units).insert(unit);
                Arc::make_mut(&mut self.ordered).push(unit);
            }
        }
        self.keep_unless_changed(before)
    }

    pub fn model_click(
        &mut self,
        picked: Unit,
        mates: &[Unit],
        ours: bool,
        shift: bool,
        theatre: &dyn Fn(Unit) -> Result<u64, &'static str>,
    ) -> (bool, Option<&'static str>) {
        let before = (Arc::clone(&self.units), Arc::clone(&self.ordered));
        if shift && self.units.contains(&picked) {
            Arc::make_mut(&mut self.units).remove(&picked);
            Arc::make_mut(&mut self.ordered).retain(|unit| *unit != picked);
            return (true, None);
        }
        if !shift {
            self.units = Arc::default();
            self.ordered = Arc::default();
        }
        let kind = |unit: &Unit| std::mem::discriminant(unit);
        let anchor = self.ordered.first().copied().unwrap_or(picked);
        let mut withheld = None;
        if ours && kind(&anchor) == kind(&picked) {
            let admitted: Result<Vec<Unit>, &'static str> = match picked {
                Unit::TaskForce(_) => Ok(mates.to_vec()),
                Unit::Division(_) | Unit::RailwayGun(_) => theatre(anchor).and_then(|home| {
                    mates.iter().filter_map(|&unit| theatre(unit).map(|at| (at == home).then_some(unit)).transpose()).collect()
                }),
            };
            match admitted {
                Ok(admitted) => {
                    for unit in admitted {
                        if !self.units.contains(&unit) {
                            Arc::make_mut(&mut self.units).insert(unit);
                            Arc::make_mut(&mut self.ordered).push(unit);
                        }
                    }
                }
                Err(why) => withheld = Some(why),
            }
        }
        (self.keep_unless_changed(before), withheld)
    }

    fn keep_unless_changed(&mut self, before: (Arc<BTreeSet<Unit>>, Arc<Vec<Unit>>)) -> bool {
        let changed = *before.0 != *self.units || *before.1 != *self.ordered;
        if !changed {
            (self.units, self.ordered) = before;
        }
        changed
    }

    pub fn context(&mut self, branch: BranchId) -> bool {
        if self.context == Some(branch) {
            return false;
        }
        self.context = Some(branch);
        self.clear()
    }

    pub fn observe(&mut self, branch: BranchId, frame: &DecodedFrame) -> bool {
        let mut changed = self.context(branch);
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
    use viewer_counter_kit::selection_joins;
    use super::*;
    use replay_data::{
        rows::CountryLinkRow, rows::DivisionRow, rows::RailwayGunCounterInputRow, rows::TableRows,
        rows::TaskForceRow,
    };
    use viewer_foundation::test_support::{test_frame, test_put as put};

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
    fn an_empty_box_clears_without_shift_and_shift_adds_without_toggling() {
        let mut selection = Selection::default();
        assert!(selection.select_box([Unit::Division(1), Unit::Division(2)], false));
        assert!(selection.select_box([Unit::Division(2), Unit::Division(3)], true));
        assert_eq!(
            selection.ordered_units().as_slice(),
            [Unit::Division(1), Unit::Division(2), Unit::Division(3)]
        );
        let held = Arc::clone(selection.ordered_units());
        assert!(!selection.select_box([Unit::Division(1)], true));
        assert!(!selection.select_box([], true));
        assert!(Arc::ptr_eq(&held, selection.ordered_units()));
        assert!(selection.select_box([Unit::Division(3)], false));
        assert_eq!(selection.ordered_units().as_slice(), [Unit::Division(3)]);
        assert!(selection.select_box([], false));
        assert!(selection.units().is_empty());
    }

    #[test]
    fn a_model_click_selects_our_mates_clears_on_a_unit_not_ours_and_shift_toggles_the_picked() {
        let mut selection = Selection::default();
        let theatres = |unit: Unit| -> Result<u64, &'static str> { Ok(if unit == Unit::Division(3) { 4 } else { 0 }) };
        let click = |selection: &mut Selection, picked, mates: &[Unit], ours, shift| {
            selection.model_click(picked, mates, ours, shift, &theatres)
        };
        let mates = [Unit::Division(1), Unit::Division(2), Unit::Division(3)];
        assert_eq!(click(&mut selection, Unit::Division(1), &mates, true, false), (true, None));
        assert_eq!(selection.ordered_units().as_slice(), [Unit::Division(1), Unit::Division(2)], "another theatre's mate stays out");
        assert_eq!(click(&mut selection, Unit::Division(9), &[Unit::Division(9)], false, true), (false, None));
        assert_eq!(click(&mut selection, Unit::Division(2), &mates, true, true), (true, None));
        assert_eq!(selection.ordered_units().as_slice(), [Unit::Division(1)]);
        assert_eq!(click(&mut selection, Unit::RailwayGun(5), &[Unit::RailwayGun(5)], true, true), (false, None));
        let unrecorded = selection.model_click(Unit::Division(7), &[Unit::Division(7)], true, true, &|_| Err("unit theatre not recorded"));
        assert_eq!(unrecorded, (false, Some("unit theatre not recorded")));
        assert_eq!(click(&mut selection, Unit::Division(9), &[Unit::Division(9)], false, false), (true, None));
        assert!(selection.units().is_empty());
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
        selection.observe(frame.branch, &frame);
        selection.click(Some(&hit(Kind::Division, &[8, 9, 7])), false);
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
        assert!(selection.observe(frame.branch, &frame));
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
    }

    #[test]
    fn missing_censuses_preserve_selection_and_complete_censuses_retire_only_their_class() {
        let mut frame = test_frame(42);
        let mut selection = Selection::default();
        selection.observe(frame.branch, &frame);
        selection.click(Some(&hit(Kind::Division, &[7, 8])), false);
        selection.click(Some(&hit(Kind::RailwayGun, &[7])), true);
        assert!(!selection.observe(frame.branch, &frame));
        put(
            &mut frame,
            TableRows::Divisions(vec![DivisionRow {
                id: 8,
                ..Default::default()
            }]),
        );
        assert!(selection.observe(frame.branch, &frame));
        assert_eq!(
            selection.units().iter().copied().collect::<Vec<_>>(),
            [Unit::Division(8), Unit::RailwayGun(7)]
        );
        assert_eq!(
            selection.ordered_units().as_slice(),
            [Unit::Division(8), Unit::RailwayGun(7)]
        );
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
        selection.observe(frame.branch, &frame);
        for kind in [Kind::Division, Kind::TaskForce, Kind::RailwayGun] {
            selection.click(Some(&hit(kind, &[STEADY, TRANSFERRED, ALIASED])), true);
        }
        let held = Arc::clone(selection.ordered_units());
        let members = Arc::clone(selection.units());

        frame = ownership_frame(&[
            (TRANSFERRED, 201, 101, 101),
            (ALIASED, 102, 201, 201),
            (STEADY, 101, 101, 0),
        ]);
        assert!(!selection.observe(frame.branch, &frame));
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

        frame = ownership_frame(&[
            (TRANSFERRED, 1, 201, 201),
            (ALIASED, 201, 201, 201),
            (STEADY, 101, 101, 0),
        ]);
        assert!(!selection.observe(frame.branch, &frame));
        assert!(Arc::ptr_eq(&held, selection.ordered_units()));
        assert!(Arc::ptr_eq(&members, selection.units()));

        frame = ownership_frame(&[(ALIASED, 201, 201, 201), (STEADY, 101, 101, 0)]);
        assert!(selection.observe(frame.branch, &frame));
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
        selection.observe(frame.branch, &frame);
        for kind in [Kind::Division, Kind::RailwayGun, Kind::TaskForce] {
            selection.click(Some(&hit(kind, &[8, 7])), true);
        }
        let held = Arc::clone(selection.ordered_units());
        assert!(!selection.observe(frame.branch, &frame));
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
            assert!(!selection.observe(frame.branch, &frame));
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
        assert!(!selection.observe(frame.branch, &frame));
        assert!(Arc::ptr_eq(&held, selection.ordered_units()));

        frame.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
            reason: "CountryLinks read failed".into(),
        }
        .into();
        put(&mut frame, TableRows::Divisions(Vec::new()));
        assert!(selection.observe(frame.branch, &frame));
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
            assert!(!selection_joins(
                &selected,
                Unit::Division(1),
                Unit::Division(3),
                split
            ));
            assert!(selection_joins(
                &selected,
                Unit::Division(3),
                Unit::Division(4),
                split
            ));
            assert_eq!(
                selection_joins(&selected, Unit::Division(1), Unit::Division(2), split),
                !split
            );
        }
        assert!(selection_joins(
            &selected,
            Unit::RailwayGun(1),
            Unit::RailwayGun(2),
            true
        ));
    }

    #[test]
    fn a_partial_census_holds_members_and_only_a_branch_change_clears_them() {
        use replay_data::rows::TableId;
        use replay_frame::frame::TableView;
        let mut frame = test_frame(42);
        let mut selection = Selection::default();
        selection.observe(frame.branch, &frame);
        selection.click(Some(&hit(Kind::Division, &[7, 8])), false);
        frame.tables[TableId::Divisions.index()] = TableView::Truncated {
            rows: TableRows::Divisions(vec![DivisionRow {
                id: 8,
                ..Default::default()
            }]),
            missing: 1,
        }
        .into();
        assert!(!selection.observe(frame.branch, &frame));
        assert_eq!(selection.units().len(), 2);
        assert_eq!(
            selection.ordered_units().as_slice(),
            [Unit::Division(7), Unit::Division(8)]
        );
        assert!(selection.observe(frame.branch + 1, &frame));
        assert!(selection.units().is_empty());
        assert!(selection.ordered_units().is_empty());
    }
}
