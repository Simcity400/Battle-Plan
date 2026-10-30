//! What a ledger's totals count of its payments, by item and without content: the emit folds
//! each settled period into a rollup ([`super::emit`]) and a reply folds the hours no rollup
//! holds with the same code, so both count alike.

use dashboard_series::Cell;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io;

/// Days per rollup period, by level: each level's period is sixteen of the level below.
pub const PERIOD_DAYS: [i32; 3] = [32, 512, 8192];

/// The `level` period holding native `day`, and its first and last day.
pub fn period(level: usize, day: i32) -> (i32, i32, i32) {
    let size = PERIOD_DAYS[level];
    let index = day.div_euclid(size);
    (index, index * size, index * size + size - 1)
}

/// The item a payment counts under, of the capture identity that recorded it.
pub type Field = (u32, u32, u64);

/// What totals count of one recorded payment.
#[derive(Clone, Copy, Debug)]
pub struct Contribution {
    pub field: Field,
    pub recurring: bool,
    /// Whether its days count one entry however many payments they merge.
    pub upkeep: bool,
    pub hour: i32,
    pub read: i32,
    /// The order it was recorded in: read hour, then its event sequence.
    pub order: (i32, u64),
    pub sequence: u64,
    pub ordinal: u32,
    /// None when the payment's charge is not recorded.
    pub charge: Option<i128>,
    pub covered: bool,
    pub realized: Option<i128>,
}

/// One recurring item's native day: its payments merged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Day {
    pub day: i32,
    pub first: (i32, u64),
    pub hour: i32,
    pub sequence: u64,
    pub ordinal: u32,
    pub lag: i32,
    pub occurrences: u64,
    pub entries: u64,
    pub charge: i128,
    pub known: u64,
    pub covered: bool,
    pub realized: (i128, i128),
    pub realized_unknown: bool,
}

fn overflow() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Politics subtotal overflow")
}

fn add(left: i128, right: i128) -> io::Result<i128> {
    left.checked_add(right).ok_or_else(overflow)
}

impl Day {
    pub fn new(contribution: &Contribution) -> Self {
        let day = contribution.hour.div_euclid(24) * 24;
        Self {
            day,
            first: contribution.order,
            hour: contribution.hour,
            sequence: contribution.sequence,
            ordinal: contribution.ordinal,
            lag: contribution.read - day,
            occurrences: 1,
            entries: 1,
            charge: contribution.charge.unwrap_or(0),
            known: u64::from(contribution.charge.is_some()),
            covered: contribution.covered,
            realized: contribution
                .realized
                .map_or((0, 0), |value| (value.max(0), value.min(0))),
            realized_unknown: contribution.realized.is_none(),
        }
    }

    /// Another payment of the same item and day.
    pub fn add(&mut self, contribution: &Contribution) -> io::Result<()> {
        if contribution.order < self.first {
            self.first = contribution.order;
            self.sequence = contribution.sequence;
            self.ordinal = contribution.ordinal;
        }
        self.hour = self.hour.min(contribution.hour);
        self.lag = self.lag.max(contribution.read - self.day);
        self.occurrences += 1;
        if !contribution.upkeep {
            self.entries += 1;
        }
        self.charge = add(self.charge, contribution.charge.unwrap_or(0))?;
        self.known += u64::from(contribution.charge.is_some());
        self.covered &= contribution.covered;
        match contribution.realized {
            Some(value) => {
                self.realized.0 = add(self.realized.0, value.max(0))?;
                self.realized.1 = add(self.realized.1, value.min(0))?;
            }
            None => self.realized_unknown = true,
        }
        Ok(())
    }
}

/// An item's one-off payments.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OneOff {
    pub known: i128,
    pub occurrences: u64,
    pub entries: u64,
    pub unknown: u64,
    pub uncovered: u64,
    /// The latest payment by native hour, sequence and ordinal, and its read.
    pub latest: (i32, u64, u32),
    pub read: i32,
}

impl OneOff {
    pub fn add(&mut self, contribution: &Contribution) -> io::Result<()> {
        let latest = (
            contribution.hour,
            contribution.sequence,
            contribution.ordinal,
        );
        self.merge(&Self {
            known: contribution.charge.unwrap_or(0),
            occurrences: 1,
            entries: 1,
            unknown: u64::from(contribution.charge.is_none()),
            uncovered: u64::from(!contribution.covered),
            latest,
            read: contribution.read,
        })
    }

    pub fn merge(&mut self, other: &Self) -> io::Result<()> {
        if self.occurrences == 0 || other.latest >= self.latest {
            self.latest = other.latest;
            self.read = other.read;
        }
        self.known = add(self.known, other.known)?;
        self.occurrences += other.occurrences;
        self.entries += other.entries;
        self.unknown += other.unknown;
        self.uncovered += other.uncovered;
        Ok(())
    }
}

/// An item's settled days.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recurring {
    pub days: u64,
    pub occurrences: u64,
    pub entries: u64,
    pub charge: i128,
    pub unknown: u64,
    pub uncovered: u64,
    pub realized: (i128, i128),
    pub realized_unknown: bool,
    pub first_day: i32,
    /// The latest day, which names the subtotal.
    pub last: Day,
}

impl Recurring {
    pub fn of(day: &Day) -> Self {
        Self {
            days: 1,
            occurrences: day.occurrences,
            entries: day.entries,
            charge: day.charge,
            unknown: day.occurrences - day.known,
            uncovered: u64::from(!day.covered),
            realized: day.realized,
            realized_unknown: day.realized_unknown,
            first_day: day.day,
            last: *day,
        }
    }

    pub fn merge(&mut self, other: &Self) -> io::Result<()> {
        self.days += other.days;
        self.occurrences += other.occurrences;
        self.entries += other.entries;
        self.charge = add(self.charge, other.charge)?;
        self.unknown += other.unknown;
        self.uncovered += other.uncovered;
        self.realized.0 = add(self.realized.0, other.realized.0)?;
        self.realized.1 = add(self.realized.1, other.realized.1)?;
        self.realized_unknown |= other.realized_unknown;
        self.first_day = self.first_day.min(other.first_day);
        if other.last.day >= self.last.day {
            self.last = other.last;
        }
        Ok(())
    }
}

/// One item's totals over some hours, with its rows that are no payment: balance lines and
/// negative cost credits, which replies count among their warnings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub one_off: Option<OneOff>,
    pub recurring: Option<Recurring>,
    pub balances: u64,
    pub credits: u64,
}

impl Entry {
    pub fn merge(&mut self, other: &Self) -> io::Result<()> {
        if let Some(more) = &other.one_off {
            match &mut self.one_off {
                Some(held) => held.merge(more)?,
                None => self.one_off = Some(*more),
            }
        }
        if let Some(more) = &other.recurring {
            match &mut self.recurring {
                Some(held) => held.merge(more)?,
                None => self.recurring = Some(*more),
            }
        }
        self.balances += other.balances;
        self.credits += other.credits;
        Ok(())
    }

    pub fn add_day(&mut self, day: &Day) -> io::Result<()> {
        self.merge(&Self {
            recurring: Some(Recurring::of(day)),
            ..Self::default()
        })
    }
}

/// What one day's daily row adds to a currency's gains ([`super::rows::gain`]): its gain, None
/// where it is not read, what the cap kept from it, and whether that currency was read.
pub type GainDay = (Option<i128>, u128, bool);

/// A currency's daily gains over some days.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gain {
    pub gained: i128,
    pub lost: u128,
    pub incomplete: u64,
    pub any: bool,
}

impl Gain {
    pub fn add(&mut self, (gained, lost, read): GainDay) -> io::Result<()> {
        self.any |= read;
        match gained {
            Some(gained) => self.gained = add(self.gained, gained)?,
            None => self.incomplete += 1,
        }
        self.lost = self.lost.checked_add(lost).ok_or_else(overflow)?;
        Ok(())
    }

    pub fn merge(&mut self, other: &Self) -> io::Result<()> {
        self.gained = add(self.gained, other.gained)?;
        self.lost = self.lost.checked_add(other.lost).ok_or_else(overflow)?;
        self.incomplete += other.incomplete;
        self.any |= other.any;
        Ok(())
    }
}

/// Every item's totals over one period, its daily gains by currency, and the latest read they
/// count.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    pub entries: HashMap<Field, Entry>,
    pub gains: [Gain; 2],
    /// Costs whose day's political application is not recorded.
    pub unmatched: u64,
    pub read: i32,
}

impl Totals {
    pub fn merge(&mut self, other: &Self) -> io::Result<()> {
        for (field, entry) in &other.entries {
            self.entries.entry(*field).or_default().merge(entry)?;
        }
        for (gain, more) in self.gains.iter_mut().zip(&other.gains) {
            gain.merge(more)?;
        }
        self.unmatched += other.unmatched;
        self.read = self.read.max(other.read);
        Ok(())
    }
}

const STRIDE: usize = 31;

fn integer(cell: &Cell) -> io::Result<i128> {
    match cell {
        Cell::Integer(value) => Ok(*value),
        Cell::Unsigned(value) => Ok(i128::from(*value)),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "A politics rollup is malformed",
        )),
    }
}

fn narrow<T: TryFrom<i128>>(cell: &Cell) -> io::Result<T> {
    T::try_from(integer(cell)?).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "A politics rollup is out of range",
        )
    })
}

/// Cells before the items: the read, each currency's gains, the unmatched costs.
const HEAD: usize = 10;

/// `totals` as one flat list of cells, `STRIDE` per item after the head.
pub fn cells(totals: &Totals) -> Vec<Cell> {
    let mut cells = Vec::with_capacity(HEAD + totals.entries.len() * STRIDE);
    cells.push(Cell::Integer(i128::from(totals.read)));
    for gain in &totals.gains {
        cells.push(Cell::Integer(gain.gained));
        cells.push(Cell::Integer(gain.lost as i128));
        cells.push(Cell::Unsigned(gain.incomplete));
        cells.push(Cell::Boolean(gain.any));
    }
    cells.push(Cell::Unsigned(totals.unmatched));
    let mut fields: Vec<&Field> = totals.entries.keys().collect();
    fields.sort_unstable();
    for field in fields {
        let entry = &totals.entries[field];
        cells.push(Cell::Unsigned(u64::from(field.0)));
        cells.push(Cell::Unsigned(u64::from(field.1)));
        cells.push(Cell::Unsigned(field.2));
        let one = entry.one_off.unwrap_or_default();
        cells.push(Cell::Boolean(entry.one_off.is_some()));
        cells.push(Cell::Integer(one.known));
        cells.push(Cell::Unsigned(one.occurrences));
        cells.push(Cell::Unsigned(one.entries));
        cells.push(Cell::Unsigned(one.unknown));
        cells.push(Cell::Unsigned(one.uncovered));
        cells.push(Cell::Integer(i128::from(one.latest.0)));
        cells.push(Cell::Unsigned(one.latest.1));
        cells.push(Cell::Unsigned(u64::from(one.latest.2)));
        cells.push(Cell::Integer(i128::from(one.read)));
        match &entry.recurring {
            Some(recurring) => {
                cells.push(Cell::Boolean(true));
                cells.push(Cell::Unsigned(recurring.days));
                cells.push(Cell::Unsigned(recurring.occurrences));
                cells.push(Cell::Unsigned(recurring.entries));
                cells.push(Cell::Integer(recurring.charge));
                cells.push(Cell::Unsigned(recurring.unknown));
                cells.push(Cell::Unsigned(recurring.uncovered));
                cells.push(Cell::Integer(recurring.realized.0));
                cells.push(Cell::Integer(recurring.realized.1));
                cells.push(Cell::Boolean(recurring.realized_unknown));
                cells.push(Cell::Integer(i128::from(recurring.first_day)));
                let last = &recurring.last;
                cells.push(Cell::Integer(i128::from(last.day)));
                cells.push(Cell::Integer(i128::from(last.lag)));
                cells.push(Cell::Integer(i128::from(last.hour)));
                cells.push(Cell::Unsigned(last.sequence));
                cells.push(Cell::Unsigned(u64::from(last.ordinal)));
            }
            None => cells.extend(std::iter::repeat_n(Cell::Null, 16)),
        }
        cells.push(Cell::Unsigned(entry.balances));
        cells.push(Cell::Unsigned(entry.credits));
    }
    cells
}

/// The totals [`cells`] wrote.
pub fn read(cells: &[Cell]) -> io::Result<Totals> {
    let malformed = || io::Error::new(io::ErrorKind::InvalidData, "A politics rollup is malformed");
    if cells.len() < HEAD || (cells.len() - HEAD) % STRIDE != 0 {
        return Err(malformed());
    }
    let (head, rows) = cells.split_at(HEAD);
    let gain = |at: usize| -> io::Result<Gain> {
        Ok(Gain {
            gained: integer(&head[at])?,
            lost: narrow(&head[at + 1])?,
            incomplete: narrow(&head[at + 2])?,
            any: matches!(head[at + 3], Cell::Boolean(true)),
        })
    };
    let mut totals = Totals {
        entries: HashMap::with_capacity(rows.len() / STRIDE),
        gains: [gain(1)?, gain(5)?],
        unmatched: narrow(&head[9])?,
        read: narrow(&head[0])?,
    };
    for row in rows.chunks_exact(STRIDE) {
        let field = (narrow(&row[0])?, narrow(&row[1])?, narrow(&row[2])?);
        let mut entry = Entry {
            balances: narrow(&row[29])?,
            credits: narrow(&row[30])?,
            ..Entry::default()
        };
        if matches!(row[3], Cell::Boolean(true)) {
            entry.one_off = Some(OneOff {
                known: integer(&row[4])?,
                occurrences: narrow(&row[5])?,
                entries: narrow(&row[6])?,
                unknown: narrow(&row[7])?,
                uncovered: narrow(&row[8])?,
                latest: (narrow(&row[9])?, narrow(&row[10])?, narrow(&row[11])?),
                read: narrow(&row[12])?,
            });
        }
        if matches!(row[13], Cell::Boolean(true)) {
            let day: i32 = narrow(&row[24])?;
            entry.recurring = Some(Recurring {
                days: narrow(&row[14])?,
                occurrences: narrow(&row[15])?,
                entries: narrow(&row[16])?,
                charge: integer(&row[17])?,
                unknown: narrow(&row[18])?,
                uncovered: narrow(&row[19])?,
                realized: (integer(&row[20])?, integer(&row[21])?),
                realized_unknown: matches!(row[22], Cell::Boolean(true)),
                first_day: narrow(&row[23])?,
                last: Day {
                    day,
                    first: (0, 0),
                    hour: narrow(&row[26])?,
                    sequence: narrow(&row[27])?,
                    ordinal: narrow(&row[28])?,
                    lag: narrow(&row[25])?,
                    occurrences: 0,
                    entries: 0,
                    charge: 0,
                    known: 0,
                    covered: true,
                    realized: (0, 0),
                    realized_unknown: false,
                },
            });
        }
        totals.entries.insert(field, entry);
    }
    Ok(totals)
}

/// One branch's rollups in the making: each recurring item's open days, and each level's open
/// periods' totals.
#[derive(Clone, Debug, Default)]
pub struct Rollups {
    pub days: HashMap<(i32, Field), Day>,
    /// Each open day's latest daily gains, by day and capture identity.
    pub daily: HashMap<(i32, u32, u64), [GainDay; 2]>,
    pub open: [std::collections::BTreeMap<i32, Totals>; 3],
}

/// [`Rollups`] as the emit's saved state holds it.
#[derive(Default, Serialize, Deserialize)]
pub struct SavedRollups {
    days: Vec<Day>,
    fields: Vec<Field>,
    daily: Vec<((i32, u32, u64), [GainDay; 2])>,
    open: Vec<(u8, i32, i32, [Gain; 2], u64, Vec<(Field, Entry)>)>,
}

impl From<&Rollups> for SavedRollups {
    fn from(rollups: &Rollups) -> Self {
        let mut saved = Self::default();
        for ((_, field), day) in &rollups.days {
            saved.days.push(*day);
            saved.fields.push(*field);
        }
        saved.daily = rollups.daily.iter().map(|(k, v)| (*k, *v)).collect();
        for (level, open) in rollups.open.iter().enumerate() {
            for (&index, totals) in open {
                saved.open.push((
                    level as u8,
                    index,
                    totals.read,
                    totals.gains,
                    totals.unmatched,
                    totals.entries.iter().map(|(f, e)| (*f, *e)).collect(),
                ));
            }
        }
        saved
    }
}

impl From<SavedRollups> for Rollups {
    fn from(saved: SavedRollups) -> Self {
        let mut rollups = Self::default();
        for (day, field) in saved.days.into_iter().zip(saved.fields) {
            rollups.days.insert((day.day, field), day);
        }
        rollups.daily = saved.daily.into_iter().collect();
        for (level, index, read, gains, unmatched, entries) in saved.open {
            if let Some(open) = rollups.open.get_mut(usize::from(level)) {
                open.insert(
                    index,
                    Totals {
                        entries: entries.into_iter().collect(),
                        gains,
                        unmatched,
                        read,
                    },
                );
            }
        }
        rollups
    }
}

impl Rollups {
    /// A payment no settled day holds yet, as the emit records it.
    pub fn add(&mut self, contribution: &Contribution) -> io::Result<()> {
        if contribution.recurring {
            let day = contribution.hour.div_euclid(24) * 24;
            match self.days.get_mut(&(day, contribution.field)) {
                Some(held) => held.add(contribution)?,
                None => {
                    self.days
                        .insert((day, contribution.field), Day::new(contribution));
                }
            }
            return Ok(());
        }
        let (index, _, _) = period(0, contribution.hour.div_euclid(24));
        let totals = self.open[0].entry(index).or_default();
        totals.read = totals.read.max(contribution.read);
        totals
            .entries
            .entry(contribution.field)
            .or_default()
            .one_off
            .get_or_insert_with(OneOff::default)
            .add(contribution)
    }

    /// A row of `field` dated `hour` that is no payment: a balance line, a credit, or (with no
    /// field) a cost whose application is not recorded.
    pub fn count(&mut self, field: Option<Field>, hour: i32, credit: bool) {
        let (index, _, _) = period(0, hour.div_euclid(24));
        let totals = self.open[0].entry(index).or_default();
        match field {
            Some(field) => {
                let entry = totals.entries.entry(field).or_default();
                if credit {
                    entry.credits += 1;
                } else {
                    entry.balances += 1;
                }
            }
            None => totals.unmatched += 1,
        }
    }

    /// A day's daily row no settled day holds yet, replacing its earlier revision.
    pub fn add_daily(&mut self, day: (i32, u32, u64), gains: [GainDay; 2]) {
        self.daily.insert(day, gains);
    }

    /// Every period whose days are all settled before `settled`, lowest level first, with its
    /// level, index and totals: each folded into the level above it.
    pub fn settle(&mut self, settled: i32) -> io::Result<Vec<(usize, i32, Totals)>> {
        let ready: Vec<(i32, Field)> = self
            .days
            .keys()
            .filter(|(day, _)| *day < settled)
            .copied()
            .collect();
        for key in ready {
            let day = self.days.remove(&key).expect("a settled day");
            let (index, _, _) = period(0, day.day.div_euclid(24));
            let totals = self.open[0].entry(index).or_default();
            totals.read = totals.read.max(day.day + day.lag);
            totals.entries.entry(key.1).or_default().add_day(&day)?;
        }
        let ready: Vec<(i32, u32, u64)> = self
            .daily
            .keys()
            .filter(|(day, _, _)| *day < settled)
            .copied()
            .collect();
        for key in ready {
            let gains = self.daily.remove(&key).expect("a settled day");
            let (index, _, _) = period(0, key.0.div_euclid(24));
            let totals = self.open[0].entry(index).or_default();
            for (gain, day) in totals.gains.iter_mut().zip(gains) {
                gain.add(day)?;
            }
        }
        let mut closed = Vec::new();
        let settled_day = settled.div_euclid(24);
        for level in 0..3 {
            let done: Vec<i32> = self.open[level]
                .keys()
                .copied()
                .filter(|&index| index * PERIOD_DAYS[level] + PERIOD_DAYS[level] <= settled_day)
                .collect();
            for index in done {
                let totals = self.open[level].remove(&index).expect("an open period");
                if level + 1 < 3 {
                    let (above, _, _) = period(level + 1, index * PERIOD_DAYS[level]);
                    self.open[level + 1]
                        .entry(above)
                        .or_default()
                        .merge(&totals)?;
                }
                closed.push((level, index, totals));
            }
        }
        Ok(closed)
    }
}

impl Serialize for Rollups {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        SavedRollups::from(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Rollups {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        SavedRollups::deserialize(deserializer).map(Self::from)
    }
}
