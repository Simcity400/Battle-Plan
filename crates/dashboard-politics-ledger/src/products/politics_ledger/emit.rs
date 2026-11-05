//! The ledger's emit: the accounting of each projected hour ([`facts::Hour`]), written as the
//! family's series publication. A country's native rows are recorded once, as the read that first
//! named their identity's tag saw them, each as the reply shows it ([`super::records`]), and each
//! item once, named at its first payment; its spending coverage, holds and spending parts as
//! runs and events that change only when they do; the census as the branch's,
//! and each tag's presence in it where that changes. An hour that changes nothing of a country
//! appends nothing.
//!
//! The emit holds what only earlier hours know: tags by capture identity, which rows were
//! recorded, the rows waiting for a tag, the native tables as the hour leaves them, each
//! country's open coverage, holds and day parts, and warning rates. A reply reads everything
//! else back from the publication ([`super::query`]).

use super::facts::{self, CensusRow, Cost, Government, Hour, Power};
use super::records::{self, CostRecord, DailyRecord, PowerRecord};
use super::rows::Holder;
use super::tally::{self, Contribution};
use super::*;
use super::{items, present};
use dashboard_base::prepared::codec;
use dashboard_base::products::politics_content::PoliticsContent;
use dashboard_series::{
    Cell, Event, Evidence, Knownness, RunMode, Span, Writer,
    carry::Carry,
    codec::Decode,
    family::{Actors, Directory, Place},
};
use dashboard_series::hash::{FastMap, Words};
use std::{collections::HashSet, hash::BuildHasherDefault};

/// A set keyed by [`Words`], as [`FastMap`] is.
type FastSet<K> = HashSet<K, BuildHasherDefault<Words>>;

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 10;

/// Each country's measures: its power rows by currency, at their read; its governments, daily
/// rows and costs, at their native hour; whether its spending in each currency is recorded, with
/// the reasons; its hold snapshots; its spending parts, by dimension.
pub const POWER: [u64; 2] = [0, 1];
pub const GOVERNMENTS: u64 = 2;
pub const DAILY: u64 = 3;
pub const COSTS: u64 = 4;
pub const COVERED: [u64; 2] = [7, 8];
pub const COVER_REASONS: u64 = 9;
pub const HOLDS: u64 = 10;
pub const PARTS: u64 = 13;
/// Each country's settled totals by item and period ([`super::tally`]), at the period's last
/// hour; its payments read after their day settled, at their read; each item as replies show
/// it ([`records::ItemRecord`]), at its first payment; whether the census lists it present and
/// refused, where that changes.
pub const ROLLUP: u64 = 15;
pub const LATE: u64 = 16;
pub const ITEMS: u64 = 17;
pub const PRESENT: u64 = 18;
/// The branch's measures, under the actor with the empty name: whether the census is complete,
/// and every prepared hour.
pub const CENSUS: u64 = 20;
pub const ACCEPTED: u64 = 21;

/// Event kinds.
pub const POWER_KIND: u64 = 1;
pub const GOVERNMENT_KIND: u64 = 2;
pub const DAILY_KIND: u64 = 3;
pub const COST_KIND: u64 = 4;
pub const COVER_KIND: u64 = 6;
pub const HOLD_KIND: u64 = 7;
pub const PRESENCE_KIND: u64 = 9;
/// A rollup's kind is this plus its level.
pub const ROLLUP_KIND: u64 = 10;

/// What a power row or cost is to a reply's payments ([`records`]).
pub const PAYMENT: u64 = 0;
pub const BALANCE: u64 = 1;
pub const SKIPPED: u64 = 2;
pub const UNMATCHED: u64 = 3;
pub const CREDIT: u64 = 4;

/// Family scalars: the first prepared hour; the native day from which days are still open; the
/// branch's warnings; the recorded content's power warnings, once an item reached them; the
/// first hour the census listed its tags; and by actor, each country's warnings and its
/// categories' first hours.
pub const FIRST: u64 = 0;
pub const SETTLED: u64 = 1;
pub const WARNINGS: u64 = 2;
pub const POWER_WARNINGS: u64 = 3;
pub const PRESENCE_FIRST: u64 = 4;
pub const TAG_WARNINGS: u64 = 1 << 24;
pub const TAG_CATEGORIES: u64 = 2 << 24;
/// Each category's spelling (`Currency:category`), by its part dimension.
pub const CATEGORY_NAMES: u64 = 3 << 24;

/// The parts every country has, by dimension.
pub const PART_FIELDS: [&str; 10] = [
    "Political:income",
    "Political:costs",
    "Political:wrapping",
    "Political:income-total",
    "Command:income",
    "Command:income-total",
    "Political:zero",
    "Political:spending-total",
    "Command:zero",
    "Command:spending-total",
];
/// A category's part dimension: this plus its number.
pub const CATEGORY_PARTS: u64 = 64;
pub const ZERO: [u64; 2] = [6, 8];
pub const TOTAL: [u64; 2] = [7, 9];
/// Fixed5 values: runs of these measures carry five decimals.
pub const DECIMALS: u8 = 5;

/// Every key's evidence: each prepared hour is known.
const DOMAIN: u64 = 0;
/// The branch's actor.
pub const BRANCH: &[u8] = b"";
const CURRENCIES: [&str; 2] = ["Political", "Command"];

fn invalid(what: &str) -> io::Error {
    dashboard_core::invalid(format!("Politics ledger emit: {what}"))
}

fn text(writer: &mut Writer<'_>, bytes: &[u8]) -> io::Result<Cell> {
    if bytes.is_empty() {
        return Ok(Cell::Text(dashboard_series::Blob::EMPTY));
    }
    Ok(Cell::Text(writer.bytes(bytes)?))
}

/// The native day a power row closes: daily upkeep and gains only.
fn power_day(row: &CountryPowerChangeRow) -> Option<i32> {
    matches!(row.cause, PowerCause::AbilityUpkeep | PowerCause::DailyGain)
        .then(|| row.hour.div_euclid(24) * 24)
}

/// A tag's capture coverage at a read, as its account reads and writes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Coverage {
    country: Option<u32>,
    instance: Option<u32>,
    since_hour: Option<i32>,
    through_hour: Option<i32>,
    epoch: Option<u64>,
    streams: u8,
    reason: Option<Gap>,
}

/// Why a read holds no current capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Gap {
    Unavailable,
    Stale,
}

impl Gap {
    fn text(self) -> &'static str {
        match self {
            Self::Unavailable => "country identity or politics coverage unavailable",
            Self::Stale => "politics coverage is stale at this read",
        }
    }
}

impl Coverage {
    /// A capture as a read's coverage names it.
    fn of(row: &PoliticsCaptureCoverageRow) -> Self {
        Self {
            country: Some(row.country),
            instance: Some(row.instance),
            since_hour: Some(row.since_hour),
            through_hour: Some(row.through_hour),
            epoch: Some(row.epoch),
            streams: row.streams,
            reason: None,
        }
    }

    fn identity(&self) -> Option<(u32, u64)> {
        self.country.zip(self.epoch)
    }

    /// Whether `value` continues `self`, the coverage a country's last read gave.
    fn continued_by(&self, value: &Self) -> bool {
        self.country == value.country
            && self.instance == value.instance
            && self.epoch == value.epoch
            && self.streams == value.streams
            && self.reason == value.reason
            && self.through_hour.and_then(|hour| hour.checked_add(1)) == value.through_hour
            && self
                .through_hour
                .zip(value.since_hour)
                .is_some_and(|(last, first)| first <= last.saturating_add(1))
    }
}

/// A row waiting for its identity's tag.
enum Waiting {
    Power(Power),
    Cost(Cost),
    Government(Government),
}

/// A recorded daily row.
struct Day {
    row: CountryPowerDailyRow,
    reads: (Option<i32>, Option<i32>),
    tag: u32,
}

/// A held power row: what its deletion and the account's indexes read of it.
#[derive(Clone, Copy)]
struct HeldPower {
    country: u32,
    epoch: u64,
    sequence: u64,
    hour: i32,
    index: Indexed,
}

/// The account's index a held power row is in ([`Emit::index_power`]).
#[derive(Clone, Copy)]
enum Indexed {
    None,
    DailyGain,
    /// An addition without an evaluated operand, by its currency's capture bit.
    UnknownAdd(u8),
}

impl HeldPower {
    fn of(row: &CountryPowerChangeRow) -> Self {
        let index = if row.cause == PowerCause::DailyGain && row.currency == PowerCurrency::Political
        {
            Indexed::DailyGain
        } else if row.operation == PowerOperation::Add && row.requested_delta_raw.is_none() {
            Indexed::UnknownAdd(match row.currency {
                PowerCurrency::Political => PoliticsCaptureCoverageRow::PP,
                PowerCurrency::Command => PoliticsCaptureCoverageRow::CP,
            })
        } else {
            Indexed::None
        };
        Self {
            country: row.country,
            epoch: row.epoch,
            sequence: row.sequence,
            hour: row.hour,
            index,
        }
    }
}

/// A held cost: what its day's sums and records read of it.
#[derive(Clone, Copy, Serialize, Deserialize)]
struct HeldCost {
    country: u32,
    epoch: u64,
    day_hour: i32,
    pp_sequence: u64,
    ordinal: u32,
    cost_raw: i64,
}

impl HeldCost {
    fn of(row: &CountryPowerDailyCostRow) -> Self {
        Self {
            country: row.country,
            epoch: row.epoch,
            day_hour: row.day_hour,
            pp_sequence: row.pp_sequence,
            ordinal: row.ordinal,
            cost_raw: row.cost_raw,
        }
    }

    /// Its row's key in the cost table.
    fn key(&self) -> u128 {
        Self::slot(self.country, self.ordinal)
    }

    /// The cost table's key of `country`'s cost `ordinal`.
    fn slot(country: u32, ordinal: u32) -> u128 {
        CountryPowerDailyCostRow {
            country,
            ordinal,
            ..CountryPowerDailyCostRow::default()
        }
        .key()
    }

    fn application(&self) -> (u32, u64, i32, u64) {
        (self.country, self.epoch, self.day_hour, self.pp_sequence)
    }
}

/// What one country's costs of a day and application read alike at an hour
/// ([`Emit::cost_day`]).
#[derive(Clone, Copy)]
struct CostDay {
    of: (u32, u64, i32, u64),
    tag: Option<u32>,
    covered: bool,
    late: bool,
    matched: bool,
    read: i32,
}

/// A recorded cost: what its revisions keep.
struct Recorded {
    row: CountryPowerDailyCostRow,
    covered: bool,
    item: u32,
}

/// A hold snapshot's operands: its total, holders and why its total is missing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Snapshot {
    total: Option<i64>,
    holders: Vec<(String, Holder)>,
    missing: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct Holds {
    identity: Option<(u32, u64)>,
    sequence: Option<u64>,
    last: Option<i32>,
    active: BTreeMap<String, Holder>,
    snapshot: Option<Snapshot>,
}

/// A capture identity's spending parts of one day.
#[derive(Default, Serialize, Deserialize)]
struct DayParts {
    /// The capture epoch they are of: another's start over.
    epoch: Option<u64>,
    day: i32,
    values: BTreeMap<u64, Option<i128>>,
    since: [Option<i32>; 2],
    last: Option<i32>,
    first: Option<i32>,
}

/// One tag's accounting.
#[derive(Default, Serialize, Deserialize)]
struct Country {
    actor: u64,
    #[serde(skip)]
    slots: Vec<u32>,
    active: bool,
    coverage: Option<Coverage>,
    spending: Option<LedgerSpendCoverage>,
    cover: Option<(bool, bool, Option<String>)>,
    identity: Option<(u32, u64)>,
    holds: Holds,
    parts: BTreeMap<u32, DayParts>,
    /// Each category's first hour, by part dimension.
    categories: BTreeMap<u64, i32>,
    categories_changed: bool,
    /// Warning rates counted for every accepted hour after `rated`.
    rates: BTreeMap<String, u64>,
    rated: Option<i32>,
    warnings: BTreeMap<String, u64>,
    warnings_changed: bool,
    /// Whether the census last listed it present, and refused.
    #[serde(default)]
    presence: (bool, bool),
    /// Its items' keys in the order first paid.
    #[serde(skip)]
    items: Vec<[u8; 32]>,
    #[serde(skip)]
    item_index: FastMap<[u8; 32], u32>,
    /// Saved in the binary part with `items` ([`Emit::save`]).
    #[serde(skip)]
    rollups: tally::Rollups,
    /// Its series keys as interned, by measure and by part dimension: an account finds each
    /// without hashing.
    #[serde(skip)]
    keys: [Option<u64>; MEASURES],
    #[serde(skip)]
    part_keys: Vec<Option<u64>>,
}

/// The measures a tag's keys are cached for: every one below [`CENSUS`].
const MEASURES: usize = CENSUS as usize;

impl Country {
    /// Counts its rates through `through`.
    fn materialize(&mut self, through: i32) {
        let Some(rated) = self.rated else {
            return;
        };
        let hours = u64::try_from(i64::from(through) - i64::from(rated)).unwrap_or(0);
        if hours != 0 {
            for (reason, rate) in &self.rates {
                *self.warnings.entry(reason.clone()).or_default() += rate * hours;
                self.warnings_changed = true;
            }
            self.rated = Some(through);
        }
    }

    /// Its rates from `hour` on, `hour` counted now.
    fn rate(&mut self, hour: i32, rates: BTreeMap<String, u64>) {
        self.materialize(hour.saturating_sub(1));
        for (reason, rate) in &rates {
            *self.warnings.entry(reason.clone()).or_default() += rate;
            self.warnings_changed = true;
        }
        self.rated = (!rates.is_empty()).then_some(hour);
        self.rates = rates;
    }
}

/// A payment recorded this hour, for its country's day parts.
struct Paid {
    currency: PowerCurrency,
    category: Arc<str>,
    hour: i32,
    charge: Option<i128>,
}

/// Tags by number, as one bit each: the accounting takes them in ascending order.
#[derive(Default)]
struct Tags(Vec<u64>);

impl Tags {
    fn insert(&mut self, tag: u32) {
        let word = (tag / 64) as usize;
        if word >= self.0.len() {
            self.0.resize(word + 1, 0);
        }
        self.0[word] |= 1 << (tag % 64);
    }

    fn iter(&self) -> Ascending {
        Ascending {
            words: self.0.clone(),
            at: 0,
        }
    }
}

impl FromIterator<u32> for Tags {
    fn from_iter<I: IntoIterator<Item = u32>>(tags: I) -> Self {
        let mut set = Self::default();
        set.extend(tags);
        set
    }
}

impl Extend<u32> for Tags {
    fn extend<I: IntoIterator<Item = u32>>(&mut self, tags: I) {
        for tag in tags {
            self.insert(tag);
        }
    }
}

impl IntoIterator for Tags {
    type Item = u32;
    type IntoIter = Ascending;

    fn into_iter(self) -> Ascending {
        Ascending {
            words: self.0,
            at: 0,
        }
    }
}

/// The tags of [`Tags`], ascending.
struct Ascending {
    words: Vec<u64>,
    at: usize,
}

impl Iterator for Ascending {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        while let Some(word) = self.words.get_mut(self.at) {
            if *word != 0 {
                let bit = word.trailing_zeros();
                *word &= *word - 1;
                return Some(self.at as u32 * 64 + bit);
            }
            self.at += 1;
        }
        None
    }
}

/// What the hour joins, as the accounting reads it.
#[derive(Default)]
struct Joined {
    /// Tags whose inputs the hour changed.
    dirty: Tags,
    /// Command power rows recorded this hour by identity: covered, and their holder's operands.
    command: FastMap<(u32, u64), Vec<(CountryPowerChangeRow, bool, Option<Holder>)>>,
    /// Costs recorded this hour by tag: their amount and whether their application matched.
    costs: FastMap<u32, Vec<(i64, bool)>>,
    payments: FastMap<u32, Vec<Paid>>,
    /// Raw countries with a political daily gain read before its application.
    unmatched: FastSet<u32>,
}

/// The family's emit on one branch.
pub struct Emit {
    actors: Actors,
    carry: Carry,
    /// The recorded content items and holders are named with.
    content: Option<Arc<PoliticsContent>>,
    content_failure: Option<String>,
    /// Whether the content's power warnings were written since the emit began.
    power_warned: bool,
    last: Option<i32>,
    /// The first hour of the accepted stretch not yet given its evidence.
    known_from: Option<i32>,
    begun: bool,
    census: BTreeMap<u32, CensusRow>,
    census_complete: bool,
    known: FastMap<u32, u32>,
    tags: Vec<Box<str>>,
    numbers: FastMap<Box<str>, u32>,
    countries: Vec<Country>,
    slot_tags: FastMap<u32, u32>,
    refused: BTreeMap<u32, String>,
    links: FastMap<u32, u32>,
    raw_slots: FastMap<u32, Vec<u32>>,
    witnesses: FastMap<u32, PoliticsCaptureCoverageRow>,
    extended: Arc<[u32]>,
    extended_through: i32,
    /// The raw countries whose capture moved on at the last hour, ascending.
    advanced: Vec<u32>,
    /// The extended list and the captures that followed, in the order handed, `advanced` was
    /// made of: an hour handing both again leaves it as it was.
    advanced_from: Option<(Arc<[u32]>, Vec<u32>)>,
    /// Tags whose last account read inputs of its hour alone: accounted again at the next.
    volatile: Tags,
    identities: FastMap<(u32, u64), Vec<(i32, u32)>>,
    complete: u16,
    power: FastMap<u128, HeldPower>,
    /// Of `power`, the political daily gains by raw country and hour, and the additions without
    /// an evaluated operand by raw country, epoch, hour and currency bit: what an account asks
    /// of a country's rows, found without walking them.
    daily_gains: BTreeSet<(u32, i32, u128)>,
    unknown_adds: BTreeSet<(u32, u64, i32, u8, u128)>,
    /// The held government rows' identities.
    governments: FastMap<u128, (u32, u64, u64)>,
    recorded: FastSet<(u32, u64, u64)>,
    waiting: BTreeMap<(u8, u128), Waiting>,
    daily: FastMap<u32, CountryPowerDailyRow>,
    days: BTreeMap<(u32, u64, i32), Day>,
    costs: FastMap<u128, HeldCost>,
    recorded_costs: FastMap<(u32, u64, i32, u32), Recorded>,
    sums: FastMap<(u32, u64, i32, u64), (i64, u32)>,
    command: FastMap<u32, i64>,
    presence: Option<(Vec<Box<str>>, Vec<Box<str>>)>,
    settled: Option<i32>,
    settled_written: Option<i32>,
    /// The frontier the rollups were last settled at.
    rolled: Option<i32>,
    warnings: BTreeMap<String, u64>,
    warnings_changed: bool,
    /// Each currency's categories by name, with their part dimension.
    categories: [FastMap<Box<str>, u64>; 2],
    category_currency: FastMap<u64, usize>,
    tomorrow: BTreeMap<i32, BTreeSet<u32>>,
    /// The branch's accepted and census keys as interned.
    branch_keys: [Option<u64>; 2],
    scratch: Scratch,
}

/// The buffers each account's parts reuse ([`Emit::write_parts`]).
#[derive(Default)]
struct Scratch {
    values: Vec<(u64, Option<i128>)>,
    categories: Vec<(u64, Option<usize>)>,
}

impl Emit {
    pub fn new(content: &Result<Option<Arc<PoliticsContent>>, String>) -> Self {
        Self {
            actors: Actors::new(SCHEMA),
            carry: Carry::default(),
            content: content.as_ref().ok().cloned().flatten(),
            content_failure: content.as_ref().err().cloned(),
            power_warned: false,
            last: None,
            known_from: None,
            begun: false,
            census: BTreeMap::new(),
            census_complete: false,
            known: FastMap::default(),
            tags: Vec::new(),
            numbers: FastMap::default(),
            countries: Vec::new(),
            slot_tags: FastMap::default(),
            refused: BTreeMap::new(),
            links: FastMap::default(),
            raw_slots: FastMap::default(),
            witnesses: FastMap::default(),
            extended: Arc::from(Vec::new()),
            extended_through: 0,
            advanced: Vec::new(),
            advanced_from: None,
            volatile: Tags::default(),
            identities: FastMap::default(),
            complete: 0,
            power: FastMap::default(),
            daily_gains: BTreeSet::new(),
            unknown_adds: BTreeSet::new(),
            governments: FastMap::default(),
            recorded: FastSet::default(),
            waiting: BTreeMap::new(),
            daily: FastMap::default(),
            days: BTreeMap::new(),
            costs: FastMap::default(),
            recorded_costs: FastMap::default(),
            sums: FastMap::default(),
            command: FastMap::default(),
            presence: None,
            settled: None,
            settled_written: None,
            rolled: None,
            warnings: BTreeMap::new(),
            warnings_changed: false,
            categories: [FastMap::default(), FastMap::default()],
            category_currency: FastMap::default(),
            tomorrow: BTreeMap::new(),
            branch_keys: [None; 2],
            scratch: Scratch::default(),
        }
    }

    /// The tag spelled `spelling`, numbered and catalogued as an actor when new.
    fn tag(&mut self, writer: &mut Writer<'_>, spelling: &str, hour: i32) -> io::Result<u32> {
        if let Some(&tag) = self.numbers.get(spelling) {
            return Ok(tag);
        }
        let actor = self.actors.actor(writer, spelling.as_bytes(), hour)?;
        let tag = u32::try_from(self.tags.len()).map_err(|_| invalid("tags exhausted"))?;
        self.tags.push(spelling.into());
        self.numbers.insert(spelling.into(), tag);
        self.countries.push(Country {
            actor,
            ..Country::default()
        });
        Ok(tag)
    }

    fn key(
        &mut self,
        writer: &mut Writer<'_>,
        actor: u64,
        measure: u64,
        dimension: Option<u64>,
    ) -> io::Result<u64> {
        let decimals = if measure == PARTS {
            DECIMALS
        } else {
            0
        };
        self.actors.key(
            writer,
            Place {
                actor,
                measure,
                dimension,
            },
            (RunMode::Exact, decimals),
            Some(DOMAIN),
        )
    }

    fn tag_key(
        &mut self,
        writer: &mut Writer<'_>,
        tag: u32,
        measure: u64,
        dimension: Option<u64>,
    ) -> io::Result<u64> {
        let country = &self.countries[tag as usize];
        let held = match dimension {
            None => country.keys.get(measure as usize).copied().flatten(),
            Some(dimension) if measure == PARTS => {
                country.part_keys.get(dimension as usize).copied().flatten()
            }
            Some(_) => None,
        };
        if let Some(key) = held {
            return Ok(key);
        }
        let key = self.key(writer, country.actor, measure, dimension)?;
        let country = &mut self.countries[tag as usize];
        match dimension {
            None => {
                if let Some(slot) = country.keys.get_mut(measure as usize) {
                    *slot = Some(key);
                }
            }
            Some(dimension) if measure == PARTS => {
                let at = usize::try_from(dimension).map_err(|_| invalid("part dimension"))?;
                if at >= country.part_keys.len() {
                    country.part_keys.resize(at + 1, None);
                }
                country.part_keys[at] = Some(key);
            }
            Some(_) => {}
        }
        Ok(key)
    }

    fn branch_key(&mut self, writer: &mut Writer<'_>, measure: u64, hour: i32) -> io::Result<u64> {
        let at = match measure {
            ACCEPTED => Some(0),
            CENSUS => Some(1),
            _ => None,
        };
        if let Some(&Some(key)) = at.and_then(|at| self.branch_keys.get(at)) {
            return Ok(key);
        }
        let actor = self.actors.actor(writer, BRANCH, hour)?;
        let key = self.key(writer, actor, measure, None)?;
        if let Some(at) = at {
            self.branch_keys[at] = Some(key);
        }
        Ok(key)
    }

    fn event(
        writer: &mut Writer<'_>,
        key: u64,
        hours: (i32, Option<i32>),
        kind: u64,
        cells: &[Cell],
    ) -> io::Result<u64> {
        let fields = writer.cells(cells.iter().copied())?;
        Self::put(writer, key, hours, kind, fields)
    }

    fn put(
        writer: &mut Writer<'_>,
        key: u64,
        (native, read): (i32, Option<i32>),
        kind: u64,
        fields: dashboard_series::Blob,
    ) -> io::Result<u64> {
        let sequence = writer.sequence()?;
        writer.event(Event {
            key,
            native_hour: native,
            read_hour: read,
            sequence,
            kind,
            fields,
        })?;
        Ok(sequence)
    }

    /// The tag of capture identity (`country`, `epoch`) at `hour`: the latest begun by then,
    /// else its first.
    fn tag_at(&self, country: u32, epoch: u64, hour: i32) -> Option<u32> {
        let tags = self.identities.get(&(country, epoch))?;
        let at = tags.partition_point(|&(begun, _)| begun <= hour);
        at.checked_sub(1)
            .and_then(|at| tags.get(at))
            .or_else(|| tags.first())
            .map(|&(_, tag)| tag)
    }

    /// Raw country `raw`'s capture as of the hour.
    fn witness(&self, raw: u32) -> Option<PoliticsCaptureCoverageRow> {
        let mut row = self.witnesses.get(&raw)?.clone();
        if self.extended.binary_search(&raw).is_ok() {
            row.through_hour = self.extended_through;
        }
        Some(row)
    }

    /// `slot`'s capture: that of its last linked raw country holding one.
    fn slot_witness(&self, slot: u32) -> Option<PoliticsCaptureCoverageRow> {
        self.raw_slots
            .get(&slot)?
            .iter()
            .rev()
            .find_map(|&raw| self.witness(raw))
    }

    fn covered(&self, country: u32, epoch: u64, event: i32, bits: u8, hour: i32) -> bool {
        self.witness(country).is_some_and(|row| {
            row.epoch == epoch
                && row.through_hour == hour
                && row.streams & bits == bits
                && event >= row.since_hour
                && event <= row.through_hour
        })
    }

    /// The valid tag of the slot `raw` links to.
    fn raw_tag(&self, raw: u32) -> Option<u32> {
        self.links
            .get(&raw)
            .and_then(|slot| self.slot_tags.get(slot))
            .copied()
    }

    /// Item `ordinal` of `tag`, first paid by `row` (of `cause`, with `component`), as replies
    /// show it: named from the facts its hour recorded, `dated`, and written to the catalogue.
    #[allow(clippy::too_many_arguments)]
    fn show(
        &mut self,
        writer: &mut Writer<'_>,
        tag: u32,
        ordinal: u32,
        hours: (i32, i32),
        first: String,
        row: &PaidRow,
        (identity, cause, component): (&PowerIdentity, PowerCause, Option<PowerDailyCostKind>),
        dated: Option<&facts::DatedFacts>,
    ) -> io::Result<()> {
        let dated = dated
            .map(|dated| codec::decode::<items::Dated>(&dated.bytes))
            .transpose()?;
        let shown = present::show(
            &self.tags[tag as usize],
            ordinal,
            first,
            row,
            identity,
            cause,
            dated.as_ref(),
            component,
            self.content.as_deref(),
        );
        let key = self.tag_key(writer, tag, ITEMS, None)?;
        let cells = [
            Cell::Unsigned(u64::from(ordinal)),
            records::cell(writer, &shown)?,
        ];
        let kind = if component.is_some() { COST_KIND } else { POWER_KIND };
        Self::event(writer, key, (hours.0, Some(hours.1)), kind, &cells)?;
        Ok(())
    }

    fn warn(&mut self, reason: String, count: u64) {
        *self.warnings.entry(reason).or_default() += count;
        self.warnings_changed = true;
    }

    /// Gives the accepted stretch since its first hour its evidence, through `last`.
    fn evidence(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        if let Some(first) = self.known_from.take() {
            writer.domain(
                DOMAIN,
                Evidence {
                    span: Span::new(first, last)?,
                    known: Knownness::Known,
                    supplying_read: None,
                    holder: None,
                    identity_first: None,
                },
            )?;
        }
        Ok(())
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer<'_>, hour: Hour) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("politics_ledger.emit", None);
        let at = hour.hour;
        let previous = self.last;
        let adjacent = previous.and_then(|last| last.checked_add(1)) == Some(at);
        if let Some(last) = previous.filter(|_| !adjacent) {
            for country in &mut self.countries {
                country.materialize(last);
                country.rated = None;
                country.coverage = None;
                country.spending = None;
                country.cover = None;
                country.identity = None;
                country.holds.last = None;
            }
            self.evidence(writer, last)?;
            self.carry.close(writer, last)?;
        }
        if !self.begun {
            writer.catalogue(FIRST, Cell::Integer(i128::from(at)))?;
            if let Some(reason) = self.content_failure.clone() {
                // The content's refusal is the branch's once, whatever its hours.
                self.warnings.entry(reason).or_insert(1);
                self.warnings_changed = true;
            }
            self.begun = true;
        }
        self.known_from.get_or_insert(at);
        self.last = Some(at);
        let accepted = self.branch_key(writer, ACCEPTED, at)?;
        self.carry.value(writer, accepted, at, Some(1), !adjacent)?;
        let census_complete = hour.complete & facts::bit(TableId::Countries) != 0;
        let census_key = self.branch_key(writer, CENSUS, at)?;
        self.carry.value(
            writer,
            census_key,
            at,
            census_complete.then_some(1),
            !adjacent,
        )?;
        let mut joined = Joined::default();
        joined.dirty.extend(std::mem::take(&mut self.volatile));
        if !adjacent || hour.complete != self.complete {
            joined.dirty.extend(0..self.countries.len() as u32);
        }
        self.complete = hour.complete;
        while let Some(entry) = self.tomorrow.first_entry() {
            if *entry.key() > at {
                break;
            }
            joined.dirty.extend(entry.remove());
        }
        let identities = ::replay_frame::cpu::scope("politics_ledger.emit.identities", None);
        let census_changed = hour.census.is_some() || census_complete != self.census_complete;
        self.census_complete = census_complete;
        if census_changed {
            let census: Vec<CensusRow> = if census_complete {
                match hour.census.as_deref() {
                    Some(census) => census.to_vec(),
                    None => self.census.values().cloned().collect(),
                }
            } else {
                Vec::new()
            };
            self.apply_census(writer, &census, at, &mut joined)?;
        }
        let links_changed = hour.links.is_some();
        if let Some(links) = hour.links.as_deref() {
            self.apply_links(links, &mut joined);
        }
        let touched = self.apply_captures(&hour.captures, at, hour.whole, &mut joined);
        let raws: Vec<u32> = if census_changed || links_changed {
            self.witnesses.keys().copied().collect()
        } else {
            touched
        };
        let tagged = self.tag_identities(raws, at);
        if census_changed {
            self.presence(writer, at)?;
        }
        drop(identities);
        let rows = ::replay_frame::cpu::scope("politics_ledger.emit.rows", None);
        self.apply_rows(writer, hour, at, previous, tagged, &mut joined)?;
        drop(rows);
        let accounts = ::replay_frame::cpu::scope("politics_ledger.emit.accounts", None);
        for tag in std::mem::take(&mut joined.dirty) {
            self.account(writer, tag, at, previous, &joined)?;
        }
        drop(accounts);
        let _cpu = ::replay_frame::cpu::scope("politics_ledger.emit.settle", None);
        self.settle(writer, at)
    }

    /// `tag`'s item of `key`: its ordinal, and whether this payment is its first.
    fn item(&mut self, tag: u32, key: [u8; 32]) -> (u32, bool) {
        let country = &mut self.countries[tag as usize];
        if let Some(&ordinal) = country.item_index.get(&key) {
            return (ordinal, false);
        }
        let ordinal = country.items.len() as u32;
        country.items.push(key);
        country.item_index.insert(key, ordinal);
        (ordinal, true)
    }

    /// Writes each country's periods the settled frontier closed since it last moved.
    fn settle(&mut self, writer: &mut Writer<'_>, at: i32) -> io::Result<()> {
        let Some(settled) = self.settled.filter(|settled| self.rolled != Some(*settled)) else {
            return Ok(());
        };
        self.rolled = Some(settled);
        for tag in 0..self.countries.len() {
            let rollups = &mut self.countries[tag].rollups;
            if rollups.days.is_empty()
                && rollups.daily.is_empty()
                && rollups.open.iter().all(|open| open.is_empty())
            {
                continue;
            }
            for (level, index, totals) in rollups.settle(settled)? {
                let key = self.tag_key(writer, tag as u32, ROLLUP, None)?;
                let size = tally::PERIOD_DAYS[level];
                let last = (index * size + size) * 24 - 1;
                let kind = ROLLUP_KIND + level as u64;
                Self::event(writer, key, (last, Some(at)), kind, &tally::cells(&totals))?;
            }
        }
        Ok(())
    }

    /// The census of the hour, every row by id (none while it is not complete): each slot's
    /// last row, its tag valid or refused.
    fn apply_census(
        &mut self,
        writer: &mut Writer<'_>,
        census: &[CensusRow],
        hour: i32,
        joined: &mut Joined,
    ) -> io::Result<()> {
        let before: BTreeSet<u32> = self.slot_tags.values().copied().collect();
        self.census.clear();
        self.slot_tags.clear();
        self.refused.clear();
        for row in census {
            self.census.insert(row.slot, row.clone());
        }
        for country in &mut self.countries {
            country.slots.clear();
        }
        let rows: Vec<CensusRow> = self.census.values().cloned().collect();
        for row in rows {
            match rows::valid_tag(row.id, row.slot, &row.tag) {
                Ok(spelling) => {
                    let spelling = spelling.to_owned();
                    let tag = self.tag(writer, &spelling, hour)?;
                    self.slot_tags.insert(row.slot, tag);
                    self.known.insert(row.slot, tag);
                    self.countries[tag as usize].slots.push(row.slot);
                }
                Err(reason) => {
                    self.refused.insert(row.slot, reason);
                }
            }
        }
        // A refused country no valid tag named before counts once for the branch.
        let unknown: Vec<String> = self
            .refused
            .iter()
            .filter(|(slot, _)| !self.known.contains_key(slot))
            .map(|(_, reason)| reason.clone())
            .collect();
        for reason in unknown {
            self.warn(reason, 1);
        }
        let after: BTreeSet<u32> = self.slot_tags.values().copied().collect();
        for &tag in &after {
            joined.dirty.insert(tag);
            self.countries[tag as usize].active = true;
        }
        for &tag in before.difference(&after) {
            joined.dirty.insert(tag);
            self.countries[tag as usize].active = false;
        }
        for slot in self.refused.keys() {
            if let Some(&tag) = self.known.get(slot) {
                joined.dirty.insert(tag);
            }
        }
        Ok(())
    }

    fn apply_links(&mut self, links: &[(u32, u32)], joined: &mut Joined) {
        for slot in self.links.values() {
            if let Some(&tag) = self.slot_tags.get(slot) {
                joined.dirty.insert(tag);
            }
        }
        self.links = links.iter().copied().collect();
        self.raw_slots.clear();
        for &(raw, slot) in links {
            self.raw_slots.entry(slot).or_default().push(raw);
            if let Some(&tag) = self.slot_tags.get(&slot) {
                joined.dirty.insert(tag);
            }
        }
        for raws in self.raw_slots.values_mut() {
            raws.sort_unstable();
        }
    }

    /// The hour's captures: the raw countries whose capture changed, or moved on this hour and
    /// not the hour before, or the hour before and not now. A capture moves on when only its
    /// last hour reached does, or when its read follows the one held, of the same capture and
    /// streams, its window starting at the hour the held one reached: its tag's account holds
    /// unchanged for it.
    fn apply_captures(
        &mut self,
        captures: &facts::Captures,
        hour: i32,
        whole: bool,
        joined: &mut Joined,
    ) -> Vec<u32> {
        // The captures that moved on the hour before only moving on again touch nothing.
        if !whole
            && captures.changed.is_empty()
            && captures.deleted.is_empty()
            && *self.advanced == *captures.extended
        {
            self.extended = Arc::clone(&captures.extended);
            self.extended_through = hour;
            return Vec::new();
        }
        let mut touched = BTreeSet::new();
        if whole {
            touched.extend(self.witnesses.keys().copied());
            self.witnesses.clear();
        }
        // A capture leaving the extended list keeps the last hour it reached.
        if *self.extended != *captures.extended {
            for &raw in self.extended.iter() {
                if captures.extended.binary_search(&raw).is_err()
                    && let Some(row) = self.witnesses.get_mut(&raw)
                {
                    row.through_hour = self.extended_through;
                }
            }
        }
        let mut followed = Vec::new();
        for &raw in &captures.deleted {
            self.witnesses.remove(&raw);
            touched.insert(raw);
        }
        for row in &captures.changed {
            let follows = match self.witnesses.entry(row.country) {
                std::collections::hash_map::Entry::Occupied(mut held) => {
                    let follows = {
                        let held = held.get();
                        held.instance == row.instance
                            && held.epoch == row.epoch
                            && held.streams == row.streams
                            && held.through_hour == row.since_hour
                            && row.through_hour == hour
                    };
                    held.get_mut().clone_from(row);
                    follows
                }
                std::collections::hash_map::Entry::Vacant(held) => {
                    held.insert(row.clone());
                    false
                }
            };
            if follows {
                followed.push(row.country);
            } else {
                touched.insert(row.country);
            }
        }
        let same = !whole
            && self.advanced_from.as_ref().is_some_and(|(extended, held)| {
                **extended == *captures.extended && *held == followed
            });
        if !same {
            let mut advanced = Vec::with_capacity(captures.extended.len() + followed.len());
            advanced.extend_from_slice(&captures.extended);
            advanced.extend_from_slice(&followed);
            advanced.sort_unstable();
            let (mut a, mut b) = (0, 0);
            while a < self.advanced.len() || b < advanced.len() {
                match (self.advanced.get(a).copied(), advanced.get(b).copied()) {
                    (Some(x), Some(y)) if x == y => {
                        a += 1;
                        b += 1;
                    }
                    (Some(x), y) if y.is_none_or(|y| x < y) => {
                        touched.insert(x);
                        a += 1;
                    }
                    (_, Some(y)) => {
                        touched.insert(y);
                        b += 1;
                    }
                    _ => break,
                }
            }
            self.advanced = advanced;
            self.advanced_from = Some((Arc::clone(&captures.extended), followed));
        }
        self.extended = Arc::clone(&captures.extended);
        self.extended_through = hour;
        for &raw in &touched {
            if let Some(tag) = self.raw_tag(raw) {
                joined.dirty.insert(tag);
            }
        }
        touched.into_iter().collect()
    }

    /// Names a tag for the identities of `raws`' captures linked to a census country with a
    /// valid tag: the identities named their first tag now.
    fn tag_identities(&mut self, raws: Vec<u32>, hour: i32) -> Vec<(u32, u64)> {
        let mut tagged = Vec::new();
        for raw in raws {
            let (Some(witness), Some(tag)) = (self.witness(raw), self.raw_tag(raw)) else {
                continue;
            };
            let tags = self
                .identities
                .entry((witness.country, witness.epoch))
                .or_default();
            if tags.last().is_none_or(|&(_, old)| old != tag) {
                if tags.is_empty() {
                    tagged.push((witness.country, witness.epoch));
                }
                tags.push((hour, tag));
            }
        }
        tagged
    }

    /// The branch's present and refused tags, at each change: each tag's where it changed, every
    /// tag first absent from the first hour any is listed.
    fn presence(&mut self, writer: &mut Writer<'_>, hour: i32) -> io::Result<()> {
        if !self.census_complete {
            return Ok(());
        }
        let mut states = vec![(false, false); self.tags.len()];
        for &tag in self.slot_tags.values() {
            states[tag as usize].0 = true;
        }
        for &tag in self.refused.keys().filter_map(|slot| self.known.get(slot)) {
            states[tag as usize].1 = true;
        }
        let spellings = |listed: &dyn Fn(&(bool, bool)) -> bool| -> Vec<Box<str>> {
            let mut tags: Vec<Box<str>> = states
                .iter()
                .enumerate()
                .filter(|(_, state)| listed(state))
                .map(|(tag, _)| self.tags[tag].clone())
                .collect();
            tags.sort_unstable();
            tags
        };
        let value = (spellings(&|state| state.0), spellings(&|state| state.1));
        if self.presence.as_ref() == Some(&value) {
            return Ok(());
        }
        if self.presence.is_none() {
            writer.catalogue(PRESENCE_FIRST, Cell::Integer(i128::from(hour)))?;
        }
        self.presence = Some(value);
        for (tag, state) in states.into_iter().enumerate() {
            if self.countries[tag].presence == state {
                continue;
            }
            self.countries[tag].presence = state;
            let key = self.tag_key(writer, tag as u32, PRESENT, None)?;
            let cells = [records::cell(writer, &state)?];
            Self::event(writer, key, (hour, None), PRESENCE_KIND, &cells)?;
        }
        Ok(())
    }

    /// The hour's native rows: each table's held rows changed, and the rows recorded.
    fn apply_rows(
        &mut self,
        writer: &mut Writer<'_>,
        hour: Hour,
        at: i32,
        previous: Option<i32>,
        tagged: Vec<(u32, u64)>,
        joined: &mut Joined,
    ) -> io::Result<()> {
        let Hour {
            whole,
            power,
            governments,
            daily,
            costs,
            deleted,
            command,
            command_whole,
            ..
        } = hour;
        let [
            power_deleted,
            government_deleted,
            daily_deleted,
            cost_deleted,
        ] = deleted;
        let mut newest = None::<i32>;
        if whole {
            // A table handed whole replaces what was held: rows it no longer holds are gone.
            let current: FastSet<(u32, u64, u64)> = power
                .iter()
                .map(|power| (power.row.country, power.row.epoch, power.row.sequence))
                .chain(governments.iter().map(|government| {
                    (
                        government.row.country,
                        government.row.epoch,
                        government.row.sequence,
                    )
                }))
                .collect();
            self.recorded.retain(|key| current.contains(key));
            // What the replacement leaves out changes the accounts of the tags holding it.
            let raws: BTreeSet<u32> = self
                .daily
                .keys()
                .copied()
                .chain(self.power.values().map(|row| row.country))
                .chain(self.costs.values().map(|row| row.country))
                .collect();
            for raw in raws {
                self.dirty_days(raw, joined);
            }
            self.waiting.clear();
            self.power.clear();
            self.daily_gains.clear();
            self.unknown_adds.clear();
            self.governments.clear();
            self.daily.clear();
            self.costs.clear();
            self.sums.clear();
        }
        // A held power row an account reads changing re-accounts its raw country's current tag,
        // whether or not the row is recorded again.
        for key in power_deleted {
            if let Some(row) = self.power.remove(&key) {
                self.index_power(key, row, false);
                self.recorded
                    .remove(&(row.country, row.epoch, row.sequence));
                joined.dirty.extend(self.raw_tag(row.country));
            }
            self.waiting.remove(&(0, key));
        }
        for power in &power {
            let key = power.row.key();
            let held = HeldPower::of(&power.row);
            if let Some(old) = self.power.insert(key, held) {
                self.index_power(key, old, false);
            }
            self.index_power(key, held, true);
            joined.dirty.extend(self.raw_tag(power.row.country));
            newest = newest.max(power_day(&power.row));
        }
        for key in government_deleted {
            if let Some(identity) = self.governments.remove(&key) {
                self.recorded.remove(&identity);
            }
            self.waiting.remove(&(2, key));
        }
        for government in &governments {
            let row = &government.row;
            self.governments
                .insert(row.key(), (row.country, row.epoch, row.sequence));
            newest = newest.max(Some(government.row.hour.div_euclid(24) * 24));
        }
        // A raw country's daily row is its current tag's reconciliation, and decides whether the
        // days it holds under any tag count as that tag's income.
        for key in daily_deleted {
            self.daily.remove(&(key as u32));
            self.dirty_days(key as u32, joined);
        }
        for row in &daily {
            self.daily.insert(row.country, row.clone());
            newest = newest.max(Some(row.day_hour));
            self.dirty_days(row.country, joined);
        }
        for key in cost_deleted {
            if let Some(row) = self.costs.remove(&key) {
                self.unsum(row);
                joined.dirty.extend(self.raw_tag(row.country));
            }
            self.waiting.remove(&(1, key));
        }
        for cost in &costs {
            joined.dirty.extend(self.raw_tag(cost.row.country));
            let held = HeldCost::of(&cost.row);
            if let Some(old) = self.costs.insert(cost.row.key(), held) {
                self.unsum(old);
            }
            let sum = self.sums.entry(held.application()).or_default();
            sum.0 = sum.0.wrapping_add(cost.row.cost_raw);
            sum.1 += 1;
        }
        if whole || !costs.is_empty() {
            // A cost key a complete publication removed cannot return.
            let held = &self.costs;
            self.recorded_costs
                .retain(|&(country, epoch, day_hour, ordinal), _| {
                    held.get(&HeldCost::slot(country, ordinal))
                        .is_some_and(|row| row.epoch == epoch && row.day_hour == day_hour)
                });
        }
        // Totals the table does not hold whole this hour are unknown; a whole read replaces them.
        let command_known = self.complete & facts::bit(TableId::CountryCommandPower) != 0;
        if !command_known || command_whole {
            for slot in std::mem::take(&mut self.command).into_keys() {
                joined.dirty.extend(self.slot_tags.get(&slot).copied());
            }
        }
        for (slot, value) in command {
            let changed = match value {
                Some(value) => self.command.insert(slot, value) != Some(value),
                None => self.command.remove(&slot).is_some(),
            };
            if changed {
                joined.dirty.extend(self.slot_tags.get(&slot).copied());
            }
        }
        if let Some(newest) = newest
            && self.settled.is_none_or(|settled| newest > settled)
        {
            self.settled = Some(newest);
        }
        // Rows of identities tagged this hour that waited, then the hour's own rows.
        let _cpu = ::replay_frame::cpu::scope("politics_ledger.emit.rows.record", None);
        let mut queue: Vec<Waiting> = Vec::new();
        if !tagged.is_empty() {
            let tagged: FastSet<(u32, u64)> = tagged.into_iter().collect();
            let keys: Vec<(u8, u128)> = self
                .waiting
                .iter()
                .filter(|(_, row)| {
                    tagged.contains(&match row {
                        Waiting::Power(power) => (power.row.country, power.row.epoch),
                        Waiting::Cost(cost) => (cost.row.country, cost.row.epoch),
                        Waiting::Government(government) => {
                            (government.row.country, government.row.epoch)
                        }
                    })
                })
                .map(|(key, _)| *key)
                .collect();
            for key in keys {
                if let Some(mut row) = self.waiting.remove(&key) {
                    // Facts are the event hour's or the read before's; any other row's are not.
                    let frameless = |date: i32| date != at && Some(date) != previous;
                    match &mut row {
                        Waiting::Power(power) if frameless(power.row.hour) => power.undate(),
                        Waiting::Cost(cost) if frameless(cost.row.day_hour) => cost.undate(),
                        _ => {}
                    }
                    queue.push(row);
                }
            }
        }
        queue.extend(power.into_iter().map(Waiting::Power));
        queue.extend(governments.into_iter().map(Waiting::Government));
        for row in queue {
            match row {
                Waiting::Power(power) => {
                    self.record_power(writer, power, at, joined)?
                }
                Waiting::Government(government) => {
                    self.record_government(writer, government, at, joined)?
                }
                Waiting::Cost(cost) => self.record_cost(writer, cost, at, &mut None, joined)?,
            }
        }
        drop(_cpu);
        let _cpu = ::replay_frame::cpu::scope("politics_ledger.emit.rows.daily", None);
        for row in daily {
            self.record_daily(writer, row, at, joined)?;
        }
        drop(_cpu);
        // The hour's rows are all in place: a country's costs of one day read alike.
        let _cpu = ::replay_frame::cpu::scope("politics_ledger.emit.rows.costs", None);
        let mut run = None;
        for cost in costs {
            self.record_cost(writer, cost, at, &mut run, joined)?;
        }
        Ok(())
    }

    /// Dirties raw country `raw`'s current tag and every tag holding a day of it.
    fn dirty_days(&self, raw: u32, joined: &mut Joined) {
        joined.dirty.extend(self.raw_tag(raw));
        joined.dirty.extend(
            self.days
                .range((raw, 0, i32::MIN)..=(raw, u64::MAX, i32::MAX))
                .map(|(_, day)| day.tag),
        );
    }

    fn unsum(&mut self, row: HeldCost) {
        let key = row.application();
        if let Some(sum) = self.sums.get_mut(&key) {
            sum.0 = sum.0.wrapping_sub(row.cost_raw);
            sum.1 -= 1;
            if sum.1 == 0 {
                self.sums.remove(&key);
            }
        }
    }

    /// Whether a contribution dated `hour` reaches a native day already settled: a reply shows
    /// it on its own, beside its month.
    fn late(&self, hour: i32) -> bool {
        self.settled
            .is_some_and(|settled| hour.div_euclid(24) * 24 < settled)
    }

    fn record_power(
        &mut self,
        writer: &mut Writer<'_>,
        mut power: Power,
        at: i32,
        joined: &mut Joined,
    ) -> io::Result<()> {
        let row = &power.row;
        let identity = (row.country, row.epoch, row.sequence);
        if self.recorded.contains(&identity) {
            return Ok(());
        }
        if row.cause == PowerCause::DailyGain
            && row.currency == PowerCurrency::Political
            && !self.applied(row.country, row.epoch, row.hour, row.sequence)
        {
            joined.unmatched.insert(row.country);
        }
        let Some(tag) = self.tag_at(row.country, row.epoch, row.hour) else {
            self.waiting.insert((0, row.key()), Waiting::Power(power));
            return Ok(());
        };
        self.recorded.insert(identity);
        let bit = match row.currency {
            PowerCurrency::Political => PoliticsCaptureCoverageRow::PP,
            PowerCurrency::Command => PoliticsCaptureCoverageRow::CP,
        };
        let covered = self.covered(row.country, row.epoch, row.hour, bit, at);
        let measure = POWER[usize::from(row.currency == PowerCurrency::Command)];
        let key = self.tag_key(writer, tag, measure, None)?;
        let late = self.late(row.hour);
        let charge = rows::charge(row);
        let fixed = matches!(
            row.cause,
            PowerCause::DailyGain
                | PowerCause::CapCorrection
                | PowerCause::ResetSetup
                | PowerCause::Console
        );
        let class = match charge {
            Some(_) => PAYMENT,
            None if fixed => SKIPPED,
            None => BALANCE,
        };
        let category = power.dating.category.as_deref().unwrap_or("");
        let (recurring, upkeep) = (rows::is_recurring(category), category == "upkeep");
        let resolved = match power.resolved.take() {
            Some(resolved) => resolved,
            None => power.resolve(),
        };
        let (ordinal, new) = self.item(tag, resolved.item);
        if new {
            let first = rows::mutation_id(row.country, row.epoch, row.sequence);
            self.show(
                writer,
                tag,
                ordinal,
                (row.hour, at),
                first,
                &rows::paid(row),
                (&row.identity, row.cause, None),
                power.dating.dated.as_ref(),
            )?;
        }
        let record = PowerRecord {
            country: row.country,
            epoch: row.epoch,
            sequence: row.sequence,
            hour: row.hour,
            command: row.currency == PowerCurrency::Command,
            realized: row.realized_delta_raw(),
            covered,
            item: ordinal,
            class,
            late,
            recurring,
            upkeep,
            charge: charge.flatten(),
        };
        let cell = records::cell(writer, &record)?;
        let sequence = Self::event(writer, key, (row.hour, Some(at)), POWER_KIND, &[cell])?;
        if late && class != SKIPPED {
            let held = self.tag_key(writer, tag, LATE, None)?;
            Self::event(writer, held, (row.hour, Some(at)), POWER_KIND, &[cell])?;
        } else if class == BALANCE {
            self.countries[tag as usize].rollups.count(
                Some((ordinal, row.country, row.epoch)),
                row.hour,
                false,
            );
        } else if class == PAYMENT {
            {
                self.countries[tag as usize].rollups.add(&Contribution {
                    field: (ordinal, row.country, row.epoch),
                    recurring,
                    upkeep,
                    hour: row.hour,
                    read: at,
                    order: (at, sequence),
                    sequence: row.sequence,
                    ordinal: 0,
                    charge: charge.flatten(),
                    covered,
                    realized: Some(i128::from(row.realized_delta_raw())),
                })?;
            }
        }
        joined.dirty.insert(tag);
        if let (Some(charge), Some(category)) = (charge, power.dating.category)
            && category.as_ref() != "dailyDeficit"
        {
            joined.payments.entry(tag).or_default().push(Paid {
                currency: row.currency,
                category,
                hour: row.hour,
                charge,
            });
        }
        if power.row.currency == PowerCurrency::Command {
            joined
                .command
                .entry((power.row.country, power.row.epoch))
                .or_default()
                .push((power.row, covered, resolved.holder));
        }
        Ok(())
    }

    fn record_government(
        &mut self,
        writer: &mut Writer<'_>,
        government: Government,
        at: i32,
        joined: &mut Joined,
    ) -> io::Result<()> {
        let row = &government.row;
        let identity = (row.country, row.epoch, row.sequence);
        if self.recorded.contains(&identity) {
            return Ok(());
        }
        let Some(tag) = self.tag_at(row.country, row.epoch, row.hour) else {
            self.waiting
                .insert((2, row.key()), Waiting::Government(government));
            return Ok(());
        };
        self.recorded.insert(identity);
        let covered = self.covered(
            row.country,
            row.epoch,
            row.hour,
            PoliticsCaptureCoverageRow::GOVERNMENT,
            at,
        );
        let key = self.tag_key(writer, tag, GOVERNMENTS, None)?;
        let shown = rows::government(row, at, government.tokens.clone(), covered);
        let cell = records::cell(writer, &shown)?;
        Self::event(writer, key, (row.hour, Some(at)), GOVERNMENT_KIND, &[cell])?;
        joined.dirty.insert(tag);
        Ok(())
    }

    fn record_daily(
        &mut self,
        writer: &mut Writer<'_>,
        row: CountryPowerDailyRow,
        at: i32,
        joined: &mut Joined,
    ) -> io::Result<()> {
        let Some(tag) = self.tag_at(row.country, row.epoch, row.day_hour) else {
            return Ok(());
        };
        let key = (row.country, row.epoch, row.day_hour);
        let (pp, cp) = rows::daily_parts(&row);
        let mut reads = (pp.then_some(at), cp.then_some(at));
        if let Some(old) = self.days.get(&key) {
            if old.row == row {
                return Ok(());
            }
            // An application commits its matching political operands together,
            // independently of command power.
            if rows::daily_political(&row) == rows::daily_political(&old.row) {
                reads.0 = old.reads.0;
            }
            if rows::daily_command(&row) == rows::daily_command(&old.row) {
                reads.1 = old.reads.1;
            }
        }
        let daily_key = self.tag_key(writer, tag, DAILY, None)?;
        let record = DailyRecord {
            epoch: row.epoch,
            daily: rows::daily(&row, at, reads),
        };
        let cells = [records::cell(writer, &record)?];
        // A revision of a settled day is also kept apart: no rollup counts it.
        if self.late(row.day_hour) {
            let held = self.tag_key(writer, tag, LATE, None)?;
            Self::event(
                writer,
                held,
                (row.day_hour, Some(at)),
                DAILY_KIND,
                &cells,
            )?;
        } else {
            let gains = [
                rows::native_gain(&row, reads, false),
                rows::native_gain(&row, reads, true),
            ];
            self.countries[tag as usize]
                .rollups
                .add_daily((row.day_hour, row.country, row.epoch), gains);
        }
        Self::event(
            writer,
            daily_key,
            (row.day_hour, Some(at)),
            DAILY_KIND,
            &cells,
        )?;
        // The daily table holds one day per country: its earlier days are of no further hour.
        let earlier: Vec<(u32, u64, i32)> = self
            .days
            .range((row.country, 0, i32::MIN)..=(row.country, u64::MAX, i32::MAX))
            .map(|(&key, _)| key)
            .filter(|key| key.2 < row.day_hour)
            .collect();
        for key in earlier {
            if let Some(day) = self.days.remove(&key) {
                joined.dirty.insert(day.tag);
            }
        }
        if let Some(old) = self.days.insert(key, Day { row, reads, tag }) {
            joined.dirty.insert(old.tag);
        }
        joined.dirty.insert(tag);
        Ok(())
    }

    /// What every cost of `row`'s country, day and application reads alike at hour `at`.
    fn cost_day(&self, row: &CountryPowerDailyCostRow, at: i32) -> CostDay {
        let day = self.days.get(&(row.country, row.epoch, row.day_hour));
        CostDay {
            of: (row.country, row.epoch, row.day_hour, row.pp_sequence),
            tag: self.tag_at(row.country, row.epoch, row.day_hour),
            covered: self.covered(
                row.country,
                row.epoch,
                row.day_hour,
                PoliticsCaptureCoverageRow::PP | PoliticsCaptureCoverageRow::PP_DAILY_COSTS,
                at,
            ) && !self.unreconciled(row.country),
            late: self.late(row.day_hour),
            matched: self.applied(row.country, row.epoch, row.day_hour, row.pp_sequence)
                || day.is_some_and(|day| day.row.pp_sequence == Some(row.pp_sequence)),
            // The day's political application read by then dates the payment's read.
            read: day
                .and_then(|day| day.reads.0)
                .map_or(at, |read| read.max(at)),
        }
    }

    /// Records `cost`; `run` holds what the costs recorded before it this hour read alike, for
    /// a run of one country's day.
    fn record_cost(
        &mut self,
        writer: &mut Writer<'_>,
        cost: Cost,
        at: i32,
        run: &mut Option<CostDay>,
        joined: &mut Joined,
    ) -> io::Result<()> {
        let row = &cost.row;
        let key = (row.country, row.epoch, row.day_hour, row.ordinal);
        let of = (row.country, row.epoch, row.day_hour, row.pp_sequence);
        let day = match run.filter(|day| day.of == of) {
            Some(day) => day,
            None => *run.insert(self.cost_day(row, at)),
        };
        let Some(tag) = day.tag else {
            self.waiting.insert((1, row.key()), Waiting::Cost(cost));
            return Ok(());
        };
        let (covered, revision, held_item) = match self.recorded_costs.get(&key) {
            Some(old) if old.row == *row => return Ok(()),
            // A revision keeps what the first record read.
            Some(old) => (old.covered, true, Some(old.item)),
            None => (day.covered, false, None),
        };
        let CostDay {
            late,
            matched,
            read,
            ..
        } = day;
        let class = if row.cost_raw == 0 {
            SKIPPED
        } else if !matched {
            UNMATCHED
        } else if row.cost_raw < 0 {
            CREDIT
        } else {
            PAYMENT
        };
        let category = cost.dating.category.as_deref().unwrap_or("");
        let (recurring, upkeep) = (rows::is_recurring(category), category == "upkeep");
        let (ordinal, new) = match held_item {
            Some(ordinal) => (ordinal, false),
            None => self.item(tag, cost.item.unwrap_or_else(|| cost.resolve())),
        };
        if new {
            let first = rows::cost_id(row.country, row.epoch, row.day_hour, row.ordinal);
            self.show(
                writer,
                tag,
                ordinal,
                (row.day_hour, at),
                first,
                &rows::paid_cost(row),
                (&row.identity, row.cause, Some(row.component)),
                cost.dating.dated.as_ref(),
            )?;
        }
        // Replies read only a cost's first record, and only one that charges.
        if !revision && class != SKIPPED {
            let record = CostRecord {
                country: row.country,
                epoch: row.epoch,
                day_hour: row.day_hour,
                ordinal: row.ordinal,
                pp_sequence: row.pp_sequence,
                covered,
                cost_raw: row.cost_raw,
                item: ordinal,
                class,
                late,
                recurring,
                upkeep,
            };
            let cell = records::cell(writer, &record)?;
            let cost_key = self.tag_key(writer, tag, COSTS, None)?;
            let sequence =
                Self::event(writer, cost_key, (row.day_hour, Some(at)), COST_KIND, &[cell])?;
            let field = (ordinal, row.country, row.epoch);
            if late {
                let held = self.tag_key(writer, tag, LATE, None)?;
                Self::event(writer, held, (row.day_hour, Some(at)), COST_KIND, &[cell])?;
            } else if class == UNMATCHED || class == CREDIT {
                let field = (class == CREDIT).then_some(field);
                self.countries[tag as usize]
                    .rollups
                    .count(field, row.day_hour, true);
            } else {
                self.countries[tag as usize].rollups.add(&Contribution {
                    field,
                    recurring,
                    upkeep,
                    hour: row.day_hour,
                    read,
                    order: (at, sequence),
                    sequence: row.pp_sequence,
                    ordinal: row.ordinal,
                    charge: Some(i128::from(row.cost_raw)),
                    covered,
                    realized: None,
                })?;
            }
        }
        if row.cost_raw != 0 {
            joined
                .costs
                .entry(tag)
                .or_default()
                .push((row.cost_raw, matched));
            if matched
                && row.cost_raw > 0
                && let Some(category) = cost.dating.category
            {
                joined.payments.entry(tag).or_default().push(Paid {
                    currency: PowerCurrency::Political,
                    category,
                    hour: row.day_hour,
                    charge: Some(i128::from(row.cost_raw)),
                });
            }
        }
        self.recorded_costs.insert(
            key,
            Recorded {
                row: cost.row,
                covered,
                item: ordinal,
            },
        );
        joined.dirty.insert(tag);
        Ok(())
    }

    /// Whether the daily table holds the political application (`country`, `epoch`, `day_hour`,
    /// `sequence`) now: the gains and costs it applies match it. It holds one day a country.
    fn applied(&self, country: u32, epoch: u64, day_hour: i32, sequence: u64) -> bool {
        self.daily.get(&country).is_some_and(|row| {
            row.epoch == epoch && row.day_hour == day_hour && row.pp_sequence == Some(sequence)
        })
    }

    /// Whether raw country `raw`'s daily political cost differs from its components' sum.
    fn unreconciled(&self, raw: u32) -> bool {
        self.daily.get(&raw).is_some_and(|row| {
            row.pp_sequence.is_some_and(|sequence| {
                let key = (row.country, row.epoch, row.day_hour, sequence);
                row.pp_cost_raw
                    .is_none_or(|cost| self.sums.get(&key).map_or(0, |sum| sum.0) != cost)
            })
        })
    }

    /// Adds held power row `row`, under `key`, to the indexes an account asks, or takes it out.
    fn index_power(&mut self, key: u128, row: HeldPower, held: bool) {
        match row.index {
            Indexed::DailyGain => {
                let entry = (row.country, row.hour, key);
                if held {
                    self.daily_gains.insert(entry);
                } else {
                    self.daily_gains.remove(&entry);
                }
            }
            Indexed::UnknownAdd(bit) => {
                let entry = (row.country, row.epoch, row.hour, bit, key);
                if held {
                    self.unknown_adds.insert(entry);
                } else {
                    self.unknown_adds.remove(&entry);
                }
            }
            Indexed::None => {}
        }
    }

    /// The currencies an addition without an evaluated operand leaves unknown, among the
    /// hour's power rows of `witness`'s capture.
    fn unknown_additions(&self, witness: &PoliticsCaptureCoverageRow) -> u8 {
        let (country, epoch) = (witness.country, witness.epoch);
        if witness.since_hour > witness.through_hour {
            return 0;
        }
        self.unknown_adds
            .range(
                (country, epoch, witness.since_hour, 0, 0)
                    ..=(country, epoch, witness.through_hour, u8::MAX, u128::MAX),
            )
            .fold(0, |bits, &(.., bit, _)| bits | bit)
    }

    /// Whether `raw`'s hour holds a political daily gain within `since..=hour`.
    fn has_daily(&self, raw: u32, since: i32, hour: i32) -> bool {
        since <= hour
            && self
                .daily_gains
                .range((raw, since, 0)..=(raw, hour, u128::MAX))
                .next()
                .is_some()
    }

    /// One tag's accounting at `hour`: its coverage, spending coverage, holds, income, parts
    /// and warning rates, written where they change.
    fn account(
        &mut self,
        writer: &mut Writer<'_>,
        tag: u32,
        hour: i32,
        previous: Option<i32>,
        joined: &Joined,
    ) -> io::Result<()> {
        let adjacent = previous.and_then(|last| last.checked_add(1)) == Some(hour);
        let slots = self.countries[tag as usize].slots.len();
        let active = self.countries[tag as usize].active && slots != 0;
        let changes_known = self.complete & facts::bit(TableId::CountryPowerChanges) != 0;
        let daily_known = self.complete & facts::bit(TableId::CountryPowerDaily) != 0
            && self.complete & facts::bit(TableId::CountryPowerDailyCosts) != 0;
        let mut rates = BTreeMap::<String, u64>::new();
        let mut coverage = self.countries[tag as usize].coverage.take();
        let mut spending = None;
        let mut current = false;
        for at in (0..slots).filter(|_| active) {
            let slot = self.countries[tag as usize].slots[at];
            let Some(country) = self.census.get(&slot).map(|row| row.id) else {
                continue;
            };
            let witness = self.slot_witness(slot);
            let mut value = witness.as_ref().map_or(
                Coverage {
                    country: Some(country),
                    instance: None,
                    since_hour: Some(hour),
                    through_hour: Some(hour),
                    epoch: None,
                    streams: 0,
                    reason: Some(Gap::Unavailable),
                },
                Coverage::of,
            );
            if value.through_hour != Some(hour) {
                value = Coverage {
                    streams: 0,
                    since_hour: Some(hour),
                    through_hour: Some(hour),
                    reason: Some(Gap::Stale),
                    ..value
                };
            }
            let raw = witness.as_ref().map_or(country, |row| row.country);
            let steady = self.unreconciled(raw);
            let unreconciled = steady || joined.unmatched.contains(&raw);
            let has_daily = self.has_daily(raw, value.since_hour.unwrap_or(hour), hour);
            let unknown = witness
                .as_ref()
                .map_or(0, |witness| self.unknown_additions(witness));
            if has_daily || unknown != 0 || joined.unmatched.contains(&raw) {
                self.volatile.insert(tag);
            }
            let political = value.streams & PoliticsCaptureCoverageRow::PP != 0
                && changes_known
                && daily_known
                && !unreconciled
                && (!has_daily || value.streams & PoliticsCaptureCoverageRow::PP_DAILY_COSTS != 0)
                && unknown & PoliticsCaptureCoverageRow::PP == 0;
            let command = value.streams & PoliticsCaptureCoverageRow::CP != 0
                && changes_known
                && unknown & PoliticsCaptureCoverageRow::CP == 0;
            let reason = if !changes_known {
                Some("Spending events were not recorded at this read".into())
            } else if !daily_known
                || unreconciled
                || has_daily && value.streams & PoliticsCaptureCoverageRow::PP_DAILY_COSTS == 0
            {
                Some("Daily political power costs were not recorded at this read".into())
            } else if unknown != 0 {
                Some(
                    match unknown {
                        PoliticsCaptureCoverageRow::PP => {
                            "A political power adjustment has no recorded evaluated operand"
                        }
                        PoliticsCaptureCoverageRow::CP => {
                            "A command power adjustment has no recorded evaluated operand"
                        }
                        _ => {
                            "Political and command power adjustments have no recorded evaluated operands"
                        }
                    }
                    .into(),
                )
            } else {
                value.reason.map(|gap| gap.text().to_owned())
            };
            spending = Some(LedgerSpendCoverage {
                read_hour: hour,
                since_hour: Some(hour),
                through_hour: Some(hour),
                political,
                command,
                reason,
            });
            // The open piece goes on when the hour continues it.
            coverage = Some(match coverage.take() {
                Some(mut old)
                    if adjacent
                        && Coverage {
                            through_hour: Some(hour - 1),
                            ..old
                        }
                        .continued_by(&value) =>
                {
                    old.through_hour = value.through_hour;
                    old
                }
                _ => value,
            });
            current = true;
            if steady {
                *rates
                    .entry(
                        "Daily PP rates/components have no matching native application and wrapping sum"
                            .into(),
                    )
                    .or_default() += 1;
            }
            for table in [
                TableId::CountryPowerChanges,
                TableId::GovernmentChanges,
                TableId::CountryPowerDaily,
                TableId::CountryPowerDailyCosts,
            ] {
                if self.complete & facts::bit(table) == 0 {
                    *rates
                        .entry(format!("{table:?} unavailable in part of this branch"))
                        .or_default() += 1;
                }
            }
            if joined.unmatched.contains(&raw) && !steady {
                // A gain read before its application warns once, not as a rate.
                let country = &mut self.countries[tag as usize];
                *country
                    .warnings
                    .entry(
                        "Daily PP rates/components have no matching native application and wrapping sum"
                            .into(),
                    )
                    .or_default() += 1;
                country.warnings_changed = true;
            }
        }
        for (slot, reason) in &self.refused {
            if self.known.get(slot) == Some(&tag) {
                *rates.entry(reason.clone()).or_default() += 1;
            }
        }
        self.countries[tag as usize].rate(hour, rates);
        if !current {
            coverage = None;
        }
        // A cost read at the hour without its matched application, or negative, leaves the
        // read's political spending unknown.
        if joined.costs.get(&tag).is_some_and(|costs| !costs.is_empty()) {
            self.volatile.insert(tag);
        }
        if let Some(spending) = spending.as_mut() {
            for &(amount, matched) in joined.costs.get(&tag).into_iter().flatten() {
                if !matched {
                    spending.political = false;
                    spending.reason = Some(
                        "Daily political power costs have no matched payment at this read".into(),
                    );
                } else if amount < 0 {
                    spending.political = false;
                    spending.reason = Some(
                        "Negative daily political power cost components are not included in spending"
                            .into(),
                    );
                }
            }
        }
        let identity = coverage
            .filter(|row| row.through_hour == Some(hour))
            .and_then(|row| row.identity());
        let starts = !adjacent || identity != self.countries[tag as usize].identity;
        self.countries[tag as usize].coverage = coverage;
        self.write_spending(writer, tag, hour, adjacent, starts, spending)?;
        self.write_holds(writer, tag, hour, adjacent, active, joined)?;
        self.write_parts(writer, tag, hour, previous, starts, identity, joined)?;
        self.countries[tag as usize].identity = identity;
        Ok(())
    }

    /// Whether `tag`'s spending is recorded at `hour` in each currency, and why not.
    fn write_spending(
        &mut self,
        writer: &mut Writer<'_>,
        tag: u32,
        hour: i32,
        adjacent: bool,
        starts: bool,
        value: Option<LedgerSpendCoverage>,
    ) -> io::Result<()> {
        let keys = [
            self.tag_key(writer, tag, COVERED[0], None)?,
            self.tag_key(writer, tag, COVERED[1], None)?,
        ];
        let Some(value) = value else {
            for key in keys {
                self.carry.value(writer, key, hour, None, false)?;
            }
            self.countries[tag as usize].spending = None;
            return Ok(());
        };
        let merged = match self.countries[tag as usize].spending.take() {
            Some(mut old)
                if adjacent
                    && old.political == value.political
                    && old.command == value.command
                    && old.reason == value.reason =>
            {
                old.through_hour = Some(hour);
                old
            }
            _ => value,
        };
        for (key, known) in keys.into_iter().zip([merged.political, merged.command]) {
            self.carry
                .value(writer, key, hour, known.then_some(1), starts)?;
        }
        let held = self.countries[tag as usize]
            .cover
            .as_ref()
            .is_some_and(|(political, command, reason)| {
                *political == merged.political
                    && *command == merged.command
                    && *reason == merged.reason
            });
        if !held {
            let cover = (merged.political, merged.command, merged.reason.clone());
            let key = self.tag_key(writer, tag, COVER_REASONS, None)?;
            let cells = [records::cell(writer, &merged)?];
            Self::event(writer, key, (hour, None), COVER_KIND, &cells)?;
            self.countries[tag as usize].cover = Some(cover);
        }
        self.countries[tag as usize].spending = Some(merged);
        Ok(())
    }

    /// `tag`'s command power holds at `hour`: the allocators carried on, the hour's command rows
    /// applied, and a snapshot of the hour's total, written when it differs.
    fn write_holds(
        &mut self,
        writer: &mut Writer<'_>,
        tag: u32,
        hour: i32,
        adjacent: bool,
        active: bool,
        joined: &Joined,
    ) -> io::Result<()> {
        let mutations_complete = self.complete & facts::bit(TableId::CountryPowerChanges) != 0;
        let slot = self.countries[tag as usize].slots.last().copied();
        let witness = slot.and_then(|slot| self.slot_witness(slot));
        let (total, refusal) = if active {
            let identity = witness.as_ref().map(|row| (row.country, row.epoch));
            let total = slot.and_then(|slot| self.command.get(&slot).copied());
            let holds = &mut self.countries[tag as usize].holds;
            let contiguous = adjacent && holds.last.is_some() && holds.identity == identity;
            if holds.identity != identity {
                holds.identity = identity;
                holds.sequence = None;
                holds.active.clear();
            }
            if !contiguous
                || !mutations_complete
                || !witness.as_ref().is_some_and(|row| {
                    row.streams & PoliticsCaptureCoverageRow::CP != 0
                        && row.since_hour <= hour
                        && row.through_hour >= hour
                })
            {
                holds.active.clear();
            }
            if let Some(rows) = identity.and_then(|identity| joined.command.get(&identity)) {
                let mut rows: Vec<_> = rows.iter().collect();
                rows.sort_by_key(|(row, ..)| row.sequence);
                for (row, covered, holder) in rows {
                    if !covered {
                        if rows::is_hold(row) {
                            holds.active.clear();
                        }
                        continue;
                    }
                    apply_hold(holds, row, holder.clone());
                }
            }
            (total, None)
        } else {
            let refusal = self
                .refused
                .iter()
                .find(|(slot, _)| self.known.get(*slot) == Some(&tag))
                .map(|(_, reason)| reason.clone());
            let total = (self.census_complete && refusal.is_none()).then_some(0);
            let holds = &mut self.countries[tag as usize].holds;
            holds.active.clear();
            holds.identity = None;
            holds.sequence = None;
            (total, refusal)
        };
        let missing = if active {
            "CountryCommandPower row unavailable"
        } else {
            refusal.as_deref().unwrap_or("Countries unavailable")
        };
        let holds = &mut self.countries[tag as usize].holds;
        settle_holds(holds, total);
        let continues = adjacent
            && holds.last.is_some()
            && holds
                .snapshot
                .as_ref()
                .is_some_and(|held| held.reads(total, &holds.active, missing));
        holds.last = Some(hour);
        if continues {
            return Ok(());
        }
        let snapshot = Snapshot::of(&holds.active, total, missing);
        holds.snapshot = Some(snapshot.clone());
        let key = self.tag_key(writer, tag, HOLDS, None)?;
        let shown = present::hold(
            hour,
            snapshot.total,
            &snapshot.holders,
            snapshot.missing.as_deref(),
            self.content.as_deref(),
        );
        let cells = [records::cell(writer, &shown)?];
        Self::event(writer, key, (hour, None), HOLD_KIND, &cells)?;
        Ok(())
    }

    /// `tag`'s current daily income by currency: its latest recorded day the daily table holds.
    /// The part dimension of `category` in `currency`, numbered and named when new.
    fn category(
        &mut self,
        writer: &mut Writer<'_>,
        currency: PowerCurrency,
        category: &str,
    ) -> io::Result<u64> {
        let index = usize::from(currency == PowerCurrency::Command);
        if let Some(&dimension) = self.categories[index].get(category) {
            return Ok(dimension);
        }
        let held = self.categories.iter().map(FastMap::len).sum::<usize>();
        let dimension = CATEGORY_PARTS + held as u64;
        let spelling = format!("{}:{category}", CURRENCIES[index]);
        let cell = text(writer, spelling.as_bytes())?;
        writer.catalogue(CATEGORY_NAMES + dimension, cell)?;
        self.categories[index].insert(category.into(), dimension);
        self.category_currency.insert(dimension, index);
        Ok(dimension)
    }

    /// `tag`'s spending parts at `hour`: its capture identity's income of the day and its spending
    /// by category, each currency's while its spending was recorded since the day began.
    #[allow(clippy::too_many_arguments)]
    fn write_parts(
        &mut self,
        writer: &mut Writer<'_>,
        tag: u32,
        hour: i32,
        previous: Option<i32>,
        starts: bool,
        identity: Option<(u32, u64)>,
        joined: &Joined,
    ) -> io::Result<()> {
        let day = hour.div_euclid(24) * 24;
        let known = self.countries[tag as usize]
            .cover
            .as_ref()
            .map_or([false; 2], |cover| [cover.0, cover.1]);
        let mut paid = Vec::new();
        for payment in joined.payments.get(&tag).into_iter().flatten() {
            if payment.hour < day || payment.hour > day.saturating_add(23) {
                continue;
            }
            let dimension = self.category(writer, payment.currency, &payment.category)?;
            paid.push((dimension, payment.charge));
        }
        let mut income: [Option<i128>; 6] = [None; 6];
        // The day's values by part dimension, ascending.
        let mut values = std::mem::take(&mut self.scratch.values);
        values.clear();
        let mut since = [None; 2];
        if let Some((country, epoch)) = identity {
            let parts = self.countries[tag as usize]
                .parts
                .entry(country)
                .or_default();
            if parts.epoch != Some(epoch) {
                *parts = DayParts {
                    epoch: Some(epoch),
                    ..DayParts::default()
                };
            }
            if !starts
                && previous.and_then(|last| last.checked_add(1)) == Some(hour)
                && parts.last.is_some()
            {
                parts.last = previous;
            }
            if parts.last.and_then(|last| last.checked_add(1)) != Some(hour) {
                parts.first = Some(hour);
            }
            let first = *parts.first.get_or_insert(hour);
            if parts.day != day {
                parts.day = day;
                parts.values.clear();
            }
            for (dimension, charge) in paid {
                let value = parts.values.entry(dimension).or_insert(Some(0));
                *value = rows::sum_raw([*value, charge].into_iter())?;
            }
            for (index, known) in known.into_iter().enumerate() {
                parts.since[index] = known.then(|| {
                    if parts.last.and_then(|last| last.checked_add(1)) == Some(hour) {
                        parts.since[index].unwrap_or(hour)
                    } else {
                        hour
                    }
                });
            }
            parts.last = Some(hour);
            values.extend(parts.values.iter().map(|(&dimension, &value)| (dimension, value)));
            since = parts.since;
            if first <= day {
                let held = || {
                    self.days
                        .range((country, 0, day)..=(country, u64::MAX, day))
                        .map(|(_, held)| held)
                        .filter(move |held| held.tag == tag && held.row.day_hour == day)
                };
                // One capture's later read revises its day; only separate captures are ambiguous.
                let supplied = |command: bool| {
                    let mut found = held().filter(|held| {
                        if command { held.reads.1 } else { held.reads.0 }
                            .is_some_and(|read| read <= hour)
                    });
                    let first = found.next();
                    if found.next().is_some() { None } else { first }
                };
                if let Some(row) = supplied(false).map(|held| &held.row)
                    && let (Some(gross), Some(cost), Some(net)) =
                        (row.pp_gross_gain_raw, row.pp_cost_raw, row.pp_net_gain_raw)
                {
                    let (gross, cost, net) = (i128::from(gross), i128::from(cost), i128::from(net));
                    income[0] = Some(gross);
                    income[1] = Some(-cost);
                    income[2] = Some(net - (gross - cost));
                    income[3] = Some(net);
                }
                if let Some(cp) = supplied(true).and_then(|held| held.row.cp_gain_raw) {
                    income[4] = Some(i128::from(cp));
                    income[5] = Some(i128::from(cp));
                }
            }
            // A day's income and spending alike end at its midnight, where a currency covered
            // since within the day is covered whole.
            if (!values.is_empty()
                || income.iter().any(Option::is_some)
                || since.iter().flatten().any(|&first| first > day))
                && let Some(next) = day.checked_add(24)
            {
                self.tomorrow.entry(next).or_default().insert(tag);
            }
        }
        for (dimension, value) in income.into_iter().enumerate() {
            let key = self.tag_key(writer, tag, PARTS, Some(dimension as u64))?;
            self.carry.value(writer, key, hour, value, starts)?;
        }
        // The tag's categories, ascending, with their currency.
        let mut categories = std::mem::take(&mut self.scratch.categories);
        categories.clear();
        {
            let country = &mut self.countries[tag as usize];
            for &(dimension, _) in &values {
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    country.categories.entry(dimension)
                {
                    entry.insert(hour);
                    country.categories_changed = true;
                }
            }
            categories.extend(country.categories.keys().map(|&dimension| {
                (dimension, self.category_currency.get(&dimension).copied())
            }));
        }
        let value = |dimension: u64| {
            values
                .binary_search_by_key(&dimension, |&(held, _)| held)
                .map_or(Some(0), |at| values[at].1)
        };
        for index in 0..2 {
            let covered = since[index].is_some_and(|first| first <= day);
            let mine = || {
                categories
                    .iter()
                    .filter(move |&&(_, currency)| currency == Some(index))
                    .map(|&(dimension, _)| dimension)
            };
            let total = if covered {
                rows::sum_raw(mine().map(value))?
            } else {
                None
            };
            let shown = covered && total.is_some();
            let key = self.tag_key(writer, tag, PARTS, Some(ZERO[index]))?;
            self.carry
                .value(writer, key, hour, shown.then_some(0), starts)?;
            for dimension in mine() {
                let key = self.tag_key(writer, tag, PARTS, Some(dimension))?;
                let value = shown.then(|| value(dimension)).flatten();
                self.carry.value(writer, key, hour, value, starts)?;
            }
            let key = self.tag_key(writer, tag, PARTS, Some(TOTAL[index]))?;
            self.carry.value(writer, key, hour, total, starts)?;
        }
        self.scratch = Scratch { values, categories };
        Ok(())
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts next,
    /// with its evidence and every scalar that changed since the last cut.
    pub fn cut(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        self.carry.cut(writer, last)?;
        self.evidence(writer, last)?;
        if self.last == Some(last) {
            self.known_from = None;
        }
        if self.settled != self.settled_written
            && let Some(settled) = self.settled
        {
            writer.catalogue(SETTLED, Cell::Integer(i128::from(settled)))?;
            self.settled_written = self.settled;
        }
        for country in &mut self.countries {
            country.materialize(last);
        }
        if self.warnings_changed {
            let cell = reasons(writer, &self.warnings)?;
            writer.catalogue(WARNINGS, cell)?;
            self.warnings_changed = false;
        }
        // The content's power diagnostics are reachable once an item demanded its power content.
        if !self.power_warned
            && let Some(content) = self.content.as_deref()
            && content.power_requested()
        {
            let mut cells = Vec::new();
            for warning in content.power_warnings() {
                cells.push(text(writer, warning.as_bytes())?);
            }
            let cell = Cell::List(writer.cells(cells)?);
            writer.catalogue(POWER_WARNINGS, cell)?;
            self.power_warned = true;
        }
        for at in 0..self.countries.len() {
            if self.countries[at].warnings_changed {
                let warnings = self.countries[at].warnings.clone();
                let cell = reasons(writer, &warnings)?;
                writer.catalogue(TAG_WARNINGS + self.countries[at].actor, cell)?;
                self.countries[at].warnings_changed = false;
            }
            if self.countries[at].categories_changed {
                let mut cells = Vec::new();
                for (&dimension, &first) in &self.countries[at].categories {
                    let pair = writer
                        .cells([Cell::Unsigned(dimension), Cell::Integer(i128::from(first))])?;
                    cells.push(Cell::List(pair));
                }
                let cell = Cell::List(writer.cells(cells)?);
                writer.catalogue(TAG_CATEGORIES + self.countries[at].actor, cell)?;
                self.countries[at].categories_changed = false;
            }
        }
        Ok(())
    }

    /// What the emit goes on with after a cut. A restart goes on at a segment's first hour, whose
    /// projection hands every table whole.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let state = Saved {
            last: self.last,
            known_from: self.known_from,
            begun: self.begun,
            census_complete: self.census_complete,
            tags: self.tags.iter().map(|tag| tag.to_string()).collect(),
            countries: &self.countries,
            refused: self.refused.clone(),
            volatile: self.volatile.iter().collect(),
            complete: self.complete,
            command: self
                .command
                .iter()
                .map(|(&slot, &value)| (slot, value))
                .collect(),
            presence: self.presence.as_ref().map(|(present, refused)| {
                (
                    present.iter().map(|tag| tag.to_string()).collect(),
                    refused.iter().map(|tag| tag.to_string()).collect(),
                )
            }),
            settled: self.settled,
            settled_written: self.settled_written,
            warnings: self.warnings.clone(),
            categories: self
                .categories
                .iter()
                .zip(CURRENCIES)
                .flat_map(|(categories, currency)| {
                    categories.iter().map(move |(category, &dimension)| {
                        (format!("{currency}:{category}"), dimension)
                    })
                })
                .collect(),
            tomorrow: self.tomorrow.clone(),
        };
        let json = serde_json::to_vec(&state)?;
        // The census, links, captures and held rows with their records, as varints.
        let mut rows = Vec::new();
        let mut put = tally::Out(&mut rows);
        put.len(self.census.len());
        for row in self.census.values() {
            put.var(u64::from(row.id));
            put.var(u64::from(row.slot));
            put_bytes(&mut put, &row.tag);
        }
        put_pairs(&mut put, &self.known);
        put_pairs(&mut put, &self.links);
        put_words(&mut put, &self.extended);
        put.int(self.extended_through);
        put_words(&mut put, &self.advanced);
        put.len(self.identities.len());
        for (&(country, epoch), tags) in &self.identities {
            put.var(u64::from(country));
            put.var(epoch);
            put.len(tags.len());
            for &(hour, tag) in tags {
                put.int(hour);
                put.var(u64::from(tag));
            }
        }
        put.len(self.recorded.len());
        for &(country, epoch, sequence) in &self.recorded {
            put.var(u64::from(country));
            put.var(epoch);
            put.var(sequence);
        }
        put.len(self.witnesses.len());
        for row in self.witnesses.values() {
            put_capture(&mut put, row);
        }
        put.len(self.daily.len());
        for row in self.daily.values() {
            put_daily(&mut put, row);
        }
        put.len(self.days.len());
        for day in self.days.values() {
            put_daily(&mut put, &day.row);
            put_hour(&mut put, day.reads.0);
            put_hour(&mut put, day.reads.1);
            put.var(u64::from(day.tag));
        }
        put.len(self.costs.len());
        for cost in self.costs.values() {
            put.var(u64::from(cost.country));
            put.var(cost.epoch);
            put.int(cost.day_hour);
            put.var(cost.pp_sequence);
            put.var(u64::from(cost.ordinal));
            put.zig128(i128::from(cost.cost_raw));
        }
        put.len(self.recorded_costs.len());
        for cost in self.recorded_costs.values() {
            put_cost(&mut put, &cost.row);
            put.flag(cost.covered);
            put.var(u64::from(cost.item));
        }
        // Each country's rollups and items, the bulk of a long campaign's state, as varints.
        let mut binary = Vec::new();
        let mut put = tally::Out(&mut binary);
        for country in &self.countries {
            country.rollups.save(&mut put);
            put.len(country.items.len());
            for key in &country.items {
                put.0.extend_from_slice(key);
            }
        }
        for part in [&json, &rows, &binary] {
            out.extend_from_slice(&(part.len() as u64).to_le_bytes());
            out.extend_from_slice(part);
        }
        self.carry.save(out)
    }

    /// The emit [`Self::save`] saved, going on with the publication `directory` reads.
    pub fn load(
        content: &Result<Option<Arc<PoliticsContent>>, String>,
        directory: &Directory,
        bytes: &[u8],
    ) -> io::Result<Self> {
        let mut d = Decode::new(bytes);
        let mut parts = Vec::with_capacity(3);
        for _ in 0..3 {
            let length = usize::try_from(d.u64()?).map_err(|_| invalid("saved part length"))?;
            parts.push(d.bytes(length)?);
        }
        let at = d.position();
        let saved: Loaded = serde_json::from_slice(parts[0])?;
        let mut emit = Self::new(content);
        emit.actors = Actors::resume(SCHEMA, directory)?;
        emit.carry = Carry::load(&bytes[at..])?;
        emit.last = saved.last;
        emit.known_from = saved.known_from;
        emit.begun = saved.begun;
        emit.census_complete = saved.census_complete;
        emit.tags = saved.tags.into_iter().map(Into::into).collect();
        emit.numbers = emit
            .tags
            .iter()
            .enumerate()
            .map(|(at, tag)| (tag.clone(), at as u32))
            .collect();
        emit.countries = saved.countries;
        let mut binary = Decode::new(parts[2]);
        for country in &mut emit.countries {
            country.rollups = tally::Rollups::load(&mut binary)?;
            let count = tally::len(&mut binary)?;
            country
                .items
                .try_reserve(count)
                .map_err(|_| invalid("saved items exceed memory"))?;
            for _ in 0..count {
                country.items.push(binary.array::<32>()?);
            }
        }
        binary.end()?;
        for country in &mut emit.countries {
            country.item_index = country
                .items
                .iter()
                .enumerate()
                .map(|(ordinal, key)| (*key, ordinal as u32))
                .collect();
        }
        if emit.countries.len() != emit.tags.len() {
            return Err(invalid("saved countries and tags disagree"));
        }
        emit.refused = saved.refused;
        let mut rows = Decode::new(parts[1]);
        for _ in 0..tally::len(&mut rows)? {
            let id = tally::word(&mut rows)?;
            let slot = tally::word(&mut rows)?;
            let tag = Arc::from(read_bytes(&mut rows)?);
            emit.census.insert(slot, CensusRow { id, slot, tag });
        }
        emit.known = read_pairs(&mut rows)?;
        emit.links = read_pairs(&mut rows)?;
        emit.extended = Arc::from(read_words(&mut rows)?);
        emit.extended_through = tally::int(&mut rows)?;
        emit.advanced = read_words(&mut rows)?;
        for _ in 0..tally::len(&mut rows)? {
            let identity = (tally::word(&mut rows)?, rows.var()?);
            let mut tags = Vec::new();
            for _ in 0..tally::len(&mut rows)? {
                tags.push((tally::int(&mut rows)?, tally::word(&mut rows)?));
            }
            emit.identities.insert(identity, tags);
        }
        for _ in 0..tally::len(&mut rows)? {
            emit.recorded
                .insert((tally::word(&mut rows)?, rows.var()?, rows.var()?));
        }
        for _ in 0..tally::len(&mut rows)? {
            let row = read_capture(&mut rows)?;
            emit.witnesses.insert(row.country, row);
        }
        for _ in 0..tally::len(&mut rows)? {
            let row = read_daily(&mut rows)?;
            emit.daily.insert(row.country, row);
        }
        for _ in 0..tally::len(&mut rows)? {
            let row = read_daily(&mut rows)?;
            let reads = (read_hour(&mut rows)?, read_hour(&mut rows)?);
            let tag = tally::word(&mut rows)?;
            let key = (row.country, row.epoch, row.day_hour);
            emit.days.insert(key, Day { row, reads, tag });
        }
        for _ in 0..tally::len(&mut rows)? {
            let row = HeldCost {
                country: tally::word(&mut rows)?,
                epoch: rows.var()?,
                day_hour: tally::int(&mut rows)?,
                pp_sequence: rows.var()?,
                ordinal: tally::word(&mut rows)?,
                cost_raw: signed(&mut rows)?,
            };
            let sum = emit.sums.entry(row.application()).or_default();
            sum.0 = sum.0.wrapping_add(row.cost_raw);
            sum.1 += 1;
            emit.costs.insert(row.key(), row);
        }
        for _ in 0..tally::len(&mut rows)? {
            let row = read_cost(&mut rows)?;
            let covered = rows.flag()?;
            let item = tally::word(&mut rows)?;
            emit.recorded_costs.insert(
                (row.country, row.epoch, row.day_hour, row.ordinal),
                Recorded { row, covered, item },
            );
        }
        rows.end()?;
        for (&raw, &slot) in &emit.links {
            emit.raw_slots.entry(slot).or_default().push(raw);
        }
        for raws in emit.raw_slots.values_mut() {
            raws.sort_unstable();
        }
        let slots: Vec<(u32, CensusRow)> = emit
            .census
            .iter()
            .map(|(&slot, row)| (slot, row.clone()))
            .collect();
        for (slot, row) in slots {
            if let Ok(spelling) = rows::valid_tag(row.id, row.slot, &row.tag)
                && let Some(&tag) = emit.numbers.get(spelling)
            {
                emit.slot_tags.insert(slot, tag);
                emit.countries[tag as usize].slots.push(slot);
            }
        }
        emit.volatile = saved.volatile.into_iter().collect();
        emit.complete = saved.complete;
        emit.command = saved.command.into_iter().collect();
        emit.presence = saved.presence.map(|(present, refused)| {
            (
                present.into_iter().map(Into::into).collect(),
                refused.into_iter().map(Into::into).collect(),
            )
        });
        emit.settled = saved.settled;
        emit.settled_written = saved.settled_written;
        emit.warnings = saved.warnings;
        for (spelling, dimension) in saved.categories {
            let (index, category) = CURRENCIES
                .iter()
                .enumerate()
                .find_map(|(index, currency)| {
                    let category = spelling.strip_prefix(currency)?.strip_prefix(':')?;
                    Some((index, category))
                })
                .ok_or_else(|| invalid("saved category spelling"))?;
            emit.category_currency.insert(dimension, index);
            emit.categories[index].insert(category.into(), dimension);
        }
        emit.tomorrow = saved.tomorrow;
        Ok(emit)
    }
}

/// `reasons` as a catalogue list of (reason, count) pairs.
fn reasons(writer: &mut Writer<'_>, reasons: &BTreeMap<String, u64>) -> io::Result<Cell> {
    let mut cells = Vec::with_capacity(reasons.len());
    for (reason, &count) in reasons {
        let pair = [text(writer, reason.as_bytes())?, Cell::Unsigned(count)];
        cells.push(Cell::List(writer.cells(pair)?));
    }
    Ok(Cell::List(writer.cells(cells)?))
}

/// Applies a covered command power row to `holds`: allocations add their holder, releases and
/// refunds remove theirs, and anything a refund cannot name clears every holder.
fn apply_hold(holds: &mut Holds, row: &CountryPowerChangeRow, holder: Option<Holder>) {
    if holds.sequence.is_some_and(|old| row.sequence <= old) {
        return;
    }
    holds.sequence = Some(row.sequence);
    if !rows::is_hold(row) || row.operation == PowerOperation::AllocationClamp {
        return;
    }
    let amount = row.requested_delta_raw.map(i128::from);
    let key = rows::holder_key(&LedgerItem::from(&row.identity));
    if row.cause == PowerCause::AllocationRefund {
        // Anonymous releases cannot tell which named allocator left.
        let Some((key, amount)) = key.zip(amount).filter(|(_, amount)| *amount > 0) else {
            holds.active.clear();
            return;
        };
        match holds.active.get_mut(&key) {
            Some(held) if amount < held.amount => held.amount -= amount,
            Some(held) if amount == held.amount => {
                holds.active.remove(&key);
            }
            _ => holds.active.clear(),
        }
        return;
    }
    if row.site == PowerSite::ReleaseCommandPower {
        holds.active.clear();
        return;
    }
    if row.site != PowerSite::AllocateCommandPower {
        return;
    }
    // A charged allocation records the evaluated negative operand; uncharged ones have none, and
    // the hour's total keeps their unidentified remainder.
    let Some((key, amount)) = key.zip(amount).filter(|(_, amount)| *amount < 0) else {
        return;
    };
    let amount = -amount;
    match holds.active.get_mut(&key) {
        Some(held) => held.amount += amount,
        None => {
            if let Some(mut holder) = holder {
                holder.amount = amount;
                holds.active.insert(key, holder);
            }
        }
    }
}

/// Settles `holds` against the hour's total: an observed total never keeps stale identities
/// or invents a negative remainder.
fn settle_holds(holds: &mut Holds, total: Option<i64>) {
    let attributed: i128 = holds.active.values().map(|holder| holder.amount).sum();
    if total.is_none_or(|value| value == 0 || attributed > i128::from(value)) {
        holds.active.clear();
    }
}

impl Snapshot {
    /// The settled holders against `total`, `missing` naming why a total is unknown.
    fn of(active: &BTreeMap<String, Holder>, total: Option<i64>, missing: &str) -> Self {
        Self {
            total,
            holders: active
                .iter()
                .map(|(id, holder)| (id.clone(), holder.clone()))
                .collect(),
            missing: total.is_none().then(|| missing.to_owned()),
        }
    }

    /// Whether it is [`Self::of`] these.
    fn reads(&self, total: Option<i64>, active: &BTreeMap<String, Holder>, missing: &str) -> bool {
        self.total == total
            && self.missing.as_deref() == total.is_none().then_some(missing)
            && self.holders.len() == active.len()
            && self
                .holders
                .iter()
                .zip(active)
                .all(|((id, holder), (held, active))| id == held && holder == active)
    }
}

#[derive(Serialize)]
struct Saved<'a> {
    last: Option<i32>,
    known_from: Option<i32>,
    begun: bool,
    census_complete: bool,
    tags: Vec<String>,
    countries: &'a [Country],
    refused: BTreeMap<u32, String>,
    volatile: Vec<u32>,
    complete: u16,
    command: Vec<(u32, i64)>,
    presence: Option<(Vec<String>, Vec<String>)>,
    settled: Option<i32>,
    settled_written: Option<i32>,
    warnings: BTreeMap<String, u64>,
    categories: Vec<(String, u64)>,
    tomorrow: BTreeMap<i32, BTreeSet<u32>>,
}

#[derive(Deserialize)]
struct Loaded {
    last: Option<i32>,
    known_from: Option<i32>,
    begun: bool,
    census_complete: bool,
    tags: Vec<String>,
    countries: Vec<Country>,
    refused: BTreeMap<u32, String>,
    volatile: Vec<u32>,
    complete: u16,
    command: Vec<(u32, i64)>,
    presence: Option<(Vec<String>, Vec<String>)>,
    settled: Option<i32>,
    settled_written: Option<i32>,
    warnings: BTreeMap<String, u64>,
    categories: Vec<(String, u64)>,
    tomorrow: BTreeMap<i32, BTreeSet<u32>>,
}

/// The saved state's varints ([`tally::Out`]): a length before its bytes, a flag before a value
/// that may be missing, enums by their recorded wire values.
fn put_bytes(put: &mut tally::Out<'_>, bytes: &[u8]) {
    put.len(bytes.len());
    put.0.extend_from_slice(bytes);
}

fn put_some(put: &mut tally::Out<'_>, value: Option<u64>) {
    put.flag(value.is_some());
    if let Some(value) = value {
        put.var(value);
    }
}

fn put_signed(put: &mut tally::Out<'_>, value: Option<i64>) {
    put.flag(value.is_some());
    if let Some(value) = value {
        put.zig128(i128::from(value));
    }
}

fn put_hour(put: &mut tally::Out<'_>, value: Option<i32>) {
    put.flag(value.is_some());
    if let Some(value) = value {
        put.int(value);
    }
}

fn put_pairs(put: &mut tally::Out<'_>, pairs: &FastMap<u32, u32>) {
    put.len(pairs.len());
    for (&key, &value) in pairs {
        put.var(u64::from(key));
        put.var(u64::from(value));
    }
}

fn put_words(put: &mut tally::Out<'_>, words: &[u32]) {
    put.len(words.len());
    for &word in words {
        put.var(u64::from(word));
    }
}

fn put_capture(put: &mut tally::Out<'_>, row: &PoliticsCaptureCoverageRow) {
    put.var(u64::from(row.country));
    put.var(u64::from(row.instance));
    put.var(row.epoch);
    put.int(row.since_hour);
    put.int(row.through_hour);
    put.var(u64::from(row.streams));
}

fn put_daily(put: &mut tally::Out<'_>, row: &CountryPowerDailyRow) {
    put.var(u64::from(row.country));
    put.var(u64::from(row.instance));
    put.var(row.epoch);
    put.int(row.day_hour);
    put_signed(put, row.pp_gross_gain_raw);
    put_signed(put, row.pp_cost_raw);
    put_signed(put, row.pp_net_gain_raw);
    put_signed(put, row.pp_realized_delta_raw);
    put_some(put, row.pp_sequence);
    put_signed(put, row.cp_gain_raw);
    put_signed(put, row.cp_realized_delta_raw);
    put_some(put, row.cp_sequence);
}

fn put_cost(put: &mut tally::Out<'_>, row: &CountryPowerDailyCostRow) {
    put.var(u64::from(row.country));
    put.var(u64::from(row.instance));
    put.var(u64::from(row.ordinal));
    put.var(row.epoch);
    put.int(row.day_hour);
    put.var(row.pp_sequence);
    put.var(row.component as u64);
    put.zig128(i128::from(row.cost_raw));
    put.var(row.cause as u64);
    put.var(row.site as u64);
    let identity = &row.identity;
    put.var(identity.action as u64);
    put_some(put, identity.item_token.map(u64::from));
    put_bytes(put, &identity.item_key);
    put_bytes(put, &identity.slot_key);
    put_some(put, identity.character.map(u64::from));
    put.var(identity.object_kind as u64);
    put_some(put, identity.object_reference);
    put_some(put, identity.target_country.map(u64::from));
    put_some(put, identity.target_state.map(u64::from));
    put_some(put, identity.target_region.map(u64::from));
    put.var(identity.origin_kind as u64);
    put_some(put, identity.origin_token.map(u64::from));
    put_bytes(put, &identity.origin_key);
}

fn damaged() -> io::Error {
    invalid("saved state is damaged")
}

fn read_bytes(input: &mut Decode<'_>) -> io::Result<Vec<u8>> {
    let len = tally::len(input)?;
    Ok(input.bytes(len)?.to_vec())
}

fn read_some(input: &mut Decode<'_>) -> io::Result<Option<u64>> {
    Ok(if input.flag()? { Some(input.var()?) } else { None })
}

fn read_word(input: &mut Decode<'_>) -> io::Result<Option<u32>> {
    read_some(input)?
        .map(|value| u32::try_from(value).map_err(|_| damaged()))
        .transpose()
}

fn signed(input: &mut Decode<'_>) -> io::Result<i64> {
    i64::try_from(input.zig128()?).map_err(|_| damaged())
}

fn read_signed(input: &mut Decode<'_>) -> io::Result<Option<i64>> {
    Ok(if input.flag()? { Some(signed(input)?) } else { None })
}

fn read_hour(input: &mut Decode<'_>) -> io::Result<Option<i32>> {
    Ok(if input.flag()? {
        Some(tally::int(input)?)
    } else {
        None
    })
}

fn narrow<T: TryFrom<u64>>(input: &mut Decode<'_>) -> io::Result<T> {
    T::try_from(input.var()?).map_err(|_| damaged())
}

fn read_pairs(input: &mut Decode<'_>) -> io::Result<FastMap<u32, u32>> {
    let mut pairs = FastMap::default();
    for _ in 0..tally::len(input)? {
        pairs.insert(tally::word(input)?, tally::word(input)?);
    }
    Ok(pairs)
}

fn read_words(input: &mut Decode<'_>) -> io::Result<Vec<u32>> {
    let mut words = Vec::new();
    for _ in 0..tally::len(input)? {
        words.push(tally::word(input)?);
    }
    Ok(words)
}

fn read_capture(input: &mut Decode<'_>) -> io::Result<PoliticsCaptureCoverageRow> {
    Ok(PoliticsCaptureCoverageRow {
        country: tally::word(input)?,
        instance: tally::word(input)?,
        epoch: input.var()?,
        since_hour: tally::int(input)?,
        through_hour: tally::int(input)?,
        streams: narrow(input)?,
    })
}

fn read_daily(input: &mut Decode<'_>) -> io::Result<CountryPowerDailyRow> {
    Ok(CountryPowerDailyRow {
        country: tally::word(input)?,
        instance: tally::word(input)?,
        epoch: input.var()?,
        day_hour: tally::int(input)?,
        pp_gross_gain_raw: read_signed(input)?,
        pp_cost_raw: read_signed(input)?,
        pp_net_gain_raw: read_signed(input)?,
        pp_realized_delta_raw: read_signed(input)?,
        pp_sequence: read_some(input)?,
        cp_gain_raw: read_signed(input)?,
        cp_realized_delta_raw: read_signed(input)?,
        cp_sequence: read_some(input)?,
    })
}

fn read_cost(input: &mut Decode<'_>) -> io::Result<CountryPowerDailyCostRow> {
    Ok(CountryPowerDailyCostRow {
        country: tally::word(input)?,
        instance: tally::word(input)?,
        ordinal: tally::word(input)?,
        epoch: input.var()?,
        day_hour: tally::int(input)?,
        pp_sequence: input.var()?,
        component: PowerDailyCostKind::try_from(narrow::<u8>(input)?)?,
        cost_raw: signed(input)?,
        cause: PowerCause::try_from(narrow::<u16>(input)?)?,
        site: PowerSite::try_from(narrow::<u16>(input)?)?,
        identity: PowerIdentity {
            action: PowerAction::try_from(narrow::<u8>(input)?)?,
            item_token: read_word(input)?,
            item_key: read_bytes(input)?,
            slot_key: read_bytes(input)?,
            character: read_word(input)?,
            object_kind: PowerObjectKind::try_from(narrow::<u8>(input)?)?,
            object_reference: read_some(input)?,
            target_country: read_word(input)?,
            target_state: read_word(input)?,
            target_region: read_word(input)?,
            origin_kind: PowerOriginKind::try_from(narrow::<u8>(input)?)?,
            origin_token: read_word(input)?,
            origin_key: read_bytes(input)?,
        },
    })
}
