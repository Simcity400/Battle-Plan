
use recorder_derive_framework::framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer};
use replay_columnar::{
    leaf::Writer as Bytes,
    state::Kind,
    store::{TableView, Value as Held},
};
use replay_data::{
    format::FormatVersion,
    military_aid,
    rows::{DivisionLandInputsRow, DivisionRow, DivisionTemplateIconRow, RowCodec, TableId},
};
use std::{
    collections::{HashMap, HashSet},
    io,
};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

const DIVISION_COLUMNS: u64 = 1 << 0 | 1 << 2 | 1 << 4 | 1 << 5;
const DIVISION_READ: &[usize] = &[0, 2, 4, 5];

const READS: &[Read] = &[
    Read {
        table: TableId::Divisions,
        columns: Columns::These(DIVISION_READ),
        watch: Watch::Changed,
        required: true,
    },
    Read {
        table: TableId::Countries,
        columns: Columns::These(&[0]),
        watch: Watch::Changed,
        required: true,
    },
    Read {
        table: TableId::CountryLinks,
        columns: Columns::These(&[0]),
        watch: Watch::Changed,
        required: true,
    },
    Read {
        table: TableId::DivisionLandInputs,
        columns: Columns::Every,
        watch: Watch::Changed,
        required: false,
    },
    Read {
        table: TableId::DivisionTemplateIcons,
        columns: Columns::Every,
        watch: Watch::Changed,
        required: false,
    },
];

const SIDES: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Division {
    raw: [u32; SIDES],
    template: u64,
    hit_points: Option<i64>,
}

#[derive(Clone, Debug, Default)]
struct Side {
    divisions: i64,
    hit_points: i128,
    bad_hit_points: i64,
    manpower: i128,
    bad_manpower: i64,
    battalions: i128,
    unknown_composition: i64,
    mix: HashMap<u32, i64>,
}

type Composition = Option<Vec<(u32, i64)>>;

#[derive(Clone, Copy)]
struct Fields {
    manpower: usize,
    subunits: usize,
    unreadable: usize,
}

impl Fields {
    fn resolve() -> io::Result<Self> {
        let field = |table: TableId, name: &str| {
            (0..table.field_count())
                .find(|&at| table.field_name(at) == Some(name))
                .ok_or_else(|| invalid(format!("{table:?} has no field {name}")))
        };
        Ok(Self {
            manpower: field(TableId::DivisionLandInputs, "manpower")?,
            subunits: field(TableId::DivisionTemplateIcons, "subunits")?,
            unreadable: field(TableId::DivisionTemplateIcons, "model_unreadable")?,
        })
    }
}

#[derive(Default)]
pub struct Army {
    fields: Option<Fields>,
    rebuild: bool,
    links: HashMap<u32, u32>,
    countries: HashSet<u32>,
    divisions: HashMap<u64, Division>,
    manpower: Option<HashMap<u64, i32>>,
    templates: Option<HashMap<u64, Composition>>,
    sides: HashMap<(u32, usize), Side>,
    unresolved: [i64; SIDES],
    held_totals: HashSet<u32>,
    held_mix: HashMap<(u32, usize), HashSet<u32>>,
    mix_dirty: HashSet<(u32, usize)>,
}

fn written(hour: &Hour<'_>, table: TableId, columns: &'static [usize]) -> Vec<u32> {
    let mut slots = hour.written_rows(table, Columns::These(columns));
    slots.sort_unstable();
    slots.dedup();
    slots
}

fn row<R: RowCodec>(hour: &Hour<'_>, table: TableId, slot: u32, mask: u64) -> io::Result<R> {
    let held = hour.table(table);
    let key = held.key(slot).ok_or_else(|| invalid(format!("{table:?} slot {slot} holds no row")))?;
    let format = FormatVersion::CURRENT.version();
    R::from_wire_fields(key, mask, &mut |column| replay_frame::frame::field(&held, slot, column, format))
}

fn division(row: &DivisionRow) -> Division {
    let finite = (row.hit_points as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd;
    Division {
        raw: [military_aid::owner(row), row.roster_owner],
        template: row.template,
        hit_points: (finite && row.hit_points >= 0).then_some(row.hit_points),
    }
}

fn manpower(row: &DivisionLandInputsRow) -> i32 {
    row.manpower.iter().fold(0_i32, |sum, part| sum.wrapping_add(part.amount))
}

fn composition(row: &DivisionTemplateIconRow) -> Composition {
    if row.model_unreadable != 0 {
        return None;
    }
    let mut parts: Vec<(u32, i64)> = Vec::new();
    for unit in &row.subunits {
        if unit.token == 0 {
            continue;
        }
        if unit.count < 0 {
            return None;
        }
        match parts.iter_mut().find(|(token, _)| *token == unit.token) {
            Some((_, count)) => *count = count.checked_add(i64::from(unit.count))?,
            None => parts.push((unit.token, i64::from(unit.count))),
        }
    }
    parts.sort_unstable();
    Some(parts)
}

impl Army {
    fn fields(&mut self) -> io::Result<Fields> {
        if let Some(fields) = self.fields {
            return Ok(fields);
        }
        let fields = Fields::resolve()?;
        self.fields = Some(fields);
        Ok(fields)
    }

    fn count(&mut self, id: u64, division: &Division, sign: i64, dirty: &mut HashSet<u32>) {
        let manpower = self.manpower.as_ref().and_then(|held| held.get(&id).copied());
        let composition = self
            .templates
            .as_ref()
            .and_then(|templates| templates.get(&division.template))
            .and_then(Option::as_ref);
        for (at, &raw) in division.raw.iter().enumerate() {
            let Some(slot) = (raw != 0).then(|| self.links.get(&raw).copied()).flatten() else {
                self.unresolved[at] += sign;
                continue;
            };
            dirty.insert(slot);
            let side = self.sides.entry((slot, at)).or_default();
            side.divisions += sign;
            match division.hit_points {
                Some(points) => side.hit_points += i128::from(sign) * i128::from(points),
                None => side.bad_hit_points += sign,
            }
            match manpower {
                Some(amount) => side.manpower += i128::from(sign) * i128::from(amount),
                None => side.bad_manpower += sign,
            }
            if composition.is_some_and(|parts| !parts.is_empty()) {
                self.mix_dirty.insert((slot, at));
            }
            match composition {
                Some(parts) => {
                    for &(token, count) in parts {
                        side.battalions += i128::from(sign) * i128::from(count);
                        let held = side.mix.entry(token).or_default();
                        *held += sign * count;
                        if *held == 0 {
                            side.mix.remove(&token);
                        }
                    }
                }
                None => side.unknown_composition += sign,
            }
        }
    }

    fn rebuild(&mut self, hour: &Hour<'_>, out: &Rows<'_>, fields: Fields) -> io::Result<()> {
        self.links.clear();
        let links = hour.table(TableId::CountryLinks);
        for slot in links.rows() {
            let (Some(key), Some(canonical)) = (links.key(slot), u32_cell(&links, 0, slot)) else { continue };
            self.links.insert(key as u32, canonical);
        }
        self.countries.clear();
        let countries = hour.table(TableId::Countries);
        for slot in countries.rows() {
            if let Some(canonical) = u32_cell(&countries, 0, slot) {
                self.countries.insert(canonical);
            }
        }
        self.divisions.clear();
        for slot in hour.table(TableId::Divisions).rows() {
            let row: DivisionRow = row(hour, TableId::Divisions, slot, DIVISION_COLUMNS)?;
            self.divisions.insert(row.id, division(&row));
        }
        self.manpower = hour
            .complete(TableId::DivisionLandInputs)
            .then(|| -> io::Result<HashMap<u64, i32>> {
                let table = hour.table(TableId::DivisionLandInputs);
                let mut held = HashMap::with_capacity(table.live().count());
                for slot in table.rows() {
                    let row: DivisionLandInputsRow = row(hour, TableId::DivisionLandInputs, slot, 1 << fields.manpower)?;
                    held.insert(row.division, manpower(&row));
                }
                Ok(held)
            })
            .transpose()?;
        self.templates = hour
            .complete(TableId::DivisionTemplateIcons)
            .then(|| -> io::Result<HashMap<u64, Composition>> {
                let table = hour.table(TableId::DivisionTemplateIcons);
                let mask = 1 << fields.subunits | 1 << fields.unreadable;
                let mut held = HashMap::with_capacity(table.live().count());
                for slot in table.rows() {
                    let row: DivisionTemplateIconRow = row(hour, TableId::DivisionTemplateIcons, slot, mask)?;
                    held.insert(row.template, composition(&row));
                }
                Ok(held)
            })
            .transpose()?;
        self.sides.clear();
        self.unresolved = [0; SIDES];
        let divisions: Vec<(u64, Division)> = self.divisions.iter().map(|(&id, &division)| (id, division)).collect();
        let mut ignored = HashSet::new();
        for (id, division) in &divisions {
            self.count(*id, division, 1, &mut ignored);
        }
        self.held_totals.clear();
        let totals = out.table(TableId::ArmyTotals);
        self.held_totals.extend(totals.rows().filter_map(|slot| totals.key(slot)).map(|key| key as u32));
        self.held_mix.clear();
        let mix = out.table(TableId::ArmyMix);
        for key in mix.rows().filter_map(|slot| mix.key(slot)) {
            let (slot, side, token) = ((key >> 40) as u32, ((key >> 32) & 0xff) as usize, key as u32);
            self.held_mix.entry((slot, side)).or_default().insert(token);
        }
        Ok(())
    }

    fn recompose(&mut self, hour: &Hour<'_>, fields: Fields, dirty: &mut HashSet<u32>) -> io::Result<()> {
        let Some(templates) = &self.templates else { return Ok(()) };
        let changes = hour.changes(TableId::DivisionTemplateIcons);
        let view = hour.table(TableId::DivisionTemplateIcons);
        let mut slots: Vec<u32> = changes.written.iter().filter(|&&(_, column)| usize::from(column) == fields.subunits || usize::from(column) == fields.unreadable).map(|&(slot, _)| slot).collect();
        slots.extend_from_slice(&changes.inserted);
        slots.sort_unstable();
        slots.dedup();
        let mask = 1 << fields.subunits | 1 << fields.unreadable;
        let mut moved: HashMap<u64, Option<Composition>> = HashMap::new();
        for &(_, key) in &changes.deleted {
            if view.slot(key).is_none() {
                moved.insert(key as u64, None);
            }
        }
        for slot in slots {
            let row: DivisionTemplateIconRow = row(hour, TableId::DivisionTemplateIcons, slot, mask)?;
            moved.insert(row.template, Some(composition(&row)));
        }
        fn counted(composition: Option<&Composition>) -> Option<&Vec<(u32, i64)>> {
            composition.and_then(Option::as_ref)
        }
        let recount: HashSet<u64> = moved.iter().filter(|(template, now)| counted(templates.get(*template)) != counted(now.as_ref())).map(|(&template, _)| template).collect();
        let divisions: Vec<(u64, Division)> = if recount.is_empty() {
            Vec::new()
        } else {
            self.divisions.iter().filter(|(_, division)| recount.contains(&division.template)).map(|(&id, &division)| (id, division)).collect()
        };
        for (id, division) in &divisions {
            self.count(*id, division, -1, dirty);
        }
        for (template, now) in moved {
            self.set_template(template, now);
        }
        for (id, division) in &divisions {
            self.count(*id, division, 1, dirty);
        }
        Ok(())
    }

    fn set_template(&mut self, template: u64, composition: Option<Composition>) {
        if let Some(templates) = &mut self.templates {
            match composition {
                Some(composition) => templates.insert(template, composition),
                None => templates.remove(&template),
            };
        }
    }

    fn apply(&mut self, hour: &Hour<'_>, fields: Fields, dirty: &mut HashSet<u32>) -> io::Result<bool> {
        let manpower_mask = 1 << fields.manpower;
        if hour.wrote(TableId::CountryLinks, Columns::These(&[0]))
            || hour.wrote(TableId::Countries, Columns::These(&[0]))
            || hour.changes(TableId::DivisionTemplateIcons).whole
            || hour.complete(TableId::DivisionLandInputs) != self.manpower.is_some()
            || hour.complete(TableId::DivisionTemplateIcons) != self.templates.is_some()
        {
            return Ok(false);
        }
        self.recompose(hour, fields, dirty)?;
        let mut edited: HashMap<u64, (Option<Division>, Option<Division>)> = HashMap::new();
        for key in hour.deleted_keys(TableId::Divisions) {
            let id = key as u64;
            let old = self.divisions.get(&id).copied();
            edited.entry(id).or_insert((old, None)).1 = None;
        }
        for slot in written(hour, TableId::Divisions, DIVISION_READ) {
            let row: DivisionRow = row(hour, TableId::Divisions, slot, DIVISION_COLUMNS)?;
            let old = self.divisions.get(&row.id).copied();
            edited.entry(row.id).or_insert((old, None)).1 = Some(division(&row));
        }
        let mut amounts: Vec<(u64, Option<i32>)> = Vec::new();
        if self.manpower.is_some() {
            amounts.extend(hour.deleted_keys(TableId::DivisionLandInputs).map(|key| (key as u64, None)));
            let land = hour.changes(TableId::DivisionLandInputs);
            let mut slots: Vec<u32> = land
                .written
                .iter()
                .filter(|&&(_, column)| usize::from(column) == fields.manpower)
                .map(|&(slot, _)| slot)
                .chain(land.inserted.iter().copied())
                .collect();
            slots.sort_unstable();
            slots.dedup();
            for slot in slots {
                let row: DivisionLandInputsRow = row(hour, TableId::DivisionLandInputs, slot, manpower_mask)?;
                amounts.push((row.division, Some(manpower(&row))));
            }
        }
        for &(id, _) in &amounts {
            if let Some(&held) = self.divisions.get(&id) {
                edited.entry(id).or_insert((Some(held), Some(held)));
            }
        }
        let moved_amounts: HashSet<u64> = amounts.iter().map(|&(id, _)| id).collect();
        edited.retain(|&id, (old, new)| {
            let (Some(before), Some(after)) = (*old, *new) else { return true };
            if moved_amounts.contains(&id) {
                return true;
            }
            if before == after {
                return false;
            }
            if before.raw != after.raw || before.template != after.template {
                return true;
            }
            for (at, &raw) in before.raw.iter().enumerate() {
                let Some(slot) = (raw != 0).then(|| self.links.get(&raw).copied()).flatten() else { continue };
                let side = self.sides.entry((slot, at)).or_default();
                match before.hit_points {
                    Some(points) => side.hit_points -= i128::from(points),
                    None => side.bad_hit_points -= 1,
                }
                match after.hit_points {
                    Some(points) => side.hit_points += i128::from(points),
                    None => side.bad_hit_points += 1,
                }
                dirty.insert(slot);
            }
            self.divisions.insert(id, after);
            false
        });
        for (&id, (old, _)) in &edited {
            if let Some(old) = old {
                self.count(id, old, -1, dirty);
            }
        }
        if let Some(held) = &mut self.manpower {
            for (id, amount) in amounts {
                match amount {
                    Some(amount) => held.insert(id, amount),
                    None => held.remove(&id),
                };
            }
        }
        for (id, (_, new)) in edited {
            match new {
                Some(new) => {
                    self.count(id, &new, 1, dirty);
                    self.divisions.insert(id, new);
                }
                None => {
                    self.divisions.remove(&id);
                }
            }
        }
        Ok(true)
    }

    fn totals(&self, slot: u32) -> Vec<Value> {
        let mut cells = vec![Value::Unrecorded; 8];
        for at in 0..SIDES {
            if self.unresolved[at] != 0 {
                continue;
            }
            let side = self.sides.get(&(slot, at));
            let exact = |value: i128| i64::try_from(value).map_or(Value::Unrecorded, Value::I64);
            let (divisions, strength, battalions, manpower) = match at {
                0 => (0, 1, 4, 6),
                _ => (2, 3, 5, 7),
            };
            cells[divisions] = Value::I64(side.map_or(0, |side| side.divisions));
            if side.is_none_or(|side| side.bad_hit_points == 0) {
                cells[strength] = exact(side.map_or(0, |side| side.hit_points));
            }
            if self.manpower.is_some() && side.is_none_or(|side| side.bad_manpower == 0) {
                cells[manpower] = exact(side.map_or(0, |side| side.manpower));
            }
            if self.templates.is_some() && side.is_none_or(|side| side.unknown_composition == 0) {
                cells[battalions] = exact(side.map_or(0, |side| side.battalions));
            }
        }
        cells
    }

    fn write(&mut self, dirty: HashSet<u32>, all_mix: bool, out: &mut Rows) {
        let mix_dirty = std::mem::take(&mut self.mix_dirty);
        for slot in dirty {
            if !self.countries.contains(&slot) && !(0..SIDES).any(|at| self.sides.get(&(slot, at)).is_some_and(|side| side.divisions != 0)) {
                if self.held_totals.remove(&slot) {
                    out.delete(TableId::ArmyTotals, u128::from(slot));
                }
                for at in 0..SIDES {
                    self.sides.remove(&(slot, at));
                    for token in self.held_mix.remove(&(slot, at)).unwrap_or_default() {
                        out.delete(TableId::ArmyMix, mix_key(slot, at, token));
                    }
                }
                continue;
            }
            out.set(TableId::ArmyTotals, u128::from(slot), self.totals(slot));
            self.held_totals.insert(slot);
            for at in 0..SIDES {
                if !all_mix && !mix_dirty.contains(&(slot, at)) {
                    continue;
                }
                let known = self.templates.is_some()
                    && self.unresolved[at] == 0
                    && self.sides.get(&(slot, at)).is_none_or(|side| side.unknown_composition == 0);
                let now: HashMap<u32, i64> = match (known, self.sides.get(&(slot, at))) {
                    (true, Some(side)) => side.mix.clone(),
                    _ => HashMap::new(),
                };
                let held = self.held_mix.entry((slot, at)).or_default();
                for token in held.iter().copied().filter(|token| !now.contains_key(token)).collect::<Vec<_>>() {
                    out.delete(TableId::ArmyMix, mix_key(slot, at, token));
                    held.remove(&token);
                }
                for (token, count) in now {
                    out.set(TableId::ArmyMix, mix_key(slot, at, token), vec![Value::I64(count)]);
                    held.insert(token);
                }
            }
        }
    }
}

fn mix_key(slot: u32, side: usize, token: u32) -> u128 {
    (u128::from(slot) << 40) | ((side as u128) << 32) | u128::from(token)
}

fn u32_cell(table: &TableView<'_>, column: usize, slot: u32) -> Option<u32> {
    match (table.kinds().get(column)?, table.value(column, slot)) {
        (Kind::U32, Held::Number(bits)) => Some(bits as u32),
        _ => None,
    }
}

impl Writer for Army {
    fn name(&self) -> &'static str {
        "Army"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::ArmyTotals), Output::State(TableId::ArmyMix)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let fields = self.fields()?;
        let unresolved = self.unresolved;
        let mut dirty = HashSet::new();
        let applied = !hour.fresh && !self.rebuild && self.apply(hour, fields, &mut dirty)?;
        let all_mix = !applied || self.unresolved != unresolved;
        if !applied {
            self.rebuild(hour, out, fields)?;
            self.rebuild = false;
            dirty.extend(self.countries.iter().copied());
            dirty.extend(self.sides.keys().map(|(slot, _)| *slot));
            dirty.extend(self.held_totals.iter().copied());
        } else if self.unresolved != unresolved {
            dirty.extend(self.held_totals.iter().copied());
        }
        self.write(dirty, all_mix, out);
        Ok(())
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if !blob.is_empty() {
            return Err(invalid("Army's state holds nothing"));
        }
        self.rebuild = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::TemplateSubunit;

    fn army() -> Army {
        let mut army = Army::default();
        army.links = [(10, 1), (20, 2)].into();
        army.countries = [1, 2, 3].into();
        army.manpower = Some(HashMap::new());
        army.templates = Some([(7, Some(vec![(100, 2), (101, 1)]))].into());
        army
    }

    fn division(owner: u32, host: u32, hit_points: Option<i64>) -> Division {
        Division { raw: [owner, host], template: 7, hit_points }
    }

    #[test]
    fn a_country_with_no_division_holds_recorded_zeros_and_a_division_counts_for_owner_and_host() {
        let mut army = army();
        army.manpower.as_mut().unwrap().insert(1, 1000);
        let mut dirty = HashSet::new();
        army.count(1, &division(10, 20, Some(300_000)), 1, &mut dirty);
        assert_eq!(dirty, [1, 2].into());
        let owner = army.totals(1);
        assert_eq!(owner[0], Value::I64(1), "owner divisions");
        assert_eq!(owner[1], Value::I64(300_000), "owner strength");
        assert_eq!(owner[4], Value::I64(3), "owner battalions");
        assert_eq!(owner[6], Value::I64(1000), "owner manpower");
        assert_eq!(owner[2], Value::I64(0), "slot 1 hosts nothing");
        assert_eq!(army.totals(2)[2], Value::I64(1), "host divisions");
        assert_eq!(army.totals(3), vec![Value::I64(0), Value::I64(0), Value::I64(0), Value::I64(0), Value::I64(0), Value::I64(0), Value::I64(0), Value::I64(0)]);
        army.count(1, &division(10, 20, Some(300_000)), -1, &mut dirty);
        assert_eq!(army.totals(1)[0], Value::I64(0));
        assert!(army.sides.values().all(|side| side.mix.is_empty()));
    }

    #[test]
    fn what_is_not_known_is_not_recorded_never_zero() {
        let mut army = army();
        let mut dirty = HashSet::new();
        army.count(1, &division(10, 20, None), 1, &mut dirty);
        let owner = army.totals(1);
        assert_eq!(owner[0], Value::I64(1));
        assert_eq!(owner[1], Value::Unrecorded, "strength with an unreadable division");
        assert_eq!(owner[6], Value::Unrecorded, "manpower of a division with no land inputs");
        army.count(2, &division(99, 20, Some(1)), 1, &mut dirty);
        assert!(army.totals(3)[..2].iter().all(|cell| *cell == Value::Unrecorded) && army.totals(3)[4] == Value::Unrecorded);
        assert_eq!(army.totals(3)[2], Value::I64(0), "host divisions stay known");
        army.templates = None;
        assert_eq!(army.totals(2)[5], Value::Unrecorded);
    }

    #[test]
    fn a_template_composition_skips_the_null_token_and_refuses_negative_counts() {
        let unit = |token, count| TemplateSubunit { token, local: 0, count };
        let row = DivisionTemplateIconRow { template: 7, subunits: vec![unit(0, 5), unit(100, 2), unit(100, 1), unit(101, 3)], ..Default::default() };
        assert_eq!(composition(&row), Some(vec![(100, 3), (101, 3)]));
        let negative = DivisionTemplateIconRow { subunits: vec![unit(100, -1)], ..row.clone() };
        assert_eq!(composition(&negative), None);
        let unreadable = DivisionTemplateIconRow { model_unreadable: 1, ..row };
        assert_eq!(composition(&unreadable), None);
    }

    #[test]
    fn a_template_recomposed_counts_again_its_own_divisions_alone() {
        use recorder_derive_framework::{
            testing::deployed::{Edits, Run, division as division_row, world},
            testing::{
                self,
                noting::{self, Noting},
            },
        };
        fn noted() -> Vec<Box<dyn Writer>> {
            vec![Box::new(Noting(Army::default()))]
        }
        const INPUTS: &[TableId] = &[TableId::Countries, TableId::CountryLinks, TableId::Divisions, TableId::DivisionLandInputs, TableId::DivisionTemplateIcons];
        let unit = |token, count| TemplateSubunit { token, local: 0, count };
        let template = |template: u64, subunits: Vec<TemplateSubunit>| DivisionTemplateIconRow { template, subunits, ..Default::default() };
        let mut run = Run::new(noted, INPUTS);
        let mut edits = Edits::default();
        world(&mut edits).put(TableId::DivisionTemplateIcons, template(7, vec![unit(100, 2)])).put(TableId::DivisionTemplateIcons, template(8, vec![unit(101, 3)]));
        for (id, roster, handle) in (1..=3).map(|id| (id, 10, 7)).chain((10..=60).map(|id| (id, 20, 8))) {
            edits.put(TableId::Divisions, DivisionRow { hit_points: 100_000, ..division_row(id, roster, roster, 0, handle) });
        }
        run.hour(&edits);
        noting::take();
        let mut steps = [Edits::default(), Edits::default()];
        steps[0].put(TableId::DivisionTemplateIcons, template(9, vec![unit(102, 1)]));
        steps[1].put(TableId::DivisionTemplateIcons, template(8, vec![unit(101, 3), unit(102, 1)]));
        for (at, step) in steps.iter().enumerate() {
            let mut resumed = run.resumed();
            run.hour(step);
            let noted = noting::take();
            resumed.hour(step);
            noting::take();
            for table in [TableId::ArmyTotals, TableId::ArmyMix] {
                assert_eq!(testing::rows(&run.store, table), testing::rows(&resumed.store, table), "step {at}: {table:?}");
            }
            let slots: Vec<u32> = noted[0].iter().flat_map(|(table, sets, deletes)| sets.iter().chain(deletes).map(move |&key| if *table == TableId::ArmyMix { (key >> 40) as u32 } else { key as u32 })).collect();
            match at {
                0 => assert!(slots.is_empty(), "{noted:?}"),
                _ => assert!(!slots.is_empty() && slots.iter().all(|&slot| slot == 2), "{noted:?}"),
            }
        }
        assert_eq!(army_battalions(&run, 2), Some(51 * 4), "ITA's divisions of template 8 hold four battalions each");
    }

    fn army_battalions(run: &recorder_derive_framework::testing::deployed::Run, slot: u32) -> Option<i64> {
        recorder_derive_framework::forces::testing::cells(&run.store, TableId::ArmyTotals, u128::from(slot)).and_then(|cells| cells[4])
    }

    #[test]
    fn its_state_is_its_tables_so_a_resume_rebuilds_from_them() {
        let mut army = Army::default();
        let mut blob = Bytes::new();
        army.save(&mut blob).unwrap();
        assert!(blob.bytes().is_empty());
        army.load(&[]).unwrap();
        assert!(army.rebuild);
        assert!(army.load(&[1]).is_err());
    }
}
