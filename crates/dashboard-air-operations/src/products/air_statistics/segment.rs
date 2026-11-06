//! Air statistics on a native recording, a segment at a time ([`dashboard_keyed::segment`]). At
//! each hour a field it reads changed, a segment's reader observes the hour as the family does:
//! each row's country named `TAG@slot` ([`Countries`]) and held to its binding at the
//! statistics' supplying read, and each unit's (a country's name and a region's) [`FIELDS`]
//! values. It hands on only what the hour observed otherwise than the segment's observation
//! before, every unit at the segment's first, and its verdict. Before the segment's first read
//! it does not hold the bindings: it hands on each row country's name for the branch's emit to
//! hold to them ([`Verdict::Held`]).

use dashboard_base::prepared::frame_changes::FrameChanges;
use dashboard_keyed::{
    countries::Countries,
    segment::{Reader, Segmented, Touched},
};
use replay::container::TableFields;
use replay_data::{
    format::FormatVersion,
    rows::{AirStatisticsRow, TableId, tables},
};
use replay_frame::frame::DecodedFrame;
use std::{collections::HashMap, io, mem::size_of};

/// The tables an observation reads.
pub const TABLES: &[TableId] = &[
    TableId::AirStatistics,
    TableId::CountryLinks,
    TableId::Countries,
];
/// A unit's values: its 24 raw signed channels, then its ring slot and whether the ring wrapped.
pub const FIELDS: usize = 26;
pub const SLOT: usize = 24;
pub const WRAPPED: usize = 25;
/// Every field of a unit.
pub(super) const ALL: u32 = (1 << FIELDS) - 1;

pub(super) const UNIDENTIFIED: &str = "Air-statistics country identities are not recorded";
pub(super) const SEPARATOR: &str = "Air-statistics tag contains the series separator";
pub(super) const UNBOUND: &str =
    "Air-statistics country bindings at the supplying read are not recorded";

pub(super) fn unresolved(raw: u32) -> String {
    format!("Air-statistics country {raw} is unresolved")
}

pub(super) fn mismatched(raw: u32) -> String {
    format!(
        "Air-statistics country {raw} does not match its supplying-read binding at this hour"
    )
}

/// Air statistics, a segment at a time.
pub struct Segments;

impl Segmented for Segments {
    const TABLES: &'static [TableId] = TABLES;
    const READ: TableId = TableId::AirStatistics;
    type Read = Facts;

    fn fields() -> io::Result<Vec<TableFields>> {
        Ok(vec![
            TableFields::all(TableId::AirStatistics).when_read(),
            TableFields::all(TableId::CountryLinks),
            TableFields::named(TableId::Countries, &["canonical_slot", "tag"])?,
        ])
    }

    fn changed(changes: &FrameChanges) -> bool {
        changes.slot_read[Self::READ.index()]
            || TABLES
                .iter()
                .any(|table| changes.rows_changed[table.index()])
    }

    fn reader(_: FormatVersion) -> Box<dyn Reader<Self>> {
        Box::<Observe>::default()
    }
}

/// What a segment's reader observed of an hour.
#[derive(replay_spool::Spooled)]
pub struct Facts {
    /// The segment's first hour: its names and units are numbered anew from here.
    pub(super) fresh: bool,
    pub(super) verdict: Verdict,
    /// The country names it numbered first, in order.
    pub(super) names: Vec<Box<str>>,
    /// The units it numbered first, in order: each by its name's number and its region.
    pub(super) units: Vec<(u32, u32)>,
    /// What it observed of the units' values, where every row's country is named.
    pub(super) values: Option<Values>,
    /// The bindings an accepted read froze.
    pub(super) bindings: Option<Bindings>,
}

#[derive(replay_spool::Spooled)]
pub(super) enum Verdict {
    Accepted,
    Refused(Box<str>),
    /// Each row country and its name now, in row order, to hold to the bindings of a read
    /// before the segment's.
    Held(Box<[(u32, Option<u32>)]>),
}

/// What the units' values hold otherwise than the segment's observation before: those whose
/// values changed, with the fields they did and those fields' values packed in order; those
/// new, or whose first row is another, with its key; and those gone. Whole: every unit, with
/// every field, the units it misses gone.
#[derive(Default)]
#[derive(replay_spool::Spooled)]
pub(super) struct Values {
    pub(super) whole: bool,
    pub(super) changed: Vec<(u32, u32)>,
    pub(super) packed: Vec<i64>,
    pub(super) placed: Vec<(u32, u64)>,
    pub(super) removed: Vec<u32>,
}

/// The countries a read bound, by raw reference: in place of the segment's read before's, or
/// as changes to them.
#[derive(replay_spool::Spooled)]
pub(super) struct Bindings {
    pub(super) replace: bool,
    pub(super) set: Vec<(u32, u32)>,
    pub(super) removed: Vec<u32>,
}

impl Facts {
    fn bytes(&self) -> usize {
        let mut bytes = size_of::<Self>()
            + self.names.capacity() * size_of::<Box<str>>()
            + self.names.iter().map(|name| name.len()).sum::<usize>()
            + self.units.capacity() * size_of::<(u32, u32)>();
        bytes += match &self.verdict {
            Verdict::Accepted => 0,
            Verdict::Refused(why) => why.len(),
            Verdict::Held(names) => names.len() * size_of::<(u32, Option<u32>)>(),
        };
        if let Some(values) = &self.values {
            bytes += values.changed.capacity() * size_of::<(u32, u32)>()
                + values.packed.capacity() * size_of::<i64>()
                + values.placed.capacity() * size_of::<(u32, u64)>()
                + values.removed.capacity() * size_of::<u32>();
        }
        if let Some(bindings) = &self.bindings {
            bytes += bindings.set.capacity() * size_of::<(u32, u32)>()
                + bindings.removed.capacity() * size_of::<u32>();
        }
        bytes
    }
}

/// A unit's values as the segment last observed them.
#[derive(Clone, Copy)]
struct Held {
    present: bool,
    /// The last observation that named it, and its place there.
    seen: u32,
    slot: u32,
    position: u64,
    values: [i64; FIELDS],
}

impl Default for Held {
    fn default() -> Self {
        Self {
            present: false,
            seen: 0,
            slot: 0,
            position: 0,
            values: [0; FIELDS],
        }
    }
}

/// A segment's reader.
#[derive(Default)]
struct Observe {
    started: bool,
    names: Vec<Box<str>>,
    name_ids: HashMap<Box<str>, u32>,
    /// How many names and units it handed on.
    named: usize,
    united: usize,
    units: Vec<(u32, u32)>,
    unit_ids: HashMap<(u32, u32), u32>,
    countries: Option<Countries>,
    /// Each raw country's name by `countries`, looked up once.
    raws: HashMap<u32, Option<u32>>,
    /// Each row country and its name, in row order, as of the hour last read.
    order: Vec<(u32, Option<u32>)>,
    /// Whether that hour named every row's country.
    complete: bool,
    held: Vec<Held>,
    present: Vec<u32>,
    /// Whether it observed values in the segment.
    observed: bool,
    /// The bindings, once the segment read: None where its read refused them.
    bindings: Option<Option<HashMap<u32, u32>>>,
    generation: u32,
    next: Vec<(u32, u64, [i64; FIELDS])>,
}

fn row_values(row: &AirStatisticsRow) -> [i64; FIELDS] {
    let mut values = [0; FIELDS];
    values[..24].copy_from_slice(&row.channels);
    values[SLOT] = i64::from(row.year_slot);
    values[WRAPPED] = i64::from(row.year_window_wrapped);
    values
}

fn number<K: std::hash::Hash + Eq>(ids: &mut HashMap<K, u32>, count: usize, key: K) -> (u32, bool) {
    let next = count as u32;
    let id = *ids.entry(key).or_insert(next);
    (id, id == next)
}

impl Observe {
    fn name(&mut self, name: String) -> u32 {
        let name = name.into_boxed_str();
        let (id, new) = number(&mut self.name_ids, self.names.len(), name.clone());
        if new {
            self.names.push(name);
        }
        id
    }

    fn unit(&mut self, name: u32, region: u32) -> u32 {
        let (id, new) = number(&mut self.unit_ids, self.units.len(), (name, region));
        if new {
            self.units.push((name, region));
            self.held.push(Held::default());
        }
        id
    }

    /// Each row country and its name now, in row order.
    fn name_rows(&mut self, rows: &[AirStatisticsRow]) {
        self.order.clear();
        let Some(countries) = self.countries.take() else {
            return;
        };
        let mut last = None;
        for row in rows {
            if last == Some(row.country) {
                continue;
            }
            last = Some(row.country);
            let name = match self.raws.get(&row.country) {
                Some(&name) => name,
                None => {
                    let name = countries.raw(row.country).map(|name| self.name(name));
                    self.raws.insert(row.country, name);
                    name
                }
            };
            self.order.push((row.country, name));
        }
        self.countries = Some(countries);
    }

    /// The units' values of `rows`, every row's country named, as changes to the last held.
    fn values(&mut self, rows: &[AirStatisticsRow]) -> Values {
        self.generation += 1;
        let generation = self.generation;
        self.next.clear();
        // `order` holds each run of a country's rows once, in row order.
        let mut at = 0;
        let mut current: Option<(u32, u32)> = None;
        for row in rows {
            let name = match current {
                Some((country, name)) if country == row.country => name,
                _ => {
                    let (country, name) = self.order[at];
                    at += 1;
                    debug_assert_eq!(country, row.country);
                    let name = name.expect("every row's country is named");
                    current = Some((row.country, name));
                    name
                }
            };
            let unit = self.unit(name, row.region);
            let values = row_values(row);
            let held = &mut self.held[unit as usize];
            // A name twice keeps the last row's values at the first's place.
            if held.seen == generation {
                self.next[held.slot as usize].2 = values;
            } else {
                held.seen = generation;
                held.slot = self.next.len() as u32;
                let position = (u64::from(row.country) << 32) | u64::from(row.region);
                self.next.push((unit, position, values));
            }
        }
        let whole = !self.observed;
        self.observed = true;
        let mut out = Values {
            whole,
            ..Values::default()
        };
        for &(unit, position, values) in &self.next {
            let held = &mut self.held[unit as usize];
            let new = whole || !held.present;
            if new || held.position != position {
                out.placed.push((unit, position));
            }
            let mut mask = 0;
            for field in 0..FIELDS {
                if new || held.values[field] != values[field] {
                    mask |= 1 << field;
                    out.packed.push(values[field]);
                }
            }
            if mask != 0 {
                out.changed.push((unit, mask));
            }
            held.position = position;
            held.values = values;
        }
        for &unit in &self.present {
            let held = &mut self.held[unit as usize];
            if !whole && held.seen != generation {
                out.removed.push(unit);
            }
            held.present = false;
        }
        self.present.clear();
        for &(unit, ..) in &self.next {
            self.held[unit as usize].present = true;
            self.present.push(unit);
        }
        out
    }

    /// The verdict of the hour, `read` its statistics' supplying read, and the bindings it froze.
    fn verdict(
        &mut self,
        read: bool,
        rows: &io::Result<&[AirStatisticsRow]>,
    ) -> (Verdict, Option<Bindings>) {
        // A read frees the bindings before it binds anew; a refused one leaves none.
        let previous = if read {
            self.bindings.replace(None).flatten()
        } else {
            None
        };
        let refused = |why: String| (Verdict::Refused(why.into_boxed_str()), None);
        if self.countries.is_none() {
            return refused(UNIDENTIFIED.into());
        }
        if let Err(error) = rows {
            return refused(error.to_string());
        }
        if read {
            let mut bound = HashMap::with_capacity(self.order.len());
            for &(raw, name) in &self.order {
                let Some(name) = name else {
                    return refused(unresolved(raw));
                };
                if self.names[name as usize].contains('|') {
                    return refused(SEPARATOR.into());
                }
                bound.insert(raw, name);
            }
            let bindings = match &previous {
                Some(previous) => Bindings {
                    replace: false,
                    set: bound
                        .iter()
                        .filter(|&(raw, name)| previous.get(raw) != Some(name))
                        .map(|(&raw, &name)| (raw, name))
                        .collect(),
                    removed: previous
                        .keys()
                        .filter(|raw| !bound.contains_key(*raw))
                        .copied()
                        .collect(),
                },
                None => Bindings {
                    replace: true,
                    set: bound.iter().map(|(&raw, &name)| (raw, name)).collect(),
                    removed: Vec::new(),
                },
            };
            self.bindings = Some(Some(bound));
            return (Verdict::Accepted, Some(bindings));
        }
        match &self.bindings {
            None => (Verdict::Held(self.order.clone().into_boxed_slice()), None),
            Some(None) => refused(UNBOUND.into()),
            Some(Some(bound)) => {
                for &(raw, name) in &self.order {
                    if name.is_none() || bound.get(&raw).copied() != name {
                        return refused(mismatched(raw));
                    }
                }
                (Verdict::Accepted, None)
            }
        }
    }
}

impl Reader<Segments> for Observe {
    fn read(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        touched: &Touched,
    ) -> io::Result<(Facts, usize)> {
        let fresh = !self.started;
        self.started = true;
        let read = changes.slot_read[TableId::AirStatistics.index()];
        let renamed =
            touched.of(TableId::Countries).any() || touched.of(TableId::CountryLinks).any();
        if fresh || renamed {
            self.countries = Countries::read(frame);
            self.raws.clear();
        }
        let rows = frame.complete::<tables::AirStatistics>();
        let values = if fresh || renamed || touched.of(TableId::AirStatistics).any() {
            let all: &[AirStatisticsRow] = match &rows {
                Ok(rows) => *rows,
                Err(_) => &[],
            };
            self.name_rows(all);
            self.complete = self.countries.is_some()
                && rows.is_ok()
                && self.order.iter().all(|&(_, name)| name.is_some());
            self.complete.then(|| self.values(all))
        } else {
            // Nothing it reads moved: what it held holds.
            self.complete.then(Values::default)
        };
        let (verdict, bindings) = self.verdict(read, &rows);
        let facts = Facts {
            fresh,
            verdict,
            names: self.names[self.named..].to_vec(),
            units: self.units[self.united..].to_vec(),
            values,
            bindings,
        };
        self.named = self.names.len();
        self.united = self.units.len();
        let bytes = facts.bytes();
        Ok((facts, bytes))
    }
}
