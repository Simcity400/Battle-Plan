//! What a ledger's totals count of its payments, by item and without content: the emit folds
//! each settled period into a rollup ([`super::emit`]) and a reply folds the hours no rollup
//! holds with the same code, so both count alike.

use dashboard_series::{Cell, codec::Decode, hash::FastMap};
use serde::{Deserialize, Serialize};
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
#[derive(replay_spool::Spooled)]
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
    /// Its payments are upkeep: its days count one entry.
    pub upkeep: bool,
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
            upkeep: contribution.upkeep,
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
#[derive(replay_spool::Spooled)]
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
#[derive(replay_spool::Spooled)]
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
#[derive(replay_spool::Spooled)]
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
        let day = Recurring::of(day);
        match &mut self.recurring {
            Some(held) => held.merge(&day),
            None => {
                self.recurring = Some(day);
                Ok(())
            }
        }
    }
}

/// What one day's daily row adds to a currency's gains ([`super::rows::gain`]): its gain, None
/// where it is not read, what the cap kept from it, and whether that currency was read.
pub type GainDay = (Option<i128>, u128, bool);

/// A currency's daily gains over some days.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
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
#[derive(replay_spool::Spooled)]
pub struct Totals {
    pub entries: FastMap<Field, Entry>,
    pub gains: [Gain; 2],
    /// Costs whose day's political application is not recorded.
    pub unmatched: u64,
    pub read: i32,
}

impl Totals {
    pub fn merge(&mut self, other: &Self) -> io::Result<()> {
        self.entries.reserve(other.entries.len());
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
        entries: FastMap::with_capacity_and_hasher(rows.len() / STRIDE, Default::default()),
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
                    upkeep: false,
                },
            });
        }
        totals.entries.insert(field, entry);
    }
    Ok(totals)
}

/// One branch's rollups in the making: each recurring item's open days, and each level's open
/// periods' totals.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct Rollups {
    pub days: FastMap<(i32, Field), Day>,
    /// Each open day's latest daily gains, by day and capture identity.
    pub daily: FastMap<(i32, u32, u64), [GainDay; 2]>,
    open: std::collections::BTreeMap<i32, Totals>,
    /// Upper levels change only as a period below closes into them, so they stay packed.
    upper: [std::collections::BTreeMap<i32, Packed>; 2],
}

/// Totals as saved ([`Out::totals`]); fields are ordered, so equal totals are equal bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
struct Packed(Box<[u8]>);

impl Packed {
    fn of(totals: &Totals) -> Self {
        let mut bytes = Vec::new();
        Out(&mut bytes).totals(totals);
        Self(bytes.into_boxed_slice())
    }

    fn totals(&self) -> io::Result<Totals> {
        let mut input = Decode::new(&self.0);
        let held = totals(&mut input)?;
        input.end()?;
        Ok(held)
    }

    fn merge_into(
        level: &mut std::collections::BTreeMap<i32, Self>,
        index: i32,
        more: &Totals,
    ) -> io::Result<()> {
        let mut held = match level.get(&index) {
            Some(packed) => packed.totals()?,
            None => Totals::default(),
        };
        held.merge(more)?;
        level.insert(index, Self::of(&held));
        Ok(())
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
        let totals = self.open.entry(index).or_default();
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
        let totals = self.open.entry(index).or_default();
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
        let Self {
            days, daily, open, ..
        } = self;
        let mut failed = None;
        days.retain(|&(at, field), day| {
            if at >= settled {
                return true;
            }
            let (index, _, _) = period(0, day.day.div_euclid(24));
            let totals = open.entry(index).or_default();
            totals.read = totals.read.max(day.day + day.lag);
            if let Err(error) = totals.entries.entry(field).or_default().add_day(day) {
                failed.get_or_insert(error);
            }
            false
        });
        if let Some(error) = failed.take() {
            return Err(error);
        }
        daily.retain(|&(at, _, _), gains| {
            if at >= settled {
                return true;
            }
            let (index, _, _) = period(0, at.div_euclid(24));
            let totals = open.entry(index).or_default();
            for (gain, day) in totals.gains.iter_mut().zip(*gains) {
                if let Err(error) = gain.add(day) {
                    failed.get_or_insert(error);
                }
            }
            false
        });
        if let Some(error) = failed.take() {
            return Err(error);
        }
        let mut closed = Vec::new();
        let settled_day = settled.div_euclid(24);
        let ended = |level: usize, index: i32| {
            index * PERIOD_DAYS[level] + PERIOD_DAYS[level] <= settled_day
        };
        let done: Vec<i32> = self.open.keys().copied().filter(|&index| ended(0, index)).collect();
        for index in done {
            let totals = self.open.remove(&index).expect("an open period");
            let (above, _, _) = period(1, index * PERIOD_DAYS[0]);
            Packed::merge_into(&mut self.upper[0], above, &totals)?;
            closed.push((0, index, totals));
        }
        for level in 1..3 {
            let (below, higher) = self.upper.split_at_mut(level);
            let held = &mut below[level - 1];
            let done: Vec<i32> = held.keys().copied().filter(|&index| ended(level, index)).collect();
            for index in done {
                let totals = held.remove(&index).expect("an open period").totals()?;
                if let Some(higher) = higher.first_mut() {
                    let (above, _, _) = period(level + 1, index * PERIOD_DAYS[level]);
                    Packed::merge_into(higher, above, &totals)?;
                }
                closed.push((level, index, totals));
            }
        }
        Ok(closed)
    }
}

impl Rollups {
    pub fn is_empty(&self) -> bool {
        self.days.is_empty()
            && self.daily.is_empty()
            && self.open.is_empty()
            && self.upper.iter().all(|upper| upper.is_empty())
    }

    /// The least settled frontier at which [`Self::settle`] would change anything.
    pub fn due(&self) -> Option<i32> {
        let days = self.days.keys().map(|&(at, _)| at.saturating_add(1));
        let daily = self.daily.keys().map(|&(at, _, _)| at.saturating_add(1));
        let first = |level: usize, index: Option<&i32>| {
            (index? + 1)
                .checked_mul(PERIOD_DAYS[level])
                .and_then(|day| day.checked_mul(24))
        };
        let periods = std::iter::once(first(0, self.open.keys().next()))
            .chain(
                self.upper
                    .iter()
                    .enumerate()
                    .map(|(level, upper)| first(level + 1, upper.keys().next())),
            )
            .flatten();
        days.chain(daily).chain(periods).min()
    }
}

/// The varints of a saved state, as [`Decode`] reads them back.
pub struct Out<'a>(pub &'a mut Vec<u8>);

impl Out<'_> {
    pub fn var128(&mut self, mut value: u128) {
        while value >= 0x80 {
            self.0.push(value as u8 | 0x80);
            value >>= 7;
        }
        self.0.push(value as u8);
    }

    pub fn var(&mut self, value: u64) {
        self.var128(u128::from(value));
    }

    pub fn zig128(&mut self, value: i128) {
        self.var128(((value << 1) ^ (value >> 127)) as u128);
    }

    pub fn int(&mut self, value: i32) {
        self.zig128(i128::from(value));
    }

    pub fn flag(&mut self, value: bool) {
        self.0.push(u8::from(value));
    }

    pub fn len(&mut self, len: usize) {
        self.var(len as u64);
    }

    fn field(&mut self, (a, b, c): Field) {
        self.var(u64::from(a));
        self.var(u64::from(b));
        self.var(c);
    }

    fn day(&mut self, day: &Day) {
        self.int(day.day);
        self.int(day.first.0);
        self.var(day.first.1);
        self.int(day.hour);
        self.var(day.sequence);
        self.var(u64::from(day.ordinal));
        self.int(day.lag);
        self.var(day.occurrences);
        self.var(day.entries);
        self.zig128(day.charge);
        self.var(day.known);
        self.flag(day.covered);
        self.zig128(day.realized.0);
        self.zig128(day.realized.1);
        self.flag(day.realized_unknown);
        self.flag(day.upkeep);
    }

    fn gain(&mut self, gain: &Gain) {
        self.zig128(gain.gained);
        self.var128(gain.lost);
        self.var(gain.incomplete);
        self.flag(gain.any);
    }

    fn gain_day(&mut self, (gained, lost, read): GainDay) {
        self.flag(gained.is_some());
        if let Some(gained) = gained {
            self.zig128(gained);
        }
        self.var128(lost);
        self.flag(read);
    }

    fn entry(&mut self, entry: &Entry) {
        self.flag(entry.one_off.is_some());
        if let Some(one_off) = &entry.one_off {
            self.zig128(one_off.known);
            self.var(one_off.occurrences);
            self.var(one_off.entries);
            self.var(one_off.unknown);
            self.var(one_off.uncovered);
            self.int(one_off.latest.0);
            self.var(one_off.latest.1);
            self.var(u64::from(one_off.latest.2));
            self.int(one_off.read);
        }
        self.flag(entry.recurring.is_some());
        if let Some(recurring) = &entry.recurring {
            self.var(recurring.days);
            self.var(recurring.occurrences);
            self.var(recurring.entries);
            self.zig128(recurring.charge);
            self.var(recurring.unknown);
            self.var(recurring.uncovered);
            self.zig128(recurring.realized.0);
            self.zig128(recurring.realized.1);
            self.flag(recurring.realized_unknown);
            self.int(recurring.first_day);
            self.day(&recurring.last);
        }
        self.var(entry.balances);
        self.var(entry.credits);
    }

    fn totals(&mut self, totals: &Totals) {
        let mut entries: Vec<(&Field, &Entry)> = totals.entries.iter().collect();
        entries.sort_unstable_by_key(|(field, _)| **field);
        self.len(entries.len());
        for (&field, entry) in entries {
            self.field(field);
            self.entry(entry);
        }
        for gain in &totals.gains {
            self.gain(gain);
        }
        self.var(totals.unmatched);
        self.int(totals.read);
    }
}

fn damaged() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "A saved politics ledger rollup is damaged")
}

pub fn int(input: &mut Decode<'_>) -> io::Result<i32> {
    i32::try_from(input.zig128()?).map_err(|_| damaged())
}

pub fn word(input: &mut Decode<'_>) -> io::Result<u32> {
    u32::try_from(input.var()?).map_err(|_| damaged())
}

pub fn len(input: &mut Decode<'_>) -> io::Result<usize> {
    usize::try_from(input.var()?).map_err(|_| damaged())
}

fn field(input: &mut Decode<'_>) -> io::Result<Field> {
    Ok((word(input)?, word(input)?, input.var()?))
}

fn day(input: &mut Decode<'_>) -> io::Result<Day> {
    Ok(Day {
        day: int(input)?,
        first: (int(input)?, input.var()?),
        hour: int(input)?,
        sequence: input.var()?,
        ordinal: word(input)?,
        lag: int(input)?,
        occurrences: input.var()?,
        entries: input.var()?,
        charge: input.zig128()?,
        known: input.var()?,
        covered: input.flag()?,
        realized: (input.zig128()?, input.zig128()?),
        realized_unknown: input.flag()?,
        upkeep: input.flag()?,
    })
}

fn gain(input: &mut Decode<'_>) -> io::Result<Gain> {
    Ok(Gain {
        gained: input.zig128()?,
        lost: input.var128()?,
        incomplete: input.var()?,
        any: input.flag()?,
    })
}

fn gain_day(input: &mut Decode<'_>) -> io::Result<GainDay> {
    let gained = if input.flag()? {
        Some(input.zig128()?)
    } else {
        None
    };
    Ok((gained, input.var128()?, input.flag()?))
}

fn entry(input: &mut Decode<'_>) -> io::Result<Entry> {
    let one_off = if input.flag()? {
        Some(OneOff {
            known: input.zig128()?,
            occurrences: input.var()?,
            entries: input.var()?,
            unknown: input.var()?,
            uncovered: input.var()?,
            latest: (int(input)?, input.var()?, word(input)?),
            read: int(input)?,
        })
    } else {
        None
    };
    let recurring = if input.flag()? {
        Some(Recurring {
            days: input.var()?,
            occurrences: input.var()?,
            entries: input.var()?,
            charge: input.zig128()?,
            unknown: input.var()?,
            uncovered: input.var()?,
            realized: (input.zig128()?, input.zig128()?),
            realized_unknown: input.flag()?,
            first_day: int(input)?,
            last: day(input)?,
        })
    } else {
        None
    };
    Ok(Entry {
        one_off,
        recurring,
        balances: input.var()?,
        credits: input.var()?,
    })
}

fn totals(input: &mut Decode<'_>) -> io::Result<Totals> {
    let mut totals = Totals::default();
    for _ in 0..len(input)? {
        let field = field(input)?;
        totals.entries.insert(field, entry(input)?);
    }
    totals.gains = [gain(input)?, gain(input)?];
    totals.unmatched = input.var()?;
    totals.read = int(input)?;
    Ok(totals)
}

impl Rollups {
    /// Its saved form, every map in key order: a long campaign's rollups are the bulk of the
    /// emit's saved state, which as JSON took several times the bytes and the time.
    pub fn save(&self, out: &mut Out<'_>) {
        let mut days: Vec<(&(i32, Field), &Day)> = self.days.iter().collect();
        days.sort_unstable_by_key(|(key, _)| **key);
        out.len(days.len());
        for ((_, field), day) in days {
            out.field(*field);
            out.day(day);
        }
        let mut daily: Vec<(&(i32, u32, u64), &[GainDay; 2])> = self.daily.iter().collect();
        daily.sort_unstable_by_key(|(key, _)| **key);
        out.len(daily.len());
        for (&(day, raw, epoch), gains) in daily {
            out.int(day);
            out.var(u64::from(raw));
            out.var(epoch);
            for &gain in gains {
                out.gain_day(gain);
            }
        }
        out.len(self.open.len());
        for (&index, totals) in &self.open {
            out.int(index);
            out.totals(totals);
        }
        for upper in &self.upper {
            out.len(upper.len());
            for (&index, packed) in upper {
                out.int(index);
                out.0.extend_from_slice(&packed.0);
            }
        }
    }

    /// The rollups [`Self::save`] saved.
    pub fn load(input: &mut Decode<'_>) -> io::Result<Self> {
        let mut rollups = Self::default();
        for _ in 0..len(input)? {
            let field = field(input)?;
            let day = day(input)?;
            rollups.days.insert((day.day, field), day);
        }
        for _ in 0..len(input)? {
            let key = (int(input)?, word(input)?, input.var()?);
            rollups.daily.insert(key, [gain_day(input)?, gain_day(input)?]);
        }
        for _ in 0..len(input)? {
            let index = int(input)?;
            rollups.open.insert(index, totals(input)?);
        }
        for upper in &mut rollups.upper {
            for _ in 0..len(input)? {
                let index = int(input)?;
                upper.insert(index, Packed::of(&totals(input)?));
            }
        }
        Ok(rollups)
    }
}
