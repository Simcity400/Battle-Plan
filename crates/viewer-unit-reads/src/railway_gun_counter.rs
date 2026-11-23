
use super::counter_action::{self, Action};
use replay_data::rows::RailwayGunCounterInputRow;

pub fn join_action(row: &RailwayGunCounterInputRow) -> u8 {
    join_action_of(&GunOperands::of(row))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GunOperands {
    pub path: u32,
    pub combat_count: u32,
    pub reference_entry_count: i32,
    pub attacking: u8,
    pub target: u32,
    pub retreat: u8,
    pub strategic_redeployment: u8,
    pub naval_transfer: u8,
    pub transfer_action_status: u8,
    pub exiled: u8,
    pub railway_building_level: Option<i16>,
}

impl GunOperands {
    pub fn of(row: &RailwayGunCounterInputRow) -> Self {
        Self {
            path: row.ordinary_path.len() as u32,
            combat_count: row.combat_count,
            reference_entry_count: row.reference_entry_count,
            attacking: row.attacking,
            target: row.target,
            retreat: row.retreat,
            strategic_redeployment: row.strategic_redeployment,
            naval_transfer: row.naval_transfer,
            transfer_action_status: row.transfer_action_status,
            exiled: row.exiled,
            railway_building_level: row.railway_building_level,
        }
    }
}

pub fn join_action_of(row: &GunOperands) -> u8 {
    if !viewer_foundation::unit_rules::idle(row.path, row.combat_count) {
        2
    } else if row.reference_entry_count > 0 {
        12
    } else {
        1
    }
}

pub fn action_code(
    row: &RailwayGunCounterInputRow,
    destination_enemy: Result<bool, &'static str>,
) -> Action {
    action_code_of(&GunOperands::of(row), destination_enemy)
}

pub fn action_code_of(row: &GunOperands, destination_enemy: Result<bool, &'static str>) -> Action {
    if row.path == 0 {
        return if !viewer_foundation::unit_rules::idle(0, row.combat_count) {
            Ok(if row.attacking != 0 { 5 } else { 2 })
        } else {
            Ok(if row.reference_entry_count > 0 { 12 } else { 1 })
        };
    }
    if row.target != 0 {
        return Ok(3);
    }
    if row.retreat != 0 {
        return Ok(10);
    }
    if row.strategic_redeployment != 0 {
        return Ok(6);
    }
    if row.naval_transfer != 0 && row.transfer_action_status != 0 {
        return Ok(9);
    }
    counter_action::route_tail(destination_enemy, row.naval_transfer != 0, row.combat_count)
}

pub fn aggregate_actions(actions: impl IntoIterator<Item = Action>) -> Action {
    let mut actions = actions.into_iter();
    counter_action::aggregate_actions(
        actions.next().ok_or("railway gun action members")?,
        actions.map(|action| (true, action)),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RailStatus {
    pub neighbor_pairs: i32,
    pub cooldown_ticks: i32,
}

impl RailStatus {
    pub fn read(row: &replay_data::rows::RailCooldownRow, hour: i32) -> Result<Self, String> {
        Self::of(&row.selection_hour, &row.selection, row.cooldown_ticks, hour)
    }

    pub fn of(
        selection_hour: &replay_data::rows::Recorded<Option<i32>>,
        selection: &replay_data::rows::Recorded<Option<replay_data::rows::RailwaySelectionInputs>>,
        cooldown_ticks: i32,
        hour: i32,
    ) -> Result<Self, String> {
        if selection_hour
            .require("RailCooldowns.selection_hour")?
            .ok_or("railway raw pair-count sample hour capture refused")?
            != hour
        {
            return Err("railway raw pair-count sample is stale".into());
        }
        let inputs = selection
            .require("RailCooldowns.selection")?
            .as_ref()
            .ok_or("railway raw pair-count input capture refused")?;
        Ok(Self {
            neighbor_pairs: inputs
                .pair_count
                .ok_or("railway raw pair count capture refused")?,
            cooldown_ticks,
        })
    }
}

pub fn location_warning(
    row: &RailwayGunCounterInputRow,
    land: bool,
    rail: Option<RailStatus>,
) -> Option<bool> {
    location_warning_of(&GunOperands::of(row), land, rail)
}

pub fn location_warning_of(row: &GunOperands, land: bool, rail: Option<RailStatus>) -> Option<bool> {
    if row.path != 0 || row.naval_transfer != 0 || row.exiled != 0 || !land {
        return Some(false);
    }
    match row.railway_building_level {
        None | Some(..=0) => Some(true),
        Some(_) => rail.map(|rail| rail.neighbor_pairs == 0 || rail.cooldown_ticks != 0),
    }
}

pub fn strength_ratio(current: i64, maximum: i64) -> i64 {
    if maximum < 1 || (current as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd
    {
        100_000
    } else {
        (current.wrapping_mul(100_000) / maximum).clamp(0, 100_000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn railway_status_admits_only_current_raw_count_without_other_rail_domains() {
        use replay_data::{rows::RailCooldownRow, rows::RailwaySelectionInputs, rows::Recorded};
        let mut row = RailCooldownRow {
            province: 3,
            cooldown_ticks: -1,
            selection_hour: Some(12).into(),
            selection: Some(RailwaySelectionInputs {
                pair_count: Some(-2),
                ..Default::default()
            })
            .into(),
            ..Default::default()
        };
        assert_eq!(
            RailStatus::read(&row, 12),
            Ok(RailStatus {
                neighbor_pairs: -2,
                cooldown_ticks: -1,
            })
        );
        assert!(RailStatus::read(&row, 13).unwrap_err().contains("stale"));
        for hour in [Recorded::NotRecorded, None.into()] {
            row.selection_hour = hour;
            assert!(RailStatus::read(&row, 12).is_err());
        }
        row.selection_hour = Some(12).into();
        for selection in [
            Recorded::NotRecorded,
            None.into(),
            Some(RailwaySelectionInputs::default()).into(),
        ] {
            row.selection = selection;
            assert!(RailStatus::read(&row, 12).is_err());
        }
    }

    #[test]
    fn join_action_keeps_idle_reference_and_other_apart() {
        let mut row = RailwayGunCounterInputRow::default();
        assert_eq!(join_action(&row), 1);
        row.reference_entry_count = 1;
        assert_eq!(join_action(&row), 12);
        row.ordinary_path = vec![7];
        assert_eq!(join_action(&row), 2);
        row.ordinary_path.clear();
        row.combat_role = 2;
        row.combat_count = 1;
        assert_eq!(join_action(&row), 2);
    }

    #[test]
    fn action_code_follows_native_precedence_and_refuses_unknown_branches() {
        let mut row = RailwayGunCounterInputRow::default();
        assert_eq!(action_code(&row, Err("hostility")), Ok(1));
        row.reference_entry_count = 1;
        assert_eq!(action_code(&row, Err("hostility")), Ok(12));
        row.combat_role = 2;
        row.combat_count = 2;
        row.attacking = 1;
        assert_eq!(action_code(&row, Err("hostility")), Ok(5));
        row.attacking = 0;
        assert_eq!(action_code(&row, Err("hostility")), Ok(2));

        row.ordinary_path = vec![2];
        assert_eq!(action_code(&row, Err("hostility")), Err("hostility"));
        assert_eq!(action_code(&row, Ok(false)), Ok(2));
        assert_eq!(action_code(&row, Ok(true)), Ok(5));
        row.combat_role = 0;
        row.combat_count = 0;
        assert_eq!(action_code(&row, Ok(false)), Ok(4));
        row.naval_transfer = 1;
        assert_eq!(action_code(&row, Ok(false)), Ok(7));
        assert_eq!(action_code(&row, Ok(true)), Ok(8));
        row.transfer_action_status = 1;
        assert_eq!(action_code(&row, Err("hostility")), Ok(9));
        row.strategic_redeployment = 1;
        assert_eq!(action_code(&row, Err("hostility")), Ok(6));
        row.retreat = 1;
        assert_eq!(action_code(&row, Err("hostility")), Ok(10));
        row.target = 2;
        assert_eq!(action_code(&row, Err("hostility")), Ok(3));
    }

    #[test]
    fn aggregation_is_absorbing_on_mixture_and_refuses_unknowns() {
        assert_eq!(aggregate_actions([Ok(6), Ok(6)]), Ok(6));
        assert_eq!(aggregate_actions([Ok(6), Ok(9), Err("hostility")]), Ok(0));
        assert_eq!(aggregate_actions([Err("hostility"), Ok(6), Ok(9)]), Ok(0));
        assert_eq!(
            aggregate_actions([Ok(6), Err("hostility")]),
            Err("hostility")
        );
        assert_eq!(
            aggregate_actions([Err("hostility"), Ok(9)]),
            Err("hostility")
        );
        assert_eq!(aggregate_actions([]), Err("railway gun action members"));
        assert_eq!(counter_action::action_frame(0, true), Some(0));
        assert_eq!(counter_action::action_frame(12, true), Some(12));
        assert_eq!(counter_action::action_frame(1, true), None);
        assert_eq!(counter_action::action_frame(6, false), None);
    }

    #[test]
    fn location_warning_follows_the_native_rail_availability_test() {
        let mut row = RailwayGunCounterInputRow {
            railway_building_level: Some(2),
            ..Default::default()
        };
        let rail = |neighbor_pairs, cooldown_ticks| {
            Some(RailStatus {
                neighbor_pairs,
                cooldown_ticks,
            })
        };
        assert_eq!(location_warning(&row, true, rail(2, 0)), Some(false));
        assert_eq!(location_warning(&row, true, rail(-1, 0)), Some(false));
        assert_eq!(location_warning(&row, true, None), None);
        assert_eq!(location_warning(&row, true, rail(0, 0)), Some(true));
        assert_eq!(location_warning(&row, true, rail(2, 1)), Some(true));
        row.railway_building_level = Some(0);
        assert_eq!(location_warning(&row, true, None), Some(true));
        row.railway_building_level = None;
        assert_eq!(location_warning(&row, true, None), Some(true));
        assert_eq!(location_warning(&row, false, None), Some(false));
        row.exiled = 1;
        assert_eq!(location_warning(&row, true, None), Some(false));
        row.exiled = 0;
        row.naval_transfer = 1;
        assert_eq!(location_warning(&row, true, None), Some(false));
        row.naval_transfer = 0;
        row.ordinary_path = vec![3];
        assert_eq!(location_warning(&row, true, None), Some(false));
    }

    #[test]
    fn strength_uses_the_zero_maximum_rule_and_clamps() {
        assert_eq!(strength_ratio(5, 10), 50_000);
        assert_eq!(strength_ratio(0, 0), 100_000);
        assert_eq!(strength_ratio(7, -3), 100_000);
        assert_eq!(strength_ratio(-1, 10), 0);
        assert_eq!(strength_ratio(30, 10), 100_000);
        for current in [i64::MIN, i64::MIN + 1, i64::MAX] {
            assert_eq!(strength_ratio(current, 100_000), 100_000);
        }
        assert_eq!(strength_ratio(100_000_000_000_000, 100_000), 0);
    }
}
