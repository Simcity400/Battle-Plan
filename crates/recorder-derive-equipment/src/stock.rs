
use recorder_derive_framework::{
    forces::{self, Owners},
    framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns},
};
use replay_columnar::{leaf::Writer as Bytes, store::Kept};
use replay_data::rows::{CountryLinkRow, StockpileRow, TableId};
use std::{collections::HashMap, io};

#[cfg(test)]
mod tests;

const LINK: [usize; 1] = columns::<CountryLinkRow, 1>(["canonical_slot"]);
const STOCK: [usize; 2] = columns::<StockpileRow, 2>(["variant_handle", "amount_raw"]);

const READS: &[Read] = &[
    Read { table: TableId::CountryLinks, columns: Columns::These(&LINK), watch: Watch::Changed, required: true },
    Read { table: TableId::Stockpiles, columns: Columns::These(&STOCK), watch: Watch::Dropped, required: true },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Position {
    country: u32,
    variant: u64,
    amount: i64,
}

fn key(slot: u32, variant: u64) -> u128 {
    u128::from(slot) << 64 | u128::from(variant)
}

#[derive(Default)]
pub struct Stockpile {
    rebuild: bool,
    owners: Owners,
    unlinked: u64,
    wide: HashMap<u128, i128>,
}

fn position(hour: &Hour<'_>, slot: u32) -> io::Result<Position> {
    let view = hour.table(TableId::Stockpiles);
    let key = view.key(slot).ok_or_else(|| forces::invalid(format!("Stockpiles slot {slot} holds no row")))?;
    Ok(Position {
        country: (key >> 32) as u32,
        variant: forces::number(&view, TableId::Stockpiles, STOCK[0], slot)? as u64,
        amount: forces::number(&view, TableId::Stockpiles, STOCK[1], slot)?,
    })
}

fn kept(cell: &Kept) -> io::Result<u64> {
    match cell {
        Kept::Number(bits) => Ok(*bits),
        _ => Err(forces::invalid("a Stockpiles cell was not recorded, which its row cannot say")),
    }
}

impl Stockpile {
    fn rebuild(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        self.owners.read_links(hour);
        self.unlinked = 0;
        let mut sums: HashMap<u128, (i128, i64)> = HashMap::new();
        for slot in hour.table(TableId::Stockpiles).rows() {
            let position = position(hour, slot)?;
            match self.owners.slot(position.country) {
                Some(owner) => {
                    let held = sums.entry(key(owner, position.variant)).or_default();
                    held.0 += i128::from(position.amount);
                    held.1 += 1;
                }
                None => self.unlinked += 1,
            }
        }
        self.wide = sums.iter().filter(|(_, (sum, _))| i64::try_from(*sum).is_err()).map(|(&key, &(sum, _))| (key, sum)).collect();
        let unlinked = self.unlinked;
        forces::write(out, TableId::StockByVariant, sums.keys().copied(), true, |key| {
            let (sum, positions) = *sums.get(&key)?;
            Some(if unlinked != 0 { vec![Value::Unrecorded; 2] } else { vec![forces::exact(sum), Value::I64(positions)] })
        });
        Ok(())
    }

    fn apply(&mut self, hour: &Hour<'_>, out: &mut Rows<'_>) -> io::Result<bool> {
        let changes = hour.changes(TableId::Stockpiles);
        if changes.whole || forces::links_moved(hour) {
            return Ok(false);
        }
        let mut earlier: HashMap<(u32, u16), &Kept> = HashMap::new();
        for (slot, column, cell) in hour.previous(TableId::Stockpiles) {
            earlier.entry((*slot, *column)).or_insert(cell);
        }
        let mut gone: Vec<Position> = Vec::new();
        let mut come: Vec<Position> = Vec::new();
        for (key, cells) in hour.dropped(TableId::Stockpiles) {
            let (Some(variant), Some(amount)) = (cells.get(STOCK[0]), cells.get(STOCK[1])) else {
                return Err(forces::invalid("a dropped Stockpiles row lacks its cells"));
            };
            gone.push(Position { country: (key >> 32) as u32, variant: kept(variant)?, amount: kept(amount)? as i64 });
        }
        let mut written: Vec<u32> = changes.written.iter().filter(|(_, column)| STOCK.contains(&usize::from(*column))).map(|&(slot, _)| slot).collect();
        written.sort_unstable();
        written.dedup();
        for slot in written {
            let now = position(hour, slot)?;
            let before = |column: usize, held: u64| earlier.get(&(slot, column as u16)).map_or(Ok(held), |cell| kept(cell));
            gone.push(Position { country: now.country, variant: before(STOCK[0], now.variant)?, amount: before(STOCK[1], now.amount as u64)? as i64 });
            come.push(now);
        }
        for &slot in &changes.inserted {
            come.push(position(hour, slot)?);
        }
        let unlinked = self.unlinked;
        let mut moved: HashMap<u128, (i128, i64)> = HashMap::new();
        for (positions, sign) in [(&gone, -1_i64), (&come, 1)] {
            for position in positions {
                match self.owners.slot(position.country) {
                    Some(owner) => {
                        let change = moved.entry(key(owner, position.variant)).or_default();
                        change.0 += i128::from(sign) * i128::from(position.amount);
                        change.1 += sign;
                    }
                    None if sign > 0 => self.unlinked += 1,
                    None => self.unlinked -= 1,
                }
            }
        }
        if unlinked != 0 || self.unlinked != 0 {
            return Ok(unlinked == self.unlinked && unlinked != 0 && moved.is_empty());
        }
        let mut sums: Vec<(u128, i128, i64)> = Vec::with_capacity(moved.len());
        {
            let view = out.table(TableId::StockByVariant);
            for (key, (change, count)) in moved {
                if (change, count) == (0, 0) {
                    continue;
                }
                let (held, positions) = match view.slot(key) {
                    Some(slot) => {
                        let Some(positions) = view.number(1, slot) else { return Ok(false) };
                        let sum = match (self.wide.get(&key), view.number(0, slot)) {
                            (Some(&sum), _) => sum,
                            (None, Some(sum)) => i128::from(sum),
                            (None, None) => return Ok(false),
                        };
                        (sum, positions)
                    }
                    None => (0, 0),
                };
                sums.push((key, held + change, positions + count));
            }
        }
        for (key, sum, positions) in sums {
            if positions <= 0 || i64::try_from(sum).is_ok() {
                self.wide.remove(&key);
            } else {
                self.wide.insert(key, sum);
            }
            if positions <= 0 {
                out.delete(TableId::StockByVariant, key);
            } else {
                out.set(TableId::StockByVariant, key, vec![forces::exact(sum), Value::I64(positions)]);
            }
        }
        Ok(true)
    }
}

impl Writer for Stockpile {
    fn name(&self) -> &'static str {
        "Stockpile"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::StockByVariant)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        if hour.fresh || self.rebuild || !self.apply(hour, out)? {
            self.rebuild(hour, out)?;
            self.rebuild = false;
        }
        Ok(())
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if !blob.is_empty() {
            return Err(forces::invalid("Stockpile's state holds nothing"));
        }
        self.rebuild = true;
        Ok(())
    }
}
