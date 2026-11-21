use replay_data::{BranchId, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};

#[derive(Default)]
pub struct Selection {
    context: Option<BranchId>,
    wings: Arc<Vec<u64>>,
    bases: Arc<Vec<u64>>,
}

#[derive(Clone)]
pub struct BaseTarget {
    base: u64,
    members: Vec<u64>,
    occupied: bool,
}

impl Selection {
    pub fn wings(&self) -> &Arc<Vec<u64>> {
        &self.wings
    }

    pub fn bases(&self) -> &Arc<Vec<u64>> {
        &self.bases
    }

    pub fn mission_click(
        &mut self,
        frame: &DecodedFrame,
        members: &[u64],
        shift: bool,
    ) -> io::Result<bool> {
        let wings = frame
            .complete::<tables::AirWings>()
            .map_err(|error| io::Error::other(error.to_string()))?;
        let witnesses = frame
            .complete::<tables::AirReferenceWitnesses>()
            .map_err(|error| io::Error::other(error.to_string()))?;
        let before = Arc::clone(&self.wings);
        let changed = self.click(members, shift);
        let mut bases_changed = false;
        let bases = Arc::make_mut(&mut self.bases);
        for &handle in self.wings.iter() {
            if shift && before.contains(&handle) {
                continue;
            }
            if let Ok(at) = wings.binary_search_by_key(&handle, |wing| wing.id) {
                let base = wings[at].air_base;
                if base != 0
                    && witnesses
                        .binary_search_by_key(&base, |row| row.handle)
                        .ok()
                        .is_some_and(|at| witnesses[at].base_kind.is_some())
                    && !bases.contains(&base)
                {
                    bases.push(base);
                    bases_changed = true;
                }
            }
        }
        Ok(changed || bases_changed)
    }

    pub fn clear_bases(&mut self) -> bool {
        if self.bases.is_empty() {
            return false;
        }
        self.bases = Arc::default();
        true
    }

    pub fn grid_click(
        &mut self,
        frame: &DecodedFrame,
        members: &[u64],
        shift: bool,
    ) -> io::Result<bool> {
        let wings = frame.complete::<tables::AirWings>().map_err(unavailable)?;
        let witnesses = frame
            .complete::<tables::AirReferenceWitnesses>()
            .map_err(unavailable)?;
        let live: Vec<_> = members
            .iter()
            .filter_map(|handle| {
                let at = wings.binary_search_by_key(handle, |wing| wing.id).ok()?;
                Some((*handle, wings[at].air_base))
            })
            .collect();
        if live.is_empty() {
            return Ok(false);
        }
        let before = (Arc::clone(&self.wings), Arc::clone(&self.bases));
        if !shift {
            self.clear();
        }
        for (handle, base) in live {
            let selected = Arc::make_mut(&mut self.wings);
            if let Some(at) = selected.iter().position(|&selected| selected == handle) {
                selected.remove(at);
            } else {
                selected.push(handle);
                if base != 0
                    && witnesses
                        .binary_search_by_key(&base, |row| row.handle)
                        .ok()
                        .is_some_and(|at| witnesses[at].base_kind.is_some())
                {
                    let bases = Arc::make_mut(&mut self.bases);
                    if !bases.contains(&base) {
                        bases.push(base);
                    }
                }
            }
        }
        Ok(*before.0 != *self.wings || *before.1 != *self.bases)
    }

    pub fn box_click(
        &mut self,
        frame: &DecodedFrame,
        members: &[u64],
        shift: bool,
    ) -> io::Result<bool> {
        let wings = frame.complete::<tables::AirWings>().map_err(unavailable)?;
        let witnesses = frame
            .complete::<tables::AirReferenceWitnesses>()
            .map_err(unavailable)?;
        let before = (Arc::clone(&self.wings), Arc::clone(&self.bases));
        if !shift {
            self.clear();
        }
        for &handle in members {
            let Ok(at) = wings.binary_search_by_key(&handle, |wing| wing.id) else {
                continue;
            };
            if self.wings.contains(&handle) {
                continue;
            }
            Arc::make_mut(&mut self.wings).push(handle);
            let base = wings[at].air_base;
            if base != 0
                && witnesses
                    .binary_search_by_key(&base, |row| row.handle)
                    .ok()
                    .is_some_and(|at| witnesses[at].base_kind.is_some())
                && !self.bases.contains(&base)
            {
                Arc::make_mut(&mut self.bases).push(base);
            }
        }
        if *before.0 == *self.wings && *before.1 == *self.bases {
            (self.wings, self.bases) = before;
            return Ok(false);
        }
        Ok(true)
    }

    pub fn base_click(
        &mut self,
        frame: &DecodedFrame,
        target: BaseTarget,
        shift: bool,
    ) -> io::Result<bool> {
        let BaseTarget {
            base,
            members,
            occupied,
        } = target;
        let wings = frame.complete::<tables::AirWings>().map_err(unavailable)?;
        let members: Vec<_> = members
            .into_iter()
            .map(|handle| {
                let at = wings
                    .binary_search_by_key(&handle, |wing| wing.id)
                    .map_err(|_| io::Error::other("registered air-base wing is absent"))?;
                Ok((handle, wings[at].air_base))
            })
            .collect::<io::Result<_>>()?;
        let before = (Arc::clone(&self.wings), Arc::clone(&self.bases));
        if !shift {
            self.clear();
        }
        if !occupied {
            let bases = Arc::make_mut(&mut self.bases);
            if !bases.contains(&base) {
                bases.push(base);
            }
        }
        let mut selected: BTreeSet<_> = self.wings.iter().copied().collect();
        for (handle, pool_base) in members {
            if selected.insert(handle) {
                Arc::make_mut(&mut self.wings).push(handle);
                let bases = Arc::make_mut(&mut self.bases);
                if pool_base != 0 && !bases.contains(&pool_base) {
                    bases.push(pool_base);
                }
            }
        }
        Ok(*before.0 != *self.wings || *before.1 != *self.bases)
    }

    pub fn clear(&mut self) -> bool {
        if self.wings.is_empty() && self.bases.is_empty() {
            return false;
        }
        self.wings = Arc::default();
        self.bases = Arc::default();
        true
    }

    pub fn click(&mut self, members: &[u64], shift: bool) -> bool {
        let before = Arc::clone(&self.wings);
        let wings = Arc::make_mut(&mut self.wings);
        let mut present: BTreeSet<_> = wings.iter().copied().collect();
        if shift && !wings.is_empty() && members.iter().all(|wing| present.contains(wing)) {
            let removed: BTreeSet<_> = members.iter().copied().collect();
            wings.retain(|wing| !removed.contains(wing));
        } else {
            if !shift {
                wings.clear();
                present.clear();
            }
            for &wing in members {
                if present.insert(wing) {
                    wings.push(wing);
                }
            }
        }
        if *before == *self.wings {
            self.wings = before;
            false
        } else {
            true
        }
    }

    pub fn context(&mut self, branch: BranchId) {
        if self.context != Some(branch) {
            self.clear();
            self.context = Some(branch);
        }
    }

    pub fn observe(
        &mut self,
        branch: BranchId,
        viewer: u32,
        frame: &DecodedFrame,
    ) {
        self.context(branch);
        if let Ok(rows) = frame.complete::<tables::AirWings>() {
            Arc::make_mut(&mut self.wings)
                .retain(|wing| rows.binary_search_by_key(wing, |row| row.id).is_ok());
        }
        if let Ok(rows) = frame.complete::<tables::AirBaseRegistrations>() {
            Arc::make_mut(&mut self.bases).retain(|base| {
                rows.binary_search_by_key(&(*base, viewer), |row| (row.base, row.slot))
                    .is_ok()
            });
        }
    }
}

fn unavailable(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

pub fn base_target(
    frame: &DecodedFrame,
    viewer: u32,
    province: u32,
    kind: i32,
) -> io::Result<Option<BaseTarget>> {
    let provinces = frame.complete::<tables::Provinces>().map_err(unavailable)?;
    let state = provinces
        .binary_search_by_key(&province, |row| row.id)
        .map(|at| provinces[at].state)
        .map_err(|_| io::Error::other("clicked air-base province is absent"))?;
    let flags = frame
        .complete::<tables::StateFlags>()
        .map_err(unavailable)?;
    if flags
        .binary_search_by_key(&state, |row| row.state)
        .ok()
        .is_some_and(|at| flags[at].demilitarized != 0)
    {
        return Ok(None);
    }
    let bases = frame.complete::<tables::AirBases>().map_err(unavailable)?;
    let mut bases = bases
        .iter()
        .filter(|base| base.state == Some(state) && base.kind == kind);
    let Some(base) = bases.next() else {
        return Ok(None);
    };
    if bases.next().is_some() {
        return Err(io::Error::other(
            "clicked state has multiple air bases of the same kind",
        ));
    }
    let registrations = frame
        .complete::<tables::AirBaseRegistrations>()
        .map_err(unavailable)?;
    let Some(row) = registrations
        .binary_search_by_key(&(base.id, viewer), |row| (row.base, row.slot))
        .ok()
        .map(|at| &registrations[at])
        .filter(|row| row.capacity_raw > 0)
    else {
        return Ok(None);
    };
    let wings = frame.complete::<tables::AirWings>().map_err(unavailable)?;
    let mut occupied = false;
    for handle in &row.wings {
        let at = wings
            .binary_search_by_key(handle, |wing| wing.id)
            .map_err(|_| io::Error::other("registered air-base wing is absent"))?;
        occupied |= wings[at].planes > 0;
    }
    Ok(Some(BaseTarget {
        base: base.id,
        members: row.wings.clone(),
        occupied,
    }))
}

pub fn mission_members(
    frame: &DecodedFrame,
    viewer: u32,
    region: u32,
    mission: usize,
) -> io::Result<Vec<u64>> {
    if mission >= 18 {
        return Err(io::Error::other(
            "air mission kind is outside the native enumeration",
        ));
    }
    let wings = frame
        .complete::<tables::AirWings>()
        .map_err(|error| io::Error::other(error.to_string()))?;
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(|error| io::Error::other(error.to_string()))?;
    let registrations = frame
        .complete::<tables::AirRegionalMissions>()
        .map_err(|error| io::Error::other(error.to_string()))?;
    let wings: BTreeMap<_, _> = wings.iter().map(|wing| (wing.id, wing)).collect();
    let links: BTreeMap<_, _> = links
        .iter()
        .map(|link| (link.id, link.canonical_slot))
        .collect();
    let mut members = Vec::new();
    for row in registrations {
        let wing = wings
            .get(&row.wing)
            .ok_or_else(|| io::Error::other("registered air wing is absent"))?;
        if links.get(&wing.owner) == Some(&viewer)
            && wing.mission_region == Some(region)
            && wing.mission_mask & (1 << mission) != 0
        {
            members.push((row.ordinal, row.wing));
        }
    }
    members.sort_by_key(|(ordinal, _)| *ordinal);
    Ok(members.into_iter().map(|(_, wing)| wing).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{
        rows::AirRegionalMissionRow, rows::AirWingRow, rows::CountryLinkRow, rows::TableId,
        rows::TableRows,
    };
    use replay_frame::{frame::Metrics, frame::TableView};

    fn frame(wings: Option<Vec<AirWingRow>>) -> DecodedFrame {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 10,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "synthetic incomplete census".into(),
                }
                .into()
            }),
        };
        if let Some(wings) = wings {
            *frame.tables[TableId::AirWings.index()] =
                TableView::Complete(TableRows::AirWings(wings));
        }
        frame
    }

    fn base_frame(planes: i32) -> DecodedFrame {
        use replay_data::{rows::AirBaseRegistrationRow, rows::AirBaseRow, rows::ProvinceRow};
        let mut frame = frame(Some(vec![
            AirWingRow {
                id: 1,
                air_base: 8,
                planes,
                ..Default::default()
            },
            AirWingRow {
                id: 2,
                air_base: 8,
                ..Default::default()
            },
        ]));
        *frame.tables[TableId::Provinces.index()] =
            TableView::Complete(TableRows::Provinces(vec![ProvinceRow {
                id: 3,
                state: 4,
                ..Default::default()
            }]));
        *frame.tables[TableId::AirBases.index()] =
            TableView::Complete(TableRows::AirBases(vec![AirBaseRow {
                id: 8,
                state: Some(4),
                kind: 0,
                ..Default::default()
            }]));
        *frame.tables[TableId::StateFlags.index()] =
            TableView::Complete(TableRows::StateFlags(vec![]));
        *frame.tables[TableId::AirBaseRegistrations.index()] =
            TableView::Complete(TableRows::AirBaseRegistrations(vec![
                AirBaseRegistrationRow {
                    base: 8,
                    slot: 5,
                    capacity_raw: 1,
                    wings: vec![2, 1, 2],
                    ..Default::default()
                },
            ]));
        frame
    }

    #[test]
    fn base_buttons_replace_or_append_without_toggling_and_include_empty_wings() {
        let frame = base_frame(10);
        let mut selection = Selection::default();
        selection.click(&[99], false);
        assert!(
            selection
                .base_click(
                    &frame,
                    base_target(&frame, 5, 3, 0).unwrap().unwrap(),
                    false
                )
                .unwrap()
        );
        assert_eq!(selection.wings().as_slice(), [2, 1]);
        assert_eq!(selection.bases().as_slice(), [8]);
        assert!(
            !selection
                .base_click(&frame, base_target(&frame, 5, 3, 0).unwrap().unwrap(), true)
                .unwrap()
        );
        selection.click(&[99], false);
        selection
            .base_click(&frame, base_target(&frame, 5, 3, 0).unwrap().unwrap(), true)
            .unwrap();
        assert_eq!(selection.wings().as_slice(), [99, 2, 1]);
        assert!(base_target(&frame, 6, 3, 0).unwrap().is_none());
        assert!(base_target(&frame, 5, 3, 1).unwrap().is_none());
    }

    #[test]
    fn a_base_without_positive_aircraft_keeps_its_reference_even_without_wings_or_witnesses() {
        let mut frame = base_frame(0);
        let mut selection = Selection::default();
        selection
            .base_click(
                &frame,
                base_target(&frame, 5, 3, 0).unwrap().unwrap(),
                false,
            )
            .unwrap();
        assert_eq!(selection.wings().as_slice(), [2, 1]);
        assert_eq!(selection.bases().as_slice(), [8]);
        let TableView::Complete(TableRows::AirBaseRegistrations(rows)) =
            &mut *frame.tables[TableId::AirBaseRegistrations.index()]
        else {
            panic!()
        };
        rows[0].wings.clear();
        selection
            .base_click(
                &frame,
                base_target(&frame, 5, 3, 0).unwrap().unwrap(),
                false,
            )
            .unwrap();
        assert!(selection.wings().is_empty());
        assert_eq!(selection.bases().as_slice(), [8]);
        let TableView::Complete(TableRows::AirBaseRegistrations(rows)) =
            &mut *frame.tables[TableId::AirBaseRegistrations.index()]
        else {
            panic!()
        };
        rows[0].capacity_raw = 0;
        assert!(base_target(&frame, 5, 3, 0).unwrap().is_none());
    }

    #[test]
    fn registration_gates_disable_buttons_without_erasing_selected_base_references() {
        let mut frame = base_frame(0);
        let mut selection = Selection::default();
        selection.observe(1, 5, &frame);
        selection
            .base_click(
                &frame,
                base_target(&frame, 5, 3, 0).unwrap().unwrap(),
                false,
            )
            .unwrap();
        *frame.tables[TableId::StateFlags.index()] =
            TableView::Complete(TableRows::StateFlags(vec![
                replay_data::rows::StateFlagRow {
                    state: 4,
                    demilitarized: 1,
                    ..Default::default()
                },
            ]));
        assert!(base_target(&frame, 5, 3, 0).unwrap().is_none());
        selection.observe(1, 5, &frame);
        assert_eq!(selection.bases().as_slice(), [8]);
        assert_eq!(selection.wings().as_slice(), [2, 1]);
        let TableView::Complete(TableRows::AirBaseRegistrations(rows)) =
            &mut *frame.tables[TableId::AirBaseRegistrations.index()]
        else {
            panic!()
        };
        rows[0].capacity_raw = 0;
        selection.observe(1, 5, &frame);
        assert_eq!(selection.bases().as_slice(), [8]);
        let TableView::Complete(TableRows::AirBaseRegistrations(rows)) =
            &mut *frame.tables[TableId::AirBaseRegistrations.index()]
        else {
            panic!()
        };
        rows.clear();
        selection.observe(1, 5, &frame);
        assert!(selection.bases().is_empty());
    }

    #[test]
    fn grid_shift_toggles_individual_members_and_keeps_deselected_base_references() {
        let mut observed = frame(Some(vec![
            AirWingRow {
                id: 1,
                air_base: 8,
                ..Default::default()
            },
            AirWingRow {
                id: 2,
                air_base: 9,
                ..Default::default()
            },
        ]));
        *observed.tables[TableId::AirReferenceWitnesses.index()] =
            TableView::Complete(TableRows::AirReferenceWitnesses(
                [8, 9, 10]
                    .map(|handle| replay_data::rows::AirReferenceWitnessRow {
                        handle,
                        base_kind: (handle != 10).then_some(0),
                        ..Default::default()
                    })
                    .to_vec(),
            ));
        let mut selected = Selection::default();
        selected.mission_click(&observed, &[1], false).unwrap();
        assert!(selected.grid_click(&observed, &[1, 2], true).unwrap());
        assert_eq!(selected.wings().as_slice(), [2]);
        assert_eq!(selected.bases().as_slice(), [8, 9]);
        assert!(selected.grid_click(&observed, &[2], false).unwrap());
        assert_eq!(selected.wings().as_slice(), [2]);
        assert_eq!(selected.bases().as_slice(), [9]);
        assert!(!selected.grid_click(&observed, &[99], false).unwrap());
        assert_eq!(selected.wings().as_slice(), [2]);
        selected.grid_click(&observed, &[1, 1], false).unwrap();
        assert!(selected.wings().is_empty());
        assert_eq!(selected.bases().as_slice(), [8]);
        assert!(selected.box_click(&observed, &[2, 99, 2], true).unwrap());
        assert_eq!((selected.wings().as_slice(), selected.bases().as_slice()), ([2].as_slice(), [8, 9].as_slice()));
        assert!(!selected.box_click(&observed, &[2], true).unwrap());
        assert!(selected.box_click(&observed, &[1], false).unwrap());
        assert_eq!((selected.wings().as_slice(), selected.bases().as_slice()), ([1].as_slice(), [8].as_slice()));
        assert!(selected.box_click(&observed, &[], false).unwrap());
        assert!(selected.wings().is_empty() && selected.bases().is_empty());
    }

    #[test]
    fn selected_base_references_survive_mission_replacement_and_deselection_until_cleared() {
        let mut frame = frame(Some(vec![
            AirWingRow {
                id: 1,
                air_base: 8,
                ..Default::default()
            },
            AirWingRow {
                id: 2,
                air_base: 9,
                ..Default::default()
            },
            AirWingRow {
                id: 3,
                air_base: 10,
                ..Default::default()
            },
        ]));
        *frame.tables[TableId::AirReferenceWitnesses.index()] =
            TableView::Complete(TableRows::AirReferenceWitnesses(
                [8, 9]
                    .map(|handle| replay_data::rows::AirReferenceWitnessRow {
                        handle,
                        base_kind: Some(0),
                        ..Default::default()
                    })
                    .to_vec(),
            ));
        let mut selected = Selection::default();
        assert!(selected.mission_click(&frame, &[1], false).unwrap());
        assert_eq!(selected.bases().as_slice(), [8]);
        assert!(selected.mission_click(&frame, &[2], false).unwrap());
        assert_eq!(selected.bases().as_slice(), [8, 9]);
        assert!(selected.mission_click(&frame, &[2], true).unwrap());
        assert!(selected.wings().is_empty());
        assert_eq!(selected.bases().as_slice(), [8, 9]);
        assert!(selected.clear_bases());
        assert!(selected.mission_click(&frame, &[3], false).unwrap());
        assert!(
            selected.bases().is_empty(),
            "an unresolved pool base adds no selected base reference"
        );
        selected.clear();
        assert!(!selected.clear());
    }

    #[test]
    fn mission_buttons_select_registered_pool_owners_in_order_even_without_aircraft() {
        let mut observed = frame(Some(vec![
            AirWingRow {
                id: 1,
                owner: 11,
                mission_region: Some(8),
                mission_mask: 1,
                planes: 0,
                ..Default::default()
            },
            AirWingRow {
                id: 2,
                owner: 12,
                mission_region: Some(8),
                mission_mask: 1,
                planes: 100,
                ..Default::default()
            },
            AirWingRow {
                id: 3,
                owner: 11,
                mission_region: Some(8),
                mission_mask: 3,
                planes: 100,
                ..Default::default()
            },
            AirWingRow {
                id: 4,
                owner: 11,
                mission_region: Some(8),
                mission_mask: 1,
                planes: 100,
                ..Default::default()
            },
        ]));
        *observed.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 11,
                    canonical_slot: 5,
                },
                CountryLinkRow {
                    id: 12,
                    canonical_slot: 6,
                },
            ]));
        *observed.tables[TableId::AirRegionalMissions.index()] =
            TableView::Complete(TableRows::AirRegionalMissions(vec![
                AirRegionalMissionRow {
                    wing: 1,
                    ordinal: 2,
                    ..Default::default()
                },
                AirRegionalMissionRow {
                    wing: 2,
                    ordinal: 0,
                    ..Default::default()
                },
                AirRegionalMissionRow {
                    wing: 3,
                    ordinal: 1,
                    ..Default::default()
                },
            ]));
        assert_eq!(mission_members(&observed, 5, 8, 0).unwrap(), [3, 1]);
        assert_eq!(mission_members(&observed, 5, 8, 1).unwrap(), [3]);
        assert!(mission_members(&observed, 5, 8, 18).is_err());
    }

    #[test]
    fn missing_censuses_keep_selection_but_complete_censuses_and_branch_changes_retire_it() {
        let mut selected = Selection::default();
        selected.observe(1, 5, &frame(None));
        selected.click(&[1, 3], false);
        selected.observe(1, 5, &frame(None));
        assert_eq!(selected.wings().as_slice(), [1, 3]);
        selected.observe(
            1,
            5,
            &frame(Some(vec![AirWingRow {
                id: 3,
                ..Default::default()
            }])),
        );
        assert_eq!(selected.wings().as_slice(), [3]);
        selected.observe(2, 5, &frame(None));
        assert!(selected.wings().is_empty());
    }
    #[test]
    fn shift_adds_missing_members_and_removes_a_fully_selected_collection() {
        let mut selected = Selection::default();
        assert!(selected.click(&[7, 4], false));
        assert!(selected.click(&[4, 9], true));
        assert_eq!(selected.wings().as_slice(), &[7, 4, 9]);
        assert!(selected.click(&[4, 9], true));
        assert_eq!(selected.wings().as_slice(), &[7]);
        assert!(selected.click(&[], false));
        assert!(!selected.click(&[], false));
    }
}
