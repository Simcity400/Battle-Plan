
use recorder_derive_framework::{
    deployed::{Identity, invalid, row},
    framework::{
        Columns, Hour, Output, Read, Rows, Uses, Value, Watch, Writer, columns,
        definitions::{Define, Definition, Form},
    },
};
use replay_columnar::leaf::Writer as Bytes;
use replay_data::{
    military_aid,
    rows::{CountryLinkRow, CountryRow, DivisionLandInputsRow, DivisionRow, DivisionStatusRow, Recorded, TableId, TagHolderRow},
};
use std::{
    collections::{HashMap, HashSet},
    io,
};

#[cfg(test)]
mod tests;

const NONE: u32 = TagHolderRow::NONE;

const SLOT: [usize; 1] = columns::<CountryRow, 1>(["canonical_slot"]);
const LINK: [usize; 1] = columns::<CountryLinkRow, 1>(["canonical_slot"]);
const OWNERS: [usize; 2] = columns::<DivisionRow, 2>(["roster_owner", "expeditionary_owner"]);
const TRANSFER: [usize; 1] = columns::<DivisionLandInputsRow, 1>(["naval_transfer"]);
const SUPPLY: [usize; 5] = columns::<DivisionStatusRow, 5>([
    "supply_ratio",
    "supply_grace_maximum",
    "supply_grace_remaining",
    "supply_ratio_exact",
    "supply_grace_exact",
]);

const fn read(table: TableId, columns: &'static [usize]) -> Read {
    Read { table, columns: Columns::These(columns), watch: Watch::Changed, required: true }
}

const READS: &[Read] = &[
    read(TableId::Countries, &SLOT),
    read(TableId::CountryLinks, &LINK),
    read(TableId::Divisions, &OWNERS),
    read(TableId::DivisionLandInputs, &TRANSFER),
    read(TableId::DivisionStatus, &SUPPLY),
];

const GRACE: Define = Define { section: "NMilitary", name: "SUPPLY_GRACE", form: Form::Int32 };
const LOW: Define = Define { section: "NMilitary", name: "LOW_SUPPLY", form: Form::Fixed5 };
const VERY_LOW_ICON: Define = Define { section: "NGraphics", name: "SUPPLY_UNIT_COUNTER_VERY_LOW_THRESHOLD", form: Form::Fixed5 };
const LOW_ICON: Define = Define { section: "NGraphics", name: "SUPPLY_UNIT_COUNTER_LOW_THRESHOLD", form: Form::Fixed5 };

const DEFINITIONS: &[Uses] = &[
    Uses { definition: Definition::Define(GRACE), required: true },
    Uses { definition: Definition::Define(LOW), required: true },
    Uses { definition: Definition::Define(VERY_LOW_ICON), required: true },
    Uses { definition: Definition::Define(LOW_ICON), required: true },
];

const NO_SUPPLY: u8 = 0;
const STRAINED: u8 = 1;
const IN_SUPPLY: u8 = 2;
const UNKNOWN: u8 = 3;

const ONE: i64 = 100_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Thresholds {
    grace: i64,
    low: i64,
    very_low_icon: i64,
    low_icon: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Operands {
    received: [i64; 2],
    maximum_grace: i64,
    remaining_grace: [i64; 2],
    naval_transfer: bool,
}

impl Thresholds {
    fn read(hour: &Hour<'_>) -> io::Result<Self> {
        let definitions = hour.definitions();
        let value = |define: Define| definitions.define(define.section, define.name).map_err(|why| invalid(format!("SupplyHealth holds no {}.{}: {why}", define.section, define.name)));
        Ok(Self { grace: value(GRACE)?, low: value(LOW)?, very_low_icon: value(VERY_LOW_ICON)?, low_icon: value(LOW_ICON)? })
    }

    fn state_at(self, low: bool, received: i64) -> u8 {
        let shown = if low { received } else { ONE };
        if shown < self.very_low_icon {
            NO_SUPPLY
        } else if shown < self.low_icon || low {
            STRAINED
        } else {
            IN_SUPPLY
        }
    }

    fn state(self, division: &Operands) -> u8 {
        if division.naval_transfer {
            return self.state_at(false, ONE);
        }
        let [eff_low, eff_high] = DivisionStatusRow::effective_supply_bounds(
            division.received,
            division.maximum_grace,
            division.remaining_grace,
            self.grace as i32,
        );
        let mut decided: Option<u8> = None;
        let mut observe = |candidate: u8| -> bool {
            match decided {
                None => {
                    decided = Some(candidate);
                    true
                }
                Some(existing) => existing == candidate,
            }
        };
        if eff_high >= self.low && !observe(self.state_at(false, ONE)) {
            return UNKNOWN;
        }
        if eff_low < self.low {
            let [r_low, r_high] = division.received;
            let low_r_high = if division.maximum_grace <= 0 || self.low > ONE {
                r_high
            } else {
                r_high.min(self.low.saturating_sub(1))
            };
            if r_low <= low_r_high {
                if !observe(self.state_at(true, r_low)) {
                    return UNKNOWN;
                }
                if r_low < self.very_low_icon
                    && self.very_low_icon <= low_r_high
                    && !observe(self.state_at(true, self.very_low_icon))
                {
                    return UNKNOWN;
                }
            }
        }
        decided.unwrap_or(UNKNOWN)
    }
}

fn owner(hour: &Hour<'_>, id: u64) -> io::Result<Option<u32>> {
    let view = hour.table(TableId::Divisions);
    let Some(at) = view.slot(u128::from(id)) else { return Ok(None) };
    let division: DivisionRow = row(&view, at, &OWNERS)?;
    Ok(Some(military_aid::owner(&division)))
}

fn transfer(hour: &Hour<'_>, id: u64) -> Option<bool> {
    let view = hour.table(TableId::DivisionLandInputs);
    view.bool(TRANSFER[0], view.slot(u128::from(id))?)
}

fn supply(hour: &Hour<'_>, id: u64, grace: i32) -> io::Result<Option<([i64; 2], i64, [i64; 2])>> {
    let view = hour.table(TableId::DivisionStatus);
    let Some(at) = view.slot(u128::from(id)) else { return Ok(None) };
    let status: DivisionStatusRow = row(&view, at, &SUPPLY)?;
    Ok(match (status.recorded_supply_ratio_bounds(), status.supply_grace_maximum, status.recorded_supply_grace_bounds(grace)) {
        (Some(received), Recorded::Value(maximum), Some(remaining)) => Some((received, maximum, remaining)),
        _ => None,
    })
}

fn touched(hour: &Hour<'_>, table: TableId, columns: &'static [usize], keys: &mut Vec<u64>) {
    let view = hour.table(table);
    keys.extend(hour.deleted_keys(table).map(|key| key as u64));
    keys.extend(hour.written_rows(table, Columns::These(columns)).into_iter().filter_map(|slot| view.key(slot)).map(|key| key as u64));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Division {
    slot: u32,
    state: u8,
}

#[derive(Default)]
pub struct SupplyHealth {
    rebuild: bool,
    identity: Identity,
    divisions: HashMap<u64, Division>,
    counts: HashMap<u32, [i64; 4]>,
    unresolved: i64,
    held: HashSet<u32>,
}

impl SupplyHealth {
    fn division(&self, hour: &Hour<'_>, thresholds: Thresholds, id: u64) -> io::Result<Option<Division>> {
        let Some(raw) = owner(hour, id)? else { return Ok(None) };
        let slot = self.identity.resolve(raw).unwrap_or(NONE);
        Self::placed(hour, thresholds, id, slot).map(Some)
    }

    fn placed(hour: &Hour<'_>, thresholds: Thresholds, id: u64, slot: u32) -> io::Result<Division> {
        if slot == NONE {
            return Ok(Division { slot, state: UNKNOWN });
        }
        let state = match (transfer(hour, id), supply(hour, id, thresholds.grace as i32)?) {
            (Some(naval_transfer), Some((received, maximum_grace, remaining_grace))) => thresholds.state(&Operands { received, maximum_grace, remaining_grace, naval_transfer }),
            _ => UNKNOWN,
        };
        Ok(Division { slot, state })
    }

    fn count(&mut self, division: Division, sign: i64, dirty: &mut HashSet<u32>) {
        if division.slot == NONE {
            self.unresolved += sign;
            return;
        }
        let counts = self.counts.entry(division.slot).or_default();
        counts[usize::from(division.state)] += sign;
        if *counts == [0; 4] {
            self.counts.remove(&division.slot);
        }
        dirty.insert(division.slot);
    }

    fn place(&mut self, id: u64, next: Option<Division>, dirty: &mut HashSet<u32>) {
        let old = self.divisions.get(&id).copied();
        if old == next {
            return;
        }
        if let Some(old) = old {
            self.count(old, -1, dirty);
        }
        match next {
            Some(next) => {
                self.count(next, 1, dirty);
                self.divisions.insert(id, next);
            }
            None => {
                self.divisions.remove(&id);
            }
        }
    }

    fn rebuild(&mut self, hour: &Hour<'_>, out: &Rows<'_>, thresholds: Thresholds) -> io::Result<()> {
        let table = out.table(TableId::SupplyHealth);
        *self = Self { identity: Identity::read(hour), held: table.rows().filter_map(|slot| table.key(slot)).map(|key| key as u32).collect(), ..Self::default() };
        let divisions = hour.table(TableId::Divisions);
        self.divisions.reserve(divisions.live().count());
        let mut ignored = HashSet::new();
        for slot in divisions.rows() {
            let Some(key) = divisions.key(slot) else { continue };
            let next = self.division(hour, thresholds, key as u64)?;
            self.place(key as u64, next, &mut ignored);
        }
        Ok(())
    }

    fn apply(&mut self, hour: &Hour<'_>, thresholds: Thresholds, dirty: &mut HashSet<u32>) -> io::Result<()> {
        let mut owned = Vec::new();
        touched(hour, TableId::Divisions, &OWNERS, &mut owned);
        owned.sort_unstable();
        owned.dedup();
        let mut ids = owned.clone();
        touched(hour, TableId::DivisionLandInputs, &TRANSFER, &mut ids);
        touched(hour, TableId::DivisionStatus, &SUPPLY, &mut ids);
        ids.sort_unstable();
        ids.dedup();
        for id in ids {
            let next = if owned.binary_search(&id).is_ok() {
                self.division(hour, thresholds, id)?
            } else {
                match self.divisions.get(&id) {
                    Some(held) => Some(Self::placed(hour, thresholds, id, held.slot)?),
                    None => continue,
                }
            };
            self.place(id, next, dirty);
        }
        Ok(())
    }

    fn cells(&self, slot: u32) -> Vec<Value> {
        let counts = self.counts.get(&slot).copied().unwrap_or_default();
        if self.unresolved != 0 || counts[usize::from(UNKNOWN)] != 0 {
            return vec![Value::Unrecorded; 3];
        }
        counts[..3].iter().map(|&count| Value::I64(count)).collect()
    }

    fn write(&mut self, dirty: HashSet<u32>, out: &mut Rows) {
        for slot in dirty {
            if self.identity.countries.contains(&slot) {
                out.set(TableId::SupplyHealth, u128::from(slot), self.cells(slot));
                self.held.insert(slot);
            } else if self.held.remove(&slot) {
                out.delete(TableId::SupplyHealth, u128::from(slot));
            }
        }
    }
}

impl Writer for SupplyHealth {
    fn name(&self) -> &'static str {
        "SupplyHealth"
    }

    fn version(&self) -> u32 {
        2
    }

    fn accepts_version(&self, version: u32) -> bool {
        matches!(version, 1 | 2)
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::SupplyHealth)]
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

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let thresholds = Thresholds::read(hour)?;
        let mut dirty = HashSet::new();
        if hour.fresh || self.rebuild || Identity::changed(hour) {
            self.rebuild(hour, out, thresholds)?;
            dirty.extend(self.identity.countries.iter().copied());
            dirty.extend(self.held.iter().copied());
        } else {
            let unresolved = self.unresolved != 0;
            self.apply(hour, thresholds, &mut dirty)?;
            if unresolved != (self.unresolved != 0) {
                dirty.extend(self.identity.countries.iter().copied());
            }
        }
        dirty.remove(&NONE);
        self.write(dirty, out);
        Ok(())
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if !blob.is_empty() {
            return Err(invalid("SupplyHealth's state holds nothing"));
        }
        self.rebuild = true;
        Ok(())
    }
}
