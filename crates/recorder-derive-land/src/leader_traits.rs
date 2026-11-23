
#[cfg(test)]
mod tests;

use recorder_derive_framework::framework::{Columns, Hour, Keys, Output, Read, Rows, Value, Watch, Writer, columns};
use replay_columnar::{
    leaf::Writer as Bytes,
    store::{TableView, Value as Held},
};
use replay_data::rows::{LeaderTraitGainRow, TableId, UnitLeaderTraitRow};
use std::{
    collections::{HashMap, HashSet},
    io,
};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

const TOKEN: [usize; 1] = columns::<UnitLeaderTraitRow, 1>(["token_id"]);
const SPELLING: [usize; 1] = columns::<UnitLeaderTraitRow, 1>(["token"]);

const READS: &[Read] = &[
    Read { table: TableId::UnitLeaderTraits, columns: Columns::These(&TOKEN), watch: Watch::Dropped, required: true },
    Read { table: TableId::UnitLeaders, columns: Columns::These(&[]), watch: Watch::Changed, required: true },
];

#[derive(Default)]
pub struct LeaderTraits;

fn leader(key: u128) -> u64 {
    (key >> 32) as u64
}

fn token(view: &TableView<'_>, slot: u32) -> Option<u32> {
    view.number(TOKEN[0], slot).map(|token| token as u32)
}

fn gain(leader: u64, hour: i32, token: u32) -> u128 {
    u128::from(leader) << 64 | u128::from(hour as u32) << 32 | u128::from(token)
}

fn set(hour: &Hour<'_>, out: &mut Rows, slot: u32, leader: u64, token: u32, class: u8) {
    let traits = hour.table(TableId::UnitLeaderTraits);
    let spelling = match traits.text(SPELLING[0], slot).and_then(|id| hour.text(id)) {
        Some(text) => out.text(text),
        None => Value::Unrecorded,
    };
    out.append(TableId::LeaderTraitGains, gain(leader, hour.hour, token), vec![Value::U8(class), spelling]);
    date(out, hour.hour, leader, token, class, spelling);
}

fn date_key(leader: u64, token: u32) -> u128 {
    u128::from(leader) << 32 | u128::from(token)
}

fn date(out: &mut Rows, at: i32, leader: u64, token: u32, class: u8, spelling: Value) {
    let table = TableId::LeaderTraitDates;
    let key = date_key(leader, token);
    let held = out.table(table);
    let cells = match held.slot(key) {
        None => vec![Value::I32(at), Value::U8(class), Value::OptionalI32((class == LeaderTraitGainRow::GAINED).then_some(at)), spelling],
        Some(_) if class != LeaderTraitGainRow::GAINED => return,
        Some(slot) => vec![
            held.number(0, slot).map_or(Value::Unrecorded, |first| Value::I32(first as i32)),
            held.number(1, slot).map_or(Value::Unrecorded, |first| Value::U8(first as u8)),
            Value::OptionalI32(Some(at)),
            held.text(3, slot).map_or(Value::Unrecorded, Value::Bytes),
        ],
    };
    out.set(table, key, cells);
}

fn forget(hour: &Hour<'_>, out: &mut Rows) {
    let leaders = hour.table(TableId::UnitLeaders);
    let gone: HashSet<u64> = hour.deleted_keys(TableId::UnitLeaders).filter(|&key| leaders.slot(key).is_none()).map(|key| key as u64).collect();
    if gone.is_empty() {
        return;
    }
    let held = out.table(TableId::LeaderTraitDates);
    let keys: Vec<u128> = held.rows().filter_map(|slot| held.key(slot)).filter(|key| gone.contains(&((key >> 32) as u64))).collect();
    for key in keys {
        out.delete(TableId::LeaderTraitDates, key);
    }
}

impl Writer for LeaderTraits {
    fn name(&self) -> &'static str {
        "LeaderTraits"
    }

    fn version(&self) -> u32 {
        3
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::Log(TableId::LeaderTraitGains, Keys::Hour(32)), Output::State(TableId::LeaderTraitDates)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn hourly(&self) -> bool {
        true
    }

    fn start(&mut self) {}

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        recorder_derive_framework::forces::forget_listed(hour, out, TableId::LeaderTraitGains);
        if !hour.fresh {
            forget(hour, out);
        }
        let traits = hour.table(TableId::UnitLeaderTraits);
        if hour.fresh {
            let class = if hour.first { LeaderTraitGainRow::HELD_BEFORE } else { LeaderTraitGainRow::FIRST_OBSERVED };
            let mut seen = HashSet::new();
            for slot in traits.rows() {
                let (Some(key), Some(token)) = (traits.key(slot), token(&traits, slot)) else { continue };
                if seen.insert((leader(key), token)) {
                    set(hour, out, slot, leader(key), token, class);
                }
            }
            return Ok(());
        }
        let written = hour.written_rows(TableId::UnitLeaderTraits, Columns::These(&TOKEN));
        if written.is_empty() {
            return Ok(());
        }
        let mut before: HashMap<u64, HashSet<u32>> = HashMap::new();
        let mut first = HashSet::new();
        for (slot, column, value) in hour.previous(TableId::UnitLeaderTraits) {
            if usize::from(*column) != TOKEN[0] || !first.insert(*slot) {
                continue;
            }
            if let (Some(key), Held::Number(held)) = (traits.key(*slot), value.value()) {
                before.entry(leader(key)).or_default().insert(held as u32);
            }
        }
        for (key, cells) in hour.dropped(TableId::UnitLeaderTraits) {
            if let Some(Held::Number(held)) = cells.get(TOKEN[0]).map(|cell| cell.value()) {
                before.entry(leader(*key)).or_default().insert(held as u32);
            }
        }
        let leaders = hour.table(TableId::UnitLeaders);
        let new: HashSet<u64> = hour.changes(TableId::UnitLeaders).inserted.iter().filter_map(|&slot| leaders.key(slot)).map(|key| key as u64).collect();
        let mut gained = Vec::with_capacity(written.len());
        for slot in written {
            let key = traits.key(slot).ok_or_else(|| invalid(format!("UnitLeaderTraits slot {slot} holds no row")))?;
            let Some(token) = token(&traits, slot) else { continue };
            let leader = leader(key);
            if !before.get(&leader).is_some_and(|held| held.contains(&token)) {
                gained.push((leader, token, slot));
            }
        }
        gained.sort_unstable();
        gained.dedup_by_key(|(leader, token, _)| (*leader, *token));
        for (leader, token, slot) in gained {
            let class = if new.contains(&leader) { LeaderTraitGainRow::FIRST_OBSERVED } else { LeaderTraitGainRow::GAINED };
            set(hour, out, slot, leader, token, class);
        }
        Ok(())
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if blob.is_empty() { Ok(()) } else { Err(invalid("LeaderTraits' state holds nothing")) }
    }
}
