//! The stockpile's hours from a columnar walk ([`replay::segments`]). Each segment keeps, from
//! the tables' change lists, which holder every stock, line and logistics row belongs to and
//! which holders' rows changed since their last read in the segment. An hour that reads a table
//! hands the ordered emit ([`super::emit::Emit`]) only those holders' totals, read from
//! typed columns: every key's at a holder's first read since the segment's last whole hour,
//! else the keys whose total changed since it last sent them. An hour that reads nothing and
//! changes nothing the stockpile reads is quiet: it carries nothing but its hour.
use dashboard_base::products::equipment::{RawVariant, VariantHour, daily_units_raw};
use dashboard_world::{Naming as Names, Needs, Roster, Tagged, World};
use engine_native::cequipment_production_line::KIND_MILITARY;
use replay::{
    columnar_frames::metrics,
    columns,
    segments::{self, Item, Operator, Sink},
};
use replay_data::{
    format::FormatVersion,
    rows::{
        LexicalTokenRow, LogisticsEquipmentRow, ProductionLineRow, Recorded, StockpileRow,
        TABLE_COUNT, TableId,
    },
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    hash::{Hash, Hasher},
    io,
    mem::size_of,
    sync::Arc,
};

mod naming;
mod slots;

/// What a projection reads of its walk's world: the countries and their cosmetic tags.
pub fn needs() -> Needs {
    Needs {
        countries: true,
        naming: true,
        ..Needs::default()
    }
}

/// The tables the stockpile reads: its numbers and the variant catalogue that names them.
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::LexicalTokens,
    TableId::VariantDefinitions,
    TableId::VariantModules,
    TableId::VariantUpgrades,
    TableId::VariantChanges,
    TableId::Stockpiles,
    TableId::ProductionLines,
    TableId::LogisticsEquipment,
];

/// The tables holders' numbers are read from: stock and lines are read together, the
/// logistics record on its own.
const NUMERIC: [TableId; 3] = [
    TableId::Stockpiles,
    TableId::ProductionLines,
    TableId::LogisticsEquipment,
];

/// One hour of a segment.
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// None when the hour is quiet.
    pub(crate) facts: Option<Box<Facts>>,
}

/// What the ordered emit reads of an hour that is not quiet.
pub(crate) struct Facts {
    /// Whether the stock and line tables, and the logistics record, were read (`slot_read`).
    pub(crate) reads: [bool; 2],
    /// Whether Countries and CountryLinks are complete, so tags resolve to holders.
    pub(crate) countries: bool,
    /// Whether each numeric table's rows name holders: it, Countries and CountryLinks are
    /// complete.
    pub(crate) usable: [bool; 3],
    /// Every holder, at a whole hour: any holder or table usable may have changed, and every
    /// holder's next read of each table is whole.
    pub(crate) holders: Option<Arc<Holders>>,
    /// Whether this hour's item is the first to carry `holders`, which it is charged for.
    pub(crate) charged: bool,
    pub(crate) evidence: VariantHour,
    /// The operands of the definitions a complete variant read built.
    pub(crate) variants: Vec<Variant>,
    /// The holder slots read this hour, ascending.
    pub(crate) rows: Vec<Read>,
}

/// A definition's naming operands at a complete variant read.
pub(crate) struct Variant {
    pub(crate) id: u64,
    /// The operands but the hour they were read at, digested: equal operands, equal digests.
    pub(crate) digest: u128,
    pub(crate) raw: RawVariant,
}

impl Variant {
    fn new(id: u64, mut raw: RawVariant) -> Self {
        struct Digest(blake3::Hasher);
        impl Hasher for Digest {
            fn write(&mut self, bytes: &[u8]) {
                self.0.update(bytes);
            }
            fn finish(&self) -> u64 {
                unreachable!("a digest is read whole")
            }
        }
        let hour = std::mem::replace(&mut raw.first_hour, 0);
        let mut digest = Digest(blake3::Hasher::new());
        raw.hash(&mut digest);
        raw.first_hour = hour;
        let bytes = digest.0.finalize();
        let digest = u128::from_le_bytes(bytes.as_bytes()[..16].try_into().expect("16 bytes"));
        Self { id, digest, raw }
    }
}

/// The holders the complete Countries and CountryLinks resolve.
pub(crate) struct Holders {
    /// Each tag's canonical slot and country id, as `CountryReads` resolves them.
    pub(crate) identities: BTreeMap<String, (u32, u32)>,
}

impl Holders {
    fn heap(&self) -> usize {
        self.identities
            .keys()
            .map(|tag| size_of::<(String, (u32, u32))>() + tag.capacity())
            .sum()
    }
}

/// A table's totals for one holder slot, by key.
pub(crate) enum Totals<K, const N: usize> {
    /// Every key's: the slot's first read since the segment's last whole hour.
    Whole(Vec<(K, [Option<i64>; N])>),
    /// The keys whose totals changed since the slot's last read here; none where it holds no
    /// row now.
    Changed(Vec<(K, Option<[Option<i64>; N]>)>),
}

/// One holder slot's read: the totals of each table read and usable, as its step reads
/// them.
pub(crate) struct Read {
    pub(crate) slot: u32,
    /// Each variant's stock.
    pub(crate) stock: Option<Totals<u64, 1>>,
    /// Each variant's units and IC a day, and its lowest and highest cost per unit.
    pub(crate) lines: Option<Totals<u64, 4>>,
    /// Each archetype token's balance, stock and available stock.
    pub(crate) logistics: Option<Totals<u32, 3>>,
    /// What the read counts, in `read_counts`' order: overflowed stock; costless lines,
    /// other kinds and overflowed daily, IC and cost totals; missing available operands and
    /// overflowed balance, stock and available totals.
    pub(crate) counts: [usize; 11],
    /// Its logistics tokens' spellings, where LexicalTokens is complete.
    pub(crate) spellings: Vec<(u32, String)>,
}

impl Read {
    fn new(slot: u32) -> Self {
        Self {
            slot,
            stock: None,
            lines: None,
            logistics: None,
            counts: [0; 11],
            spellings: Vec::new(),
        }
    }

    fn heap(&self) -> usize {
        fn totals<K, const N: usize>(totals: &Option<Totals<K, N>>) -> usize {
            match totals {
                Some(Totals::Whole(rows)) => rows.capacity() * size_of::<(K, [Option<i64>; N])>(),
                Some(Totals::Changed(rows)) => {
                    rows.capacity() * size_of::<(K, Option<[Option<i64>; N]>)>()
                }
                None => 0,
            }
        }
        totals(&self.stock)
            + totals(&self.lines)
            + totals(&self.logistics)
            + self.spellings.capacity() * size_of::<(u32, String)>()
            + self
                .spellings
                .iter()
                .map(|(_, spelling)| spelling.capacity())
                .sum::<usize>()
    }
}

fn variant_heap(raw: &RawVariant) -> usize {
    [&raw.token, &raw.display_name, &raw.suffix]
        .into_iter()
        .chain(&raw.name_tag)
        .chain(&raw.type_tag)
        .chain(&raw.design)
        .map(String::capacity)
        .sum::<usize>()
        + raw.refusals.capacity() * size_of::<(String, String)>()
        + raw
            .refusals
            .iter()
            .map(|(label, why)| label.capacity() + why.capacity())
            .sum::<usize>()
}

impl Item for Hour {
    fn heap(&self) -> usize {
        let Some(facts) = &self.facts else {
            return 0;
        };
        // The projector and every whole hour until the holders change share them.
        let holders = match &facts.holders {
            Some(holders) if facts.charged => size_of::<Holders>() + holders.heap(),
            _ => 0,
        };
        holders
            + size_of::<Facts>()
            + facts.variants.capacity() * size_of::<Variant>()
            + facts
                .variants
                .iter()
                .map(|variant| variant_heap(&variant.raw))
                .sum::<usize>()
            + facts.rows.capacity() * size_of::<Read>()
            + facts.rows.iter().map(Read::heap).sum::<usize>()
    }
}

struct Fields {
    country_slot: usize,
    country_tag: usize,
    appearance_name: usize,
    literal: usize,
    /// Stockpiles' variant_handle and amount_raw.
    stock: [usize; 2],
    /// ProductionLines' country, kind, variant, cost_raw and speed_raw.
    line: [usize; 5],
    /// LogisticsEquipment's token, balance_raw, stock_raw and available_raw.
    logistics: [usize; 4],
    /// Each numeric table's fields a holder reads, and those naming its row's holder or key.
    read: [u64; 3],
    keyed: [u64; 3],
}

impl Fields {
    fn new() -> io::Result<Self> {
        let stock = [
            columns::field(TableId::Stockpiles, "variant_handle")?,
            columns::field(TableId::Stockpiles, "amount_raw")?,
        ];
        let line = [
            columns::field(TableId::ProductionLines, "country")?,
            columns::field(TableId::ProductionLines, "kind")?,
            columns::field(TableId::ProductionLines, "variant")?,
            columns::field(TableId::ProductionLines, "cost_raw")?,
            columns::field(TableId::ProductionLines, "speed_raw")?,
        ];
        let logistics = [
            columns::field(TableId::LogisticsEquipment, "token")?,
            columns::field(TableId::LogisticsEquipment, "balance_raw")?,
            columns::field(TableId::LogisticsEquipment, "stock_raw")?,
            columns::field(TableId::LogisticsEquipment, "available_raw")?,
        ];
        Ok(Self {
            country_slot: columns::field(TableId::Countries, "canonical_slot")?,
            country_tag: columns::field(TableId::Countries, "tag")?,
            appearance_name: columns::field(TableId::CountryAppearance, "name_tag")?,
            literal: columns::field(TableId::LexicalTokens, "literal")?,
            read: [
                slots::mask(&stock),
                slots::mask(&line),
                slots::mask(&logistics),
            ],
            keyed: [
                slots::mask(&stock[..1]),
                slots::mask(&[line[0], line[2]]),
                slots::mask(&logistics[..1]),
            ],
            stock,
            line,
            logistics,
        })
    }
}

/// The holders the walk's complete countries resolve, as `CountryReads` and `FrameRows`
/// resolve them: each tag's first holder by id the links resolve.
fn holders(roster: &Roster) -> Holders {
    let mut holders = Holders {
        identities: BTreeMap::new(),
    };
    for tag in roster.held().filter(|_| roster.complete()) {
        if let (Ok(text), Tagged::Found(country)) =
            (std::str::from_utf8(roster.tag(tag)), roster.tagged(tag))
        {
            holders
                .identities
                .insert(text.to_owned(), (country.slot, country.id));
        }
    }
    holders
}

/// Where a numeric table's row is: its holder slot (none where the links resolve no
/// country), its key (variant or token) and its position among its holder's rows.
#[derive(Clone, Copy)]
struct Owned {
    holder: Option<u32>,
    key: u64,
    at: usize,
}

/// A numeric table's live rows by holder slot and by key, kept from the hour's changes.
#[derive(Default)]
struct Owners {
    rows: Vec<Option<Owned>>,
    holders: HashMap<u32, Vec<u32>>,
    /// Each key's holders, with how many of their rows hold it.
    keys: HashMap<u64, HashMap<u32, u32>>,
}

impl Owners {
    fn clear(&mut self) {
        self.rows.clear();
        self.holders.clear();
        self.keys.clear();
    }

    /// Forgets the row at `slot`; its holder, when it had one.
    fn remove(&mut self, slot: u32) -> Option<u32> {
        let owned = self.rows.get_mut(slot as usize).and_then(Option::take)?;
        let holder = owned.holder?;
        let own = self.holders.get_mut(&holder).expect("indexed holder rows");
        own.swap_remove(owned.at);
        if let Some(&moved) = own.get(owned.at)
            && let Some(Some(row)) = self.rows.get_mut(moved as usize)
        {
            row.at = owned.at;
        }
        if own.is_empty() {
            self.holders.remove(&holder);
        }
        if let Some(holders) = self.keys.get_mut(&owned.key) {
            if let Some(count) = holders.get_mut(&holder) {
                *count -= 1;
                if *count == 0 {
                    holders.remove(&holder);
                }
            }
            if holders.is_empty() {
                self.keys.remove(&owned.key);
            }
        }
        Some(holder)
    }

    fn insert(&mut self, slot: u32, holder: Option<u32>, key: u64) {
        self.remove(slot);
        let index = slot as usize;
        if self.rows.len() <= index {
            self.rows.resize(index + 1, None);
        }
        let at = match holder {
            Some(holder) => {
                let own = self.holders.entry(holder).or_default();
                own.push(slot);
                *self.keys.entry(key).or_default().entry(holder).or_default() += 1;
                own.len() - 1
            }
            None => 0,
        };
        self.rows[index] = Some(Owned { holder, key, at });
    }

    /// The holder of the row at `slot`; none when the slot holds no indexed row.
    fn holder(&self, slot: u32) -> Option<Option<u32>> {
        self.rows
            .get(slot as usize)
            .copied()
            .flatten()
            .map(|owned| owned.holder)
    }

    fn of(&self, holder: u32) -> &[u32] {
        self.holders.get(&holder).map_or(&[][..], Vec::as_slice)
    }

    fn holding(&self, key: u64) -> impl Iterator<Item = u32> + '_ {
        self.keys
            .get(&key)
            .into_iter()
            .flat_map(|holders| holders.keys().copied())
    }

    fn heap(&self) -> usize {
        self.rows.capacity() * size_of::<Option<Owned>>()
            + self.holders.capacity() * size_of::<(u32, Vec<u32>)>()
            + self
                .holders
                .values()
                .map(|rows| rows.capacity() * size_of::<u32>())
                .sum::<usize>()
            + self.keys.capacity() * size_of::<(u64, HashMap<u32, u32>)>()
            + self
                .keys
                .values()
                .map(|holders| holders.capacity() * size_of::<(u32, u32)>())
                .sum::<usize>()
    }
}

/// What a holder slot was last sent of each table; none: nothing since the last whole hour.
#[derive(Default)]
struct Sent {
    stock: Option<Vec<(u64, [Option<i64>; 1])>>,
    lines: Option<Vec<(u64, [Option<i64>; 4])>>,
    logistics: Option<Vec<(u32, [Option<i64>; 3])>>,
}

impl Sent {
    fn heap(&self) -> usize {
        self.stock.as_ref().map_or(0, |rows| {
            rows.capacity() * size_of::<(u64, [Option<i64>; 1])>()
        }) + self.lines.as_ref().map_or(0, |rows| {
            rows.capacity() * size_of::<(u64, [Option<i64>; 4])>()
        }) + self.logistics.as_ref().map_or(0, |rows| {
            rows.capacity() * size_of::<(u32, [Option<i64>; 3])>()
        })
    }
}

/// What `held` last sent, set to `now`: every key where nothing was, else the keys whose
/// totals differ.
fn update<K: Ord + Copy, const N: usize>(
    held: &mut Option<Vec<(K, [Option<i64>; N])>>,
    now: Vec<(K, [Option<i64>; N])>,
) -> Totals<K, N> {
    let Some(before) = held.replace(now) else {
        return Totals::Whole(held.clone().expect("totals just sent"));
    };
    let now = held.as_deref().expect("totals just sent");
    let mut changed = Vec::new();
    let (mut old, mut new) = (before.iter().peekable(), now.iter().peekable());
    loop {
        match (old.peek().copied(), new.peek().copied()) {
            (None, None) => break,
            (Some(&(key, _)), None) => {
                changed.push((key, None));
                old.next();
            }
            (None, Some(&(key, values))) => {
                changed.push((key, Some(values)));
                new.next();
            }
            (Some(&(gone, was)), Some(&(key, values))) => match gone.cmp(&key) {
                std::cmp::Ordering::Less => {
                    changed.push((gone, None));
                    old.next();
                }
                std::cmp::Ordering::Greater => {
                    changed.push((key, Some(values)));
                    new.next();
                }
                std::cmp::Ordering::Equal => {
                    if was != values {
                        changed.push((key, Some(values)));
                    }
                    old.next();
                    new.next();
                }
            },
        }
    }
    Totals::Changed(changed)
}

/// An exact sum, unknown where a value is or where it does not fit (counted in `overflowed`),
/// as `ReadRuns::read` totals a key.
fn total(sum: Option<i128>, overflowed: &mut usize) -> Option<i64> {
    sum.and_then(|sum| {
        let fits = i64::try_from(sum).ok();
        *overflowed += usize::from(fits.is_none());
        fits
    })
}

fn add(sum: Option<i128>, value: Option<i64>) -> Option<i128> {
    sum.zip(value)
        .and_then(|(sum, value)| sum.checked_add(i128::from(value)))
}

/// What a holder's read takes of one production line.
#[derive(Clone, Copy)]
struct Line {
    kind: u8,
    speed_raw: i64,
    cost_raw: i64,
}

/// What a holder's read takes of one logistics row.
#[derive(Clone, Copy)]
struct Logistics {
    balance_raw: i64,
    stock_raw: i64,
    available_raw: Recorded<Option<i64>>,
}

pub struct Projector {
    format: FormatVersion,
    fields: Fields,
    shapes: [Option<columns::Status>; TABLE_COUNT],
    /// Nothing is held: the next hour is read whole, as a segment's first.
    cold: bool,
    last: Option<i32>,
    complete: [bool; 3],
    usable: [bool; 3],
    holders: Arc<Holders>,
    /// Whether no item carried `holders` yet.
    uncharged: bool,
    /// The holders' slots, ascending.
    slots: Vec<u32>,
    owners: [Owners; 3],
    /// Holder slots whose stock or line rows, and whose logistics rows, changed since their
    /// last read here.
    dirty: [HashSet<u32>; 2],
    /// Every holder's next read of the stock and lines, or of the logistics record, is
    /// sent.
    stale: [bool; 2],
    sent: HashMap<u32, Sent>,
    /// The logistics tokens whose spelling a read sent in this segment: the emit keeps each.
    spelled: HashSet<u32>,
    naming: naming::Naming,
}

/// What [`Projector::save`] writes: nothing but that it restarts, since everything it holds
/// is read again from the walker's tables, as at a segment's first hour.
const SAVED: &[u8] = b"stockpile projector: cold";

impl Projector {
    pub fn new(format: FormatVersion) -> io::Result<Self> {
        Ok(Self {
            format,
            fields: Fields::new()?,
            shapes: [None; TABLE_COUNT],
            cold: true,
            last: None,
            complete: [false; 3],
            usable: [false; 3],
            holders: Arc::new(Holders {
                identities: BTreeMap::new(),
            }),
            uncharged: true,
            slots: Vec::new(),
            owners: Default::default(),
            dirty: Default::default(),
            stale: [true; 2],
            sent: HashMap::new(),
            spelled: HashSet::new(),
            naming: naming::Naming::new()?,
        })
    }

    /// The stock row at `slot`: its variant and amount.
    fn stock(&self, state: &segments::State, slot: u32) -> io::Result<(u64, i64)> {
        let table = TableId::Stockpiles;
        let held = columns::table(state, table)?;
        let [variant, amount] = self.fields.stock;
        if !held.recorded(variant, slot) || !held.recorded(amount, slot) {
            let row: StockpileRow = columns::row(state, table, slot, self.format.version())?;
            return Ok((row.variant_handle, row.amount_raw));
        }
        Ok((
            columns::plain::<u64>(held, variant)?[slot as usize],
            columns::plain::<i64>(held, amount)?[slot as usize],
        ))
    }

    /// A line's raw country and variant, and what its holder reads of it.
    fn line(&self, state: &segments::State, slot: u32) -> io::Result<(u32, u64, Line)> {
        let table = TableId::ProductionLines;
        let held = columns::table(state, table)?;
        let [country, kind, variant, cost, speed] = self.fields.line;
        if self
            .fields
            .line
            .iter()
            .any(|&field| !held.recorded(field, slot))
        {
            let row: ProductionLineRow = columns::row(state, table, slot, self.format.version())?;
            return Ok((
                row.country,
                row.variant,
                Line {
                    kind: row.kind,
                    speed_raw: row.speed_raw,
                    cost_raw: row.cost_raw,
                },
            ));
        }
        let at = slot as usize;
        Ok((
            columns::plain::<u32>(held, country)?[at],
            columns::plain::<u64>(held, variant)?[at],
            Line {
                kind: columns::plain::<u8>(held, kind)?[at],
                speed_raw: columns::plain::<i64>(held, speed)?[at],
                cost_raw: columns::plain::<i64>(held, cost)?[at],
            },
        ))
    }

    /// A logistics row's token, and what its holder reads of it.
    fn logistics(&self, state: &segments::State, slot: u32) -> io::Result<(u32, Logistics)> {
        let table = TableId::LogisticsEquipment;
        let held = columns::table(state, table)?;
        let [token, balance, stock, available] = self.fields.logistics;
        if self
            .fields
            .logistics
            .iter()
            .any(|&field| !held.recorded(field, slot))
        {
            let row: LogisticsEquipmentRow =
                columns::row(state, table, slot, self.format.version())?;
            return Ok((
                row.token,
                Logistics {
                    balance_raw: row.balance_raw,
                    stock_raw: row.stock_raw,
                    available_raw: row.available_raw,
                },
            ));
        }
        let at = slot as usize;
        let (values, present) = columns::optional::<i64>(held, available)?;
        Ok((
            columns::plain::<u32>(held, token)?[at],
            Logistics {
                balance_raw: columns::plain::<i64>(held, balance)?[at],
                stock_raw: columns::plain::<i64>(held, stock)?[at],
                available_raw: Recorded::Value(present.get(slot).then(|| values[at])),
            },
        ))
    }

    /// The holder slot and key of numeric table `at`'s row at `slot`.
    fn owned(
        &self,
        state: &segments::State,
        roster: &Roster,
        at: usize,
        slot: u32,
    ) -> io::Result<(Option<u32>, u64)> {
        let row_key = columns::table(state, NUMERIC[at])?
            .key(slot)
            .ok_or_else(|| io::Error::other("A stockpile row's slot holds no row"))?;
        // Stock and logistics keys lead with the raw country reference.
        let (raw, key) = match at {
            0 => ((row_key >> 32) as u32, self.stock(state, slot)?.0),
            1 => {
                let (country, variant, _) = self.line(state, slot)?;
                (country, variant)
            }
            _ => (
                (row_key >> 32) as u32,
                u64::from(self.logistics(state, slot)?.0),
            ),
        };
        Ok((roster.slot_of(raw), key))
    }

    /// The numeric tables' row indexes and the holders whose rows the hour changed.
    fn index(
        &mut self,
        state: &segments::State,
        roster: &Roster,
        whole: bool,
        changed: &[bool; TABLE_COUNT],
        shaped: &[bool; TABLE_COUNT],
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("stockpile.columnar.rows", None);
        for (at, table) in NUMERIC.into_iter().enumerate() {
            if !self.complete[at] {
                self.owners[at].clear();
                continue;
            }
            if whole {
                self.owners[at].clear();
                let held = columns::table(state, table)?;
                for slot in held.live().ones() {
                    let (holder, key) = self.owned(state, roster, at, slot)?;
                    self.owners[at].insert(slot, holder, key);
                }
                continue;
            }
            if !changed[table.index()] {
                continue;
            }
            let group = usize::from(at == 2);
            let edits = columns::changes(state, table)?;
            for &slot in &edits.deleted {
                if let Some(holder) = self.owners[at].remove(slot) {
                    self.dirty[group].insert(holder);
                }
            }
            let touched = edits.inserted.iter().map(|&slot| (slot, u64::MAX)).chain(
                edits
                    .changed
                    .iter()
                    .zip(&edits.masks)
                    .chain(edits.fill_slots.iter().zip(&edits.fill_masks))
                    .map(|(&slot, &mask)| (slot, mask)),
            );
            for (slot, mask) in touched {
                if mask & self.fields.read[at] == 0 {
                    continue;
                }
                let indexed = self.owners[at].holder(slot);
                if indexed.is_some() && mask & self.fields.keyed[at] == 0 {
                    if let Some(Some(holder)) = indexed {
                        self.dirty[group].insert(holder);
                    }
                    continue;
                }
                let (holder, key) = self.owned(state, roster, at, slot)?;
                for holder in indexed.flatten().into_iter().chain(holder) {
                    self.dirty[group].insert(holder);
                }
                self.owners[at].insert(slot, holder, key);
            }
        }
        // A spelling reaches only the next logistics read of its token's holders.
        let table = TableId::LexicalTokens;
        if !whole && changed[table.index()] {
            if shaped[table.index()] {
                self.stale[1] = true;
            } else if columns::complete(state, table) {
                let held = columns::table(state, table)?;
                let tokens: Vec<u32> = slots::edits(state, table, 1_u64 << self.fields.literal)?
                    .into_iter()
                    .filter_map(|slot| held.key(slot).map(|key| key as u32))
                    .chain(
                        columns::changes(state, table)?
                            .deleted_keys
                            .iter()
                            .map(|&key| key as u32),
                    )
                    .collect();
                for token in tokens {
                    self.dirty[1].extend(self.owners[2].holding(u64::from(token)));
                }
            }
        }
        Ok(())
    }

    fn literal(&self, state: &segments::State, slot: u32) -> io::Result<Vec<u8>> {
        let table = TableId::LexicalTokens;
        if columns::table(state, table)?.recorded(self.fields.literal, slot) {
            return slots::text(state, table, self.fields.literal, slot);
        }
        Ok(columns::row::<LexicalTokenRow>(state, table, slot, self.format.version())?.literal)
    }

    /// The holder slot's stock by variant, and how many totals overflowed.
    fn stock_totals(
        &self,
        state: &segments::State,
        slot: u32,
    ) -> io::Result<(Vec<(u64, [Option<i64>; 1])>, usize)> {
        let mut rows = self.owners[0]
            .of(slot)
            .iter()
            .map(|&row| self.stock(state, row))
            .collect::<io::Result<Vec<_>>>()?;
        rows.sort_unstable_by_key(|&(variant, _)| variant);
        let mut overflowed = 0;
        let mut totals = Vec::new();
        for rows in rows.chunk_by(|a, b| a.0 == b.0) {
            let sum = rows
                .iter()
                .fold(Some(0), |sum, &(_, amount)| add(sum, Some(amount)));
            totals.push((rows[0].0, [total(sum, &mut overflowed)]));
        }
        Ok((totals, overflowed))
    }

    /// The holder slot's lines by variant: units and IC a day, lowest and highest cost per
    /// unit; and its costless lines, other kinds and overflowed totals.
    fn line_totals(
        &self,
        state: &segments::State,
        slot: u32,
    ) -> io::Result<(Vec<(u64, [Option<i64>; 4])>, [usize; 6])> {
        let mut rows = self.owners[1]
            .of(slot)
            .iter()
            .map(|&row| {
                self.line(state, row)
                    .map(|(_, variant, line)| (variant, line))
            })
            .collect::<io::Result<Vec<_>>>()?;
        rows.sort_unstable_by_key(|&(variant, _)| variant);
        let mut counts = [0; 6];
        let mut totals = Vec::new();
        for rows in rows.chunk_by(|a, b| a.0 == b.0) {
            let (mut daily, mut ic) = (Some(0), Some(0));
            let mut costs: Option<Option<(i64, i64)>> = None;
            for &(_, line) in rows {
                // Only a military line's rate is the military formula.
                let military = line.kind == KIND_MILITARY;
                let units = if military {
                    let units = daily_units_raw(line.speed_raw, line.cost_raw);
                    counts[0] += usize::from(units.is_none());
                    units
                } else {
                    counts[1] += 1;
                    None
                };
                daily = add(daily, units);
                ic = add(ic, military.then_some(line.speed_raw));
                let cost = (military && line.cost_raw > 0).then_some(line.cost_raw);
                costs = Some(match costs {
                    Some(range) => range
                        .zip(cost)
                        .map(|((low, high), cost)| (low.min(cost), high.max(cost))),
                    None => cost.map(|cost| (cost, cost)),
                });
            }
            let range = costs.flatten();
            totals.push((
                rows[0].0,
                [
                    total(daily, &mut counts[2]),
                    total(ic, &mut counts[3]),
                    range.map(|range| range.0),
                    range.map(|range| range.1),
                ],
            ));
        }
        Ok((totals, counts))
    }

    /// The holder slot's logistics record by token: balance, stock and available stock; and
    /// its rows without an available operand and overflowed totals.
    fn logistics_totals(
        &self,
        state: &segments::State,
        slot: u32,
    ) -> io::Result<(Vec<(u32, [Option<i64>; 3])>, [usize; 4])> {
        let mut rows = self.owners[2]
            .of(slot)
            .iter()
            .map(|&row| self.logistics(state, row))
            .collect::<io::Result<Vec<_>>>()?;
        rows.sort_unstable_by_key(|&(token, _)| token);
        let mut counts = [0; 4];
        let mut totals = Vec::new();
        for rows in rows.chunk_by(|a, b| a.0 == b.0) {
            let mut sums = [Some(0); 3];
            for &(_, row) in rows {
                let available = match row.available_raw {
                    Recorded::Value(value) => value,
                    Recorded::NotRecorded => None,
                };
                counts[0] += usize::from(available.is_none());
                sums[0] = add(sums[0], Some(row.balance_raw));
                sums[1] = add(sums[1], Some(row.stock_raw));
                sums[2] = add(sums[2], available);
            }
            let [balance, stock, available] = sums;
            totals.push((
                rows[0].0,
                [
                    total(balance, &mut counts[1]),
                    total(stock, &mut counts[2]),
                    total(available, &mut counts[3]),
                ],
            ));
        }
        Ok((totals, counts))
    }

    /// The spellings LexicalTokens holds of `tokens` not sent in this segment yet.
    fn spellings(
        &mut self,
        state: &segments::State,
        tokens: impl Iterator<Item = u32>,
    ) -> io::Result<Vec<(u32, String)>> {
        let held = columns::table(state, TableId::LexicalTokens)?;
        let mut spellings = Vec::new();
        for token in tokens {
            if self.spelled.contains(&token) {
                continue;
            }
            if let Some(slot) = held.slot(u128::from(token)) {
                let literal = self.literal(state, slot)?;
                spellings.push((token, String::from_utf8_lossy(&literal).into_owned()));
                self.spelled.insert(token);
            }
        }
        Ok(spellings)
    }

    /// The reads of the holder slots due at `reads`: every holder's after a whole hour, else
    /// those whose rows changed since their last read here.
    fn reads(
        &mut self,
        state: &segments::State,
        [stock_read, logistics_read]: [bool; 2],
    ) -> io::Result<Vec<Read>> {
        let _cpu = replay_frame::cpu::scope("stockpile.columnar.reads", None);
        let mut reads: BTreeMap<u32, Read> = BTreeMap::new();
        for (group, due) in [stock_read, logistics_read].into_iter().enumerate() {
            if !due {
                continue;
            }
            let sent: Vec<u32> = if self.stale[group] {
                self.slots.clone()
            } else {
                self.dirty[group]
                    .iter()
                    .copied()
                    .filter(|slot| self.slots.binary_search(slot).is_ok())
                    .collect()
            };
            self.stale[group] = false;
            self.dirty[group].clear();
            for slot in sent {
                let mut read = reads.remove(&slot).unwrap_or_else(|| Read::new(slot));
                if group == 0 {
                    if self.usable[0] {
                        let (totals, overflowed) = self.stock_totals(state, slot)?;
                        read.counts[0] = overflowed;
                        read.stock = Some(update(
                            &mut self.sent.entry(slot).or_default().stock,
                            totals,
                        ));
                    }
                    if self.usable[1] {
                        let (totals, counts) = self.line_totals(state, slot)?;
                        read.counts[1..7].copy_from_slice(&counts);
                        read.lines = Some(update(
                            &mut self.sent.entry(slot).or_default().lines,
                            totals,
                        ));
                    }
                } else if self.usable[2] {
                    let (totals, counts) = self.logistics_totals(state, slot)?;
                    read.counts[7..11].copy_from_slice(&counts);
                    if columns::complete(state, TableId::LexicalTokens) {
                        read.spellings =
                            self.spellings(state, totals.iter().map(|&(token, _)| token))?;
                    }
                    read.logistics = Some(update(
                        &mut self.sent.entry(slot).or_default().logistics,
                        totals,
                    ));
                }
                reads.insert(slot, read);
            }
        }
        Ok(reads
            .into_values()
            .filter(|read| read.stock.is_some() || read.lines.is_some() || read.logistics.is_some())
            .collect())
    }

    fn heap(&self) -> usize {
        self.slots.capacity() * size_of::<u32>()
            + self.owners.iter().map(Owners::heap).sum::<usize>()
            + self
                .dirty
                .iter()
                .map(|dirty| dirty.capacity() * size_of::<u32>())
                .sum::<usize>()
            + self.sent.capacity() * size_of::<(u32, Sent)>()
            + self.sent.values().map(Sent::heap).sum::<usize>()
            + self.spelled.capacity() * size_of::<u32>()
            + self.naming.heap()
    }
}

// Every segment starts cold: its first hour reads every holder from its rows again; what came
// before is the emit's.
impl Operator<World> for Projector {
    type Output = Hour;

    fn start(&mut self, _: segments::Start<'_>, _: u64, _: i32) -> io::Result<()> {
        self.cold = true;
        self.spelled.clear();
        Ok(())
    }

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("stockpile.columnar", Some("stockpile"));
        let state = hour.state;
        let (roster, names) = (hour.world.countries()?, hour.world.naming()?);
        let opens = self.cold
            || hour.starts
            || hour.opens
            || self.last.and_then(|last| last.checked_add(1)) != Some(hour.hour);
        self.last = Some(hour.hour);
        let mut changed = [false; TABLE_COUNT];
        let mut shaped = [false; TABLE_COUNT];
        for &table in TABLES {
            let index = table.index();
            let status = columns::status(state, table);
            let edits = columns::changes(state, table)?;
            shaped[index] = self.shapes[index] != Some(status);
            changed[index] = opens
                || shaped[index]
                || edits.truncated
                || !edits.changed.is_empty()
                || !edits.fill_slots.is_empty()
                || !edits.inserted.is_empty()
                || !edits.deleted.is_empty();
            self.shapes[index] = Some(status);
        }
        let holders = changed[TableId::CountryLinks.index()]
            || shaped[TableId::Countries.index()]
            || roster.changed();
        if holders {
            let resolved = self::holders(roster);
            self.slots = resolved
                .identities
                .values()
                .map(|&(slot, _)| slot)
                .collect();
            self.slots.sort_unstable();
            self.slots.dedup();
            self.holders = Arc::new(resolved);
            self.uncharged = true;
        }
        self.complete = NUMERIC.map(|table| columns::complete(state, table));
        let countries = roster.complete();
        let usable = self.complete.map(|complete| complete && countries);
        // As a row block reads the whole tables where a numeric table became or stopped
        // being complete, or the holders changed.
        let whole = opens || holders || usable != self.usable;
        self.usable = usable;
        self.index(state, roster, whole, &changed, &shaped)?;
        if whole {
            self.dirty = Default::default();
            self.stale = [true; 2];
            self.sent.clear();
        }
        let flags =
            dashboard_text::metadata::slot_reads(self.format, &metrics(hour.manifest), hour.hour);
        let reads = [
            flags[TableId::Stockpiles.index()],
            flags[TableId::LogisticsEquipment.index()],
        ];
        let (evidence, variants) = {
            let _cpu = replay_frame::cpu::scope("stockpile.columnar.naming", None);
            self.naming.project(
                state,
                (roster, names),
                &self.fields,
                self.format,
                hour.hour,
                whole,
                &shaped,
                &changed,
                flags[TableId::VariantDefinitions.index()],
            )?
        };
        let rows = if reads[0] || reads[1] {
            self.reads(state, reads)?
        } else {
            Vec::new()
        };
        self.cold = false;
        // A complete variant read that built nothing leaves every name as it is.
        let quiet = !whole
            && !reads[0]
            && !reads[1]
            && matches!(
                evidence,
                VariantHour::Carried { complete: true } | VariantHour::Read
            )
            && variants.is_empty();
        let facts = if quiet {
            None
        } else {
            let variants = {
                let _cpu = replay_frame::cpu::scope("stockpile.columnar.digests", None);
                variants
                    .into_iter()
                    .map(|(id, raw)| Variant::new(id, raw))
                    .collect()
            };
            Some(Box::new(Facts {
                reads,
                countries,
                usable,
                holders: whole.then(|| Arc::clone(&self.holders)),
                charged: whole && std::mem::take(&mut self.uncharged),
                evidence,
                variants,
                rows,
            }))
        };
        out.push(Hour {
            branch: hour.branch,
            hour: hour.hour,
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
                "Not a saved stockpile projector",
            ));
        }
        *self = Self::new(self.format)?;
        Ok(())
    }
}

/// Whether the hour changed a row of `table` in a field of `mask`, inserted or deleted one,
/// or read it in part.
fn relevant(state: &segments::State, table: TableId, mask: u64) -> io::Result<bool> {
    let edits = columns::changes(state, table)?;
    Ok(edits.truncated
        || !edits.inserted.is_empty()
        || !edits.deleted.is_empty()
        || edits
            .masks
            .iter()
            .chain(&edits.fill_masks)
            .any(|written| written & mask != 0))
}

/// What a projected hour read, for tests.
#[cfg(test)]
pub(crate) struct Shape {
    /// Each slot read, ascending, with whether its stock, lines and logistics totals were
    /// whole.
    pub(crate) reads: Vec<(u32, [Option<bool>; 3])>,
}

#[cfg(test)]
impl Shape {
    pub(crate) fn slots(&self) -> Vec<u32> {
        self.reads.iter().map(|&(slot, _)| slot).collect()
    }

    pub(crate) fn whole(&self, slot: u32, table: usize) -> Option<bool> {
        self.reads
            .iter()
            .find(|(read, _)| *read == slot)
            .and_then(|(_, whole)| whole[table])
    }
}

#[cfg(test)]
impl Hour {
    pub(crate) fn shape(&self) -> Shape {
        fn whole<K, const N: usize>(totals: &Option<Totals<K, N>>) -> Option<bool> {
            totals
                .as_ref()
                .map(|totals| matches!(totals, Totals::Whole(_)))
        }
        Shape {
            reads: self
                .facts
                .iter()
                .flat_map(|facts| &facts.rows)
                .map(|read| {
                    (
                        read.slot,
                        [
                            whole(&read.stock),
                            whole(&read.lines),
                            whole(&read.logistics),
                        ],
                    )
                })
                .collect(),
        }
    }
}
