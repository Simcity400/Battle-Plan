
use recorder_derive_framework::{
    forces::{self, Owners, exact, invalid, number, written},
    framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns},
};
use replay_columnar::{
    leaf::{Reader, Writer as Bytes},
    store::TableView,
};
use replay_data::rows::{AirForceWingSignatureRow, AirWingRow, TableId, WingEquipmentRow};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    sync::Arc,
};

const WING_COLUMNS: [usize; 3] = columns::<AirWingRow, 3>(["owner", "country_override", "planes"]);
const EQUIPMENT_COLUMNS: [usize; 2] = columns::<WingEquipmentRow, 2>(["variant", "amount_raw"]);

const SCALE: i64 = 100_000;

const SIDES: usize = 2;

const WINGS: Read = Read { table: TableId::AirWings, columns: Columns::These(&WING_COLUMNS), watch: Watch::Changed, required: true };
const COUNTRIES: Read = Read { table: TableId::Countries, columns: Columns::These(&[0]), watch: Watch::Changed, required: true };
const LINKS: Read = Read { table: TableId::CountryLinks, columns: Columns::These(&[0]), watch: Watch::Changed, required: true };
const EQUIPMENT: Read = Read { table: TableId::WingEquipment, columns: Columns::These(&EQUIPMENT_COLUMNS), watch: Watch::Changed, required: true };

fn owners(owner: u32, country_override: i32) -> [u32; SIDES] {
    [owner, if country_override > 0 { country_override as u32 } else { owner }]
}

fn wing(view: &TableView<'_>, slot: u32) -> io::Result<(u64, [u32; SIDES], Option<i64>)> {
    let table = TableId::AirWings;
    let id = view.key(slot).ok_or_else(|| invalid(format!("AirWings slot {slot} holds no row")))? as u64;
    let owner = number(view, table, WING_COLUMNS[0], slot)? as u32;
    let country_override = number(view, table, WING_COLUMNS[1], slot)? as i32;
    let planes = number(view, table, WING_COLUMNS[2], slot)?;
    Ok((id, owners(owner, country_override), (planes >= 0).then_some(planes)))
}

fn known(unresolved: [i64; SIDES]) -> [bool; SIDES] {
    unresolved.map(|count| count == 0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Wing {
    raw: [u32; SIDES],
    planes: Option<i64>,
}

#[derive(Default)]
struct Total {
    wings: i64,
    planes: i128,
    bad_planes: i64,
}

#[derive(Default)]
pub struct AirForce {
    rebuild: bool,
    owners: Owners,
    wings: HashMap<u64, Wing>,
    totals: HashMap<(u32, usize), Total>,
    unresolved: [i64; SIDES],
    dirty: HashSet<u32>,
}

impl AirForce {
    fn count(&mut self, wing: &Wing, sign: i64) {
        for (at, &raw) in wing.raw.iter().enumerate() {
            let Some(slot) = self.owners.slot(raw) else {
                self.unresolved[at] += sign;
                continue;
            };
            self.dirty.insert(slot);
            let total = self.totals.entry((slot, at)).or_default();
            total.wings += sign;
            match wing.planes {
                Some(planes) => total.planes += i128::from(sign) * i128::from(planes),
                None => total.bad_planes += sign,
            }
            if total.wings == 0 {
                self.totals.remove(&(slot, at));
            }
        }
    }

    fn rebuild(&mut self, hour: &Hour<'_>) -> io::Result<()> {
        self.owners.read_links(hour);
        self.owners.read_countries(hour);
        self.wings.clear();
        self.totals.clear();
        self.unresolved = [0; SIDES];
        let view = hour.table(TableId::AirWings);
        for slot in view.rows() {
            let (id, raw, planes) = wing(&view, slot)?;
            let wing = Wing { raw, planes };
            self.count(&wing, 1);
            self.wings.insert(id, wing);
        }
        Ok(())
    }

    fn apply(&mut self, hour: &Hour<'_>) -> io::Result<bool> {
        if forces::links_moved(hour) || forces::countries_moved(hour) {
            return Ok(false);
        }
        for key in hour.deleted_keys(TableId::AirWings) {
            if let Some(old) = self.wings.remove(&(key as u64)) {
                self.count(&old, -1);
            }
        }
        let view = hour.table(TableId::AirWings);
        for slot in written(hour, TableId::AirWings, &WING_COLUMNS) {
            let (id, raw, planes) = wing(&view, slot)?;
            let new = Wing { raw, planes };
            match self.wings.insert(id, new) {
                Some(old) if old == new => {}
                old => {
                    if let Some(old) = old {
                        self.count(&old, -1);
                    }
                    self.count(&new, 1);
                }
            }
        }
        Ok(true)
    }

    fn row(&self, slot: u32) -> Option<Vec<Value>> {
        let held = [self.totals.get(&(slot, 0)), self.totals.get(&(slot, 1))];
        if !self.owners.countries().contains(&slot) && held.iter().all(Option::is_none) {
            return None;
        }
        let mut cells = vec![Value::Unrecorded; 2 * SIDES];
        for (at, known) in known(self.unresolved).into_iter().enumerate() {
            if !known {
                continue;
            }
            let total = held[at];
            if total.is_none_or(|total| total.bad_planes == 0) {
                cells[2 * at] = exact(total.map_or(0, |total| total.planes));
            }
            cells[2 * at + 1] = Value::I64(total.map_or(0, |total| total.wings));
        }
        Some(cells)
    }
}

impl Writer for AirForce {
    fn name(&self) -> &'static str {
        "AirForce"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::AirForceTotals)]
    }

    fn reads(&self) -> &'static [Read] {
        &[WINGS, COUNTRIES, LINKS]
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let before = known(self.unresolved);
        let applied = !hour.fresh && !self.rebuild && self.apply(hour)?;
        if !applied {
            self.rebuild(hour)?;
            self.rebuild = false;
        }
        let all = !applied || before != known(self.unresolved);
        let dirty = std::mem::take(&mut self.dirty);
        let slots: Vec<u32> = match all {
            true => self.owners.countries().iter().copied().chain(self.totals.keys().map(|&(slot, _)| slot)).collect(),
            false => dirty.into_iter().collect(),
        };
        forces::write(out, TableId::AirForceTotals, slots.into_iter().map(u128::from), all, |key| self.row(key as u32));
        Ok(())
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if !blob.is_empty() {
            return Err(invalid("AirForce's state holds nothing"));
        }
        self.rebuild = true;
        Ok(())
    }
}

type Part = Option<u64>;

type Shape = Arc<[u64]>;

fn split(planes: Option<i64>, entries: &[(u64, i64)]) -> Vec<(Part, Option<i64>)> {
    let mut parts: BTreeMap<Part, Option<i64>> = BTreeMap::new();
    let Some(planes) = planes else {
        parts.extend(entries.iter().map(|&(variant, _)| (Some(variant), None)));
        parts.insert(None, None);
        return parts.into_iter().collect();
    };
    let summed: i128 = entries.iter().map(|&(_, amount)| i128::from(amount)).sum();
    let splittable = entries.iter().all(|&(_, amount)| amount >= 0 && amount % SCALE == 0) && summed <= i128::from(planes) * i128::from(SCALE);
    if !splittable {
        parts.extend(entries.iter().map(|&(variant, _)| (Some(variant), None)));
        parts.insert(None, Some(planes));
        return parts.into_iter().collect();
    }
    let mut identified = 0;
    for &(variant, amount) in entries {
        let whole = amount / SCALE;
        identified += whole;
        if let Some(held) = parts.entry(Some(variant)).or_insert(Some(0)) {
            *held += whole;
        }
    }
    if planes != identified {
        parts.insert(None, Some(planes - identified));
    }
    parts.into_iter().collect()
}

fn signature(entries: &[(u64, i64)]) -> Vec<u64> {
    let mut variants: Vec<u64> = entries.iter().filter(|(_, amount)| *amount > 0).map(|&(variant, _)| variant).collect();
    variants.sort_unstable();
    variants.dedup();
    variants
}

fn part_key(slot: u32, side: usize, part: Part) -> u128 {
    (u128::from(slot) << 73) | ((side as u128) << 65) | part.map_or(0, |variant| (1 << 64) | u128::from(variant))
}

fn part_of(key: u128) -> (u32, usize, Part) {
    ((key >> 73) as u32, ((key >> 65) & 0xff) as usize, ((key >> 64) & 1 == 1).then_some(key as u64))
}

fn kind_key(slot: u32, side: usize, number: u32) -> u128 {
    (u128::from(slot) << 40) | ((side as u128) << 32) | u128::from(number)
}

fn kind_of(key: u128) -> (u32, usize, u32) {
    ((key >> 40) as u32, ((key >> 32) & 0xff) as usize, key as u32)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Bank {
    raw: [u32; SIDES],
    parts: Vec<(Part, Option<i64>)>,
    shape: Shape,
}

#[derive(Clone, Copy, Default)]
struct PartSum {
    entries: i64,
    sum: i128,
    unknown: i64,
}

#[derive(Default)]
struct Mix {
    parts: HashMap<Part, PartSum>,
    kinds: HashMap<Shape, i64>,
}

#[derive(Default)]
pub struct AirForceComposition {
    rebuild: bool,
    owners: Owners,
    wings: HashMap<u64, Bank>,
    entries: HashMap<u64, Vec<u32>>,
    mixes: HashMap<(u32, usize), Mix>,
    unresolved: [i64; SIDES],
    held: HashMap<Shape, u32>,
    released: Vec<Shape>,
    numbers: HashMap<Shape, u32>,
    named: HashMap<u32, Shape>,
    next: u32,
    dirty_parts: HashSet<(u32, usize, Part)>,
    dirty_kinds: HashSet<(u32, usize, Shape)>,
}

impl AirForceComposition {
    fn shape(&self, variants: Vec<u64>) -> Shape {
        if let Some((held, _)) = self.held.get_key_value(variants.as_slice()) {
            return Arc::clone(held);
        }
        if let Some((named, _)) = self.numbers.get_key_value(variants.as_slice()) {
            return Arc::clone(named);
        }
        variants.into()
    }

    fn hold(&mut self, shape: &Shape, sign: i64) {
        if sign > 0 {
            *self.held.entry(Arc::clone(shape)).or_default() += 1;
            return;
        }
        let Some(wings) = self.held.get_mut(shape) else { return };
        *wings -= 1;
        if *wings == 0 {
            self.held.remove(shape);
            self.released.push(Arc::clone(shape));
        }
    }

    fn count(&mut self, bank: &Bank, sign: i64) {
        self.hold(&bank.shape, sign);
        for (at, &raw) in bank.raw.iter().enumerate() {
            let Some(slot) = self.owners.slot(raw) else {
                self.unresolved[at] += sign;
                continue;
            };
            let mix = self.mixes.entry((slot, at)).or_default();
            for &(part, amount) in &bank.parts {
                let held = mix.parts.entry(part).or_default();
                held.entries += sign;
                match amount {
                    Some(amount) => held.sum += i128::from(sign) * i128::from(amount),
                    None => held.unknown += sign,
                }
                if held.entries == 0 {
                    mix.parts.remove(&part);
                }
                self.dirty_parts.insert((slot, at, part));
            }
            let wings = mix.kinds.entry(Arc::clone(&bank.shape)).or_default();
            *wings += sign;
            if *wings == 0 {
                mix.kinds.remove(&bank.shape);
            }
            self.dirty_kinds.insert((slot, at, Arc::clone(&bank.shape)));
            if mix.kinds.is_empty() {
                self.mixes.remove(&(slot, at));
            }
        }
    }

    fn bank(&mut self, hour: &Hour<'_>, wings: &TableView<'_>, slot: u32) -> io::Result<(u64, Bank)> {
        let (id, raw, planes) = wing(wings, slot)?;
        let table = TableId::WingEquipment;
        let equipment = hour.table(table);
        let mut entries = Vec::new();
        for &ordinal in self.entries.get(&id).into_iter().flatten() {
            let key = (u128::from(id) << 32) | u128::from(ordinal);
            let slot = equipment.slot(key).ok_or_else(|| invalid(format!("WingEquipment holds no row {key}")))?;
            entries.push((number(&equipment, table, EQUIPMENT_COLUMNS[0], slot)? as u64, number(&equipment, table, EQUIPMENT_COLUMNS[1], slot)?));
        }
        let shape = self.shape(signature(&entries));
        Ok((id, Bank { raw, parts: split(planes, &entries), shape }))
    }

    fn replace(&mut self, id: u64, bank: Bank) {
        if self.wings.get(&id) == Some(&bank) {
            return;
        }
        if let Some(old) = self.wings.remove(&id) {
            self.count(&old, -1);
        }
        self.count(&bank, 1);
        self.wings.insert(id, bank);
    }

    fn read_numbers(&mut self, out: &Rows<'_>) -> io::Result<()> {
        self.numbers.clear();
        self.named.clear();
        let view = out.table(TableId::AirForceWingSignatures);
        for slot in view.rows() {
            let number = view.key(slot).ok_or_else(|| invalid("A wing signature slot holds no row"))? as u32;
            let text = view.text(0, slot).and_then(|id| view.string(id)).ok_or_else(|| invalid("A wing signature is not recorded"))?;
            let variants = AirForceWingSignatureRow { number, variants: text.to_vec() }.variants()?;
            let shape = self.shape(variants);
            self.numbers.insert(Arc::clone(&shape), number);
            self.named.insert(number, shape);
            self.next = self.next.max(number + 1);
        }
        Ok(())
    }

    fn rebuild(&mut self, hour: &Hour<'_>, out: &Rows<'_>) -> io::Result<()> {
        if hour.fresh || self.rebuild {
            self.read_numbers(out)?;
        }
        self.owners.read_links(hour);
        self.wings.clear();
        self.entries.clear();
        self.mixes.clear();
        self.held.clear();
        self.released.clear();
        self.unresolved = [0; SIDES];
        let equipment = hour.table(TableId::WingEquipment);
        for key in equipment.rows().filter_map(|slot| equipment.key(slot)) {
            self.entries.entry((key >> 32) as u64).or_default().push(key as u32);
        }
        let wings = hour.table(TableId::AirWings);
        for slot in wings.rows() {
            let (id, bank) = self.bank(hour, &wings, slot)?;
            self.replace(id, bank);
        }
        Ok(())
    }

    fn apply(&mut self, hour: &Hour<'_>) -> io::Result<bool> {
        if forces::links_moved(hour) {
            return Ok(false);
        }
        let mut changed: HashSet<u64> = HashSet::new();
        for key in hour.deleted_keys(TableId::WingEquipment) {
            let (id, ordinal) = ((key >> 32) as u64, key as u32);
            if let Some(ordinals) = self.entries.get_mut(&id) {
                ordinals.retain(|&held| held != ordinal);
                if ordinals.is_empty() {
                    self.entries.remove(&id);
                }
            }
            changed.insert(id);
        }
        let equipment = hour.table(TableId::WingEquipment);
        for &slot in &hour.changes(TableId::WingEquipment).inserted {
            let key = equipment.key(slot).ok_or_else(|| invalid(format!("WingEquipment slot {slot} holds no row")))?;
            self.entries.entry((key >> 32) as u64).or_default().push(key as u32);
        }
        changed.extend(written(hour, TableId::WingEquipment, &EQUIPMENT_COLUMNS).into_iter().filter_map(|slot| equipment.key(slot)).map(|key| (key >> 32) as u64));
        for key in hour.deleted_keys(TableId::AirWings) {
            if let Some(old) = self.wings.remove(&(key as u64)) {
                self.count(&old, -1);
            }
        }
        let wings = hour.table(TableId::AirWings);
        changed.extend(written(hour, TableId::AirWings, &WING_COLUMNS).into_iter().filter_map(|slot| wings.key(slot)).map(|key| key as u64));
        let mut changed: Vec<u64> = changed.into_iter().collect();
        changed.sort_unstable();
        for id in changed {
            let Some(slot) = wings.slot(u128::from(id)) else { continue };
            let (id, bank) = self.bank(hour, &wings, slot)?;
            self.replace(id, bank);
        }
        Ok(true)
    }

    fn number_shapes(&mut self, shapes: Vec<Shape>, out: &mut Rows) {
        let mut fresh: Vec<Shape> = shapes.into_iter().filter(|shape| !self.numbers.contains_key(shape)).collect();
        fresh.sort_unstable();
        fresh.dedup();
        for shape in fresh {
            let number = self.next;
            self.next += 1;
            let text = out.text(&AirForceWingSignatureRow::encode(&shape));
            out.set(TableId::AirForceWingSignatures, u128::from(number), vec![text]);
            self.numbers.insert(Arc::clone(&shape), number);
            self.named.insert(number, shape);
        }
    }

    fn forget_shapes(&mut self, out: &mut Rows<'_>, all: bool) {
        let released = std::mem::take(&mut self.released);
        let mut gone: Vec<u32> = match all {
            true => self.numbers.iter().filter(|(shape, _)| !self.held.contains_key(*shape)).map(|(_, &number)| number).collect(),
            false => released.iter().filter(|shape| !self.held.contains_key(*shape)).filter_map(|shape| self.numbers.get(shape).copied()).collect(),
        };
        gone.sort_unstable();
        gone.dedup();
        for number in gone {
            if let Some(shape) = self.named.remove(&number) {
                self.numbers.remove(&shape);
            }
            if out.table(TableId::AirForceWingSignatures).slot(u128::from(number)).is_some() {
                out.delete(TableId::AirForceWingSignatures, u128::from(number));
            }
        }
    }

    fn part(&self, key: u128) -> Option<Vec<Value>> {
        let (slot, at, part) = part_of(key);
        if self.unresolved.get(at).is_none_or(|&count| count != 0) {
            return None;
        }
        let held = self.mixes.get(&(slot, at))?.parts.get(&part)?;
        Some(vec![if held.unknown == 0 { exact(held.sum) } else { Value::Unrecorded }])
    }

    fn kind(&self, key: u128) -> Option<Vec<Value>> {
        let (slot, at, number) = kind_of(key);
        if self.unresolved.get(at).is_none_or(|&count| count != 0) {
            return None;
        }
        let shape = self.named.get(&number)?;
        let wings = *self.mixes.get(&(slot, at))?.kinds.get(shape)?;
        Some(vec![Value::I64(wings)])
    }
}

impl Writer for AirForceComposition {
    fn name(&self) -> &'static str {
        "AirForceComposition"
    }

    fn version(&self) -> u32 {
        2
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::AirForceParts), Output::State(TableId::AirForceWingKinds), Output::State(TableId::AirForceWingSignatures)]
    }

    fn reads(&self) -> &'static [Read] {
        &[WINGS, EQUIPMENT, LINKS]
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let before = known(self.unresolved);
        let applied = !hour.fresh && !self.rebuild && self.apply(hour)?;
        if !applied {
            self.rebuild(hour, out)?;
            self.rebuild = false;
        }
        let known = known(self.unresolved);
        let all = !applied || before != known;
        let dirty_parts = std::mem::take(&mut self.dirty_parts);
        let dirty_kinds = std::mem::take(&mut self.dirty_kinds);
        let (parts, kinds): (Vec<u128>, Vec<(u32, usize, Shape)>) = match all {
            true => (
                self.mixes.iter().flat_map(|(&(slot, at), mix)| mix.parts.keys().map(move |&part| part_key(slot, at, part))).collect(),
                self.mixes.iter().flat_map(|(&(slot, at), mix)| mix.kinds.keys().map(move |shape| (slot, at, Arc::clone(shape)))).collect(),
            ),
            false => (dirty_parts.into_iter().map(|(slot, at, part)| part_key(slot, at, part)).collect(), dirty_kinds.into_iter().collect()),
        };
        let shown = kinds
            .iter()
            .filter(|(slot, at, shape)| known[*at] && self.mixes.get(&(*slot, *at)).is_some_and(|mix| mix.kinds.contains_key(shape)))
            .map(|(_, _, shape)| Arc::clone(shape))
            .collect();
        self.number_shapes(shown, out);
        let kinds: Vec<u128> = kinds.into_iter().filter_map(|(slot, at, shape)| self.numbers.get(&shape).map(|&number| kind_key(slot, at, number))).collect();
        forces::write(out, TableId::AirForceParts, parts, all, |key| self.part(key));
        forces::write(out, TableId::AirForceWingKinds, kinds, all, |key| self.kind(key));
        self.forget_shapes(out, all);
        Ok(())
    }

    fn save(&self, blob: &mut Bytes) -> io::Result<()> {
        blob.varint(u64::from(self.next))
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        *self = Self::default();
        let mut reader = Reader::new(blob);
        self.next = reader.varint_u32()?;
        reader.finish()?;
        self.rebuild = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
