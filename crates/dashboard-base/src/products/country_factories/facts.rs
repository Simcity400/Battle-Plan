//! Factory facts, projected hour by hour from a columnar walk ([`replay::segments`]): at each
//! read of a complete CountryFactories table, the values of every holder slot whose rows changed
//! since its last read in the segment (every slot's after a whole hour), with the read's counts
//! where they changed; and the tags the walk's countries resolve ([`dashboard_world`]) where
//! they changed. What a tag held before the segment is the ordered emit's ([`super::emit`]). An
//! hour that changes nothing the factories read carries nothing but its hour and its read.

use super::{FIELD_COUNT, fill};
use dashboard_world::{Needs, Roster, Tagged, World};
use replay::{
    columnar_frames::metrics,
    columns,
    segments::{self, Item, Operator, Sink, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{CountryFactoriesRow, TableId},
};
use std::{
    collections::{HashMap, HashSet},
    io,
    mem::size_of,
    sync::Arc,
};

/// The tables a projection reads.
pub const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryFactories,
];

/// What a projection reads of its walk's world: the countries.
pub fn needs() -> Needs {
    Needs {
        countries: true,
        ..Needs::default()
    }
}

/// A holder's values in [`super::fields`] order.
pub type Values = Arc<[i32]>;

/// The tags the complete Countries and CountryLinks resolve, as the row scan resolves them:
/// each tag's first holder by id the links resolve.
#[derive(Debug, PartialEq, Eq)]
pub struct Countries {
    /// Each UTF-8 tag's canonical slot and id, by tag.
    pub tags: Vec<(Arc<str>, (u32, u32))>,
}

impl Countries {
    fn heap(&self) -> usize {
        self.tags.capacity() * size_of::<(Arc<str>, (u32, u32))>()
            + self.tags.iter().map(|(tag, _)| tag.len()).sum::<usize>()
    }
}

/// A read's holder values: every slot holding rows (the others hold none), or the slots whose
/// rows changed since their last read. None where a slot holds no row, two rows, or a total
/// the game could not show.
pub struct Stocks {
    pub whole: bool,
    pub slots: Vec<(u32, Option<Values>)>,
}

/// What an hour counts, as the row scan counts it at a read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Census {
    /// Rows no link resolves, each row repeating its holder, and each tag's row holding a
    /// total the game could not show: counted at reads only.
    pub unreadable: u64,
    /// Each tag's row whose recorded `unavailable` is not the sum of its parts, by group in
    /// [`FactoryGroupKey::ALL`] order: counted at reads only.
    pub unexplained: [u64; 3],
    /// Resolved tags that are not UTF-8: counted every hour.
    pub unnamed: u64,
}

/// What the ordered emit reads of an hour that changes what the factories read.
pub struct Facts {
    /// Whether the CountryFactories table holds every row.
    pub complete: bool,
    /// The countries, at a whole hour or where they changed: none while Countries or
    /// CountryLinks is not complete.
    pub countries: Option<Option<Arc<Countries>>>,
    /// Whether this hour's item is the first to carry `countries`, which it is charged for.
    pub charged: bool,
    /// The values read, at a read of a complete table while the links are complete.
    pub stocks: Option<Stocks>,
    /// The hour's counts where they changed.
    pub census: Option<Census>,
}

/// One hour of a segment.
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// Whether the CountryFactories table was read (`slot_read`).
    pub read: bool,
    /// None when the hour changes nothing the factories read.
    pub facts: Option<Box<Facts>>,
}

impl Item for Hour {
    fn heap(&self) -> usize {
        let Some(facts) = &self.facts else {
            return 0;
        };
        // The projector and every hour until the countries change share them.
        let countries = match &facts.countries {
            Some(Some(countries)) if facts.charged => size_of::<Countries>() + countries.heap(),
            _ => 0,
        };
        size_of::<Facts>()
            + countries
            + facts.stocks.as_ref().map_or(0, |stocks| {
                stocks.slots.capacity() * size_of::<(u32, Option<Values>)>()
                    + stocks
                        .slots
                        .iter()
                        .flat_map(|(_, values)| values)
                        .map(|values| values.len() * 4 + 16)
                        .sum::<usize>()
            })
    }
}

/// A CountryFactories row: its holder slot, none where no link resolves its country, and its
/// position among its holder's rows.
#[derive(Clone, Copy)]
struct Row {
    holder: Option<u32>,
    at: usize,
}

/// The CountryFactories rows by holder slot, kept from the hour's changes.
#[derive(Default)]
struct Rows {
    rows: Vec<Option<Row>>,
    holders: HashMap<u32, Vec<u32>>,
    /// Rows no link resolves, and rows a link resolves.
    unlinked: usize,
    linked: usize,
}

impl Rows {
    fn clear(&mut self) {
        self.rows.clear();
        self.holders.clear();
        self.unlinked = 0;
        self.linked = 0;
    }

    /// Forgets the row at `slot`; its holder, when a link resolves it.
    fn remove(&mut self, slot: u32) -> Option<u32> {
        let row = self.rows.get_mut(slot as usize).and_then(Option::take)?;
        let Some(holder) = row.holder else {
            self.unlinked -= 1;
            return None;
        };
        self.linked -= 1;
        let own = self.holders.get_mut(&holder).expect("indexed holder rows");
        own.swap_remove(row.at);
        if let Some(&moved) = own.get(row.at)
            && let Some(Some(other)) = self.rows.get_mut(moved as usize)
        {
            other.at = row.at;
        }
        if own.is_empty() {
            self.holders.remove(&holder);
        }
        Some(holder)
    }

    fn insert(&mut self, slot: u32, holder: Option<u32>) {
        self.remove(slot);
        let index = slot as usize;
        if self.rows.len() <= index {
            self.rows.resize(index + 1, None);
        }
        let at = match holder {
            Some(holder) => {
                self.linked += 1;
                let own = self.holders.entry(holder).or_default();
                own.push(slot);
                own.len() - 1
            }
            None => {
                self.unlinked += 1;
                0
            }
        };
        self.rows[index] = Some(Row { holder, at });
    }

    fn holder(&self, slot: u32) -> Option<Option<u32>> {
        self.rows
            .get(slot as usize)
            .copied()
            .flatten()
            .map(|row| row.holder)
    }

    /// Rows repeating a holder another row holds.
    fn repeated(&self) -> usize {
        self.linked - self.holders.len()
    }

    fn heap(&self) -> usize {
        self.rows.capacity() * size_of::<Option<Row>>()
            + self.holders.capacity() * size_of::<(u32, Vec<u32>)>()
            + self
                .holders
                .values()
                .map(|rows| rows.capacity() * 4)
                .sum::<usize>()
    }
}

/// A holder slot's last read: its values, whether its one row held a total the game could not
/// show, and which groups' `unavailable` is not the sum of their parts.
struct Read {
    values: Option<Values>,
    overflow: bool,
    unexplained: [bool; 3],
}

/// The columns a read reads, by name; `levels` and `used` are not read.
const I64_FIELDS: [&str; 12] = [
    "military_total_raw",
    "military_controlled_raw",
    "military_owned_raw",
    "military_damaged_raw",
    "naval_total_raw",
    "naval_controlled_raw",
    "naval_owned_raw",
    "naval_damaged_raw",
    "civilian_total_raw",
    "civilian_controlled_raw",
    "civilian_owned_raw",
    "civilian_damaged_raw",
];
const I32_FIELDS: [&str; 22] = [
    "military_unavailable",
    "military_from_subjects",
    "military_to_overlord",
    "military_from_others",
    "military_to_others",
    "naval_unavailable",
    "naval_from_subjects",
    "naval_to_overlord",
    "naval_from_others",
    "naval_to_others",
    "civilian_unavailable",
    "civilian_from_subjects",
    "civilian_to_overlord",
    "civilian_from_others",
    "civilian_to_others",
    "civilian_license_exports",
    "civilian_trade_exports",
    "consumer_goods",
    "special_projects",
    "license_imports",
    "trade_imports",
    "contract_payments",
];

/// The row a read reads, its fields in [`I64_FIELDS`] and [`I32_FIELDS`] order.
fn row_of(wide: [i64; 12], narrow: [i32; 22]) -> CountryFactoriesRow {
    let [
        military_total_raw,
        military_controlled_raw,
        military_owned_raw,
        military_damaged_raw,
        naval_total_raw,
        naval_controlled_raw,
        naval_owned_raw,
        naval_damaged_raw,
        civilian_total_raw,
        civilian_controlled_raw,
        civilian_owned_raw,
        civilian_damaged_raw,
    ] = wide;
    let [
        military_unavailable,
        military_from_subjects,
        military_to_overlord,
        military_from_others,
        military_to_others,
        naval_unavailable,
        naval_from_subjects,
        naval_to_overlord,
        naval_from_others,
        naval_to_others,
        civilian_unavailable,
        civilian_from_subjects,
        civilian_to_overlord,
        civilian_from_others,
        civilian_to_others,
        civilian_license_exports,
        civilian_trade_exports,
        consumer_goods,
        special_projects,
        license_imports,
        trade_imports,
        contract_payments,
    ] = narrow;
    CountryFactoriesRow {
        military_total_raw,
        military_controlled_raw,
        military_owned_raw,
        military_damaged_raw,
        naval_total_raw,
        naval_controlled_raw,
        naval_owned_raw,
        naval_damaged_raw,
        civilian_total_raw,
        civilian_controlled_raw,
        civilian_owned_raw,
        civilian_damaged_raw,
        military_unavailable,
        military_from_subjects,
        military_to_overlord,
        military_from_others,
        military_to_others,
        naval_unavailable,
        naval_from_subjects,
        naval_to_overlord,
        naval_from_others,
        naval_to_others,
        civilian_unavailable,
        civilian_from_subjects,
        civilian_to_overlord,
        civilian_from_others,
        civilian_to_others,
        civilian_license_exports,
        civilian_trade_exports,
        consumer_goods,
        special_projects,
        license_imports,
        trade_imports,
        contract_payments,
        ..Default::default()
    }
}

/// The countries the walk's roster resolves, how many UTF-8 tags hold each slot, and how many
/// resolved tags are not UTF-8.
fn countries(roster: &Roster) -> (Countries, HashMap<u32, u64>, u64) {
    let mut resolved = Countries { tags: Vec::new() };
    let mut weights: HashMap<u32, u64> = HashMap::new();
    let mut unnamed = 0;
    for tag in roster.held() {
        let Tagged::Found(country) = roster.tagged(tag) else {
            continue;
        };
        match std::str::from_utf8(roster.tag(tag)) {
            Ok(text) => {
                *weights.entry(country.slot).or_default() += 1;
                resolved
                    .tags
                    .push((Arc::from(text), (country.slot, country.id)));
            }
            Err(_) => unnamed += 1,
        }
    }
    resolved
        .tags
        .sort_unstable_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    (resolved, weights, unnamed)
}

/// One segment's projection: every segment starts cold.
pub struct Projector {
    format: FormatVersion,
    wide: [usize; 12],
    narrow: [usize; 22],
    /// The change-mask bits of every column a read reads.
    mask: u64,
    /// Nothing is held: the next hour is read whole, as a segment's first.
    cold: bool,
    last: Option<i32>,
    status: Option<columns::Status>,
    /// Whether Countries and CountryLinks were complete, and CountryLinks alone.
    roster: bool,
    links: bool,
    countries: Arc<Countries>,
    /// Whether no item carried `countries` yet.
    uncharged: bool,
    /// How many UTF-8 tags hold each slot, and the resolved tags that are not UTF-8.
    weights: HashMap<u32, u64>,
    unnamed: u64,
    rows: Rows,
    /// Holder slots whose rows changed since their last read, and a read's slots.
    dirty: HashSet<u32>,
    slots: Vec<u32>,
    /// The next read reads every slot.
    stale: bool,
    reads: HashMap<u32, Read>,
    /// The tag-weighted reads holding a total the game could not show, and unexplained groups.
    overflow: u64,
    unexplained: [u64; 3],
    /// The last counts sent, none in this segment yet, and whether they may have changed.
    census: Option<Census>,
    recount: bool,
}

/// What [`Projector::save`] writes: nothing but that it restarts, since everything it holds is
/// read again from the walker's tables, as at a segment's first hour.
const SAVED: &[u8] = b"factories projector: cold";

impl Projector {
    pub fn new(format: FormatVersion) -> io::Result<Self> {
        let table = TableId::CountryFactories;
        let mut wide = [0; 12];
        for (column, name) in wide.iter_mut().zip(I64_FIELDS) {
            *column = columns::field(table, name)?;
        }
        let mut narrow = [0; 22];
        for (column, name) in narrow.iter_mut().zip(I32_FIELDS) {
            *column = columns::field(table, name)?;
        }
        let mask = wide
            .iter()
            .chain(&narrow)
            .map(|&column| u32::try_from(column).ok().and_then(|bit| 1_u64.checked_shl(bit)))
            .try_fold(0_u64, |mask, bit| bit.map(|bit| mask | bit))
            .ok_or_else(|| io::Error::other("A factory column lies past the change mask"))?;
        Ok(Self {
            format,
            wide,
            narrow,
            mask,
            cold: true,
            last: None,
            status: None,
            roster: false,
            links: false,
            countries: Arc::new(Countries { tags: Vec::new() }),
            uncharged: true,
            weights: HashMap::new(),
            unnamed: 0,
            rows: Rows::default(),
            dirty: HashSet::new(),
            slots: Vec::new(),
            stale: true,
            reads: HashMap::new(),
            overflow: 0,
            unexplained: [0; 3],
            census: None,
            recount: true,
        })
    }

    /// The raw country of the CountryFactories row at `slot`.
    fn country(state: &State, slot: u32) -> io::Result<u32> {
        let key = columns::table(state, TableId::CountryFactories)?
            .key(slot)
            .ok_or_else(|| io::Error::other("A CountryFactories slot holds no row"))?;
        Ok(key as u32)
    }

    /// The row at `slot`, with the fields a read reads.
    fn row(&self, state: &State, slot: u32) -> io::Result<CountryFactoriesRow> {
        let table = TableId::CountryFactories;
        let held = columns::table(state, table)?;
        if self
            .wide
            .iter()
            .chain(&self.narrow)
            .any(|&column| !held.recorded(column, slot))
        {
            return columns::row(state, table, slot, self.format.version());
        }
        let at = slot as usize;
        let cell = |values: &[i64]| {
            values
                .get(at)
                .copied()
                .ok_or_else(|| io::Error::other("A factory slot holds no cell"))
        };
        let mut wide = [0; 12];
        for (value, &column) in wide.iter_mut().zip(&self.wide) {
            *value = cell(columns::plain::<i64>(held, column)?)?;
        }
        let mut narrow = [0; 22];
        for (value, &column) in narrow.iter_mut().zip(&self.narrow) {
            *value = columns::plain::<i32>(held, column)?
                .get(at)
                .copied()
                .ok_or_else(|| io::Error::other("A factory slot holds no cell"))?;
        }
        Ok(row_of(wide, narrow))
    }

    /// The CountryFactories rows by holder: read whole, or by the hour's changes.
    fn index(&mut self, state: &State, roster: &Roster, whole: bool) -> io::Result<()> {
        let table = TableId::CountryFactories;
        self.recount = true;
        if whole {
            self.rows.clear();
            let live: Vec<u32> = columns::table(state, table)?.live().ones().collect();
            for slot in live {
                let country = Self::country(state, slot)?;
                self.rows.insert(slot, roster.slot_of(country));
            }
            return Ok(());
        }
        let edits = columns::changes(state, table)?;
        for &slot in &edits.deleted {
            if let Some(holder) = self.rows.remove(slot) {
                self.dirty.insert(holder);
            }
        }
        for &slot in &edits.inserted {
            let holder = roster.slot_of(Self::country(state, slot)?);
            self.rows.insert(slot, holder);
            self.dirty.extend(holder);
        }
        let written = edits
            .changed
            .iter()
            .zip(&edits.masks)
            .chain(edits.fill_slots.iter().zip(&edits.fill_masks));
        for (&slot, &mask) in written {
            if mask & self.mask != 0
                && let Some(Some(holder)) = self.rows.holder(slot)
            {
                self.dirty.insert(holder);
            }
        }
        Ok(())
    }

    /// A holder's read from its `rows`, as the row scan reads it: one row's values, none for
    /// two rows. Values equal to `held`'s are `held`'s, allocating nothing.
    fn read(&self, state: &State, rows: &[u32], held: Option<&Read>) -> io::Result<Read> {
        let [row] = rows else {
            return Ok(Read {
                values: None,
                overflow: false,
                unexplained: [false; 3],
            });
        };
        let row = self.row(state, *row)?;
        let mut values = [0; FIELD_COUNT];
        Ok(match fill(&row, &mut values) {
            Some(unexplained) => Read {
                values: Some(
                    match held.and_then(|held| held.values.as_ref()) {
                        Some(held) if **held == values[..] => Arc::clone(held),
                        _ => Values::from(&values[..]),
                    },
                ),
                overflow: false,
                unexplained,
            },
            None => Read {
                values: None,
                overflow: true,
                unexplained: [false; 3],
            },
        })
    }

    /// Adds (or with `sign` false takes away) `read`'s tag-weighted counts of `slot`.
    fn weigh(&mut self, slot: u32, read: &Read, sign: bool) {
        let weight = self.weights.get(&slot).copied().unwrap_or(0);
        if weight == 0 {
            return;
        }
        let apply = |total: &mut u64, counted: bool| {
            if counted {
                if sign {
                    *total += weight;
                } else {
                    *total -= weight;
                }
            }
        };
        apply(&mut self.overflow, read.overflow);
        for (total, &counted) in self.unexplained.iter_mut().zip(&read.unexplained) {
            apply(total, counted);
        }
    }

    /// The values a read reads: every slot's, or those that changed since their last.
    fn stocks(&mut self, state: &State) -> io::Result<Stocks> {
        let _cpu = replay_frame::cpu::scope("factories.columnar.stocks", None);
        let whole = std::mem::take(&mut self.stale);
        let mut slots = std::mem::take(&mut self.slots);
        slots.clear();
        if whole {
            self.recount = true;
            self.reads.clear();
            self.overflow = 0;
            self.unexplained = [0; 3];
            self.dirty.clear();
            slots.extend(self.rows.holders.keys().copied());
        } else {
            slots.extend(self.dirty.drain());
        }
        slots.sort_unstable();
        let mut read = Vec::new();
        for &slot in &slots {
            self.recount = true;
            let held = self.reads.remove(&slot);
            if let Some(held) = &held {
                self.weigh(slot, held, false);
            }
            let Some(rows) = self.rows.holders.get(&slot) else {
                if held.is_some() {
                    read.push((slot, None));
                }
                continue;
            };
            let slot_read = self.read(state, rows, held.as_ref())?;
            self.weigh(slot, &slot_read, true);
            // The emit carries values its slot's rows left as they were.
            if held.is_none_or(|held| held.values != slot_read.values) {
                read.push((slot, slot_read.values.clone()));
            }
            self.reads.insert(slot, slot_read);
        }
        self.slots = slots;
        Ok(Stocks { whole, slots: read })
    }

    /// The hour's counts, where they may have changed since the last sent.
    fn census(&mut self, rows: bool) -> Option<Census> {
        if !std::mem::take(&mut self.recount) && self.census.is_some() {
            return None;
        }
        let census = if rows {
            Census {
                unreadable: (self.rows.unlinked + self.rows.repeated()) as u64 + self.overflow,
                unexplained: self.unexplained,
                unnamed: self.unnamed,
            }
        } else {
            Census {
                unnamed: self.unnamed,
                ..Census::default()
            }
        };
        if self.census == Some(census) {
            return None;
        }
        self.census = Some(census);
        Some(census)
    }

    fn heap(&self) -> usize {
        size_of::<Countries>()
            + self.countries.heap()
            + self.weights.capacity() * 12
            + self.rows.heap()
            + self.dirty.capacity() * 4
            + self.slots.capacity() * 4
            + self.reads.capacity() * size_of::<(u32, Read)>()
            + self
                .reads
                .values()
                .flat_map(|read| &read.values)
                .map(|values| values.len() * 4 + 16)
                .sum::<usize>()
    }
}

/// Whether the hour inserted, deleted or changed a row of `table`, or read it in part.
fn touched(state: &State, table: TableId) -> io::Result<bool> {
    let changes = columns::changes(state, table)?;
    Ok(changes.truncated
        || !changes.changed.is_empty()
        || !changes.fill_slots.is_empty()
        || !changes.inserted.is_empty()
        || !changes.deleted.is_empty())
}

// Every segment starts cold: its first hour reads every holder and country again; what came
// before is the emit's.
impl Operator<World> for Projector {
    type Output = Hour;

    fn start(&mut self, _: segments::Start<'_>, _: u64, _: i32) -> io::Result<()> {
        self.cold = true;
        Ok(())
    }

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("factories.columnar", Some("factories"));
        let state = hour.state;
        let at = hour.hour;
        let roster = hour.world.countries()?;
        let opens = self.cold
            || hour.starts
            || hour.opens
            || self.last.and_then(|last| last.checked_add(1)) != Some(at);
        if opens {
            self.census = None;
        }
        self.cold = false;
        self.last = Some(at);
        let table = TableId::CountryFactories;
        let status = columns::status(state, table);
        let complete = status == columns::Status::Complete;
        let shaped = opens || self.status != Some(status);
        self.status = Some(status);
        let complete_roster = roster.complete();
        let links = roster.links_complete();
        let regrouped = opens || roster.rebuilt() || !roster.changed_links().is_empty();
        let became = opens || complete_roster != self.roster || links != self.links;
        self.roster = complete_roster;
        self.links = links;
        let mut countries = None;
        if became || roster.changed() {
            let (resolved, weights, unnamed) = self::countries(roster);
            self.weights = weights;
            self.unnamed = unnamed;
            self.recount = true;
            if !complete_roster {
                if became {
                    countries = Some(None);
                }
            } else if became || resolved != *self.countries {
                self.countries = Arc::new(resolved);
                self.uncharged = true;
                countries = Some(Some(Arc::clone(&self.countries)));
            }
            // The tags count their slots anew, and a tag moved reads its new slot.
            self.stale = true;
        }
        if !complete || !links {
            self.rows.clear();
            self.dirty.clear();
            self.stale = true;
            self.recount = true;
        } else if shaped || regrouped {
            self.index(state, roster, true)?;
            self.stale = true;
        } else if touched(state, table)? {
            self.index(state, roster, false)?;
        }
        let flags = metrics(hour.manifest);
        let read = crate::metadata::slot_read(self.format, &flags, table, at);
        let counted = complete && links;
        let stocks = if read && counted {
            let stocks = self.stocks(state)?;
            (stocks.whole || !stocks.slots.is_empty()).then_some(stocks)
        } else {
            if !counted {
                self.reads.clear();
                self.overflow = 0;
                self.unexplained = [0; 3];
            }
            None
        };
        let census = self.census(counted);
        let quiet = !shaped && countries.is_none() && stocks.is_none() && census.is_none();
        let facts = (!quiet).then(|| {
            let charged = matches!(countries, Some(Some(_))) && std::mem::take(&mut self.uncharged);
            Box::new(Facts {
                complete,
                countries,
                charged,
                stocks,
                census,
            })
        });
        out.push(Hour {
            branch: hour.branch,
            hour: at,
            read,
            facts,
        })
    }

    fn carried(&self) -> Option<usize> {
        Some(self.heap())
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<bool> {
        out.extend_from_slice(SAVED);
        Ok(true)
    }

    fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes != SAVED {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not a saved factories projector",
            ));
        }
        *self = Self::new(self.format)?;
        Ok(())
    }
}
