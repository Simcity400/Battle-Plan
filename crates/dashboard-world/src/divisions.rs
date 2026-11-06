//! The divisions of the hour: each division's countries, template, hit points and reveal byte
//! (Divisions), its maximum hit points (DivisionStatus) and its manpower (DivisionLandInputs),
//! kept from the rows each hour wrote in those fields. A table is held while it holds every
//! row; it is read whole at the walk's first hour, when it is held again, and when its
//! changes name a row it does not hold.

use super::{Facts, changes::Changes};
use replay::{columns, segments::State};
use replay_data::{
    display::stepped_format,
    format::FormatVersion,
    rows::{DivisionLandInputsRow, DivisionRow, DivisionStatusRow, TableId},
};
use std::{collections::HashMap, io};

/// A division as Divisions holds it, in the fields its readers read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct Division {
    pub id: u64,
    pub roster_owner: u32,
    pub logical_owner: u32,
    pub expeditionary_owner: u32,
    pub template: u64,
    /// Native fixed5, as the row decoder reads it.
    pub hit_points: i64,
    pub reveal_skip: u8,
}

impl Division {
    /// The Divisions row of these fields alone, for readers of rows.
    pub fn row(&self) -> DivisionRow {
        DivisionRow {
            id: self.id,
            roster_owner: self.roster_owner,
            logical_owner: self.logical_owner,
            expeditionary_owner: self.expeditionary_owner,
            template: self.template,
            hit_points: self.hit_points,
            reveal_skip: self.reveal_skip,
            ..Default::default()
        }
    }
}

/// What one of the tables holds this hour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Held {
    /// Every row; read whole this hour when `whole`.
    Rows { whole: bool },
    /// Not every row: unavailable, partly read, not recorded or not walked.
    Missing,
    /// Its rows could not be read, until it is read whole again: why.
    Refused(String),
}

/// One table's values by division, and the hour's edits of them.
pub struct Kept<T> {
    held: Held,
    values: HashMap<u64, T>,
    /// Each division the hour wrote in the fields read: before and after. None when read whole.
    edits: Vec<(u64, Option<T>, Option<T>)>,
    changes: Changes,
}

impl<T: Copy + PartialEq> Kept<T> {
    fn new() -> Self {
        Self {
            held: Held::Missing,
            values: HashMap::new(),
            edits: Vec::new(),
            changes: Changes::default(),
        }
    }

    pub fn held(&self) -> &Held {
        &self.held
    }

    /// Whether it holds every row this hour.
    pub fn holds(&self) -> bool {
        matches!(self.held, Held::Rows { .. })
    }

    /// Whether it was read whole this hour.
    pub fn whole(&self) -> bool {
        matches!(self.held, Held::Rows { whole: true })
    }

    /// Why its rows could not be read; none while they could.
    pub fn refusal(&self) -> Option<&str> {
        match &self.held {
            Held::Refused(why) => Some(why),
            _ => None,
        }
    }

    pub fn get(&self, division: u64) -> Option<&T> {
        self.values.get(&division)
    }

    /// Every value, unordered.
    pub fn iter(&self) -> impl Iterator<Item = (u64, &T)> + '_ {
        self.values
            .iter()
            .map(|(&division, value)| (division, value))
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// The divisions the hour wrote in the fields read, each once: before and after.
    pub fn edits(&self) -> &[(u64, Option<T>, Option<T>)] {
        &self.edits
    }

    /// Follows `table` this hour, reading a row's value with `read`.
    fn step(
        &mut self,
        state: &State,
        (table, fields): (TableId, u64),
        first: bool,
        read: impl FnMut(u32, u128) -> io::Result<T>,
    ) -> io::Result<()> {
        self.edits.clear();
        if state.skips(table.index()) || !columns::complete(state, table) {
            self.values.clear();
            self.held = Held::Missing;
            return Ok(());
        }
        let whole = first || !self.holds();
        match self.read(state, (table, fields), whole, read) {
            Ok(()) => Ok(()),
            Err(error) if engine_allocation::is_allocation_error(&error) => Err(error),
            Err(error) => {
                self.values.clear();
                self.edits.clear();
                self.held = Held::Refused(error.to_string());
                Ok(())
            }
        }
    }

    fn read(
        &mut self,
        state: &State,
        (table, fields): (TableId, u64),
        whole: bool,
        mut read: impl FnMut(u32, u128) -> io::Result<T>,
    ) -> io::Result<()> {
        let division = |key: u128| {
            u64::try_from(key)
                .map_err(|_| invalid(format!("{table:?} keys a division past 64 bits")))
        };
        let whole = whole || {
            self.changes.read(state, table)?;
            self.changes
                .deleted()
                .iter()
                .any(|&key| u64::try_from(key).map_or(true, |key| !self.values.contains_key(&key)))
        };
        if whole {
            let held = columns::table(state, table)?;
            self.values.clear();
            engine_allocation::reserve_map(&mut self.values, held.live().count())?;
            for slot in held.live().ones() {
                let key = held
                    .key(slot)
                    .ok_or_else(|| invalid(format!("{table:?} slot {slot} holds no key")))?;
                let value = read(slot, key)?;
                if self.values.insert(division(key)?, value).is_some() {
                    return Err(invalid(format!("{table:?} holds a division twice")));
                }
            }
            self.held = Held::Rows { whole: true };
            return Ok(());
        }
        let changes = &self.changes;
        // A key deleted and written again in one hour is one edit.
        let mut at = HashMap::<u64, usize>::new();
        for &key in changes.deleted() {
            let key = division(key)?;
            if let Some(before) = self.values.remove(&key) {
                engine_allocation::reserve(&mut self.edits, 1)?;
                at.insert(key, self.edits.len());
                self.edits.push((key, Some(before), None));
            }
        }
        let mut written =
            engine_allocation::vec(changes.written().len() + changes.inserted().len())?;
        written.extend(
            changes
                .written()
                .iter()
                .filter(|&&(key, _, mask)| {
                    mask & fields != 0
                        || u64::try_from(key).is_ok_and(|key| !self.values.contains_key(&key))
                })
                .map(|&(key, slot, _)| (key, slot))
                .chain(changes.inserted().iter().copied()),
        );
        for (key, slot) in written {
            let value = read(slot, key)?;
            let key = division(key)?;
            let before = self.values.insert(key, value);
            match at.get(&key) {
                Some(&edit) => self.edits[edit].2 = Some(value),
                None if before != Some(value) => {
                    engine_allocation::reserve(&mut self.edits, 1)?;
                    at.insert(key, self.edits.len());
                    self.edits.push((key, before, Some(value)));
                }
                None => {}
            }
        }
        self.edits.retain(|(_, before, after)| before != after);
        self.held = Held::Rows { whole: false };
        Ok(())
    }
}

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

/// The columns a part reads: Divisions', DivisionStatus' maximum and DivisionLandInputs'
/// manpower list.
struct Fields {
    roster: usize,
    logical: usize,
    expeditionary: usize,
    template: usize,
    hit_points: usize,
    reveal: usize,
    maximum: usize,
    manpower: usize,
}

/// The hour's divisions, their maximum hit points and their manpower.
pub struct Divisions {
    format: FormatVersion,
    fields: Fields,
    divisions: Kept<Division>,
    maxima: Kept<i64>,
    manpower: Kept<i32>,
}

/// A stepped field's native value from the units a typed column holds in `format`.
fn native(table: TableId, field: usize, format: FormatVersion, units: i64) -> io::Result<i64> {
    match table.display_step(field) {
        Some(step) if stepped_format(format.version()) => step.value(units),
        _ => Ok(units),
    }
}

impl Divisions {
    pub(crate) fn new(format: FormatVersion) -> io::Result<Self> {
        let field = |table, name| columns::field(table, name);
        Ok(Self {
            format,
            fields: Fields {
                roster: field(TableId::Divisions, "roster_owner")?,
                logical: field(TableId::Divisions, "logical_owner")?,
                expeditionary: field(TableId::Divisions, "expeditionary_owner")?,
                template: field(TableId::Divisions, "template")?,
                hit_points: field(TableId::Divisions, "hit_points")?,
                reveal: field(TableId::Divisions, "reveal_skip")?,
                maximum: field(TableId::DivisionStatus, "maximum_hit_points")?,
                manpower: field(TableId::DivisionLandInputs, "manpower")?,
            },
            divisions: Kept::new(),
            maxima: Kept::new(),
            manpower: Kept::new(),
        })
    }

    /// Each division's Divisions facts.
    pub fn divisions(&self) -> &Kept<Division> {
        &self.divisions
    }

    /// Each division's maximum hit points, native fixed5.
    pub fn maxima(&self) -> &Kept<i64> {
        &self.maxima
    }

    /// Each division's manpower: its contributors' signed amounts summed as the game's dwords
    /// wrap.
    pub fn manpower(&self) -> &Kept<i32> {
        &self.manpower
    }

    /// The division `id` names, when Divisions holds it.
    pub fn division(&self, id: u64) -> Option<&Division> {
        self.divisions.get(id)
    }

    pub(crate) fn hour(&mut self, state: &State, facts: &Facts<'_>) -> io::Result<()> {
        let (format, first, fields) = (self.format, facts.first(), &self.fields);
        let version = format.version();
        let division_fields = [
            fields.roster,
            fields.logical,
            fields.expeditionary,
            fields.template,
            fields.hit_points,
            fields.reveal,
        ];
        let mask = division_fields
            .iter()
            .fold(0_u64, |mask, &field| mask | 1 << field);
        self.divisions
            .step(state, (TableId::Divisions, mask), first, |slot, key| {
                let held = columns::table(state, TableId::Divisions)?;
                if division_fields
                    .iter()
                    .any(|&field| !held.recorded(field, slot))
                {
                    // An unrecorded cell reads as the row decoder reads it.
                    let row: DivisionRow = columns::row(state, TableId::Divisions, slot, version)?;
                    return Ok(Division {
                        id: row.id,
                        roster_owner: row.roster_owner,
                        logical_owner: row.logical_owner,
                        expeditionary_owner: row.expeditionary_owner,
                        template: row.template,
                        hit_points: row.hit_points,
                        reveal_skip: row.reveal_skip,
                    });
                }
                let at = slot as usize;
                let missing = || invalid(format!("Divisions slot {slot} holds no cell"));
                let u32s = |field| -> io::Result<u32> {
                    columns::plain::<u32>(held, field)?
                        .get(at)
                        .copied()
                        .ok_or_else(missing)
                };
                let units = *columns::plain::<i64>(held, fields.hit_points)?
                    .get(at)
                    .ok_or_else(missing)?;
                Ok(Division {
                    id: u64::try_from(key).map_err(|_| missing())?,
                    roster_owner: u32s(fields.roster)?,
                    logical_owner: u32s(fields.logical)?,
                    expeditionary_owner: u32s(fields.expeditionary)?,
                    template: *columns::plain::<u64>(held, fields.template)?
                        .get(at)
                        .ok_or_else(missing)?,
                    hit_points: native(TableId::Divisions, fields.hit_points, format, units)?,
                    reveal_skip: *columns::plain::<u8>(held, fields.reveal)?
                        .get(at)
                        .ok_or_else(missing)?,
                })
            })?;
        let maximum = fields.maximum;
        self.maxima.step(
            state,
            (TableId::DivisionStatus, 1 << maximum),
            first,
            |slot, _| {
                let held = columns::table(state, TableId::DivisionStatus)?;
                if !held.recorded(maximum, slot) {
                    let row: DivisionStatusRow =
                        columns::row(state, TableId::DivisionStatus, slot, version)?;
                    return Ok(row.maximum_hit_points);
                }
                let units = *columns::plain::<i64>(held, maximum)?
                    .get(slot as usize)
                    .ok_or_else(|| invalid(format!("DivisionStatus slot {slot} holds no cell")))?;
                native(TableId::DivisionStatus, maximum, format, units)
            },
        )?;
        // The contributors are a list: decoded for the rows whose list the hour wrote.
        self.manpower.step(
            state,
            (TableId::DivisionLandInputs, 1 << fields.manpower),
            first,
            |slot, _| {
                let row: DivisionLandInputsRow =
                    columns::row(state, TableId::DivisionLandInputs, slot, version)?;
                Ok(row
                    .manpower
                    .iter()
                    .fold(0_i32, |sum, part| sum.wrapping_add(part.amount)))
            },
        )
    }

    /// The bytes it holds.
    pub fn held_bytes(&self) -> usize {
        self.divisions.values.capacity() * (size_of::<(u64, Division)>() + 1)
            + self.maxima.values.capacity() * (size_of::<(u64, i64)>() + 1)
            + self.manpower.values.capacity() * (size_of::<(u64, i32)>() + 1)
    }
}
