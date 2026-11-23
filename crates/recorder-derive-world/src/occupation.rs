
use recorder_derive_framework::{
    forces::{self, invalid, number, written},
    framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns},
    holders::{self, Holders, Identity},
};
use replay_columnar::{leaf::Writer as Bytes, store::TableView};
use replay_data::rows::{StateResistanceRow, StateRow, TableId};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    io,
};

const RESISTANCE_COLUMNS: [usize; 4] = columns::<StateResistanceRow, 4>(["country", "level", "compliance", "active"]);
const STATE_COLUMNS: [usize; 1] = columns::<StateRow, 1>(["controller"]);

const READS: &[Read] = &[
    Read { table: TableId::StateResistance, columns: Columns::These(&RESISTANCE_COLUMNS), watch: Watch::Changed, required: true },
    Read { table: TableId::States, columns: Columns::These(&STATE_COLUMNS), watch: Watch::Changed, required: true },
    Read { table: TableId::StateGarrisons, columns: Columns::These(&[]), watch: Watch::Changed, required: false },
    Read { table: TableId::Countries, columns: Columns::These(&holders::COUNTRY_SLOT), watch: Watch::Changed, required: true },
    Read { table: TableId::CountryLinks, columns: Columns::These(&holders::LINK_SLOT), watch: Watch::Changed, required: true },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Occupied {
    controller: Option<Identity>,
    level: i64,
    compliance: i64,
}

#[derive(Clone, Copy, Default)]
struct Held {
    garrisons: u32,
    occupied: bool,
}

#[derive(Default)]
struct Occupier {
    resistance: i64,
    compliance: i64,
    occupied: i64,
    garrisons: i64,
    held: HashMap<u32, Held>,
    bare: i64,
}

#[derive(Default)]
pub struct Occupation {
    rebuild: bool,
    holders: Holders,
    occupied: HashMap<u32, Occupied>,
    garrisons: HashMap<u128, Option<Identity>>,
    occupiers: HashMap<Identity, Occupier>,
    unresolved: i64,
    unresolved_garrisons: i64,
    garrisons_complete: bool,
    dirty: HashSet<Identity>,
}

impl Occupation {
    fn occupy(&mut self, state: u32, occupied: Occupied, sign: i64) {
        let Some(identity) = occupied.controller else {
            self.unresolved += sign;
            return;
        };
        self.dirty.insert(identity);
        let occupier = self.occupiers.entry(identity).or_default();
        if sign > 0 {
            occupier.resistance = occupier.resistance.wrapping_add(occupied.level);
            occupier.compliance = occupier.compliance.wrapping_add(occupied.compliance);
        } else {
            occupier.resistance = occupier.resistance.wrapping_sub(occupied.level);
            occupier.compliance = occupier.compliance.wrapping_sub(occupied.compliance);
        }
        occupier.occupied += sign;
        let held = occupier.held.entry(state).or_default();
        held.occupied = sign > 0;
        if held.garrisons == 0 {
            occupier.bare += sign;
        }
        Self::settle(&mut self.occupiers, identity, state);
    }

    fn garrison(&mut self, controller: Option<Identity>, state: u32, sign: i64) {
        let Some(identity) = controller else {
            self.unresolved_garrisons += sign;
            return;
        };
        self.dirty.insert(identity);
        let occupier = self.occupiers.entry(identity).or_default();
        occupier.garrisons += sign;
        let held = occupier.held.entry(state).or_default();
        let before = held.garrisons;
        held.garrisons = held.garrisons.saturating_add_signed(sign as i32);
        if held.occupied && (before == 0) != (held.garrisons == 0) {
            occupier.bare -= sign;
        }
        Self::settle(&mut self.occupiers, identity, state);
    }

    fn settle(occupiers: &mut HashMap<Identity, Occupier>, identity: Identity, state: u32) {
        let Some(occupier) = occupiers.get_mut(&identity) else { return };
        if occupier.held.get(&state).is_some_and(|held| held.garrisons == 0 && !held.occupied) {
            occupier.held.remove(&state);
        }
        if occupier.held.is_empty() {
            occupiers.remove(&identity);
        }
    }

    fn read_occupied(&self, resistance: &TableView<'_>, states: &TableView<'_>, slot: u32) -> io::Result<(u32, Option<Occupied>)> {
        let table = TableId::StateResistance;
        let state = resistance.key(slot).ok_or_else(|| invalid(format!("StateResistance slot {slot} holds no row")))? as u32;
        let cell = |at: usize| number(resistance, table, RESISTANCE_COLUMNS[at], slot);
        let (country, active) = (cell(0)? as u32, cell(3)?);
        if active == 0 || country as i32 <= 0 {
            return Ok((state, None));
        }
        let controller = match states.slot(u128::from(state)) {
            Some(row) => self.holders.identity(number(states, TableId::States, STATE_COLUMNS[0], row)? as u32),
            None => None,
        };
        Ok((state, Some(Occupied { controller, level: cell(1)?, compliance: cell(2)? })))
    }

    fn read_garrisons(&mut self, hour: &Hour<'_>, complete: bool) -> io::Result<()> {
        for (key, controller) in std::mem::take(&mut self.garrisons) {
            self.garrison(controller, key as u32, -1);
        }
        self.garrisons_complete = complete;
        if !complete {
            return Ok(());
        }
        let view = hour.table(TableId::StateGarrisons);
        for slot in view.rows() {
            let key = view.key(slot).ok_or_else(|| invalid(format!("StateGarrisons slot {slot} holds no row")))?;
            self.add_garrison(key);
        }
        Ok(())
    }

    fn add_garrison(&mut self, key: u128) {
        let controller = self.holders.identity((key >> 32) as u32);
        if self.garrisons.insert(key, controller).is_none() {
            self.garrison(controller, key as u32, 1);
        }
    }

    fn read_all(&mut self, hour: &Hour<'_>, garrisons: bool) -> io::Result<()> {
        self.holders.read(hour);
        self.occupied.clear();
        self.garrisons.clear();
        self.occupiers.clear();
        self.unresolved = 0;
        self.unresolved_garrisons = 0;
        let (resistance, states) = (hour.table(TableId::StateResistance), hour.table(TableId::States));
        for slot in resistance.rows() {
            if let (state, Some(occupied)) = self.read_occupied(&resistance, &states, slot)? {
                self.occupy(state, occupied, 1);
                self.occupied.insert(state, occupied);
            }
        }
        self.read_garrisons(hour, garrisons)
    }

    fn apply_garrisons(&mut self, hour: &Hour<'_>) -> io::Result<()> {
        for key in hour.deleted_keys(TableId::StateGarrisons) {
            if let Some(controller) = self.garrisons.remove(&key) {
                self.garrison(controller, key as u32, -1);
            }
        }
        let view = hour.table(TableId::StateGarrisons);
        for &slot in &hour.changes(TableId::StateGarrisons).inserted {
            let key = view.key(slot).ok_or_else(|| invalid(format!("StateGarrisons slot {slot} holds no row")))?;
            self.add_garrison(key);
        }
        Ok(())
    }

    fn apply_states(&mut self, hour: &Hour<'_>) -> io::Result<()> {
        let (resistance, states) = (hour.table(TableId::StateResistance), hour.table(TableId::States));
        let mut moved: BTreeSet<u32> = hour.deleted_keys(TableId::StateResistance).map(|key| key as u32).collect();
        moved.extend(written(hour, TableId::StateResistance, &RESISTANCE_COLUMNS).into_iter().filter_map(|slot| resistance.key(slot)).map(|key| key as u32));
        moved.extend(hour.deleted_keys(TableId::States).map(|key| key as u32).filter(|state| self.occupied.contains_key(state)));
        moved.extend(
            written(hour, TableId::States, &STATE_COLUMNS)
                .into_iter()
                .filter_map(|slot| states.key(slot))
                .map(|key| key as u32)
                .filter(|state| self.occupied.contains_key(state)),
        );
        for state in moved {
            let now = match resistance.slot(u128::from(state)) {
                Some(slot) => self.read_occupied(&resistance, &states, slot)?.1,
                None => None,
            };
            let before = self.occupied.get(&state).copied();
            if before == now {
                continue;
            }
            if let Some(before) = before {
                self.occupy(state, before, -1);
                self.occupied.remove(&state);
            }
            if let Some(now) = now {
                self.occupy(state, now, 1);
                self.occupied.insert(state, now);
            }
        }
        Ok(())
    }

    fn flags(&self) -> (bool, bool, bool) {
        (self.unresolved == 0, self.garrisons_complete, self.unresolved_garrisons == 0)
    }

    fn row(&self, key: u128) -> Option<Vec<Value>> {
        let occupier = self.occupiers.get(&holders::of(key))?;
        let known = self.unresolved == 0;
        let cell = |value: i64| if known { Value::I64(value) } else { Value::Unrecorded };
        let complete = known && self.garrisons_complete && self.unresolved_garrisons == 0 && occupier.garrisons > 0 && occupier.bare == 0;
        Some(vec![
            cell(occupier.resistance),
            cell(occupier.compliance),
            cell(occupier.occupied),
            cell(occupier.held.len() as i64),
            Value::U8(u8::from(complete)),
        ])
    }
}

impl Writer for Occupation {
    fn name(&self) -> &'static str {
        "Occupation"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::OccupierTotals)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let before = self.flags();
        let garrisons = hour.complete(TableId::StateGarrisons);
        let whole = hour.fresh || self.rebuild || Holders::moved(hour);
        if whole {
            self.read_all(hour, garrisons)?;
            self.rebuild = false;
        } else {
            if garrisons != self.garrisons_complete {
                self.read_garrisons(hour, garrisons)?;
            } else if garrisons {
                self.apply_garrisons(hour)?;
            }
            self.apply_states(hour)?;
        }
        let all = whole || before != self.flags();
        let dirty = std::mem::take(&mut self.dirty);
        let keys: Vec<u128> = match all {
            true => self.occupiers.keys().copied().map(holders::key).collect(),
            false => dirty.into_iter().map(holders::key).collect(),
        };
        forces::write(out, TableId::OccupierTotals, keys, all, |key| self.row(key));
        Ok(())
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if !blob.is_empty() {
            return Err(invalid("Occupation's state holds nothing"));
        }
        self.rebuild = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
