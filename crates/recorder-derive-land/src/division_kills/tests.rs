use super::*;
use recorder_derive_framework::testing::deployed::{Edits, Run};

const INPUTS: &[TableId] = &[TableId::LandDivisionDamage, TableId::LandCombatEdges];
const BATTLE: u64 = 0x8000_0002_0000_003e;
const TARGET: u64 = 0x8000_0001_0000_0100;
const OTHER: u64 = 0x8000_0001_0000_0101;
const FIRST: u64 = 0x8000_0001_0000_0200;
const SECOND: u64 = 0x8000_0001_0000_0201;
const OTHER_BATTLE: u64 = 0x8000_0002_0000_0040;

fn run() -> Run {
    Run::new(|| vec![Box::new(DivisionKills)], INPUTS)
}

fn damage(exit: u64, ordinal: u64, target: u64, attacker: u64, strength_raw: i64) -> LandDivisionDamageRow {
    LandDivisionDamageRow { id: exit << 16 | ordinal, hour: 0, combat: BATTLE, target, attacker, army: Some(7), strength_raw }
}

#[test]
fn damage_dealt_in_one_combat_is_shared_against_the_exit_from_another_that_flushed_the_pool() {
    let mut run = run();
    let mut hour = Edits::default();
    hour.put(TableId::LandDivisionDamage, LandDivisionDamageRow { combat: OTHER_BATTLE, ..damage(4, 0, TARGET, FIRST, 50) })
        .put(TableId::LandDivisionDamage, damage(4, 1, TARGET, SECOND, 50))
        .put(TableId::LandCombatEdges, exit(4, TARGET, 30, Some(100)));
    run.hour(&hour);
    assert_eq!(kills(&run), [(FIRST, 15 * MEN, None), (SECOND, 15 * MEN, None)]);
}

fn exit(id: u64, division: u64, losses: i32, pool: Option<i64>) -> LandCombatEdgesRow {
    LandCombatEdgesRow {
        id,
        kind: LandCombatEdgesRow::EXITED,
        combat: BATTLE,
        division: Some(division),
        casualties_before: Some(1_000),
        casualties_after: Some(1_000 + losses),
        pending_strength_raw: pool,
        ..Default::default()
    }
}

fn kills(run: &Run) -> Vec<(u64, i64, Option<i32>)> {
    run.rows::<DivisionKillsRow>(TableId::DivisionKills).into_iter().map(|row| (row.division, row.killed_raw, row.incomplete_from)).collect()
}

const MEN: i64 = 100_000;

#[test]
fn a_target_s_losses_are_shared_by_strength_damage_against_its_whole_pending_damage() {
    let mut run = run();
    let mut hour = Edits::default();
    hour.put(TableId::LandDivisionDamage, damage(3, 0, TARGET, FIRST, 300))
        .put(TableId::LandDivisionDamage, damage(3, 1, TARGET, SECOND, 100))
        .put(TableId::LandCombatEdges, exit(3, TARGET, 800, Some(500)))
        .put(TableId::LandCombatEdges, exit(9, TARGET, 0, Some(0)));
    run.hour(&hour);
    assert_eq!(run.status(TableId::DivisionKills), Status::Complete, "{}", run.why(TableId::DivisionKills));
    assert_eq!(
        kills(&run),
        [(FIRST, 480 * MEN, None), (SECOND, 160 * MEN, None)],
        "a fifth of the pool came from no division (air support), so a fifth of the losses goes to no one; the target's later empty exit in the same hour is not the one its damage closed on"
    );
    let mut next = Edits::default();
    next.put(TableId::LandDivisionDamage, damage(5, 0, OTHER, FIRST, 50)).put(TableId::LandCombatEdges, exit(5, OTHER, 3, Some(50)));
    run.hour(&next);
    assert_eq!(kills(&run), [(FIRST, 483 * MEN, None), (SECOND, 160 * MEN, None)], "kills add up across battles");
}

#[test]
fn a_share_that_cannot_be_worked_out_leaves_the_count_incomplete_from_that_hour() {
    let mut run = run();
    let mut hour = Edits::default();
    hour.put(TableId::LandDivisionDamage, damage(2, 0, TARGET, FIRST, 300)).put(TableId::LandCombatEdges, exit(2, TARGET, 800, Some(500)));
    run.hour(&hour);
    let mut unread = Edits::default();
    unread
        .put(TableId::LandDivisionDamage, damage(5, 0, OTHER, FIRST, 40))
        .put(TableId::LandDivisionDamage, damage(6, 0, TARGET, SECOND, 40))
        .put(TableId::LandCombatEdges, exit(5, OTHER, 9, None))
        .put(TableId::LandCombatEdges, exit(6, TARGET, 9, Some(30)));
    run.hour(&unread);
    assert_eq!(
        kills(&run),
        [(FIRST, 480 * MEN, Some(1)), (SECOND, 0, Some(1))],
        "an unread pool, or more division damage than the pool holds, is never shared as a guess"
    );
    let mut later = Edits::default();
    later.put(TableId::LandDivisionDamage, damage(8, 0, TARGET, FIRST, 10)).put(TableId::LandCombatEdges, exit(8, TARGET, 10, Some(10)));
    run.hour(&later);
    assert_eq!(kills(&run)[0], (FIRST, 480 * MEN, Some(1)), "an incomplete count stays incomplete");
}

#[test]
fn counting_starts_at_the_first_recorded_hour_and_ends_at_the_first_unrecorded_one() {
    let mut run = run();
    let mut unrecorded = Edits::default();
    unrecorded.unavailable(TableId::LandDivisionDamage, b"land events before first sample/load/gap unknown");
    run.hour(&unrecorded);
    assert_eq!(run.why(TableId::DivisionKills), format!("unavailable: {}", REASONS[NOT_YET]));
    let mut first = Edits::default();
    first.put(TableId::LandDivisionDamage, damage(2, 0, TARGET, FIRST, 1)).put(TableId::LandCombatEdges, exit(2, TARGET, 5, Some(1)));
    first.status(TableId::LandDivisionDamage, Status::Complete);
    run.hour(&first);
    assert_eq!(run.status(TableId::DivisionKills), Status::Complete);
    assert_eq!(kills(&run), [(FIRST, 5 * MEN, None)]);
    let mut lost = Edits::default();
    lost.unavailable(TableId::LandCombatEdges, b"land event drain capacity exceeded");
    run.hour(&lost);
    assert_eq!(run.why(TableId::DivisionKills), format!("unavailable: {}", REASONS[LOST]));
    let mut recovered = Edits::default();
    recovered.status(TableId::LandCombatEdges, Status::Complete);
    run.hour(&recovered);
    assert_eq!(run.why(TableId::DivisionKills), format!("unavailable: {}", REASONS[LOST]), "a gap leaves every later count short, so it is never complete again in the branch");
    let resumed = run.resumed();
    assert_eq!(resumed.why(TableId::DivisionKills), format!("unavailable: {}", REASONS[LOST]));
}
