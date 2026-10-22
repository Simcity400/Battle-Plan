//! Playback of the retained regional air-combat records
//! ([`crate::rows::AirRegionCombatRow`]): the daily pruning the collector does
//! not observe, and the totals the game's strategic air view draws from them.
//!
//! A row holds a holder's records as the collector last read them. The daily air pass
//! prunes every holder of the countries it visits. The hourly update advances the clock
//! first and enters the daily update when the day changes, so the pass of the tick before
//! a sample with `hour % 24 == 0` prunes with that hour (raw hour 0, 43800000, is a
//! multiple of 24). Its cutoff is the clock minus `NGame.AIR_LOG_TIMEOUT_HOURS`, read as
//! a signed integer without a clamp; each vector loses the leading run of records dated
//! before it and keeps its order.
//!
//! Applying the latest pass's cutoff to a row is exact however old the row is: the pass
//! removes the same leading run again, records the updater appended since are dated at or
//! after the pass's hour, which no cutoff of that pass reaches, and a pass that already
//! ran before the read left a first record its own cutoff keeps. The pass visits only the
//! countries in the game's daily simulation roster (those owning a state); a country
//! outside it keeps its records unpruned, which [`retained`] cannot know, so a caller that
//! has not established the country was in the roster refuses such a container by name
//! ([`UNPRUNED_CONTAINER`]).
//!
//! The hourly AirDailyRoster records that roster. [`visiting_pass`] finds the latest pass
//! that visited a container country and [`retained_through`] applies it alone: passes
//! before it are subsumed by its later cutoff, and passes after it did not touch the
//! country, so the result is exact for any country, with or without an owned state.

use crate::Hour;
use crate::rows::{AirCombatRecord, AirRegionCombatRow};

/// Hours between daily passes.
pub const DAY: Hour = 24;

/// The record updater dispatches causes 0 through 10; the view's aggregate holds one
/// entry per cause.
pub const CAUSES: usize = 11;

/// Why a container country's derived state is refused.
pub const UNPRUNED_CONTAINER: &str = "the daily pass does not prune a country without an owned state, so its retained air-combat records are unknown";

/// Latest daily pass; None when its midnight precedes the earliest representable hour.
pub fn last_pass(hour: Hour) -> Option<Hour> {
    hour.checked_sub(hour.rem_euclid(DAY))
}

/// Pruning cutoff for a pass at `pass`: subtract the timeout from the clock with signed
/// 32-bit wrap.
pub fn cutoff(pass: Hour, timeout: i32) -> i32 {
    pass.wrapping_sub(timeout)
}

/// One pruned vector: the leading records dated before `cutoff` removed, the rest in
/// order.
pub fn pruned(records: &[AirCombatRecord], cutoff: i32) -> &[AirCombatRecord] {
    let start = records
        .iter()
        .position(|record| record.date >= cutoff)
        .unwrap_or(records.len());
    &records[start..]
}

/// What the game holds at `hour` in a holder of a country the daily pass
/// visits, from its row read at or before `hour`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Retained<'a> {
    /// The own-records vector: the owner's own wings.
    pub own: &'a [AirCombatRecord],
    /// The foreign-records vector: other countries' wings.
    pub foreign: &'a [AirCombatRecord],
}

pub fn retained(row: &AirRegionCombatRow, hour: Hour, timeout: i32) -> Retained<'_> {
    retained_through(row, last_pass(hour), timeout)
}

/// The latest daily pass at or before `hour` that visited a container country, searching
/// back over the branch's passes after its first sample `first` (a row read at or after
/// that sample already holds any earlier pass). `visited` answers one pass's roster from
/// the samples on either side of its tick; its refusal is the result's. None when no pass
/// of the branch visited the country.
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

/// Whether the pass in the tick before sample `pass` visited a country, from its roster
/// eligibility at the samples before and after that tick: the pass reads the roster inside
/// the tick, so samples that disagree leave it unknown.
pub fn pass_visited(pass: Hour, before: bool, after: bool) -> Result<bool, String> {
    if before != after {
        return Err(format!(
            "the daily air roster changed in the tick of the daily pass at hour {pass}"
        ));
    }
    Ok(after)
}

/// What the game holds in a holder whose container country the pass at `pass` visited
/// last ([`visiting_pass`]); with no visiting pass, the row as read.
pub fn retained_through(
    row: &AirRegionCombatRow,
    pass: Option<Hour>,
    timeout: i32,
) -> Retained<'_> {
    match pass {
        Some(pass) => {
            let cutoff = cutoff(pass, timeout);
            Retained {
                own: pruned(&row.own, cutoff),
                foreign: pruned(&row.foreign, cutoff),
            }
        }
        None => Retained {
            own: &row.own,
            foreign: &row.foreign,
        },
    }
}

/// The view's aggregate: one signed fixed5 amount per cause, the entry for cause 3
/// replaced by a ratio when records with planes match.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Received {
    pub amounts: [i64; CAUSES],
}

/// What one aggregation reads: the mission bits a record must share, the window in hours and the
/// classification byte it must equal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Filter {
    pub missions: u32,
    pub window: i32,
    pub classification: u8,
}

/// The native refusal of a damage cause outside the aggregate.
pub const CAUSE_OUTSIDE: &str =
    "an air-combat damage cause lies outside the strategic air view's eleven entries";

/// Scan `records` from the newest while their date is at least `now - window` (an `i32`
/// subtraction); each record whose mission mask shares a bit with `missions` and whose
/// classification equals the filter's adds every entry of the selected damage vector to
/// its cause (`i64` wrapping) and its plane high-water mark to an `i32` sum. A positive
/// sum then replaces cause 3 with its fixed5 ratio to the plane count, or the native
/// `0xFFFFFFFF` for the three sentinel words.
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

/// For the viewing side `enemy` (the view's mission side, 0 for our missions), the first scan reads
/// the side's own vector (the own records for ours, the foreign ones for the enemy's) with its
/// first damage vector, the second the other vector with its second damage vector, into one
/// aggregate in that order. Equipment totals added from the copied pools are not recorded.
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

/// The air-combat window's initial mission filter, the ground-mission bits.
pub const DEFAULT_MISSIONS: u32 = 0x1f7fa;

/// The window used by the air and ground grids: the last day.
pub const LAST_DAY: i32 = 24;

/// One received-damage grid line in the strategic air view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Item {
    pub cause: u32,
    pub amount: i64,
    /// The item's colour and tooltip side: 1 and 3 read as ours (green, `_BY_US`), others
    /// as the enemy's (red, `_BY_ENEMY`).
    pub flag: u32,
}

impl Item {
    pub fn ours(self) -> bool {
        self.flag.wrapping_sub(1) & 0xffff_fffd == 0
    }
}

/// Each listed cause's amount (cause 4 adds cause 9's), kept when nonzero, not cause 9
/// and allowed by the view's mission filter: causes 0 and 7 need bit 3, 1 any of 0x3190,
/// 2 bit 1 and 6 bit 9; 3, 4, 5, 10 always.
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

/// The two grids of the default strategic air view (view mode zero and the initial filter): the air
/// grid uses mission bits 5 over unclassified records, causes 4, 5 and 9, for one side. The ground
/// grid uses the filter over classified records, causes 4, 5, 3, 9 and 10, for our missions; for
/// the enemy's it uses bits 5 then the filter, causes 4, 5 and 9 then 2, 0, 1, 6, 7 and 9.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Grids {
    pub air: Vec<Item>,
    pub ground: Vec<Item>,
}

pub fn grids(retained: Retained<'_>, now: Hour, enemy: bool) -> Result<Grids, &'static str> {
    let filter = |missions, classification| Filter {
        missions,
        window: LAST_DAY,
        classification,
    };
    let side = u32::from(enemy);
    let mut grids = Grids::default();
    let air = received(retained, now, enemy, filter(5, 0))?;
    items(&air, &[4, 5, 9], DEFAULT_MISSIONS, side + 2, &mut grids.air);
    if enemy {
        let first = received(retained, now, true, filter(5, 1))?;
        items(
            &first,
            &[4, 5, 9],
            DEFAULT_MISSIONS,
            side,
            &mut grids.ground,
        );
        let second = received(retained, now, true, filter(DEFAULT_MISSIONS, 1))?;
        items(
            &second,
            &[2, 0, 1, 6, 7, 9],
            DEFAULT_MISSIONS,
            side,
            &mut grids.ground,
        );
    } else {
        let ground = received(retained, now, false, filter(DEFAULT_MISSIONS, 1))?;
        items(
            &ground,
            &[4, 5, 3, 9, 10],
            DEFAULT_MISSIONS,
            side,
            &mut grids.ground,
        );
    }
    Ok(grids)
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
        // An out-of-order record after the first kept one stays, as the
        // native forward scan stops at the first record it keeps.
        let row = AirRegionCombatRow {
            own: vec![
                record(kept - 2, 1, 0, 1),
                record(kept - 1, 1, 0, 1),
                record(kept, 1, 0, 1),
                record(kept - 9, 1, 0, 1),
                record(HOUR, 1, 0, 1),
            ],
            foreign: vec![record(kept - 1, 1, 0, 1)],
            ..Default::default()
        };
        let state = retained(&row, HOUR, 24);
        assert_eq!(
            state.own.iter().map(|r| r.date).collect::<Vec<_>>(),
            [kept, kept - 9, HOUR]
        );
        assert!(state.foreign.is_empty());
        // A mod's longer timeout keeps more; the cutoff wraps as native i32.
        assert_eq!(retained(&row, HOUR, 48).own.len(), 5);
        assert_eq!(cutoff(i32::MIN, 1), i32::MAX);
    }

    #[test]
    fn the_latest_visiting_pass_alone_prunes_and_no_visit_keeps_the_row() {
        let pass = last_pass(HOUR).unwrap();
        let first = pass - 24 * 5;
        let mut asked = Vec::new();
        // Only the pass two days back visited the country.
        let found = visiting_pass(HOUR, first, |at| {
            asked.push(at);
            Ok(at == pass - 48)
        });
        assert_eq!(found, Ok(Some(pass - 48)));
        assert_eq!(asked, [pass, pass - 24, pass - 48]);
        // No visit since the branch's first sample: the pass at that sample is not asked.
        let mut asked = Vec::new();
        assert_eq!(
            visiting_pass(HOUR, first, |at| {
                asked.push(at);
                Ok(false)
            }),
            Ok(None)
        );
        assert_eq!(asked.last(), Some(&(first + 24)));
        // An unknown roster before any visit is the refusal.
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
            own: vec![record(older - 1, 1, 0, 1), record(older, 1, 0, 1)],
            foreign: vec![record(older - 5, 1, 0, 1)],
            ..Default::default()
        };
        let state = retained_through(&row, Some(pass - 48), 24);
        assert_eq!(
            state.own.iter().map(|r| r.date).collect::<Vec<_>>(),
            [older]
        );
        assert!(state.foreign.is_empty());
        // The latest pass's cutoff would have removed both.
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
            own: vec![record(Hour::MIN, 1, 0, 1)],
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
        assert_eq!(visiting_pass(Hour::MIN + 3, Hour::MIN, |_| Ok(true)), Ok(None));
    }

    #[test]
    fn received_sums_the_window_by_cause_and_replaces_cause_three_by_its_plane_ratio() {
        let mut ours = record(HOUR - 3, 0b101, 0, 4);
        ours.second_damage = vec![damage(3, 200_000), damage(4, 100_000)];
        let mut old = record(HOUR - 30, 0b101, 0, 9);
        old.second_damage = vec![damage(4, 900_000)];
        let mut other_class = record(HOUR, 0b101, 1, 9);
        other_class.second_damage = vec![damage(4, 900_000)];
        // The reverse scan stops at the first record older than the window,
        // so `old` and anything before it never count.
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
        // 2 disrupted over 4 planes; the foreign scan has no planes, so the
        // ratio is not divided again.
        assert_eq!(total.amounts[3], 50_000);
        assert_eq!(total.amounts[2], 50_000);
        // From the enemy's side the vectors and damage channels swap.
        let enemy_total = received(state, HOUR, true, filter).unwrap();
        assert_eq!(enemy_total.amounts[4], 7);
        assert_eq!(enemy_total.amounts[2], 0);
        // A cause outside the aggregate is refused rather than indexed.
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
        // The second scan divides the first scan's ratio by its own planes.
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
        // Cause 4 carries cause 9's amount; 9 itself and zero amounts are
        // never items.
        assert_eq!(
            ours.air,
            [Item {
                cause: 4,
                amount: 300_000,
                flag: 2
            }]
        );
        // The bombers' one disrupted plane over five, divided again by the enemy scan's
        // two planes (cause 3 is replaced per scan).
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
        // Our bombers' records are the enemy's foreign vector: the enemy
        // ground grid reads their first damage channel, causes 0 then 1.
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
