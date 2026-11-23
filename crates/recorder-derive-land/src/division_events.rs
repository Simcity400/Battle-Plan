
mod inputs;
#[cfg(test)]
mod tests;

use recorder_derive_framework::framework::{Columns, Hour, Keys, Output, Read, Rows, Value, Watch, Writer};
use inputs::{Icon, Names, Placed, Signature, invalid};
use replay_columnar::{
    leaf::{Reader, Writer as Bytes},
    manifest::Status,
    store::Kept,
};
use replay_data::rows::{DivisionDeletionDisposition as Disposition, DivisionEquipmentDeletionRow, DivisionEventRow, EquipmentDestructionCause as Cause, TableId, TagHolderRow};
use std::{
    collections::{HashMap, HashSet},
    io,
};

const READS: &[Read] = &[
    Read { table: TableId::Divisions, columns: Columns::These(&inputs::SHOWN), watch: Watch::Deleted, required: true },
    Read { table: TableId::DivisionEquipmentDeletions, columns: Columns::Every, watch: Watch::Changed, required: true },
    Read { table: TableId::DivisionStatus, columns: Columns::These(&inputs::NAMES), watch: Watch::Deleted, required: false },
    Read { table: TableId::DivisionTemplateIcons, columns: Columns::These(&inputs::ICON), watch: Watch::Deleted, required: false },
    Read { table: TableId::DivisionTemplates, columns: Columns::Every, watch: Watch::Changed, required: false },
    Read { table: TableId::DestroyedDivisions, columns: Columns::Every, watch: Watch::Appended, required: false },
    Read { table: TableId::LandEventCoverage, columns: Columns::Every, watch: Watch::Changed, required: false },
    Read { table: TableId::CountryLinks, columns: Columns::These(&inputs::LINK), watch: Watch::Changed, required: false },
];

#[derive(Default)]
pub struct DivisionLog {
    seen: HashSet<u64>,
    witnesses: HashMap<u64, Signature>,
    census: Option<i32>,
    names: Option<i32>,
    icons: Option<i32>,
    deaths: Vec<(u64, Death)>,
}

struct Copied<'a> {
    placed: Option<(Placed, Option<i32>)>,
    names: Option<(Names<'a>, Option<i32>)>,
    icon: Option<(Icon<'a>, Option<i32>)>,
    template: Option<inputs::TemplateName>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Death {
    of: u64,
    hour: i32,
    province: u32,
    killer: u32,
    division: Option<u64>,
}

#[derive(Default)]
struct Dating {
    death: Option<Death>,
    matched: Option<u32>,
}

fn optional<T>(value: Option<T>, cell: impl Fn(T) -> Value) -> Value {
    value.map_or(Value::Unrecorded, cell)
}

fn opaque(out: &mut Rows, bytes: Option<&[u8]>) -> Value {
    match bytes {
        Some(bytes) => out.opaque(bytes.to_vec()),
        None => Value::Unrecorded,
    }
}

struct Dropped<'a> {
    divisions: HashMap<u128, &'a [Kept]>,
    names: HashMap<u128, &'a [Kept]>,
    icons: HashMap<u128, &'a [Kept]>,
}

impl<'a> Dropped<'a> {
    fn of(hour: &Hour<'a>) -> Self {
        let by_key = |table: TableId| -> HashMap<u128, &'a [Kept]> { hour.dropped(table).iter().map(|(key, cells)| (*key, &cells[..])).collect() };
        Self { divisions: by_key(TableId::Divisions), names: by_key(TableId::DivisionStatus), icons: by_key(TableId::DivisionTemplateIcons) }
    }
}

impl DivisionLog {
    fn copied<'a>(&self, hour: &Hour<'a>, division: u64, dropped: &Dropped<'a>) -> Copied<'a> {
        let key = u128::from(division);
        let earlier = |at: Option<i32>| at.filter(|_| !hour.fresh);
        let census = hour.table(TableId::Divisions);
        let placed = match dropped.divisions.get(&key) {
            Some(cells) => Some((inputs::dropped_division(cells), earlier(self.census))),
            None => census.slot(key).map(|slot| (inputs::division(&census, slot), Some(hour.hour))),
        };
        let names = match dropped.names.get(&key) {
            Some(cells) => Some((inputs::dropped_names(*cells), earlier(self.names))),
            None => inputs::names(hour, division).map(|names| (names, hour.table(TableId::DivisionStatus).sampled_at())),
        };
        let icon = placed.and_then(|(placed, _)| placed.template).and_then(|template| match dropped.icons.get(&u128::from(template)) {
            Some(cells) => Some((inputs::dropped_icon(hour, cells), earlier(self.icons))),
            None => inputs::icon(hour, template).map(|icon| (icon, hour.table(TableId::DivisionTemplateIcons).sampled_at())),
        });
        let template = placed.and_then(|(placed, _)| placed.template).and_then(|template| inputs::template_name(hour, template));
        Copied { placed, names, icon, template }
    }
}

fn cells(hour: &Hour<'_>, out: &mut Rows, witness: Option<&DivisionEquipmentDeletionRow>, owner: Option<u32>, after: Option<i32>, copied: &Copied<'_>, dating: &Dating) -> Vec<Value> {
    let at = hour.hour;
    let death = dating.death.as_ref();
    let (disposition, cause) = match witness {
        Some(witness) => (Some(witness.disposition as u32), witness.cause.map(|cause| cause as u32)),
        None => (None, None),
    };
    let slot = owner.map(|owner| inputs::resolve(hour, owner).unwrap_or(TagHolderRow::NONE));
    let (province, place_hour) = match (death, copied.placed) {
        (Some(death), _) => (Value::U32(death.province), Value::I32(death.hour)),
        (None, Some((placed, supplied))) => (optional(placed.province, Value::U32), optional(supplied.filter(|_| placed.province.is_some()), Value::I32)),
        (None, None) => (Value::Unrecorded, Value::Unrecorded),
    };
    let (template, template_hour) = match copied.placed {
        Some((placed, supplied)) => (optional(placed.template, Value::U64), optional(supplied.filter(|_| placed.template.is_some()), Value::I32)),
        None => (Value::Unrecorded, Value::Unrecorded),
    };
    let (name, naming, name_hour) = match &copied.names {
        Some(([name, naming], supplied)) => (opaque(out, *name), opaque(out, *naming), optional(*supplied, Value::I32)),
        None => (Value::Unrecorded, Value::Unrecorded, Value::Unrecorded),
    };
    let (icon_unit, icon_custom, icon_name, icon_hour) = match &copied.icon {
        Some((icon, supplied)) => {
            let name = match icon.name {
                Some(text) => out.text(text),
                None => Value::Unrecorded,
            };
            (optional(icon.unit, Value::U32), optional(icon.custom, Value::I32), name, optional(*supplied, Value::I32))
        }
        None => (Value::Unrecorded, Value::Unrecorded, Value::Unrecorded, Value::Unrecorded),
    };
    vec![
        Value::OptionalU32(disposition),
        Value::OptionalU32(cause),
        optional(owner, Value::U32),
        optional(slot, Value::U32),
        Value::I32(death.map_or(at, |death| death.hour)),
        Value::Bool(death.is_some()),
        Value::OptionalI32(after),
        province,
        place_hour,
        template,
        template_hour,
        name,
        naming,
        name_hour,
        icon_unit,
        icon_custom,
        icon_name,
        icon_hour,
        Value::OptionalU32(death.map(|death| death.killer)),
        Value::OptionalU32(death.and_then(|death| inputs::resolve(hour, death.killer))),
        Value::OptionalU64(death.and_then(|death| death.division)),
        optional(copied.placed.and_then(|(placed, _)| placed.template), |template| Value::U32((template >> 32) as u32)),
        Value::OptionalU32(dating.matched),
        match &copied.template {
            Some(named) => out.text(&named.literal),
            None => Value::Unrecorded,
        },
        match copied.template.as_ref().and_then(|named| named.key.as_deref()) {
            Some(key) => out.text(key),
            None => Value::Unrecorded,
        },
        optional(copied.template.as_ref().map(|named| named.country), Value::I32),
    ]
}

fn key(kind: u8, hour: i32, division: u64) -> u128 {
    u128::from(kind) << 96 | u128::from(hour as u32) << 64 | u128::from(division)
}

fn killed(witness: &DivisionEquipmentDeletionRow) -> bool {
    matches!(witness.cause, Some(Cause::DivisionDestroyed | Cause::DivisionOverrun))
}

impl Writer for DivisionLog {
    fn name(&self) -> &'static str {
        "DivisionLog"
    }

    fn version(&self) -> u32 {
        3
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::Log(TableId::DivisionEvents, Keys::Hour(64))]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn hourly(&self) -> bool {
        true
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        out.status(TableId::DivisionEvents, Status::Complete);
        recorder_derive_framework::forces::forget_listed(hour, out, TableId::DivisionEvents);
        let at = hour.hour;
        let baseline = self.census.is_none();
        let dropped = Dropped::of(hour);
        let census = hour.table(TableId::Divisions);

        let sighted: Vec<u32> = if hour.fresh { census.rows().collect() } else { hour.written_rows(TableId::Divisions, Columns::These(&inputs::SHOWN)) };
        let mut sightings: Vec<(u64, u32)> = Vec::new();
        for slot in sighted {
            if !inputs::shown(&census, slot) {
                continue;
            }
            let Some(division) = census.key(slot).map(|key| key as u64) else { continue };
            if self.seen.insert(division) && !baseline {
                sightings.push((division, slot));
            }
        }
        sightings.sort_unstable();
        for (division, slot) in sightings {
            let copied = self.copied(hour, division, &dropped);
            let owner = inputs::division(&census, slot).owner;
            let row = cells(hour, out, None, owner, self.census, &copied, &Dating::default());
            out.append(TableId::DivisionEvents, key(DivisionEventRow::CREATED, at, division), row);
        }

        let removals = self.witnessed(hour)?;
        let gone = hour.forgotten(TableId::DestroyedDivisions);
        self.deaths.retain(|(ordinal, _)| !gone.iter().any(|run| run.contains(ordinal)));
        for (ordinal, death) in inputs::deaths(hour)? {
            self.deaths.push((ordinal, Death { of: death.division, hour: death.hour, province: death.province, killer: death.killer_country, division: death.killer_division.filter(|&id| id != 0) }));
        }
        if !baseline {
            let destroyed: Vec<u64> = removals.iter().filter(|witness| killed(witness)).map(|witness| witness.division_handle).collect();
            let deaths: Vec<Death> = self.deaths.iter().map(|(_, death)| *death).filter(|death| destroyed.binary_search(&death.of).is_ok()).collect();
            let spans = if deaths.is_empty() { Vec::new() } else { inputs::covered(hour)? };
            for witness in &removals {
                let division = witness.division_handle;
                let dating = if killed(witness) {
                    let matched: Vec<_> = deaths
                        .iter()
                        .filter(|death| death.of == division && death.hour <= at && self.census.is_some_and(|last| death.hour >= last) && spans.iter().any(|&(from, to)| from <= death.hour && to >= death.hour))
                        .collect();
                    let death = match matched[..] {
                        [death] => Some(*death),
                        _ => None,
                    };
                    Dating { death, matched: Some(matched.len() as u32) }
                } else {
                    Dating::default()
                };
                let kind = match witness.disposition {
                    Disposition::Loss | Disposition::Returned => DivisionEventRow::LOST,
                    _ => DivisionEventRow::TRANSFER,
                };
                let owner = if witness.expeditionary_owner as i32 > 0 { witness.expeditionary_owner } else { witness.country };
                let copied = self.copied(hour, division, &dropped);
                let row = cells(hour, out, Some(witness), Some(owner), None, &copied, &dating);
                out.append(TableId::DivisionEvents, key(kind, at, division), row);
            }
            if !hour.fresh {
                let witnesses = hour.table(TableId::DivisionEquipmentDeletions);
                let mut vanished: Vec<(u64, inputs::Placed)> = dropped
                    .divisions
                    .iter()
                    .filter(|(key, cells)| inputs::dropped_shown(cells) && witnesses.slot(**key).is_none())
                    .map(|(key, cells)| (*key as u64, inputs::dropped_division(cells)))
                    .collect();
                vanished.sort_unstable_by_key(|(division, _)| *division);
                for (division, placed) in vanished {
                    let copied = self.copied(hour, division, &dropped);
                    let row = cells(hour, out, None, placed.owner, None, &copied, &Dating::default());
                    out.append(TableId::DivisionEvents, key(DivisionEventRow::VANISHED, at, division), row);
                }
            }
        }
        self.census = Some(at);
        self.deaths.retain(|(_, death)| death.hour >= at);
        self.names = hour.table(TableId::DivisionStatus).sampled_at();
        self.icons = hour.table(TableId::DivisionTemplateIcons).sampled_at();
        Ok(())
    }

    fn save(&self, blob: &mut Bytes) -> io::Result<()> {
        for hour in [self.census, self.names, self.icons] {
            match hour {
                Some(hour) => {
                    blob.u8(1)?;
                    blob.raw(&hour.to_le_bytes())?;
                }
                None => blob.u8(0)?,
            }
        }
        let mut witnesses: Vec<(&u64, &Signature)> = self.witnesses.iter().collect();
        witnesses.sort_unstable_by_key(|(division, _)| **division);
        blob.varint(witnesses.len() as u64)?;
        for (division, (country, owner, disposition, cause)) in witnesses {
            blob.varint(*division)?;
            blob.varint(u64::from(*country))?;
            blob.varint(u64::from(*owner))?;
            blob.u8(*disposition)?;
            blob.u8(cause.map_or(0, |cause| cause.saturating_add(1)))?;
        }
        let mut seen: Vec<u64> = self.seen.iter().copied().collect();
        seen.sort_unstable();
        blob.varint(seen.len() as u64)?;
        let mut previous = 0;
        for division in seen {
            blob.varint(division - previous)?;
            previous = division;
        }
        blob.varint(self.deaths.len() as u64)?;
        for (ordinal, death) in &self.deaths {
            blob.varint(*ordinal)?;
            blob.varint(death.of)?;
            blob.raw(&death.hour.to_le_bytes())?;
            blob.varint(u64::from(death.province))?;
            blob.varint(u64::from(death.killer))?;
            blob.varint(death.division.map_or(0, |division| division.saturating_add(1)))?;
        }
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        *self = Self::default();
        let mut reader = Reader::new(blob);
        let mut hours = [None; 3];
        for hour in &mut hours {
            *hour = match reader.u8()? {
                0 => None,
                _ => Some(i32::from_le_bytes(reader.raw(4)?.try_into().map_err(|_| invalid("DivisionLog's state holds a cut hour"))?)),
            };
        }
        [self.census, self.names, self.icons] = hours;
        let count = reader.count()?;
        if count > blob.len() {
            return Err(invalid("DivisionLog's state counts more witnesses than it holds"));
        }
        for _ in 0..count {
            let division = reader.varint()?;
            let country = reader.varint_u32()?;
            let owner = reader.varint_u32()?;
            let disposition = reader.u8()?;
            let cause = reader.u8()?.checked_sub(1);
            self.witnesses.insert(division, (country, owner, disposition, cause));
        }
        let count = reader.count()?;
        if count > blob.len() {
            return Err(invalid("DivisionLog's state counts more divisions than it holds"));
        }
        self.seen.reserve(count);
        let mut division = 0u64;
        for _ in 0..count {
            division = division.checked_add(reader.varint()?).ok_or_else(|| invalid("DivisionLog's state runs a division past its handles"))?;
            self.seen.insert(division);
        }
        let count = reader.count()?;
        if count > blob.len() {
            return Err(invalid("DivisionLog's state counts more deaths than it holds"));
        }
        self.deaths.reserve(count);
        for _ in 0..count {
            let ordinal = reader.varint()?;
            let of = reader.varint()?;
            let hour = i32::from_le_bytes(reader.raw(4)?.try_into().map_err(|_| invalid("DivisionLog's state holds a cut death hour"))?);
            let province = reader.varint_u32()?;
            let killer = reader.varint_u32()?;
            let division = reader.varint()?.checked_sub(1);
            self.deaths.push((ordinal, Death { of, hour, province, killer, division }));
        }
        reader.finish()
    }
}

impl DivisionLog {
    fn witnessed(&mut self, hour: &Hour<'_>) -> io::Result<Vec<DivisionEquipmentDeletionRow>> {
        let table = TableId::DivisionEquipmentDeletions;
        let view = hour.table(table);
        let slots: Vec<u32> = if hour.fresh {
            let held: HashSet<u64> = view.rows().filter_map(|slot| view.key(slot)).map(|key| key as u64).collect();
            self.witnesses.retain(|division, _| held.contains(division));
            view.rows().collect()
        } else {
            for key in hour.deleted_keys(table) {
                if view.slot(key).is_none() {
                    self.witnesses.remove(&(key as u64));
                }
            }
            hour.written_rows(table, Columns::Every)
        };
        let mut removals = Vec::new();
        for slot in slots {
            let witness = inputs::witness(hour, slot)?;
            let signature = inputs::signature(&witness);
            if self.witnesses.insert(witness.division_handle, signature) != Some(signature) {
                removals.push(witness);
            }
        }
        removals.sort_unstable_by_key(|witness| witness.division_handle);
        Ok(removals)
    }
}
