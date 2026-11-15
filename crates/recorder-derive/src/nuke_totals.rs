//! `CountryNukeTotals`: each live country's nuclear stockpile as the top bar adds it, so a
//! country's total ranks and draws over time as a declared series. State is the tables: each
//! country's types as Nukes holds them, applied as the hour changes them; a fresh hour or a resume
//! reads Nukes and Countries whole.

use crate::{
    forces::{invalid, written},
    framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns},
};
use replay_columnar::{leaf::Writer as Bytes, store::TableView};
use replay_data::rows::{NukeRow, TableId};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
};

const STOCK: [usize; 2] = columns::<NukeRow, 2>(["amount_raw", "ready"]);

const READS: &[Read] = &[
    Read { table: TableId::Nukes, columns: Columns::These(&STOCK), watch: Watch::Changed, required: true },
    // Only countries coming and going.
    Read { table: TableId::Countries, columns: Columns::These(&[]), watch: Watch::Changed, required: true },
];

/// One type of a country's stockpile: whole bombs as shown and ready drops; none while a cell is
/// not recorded.
type Stock = Option<(i32, i64)>;

/// The Nukes row at `slot`: its country, its type and its stock.
fn stock(view: &TableView<'_>, slot: u32) -> io::Result<(u32, u32, Stock)> {
    let key = view.key(slot).ok_or_else(|| invalid(format!("Nukes slot {slot} holds no row")))?;
    let amount = view.number(STOCK[0], slot);
    let ready = view.number(STOCK[1], slot);
    Ok(((key >> 32) as u32, key as u32, amount.zip(ready).map(|(amount, ready)| (NukeRow::shown(amount), ready))))
}

#[derive(Default)]
pub struct NukeTotals {
    /// The writer must read its inputs whole before its next hour: a resume.
    rebuild: bool,
    countries: HashSet<u32>,
    /// Each country's types with a Nukes row, by type.
    stocks: HashMap<u32, BTreeMap<u32, Stock>>,
}

impl NukeTotals {
    fn set(&mut self, country: u32, kind: u32, stock: Option<Stock>) {
        match stock {
            Some(stock) => {
                self.stocks.entry(country).or_default().insert(kind, stock);
            }
            None => {
                if let Some(types) = self.stocks.get_mut(&country) {
                    types.remove(&kind);
                    if types.is_empty() {
                        self.stocks.remove(&country);
                    }
                }
            }
        }
    }

    /// `country`'s row: total and ready, each not recorded while a stock it adds is not.
    fn cells(&self, country: u32) -> Vec<Value> {
        let types = self.stocks.get(&country);
        let mut total = Some(0_i32);
        let mut ready = Some(0_i64);
        for stock in types.into_iter().flat_map(BTreeMap::values) {
            match stock {
                Some((shown, drops)) => {
                    total = total.map(|sum| sum.wrapping_add(*shown));
                    ready = ready.and_then(|sum| sum.checked_add(*drops));
                }
                None => (total, ready) = (None, None),
            }
        }
        vec![total.map_or(Value::Unrecorded, |total| Value::I64(i64::from(total))), ready.map_or(Value::Unrecorded, Value::I64)]
    }
}

impl Writer for NukeTotals {
    fn name(&self) -> &'static str {
        "NukeTotals"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::CountryNukeTotals)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let table = TableId::CountryNukeTotals;
        let whole = hour.fresh || self.rebuild;
        let countries = hour.table(TableId::Countries);
        let nukes = hour.table(TableId::Nukes);
        let mut dirty = BTreeSet::new();
        if whole {
            self.rebuild = false;
            self.countries = countries.rows().filter_map(|slot| countries.key(slot)).map(|key| key as u32).collect();
            self.stocks.clear();
            for slot in nukes.rows() {
                let (country, kind, stock) = stock(&nukes, slot)?;
                self.set(country, kind, Some(stock));
            }
            dirty.extend(self.countries.iter().copied());
            dirty.extend(self.stocks.keys().copied());
            dirty.extend(crate::forces::held(out, table).into_iter().map(|key| key as u32));
        } else {
            let born = hour.wrote(TableId::Countries, Columns::These(&[]));
            if !born && !hour.wrote(TableId::Nukes, Columns::These(&STOCK)) {
                return Ok(());
            }
            for key in hour.deleted_keys(TableId::Countries) {
                self.countries.remove(&(key as u32));
                dirty.insert(key as u32);
            }
            for slot in hour.written_rows(TableId::Countries, Columns::These(&[])) {
                let key = countries.key(slot).ok_or_else(|| invalid(format!("Countries slot {slot} holds no row")))? as u32;
                self.countries.insert(key);
                dirty.insert(key);
            }
            for key in hour.deleted_keys(TableId::Nukes) {
                self.set((key >> 32) as u32, key as u32, None);
                dirty.insert((key >> 32) as u32);
            }
            for slot in written(hour, TableId::Nukes, &STOCK) {
                let (country, kind, stock) = stock(&nukes, slot)?;
                self.set(country, kind, Some(stock));
                dirty.insert(country);
            }
        }
        for country in dirty {
            let key = u128::from(country);
            if self.countries.contains(&country) || self.stocks.contains_key(&country) {
                out.set(table, key, self.cells(country));
            } else if out.table(table).slot(key).is_some() {
                out.delete(table, key);
            }
        }
        Ok(())
    }

    /// Nothing but the tables: a resume reads Nukes and Countries whole.
    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if !blob.is_empty() {
            return Err(invalid("NukeTotals' state holds nothing"));
        }
        self.rebuild = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
