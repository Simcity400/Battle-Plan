use recorder_derive_framework::framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns};
use replay_columnar::{leaf::Writer as Bytes, manifest::Status};
use replay_data::rows::{DivisionKillsRow, LandCombatEdgesRow, LandDivisionDamageRow, TableId};
use std::{collections::HashMap, io};

const DAMAGE: [usize; 2] = columns::<LandDivisionDamageRow, 2>(["target", "attacker"]);
const STRENGTH: [usize; 1] = columns::<LandDivisionDamageRow, 1>(["strength_raw"]);
const EDGES: [usize; 5] = columns::<LandCombatEdgesRow, 5>(["kind", "division", "casualties_before", "casualties_after", "pending_strength_raw"]);
const KILLS: [usize; 2] = columns::<DivisionKillsRow, 2>(["killed_raw", "incomplete_from"]);
const READ_COLUMNS: [usize; 3] = [DAMAGE[0], DAMAGE[1], STRENGTH[0]];
const SCALE: i128 = 100_000;

const READS: &[Read] = &[
    Read { table: TableId::LandDivisionDamage, columns: Columns::These(&READ_COLUMNS), watch: Watch::Changed, required: false },
    Read { table: TableId::LandCombatEdges, columns: Columns::These(&EDGES), watch: Watch::Changed, required: false },
];

pub const NOT_YET: usize = 0;
pub const LOST: usize = 1;
const REASONS: [&str; 2] = [
    "Division damage has not been recorded yet in this branch",
    "Division damage went unrecorded for an hour of this branch, so kills from then on are not known",
];

#[derive(Default)]
pub struct DivisionKills;

fn inserted(hour: &Hour<'_>, table: TableId) -> Vec<u32> {
    let changes = hour.changes(table);
    if changes.whole { hour.table(table).rows().collect() } else { changes.inserted.clone() }
}

struct Exit {
    division: u64,
    losses: Option<i64>,
    pool: Option<i64>,
}

fn exits(hour: &Hour<'_>) -> HashMap<u64, Exit> {
    let table = hour.table(TableId::LandCombatEdges);
    let mut exits = HashMap::new();
    for slot in inserted(hour, TableId::LandCombatEdges) {
        let (Some(key), Some(kind), Some(division)) = (table.key(slot), table.number(EDGES[0], slot), table.number(EDGES[1], slot)) else {
            continue;
        };
        if kind != i64::from(LandCombatEdgesRow::EXITED) {
            continue;
        }
        let losses = table.number(EDGES[2], slot).zip(table.number(EDGES[3], slot)).map(|(before, after)| after - before);
        exits.insert(key as u64, Exit { division: division as u64, losses, pool: table.number(EDGES[4], slot) });
    }
    exits
}

fn shares(hour: &Hour<'_>) -> HashMap<u64, Option<i64>> {
    let exits = exits(hour);
    let table = hour.table(TableId::LandDivisionDamage);
    let mut targets = HashMap::<(u64, u64), (u64, Vec<(u64, i64)>)>::new();
    for slot in inserted(hour, TableId::LandDivisionDamage) {
        let (Some(key), Some(target), Some(attacker), Some(strength)) =
            (table.key(slot), table.number(DAMAGE[0], slot), table.number(DAMAGE[1], slot), table.number(STRENGTH[0], slot))
        else {
            continue;
        };
        let closed = targets.entry((key as u64 >> 16, target as u64)).or_insert((target as u64, Vec::new()));
        match closed.1.iter_mut().find(|(held, _)| *held == attacker as u64) {
            Some((_, total)) => *total = total.saturating_add(strength),
            None => closed.1.push((attacker as u64, strength)),
        }
    }
    let mut killed = HashMap::<u64, Option<i64>>::new();
    for (exit, (target, attackers)) in targets {
        let dealt: i128 = attackers.iter().map(|&(_, strength)| i128::from(strength)).sum();
        let split = exits.get(&exit.0).filter(|exit| exit.division == target).and_then(|exit| match (exit.losses, exit.pool) {
            (Some(losses), Some(pool)) if losses >= 0 && pool > 0 && dealt <= i128::from(pool) => Some((i128::from(losses), i128::from(pool))),
            _ => None,
        });
        for (attacker, strength) in attackers {
            let share = split.and_then(|(losses, pool)| i64::try_from(losses * i128::from(strength) * SCALE / pool).ok());
            let held = killed.entry(attacker).or_insert(Some(0));
            *held = held.zip(share).and_then(|(sum, share)| sum.checked_add(share));
        }
    }
    killed
}

impl Writer for DivisionKills {
    fn name(&self) -> &'static str {
        "DivisionKills"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::DivisionKills)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn hourly(&self) -> bool {
        true
    }

    fn strings(&self) -> &'static [&'static str] {
        &REASONS
    }

    fn start(&mut self) {}

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let lost = Status::Unavailable { reason: hour.strings[LOST] };
        let held = out.table(TableId::DivisionKills).status();
        let recorded = hour.complete(TableId::LandDivisionDamage) && hour.complete(TableId::LandCombatEdges);
        if held == lost || (held == Status::Complete && !recorded) {
            out.status(TableId::DivisionKills, lost);
            return Ok(());
        }
        if !recorded {
            out.status(TableId::DivisionKills, Status::Unavailable { reason: hour.strings[NOT_YET] });
            return Ok(());
        }
        out.status(TableId::DivisionKills, Status::Complete);
        let mut rows = Vec::new();
        {
            let kills = out.table(TableId::DivisionKills);
            for (division, share) in shares(hour) {
                let key = u128::from(division);
                let slot = kills.slot(key);
                let before = slot.and_then(|slot| kills.number(KILLS[0], slot)).unwrap_or(0);
                let incomplete = slot.and_then(|slot| kills.number(KILLS[1], slot)).map(|at| at as i32);
                let (killed, incomplete) = match (incomplete, share.and_then(|share| before.checked_add(share))) {
                    (None, Some(killed)) => (killed, None),
                    (earlier, _) => (before, Some(earlier.unwrap_or(hour.hour))),
                };
                rows.push((key, vec![Value::I64(killed), Value::OptionalI32(incomplete)]));
            }
        }
        for (key, cells) in rows {
            out.set(TableId::DivisionKills, key, cells);
        }
        Ok(())
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if blob.is_empty() {
            Ok(())
        } else {
            Err(io::Error::new(io::ErrorKind::InvalidData, "DivisionKills' state holds nothing"))
        }
    }
}

#[cfg(test)]
mod tests;
