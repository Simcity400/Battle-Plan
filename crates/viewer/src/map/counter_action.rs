//! Land-counter action frames from recorded simulation state.
//!
//! `Unit_GetActionCode@140BEF850` classifies movement, not battle-plan orders.
//! `1418C5DAF` classifies the representative before `1418C632E` compares fully
//! visible division members. `1418C645E` writes `ActionCodeToFrame@140BEF9F0`:
//! code + 1 in native one-based frames. Own code 1 hides the child; mixed code
//! 0 shows the first frame. The sprite and placement remain authored GUI facts.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use replay::{
    frame::DecodedFrame,
    rows::{
        DivisionMovementRow, NavalCombatTransferRow, NavalTransferUnitRow, StrategicAction,
        StrategicActionFailure, tables,
    },
};

use super::{tables::rows, visibility::Relations};

pub(super) type Action = Result<u8, &'static str>;

/// Missing current-hour operands for the native `140C79B70` decision.
pub(super) const CURRENT_RAIL_MODIFIERS: &str = "current strategic-redeployment modifier operands";
pub(super) const STRATEGIC_LAYOUT: &str = "division strategic action fixed layout";

/// `140BEF850`, class 0. Branch operands are consulted only where native does.
pub(super) fn action_code(
    movement: &DivisionMovementRow,
    any_attacking: Option<bool>,
    destination_enemy: Result<bool, &'static str>,
    transfer_in_combat: Option<bool>,
) -> Action {
    if (movement.path_total as i32) <= 0 {
        return if idle(movement.path_total, movement.combat_count) {
            // CArmy primary vtable +0x178 is ReturnZero@14011CD10.
            Ok(1)
        } else {
            any_attacking
                .map(|attacking| if attacking { 5 } else { 2 })
                .ok_or("division combat-reference attacker countries")
        };
    }
    if movement.target != 0 {
        return Ok(3);
    }
    if movement.retreat != 0 {
        return Ok(10);
    }
    if movement.strategic_redeployment != 0 {
        return match movement.strategic_action {
            StrategicAction::Available(inputs) => Ok(if inputs.blocked() { 11 } else { 6 }),
            StrategicAction::Withheld(reason) => Err(reason.reason()),
            StrategicAction::Unreached => Err(CURRENT_RAIL_MODIFIERS),
        };
    }
    if movement.naval_transfer != 0
        && transfer_in_combat.ok_or("naval-transfer combat membership")?
    {
        return Ok(9);
    }
    route_tail(
        destination_enemy,
        movement.naval_transfer != 0,
        movement.combat_count,
    )
}

/// Signed route/combat guards in `Unit_GetActionCode@140BEF850`.
pub(super) fn idle(path_count: u32, combat_count: u32) -> bool {
    (path_count as i32) <= 0 && (combat_count as i32) <= 0
}

/// Common route tail in `140BEF850`, after target, retreat, rail and transfer combat.
pub(super) fn route_tail(
    enemy: Result<bool, &'static str>,
    transfer: bool,
    combats: u32,
) -> Action {
    Ok(match (enemy?, transfer) {
        (true, true) => 8,
        (true, false) => 5,
        (false, true) => 7,
        (false, false) if (combats as i32) > 0 => 2,
        (false, false) => 4,
    })
}

/// `140BF0EF0`: ordinary-route destination controller, raw identity first,
/// then canonical country identity and active war (`1406F3A00`).
pub(super) struct DestinationHostility<'a> {
    controllers: BTreeMap<u32, u32>,
    links: &'a BTreeMap<u32, u32>,
    relations: Result<&'a Relations, &'static str>,
}

impl<'a> DestinationHostility<'a> {
    pub(super) fn read(
        frame: &DecodedFrame,
        links: &'a BTreeMap<u32, u32>,
        relations: Result<&'a Relations, &'static str>,
    ) -> io::Result<Self> {
        Ok(Self {
            controllers: rows::<tables::Provinces>(frame)?
                .rows()
                .iter()
                .map(|row| (row.id, row.controller))
                .collect(),
            links,
            relations,
        })
    }

    pub(super) fn destination_enemy(
        &self,
        path: &[u32],
        total: u32,
        owner: Option<u32>,
    ) -> Result<bool, &'static str> {
        if path.len() != total as usize {
            return Err("last ordinary-route province hostility");
        }
        let last = *path
            .last()
            .ok_or("last ordinary-route province hostility")?;
        // Province provider@1401C9D30 rejects signed nonpositive IDs.
        if (last as i32) <= 0 {
            return Ok(false);
        }
        let controller = *self
            .controllers
            .get(&last)
            .ok_or("last ordinary-route province controller")?;
        if (controller as i32) <= 0 {
            return Ok(false);
        }
        let owner = owner.ok_or("action logical owner")?;
        if controller == owner {
            return Ok(false);
        }
        let controller = *self
            .links
            .get(&controller)
            .ok_or("action controller CountryLinks")?;
        let owner = *self.links.get(&owner).ok_or("action owner CountryLinks")?;
        if controller == owner {
            return Ok(false);
        }
        Ok(self.relations?.at_war(controller, owner))
    }
}

/// `140E111B0/140E11360` link a unit in both directions; `140E13F20`
/// removes both links. `1415B1960 -> 140E110D0` links a registered combat
/// to its transfer in both directions; `1415B0500 -> 140E13180` removes it.
/// The hourly inverse census therefore proves the transfer's +0x24 nonzero
/// test without another recorded field. A prefix proves positives only.
struct TransferCombatUnits {
    units: BTreeSet<u64>,
    complete: bool,
}

impl TransferCombatUnits {
    fn from_rows(
        combats: &[NavalCombatTransferRow],
        transfers: &[NavalTransferUnitRow],
        complete: bool,
    ) -> Self {
        let named: BTreeSet<u64> = combats
            .iter()
            .flat_map(|combat| combat.transfers.iter().copied())
            .filter(|&transfer| transfer != 0)
            .collect();
        Self {
            units: transfers
                .iter()
                .filter(|transfer| named.contains(&transfer.transfer))
                .flat_map(|transfer| transfer.units.iter().copied())
                .collect(),
            complete,
        }
    }

    fn read(frame: &DecodedFrame) -> io::Result<Self> {
        let combats = rows::<tables::NavalCombatTransfers>(frame)?;
        let transfers = rows::<tables::NavalTransferUnits>(frame)?;
        Ok(Self::from_rows(
            combats.rows(),
            transfers.rows(),
            combats.is_complete() && transfers.is_complete(),
        ))
    }

    fn get(&self, division: u64) -> Option<bool> {
        if self.units.contains(&division) {
            Some(true)
        } else {
            self.complete.then_some(false)
        }
    }
}

/// One hour's class-0 actions for the viewing country's roster, independent of camera.
#[derive(Clone, Default)]
pub(super) struct Actions {
    by_division: BTreeMap<u64, Action>,
    movement_complete: bool,
}

impl Actions {
    pub(super) fn read(
        frame: &DecodedFrame,
        hostility: &DestinationHostility<'_>,
        ours: Option<u32>,
    ) -> io::Result<Self> {
        let movements = rows::<tables::DivisionMovement>(frame)?;
        let divisions = rows::<tables::Divisions>(frame)?;
        let owners: BTreeMap<u64, u32> = divisions
            .rows()
            .iter()
            .map(|division| (division.id, division.logical_owner))
            .collect();
        let own: BTreeSet<u64> = divisions
            .rows()
            .iter()
            .filter(|division| {
                ours.is_none_or(|ours| hostility.links.get(&division.roster_owner) == Some(&ours))
            })
            .map(|division| division.id)
            .collect();
        let transfers = TransferCombatUnits::read(frame)?;
        let standing = standing_attacks(
            frame,
            movements.rows(),
            &owners,
            hostility.links,
            ours.map(|_| &own),
        )?;
        let strategic_layout =
            replay::fixed_layout::validate_recording(frame, replay::fixed_layout::STRATEGIC_ACTION);
        let by_division = movements
            .rows()
            .iter()
            .filter(|movement| ours.is_none() || own.contains(&movement.division))
            .map(|movement| {
                let transfer = transfers.get(movement.division);
                let needs_enemy = (movement.path_total as i32) > 0
                    && movement.target == 0
                    && movement.retreat == 0
                    && movement.strategic_redeployment == 0
                    && (movement.naval_transfer == 0 || transfer == Some(false));
                (
                    movement.division,
                    admit_strategic_layout(
                        action_code(
                            movement,
                            standing.get(&movement.division).copied().flatten(),
                            if needs_enemy {
                                hostility.destination_enemy(
                                    &movement.path,
                                    movement.path_total,
                                    owners.get(&movement.division).copied(),
                                )
                            } else {
                                Err("last ordinary-route province hostility")
                            },
                            transfer,
                        ),
                        strategic_layout.is_ok(),
                    ),
                )
            })
            .collect();
        Ok(Self {
            by_division,
            movement_complete: movements.is_complete(),
        })
    }

    pub(super) fn get(&self, division: u64) -> Action {
        self.by_division
            .get(&division)
            .copied()
            .unwrap_or(Err(if self.movement_complete {
                "DivisionMovement division"
            } else {
                "DivisionMovement"
            }))
    }
}

/// `1413CB9C0/1413CBD30 -> 140BEA140` add both the combat-side member
/// and the unit's combat pointer; removal (`1413CE250/140BF48D0`) removes
/// both. `LandCombats` plus the attacker's primary +0x38 country set can
/// therefore prove any-attacking. A negative also requires every reference
/// to be covered; naval or missing references remain a named gap.
fn standing_attacks(
    frame: &DecodedFrame,
    movements: &[DivisionMovementRow],
    owners: &BTreeMap<u64, u32>,
    links: &BTreeMap<u32, u32>,
    own: Option<&BTreeSet<u64>>,
) -> io::Result<BTreeMap<u64, Option<bool>>> {
    let wanted: BTreeSet<u64> = movements
        .iter()
        .filter(|movement| {
            own.is_none_or(|own| own.contains(&movement.division))
                && (movement.path_total as i32) <= 0
                && (movement.combat_count as i32) > 0
        })
        .map(|movement| movement.division)
        .collect();
    if wanted.is_empty() {
        return Ok(BTreeMap::new());
    }
    let combats = rows::<tables::LandCombats>(frame)?;
    let sides = rows::<tables::LandCombatSides>(frame)?;
    let sides: BTreeMap<_, _> = sides
        .rows()
        .iter()
        .map(|side| (side.combat, side.attacker_primary.as_slice()))
        .collect();
    let mut membership: BTreeMap<u64, BTreeSet<u64>> = BTreeMap::new();
    for combat in combats.rows() {
        for &division in combat.attackers.iter().chain(&combat.defenders) {
            if wanted.contains(&division) {
                membership.entry(division).or_default().insert(combat.id);
            }
        }
    }
    Ok(movements
        .iter()
        .filter(|movement| wanted.contains(&movement.division))
        .map(|movement| {
            let result = (|| {
                let owner = *owners.get(&movement.division)?;
                let references = membership.get(&movement.division)?;
                let mut unknown = false;
                for reference in references {
                    let Some(countries) = sides.get(reference) else {
                        unknown = true;
                        continue;
                    };
                    for &country in *countries {
                        if country == owner {
                            return Some(true);
                        }
                        if country != 0 && owner != 0 {
                            match (links.get(&country), links.get(&owner)) {
                                (Some(country), Some(owner)) if country == owner => {
                                    return Some(true);
                                }
                                (Some(_), Some(_)) => {}
                                _ => unknown = true,
                            }
                        }
                    }
                }
                (!unknown
                    && combats.is_complete()
                    && references.len() == movement.combat_count as usize)
                    .then_some(false)
            })();
            (movement.division, result)
        })
        .collect())
}

/// A fixed-key failure affects only the two strategic-redeployment actions.
fn admit_strategic_layout(action: Action, admitted: bool) -> Action {
    if !admitted && matches!(action, Ok(6 | 11)) {
        Err(STRATEGIC_LAYOUT)
    } else {
        action
    }
}

fn strategic_unknown(reason: &str) -> bool {
    reason == CURRENT_RAIL_MODIFIERS
        || reason == STRATEGIC_LAYOUT
        || [
            StrategicActionFailure::Owner,
            StrategicActionFailure::CountryModifier,
            StrategicActionFailure::CurrentState,
            StrategicActionFailure::StateModifier,
            StrategicActionFailure::Friendly,
            StrategicActionFailure::FriendlyStateModifier,
        ]
        .iter()
        .any(|failure| failure.reason() == reason)
}

/// Representative unconditional, then fully visible members only. Different
/// known actions prove mixed 0. A strategic gap permits only 6/11 (140BEF850),
/// so a participating known action outside that pair also proves mixed 0.
pub(super) fn aggregate_actions(
    representative: Action,
    members: impl IntoIterator<Item = (bool, Action)>,
) -> Action {
    let mut known = representative.ok();
    let mut unknown = representative.err();
    let mut rail_unknown = unknown.is_some_and(strategic_unknown);
    for (fully_visible, action) in members {
        if !fully_visible {
            continue;
        }
        match action {
            Ok(code) => match known {
                Some(previous) if previous != code => return Ok(0),
                _ => known = Some(code),
            },
            Err(reason) => {
                rail_unknown |= strategic_unknown(reason);
                unknown.get_or_insert(reason);
            }
        }
        if rail_unknown && known.is_some_and(|code| !matches!(code, 6 | 11)) {
            return Ok(0);
        }
    }
    unknown.map_or_else(|| Ok(known.unwrap_or(0)), Err)
}

/// Zero-based frame of the authored `onmap_action_icon` child.
pub(super) fn action_frame(code: u8, own: bool) -> Option<u32> {
    (own && code != 1).then_some(u32::from(code))
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::{
        frame::{Metrics, TableView},
        rows::{DivisionRow, LandCombatRow, LandCombatSideRow, ProvinceRow, TableId, TableRows},
    };

    fn read_actions(
        frame: &DecodedFrame,
        links: &BTreeMap<u32, u32>,
        relations: Result<&Relations, &'static str>,
    ) -> io::Result<Actions> {
        Actions::read(
            frame,
            &DestinationHostility::read(frame, links, relations)?,
            None,
        )
    }

    fn set(frame: &mut DecodedFrame, table: TableRows) {
        let index = table.table().index();
        *frame.tables[index] = TableView::Complete(table);
    }

    fn moving() -> DivisionMovementRow {
        DivisionMovementRow {
            division: 7,
            path_total: 1,
            path: vec![20],
            ..Default::default()
        }
    }

    #[test]
    fn counter_action_strategic_layout_admits_every_other_action() {
        for code in 0..=12 {
            assert_eq!(
                admit_strategic_layout(Ok(code), false),
                if matches!(code, 6 | 11) {
                    Err(STRATEGIC_LAYOUT)
                } else {
                    Ok(code)
                }
            );
        }
        assert_eq!(
            admit_strategic_layout(Err("query unavailable"), false),
            Err("query unavailable")
        );
    }

    #[test]
    fn counter_action_strategic_gaps_preserve_proven_mixed_frames() {
        for reason in [
            STRATEGIC_LAYOUT,
            StrategicActionFailure::StateModifier.reason(),
        ] {
            assert_eq!(aggregate_actions(Err(reason), [(true, Ok(4))]), Ok(0));
            assert_eq!(aggregate_actions(Ok(8), [(true, Err(reason))]), Ok(0));
            assert_eq!(aggregate_actions(Ok(6), [(true, Err(reason))]), Err(reason));
            assert_eq!(
                aggregate_actions(Ok(11), [(true, Err(reason))]),
                Err(reason)
            );
            assert_eq!(aggregate_actions(Ok(4), [(false, Err(reason))]), Ok(4));
        }
    }

    #[test]
    fn counter_action_native_actions_consult_only_the_reached_branch() {
        let mut row = DivisionMovementRow::default();
        assert_eq!(
            action_code(
                &row,
                None,
                Err("last ordinary-route province hostility"),
                None
            ),
            Ok(1)
        );
        row.combat_count = 2;
        assert_eq!(
            action_code(
                &row,
                Some(true),
                Err("last ordinary-route province hostility"),
                None
            ),
            Ok(5)
        );
        assert_eq!(
            action_code(
                &row,
                Some(false),
                Err("last ordinary-route province hostility"),
                None
            ),
            Ok(2)
        );
        assert!(
            action_code(
                &row,
                None,
                Err("last ordinary-route province hostility"),
                None
            )
            .is_err()
        );
        row = moving();
        row.target = 10;
        row.retreat = 1;
        row.strategic_redeployment = 1;
        row.naval_transfer = 1;
        assert_eq!(
            action_code(
                &row,
                None,
                Err("last ordinary-route province hostility"),
                None
            ),
            Ok(3)
        );
        row.target = 0;
        assert_eq!(
            action_code(
                &row,
                None,
                Err("last ordinary-route province hostility"),
                None
            ),
            Ok(10)
        );
        row.retreat = 0;
        assert_eq!(
            action_code(
                &row,
                None,
                Err("last ordinary-route province hostility"),
                None
            ),
            Err(CURRENT_RAIL_MODIFIERS)
        );
        row.strategic_redeployment = 0;
        assert_eq!(
            action_code(
                &row,
                None,
                Err("last ordinary-route province hostility"),
                Some(true)
            ),
            Ok(9)
        );
        assert_eq!(action_code(&row, None, Ok(true), Some(false)), Ok(8));
        assert_eq!(action_code(&row, None, Ok(false), Some(false)), Ok(7));
        assert!(action_code(&row, None, Ok(true), None).is_err());
        row.naval_transfer = 0;
        assert_eq!(action_code(&row, None, Ok(true), None), Ok(5));
        assert_eq!(action_code(&row, None, Ok(false), None), Ok(4));
        row.combat_count = 1;
        assert_eq!(action_code(&row, None, Ok(false), None), Ok(2));
        row.path_total = u32::MAX;
        row.combat_count = u32::MAX;
        assert_eq!(
            action_code(
                &row,
                None,
                Err("last ordinary-route province hostility"),
                None
            ),
            Ok(1)
        );
    }

    #[test]
    fn counter_action_signed_idle_and_shared_hostility_keep_native_guards() {
        assert!(idle(u32::MAX, u32::MAX));
        assert!(!idle(0, 1));
        let links = BTreeMap::new();
        let mut frame = frame();
        set(
            &mut frame,
            TableRows::Provinces(vec![ProvinceRow {
                id: 20,
                controller: 100,
                ..Default::default()
            }]),
        );
        let hostility =
            DestinationHostility::read(&frame, &links, Err("WarRelations missing")).unwrap();
        assert_eq!(hostility.destination_enemy(&[20], 1, Some(100)), Ok(false));
        assert_eq!(hostility.destination_enemy(&[u32::MAX], 1, None), Ok(false));
        assert!(hostility.destination_enemy(&[20], 2, Some(100)).is_err());
        let links = BTreeMap::from([(100, 1), (200, 2)]);
        let hostility =
            DestinationHostility::read(&frame, &links, Err("WarRelations missing")).unwrap();
        assert_eq!(
            hostility.destination_enemy(&[20], 1, Some(200)),
            Err("WarRelations missing")
        );
    }

    #[test]
    fn counter_action_transfer_inverse_prefix_proves_only_present_combat_units() {
        let combats = [NavalCombatTransferRow {
            combat: 40,
            transfers: vec![3, 3, 0],
        }];
        let transfers = [
            NavalTransferUnitRow {
                transfer: 3,
                units: vec![7, 8],
            },
            NavalTransferUnitRow {
                transfer: 4,
                units: vec![9],
            },
        ];
        let partial = TransferCombatUnits::from_rows(&combats, &transfers, false);
        assert_eq!(partial.get(7), Some(true));
        assert_eq!(partial.get(9), None);
        let complete = TransferCombatUnits::from_rows(&combats, &transfers, true);
        assert_eq!(complete.get(7), Some(true));
        assert_eq!(complete.get(9), Some(false));
    }

    #[test]
    fn counter_action_representative_and_full_members_select_native_frames() {
        assert_eq!(aggregate_actions(Ok(8), [(false, Ok(9))]), Ok(8));
        assert_eq!(aggregate_actions(Ok(8), [(true, Ok(9))]), Ok(0));
        assert_eq!(
            aggregate_actions(Err("missing"), [(true, Ok(8))]),
            Err("missing")
        );
        assert_eq!(
            aggregate_actions(Err("missing"), [(true, Ok(8)), (true, Ok(7))]),
            Ok(0)
        );
        assert_eq!(action_frame(8, true), Some(8));
        assert_eq!(action_frame(0, true), Some(0));
        assert_eq!(action_frame(1, true), None);
        assert_eq!(action_frame(8, false), None);
    }

    fn frame() -> DecodedFrame {
        DecodedFrame {
            branch: 1,
            hour: 10,
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "not recorded".into(),
                }
                .into()
            }),
            metrics: Metrics::default(),
        }
    }

    #[test]
    fn counter_action_recorded_hostile_transfer_selects_anchor_before_combat_override() {
        use replay::rows::WarRelationRow;
        let mut frame = frame();
        set(
            &mut frame,
            TableRows::DivisionMovement(vec![DivisionMovementRow {
                naval_transfer: 1,
                ..moving()
            }]),
        );
        set(
            &mut frame,
            TableRows::Divisions(vec![DivisionRow {
                id: 7,
                roster_owner: 300,
                logical_owner: 100,
                ..Default::default()
            }]),
        );
        set(
            &mut frame,
            TableRows::Provinces(vec![ProvinceRow {
                id: 20,
                controller: 200,
                ..Default::default()
            }]),
        );
        set(&mut frame, TableRows::FactionMembers(vec![]));
        set(&mut frame, TableRows::Subjects(vec![]));
        set(
            &mut frame,
            TableRows::WarRelations(vec![WarRelationRow {
                first_country: 100,
                second_country: 200,
                ..Default::default()
            }]),
        );
        set(&mut frame, TableRows::NavalCombatTransfers(vec![]));
        set(&mut frame, TableRows::NavalTransferUnits(vec![]));
        let links = BTreeMap::from([(100, 1), (200, 2), (300, 3)]);
        let relations = Relations::read(&frame, &links).unwrap();
        assert_eq!(
            read_actions(&frame, &links, Ok(&relations)).unwrap().get(7),
            Ok(8)
        );
        set(
            &mut frame,
            TableRows::NavalCombatTransfers(vec![NavalCombatTransferRow {
                combat: 40,
                transfers: vec![3],
            }]),
        );
        set(
            &mut frame,
            TableRows::NavalTransferUnits(vec![NavalTransferUnitRow {
                transfer: 3,
                units: vec![7],
            }]),
        );
        assert_eq!(
            read_actions(&frame, &links, Ok(&relations)).unwrap().get(7),
            Ok(9)
        );
    }

    #[test]
    fn counter_action_hour_refuses_missing_transfer_census_but_keeps_lazy_proofs() {
        let mut frame = frame();
        let mut row = moving();
        row.naval_transfer = 1;
        set(&mut frame, TableRows::DivisionMovement(vec![row.clone()]));
        set(
            &mut frame,
            TableRows::Divisions(vec![DivisionRow {
                id: 7,
                logical_owner: 100,
                ..Default::default()
            }]),
        );
        set(
            &mut frame,
            TableRows::Provinces(vec![ProvinceRow {
                id: 20,
                controller: 100,
                ..Default::default()
            }]),
        );
        let links = BTreeMap::from([(100, 1)]);
        let actions = read_actions(&frame, &links, Err("relations")).unwrap();
        assert_eq!(actions.get(7), Err("naval-transfer combat membership"));
        set(&mut frame, TableRows::NavalCombatTransfers(vec![]));
        set(&mut frame, TableRows::NavalTransferUnits(vec![]));
        let actions = read_actions(&frame, &links, Err("relations")).unwrap();
        assert_eq!(actions.get(7), Ok(7));
        row.path_total = 300;
        set(&mut frame, TableRows::DivisionMovement(vec![row.clone()]));
        let actions = read_actions(&frame, &links, Err("relations")).unwrap();
        assert_eq!(
            actions.get(7),
            Err("last ordinary-route province hostility")
        );
        row.target = 21;
        set(&mut frame, TableRows::DivisionMovement(vec![row]));
        *frame.tables[TableId::NavalCombatTransfers.index()] = TableView::Unavailable {
            reason: "not recorded".into(),
        };
        let actions = read_actions(&frame, &BTreeMap::new(), Err("relations")).unwrap();
        assert_eq!(actions.get(7), Ok(3));
    }

    #[test]
    fn counter_action_null_last_route_proves_nonhostile_without_countries() {
        let mut frame = frame();
        set(&mut frame, TableRows::NavalCombatTransfers(vec![]));
        set(&mut frame, TableRows::NavalTransferUnits(vec![]));
        for last in [0, u32::MAX] {
            let mut row = moving();
            row.path = vec![last];
            row.naval_transfer = 1;
            set(&mut frame, TableRows::DivisionMovement(vec![row]));
            let actions = read_actions(&frame, &BTreeMap::new(), Err("relations")).unwrap();
            assert_eq!(actions.get(7), Ok(7));
        }
    }

    #[test]
    fn counter_action_standing_scans_all_primary_countries_with_prefix_proofs() {
        let mut frame = frame();
        let movement = DivisionMovementRow {
            division: 7,
            combat_count: 2,
            combat_role: 2,
            ..Default::default()
        };
        let combats = vec![
            LandCombatRow {
                id: 40,
                defenders: vec![7],
                ..Default::default()
            },
            LandCombatRow {
                id: 41,
                attackers: vec![7],
                ..Default::default()
            },
        ];
        let mut sides = vec![
            LandCombatSideRow {
                combat: 40,
                attacker_primary: vec![200],
                attacker_secondary: vec![100],
                ..Default::default()
            },
            LandCombatSideRow {
                combat: 41,
                attacker_primary: vec![101],
                ..Default::default()
            },
        ];
        set(&mut frame, TableRows::LandCombats(combats.clone()));
        set(&mut frame, TableRows::LandCombatSides(sides.clone()));
        let owners = BTreeMap::from([(7, 100)]);
        let links = BTreeMap::from([(100, 1), (101, 1), (200, 2)]);
        let actions = standing_attacks(
            &frame,
            std::slice::from_ref(&movement),
            &owners,
            &links,
            None,
        )
        .unwrap();
        assert_eq!(actions[&7], Some(true));
        assert_eq!(
            action_code(&movement, actions[&7], Err("hostility"), None),
            Ok(5)
        );
        let signed = DivisionMovementRow {
            path_total: u32::MAX,
            ..movement.clone()
        };
        let actions = standing_attacks(&frame, &[signed], &owners, &links, None).unwrap();
        assert_eq!(actions[&7], Some(true));

        sides[1].attacker_primary = vec![200];
        set(&mut frame, TableRows::LandCombatSides(sides.clone()));
        let actions = standing_attacks(
            &frame,
            std::slice::from_ref(&movement),
            &owners,
            &links,
            None,
        )
        .unwrap();
        assert_eq!(
            actions[&7],
            Some(false),
            "secondary country set is not queried"
        );
        *frame.tables[TableId::LandCombats.index()] = TableView::Truncated {
            rows: TableRows::LandCombats(combats),
            missing: 1,
        };
        let actions = standing_attacks(
            &frame,
            std::slice::from_ref(&movement),
            &owners,
            &links,
            None,
        )
        .unwrap();
        assert_eq!(
            actions[&7], None,
            "a prefix cannot prove every reference absent"
        );

        sides[0].attacker_primary = vec![100];
        *frame.tables[TableId::LandCombatSides.index()] = TableView::Truncated {
            rows: TableRows::LandCombatSides(vec![sides[0].clone()]),
            missing: 1,
        };
        let actions =
            standing_attacks(&frame, &[movement], &owners, &BTreeMap::new(), None).unwrap();
        assert_eq!(
            actions[&7],
            Some(true),
            "raw equality proves true without links"
        );
    }
}
