
use crate::Hour;
use crate::rows::{AirCombatRecord, AirRegionCombatRow, Recorded};

pub const DAY: Hour = 24;

pub const CAUSES: usize = 11;

pub const UNPRUNED_CONTAINER: &str = "the daily pass does not prune a country without an owned state, so its retained air-combat records are unknown";

pub fn last_pass(hour: Hour) -> Option<Hour> {
    hour.checked_sub(hour.rem_euclid(DAY))
}

pub fn cutoff(pass: Hour, timeout: i32) -> i32 {
    pass.wrapping_sub(timeout)
}

pub fn pruning_cutoff(pass: Hour, timeout: i32) -> i32 {
    cutoff(pass, timeout).min(pass)
}

pub fn pruned(records: &[AirCombatRecord], cutoff: i32) -> &[AirCombatRecord] {
    let start = records
        .iter()
        .position(|record| record.date >= cutoff)
        .unwrap_or(records.len());
    &records[start..]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Retained<'a> {
    pub own: &'a [AirCombatRecord],
    pub foreign: &'a [AirCombatRecord],
}

pub fn retained(row: &AirRegionCombatRow, hour: Hour, timeout: i32) -> Retained<'_> {
    retained_through(row, last_pass(hour), timeout)
}

pub fn visiting_pass(
    hour: Hour,
    first: Hour,
    mut visited: impl FnMut(Hour) -> Result<bool, String>,
) -> Result<Option<Hour>, String> {
    let mut pass = last_pass(hour);
    while let Some(at) = pass.filter(|&at| at > first) {
        if visited(at)? {
            return Ok(Some(at));
        }
        pass = at.checked_sub(DAY);
    }
    Ok(None)
}

pub fn pass_visited(pass: Hour, before: bool, after: bool) -> Result<bool, String> {
    if before != after {
        return Err(format!(
            "the daily air roster changed in the tick of the daily pass at hour {pass}"
        ));
    }
    Ok(after)
}

pub fn retained_through(
    row: &AirRegionCombatRow,
    pass: Option<Hour>,
    timeout: i32,
) -> Retained<'_> {
    retained_of(Retained { own: &row.own, foreign: &row.foreign }, pass, timeout)
}

pub fn retained_of(read: Retained<'_>, pass: Option<Hour>, timeout: i32) -> Retained<'_> {
    match pass {
        Some(pass) => {
            let cutoff = pruning_cutoff(pass, timeout);
            Retained {
                own: pruned(read.own, cutoff),
                foreign: pruned(read.foreign, cutoff),
            }
        }
        None => read,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Received {
    pub amounts: [i64; CAUSES],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Filter {
    pub missions: u32,
    pub window: i32,
    pub classification: u8,
}

pub const CAUSE_OUTSIDE: &str =
    "an air-combat damage cause lies outside the strategic air view's eleven entries";

fn add_vector(
    received: &mut Received,
    records: &[AirCombatRecord],
    now: Hour,
    filter: Filter,
    first_damage: bool,
) -> Result<(), &'static str> {
    let start = now.wrapping_sub(filter.window);
    let mut planes = 0_i32;
    for record in records.iter().rev() {
        if record.date < start {
            break;
        }
        if record.mission_mask & filter.missions == 0
            || record.classification != filter.classification
        {
            continue;
        }
        let damage = if first_damage {
            &record.first_damage
        } else {
            &record.second_damage
        };
        for entry in damage {
            let cause = usize::try_from(entry.cause)
                .ok()
                .filter(|&cause| cause < CAUSES)
                .ok_or(CAUSE_OUTSIDE)?;
            received.amounts[cause] = received.amounts[cause].wrapping_add(entry.amount);
        }
        planes = planes.wrapping_add(record.planes);
    }
    if planes > 0 {
        let amount = received.amounts[3];
        received.amounts[3] =
            if (amount as u64).wrapping_add(0x7fff_ffff_ffff_fffe) > 0xffff_ffff_ffff_fffc {
                0xffff_ffff
            } else {
                amount.wrapping_mul(100_000) / i64::from(planes).wrapping_mul(100_000)
            };
    }
    Ok(())
}

pub fn received(
    retained: Retained<'_>,
    now: Hour,
    enemy: bool,
    filter: Filter,
) -> Result<Received, &'static str> {
    let (first, second) = if enemy {
        (retained.foreign, retained.own)
    } else {
        (retained.own, retained.foreign)
    };
    let mut received = Received::default();
    add_vector(&mut received, first, now, filter, false)?;
    add_vector(&mut received, second, now, filter, true)?;
    Ok(received)
}

pub const DEFAULT_MISSIONS: u32 = 0x1f7fa;

pub const LAST_DAY: i32 = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Item {
    pub cause: u32,
    pub amount: i64,
    pub flag: u32,
}

impl Item {
    pub fn ours(self) -> bool {
        self.flag.wrapping_sub(1) & 0xffff_fffd == 0
    }
}

fn items(
    received: &Received,
    causes: &[u32],
    filter_missions: u32,
    flag: u32,
    out: &mut Vec<Item>,
) {
    for &cause in causes {
        let mut amount = received.amounts[cause as usize];
        if cause == 4 {
            amount = amount.wrapping_add(received.amounts[9]);
        }
        if amount == 0 || cause == 9 {
            continue;
        }
        let allowed = match cause {
            0 | 7 => filter_missions >> 3 & 1 != 0,
            1 => filter_missions & 0x3190 != 0,
            2 => filter_missions >> 1 & 1 != 0,
            6 => filter_missions >> 9 & 1 != 0,
            _ => true,
        };
        if allowed {
            out.push(Item {
                cause,
                amount,
                flag,
            });
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Grids {
    pub air: Vec<Item>,
    pub ground: Vec<Item>,
}

pub fn grids(retained: Retained<'_>, now: Hour, enemy: bool) -> Result<Grids, &'static str> {
    filtered_grids(retained, now, enemy, false, DEFAULT_MISSIONS)
}

pub fn filtered_grids(
    retained: Retained<'_>,
    now: Hour,
    enemy: bool,
    enemy_missions: bool,
    missions: u32,
) -> Result<Grids, &'static str> {
    let filter = |missions, classification| Filter {
        missions,
        window: LAST_DAY,
        classification,
    };
    let side = u32::from(enemy);
    let mut grids = Grids::default();
    let air = received(retained, now, enemy, filter(5, 0))?;
    items(&air, &[4, 5, 9], missions, side + 2, &mut grids.air);
    if enemy != enemy_missions {
        let first = received(retained, now, enemy, filter(5, 1))?;
        items(&first, &[4, 5, 9], missions, side, &mut grids.ground);
        let second = received(retained, now, enemy, filter(missions, 1))?;
        items(
            &second,
            &[2, 0, 1, 6, 7, 9],
            missions,
            side,
            &mut grids.ground,
        );
    } else {
        let ground = received(retained, now, enemy, filter(missions, 1))?;
        items(
            &ground,
            &[4, 5, 3, 9, 10],
            missions,
            side,
            &mut grids.ground,
        );
    }
    Ok(grids)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EquipmentOrder {
    pub archetype: usize,
    pub version: u8,
    pub equipment: usize,
    pub creator: u32,
    pub origin: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EquipmentPool {
    pub variants: Vec<(u64, i64)>,
    pub total_raw: i64,
}

pub type EquipmentPools = [EquipmentPool; 2];

pub fn equipment(
    retained: Retained<'_>,
    now: Hour,
    enemy: bool,
    filter: Filter,
    mut order: impl FnMut(u64) -> Result<EquipmentOrder, String>,
) -> Result<EquipmentPools, String> {
    use std::collections::{BTreeMap, BTreeSet};
    let records = if enemy {
        retained.foreign
    } else {
        retained.own
    };
    let start = now.wrapping_sub(filter.window);
    let first = records
        .iter()
        .rposition(|record| record.date < start)
        .map_or(0, |at| at + 1);
    let records = &records[first..];
    let mut best = BTreeMap::<u32, usize>::new();
    let prefixes: Vec<_> = records
        .iter()
        .enumerate()
        .map(|(at, record)| {
            let selected = best.entry(record.wing_id).or_insert(at);
            if record.planes >= records[*selected].planes {
                *selected = at;
            }
            *selected
        })
        .collect();
    let mut seen = BTreeSet::new();
    let mut groups = [
        Vec::<BTreeMap<(u8, usize, u32, u32), (u64, i64)>>::new(),
        Vec::new(),
    ];
    let mut indices = [BTreeMap::<usize, usize>::new(), BTreeMap::new()];
    let mut totals = [BTreeMap::<usize, i64>::new(), BTreeMap::new()];
    for (index, record) in records.iter().enumerate().rev() {
        if record.mission_mask & filter.missions == 0
            || record.classification != filter.classification
            || !seen.insert(record.wing_id)
        {
            continue;
        }
        let selected = &records[prefixes[index]];
        let equipment = match &selected.equipment {
            Recorded::Value(Some(equipment)) => equipment,
            Recorded::Value(None) => return Err("retained air-combat equipment pool was unreadable".into()),
            Recorded::NotRecorded => return Err("air-combat equipment was not recorded".into()),
        };
        let pool = usize::from(selected.selection != 0);
        for entry in equipment {
            let key = order(entry.variant)?;
            let total = totals[pool].entry(key.archetype).or_default();
            *total = total
                .wrapping_add(entry.amount_raw)
                .clamp(-0x53e2_d620_fb40, 0x53e2_d620_fb40);
            let group = *indices[pool].entry(key.archetype).or_insert_with(|| {
                groups[pool].push(BTreeMap::new());
                groups[pool].len() - 1
            });
            let value = groups[pool][group]
                .entry((key.version, key.equipment, key.creator, key.origin))
                .or_insert((entry.variant, 0));
            value.1 = value
                .1
                .wrapping_add(entry.amount_raw)
                .clamp(-0x53e2_d620_fb40, 0x53e2_d620_fb40);
        }
    }
    let mut groups = groups.into_iter();
    Ok(totals.map(|totals| EquipmentPool {
        variants: groups
            .next()
            .unwrap()
            .into_iter()
            .flat_map(BTreeMap::into_values)
            .collect(),
        total_raw: totals.into_values().fold(0_i64, |total, value| {
            total
                .wrapping_add(value)
                .clamp(-0x53e2_d620_fb40, 0x53e2_d620_fb40)
        }),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rows::AirCombatDamage;

    fn record(date: i32, mission_mask: u32, classification: u8, planes: i32) -> AirCombatRecord {
        AirCombatRecord {
            wing_id: 7,
            date,
            planes,
            mission_mask,
            classification,
            ..Default::default()
        }
    }

    fn damage(cause: u32, amount: i64) -> AirCombatDamage {
        AirCombatDamage { cause, amount }
    }

    const HOUR: Hour = 43_800_000 + 24 * 30 + 5;

    #[test]
    fn the_latest_daily_pass_prunes_only_the_leading_run() {
        let pass = last_pass(HOUR).unwrap();
        assert_eq!(pass, 43_800_000 + 24 * 30);
        assert_eq!(last_pass(pass), Some(pass));
        assert_eq!(last_pass(pass - 1), Some(pass - 24));
        let kept = cutoff(pass, 24);
        let row = AirRegionCombatRow {
            own: std::sync::Arc::new(vec![
                record(kept - 2, 1, 0, 1),
                record(kept - 1, 1, 0, 1),
                record(kept, 1, 0, 1),
                record(kept - 9, 1, 0, 1),
                record(HOUR, 1, 0, 1),
            ]),
            foreign: std::sync::Arc::new(vec![record(kept - 1, 1, 0, 1)]),
            ..Default::default()
        };
        let state = retained(&row, HOUR, 24);
        assert_eq!(
            state.own.iter().map(|r| r.date).collect::<Vec<_>>(),
            [kept, kept - 9, HOUR]
        );
        assert!(state.foreign.is_empty());
        assert_eq!(retained(&row, HOUR, 48).own.len(), 5);
        assert_eq!(cutoff(i32::MIN, 1), i32::MAX);
    }

    #[test]
    fn a_negative_timeout_keeps_records_created_since_the_pass() {
        let pass = last_pass(HOUR).unwrap();
        assert_eq!(cutoff(pass, -12), pass + 12);
        let fresh = AirRegionCombatRow {
            own: std::sync::Arc::new(vec![record(pass + 1, 1, 0, 1)]),
            ..Default::default()
        };
        assert_eq!(retained(&fresh, pass + 2, -12).own.len(), 1);
        let stale = AirRegionCombatRow {
            own: std::sync::Arc::new(vec![record(pass - 3, 1, 0, 1), record(pass - 1, 1, 0, 1)]),
            ..Default::default()
        };
        assert!(retained(&stale, pass + 2, -12).own.is_empty());
        assert!(retained(&fresh, pass + DAY, -12).own.is_empty());
        assert_eq!(pruning_cutoff(pass, i32::MIN), pass.wrapping_sub(i32::MIN));
    }

    #[test]
    fn the_latest_visiting_pass_alone_prunes_and_no_visit_keeps_the_row() {
        let pass = last_pass(HOUR).unwrap();
        let first = pass - 24 * 5;
        let mut asked = Vec::new();
        let found = visiting_pass(HOUR, first, |at| {
            asked.push(at);
            Ok(at == pass - 48)
        });
        assert_eq!(found, Ok(Some(pass - 48)));
        assert_eq!(asked, [pass, pass - 24, pass - 48]);
        let mut asked = Vec::new();
        assert_eq!(
            visiting_pass(HOUR, first, |at| {
                asked.push(at);
                Ok(false)
            }),
            Ok(None)
        );
        assert_eq!(asked.last(), Some(&(first + 24)));
        assert_eq!(
            visiting_pass(HOUR, first, |at| if at == pass {
                Ok(false)
            } else {
                Err("gap".to_owned())
            }),
            Err("gap".to_owned())
        );
        assert_eq!(pass_visited(pass, true, true), Ok(true));
        assert_eq!(pass_visited(pass, false, false), Ok(false));
        assert!(pass_visited(pass, false, true).is_err());

        let older = cutoff(pass - 48, 24);
        let row = AirRegionCombatRow {
            own: std::sync::Arc::new(vec![record(older - 1, 1, 0, 1), record(older, 1, 0, 1)]),
            foreign: std::sync::Arc::new(vec![record(older - 5, 1, 0, 1)]),
            ..Default::default()
        };
        let state = retained_through(&row, Some(pass - 48), 24);
        assert_eq!(
            state.own.iter().map(|r| r.date).collect::<Vec<_>>(),
            [older]
        );
        assert!(state.foreign.is_empty());
        assert!(retained(&row, HOUR, 24).own.is_empty());
        let whole = retained_through(&row, None, 24);
        assert_eq!((whole.own.len(), whole.foreign.len()), (2, 1));
    }

    #[test]
    fn passes_end_at_the_earliest_signed_hour() {
        let midnight = Hour::MIN + 8;
        assert_eq!(midnight.rem_euclid(DAY), 0);
        assert_eq!(last_pass(Hour::MIN), None);
        assert_eq!(last_pass(midnight - 1), None);
        assert_eq!(last_pass(midnight), Some(midnight));
        assert_eq!(last_pass(Hour::MAX), Some(Hour::MAX - 7));
        let row = AirRegionCombatRow {
            own: std::sync::Arc::new(vec![record(Hour::MIN, 1, 0, 1)]),
            ..Default::default()
        };
        assert_eq!(retained(&row, Hour::MIN, 24).own.len(), 1);
        let mut asked = Vec::new();
        let found = visiting_pass(midnight + 5, Hour::MIN, |at| {
            asked.push(at);
            Ok(false)
        });
        assert_eq!(found, Ok(None));
        assert_eq!(asked, [midnight]);
        assert_eq!(
            visiting_pass(Hour::MIN + 3, Hour::MIN, |_| Ok(true)),
            Ok(None)
        );
    }

    #[test]
    fn received_sums_the_window_by_cause_and_replaces_cause_three_by_its_plane_ratio() {
        let mut ours = record(HOUR - 3, 0b101, 0, 4);
        ours.second_damage = vec![damage(3, 200_000), damage(4, 100_000)];
        let mut old = record(HOUR - 30, 0b101, 0, 9);
        old.second_damage = vec![damage(4, 900_000)];
        let mut other_class = record(HOUR, 0b101, 1, 9);
        other_class.second_damage = vec![damage(4, 900_000)];
        let own = vec![old, ours, other_class];
        let mut enemy = record(HOUR - 1, 0b100, 0, 0);
        enemy.first_damage = vec![damage(2, 50_000)];
        enemy.second_damage = vec![damage(4, 7)];
        let foreign = vec![enemy];
        let state = Retained {
            own: &own,
            foreign: &foreign,
        };
        let filter = Filter {
            missions: 5,
            window: 24,
            classification: 0,
        };
        let total = received(state, HOUR, false, filter).unwrap();
        assert_eq!(total.amounts[4], 100_000);
        assert_eq!(total.amounts[3], 50_000);
        assert_eq!(total.amounts[2], 50_000);
        let enemy_total = received(state, HOUR, true, filter).unwrap();
        assert_eq!(enemy_total.amounts[4], 7);
        assert_eq!(enemy_total.amounts[2], 0);
        let mut bad = record(HOUR, 1, 0, 1);
        bad.second_damage = vec![damage(11, 1)];
        let bad = vec![bad];
        assert_eq!(
            received(
                Retained {
                    own: &bad,
                    foreign: &[]
                },
                HOUR,
                false,
                filter
            ),
            Err(CAUSE_OUTSIDE)
        );
    }

    #[test]
    fn the_cause_three_ratio_keeps_the_native_sentinel_and_divides_per_scan() {
        let mut first = record(HOUR, 1, 0, 2);
        first.second_damage = vec![damage(3, i64::MAX)];
        let own = vec![first];
        let filter = Filter {
            missions: 1,
            window: 24,
            classification: 0,
        };
        let state = Retained {
            own: &own,
            foreign: &[],
        };
        assert_eq!(
            received(state, HOUR, false, filter).unwrap().amounts[3],
            0xffff_ffff
        );
        let mut first = record(HOUR, 1, 0, 2);
        first.second_damage = vec![damage(3, 400_000)];
        let own = vec![first];
        let foreign = vec![record(HOUR, 1, 0, 4)];
        let state = Retained {
            own: &own,
            foreign: &foreign,
        };
        assert_eq!(
            received(state, HOUR, false, filter).unwrap().amounts[3],
            50_000
        );
    }

    #[test]
    fn grids_follow_the_default_view_lists_and_filters() {
        let mut fighter = record(HOUR, 0b101, 0, 3);
        fighter.second_damage = vec![damage(4, 200_000), damage(9, 100_000), damage(5, 0)];
        let mut bomber = record(HOUR, 0b1000, 1, 5);
        bomber.second_damage = vec![damage(10, 300_000), damage(3, 100_000)];
        let mut enemy = record(HOUR, 0b1000, 1, 2);
        enemy.first_damage = vec![damage(0, 123_456), damage(1, 7)];
        let own = vec![fighter, bomber];
        let foreign = vec![enemy];
        let state = Retained {
            own: &own,
            foreign: &foreign,
        };
        let ours = grids(state, HOUR, false).unwrap();
        assert_eq!(
            ours.air,
            [Item {
                cause: 4,
                amount: 300_000,
                flag: 2
            }]
        );
        assert_eq!(
            ours.ground,
            [
                Item {
                    cause: 3,
                    amount: 10_000,
                    flag: 0
                },
                Item {
                    cause: 10,
                    amount: 300_000,
                    flag: 0
                },
            ]
        );
        assert!(
            !ours.ground[0].ours()
                && Item {
                    cause: 0,
                    amount: 1,
                    flag: 3
                }
                .ours()
        );
        let theirs = grids(
            Retained {
                own: &foreign,
                foreign: &own,
            },
            HOUR,
            true,
        )
        .unwrap();
        assert_eq!(
            theirs.ground,
            [
                Item {
                    cause: 0,
                    amount: 123_456,
                    flag: 1
                },
                Item {
                    cause: 1,
                    amount: 7,
                    flag: 1
                },
            ]
        );
        assert!(theirs.ground[0].ours());
    }
}
