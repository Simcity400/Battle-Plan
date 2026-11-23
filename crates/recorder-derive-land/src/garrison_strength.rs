
use recorder_derive_framework::{
    forces::{self, Owners, invalid, number},
    framework::{
        Columns, Hour, Output, Read, Rows, Uses, Value, Watch, Writer, columns,
        definitions::{Define, Definition, Form},
    },
};
use replay_columnar::leaf::Writer as Bytes;
use replay_data::rows::{CountryLinkRow, GarrisonEquipment, GarrisonEquipmentInputs, GarrisonStrengthRow, StateAssignedManpowerRow, StateGarrisonRow, TableId};
use std::{
    collections::{BTreeSet, HashMap},
    io,
};

const GARRISON: [usize; 4] = columns::<StateGarrisonRow, 4>(["occupied", "required_manpower", "serialized_flag", "equipment"]);
const ASSIGNED: [usize; 2] = columns::<StateAssignedManpowerRow, 2>(["occupied", "amount"]);
const LINK: [usize; 1] = columns::<CountryLinkRow, 1>(["canonical_slot"]);

const READS: &[Read] = &[
    Read { table: TableId::StateGarrisons, columns: Columns::These(&GARRISON), watch: Watch::Changed, required: true },
    Read { table: TableId::StateAssignedManpower, columns: Columns::These(&ASSIGNED), watch: Watch::Changed, required: false },
    Read { table: TableId::CountryLinks, columns: Columns::These(&LINK), watch: Watch::Changed, required: true },
];

const MANPOWER_POWER: Define = Define { section: "NResistance", name: "GARRISON_STR_POW_MANPOWER", form: Form::Int32 };
const EQUIPMENT_POWER: Define = Define { section: "NResistance", name: "GARRISON_STR_POW_EQUIPMENT", form: Form::Int32 };

const DEFINITIONS: &[Uses] = &[Uses { definition: Definition::Define(MANPOWER_POWER), required: true }, Uses { definition: Definition::Define(EQUIPMENT_POWER), required: true }];

const ONE: i64 = 100_000;

#[derive(Default)]
pub struct GarrisonStrength {
    rebuild: bool,
    owners: Owners,
    powers: Option<(i32, i32)>,
    assigned_complete: bool,
    garrisons: HashMap<u32, Vec<u128>>,
    assigned: HashMap<u32, Vec<u128>>,
}

fn add(index: &mut HashMap<u32, Vec<u128>>, state: u32, key: u128) {
    let keys = index.entry(state).or_default();
    if !keys.contains(&key) {
        keys.push(key);
    }
}

fn remove(index: &mut HashMap<u32, Vec<u128>>, state: u32, key: u128) {
    if let Some(keys) = index.get_mut(&state) {
        keys.retain(|held| *held != key);
        if keys.is_empty() {
            index.remove(&state);
        }
    }
}

const fn assigned_state(key: u128) -> u32 {
    (key >> 32) as u32
}

fn powers(hour: &Hour<'_>) -> io::Result<(i32, i32)> {
    let definitions = hour.definitions();
    let value = |define: Define| {
        definitions.define(define.section, define.name).map(|value| value as i32).map_err(|why| invalid(format!("GarrisonStrength holds no {}.{}: {why}", define.section, define.name)))
    };
    Ok((value(MANPOWER_POWER)?, value(EQUIPMENT_POWER)?))
}

impl GarrisonStrength {
    fn read_all(&mut self, hour: &Hour<'_>) -> io::Result<Vec<u128>> {
        self.owners.read_links(hour);
        self.garrisons.clear();
        self.assigned.clear();
        let view = hour.table(TableId::StateGarrisons);
        let mut keys = Vec::new();
        for slot in view.rows() {
            let key = view.key(slot).ok_or_else(|| invalid(format!("StateGarrisons slot {slot} holds no row")))?;
            add(&mut self.garrisons, key as u32, key);
            keys.push(key);
        }
        if self.assigned_complete {
            let view = hour.table(TableId::StateAssignedManpower);
            for slot in view.rows() {
                let key = view.key(slot).ok_or_else(|| invalid(format!("StateAssignedManpower slot {slot} holds no row")))?;
                add(&mut self.assigned, assigned_state(key), key);
            }
        }
        Ok(keys)
    }

    fn apply(&mut self, hour: &Hour<'_>) -> io::Result<Vec<u128>> {
        let mut dirty = BTreeSet::new();
        for key in hour.deleted_keys(TableId::StateGarrisons) {
            remove(&mut self.garrisons, key as u32, key);
            dirty.insert(key);
        }
        let view = hour.table(TableId::StateGarrisons);
        for slot in forces::written(hour, TableId::StateGarrisons, &GARRISON) {
            let key = view.key(slot).ok_or_else(|| invalid(format!("StateGarrisons slot {slot} holds no row")))?;
            add(&mut self.garrisons, key as u32, key);
            dirty.insert(key);
        }
        if self.assigned_complete {
            let mut states = BTreeSet::new();
            for key in hour.deleted_keys(TableId::StateAssignedManpower) {
                remove(&mut self.assigned, assigned_state(key), key);
                states.insert(assigned_state(key));
            }
            let view = hour.table(TableId::StateAssignedManpower);
            for slot in forces::written(hour, TableId::StateAssignedManpower, &ASSIGNED) {
                let key = view.key(slot).ok_or_else(|| invalid(format!("StateAssignedManpower slot {slot} holds no row")))?;
                add(&mut self.assigned, assigned_state(key), key);
                states.insert(assigned_state(key));
            }
            for state in states {
                dirty.extend(self.garrisons.get(&state).into_iter().flatten().copied());
            }
        }
        Ok(dirty.into_iter().collect())
    }

    fn assigned(&self, hour: &Hour<'_>, state: u32, controller: u32, occupied: u32) -> io::Result<Option<i32>> {
        if !self.assigned_complete {
            return Ok(None);
        }
        let table = TableId::StateAssignedManpower;
        let view = hour.table(table);
        let mut total = 0_i32;
        for &key in self.assigned.get(&state).into_iter().flatten() {
            let slot = view.slot(key).ok_or_else(|| invalid(format!("StateAssignedManpower holds no row {key}")))?;
            let (Some(row_controller), Some(row_occupied)) = (self.owners.slot((key >> 64) as u32), self.owners.slot(number(&view, table, ASSIGNED[0], slot)? as u32)) else {
                return Ok(None);
            };
            if (row_controller, row_occupied) == (controller, occupied) {
                total = total.wrapping_add(number(&view, table, ASSIGNED[1], slot)? as i32);
            }
        }
        Ok(Some(total))
    }

    fn row(&self, hour: &Hour<'_>, key: u128, powers: (i32, i32)) -> io::Result<Option<Vec<Value>>> {
        let table = TableId::StateGarrisons;
        let view = hour.table(table);
        let Some(slot) = view.slot(key) else { return Ok(None) };
        let unknown = |reason: u8| Some(vec![Value::Unrecorded, Value::U8(reason)]);
        let known = |strength: i64| Some(vec![Value::I64(strength), Value::U8(GarrisonStrengthRow::KNOWN)]);
        let occupied = number(&view, table, GARRISON[0], slot)? as u32;
        let (Some(controller), Some(occupied)) = (self.owners.slot((key >> 32) as u32), self.owners.slot(occupied)) else {
            return Ok(unknown(GarrisonStrengthRow::UNRESOLVED));
        };
        if number(&view, table, GARRISON[2], slot)? != 0 {
            return Ok(known(0));
        }
        let Some(assigned) = self.assigned(hour, key as u32, controller, occupied)? else { return Ok(unknown(GarrisonStrengthRow::ASSIGNED)) };
        if assigned == 0 {
            return Ok(known(0));
        }
        let Some(bytes) = view.list(GARRISON[3], slot) else { return Ok(unknown(GarrisonStrengthRow::OPERANDS)) };
        let mut decoder = replay_data::codec::Decoder::new(bytes);
        let inputs = <GarrisonEquipmentInputs as replay_data::value::Value>::decode(&mut decoder)?;
        decoder.finish()?;
        if inputs.failure.is_some() {
            return Ok(unknown(GarrisonStrengthRow::FAILED));
        }
        Ok(known(displayed(number(&view, table, GARRISON[1], slot)? as i32, assigned, &inputs.rows, powers)))
    }
}

pub(crate) fn displayed(required_manpower: i32, assigned: i32, equipment: &[GarrisonEquipment], (manpower_power, equipment_power): (i32, i32)) -> i64 {
    let equipment = equipment_fraction(equipment);
    let manpower = if required_manpower > 0 {
        let denominator = i64::from(required_manpower).wrapping_mul(ONE);
        let numerator = i64::from(assigned).wrapping_mul(ONE);
        let fraction = if denominator == 0 || numerator <= -0x7fff_ffff_ffff_ffff { i64::from(u32::MAX) } else { numerator.wrapping_mul(ONE) / denominator };
        fraction.min(ONE)
    } else {
        ONE
    };
    ONE.wrapping_sub(shortage_power(ONE.wrapping_sub(equipment), equipment_power)).min(ONE.wrapping_sub(shortage_power(ONE.wrapping_sub(manpower), manpower_power)))
}

pub(crate) fn equipment_fraction(entries: &[GarrisonEquipment]) -> i64 {
    let (mut allocated, mut required) = (0_i64, 0_i64);
    for entry in entries {
        let effective = if entry.allocated_raw < 0 { 0 } else { entry.allocated_raw.min(entry.required_lookup_raw) };
        allocated = allocated.wrapping_add(entry.weight_raw.wrapping_mul(effective) / ONE);
        required = required.wrapping_add(entry.weight_raw.wrapping_mul(entry.required_lookup_raw) / ONE);
    }
    if required <= 0 {
        ONE
    } else if (allocated as u64).wrapping_add(0x7fff_ffff_ffff_fffe) > 0xffff_ffff_ffff_fffc {
        i64::from(u32::MAX)
    } else {
        allocated.wrapping_mul(ONE) / required
    }
}

pub(crate) fn shortage_power(shortage: i64, power: i32) -> i64 {
    let mut result = shortage;
    for _ in 1..power {
        let next = shortage.wrapping_mul(result) / ONE;
        if next == result {
            break;
        }
        result = next;
    }
    result
}

impl Writer for GarrisonStrength {
    fn name(&self) -> &'static str {
        "GarrisonStrength"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::GarrisonStrengths)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn definitions(&self) -> &'static [Uses] {
        DEFINITIONS
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows<'_>) -> io::Result<()> {
        let powers = powers(hour)?;
        let assigned_complete = hour.complete(TableId::StateAssignedManpower);
        let whole = hour.fresh || self.rebuild || forces::links_moved(hour) || self.powers != Some(powers) || self.assigned_complete != assigned_complete;
        self.powers = Some(powers);
        self.assigned_complete = assigned_complete;
        let keys = if whole {
            self.rebuild = false;
            self.read_all(hour)?
        } else {
            self.apply(hour)?
        };
        let mut rows = HashMap::with_capacity(keys.len());
        for &key in &keys {
            rows.insert(key, self.row(hour, key, powers)?);
        }
        forces::write(out, TableId::GarrisonStrengths, keys, whole, |key| rows.remove(&key).flatten());
        Ok(())
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if !blob.is_empty() {
            return Err(invalid("GarrisonStrength's state holds nothing"));
        }
        self.rebuild = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
