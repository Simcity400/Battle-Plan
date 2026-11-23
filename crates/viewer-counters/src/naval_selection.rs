use replay_data::BranchId;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::{io, sync::Arc};

#[derive(Default)]
pub struct Selection {
    context: Option<BranchId>,
    fleets: Arc<Vec<u64>>,
}

impl Selection {
    pub fn fleets(&self) -> &Arc<Vec<u64>> {
        &self.fleets
    }
    pub fn clear(&mut self) -> bool {
        if self.fleets.is_empty() {
            return false;
        }
        self.fleets = Arc::default();
        true
    }
    pub fn click(&mut self, fleet: u64, shift: bool) -> bool {
        let before = Arc::clone(&self.fleets);
        let selected = Arc::make_mut(&mut self.fleets);
        if !shift {
            selected.clear();
        }
        if shift && let Some(at) = selected.iter().position(|&id| id == fleet) {
            selected.remove(at);
        } else if !selected.contains(&fleet) {
            selected.push(fleet);
        }
        *before != *self.fleets
    }
    pub fn context(&mut self, branch: BranchId) {
        if self.context != Some(branch) {
            self.clear();
            self.context = Some(branch);
        }
    }
    pub fn observe(&mut self, branch: BranchId, frame: &DecodedFrame) {
        self.context(branch);
        if let Ok(rows) = frame.complete::<tables::Fleets>() {
            Arc::make_mut(&mut self.fleets)
                .retain(|id| rows.binary_search_by_key(id, |row| row.id).is_ok());
        }
    }
}

pub fn fleet_choices(frame: &DecodedFrame, viewer: u32) -> io::Result<Vec<(u64, String)>> {
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(|e| io::Error::other(e.to_string()))?;
    let scores = frame
        .complete::<tables::CountryGameScoreInputs>()
        .map_err(|e| io::Error::other(e.to_string()))?;
    let fleets = frame
        .complete::<tables::Fleets>()
        .map_err(|e| io::Error::other(e.to_string()))?;
    let country = scores
        .iter()
        .find(|row| {
            links
                .binary_search_by_key(&row.country, |link| link.id)
                .ok()
                .is_some_and(|at| links[at].canonical_slot == viewer)
        })
        .ok_or_else(|| io::Error::other("viewing navy's fleet roster is absent"))?;
    if !country.navy_error.is_empty() {
        return Err(io::Error::other(
            "viewing navy's fleet roster is unreadable",
        ));
    }
    let mut result = Vec::new();
    for occurrence in &country.fleets {
        let fleet = fleets
            .binary_search_by_key(&occurrence.fleet, |row| row.id)
            .ok()
            .map(|at| &fleets[at])
            .ok_or_else(|| {
                io::Error::other("viewing navy's fleet roster has an unresolved handle")
            })?;
        let name = match &fleet.name {
            replay_data::rows::Recorded::Value(Some(name)) => String::from_utf8(name.clone())
                .map_err(|_| {
                    io::Error::other("fleet name is not text in the recording language")
                })?,
            _ => return Err(io::Error::other("fleet name is unavailable")),
        };
        result.push((fleet.id, name));
    }
    Ok(result)
}

pub fn base_members(frame: &DecodedFrame, viewer: u32, province: u32) -> io::Result<Vec<u64>> {
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(|e| io::Error::other(e.to_string()))?;
    let scores = frame
        .complete::<tables::CountryGameScoreInputs>()
        .map_err(|e| io::Error::other(e.to_string()))?;
    let country = scores
        .iter()
        .find(|row| {
            links
                .binary_search_by_key(&row.country, |link| link.id)
                .ok()
                .is_some_and(|at| links[at].canonical_slot == viewer)
        })
        .ok_or_else(|| io::Error::other("viewing navy's fleet roster is absent"))?;
    if !country.navy_error.is_empty() {
        return Err(io::Error::other("viewing navy's fleet roster is unreadable"));
    }
    if country.fleets.is_empty() {
        return Ok(Vec::new());
    }
    let forces = frame
        .complete::<tables::TaskForces>()
        .map_err(|e| io::Error::other(e.to_string()))?;
    let ships = frame
        .complete::<tables::Ships>()
        .map_err(|e| io::Error::other(e.to_string()))?;
    let queue = frame
        .complete::<tables::NavalRepairQueue>()
        .map_err(|e| io::Error::other(e.to_string()))?;
    let queued: std::collections::HashSet<u64> = queue
        .iter()
        .filter(|row| row.province == province)
        .filter_map(|row| {
            ships
                .binary_search_by_key(&row.ship, |ship| ship.id)
                .ok()
                .map(|at| ships[at].task_force)
        })
        .collect();
    let viewing_fleets: std::collections::HashSet<u64> = country
        .fleets
        .iter()
        .map(|occurrence| occurrence.fleet)
        .collect();
    let mut fleet_forces: std::collections::HashMap<u64, Vec<u64>> =
        std::collections::HashMap::with_capacity(viewing_fleets.len());
    let mut contributing_fleets: std::collections::HashSet<u64> = std::collections::HashSet::new();
    for force in forces {
        if viewing_fleets.contains(&force.fleet) {
            fleet_forces.entry(force.fleet).or_default().push(force.id);
            if force.province == province || queued.contains(&force.id) {
                contributing_fleets.insert(force.fleet);
            }
        }
    }
    if contributing_fleets.is_empty() {
        return Ok(Vec::new());
    }
    let (fleet_inputs, truncated_missing) =
        match viewer_foundation::tables::rows::<tables::FleetCounterInputs>(frame)? {
            viewer_foundation::tables::Available::Complete(rows) => (rows, None),
            viewer_foundation::tables::Available::Truncated { rows, missing } => {
                (rows, Some(missing))
            }
            viewer_foundation::tables::Available::Unavailable { reason } => {
                return Err(io::Error::other(format!(
                    "fleet task-force order is unavailable: {reason}"
                )));
            }
            viewer_foundation::tables::Available::NotRecorded { mode } => {
                return Err(io::Error::other(format!(
                    "fleet task-force order is unavailable: not recorded in a {mode} recording"
                )));
            }
        };
    let mut members = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for occurrence in &country.fleets {
        if !contributing_fleets.contains(&occurrence.fleet) {
            continue;
        }
        let roster = fleet_inputs
            .binary_search_by_key(&occurrence.fleet, |row| row.fleet)
            .ok()
            .map(|at| &fleet_inputs[at])
            .ok_or_else(|| match truncated_missing {
                Some(missing) => io::Error::other(format!(
                    "fleet task-force order is unavailable: viewing navy's fleet is absent from truncated FleetCounterInputs ({missing} missing rows)"
                )),
                None => io::Error::other(
                    "fleet task-force order is unavailable: viewing navy's fleet is absent from FleetCounterInputs",
                ),
            })?;
        let listed: std::collections::HashSet<u64> = roster.task_forces.iter().copied().collect();
        if let Some(expected) = fleet_forces.get(&occurrence.fleet)
            && expected.iter().any(|id| !listed.contains(id))
        {
            return Err(io::Error::other(
                "fleet task-force order is unavailable: FleetCounterInputs is missing a task force of the viewing navy's fleet",
            ));
        }
        for &handle in &roster.task_forces {
            let force = forces
                .binary_search_by_key(&handle, |force| force.id)
                .ok()
                .map(|at| &forces[at])
                .ok_or_else(|| {
                    io::Error::other("fleet task-force order has an unresolved task-force handle")
                })?;
            if force.fleet != occurrence.fleet {
                return Err(io::Error::other(
                    "fleet task-force order is contradictory: FleetCounterInputs lists a task force belonging to another fleet",
                ));
            }
            if (force.province == province || queued.contains(&force.id)) && seen.insert(force.id) {
                members.push(force.id);
            }
        }
    }
    Ok(members)
}

pub fn mission_members(
    frame: &DecodedFrame,
    viewer: u32,
    region: u32,
    kind: u32,
) -> io::Result<Vec<u64>> {
    if kind == 0 || kind > i32::MAX as u32 {
        return Ok(Vec::new());
    }
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(|e| io::Error::other(e.to_string()))?;
    let ledger = frame
        .complete::<tables::NavalMissionLedgers>()
        .map_err(|e| io::Error::other(e.to_string()))?;
    let forces = frame
        .complete::<tables::TaskForces>()
        .map_err(|e| io::Error::other(e.to_string()))?;
    let mut members = Vec::new();
    for row in ledger.iter().filter(|row| row.region == region) {
        let slot = links
            .binary_search_by_key(&row.owner, |link| link.id)
            .ok()
            .map(|at| links[at].canonical_slot)
            .ok_or_else(|| io::Error::other("naval mission ledger owner is unresolved"))?;
        if slot != viewer {
            continue;
        }
        for &handle in &row.task_forces {
            let force = forces
                .binary_search_by_key(&handle, |force| force.id)
                .ok()
                .map(|at| &forces[at])
                .ok_or_else(|| io::Error::other("naval mission registration has no task force"))?;
            if force.mission == kind {
                members.push(handle);
            }
        }
    }
    Ok(members)
}

#[cfg(test)]
mod tests {
    use viewer_counters_naval::naval::home_bases;
    use super::*;
    use replay_data::{
        rows::CountryLinkRow, rows::NavalMissionLedgerRow, rows::TableRows, rows::TaskForceRow,
    };
    use viewer_foundation::test_support::{test_frame, test_put as put};

    #[test]
    fn mission_click_keeps_registered_order_repeats_and_off_region_or_fighting_forces() {
        let mut frame = test_frame(1);
        put(
            &mut frame,
            TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 7,
                    canonical_slot: 3,
                },
                CountryLinkRow {
                    id: 8,
                    canonical_slot: 4,
                },
            ]),
        );
        put(
            &mut frame,
            TableRows::TaskForces(vec![
                TaskForceRow {
                    id: 1,
                    mission: 2,
                    ..Default::default()
                },
                TaskForceRow {
                    id: 2,
                    mission: 1,
                    ..Default::default()
                },
                TaskForceRow {
                    id: 3,
                    mission: 2,
                    province: 99,
                    combat_references: 1,
                    ..Default::default()
                },
            ]),
        );
        put(
            &mut frame,
            TableRows::NavalMissionLedgers(vec![
                NavalMissionLedgerRow {
                    owner: 7,
                    region: 5,
                    task_forces: vec![3, 2, 1, 3],
                    ..Default::default()
                },
                NavalMissionLedgerRow {
                    owner: 7,
                    region: 6,
                    task_forces: vec![2],
                    ..Default::default()
                },
                NavalMissionLedgerRow {
                    owner: 8,
                    region: 5,
                    task_forces: vec![1],
                    ..Default::default()
                },
            ]),
        );
        assert_eq!(mission_members(&frame, 3, 5, 2).unwrap(), [3, 1, 3]);
        assert_eq!(mission_members(&frame, 3, 5, 1).unwrap(), [2]);
        assert_eq!(mission_members(&frame, 4, 5, 2).unwrap(), [1]);
        assert!(mission_members(&frame, 3, 5, 0).unwrap().is_empty());
    }
    #[test]
    fn explicit_fleet_selection_keeps_order_and_home_bases_are_owned_live_fleets() {
        use replay_data::{rows::FleetRow, rows::Recorded};
        let mut frame = test_frame(1);
        put(
            &mut frame,
            TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 7,
                    canonical_slot: 3,
                },
                CountryLinkRow {
                    id: 8,
                    canonical_slot: 4,
                },
            ]),
        );
        put(
            &mut frame,
            TableRows::Fleets(vec![
                FleetRow {
                    id: 1,
                    owner: 7,
                    home_base: 10,
                    name: Recorded::Value(Some(b"First".to_vec())),
                    ..Default::default()
                },
                FleetRow {
                    id: 2,
                    owner: 8,
                    home_base: 20,
                    ..Default::default()
                },
                FleetRow {
                    id: 3,
                    owner: 7,
                    home_base: 10,
                    ..Default::default()
                },
            ]),
        );
        let mut selection = Selection::default();
        selection.observe(1, &frame);
        selection.click(3, false);
        selection.click(1, true);
        selection.click(2, true);
        assert_eq!(selection.fleets().as_slice(), [3, 1, 2]);
        assert_eq!(home_bases(&frame, 3, selection.fleets()).unwrap(), [10]);
        selection.click(1, true);
        assert_eq!(selection.fleets().as_slice(), [3, 2]);
        selection.click(1, false);
        assert_eq!(selection.fleets().as_slice(), [1]);
        put(&mut frame, TableRows::Fleets(Vec::new()));
        selection.observe(1, &frame);
        assert!(selection.fleets().is_empty());
    }

    #[test]
    fn a_naval_base_click_takes_own_task_forces_there_or_queued_for_its_repairs() {
        use replay_data::rows::{
            CountryGameScoreInputRow, GameScoreFleetCount, NavalRepairQueueRow, ShipRow,
        };
        let mut frame = test_frame(1);
        put(
            &mut frame,
            TableRows::CountryLinks(vec![
                CountryLinkRow { id: 7, canonical_slot: 3 },
                CountryLinkRow { id: 8, canonical_slot: 4 },
            ]),
        );
        let force = |id, fleet, province| TaskForceRow { id, fleet, province, ..Default::default() };
        put(
            &mut frame,
            TableRows::TaskForces(vec![
                force(1, 10, 5),
                force(2, 11, 5),
                force(3, 11, 99),
                force(4, 11, 98),
                force(5, 20, 5),
            ]),
        );
        put(
            &mut frame,
            TableRows::Ships(vec![
                ShipRow { id: 30, task_force: 3, ..Default::default() },
                ShipRow { id: 40, task_force: 4, ..Default::default() },
            ]),
        );
        put(
            &mut frame,
            TableRows::NavalRepairQueue(vec![
                NavalRepairQueueRow { province: 5, ordinal: 0, ship: 30 },
                NavalRepairQueueRow { province: 6, ordinal: 0, ship: 40 },
            ]),
        );
        let roster = |country, fleets: &[u64]| CountryGameScoreInputRow {
            country,
            fleets: fleets
                .iter()
                .map(|&fleet| GameScoreFleetCount { fleet, ..Default::default() })
                .collect(),
            ..Default::default()
        };
        put(
            &mut frame,
            TableRows::CountryGameScoreInputs(vec![roster(7, &[11, 10]), roster(8, &[20])]),
        );
        put(
            &mut frame,
            TableRows::FleetCounterInputs(vec![
                replay_data::rows::FleetCounterInputRow {
                    fleet: 10,
                    task_forces: vec![1],
                    ..Default::default()
                },
                replay_data::rows::FleetCounterInputRow {
                    fleet: 11,
                    task_forces: vec![2, 3, 4],
                    ..Default::default()
                },
                replay_data::rows::FleetCounterInputRow {
                    fleet: 20,
                    task_forces: vec![5],
                    ..Default::default()
                },
            ]),
        );
        assert_eq!(base_members(&frame, 3, 5).unwrap(), [2, 3, 1]);
        assert_eq!(base_members(&frame, 4, 5).unwrap(), [5]);
        assert!(base_members(&frame, 3, 7).unwrap().is_empty());
    }

    #[test]
    fn fleet_choices_use_country_roster_order_including_empty_fleets() {
        use replay_data::{
            rows::CountryGameScoreInputRow, rows::FleetRow, rows::GameScoreFleetCount,
            rows::Recorded,
        };
        let mut frame = test_frame(1);
        put(
            &mut frame,
            TableRows::CountryLinks(vec![CountryLinkRow {
                id: 7,
                canonical_slot: 3,
            }]),
        );
        put(
            &mut frame,
            TableRows::Fleets(vec![
                FleetRow {
                    id: 1,
                    owner: 7,
                    name: Recorded::Value(Some(b"First".to_vec())),
                    ..Default::default()
                },
                FleetRow {
                    id: 2,
                    owner: 7,
                    name: Recorded::Value(Some(b"Empty".to_vec())),
                    ..Default::default()
                },
            ]),
        );
        put(
            &mut frame,
            TableRows::CountryGameScoreInputs(vec![CountryGameScoreInputRow {
                country: 7,
                fleets: vec![
                    GameScoreFleetCount {
                        fleet: 2,
                        task_forces: Some(0),
                        ..Default::default()
                    },
                    GameScoreFleetCount {
                        fleet: 1,
                        task_forces: Some(2),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }]),
        );
        assert_eq!(
            fleet_choices(&frame, 3).unwrap(),
            [(2, "Empty".into()), (1, "First".into())]
        );
    }

    #[test]
    fn base_members_uses_fleet_counter_inputs_order_and_refuses_incomplete_roster() {
        use replay_data::rows::{
            CountryGameScoreInputRow, FleetCounterInputRow, GameScoreFleetCount,
            NavalRepairQueueRow, ShipRow, TableId,
        };
        use replay_frame::frame::TableView;
        let mut frame = test_frame(1);
        put(
            &mut frame,
            TableRows::CountryLinks(vec![CountryLinkRow {
                id: 7,
                canonical_slot: 3,
            }]),
        );
        let force = |id, fleet, province| TaskForceRow {
            id,
            fleet,
            province,
            ..Default::default()
        };
        put(
            &mut frame,
            TableRows::TaskForces(vec![
                force(1, 10, 5),
                force(2, 11, 5),
                force(3, 11, 99),
                force(4, 11, 5),
            ]),
        );
        put(
            &mut frame,
            TableRows::Ships(vec![ShipRow {
                id: 30,
                task_force: 3,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::NavalRepairQueue(vec![NavalRepairQueueRow {
                province: 5,
                ordinal: 0,
                ship: 30,
            }]),
        );
        put(
            &mut frame,
            TableRows::CountryGameScoreInputs(vec![CountryGameScoreInputRow {
                country: 7,
                fleets: [11, 10]
                    .into_iter()
                    .map(|fleet| GameScoreFleetCount {
                        fleet,
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::FleetCounterInputs(vec![
                FleetCounterInputRow {
                    fleet: 10,
                    task_forces: vec![1],
                    ..Default::default()
                },
                FleetCounterInputRow {
                    fleet: 11,
                    task_forces: vec![4, 3, 2],
                    ..Default::default()
                },
            ]),
        );
        assert_eq!(base_members(&frame, 3, 5).unwrap(), [4, 3, 2, 1]);

        put(
            &mut frame,
            TableRows::FleetCounterInputs(vec![
                FleetCounterInputRow {
                    fleet: 10,
                    task_forces: vec![1],
                    ..Default::default()
                },
                FleetCounterInputRow {
                    fleet: 11,
                    task_forces: vec![4, 3, 4, 2],
                    ..Default::default()
                },
            ]),
        );
        assert_eq!(base_members(&frame, 3, 5).unwrap(), [4, 3, 2, 1]);

        *frame.tables[TableId::FleetCounterInputs.index()] = TableView::Truncated {
            rows: TableRows::FleetCounterInputs(vec![
                FleetCounterInputRow {
                    fleet: 10,
                    task_forces: vec![1],
                    ..Default::default()
                },
                FleetCounterInputRow {
                    fleet: 11,
                    task_forces: vec![4, 3, 2],
                    ..Default::default()
                },
            ]),
            missing: 1,
        };
        assert_eq!(base_members(&frame, 3, 5).unwrap(), [4, 3, 2, 1]);

        put(
            &mut frame,
            TableRows::TaskForces(vec![
                force(1, 10, 5),
                force(2, 11, 5),
                force(3, 11, 99),
                force(4, 11, 5),
                force(5, 20, 5),
            ]),
        );
        put(
            &mut frame,
            TableRows::FleetCounterInputs(vec![
                FleetCounterInputRow {
                    fleet: 10,
                    task_forces: vec![1, 5],
                    ..Default::default()
                },
                FleetCounterInputRow {
                    fleet: 11,
                    task_forces: vec![4, 3, 2],
                    ..Default::default()
                },
            ]),
        );
        assert!(base_members(&frame, 3, 5).is_err());

        put(
            &mut frame,
            TableRows::TaskForces(vec![
                force(1, 10, 5),
                force(2, 11, 5),
                force(3, 11, 99),
                force(4, 11, 5),
            ]),
        );
        put(
            &mut frame,
            TableRows::FleetCounterInputs(vec![
                FleetCounterInputRow {
                    fleet: 10,
                    task_forces: vec![1],
                    ..Default::default()
                },
                FleetCounterInputRow {
                    fleet: 11,
                    task_forces: vec![4, 2],
                    ..Default::default()
                },
            ]),
        );
        assert!(base_members(&frame, 3, 5).is_err());

        *frame.tables[TableId::FleetCounterInputs.index()] = TableView::Unavailable {
            reason: "older recording".into(),
        };
        assert!(base_members(&frame, 3, 5).is_err());
        assert!(base_members(&frame, 3, 7).unwrap().is_empty());

        put(
            &mut frame,
            TableRows::TaskForces(vec![
                force(1, 10, 88),
                force(2, 11, 5),
                force(3, 11, 99),
                force(4, 11, 5),
            ]),
        );
        *frame.tables[TableId::FleetCounterInputs.index()] = TableView::Truncated {
            rows: TableRows::FleetCounterInputs(vec![FleetCounterInputRow {
                fleet: 11,
                task_forces: vec![4, 3, 2],
                ..Default::default()
            }]),
            missing: 1,
        };
        assert_eq!(base_members(&frame, 3, 5).unwrap(), [4, 3, 2]);
        assert!(base_members(&frame, 3, 88).is_err());

        *frame.tables[TableId::FleetCounterInputs.index()] = TableView::Unavailable {
            reason: "older recording".into(),
        };
        put(
            &mut frame,
            TableRows::CountryGameScoreInputs(vec![CountryGameScoreInputRow {
                country: 7,
                fleets: Vec::new(),
                ..Default::default()
            }]),
        );
        assert!(base_members(&frame, 3, 5).unwrap().is_empty());
    }
}
