
use viewer_foundation::unit_rules::idle;
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    io,
};

use replay_data::{
    rows::DivisionMovementRow, rows::NavalCombatTransferRow, rows::NavalTransferUnitRow,
    rows::StrategicAction, rows::StrategicActionFailure, rows::tables,
};
use replay_frame::frame::DecodedFrame;

use replay_derive::directed_wars::Owner;
use viewer_foundation::tables::rows;

pub type Action = Result<u8, &'static str>;

pub const CURRENT_RAIL_MODIFIERS: &str = "current strategic-redeployment modifier operands";
const STRATEGIC_NOT_RECORDED: &str = "DivisionMovement.strategic_action not recorded";
pub const STRATEGIC_LAYOUT: &str = "division strategic action fixed layout";

pub fn action_code(
    movement: &DivisionMovementRow,
    any_attacking: Option<bool>,
    destination_enemy: Result<bool, &'static str>,
    transfer_in_combat: Option<bool>,
) -> Action {
    if (movement.path_total as i32) <= 0 {
        return if idle(movement.path_total, movement.combat_count) {
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
        return match *movement
            .strategic_action
            .require("DivisionMovement.strategic_action")
            .map_err(|_| STRATEGIC_NOT_RECORDED)?
        {
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

pub fn route_tail(enemy: Result<bool, &'static str>, transfer: bool, combats: u32) -> Action {
    Ok(match (enemy?, transfer) {
        (true, true) => 8,
        (true, false) => 5,
        (false, true) => 7,
        (false, false) if (combats as i32) > 0 => 2,
        (false, false) => 4,
    })
}

pub struct DestinationHostility<'a> {
    frame: &'a DecodedFrame,
    controllers: BTreeMap<u32, u32>,
    links: &'a BTreeMap<u32, u32>,
    owners: RefCell<BTreeMap<u32, Result<Owner<'a>, String>>>,
}

impl<'a> DestinationHostility<'a> {
    pub fn read(frame: &'a DecodedFrame, links: &'a BTreeMap<u32, u32>) -> io::Result<Self> {
        Ok(Self {
            frame,
            controllers: rows::<tables::Provinces>(frame)?
                .rows()
                .iter()
                .map(|row| (row.id, row.controller))
                .collect(),
            links,
            owners: RefCell::default(),
        })
    }

    pub fn destination_enemy(
        &self,
        path: &[u32],
        total: u32,
        owner: Option<u32>,
    ) -> Result<bool, &'static str> {
        destination_enemy_by(
            path,
            total,
            owner,
            |province| self.controllers.get(&province).copied(),
            |controller, owner| {
                let mut owners = self.owners.borrow_mut();
                let receiver = owners
                    .entry(controller)
                    .or_insert_with(|| Owner::new(self.frame, controller));
                receiver
                    .as_ref()
                    .map_err(|_| "action destination directed-war receiver unavailable")?
                    .at_war(owner)
                    .map_err(|_| "action destination directed-war operands unavailable")
            },
        )
    }
}

pub fn destination_enemy_by(
    path: &[u32],
    total: u32,
    owner: Option<u32>,
    controller_of: impl Fn(u32) -> Option<u32>,
    at_war: impl FnOnce(u32, u32) -> Result<bool, &'static str>,
) -> Result<bool, &'static str> {
    if (total as i32) <= 0 {
        return Ok(false);
    }
    if path.len() != total as usize {
        return Err("last ordinary-route province hostility");
    }
    let last = *path
        .last()
        .ok_or("last ordinary-route province hostility")?;
    if (last as i32) <= 0 {
        return Ok(false);
    }
    let controller = controller_of(last).ok_or("last ordinary-route province controller")?;
    if (controller as i32) <= 0 {
        return Ok(false);
    }
    let owner = owner.ok_or("action logical owner")?;
    if controller == owner {
        return Ok(false);
    }
    at_war(controller, owner)
}

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
        transfer_membership(self.units.contains(&division), self.complete)
    }
}

pub fn transfer_membership(named: bool, complete: bool) -> Option<bool> {
    if named { Some(true) } else { complete.then_some(false) }
}

#[derive(Clone, Default)]
pub struct Actions {
    by_division: BTreeMap<u64, Action>,
    movement_complete: bool,
}

impl Actions {
    pub fn read(
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
        let strategic_layout = replay_derive::fixed_layout::validate_recording(
            frame,
            replay_derive::fixed_layout::STRATEGIC_ACTION,
        );
        let by_division = movements
            .rows()
            .iter()
            .filter(|movement| ours.is_none() || own.contains(&movement.division))
            .map(|movement| {
                (
                    movement.division,
                    division_action(
                        movement,
                        standing.get(&movement.division).copied().flatten(),
                        transfers.get(movement.division),
                        || {
                            hostility.destination_enemy(
                                &movement.path,
                                movement.path_total,
                                owners.get(&movement.division).copied(),
                            )
                        },
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

    pub fn get(&self, division: u64) -> Action {
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

pub fn division_action(
    movement: &DivisionMovementRow,
    standing: Option<bool>,
    transfer: Option<bool>,
    destination_enemy: impl FnOnce() -> Result<bool, &'static str>,
    strategic_layout: bool,
) -> Action {
    let needs_enemy = (movement.path_total as i32) > 0
        && movement.target == 0
        && movement.retreat == 0
        && movement.strategic_redeployment == 0
        && (movement.naval_transfer == 0 || transfer == Some(false));
    admit_strategic_layout(
        action_code(
            movement,
            standing,
            if needs_enemy {
                destination_enemy()
            } else {
                Err("last ordinary-route province hostility")
            },
            transfer,
        ),
        strategic_layout,
    )
}

fn recorded_standing(
    frame: &DecodedFrame,
    packet: &replay_data::rows::UnitCombatReferences,
    owner: u32,
    links: &BTreeMap<u32, u32>,
) -> Option<bool> {
    recorded_standing_by(packet, owner, links, |kind, combat| match kind {
        0 | 1 => {
            let sides = rows::<tables::LandCombatSides>(frame).ok()?;
            let side = sides.rows().iter().find(|side| side.combat == combat)?;
            Some(side.attacker_primary.clone())
        }
        _ => {
            let naval = rows::<tables::NavalCombatDetails>(frame).ok()?;
            let row = naval.rows().iter().find(|row| row.combat == combat)?;
            Some(row.sides[0].summary.countries.clone())
        }
    })
}

pub fn recorded_standing_by(
    packet: &replay_data::rows::UnitCombatReferences,
    owner: u32,
    links: &BTreeMap<u32, u32>,
    countries: impl Fn(u8, u64) -> Option<Vec<u32>>,
) -> Option<bool> {
    for reference in &packet.entries {
        if !matches!(reference.kind, 0..=2) {
            return None;
        }
        let countries = countries(reference.kind, reference.combat)?;
        for country in countries {
            if country == owner {
                return Some(true);
            }
            if country != 0 && owner != 0 && links.get(&country)? == links.get(&owner)? {
                return Some(true);
            }
        }
    }
    Some(false)
}

pub fn membership_standing(
    owner: u32,
    references: &BTreeSet<u64>,
    sides: impl Fn(u64) -> Option<Vec<u32>>,
    links: &BTreeMap<u32, u32>,
    combats_complete: bool,
    combat_count: u32,
) -> Option<bool> {
    let mut unknown = false;
    for &reference in references {
        let Some(countries) = sides(reference) else {
            unknown = true;
            continue;
        };
        for country in countries {
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
    (!unknown && combats_complete && references.len() == combat_count as usize).then_some(false)
}

fn standing_attacks(
    frame: &DecodedFrame,
    movements: &[DivisionMovementRow],
    owners: &BTreeMap<u64, u32>,
    links: &BTreeMap<u32, u32>,
    own: Option<&BTreeSet<u64>>,
) -> io::Result<BTreeMap<u64, Option<bool>>> {
    let mut result = BTreeMap::new();
    let wanted: BTreeSet<u64> = movements
        .iter()
        .filter(|movement| {
            own.is_none_or(|own| own.contains(&movement.division))
                && (movement.path_total as i32) <= 0
                && (movement.combat_count as i32) > 0
        })
        .filter_map(|movement| {
            use replay_data::rows::Recorded;
            let attack = match &movement.combat_references {
                Recorded::NotRecorded => return Some(movement.division),
                Recorded::Value(Some(packet))
                    if packet.valid_at(frame.hour, movement.combat_count) =>
                {
                    owners
                        .get(&movement.division)
                        .and_then(|&owner| recorded_standing(frame, packet, owner, links))
                }
                Recorded::Value(_) => None,
            };
            result.insert(movement.division, attack);
            None
        })
        .collect();
    if wanted.is_empty() {
        return Ok(result);
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
    result.extend(
        movements
            .iter()
            .filter(|movement| wanted.contains(&movement.division))
            .map(|movement| {
                let result = (|| {
                    let owner = *owners.get(&movement.division)?;
                    let references = membership.get(&movement.division)?;
                    membership_standing(
                        owner,
                        references,
                        |combat| sides.get(&combat).map(|countries| countries.to_vec()),
                        links,
                        combats.is_complete(),
                        movement.combat_count,
                    )
                })();
                (movement.division, result)
            }),
    );
    Ok(result)
}

pub fn strategic_layout_of(
    whole: Result<(), String>,
    token: impl Fn(u32) -> Option<u32>,
    spells: impl Fn(u32, &[u8]) -> Option<bool>,
) -> bool {
    replay_derive::fixed_layout::validate_definitions(replay_derive::fixed_layout::STRATEGIC_ACTION, whole, token, spells).is_ok()
}

fn admit_strategic_layout(action: Action, admitted: bool) -> Action {
    if !admitted && matches!(action, Ok(6 | 11)) {
        Err(STRATEGIC_LAYOUT)
    } else {
        action
    }
}

fn strategic_unknown(reason: &str) -> bool {
    reason == STRATEGIC_NOT_RECORDED
        || reason == CURRENT_RAIL_MODIFIERS
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

pub fn aggregate_actions(
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

pub fn action_frame(code: u8, own: bool) -> Option<u32> {
    (own && code != 1).then_some(u32::from(code))
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{
        rows::DivisionRow, rows::LandCombatRow, rows::LandCombatSideRow, rows::ProvinceRow,
        rows::TableId, rows::TableRows,
    };
    use replay_frame::{frame::Metrics, frame::TableView};

    fn read_actions(frame: &DecodedFrame, links: &BTreeMap<u32, u32>) -> io::Result<Actions> {
        Actions::read(frame, &DestinationHostility::read(frame, links)?, None)
    }

    fn set(frame: &mut DecodedFrame, table: TableRows) {
        let index = table.table().index();
        *frame.tables[index] = TableView::Complete(table);
    }

    fn directed_fixture(
        frame: &mut DecodedFrame,
        countries: &[(u32, u32)],
        cells: &[(u32, u32, bool)],
    ) {
        use replay_data::{
            rows::CountryCounterInputRow, rows::CountryLinkRow, rows::CountryRow,
            rows::DirectedWars, rows::Recorded, rows::WarCell,
        };
        set(
            frame,
            TableRows::CountryLinks(
                countries
                    .iter()
                    .map(|&(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
                    .collect(),
            ),
        );
        set(
            frame,
            TableRows::Countries(
                countries
                    .iter()
                    .map(|&(id, canonical_slot)| CountryRow {
                        id,
                        canonical_slot,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        let sample_hour = frame.hour;
        set(
            frame,
            TableRows::CountryCounterInputs(
                countries
                    .iter()
                    .map(|&(country, slot)| CountryCounterInputRow {
                        country,
                        directed_wars: Recorded::Value(Some(DirectedWars {
                            sample_hour: replay_data::rows::PacketHour::Stamped(sample_hour),
                            receiver_raw_id: country as i32,
                            receiver_slot: slot,
                            matrix_count: 4,
                            cells: cells
                                .iter()
                                .filter(|&&(raw, _, _)| raw == country)
                                .map(|&(_, target_slot, active)| WarCell {
                                    target_slot,
                                    active,
                                })
                                .collect(),
                            ..Default::default()
                        })),
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
    }

    #[test]
    fn destination_hostility_uses_controller_to_logical_direction_and_refuses_missing_cells() {
        let mut frame = frame();
        set(
            &mut frame,
            TableRows::Provinces(vec![ProvinceRow {
                id: 20,
                controller: 200,
                ..Default::default()
            }]),
        );
        directed_fixture(
            &mut frame,
            &[(100, 1), (200, 2)],
            &[(100, 2, true), (200, 1, false)],
        );
        let links = BTreeMap::from([(100, 1), (101, 1), (200, 2)]);
        let hostility = DestinationHostility::read(&frame, &links).unwrap();
        assert_eq!(hostility.destination_enemy(&[20], 1, Some(100)), Ok(false));
        assert_eq!(hostility.destination_enemy(&[], u32::MAX, None), Ok(false));
        directed_fixture(&mut frame, &[(100, 1), (200, 2)], &[(100, 2, true)]);
        let hostility = DestinationHostility::read(&frame, &links).unwrap();
        assert_eq!(
            hostility.destination_enemy(&[20], 1, Some(100)),
            Err("action destination directed-war operands unavailable")
        );
        assert_eq!(hostility.destination_enemy(&[20], 1, Some(200)), Ok(false));
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
    fn compatibility_absent_rail_operand_refuses_only_the_reached_glyph() {
        let mut movement = DivisionMovementRow {
            path_total: 1,
            target: 8,
            strategic_redeployment: 1,
            strategic_action: replay_data::rows::Recorded::NotRecorded,
            ..Default::default()
        };
        assert_eq!(action_code(&movement, None, Ok(false), None), Ok(3));
        movement.target = 0;
        assert_eq!(
            action_code(&movement, None, Ok(false), None),
            Err(STRATEGIC_NOT_RECORDED)
        );
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
        let code = |row: &DivisionMovementRow, attacking, transfer| {
            action_code(
                row,
                attacking,
                Err("last ordinary-route province hostility"),
                transfer,
            )
        };
        let mut row = DivisionMovementRow::default();
        assert_eq!(code(&row, None, None), Ok(1));
        row.combat_count = 2;
        assert_eq!(code(&row, Some(true), None), Ok(5));
        assert_eq!(code(&row, Some(false), None), Ok(2));
        assert!(code(&row, None, None).is_err());
        row = moving();
        row.target = 10;
        row.retreat = 1;
        row.strategic_redeployment = 1;
        row.naval_transfer = 1;
        assert_eq!(code(&row, None, None), Ok(3));
        row.target = 0;
        assert_eq!(code(&row, None, None), Ok(10));
        row.retreat = 0;
        assert_eq!(code(&row, None, None), Err(CURRENT_RAIL_MODIFIERS));
        row.strategic_redeployment = 0;
        assert_eq!(code(&row, None, Some(true)), Ok(9));
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
        assert_eq!(code(&row, None, None), Ok(1));
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
        let hostility = DestinationHostility::read(&frame, &links).unwrap();
        assert_eq!(hostility.destination_enemy(&[20], 1, Some(100)), Ok(false));
        assert_eq!(hostility.destination_enemy(&[u32::MAX], 1, None), Ok(false));
        assert!(hostility.destination_enemy(&[20], 2, Some(100)).is_err());
        let links = BTreeMap::from([(100, 1), (200, 2)]);
        let hostility = DestinationHostility::read(&frame, &links).unwrap();
        assert_eq!(
            hostility.destination_enemy(&[20], 1, Some(200)),
            Err("action destination directed-war receiver unavailable")
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
        set(&mut frame, TableRows::NavalCombatTransfers(vec![]));
        set(&mut frame, TableRows::NavalTransferUnits(vec![]));
        let links = BTreeMap::from([(100, 1), (200, 2), (300, 3)]);
        directed_fixture(
            &mut frame,
            &[(100, 1), (200, 2), (300, 3)],
            &[(200, 1, true)],
        );
        assert_eq!(read_actions(&frame, &links).unwrap().get(7), Ok(8));
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
        assert_eq!(read_actions(&frame, &links).unwrap().get(7), Ok(9));
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
        let actions = read_actions(&frame, &links).unwrap();
        assert_eq!(actions.get(7), Err("naval-transfer combat membership"));
        set(&mut frame, TableRows::NavalCombatTransfers(vec![]));
        set(&mut frame, TableRows::NavalTransferUnits(vec![]));
        let actions = read_actions(&frame, &links).unwrap();
        assert_eq!(actions.get(7), Ok(7));
        row.path_total = 300;
        set(&mut frame, TableRows::DivisionMovement(vec![row.clone()]));
        let actions = read_actions(&frame, &links).unwrap();
        assert_eq!(
            actions.get(7),
            Err("last ordinary-route province hostility")
        );
        row.target = 21;
        set(&mut frame, TableRows::DivisionMovement(vec![row]));
        *frame.tables[TableId::NavalCombatTransfers.index()] = TableView::Unavailable {
            reason: "not recorded".into(),
        };
        let actions = read_actions(&frame, &BTreeMap::new()).unwrap();
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
            let actions = read_actions(&frame, &BTreeMap::new()).unwrap();
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
            combat_references: replay_data::rows::Recorded::NotRecorded,
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
                attacker_classes: vec![0],
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

    #[test]
    fn current_standing_references_keep_native_order_and_never_retry_a_refused_packet() {
        use replay_data::{rows::Recorded, rows::UnitCombatReference, rows::UnitCombatReferences};
        let mut frame = frame();
        set(
            &mut frame,
            TableRows::LandCombatSides(vec![LandCombatSideRow {
                combat: 40,
                attacker_primary: vec![100],
                ..Default::default()
            }]),
        );
        let owners = BTreeMap::from([(7, 100)]);
        let links = BTreeMap::new();
        let packet = |entries| UnitCombatReferences {
            sample_hour: replay_data::rows::PacketHour::Stamped(10),
            entries,
        };
        let mut movement = DivisionMovementRow {
            division: 7,
            combat_count: 2,
            combat_references: Recorded::Value(Some(packet(vec![
                UnitCombatReference {
                    combat: 40,
                    kind: 1,
                },
                UnitCombatReference {
                    combat: 41,
                    kind: 2,
                },
            ]))),
            ..Default::default()
        };
        let check = |frame: &DecodedFrame, movement: &DivisionMovementRow| {
            standing_attacks(frame, std::slice::from_ref(movement), &owners, &links, None).unwrap()
                [&7]
        };
        assert_eq!(check(&frame, &movement), Some(true));
        if let Recorded::Value(Some(packet)) = &mut movement.combat_references {
            packet.entries.reverse();
        }
        assert_eq!(check(&frame, &movement), None);
        movement.combat_references = Recorded::Value(None);
        set(
            &mut frame,
            TableRows::LandCombats(vec![LandCombatRow {
                id: 40,
                attacker_classes: vec![0],
                attackers: vec![7],
                ..Default::default()
            }]),
        );
        assert_eq!(check(&frame, &movement), None);
        movement.combat_references = Recorded::Value(Some(packet(vec![
            UnitCombatReference {
                combat: 40,
                kind: 0,
            },
            UnitCombatReference {
                combat: 40,
                kind: 0,
            },
        ])));
        let mut stale = frame.clone();
        stale.hour = 11;
        assert_eq!(check(&stale, &movement), None);
        set(
            &mut frame,
            TableRows::LandCombatSides(vec![LandCombatSideRow {
                combat: 40,
                attacker_primary: vec![0],
                ..Default::default()
            }]),
        );
        assert_eq!(check(&frame, &movement), Some(false));
    }

    #[test]
    fn current_standing_naval_uses_only_ordered_attacker_primary_countries() {
        use replay_data::{
            rows::NavalCombatDetailsRow, rows::Recorded, rows::UnitCombatReference,
            rows::UnitCombatReferences,
        };
        let mut frame = frame();
        let movement = DivisionMovementRow {
            division: 7,
            combat_count: 1,
            combat_references: Recorded::Value(Some(UnitCombatReferences {
                sample_hour: replay_data::rows::PacketHour::Stamped(frame.hour),
                entries: vec![UnitCombatReference {
                    combat: 41,
                    kind: 2,
                }],
            })),
            ..Default::default()
        };
        let mut naval = NavalCombatDetailsRow {
            combat: 41,
            duration: i32::MIN,
            custom_sea: Some(Vec::new()),
            ..Default::default()
        };
        naval.sides[0].summary.secondary_countries = vec![100];
        naval.sides[1].summary.countries = vec![100];
        let links = BTreeMap::from([(100, 1), (101, 1), (200, 2)]);
        let check = |frame: &DecodedFrame, owner, links: &BTreeMap<u32, u32>| {
            let owners = BTreeMap::from([(7, owner)]);
            let attacking =
                standing_attacks(frame, std::slice::from_ref(&movement), &owners, links, None)
                    .unwrap()[&7];
            action_code(&movement, attacking, Err("unreached hostility"), None)
        };
        for (countries, owner, expected) in [
            (vec![0, 200], 100, 2),
            (vec![0, 200, 101], 100, 5),
            (vec![0], 100, 2),
            (vec![0], 0, 5),
        ] {
            naval.sides[0].summary.countries = countries;
            set(
                &mut frame,
                TableRows::NavalCombatDetails(vec![naval.clone()]),
            );
            assert_eq!(check(&frame, owner, &links), Ok(expected));
        }
        naval.sides[0].summary.countries = vec![100, 999];
        set(
            &mut frame,
            TableRows::NavalCombatDetails(vec![naval.clone()]),
        );
        assert_eq!(check(&frame, 100, &BTreeMap::new()), Ok(5));
        naval.sides[0].summary.countries.reverse();
        set(&mut frame, TableRows::NavalCombatDetails(vec![naval]));
        assert_eq!(
            check(&frame, 100, &links),
            Err("division combat-reference attacker countries")
        );
    }

    #[test]
    fn current_standing_naval_preserves_mixed_reference_order_and_partial_row_proofs() {
        use replay_data::{
            rows::NavalCombatDetailsRow, rows::Recorded, rows::UnitCombatReference,
            rows::UnitCombatReferences,
        };
        let mut frame = frame();
        let owners = BTreeMap::from([(7, 100)]);
        let links = BTreeMap::from([(100, 1), (200, 2)]);
        let mut movement = DivisionMovementRow {
            division: 7,
            combat_count: 3,
            combat_references: Recorded::Value(Some(UnitCombatReferences {
                sample_hour: replay_data::rows::PacketHour::Stamped(frame.hour),
                entries: vec![
                    UnitCombatReference {
                        combat: 41,
                        kind: 2,
                    },
                    UnitCombatReference {
                        combat: 41,
                        kind: 2,
                    },
                    UnitCombatReference {
                        combat: 40,
                        kind: 0,
                    },
                ],
            })),
            ..Default::default()
        };
        let mut naval = NavalCombatDetailsRow {
            combat: 41,
            custom_sea: Some(Vec::new()),
            ..Default::default()
        };
        naval.sides[0].summary.countries = vec![0, 200];
        *frame.tables[TableId::NavalCombatDetails.index()] = TableView::Truncated {
            rows: TableRows::NavalCombatDetails(vec![naval]),
            missing: 1,
        };
        set(
            &mut frame,
            TableRows::LandCombatSides(vec![LandCombatSideRow {
                combat: 40,
                attacker_primary: vec![100],
                ..Default::default()
            }]),
        );
        let check = |frame: &DecodedFrame, movement: &DivisionMovementRow| {
            standing_attacks(frame, std::slice::from_ref(movement), &owners, &links, None).unwrap()
                [&7]
        };
        assert_eq!(check(&frame, &movement), Some(true));
        movement.combat_count = 2;
        if let Recorded::Value(Some(packet)) = &mut movement.combat_references {
            packet.entries.pop();
        }
        assert_eq!(check(&frame, &movement), Some(false));
        movement.combat_count = 3;
        if let Recorded::Value(Some(packet)) = &mut movement.combat_references {
            packet.entries.push(UnitCombatReference {
                combat: 40,
                kind: 0,
            });
        }
        *frame.tables[TableId::NavalCombatDetails.index()] = TableView::Unavailable {
            reason: "naval list capture failed".into(),
        };
        assert_eq!(check(&frame, &movement), None);
        if let Recorded::Value(Some(packet)) = &mut movement.combat_references {
            packet.entries.reverse();
        }
        assert_eq!(check(&frame, &movement), Some(true));
        let mut stale = frame.clone();
        stale.hour += 1;
        assert_eq!(check(&stale, &movement), None);
    }

    #[test]
    fn current_standing_naval_country_list_follows_live_countries() {
        use replay_data::{
            rows::NavalCombatDetailsRow, rows::UnitCombatReference, rows::UnitCombatReferences,
        };
        let mut frame = frame();
        let mut naval = NavalCombatDetailsRow {
            combat: 41,
            custom_sea: Some(Vec::new()),
            ..Default::default()
        };
        naval.sides[0].summary.countries = (1..=50).collect();
        set(&mut frame, TableRows::NavalCombatDetails(vec![naval]));
        let packet = UnitCombatReferences {
            sample_hour: replay_data::rows::PacketHour::Stamped(frame.hour),
            entries: vec![UnitCombatReference {
                combat: 41,
                kind: 2,
            }],
        };
        let links = (1..=50).map(|country| (country, country)).collect();
        assert_eq!(recorded_standing(&frame, &packet, 50, &links), Some(true));
        assert_eq!(recorded_standing(&frame, &packet, 51, &links), None);
        let mut links = links;
        links.insert(51, 51);
        assert_eq!(recorded_standing(&frame, &packet, 51, &links), Some(false));
    }
}
