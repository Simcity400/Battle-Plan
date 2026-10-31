//! The ledger's emit: the accounting of each projected hour ([`facts::Hour`]), written as the
//! family's series publication. A country's native rows are recorded once, as the read that first
//! named their identity's tag saw them, as typed events; its capture and spending coverage,
//! holds, income and spending parts as runs and events that change only when they do; the census
//! and its presence as the branch's. An hour that changes nothing of a country appends nothing.
//!
//! The emit holds what only earlier hours know: tags by capture identity, which rows were
//! recorded, the rows waiting for a tag, the native tables as the hour leaves them, each
//! country's open coverage, holds and day parts, and warning rates. A reply reads everything
//! else back from the publication ([`super::query`]).

use super::facts::{self, CensusRow, Cost, Government, Hour, Power};
use super::tally::{self, Contribution};
use super::*;
use dashboard_base::products::politics_content::PoliticsContent;
use dashboard_series::{
    Cell, Event, Evidence, Knownness, RunMode, Span, Writer,
    carry::Carry,
    codec::Decode,
    family::{Actors, Directory, Place},
};
use std::collections::{HashMap, HashSet};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 8;

/// Each country's measures: its power rows by currency, at their read; its governments, daily
/// rows and costs, at their native hour; its capture coverage (the fact event's sequence) and
/// those facts; whether its spending in each currency is recorded, with the reasons; its hold
/// snapshots; its daily income by currency; its spending parts, by dimension; its interned item
/// facts.
pub const POWER: [u64; 2] = [0, 1];
pub const GOVERNMENTS: u64 = 2;
pub const DAILY: u64 = 3;
pub const COSTS: u64 = 4;
pub const CAPTURE: u64 = 5;
pub const CAPTURE_FACTS: u64 = 6;
pub const COVERED: [u64; 2] = [7, 8];
pub const COVER_REASONS: u64 = 9;
pub const HOLDS: u64 = 10;
pub const INCOME: [u64; 2] = [11, 12];
pub const PARTS: u64 = 13;
pub const DATED: u64 = 14;
/// Each country's settled totals by item and period ([`super::tally`]), at the period's last
/// hour; its payments read after their day settled, at their read; each item's first payment.
pub const ROLLUP: u64 = 15;
pub const LATE: u64 = 16;
pub const ITEMS: u64 = 17;
/// The branch's measures, under the actor with the empty name: whether the census is complete,
/// every prepared hour, and the present and refused tags at each change.
pub const CENSUS: u64 = 20;
pub const ACCEPTED: u64 = 21;
pub const PRESENCE: u64 = 22;

/// Event kinds.
pub const POWER_KIND: u64 = 1;
pub const GOVERNMENT_KIND: u64 = 2;
pub const DAILY_KIND: u64 = 3;
pub const COST_KIND: u64 = 4;
pub const CAPTURE_KIND: u64 = 5;
pub const COVER_KIND: u64 = 6;
pub const HOLD_KIND: u64 = 7;
pub const DATED_KIND: u64 = 8;
pub const PRESENCE_KIND: u64 = 9;
/// A rollup's kind is this plus its level.
pub const ROLLUP_KIND: u64 = 10;

/// What a power row or cost is to a reply's payments, as its last cells say.
pub const PAYMENT: u64 = 0;
pub const BALANCE: u64 = 1;
pub const SKIPPED: u64 = 2;
pub const UNMATCHED: u64 = 3;
pub const CREDIT: u64 = 4;
/// The cells every payment event ends with: its item's ordinal and first payment, its class,
/// and whether it recurs and counts its days one entry each.
pub const ITEM_CELLS: usize = 10;

/// Family scalars: the first prepared hour; the native day from which days are still open; the
/// branch's warnings; and by actor, each country's warnings and its categories' first hours.
pub const FIRST: u64 = 0;
pub const SETTLED: u64 = 1;
pub const WARNINGS: u64 = 2;
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

fn integer(value: Option<impl Into<i128>>) -> Cell {
    value.map_or(Cell::Null, |value| Cell::Integer(value.into()))
}

fn unsigned(value: Option<impl Into<u64>>) -> Cell {
    value.map_or(Cell::Null, |value| Cell::Unsigned(value.into()))
}

fn text(writer: &mut Writer<'_>, bytes: &[u8]) -> io::Result<Cell> {
    if bytes.is_empty() {
        return Ok(Cell::Text(dashboard_series::Blob::EMPTY));
    }
    Ok(Cell::Text(
        writer.blob(&mut &bytes[..], bytes.len() as u64)?,
    ))
}

fn optional_text(writer: &mut Writer<'_>, bytes: Option<&[u8]>) -> io::Result<Cell> {
    bytes.map_or(Ok(Cell::Null), |bytes| text(writer, bytes))
}

/// A recorded item identity as cells, in [`PowerIdentity`]'s field order.
fn identity_cells(writer: &mut Writer<'_>, identity: &PowerIdentity) -> io::Result<Cell> {
    let cells = [
        Cell::Unsigned(identity.action as u64),
        unsigned(identity.item_token),
        text(writer, &identity.item_key)?,
        text(writer, &identity.slot_key)?,
        unsigned(identity.character),
        Cell::Unsigned(identity.object_kind as u64),
        unsigned(identity.object_reference),
        unsigned(identity.target_country),
        unsigned(identity.target_state),
        unsigned(identity.target_region),
        Cell::Unsigned(identity.origin_kind as u64),
        unsigned(identity.origin_token),
        text(writer, &identity.origin_key)?,
    ];
    Ok(Cell::List(writer.cells(cells)?))
}

/// The native day a power row closes: daily upkeep and gains only.
fn power_day(row: &CountryPowerChangeRow) -> Option<i32> {
    matches!(row.cause, PowerCause::AbilityUpkeep | PowerCause::DailyGain)
        .then(|| row.hour.div_euclid(24) * 24)
}

fn identity_of(row: &LedgerCoverage) -> Option<(u32, u64)> {
    row.country
        .zip(row.epoch.as_deref().and_then(|epoch| epoch.parse().ok()))
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

/// A recorded cost: what its revisions keep.
struct Recorded {
    row: CountryPowerDailyCostRow,
    covered: bool,
    dated: Option<(i32, u64)>,
    item: u32,
}

/// An item: its key, and its first payment (a cost, the country, epoch, sequence, day and
/// ordinal).
#[derive(Clone, Copy, Serialize, Deserialize)]
struct ItemRef {
    key: [u8; 32],
    first: (bool, u32, u64, u64, i32, u32),
}

/// What makes two payments one item: the facts their hour recorded and what they record.
fn item_key(dated: Option<&[u8]>, what: &str) -> [u8; 32] {
    let mut hash = blake3::Hasher::new();
    match dated {
        Some(bytes) => {
            hash.update(&[1]);
            hash.update(blake3::hash(bytes).as_bytes());
        }
        None => {
            hash.update(&[0]);
        }
    }
    hash.update(what.as_bytes());
    *hash.finalize().as_bytes()
}

/// The cells [`ITEM_CELLS`] counts.
fn item_cells(
    ordinal: u32,
    item: &ItemRef,
    class: u64,
    recurring: bool,
    upkeep: bool,
) -> [Cell; 10] {
    let (cost, country, epoch, sequence, day, ordinal_of) = item.first;
    [
        Cell::Unsigned(u64::from(ordinal)),
        Cell::Boolean(cost),
        Cell::Unsigned(u64::from(country)),
        Cell::Unsigned(epoch),
        Cell::Unsigned(sequence),
        Cell::Integer(i128::from(day)),
        Cell::Unsigned(u64::from(ordinal_of)),
        Cell::Unsigned(class),
        Cell::Boolean(recurring),
        Cell::Boolean(upkeep),
    ]
}

/// A command power holder, by the operands its allocation recorded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Holder {
    kind: String,
    amount: i128,
    ability: bool,
    token: Option<String>,
    character: Option<(Option<String>, String)>,
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
    coverage: Option<LedgerCoverage>,
    /// The fact its capture run holds, and the coverage that fact was written for.
    capture: Option<(u64, LedgerCoverage)>,
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
    #[serde(skip)]
    dated: HashMap<[u8; 32], (i32, u64)>,
    /// Its items in the order first paid.
    items: Vec<ItemRef>,
    #[serde(skip)]
    item_index: HashMap<[u8; 32], u32>,
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

/// What the hour joins, as the accounting reads it.
#[derive(Default)]
struct Joined {
    /// Tags whose inputs the hour changed.
    dirty: BTreeSet<u32>,
    /// Command power rows recorded this hour by identity: covered, and their holder's operands.
    command: BTreeMap<(u32, u64), Vec<(CountryPowerChangeRow, bool, Option<Holder>)>>,
    /// Costs recorded this hour by tag: their amount and whether their application matched.
    costs: BTreeMap<u32, Vec<(i64, bool)>>,
    payments: BTreeMap<u32, Vec<Paid>>,
    /// Raw countries with a political daily gain read before its application.
    unmatched: BTreeSet<u32>,
}

/// The family's emit on one branch.
pub struct Emit {
    actors: Actors,
    carry: Carry,
    content_failure: Option<String>,
    last: Option<i32>,
    /// The first hour of the accepted stretch not yet given its evidence.
    known_from: Option<i32>,
    begun: bool,
    census: BTreeMap<u32, CensusRow>,
    census_complete: bool,
    known: HashMap<u32, u32>,
    tags: Vec<Box<str>>,
    numbers: HashMap<Box<str>, u32>,
    countries: Vec<Country>,
    slot_tags: HashMap<u32, u32>,
    refused: BTreeMap<u32, String>,
    links: HashMap<u32, u32>,
    raw_slots: HashMap<u32, Vec<u32>>,
    witnesses: HashMap<u32, PoliticsCaptureCoverageRow>,
    extended: Arc<[u32]>,
    extended_through: i32,
    /// The raw countries whose capture moved on at the last hour, ascending.
    advanced: Vec<u32>,
    /// Tags whose last account read inputs of its hour alone: accounted again at the next.
    volatile: BTreeSet<u32>,
    /// Each held day by tag, day hour, raw country and epoch.
    tag_days: BTreeSet<(u32, i32, u32, u64)>,
    identities: HashMap<(u32, u64), Vec<(i32, u32)>>,
    complete: u16,
    power: BTreeMap<u128, CountryPowerChangeRow>,
    /// Of `power`, the political daily gains by raw country and hour, and the additions without
    /// an evaluated operand by raw country, epoch, hour and currency bit: what an account asks
    /// of a country's rows, found without walking them.
    daily_gains: BTreeSet<(u32, i32, u128)>,
    unknown_adds: BTreeSet<(u32, u64, i32, u8, u128)>,
    governments: BTreeMap<u128, GovernmentChangeRow>,
    recorded: HashSet<(u32, u64, u64)>,
    waiting: BTreeMap<(u8, u128), Waiting>,
    daily: HashMap<u32, CountryPowerDailyRow>,
    days: BTreeMap<(u32, u64, i32), Day>,
    costs: BTreeMap<u128, CountryPowerDailyCostRow>,
    recorded_costs: BTreeMap<(u32, u64, i32, u32), Recorded>,
    sums: HashMap<(u32, u64, i32, u64), (i64, u32)>,
    command: HashMap<u32, i64>,
    presence: Option<(Vec<Box<str>>, Vec<Box<str>>)>,
    settled: Option<i32>,
    settled_written: Option<i32>,
    /// The frontier the rollups were last settled at.
    rolled: Option<i32>,
    warnings: BTreeMap<String, u64>,
    warnings_changed: bool,
    categories: HashMap<String, u64>,
    category_currency: HashMap<u64, usize>,
    tomorrow: BTreeMap<i32, BTreeSet<u32>>,
}

impl Emit {
    pub fn new(content: &Result<Option<Arc<PoliticsContent>>, String>) -> Self {
        Self {
            actors: Actors::new(SCHEMA),
            carry: Carry::default(),
            content_failure: content.as_ref().err().cloned(),
            last: None,
            known_from: None,
            begun: false,
            census: BTreeMap::new(),
            census_complete: false,
            known: HashMap::new(),
            tags: Vec::new(),
            numbers: HashMap::new(),
            countries: Vec::new(),
            slot_tags: HashMap::new(),
            refused: BTreeMap::new(),
            links: HashMap::new(),
            raw_slots: HashMap::new(),
            witnesses: HashMap::new(),
            extended: Arc::from(Vec::new()),
            extended_through: 0,
            advanced: Vec::new(),
            volatile: BTreeSet::new(),
            tag_days: BTreeSet::new(),
            identities: HashMap::new(),
            complete: 0,
            power: BTreeMap::new(),
            daily_gains: BTreeSet::new(),
            unknown_adds: BTreeSet::new(),
            governments: BTreeMap::new(),
            recorded: HashSet::new(),
            waiting: BTreeMap::new(),
            daily: HashMap::new(),
            days: BTreeMap::new(),
            costs: BTreeMap::new(),
            recorded_costs: BTreeMap::new(),
            sums: HashMap::new(),
            command: HashMap::new(),
            presence: None,
            settled: None,
            settled_written: None,
            rolled: None,
            warnings: BTreeMap::new(),
            warnings_changed: false,
            categories: HashMap::new(),
            category_currency: HashMap::new(),
            tomorrow: BTreeMap::new(),
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
        let decimals = if measure == PARTS || INCOME.contains(&measure) {
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
        let actor = self.actors.actor(writer, BRANCH, hour)?;
        self.key(writer, actor, measure, None)
    }

    fn event(
        writer: &mut Writer<'_>,
        key: u64,
        (native, read): (i32, Option<i32>),
        kind: u64,
        cells: Vec<Cell>,
    ) -> io::Result<u64> {
        let fields = writer.cells(cells)?;
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

    /// `bytes`, an item's facts, interned for `tag`: the event holding them.
    fn dated(
        &mut self,
        writer: &mut Writer<'_>,
        tag: u32,
        hour: i32,
        bytes: &[u8],
    ) -> io::Result<(i32, u64)> {
        let hash = *blake3::hash(bytes).as_bytes();
        if let Some(&held) = self.countries[tag as usize].dated.get(&hash) {
            return Ok(held);
        }
        let key = self.tag_key(writer, tag, DATED, None)?;
        let cell = text(writer, bytes)?;
        let sequence = Self::event(writer, key, (hour, None), DATED_KIND, vec![cell])?;
        self.countries[tag as usize]
            .dated
            .insert(hash, (hour, sequence));
        Ok((hour, sequence))
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
                country.capture = None;
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

    /// `tag`'s item of `key`, interned at its first payment `first`: its ordinal, the item, and
    /// whether this payment is its first.
    fn item(
        &mut self,
        tag: u32,
        key: [u8; 32],
        first: (bool, u32, u64, u64, i32, u32),
    ) -> (u32, ItemRef, bool) {
        let country = &mut self.countries[tag as usize];
        if let Some(&ordinal) = country.item_index.get(&key) {
            return (ordinal, country.items[ordinal as usize], false);
        }
        let ordinal = country.items.len() as u32;
        let item = ItemRef { key, first };
        country.items.push(item);
        country.item_index.insert(key, ordinal);
        (ordinal, item, true)
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
                Self::event(writer, key, (last, Some(at)), kind, tally::cells(&totals))?;
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
        let mut advanced = captures.extended.to_vec();
        for &raw in &captures.deleted {
            self.witnesses.remove(&raw);
            touched.insert(raw);
        }
        for row in &captures.changed {
            let follows = self.witnesses.get(&row.country).is_some_and(|held| {
                held.instance == row.instance
                    && held.epoch == row.epoch
                    && held.streams == row.streams
                    && held.through_hour == row.since_hour
                    && row.through_hour == hour
            });
            if follows {
                advanced.push(row.country);
            } else {
                touched.insert(row.country);
            }
            self.witnesses.insert(row.country, row.clone());
        }
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

    /// The branch's present and refused tags, at each change.
    fn presence(&mut self, writer: &mut Writer<'_>, hour: i32) -> io::Result<()> {
        if !self.census_complete {
            return Ok(());
        }
        let mut present: Vec<Box<str>> = self
            .slot_tags
            .values()
            .map(|&tag| self.tags[tag as usize].clone())
            .collect();
        present.sort_unstable();
        present.dedup();
        let mut refused: Vec<Box<str>> = self
            .refused
            .keys()
            .filter_map(|slot| self.known.get(slot))
            .map(|&tag| self.tags[tag as usize].clone())
            .collect();
        refused.sort_unstable();
        refused.dedup();
        let value = (present, refused);
        if self.presence.as_ref() == Some(&value) {
            return Ok(());
        }
        let key = self.branch_key(writer, PRESENCE, hour)?;
        let mut lists = Vec::with_capacity(2);
        for tags in [&value.0, &value.1] {
            let mut cells = Vec::with_capacity(tags.len());
            for tag in tags {
                cells.push(text(writer, tag.as_bytes())?);
            }
            lists.push(Cell::List(writer.cells(cells)?));
        }
        Self::event(writer, key, (hour, None), PRESENCE_KIND, lists)?;
        self.presence = Some(value);
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
            let current: HashSet<(u32, u64, u64)> = power
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
                self.index_power(key, &row, false);
                self.recorded
                    .remove(&(row.country, row.epoch, row.sequence));
                joined.dirty.extend(self.raw_tag(row.country));
            }
            self.waiting.remove(&(0, key));
        }
        for power in &power {
            let key = power.row.key();
            if let Some(old) = self.power.insert(key, power.row.clone()) {
                self.index_power(key, &old, false);
            }
            self.index_power(key, &power.row, true);
            joined.dirty.extend(self.raw_tag(power.row.country));
            newest = newest.max(power_day(&power.row));
        }
        for key in government_deleted {
            if let Some(row) = self.governments.remove(&key) {
                self.recorded
                    .remove(&(row.country, row.epoch, row.sequence));
            }
            self.waiting.remove(&(2, key));
        }
        for government in &governments {
            self.governments
                .insert(government.row.key(), government.row.clone());
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
                self.unsum(&row);
                joined.dirty.extend(self.raw_tag(row.country));
            }
            self.waiting.remove(&(1, key));
        }
        for cost in &costs {
            joined.dirty.extend(self.raw_tag(cost.row.country));
            if let Some(old) = self.costs.insert(cost.row.key(), cost.row.clone()) {
                self.unsum(&old);
            }
            let sum = self
                .sums
                .entry((
                    cost.row.country,
                    cost.row.epoch,
                    cost.row.day_hour,
                    cost.row.pp_sequence,
                ))
                .or_default();
            sum.0 = sum.0.wrapping_add(cost.row.cost_raw);
            sum.1 += 1;
        }
        if whole || !costs.is_empty() {
            // A cost key a complete publication removed cannot return.
            let current: HashSet<(u32, u64, i32, u32)> = self
                .costs
                .values()
                .map(|row| (row.country, row.epoch, row.day_hour, row.ordinal))
                .collect();
            self.recorded_costs.retain(|key, _| current.contains(key));
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
        let mut queue: Vec<Waiting> = Vec::new();
        if !tagged.is_empty() {
            let tagged: HashSet<(u32, u64)> = tagged.into_iter().collect();
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
                        Waiting::Power(power) if frameless(power.row.hour) => {
                            power.dating.dated = None;
                        }
                        Waiting::Cost(cost) if frameless(cost.row.day_hour) => {
                            cost.dating.dated = None;
                        }
                        _ => {}
                    }
                    queue.push(row);
                }
            }
        }
        queue.extend(power.into_iter().map(Waiting::Power));
        queue.extend(governments.into_iter().map(Waiting::Government));
        // The applications the daily table holds now, which political gains and costs match:
        // gathered only for an hour holding any.
        let applications: HashSet<(u32, u64, i32, u64)> = if queue.is_empty() && costs.is_empty() {
            HashSet::new()
        } else {
            self.daily
                .values()
                .filter_map(|row| {
                    row.pp_sequence
                        .map(|sequence| (row.country, row.epoch, row.day_hour, sequence))
                })
                .collect()
        };
        for row in queue {
            match row {
                Waiting::Power(power) => {
                    self.record_power(writer, power, at, &applications, joined)?
                }
                Waiting::Government(government) => {
                    self.record_government(writer, government, at, joined)?
                }
                Waiting::Cost(cost) => self.record_cost(writer, cost, at, &applications, joined)?,
            }
        }
        for row in daily {
            self.record_daily(writer, row, at, joined)?;
        }
        for cost in costs {
            self.record_cost(writer, cost, at, &applications, joined)?;
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

    fn unsum(&mut self, row: &CountryPowerDailyCostRow) {
        let key = (row.country, row.epoch, row.day_hour, row.pp_sequence);
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
        power: Power,
        at: i32,
        applications: &HashSet<(u32, u64, i32, u64)>,
        joined: &mut Joined,
    ) -> io::Result<()> {
        let row = &power.row;
        let identity = (row.country, row.epoch, row.sequence);
        if self.recorded.contains(&identity) {
            return Ok(());
        }
        if row.cause == PowerCause::DailyGain
            && row.currency == PowerCurrency::Political
            && !applications.contains(&(row.country, row.epoch, row.hour, row.sequence))
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
        let dated = match &power.dating.dated {
            Some(bytes) => Some(self.dated(writer, tag, at, bytes)?),
            None => None,
        };
        let measure = POWER[usize::from(row.currency == PowerCurrency::Command)];
        let key = self.tag_key(writer, tag, measure, None)?;
        let identity_cell = identity_cells(writer, &row.identity)?;
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
        // Every ability's upkeep is one line a day.
        let item =
            if row.currency == PowerCurrency::Command && row.cause == PowerCause::AbilityUpkeep {
                item_key(None, "ability upkeep")
            } else {
                item_key(
                    power.dating.dated.as_deref(),
                    &format!(
                        "power {:?} {:?} {:?} {}",
                        row.cause,
                        row.currency,
                        row.identity,
                        row.requested_delta_raw.is_some_and(|raw| raw < 0)
                    ),
                )
            };
        let (ordinal, item, new) = self.item(
            tag,
            item,
            (false, row.country, row.epoch, row.sequence, 0, 0),
        );
        let mut cells = vec![
            Cell::Unsigned(u64::from(row.country)),
            Cell::Unsigned(u64::from(row.instance)),
            Cell::Unsigned(row.epoch),
            Cell::Unsigned(row.sequence),
            Cell::Integer(i128::from(row.hour)),
            Cell::Unsigned(row.currency as u64),
            Cell::Unsigned(row.operation as u64),
            Cell::Unsigned(row.cause as u64),
            Cell::Unsigned(row.site as u64),
            integer(row.requested_delta_raw),
            Cell::Integer(i128::from(row.balance_before_raw)),
            Cell::Integer(i128::from(row.balance_after_raw)),
            Cell::Boolean(covered),
            identity_cell,
            integer(dated.map(|(hour, _)| hour)),
            unsigned(dated.map(|(_, sequence)| sequence)),
            Cell::Boolean(late),
        ];
        cells.extend(item_cells(ordinal, &item, class, recurring, upkeep));
        let sequence = Self::event(writer, key, (row.hour, Some(at)), POWER_KIND, cells.clone())?;
        if new {
            let items = self.tag_key(writer, tag, ITEMS, None)?;
            Self::event(
                writer,
                items,
                (row.hour, Some(at)),
                POWER_KIND,
                cells.clone(),
            )?;
        }
        if late && class != SKIPPED {
            let held = self.tag_key(writer, tag, LATE, None)?;
            Self::event(writer, held, (row.hour, Some(at)), POWER_KIND, cells)?;
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
        if row.currency == PowerCurrency::Command {
            let holder = rows::is_hold(row)
                .then(|| holder(row, power.dating.dated.as_deref()))
                .flatten();
            joined
                .command
                .entry((row.country, row.epoch))
                .or_default()
                .push((row.clone(), covered, holder));
        }
        if let (Some(charge), Some(category)) = (rows::charge(row), power.dating.category.clone())
            && category.as_ref() != "dailyDeficit"
        {
            joined.payments.entry(tag).or_default().push(Paid {
                currency: row.currency,
                category,
                hour: row.hour,
                charge,
            });
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
        let cells = vec![
            Cell::Unsigned(u64::from(row.country)),
            Cell::Unsigned(u64::from(row.instance)),
            Cell::Unsigned(row.epoch),
            Cell::Unsigned(row.sequence),
            unsigned(row.before_party),
            unsigned(row.after_party),
            unsigned(row.before_ideology),
            unsigned(row.after_ideology),
            optional_text(writer, government.tokens.0.as_deref().map(str::as_bytes))?,
            optional_text(writer, government.tokens.1.as_deref().map(str::as_bytes))?,
            unsigned(row.actor_country),
            Cell::Unsigned(row.cause as u64),
            Cell::Unsigned(row.origin_kind as u64),
            unsigned(row.origin_token),
            text(writer, &row.origin_key)?,
            Cell::Boolean(covered),
        ];
        Self::event(writer, key, (row.hour, Some(at)), GOVERNMENT_KIND, cells)?;
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
        let cells = vec![
            Cell::Unsigned(u64::from(row.country)),
            Cell::Unsigned(u64::from(row.instance)),
            Cell::Unsigned(row.epoch),
            integer(row.pp_gross_gain_raw),
            integer(row.pp_cost_raw),
            integer(row.pp_net_gain_raw),
            integer(row.pp_realized_delta_raw),
            unsigned(row.pp_sequence),
            integer(row.cp_gain_raw),
            integer(row.cp_realized_delta_raw),
            unsigned(row.cp_sequence),
            integer(reads.0),
            integer(reads.1),
        ];
        // A revision of a settled day is also kept apart: no rollup counts it.
        if self.late(row.day_hour) {
            let held = self.tag_key(writer, tag, LATE, None)?;
            Self::event(
                writer,
                held,
                (row.day_hour, Some(at)),
                DAILY_KIND,
                cells.clone(),
            )?;
        } else {
            let daily = rows::daily(&row, at, reads);
            let gains = [rows::gain(&daily, false)?, rows::gain(&daily, true)?];
            self.countries[tag as usize]
                .rollups
                .add_daily((row.day_hour, row.country, row.epoch), gains);
        }
        Self::event(
            writer,
            daily_key,
            (row.day_hour, Some(at)),
            DAILY_KIND,
            cells,
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
                self.tag_days.remove(&(day.tag, key.2, key.0, key.1));
                joined.dirty.insert(day.tag);
            }
        }
        if let Some(old) = self.days.insert(key, Day { row, reads, tag }) {
            self.tag_days.remove(&(old.tag, key.2, key.0, key.1));
            joined.dirty.insert(old.tag);
        }
        self.tag_days.insert((tag, key.2, key.0, key.1));
        joined.dirty.insert(tag);
        Ok(())
    }

    fn record_cost(
        &mut self,
        writer: &mut Writer<'_>,
        cost: Cost,
        at: i32,
        applications: &HashSet<(u32, u64, i32, u64)>,
        joined: &mut Joined,
    ) -> io::Result<()> {
        let row = &cost.row;
        let key = (row.country, row.epoch, row.day_hour, row.ordinal);
        let Some(tag) = self.tag_at(row.country, row.epoch, row.day_hour) else {
            self.waiting.insert((1, row.key()), Waiting::Cost(cost));
            return Ok(());
        };
        let (covered, dated, revision, held_item) = match self.recorded_costs.get(&key) {
            Some(old) if old.row == *row => return Ok(()),
            // A revision keeps what the first record read.
            Some(old) => (old.covered, old.dated, true, Some(old.item)),
            None => {
                let covered = self.covered(
                    row.country,
                    row.epoch,
                    row.day_hour,
                    PoliticsCaptureCoverageRow::PP | PoliticsCaptureCoverageRow::PP_DAILY_COSTS,
                    at,
                ) && !self.unreconciled(row.country);
                let dated = match &cost.dating.dated {
                    Some(bytes) => Some(self.dated(writer, tag, at, bytes)?),
                    None => None,
                };
                (covered, dated, false, None)
            }
        };
        let cost_key = self.tag_key(writer, tag, COSTS, None)?;
        let identity = identity_cells(writer, &row.identity)?;
        let late = self.late(row.day_hour);
        let application = (row.country, row.epoch, row.day_hour, row.pp_sequence);
        let day = self.days.get(&(row.country, row.epoch, row.day_hour));
        let matched = applications.contains(&application)
            || day.is_some_and(|day| day.row.pp_sequence == Some(row.pp_sequence));
        // The day's political application read by then dates the payment's read.
        let read = day
            .and_then(|day| day.reads.0)
            .map_or(at, |read| read.max(at));
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
        let (ordinal, item, new) = match held_item {
            Some(ordinal) => (
                ordinal,
                self.countries[tag as usize].items[ordinal as usize],
                false,
            ),
            None => {
                let item = item_key(
                    cost.dating.dated.as_deref(),
                    &format!(
                        "cost {:?} {:?} {:?} {}",
                        row.cause,
                        row.component,
                        row.identity,
                        row.cost_raw < 0
                    ),
                );
                self.item(
                    tag,
                    item,
                    (true, row.country, row.epoch, 0, row.day_hour, row.ordinal),
                )
            }
        };
        let mut cells = vec![
            Cell::Unsigned(u64::from(row.country)),
            Cell::Unsigned(u64::from(row.instance)),
            Cell::Unsigned(row.epoch),
            Cell::Unsigned(u64::from(row.ordinal)),
            Cell::Unsigned(row.pp_sequence),
            Cell::Unsigned(row.component as u64),
            Cell::Integer(i128::from(row.cost_raw)),
            Cell::Unsigned(row.cause as u64),
            Cell::Unsigned(row.site as u64),
            identity,
            Cell::Boolean(covered),
            integer(dated.map(|(hour, _)| hour)),
            unsigned(dated.map(|(_, sequence)| sequence)),
            Cell::Boolean(revision),
            Cell::Boolean(late),
        ];
        cells.extend(item_cells(ordinal, &item, class, recurring, upkeep));
        let sequence = Self::event(
            writer,
            cost_key,
            (row.day_hour, Some(at)),
            COST_KIND,
            cells.clone(),
        )?;
        if new {
            let items = self.tag_key(writer, tag, ITEMS, None)?;
            Self::event(
                writer,
                items,
                (row.day_hour, Some(at)),
                COST_KIND,
                cells.clone(),
            )?;
        }
        let field = (ordinal, row.country, row.epoch);
        if revision || class == SKIPPED {
        } else if late {
            let held = self.tag_key(writer, tag, LATE, None)?;
            Self::event(writer, held, (row.day_hour, Some(at)), COST_KIND, cells)?;
        } else if class == UNMATCHED || class == CREDIT {
            let field = (class == CREDIT).then_some(field);
            self.countries[tag as usize]
                .rollups
                .count(field, row.day_hour, true);
        } else {
            {
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
                && let Some(category) = cost.dating.category.clone()
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
                row: row.clone(),
                covered,
                dated,
                item: ordinal,
            },
        );
        joined.dirty.insert(tag);
        Ok(())
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
    fn index_power(&mut self, key: u128, row: &CountryPowerChangeRow, held: bool) {
        if row.cause == PowerCause::DailyGain && row.currency == PowerCurrency::Political {
            let entry = (row.country, row.hour, key);
            if held {
                self.daily_gains.insert(entry);
            } else {
                self.daily_gains.remove(&entry);
            }
        } else if row.operation == PowerOperation::Add && row.requested_delta_raw.is_none() {
            let bit = match row.currency {
                PowerCurrency::Political => PoliticsCaptureCoverageRow::PP,
                PowerCurrency::Command => PoliticsCaptureCoverageRow::CP,
            };
            let entry = (row.country, row.epoch, row.hour, bit, key);
            if held {
                self.unknown_adds.insert(entry);
            } else {
                self.unknown_adds.remove(&entry);
            }
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
        let slots = self.countries[tag as usize].slots.clone();
        let active = self.countries[tag as usize].active && !slots.is_empty();
        let changes_known = self.complete & facts::bit(TableId::CountryPowerChanges) != 0;
        let daily_known = self.complete & facts::bit(TableId::CountryPowerDaily) != 0
            && self.complete & facts::bit(TableId::CountryPowerDailyCosts) != 0;
        let mut rates = BTreeMap::<String, u64>::new();
        let mut coverage = self.countries[tag as usize].coverage.take();
        let mut spending = None;
        let mut current = false;
        for &slot in slots.iter().filter(|_| active) {
            let Some(country) = self.census.get(&slot).cloned() else {
                continue;
            };
            let witness = self.slot_witness(slot);
            let mut value = witness.as_ref().map_or_else(
                || LedgerCoverage {
                    country: Some(country.id),
                    instance: None,
                    read_hour: hour,
                    since_hour: Some(hour),
                    through_hour: Some(hour),
                    epoch: None,
                    streams: 0,
                    reason: Some("country identity or politics coverage unavailable".into()),
                },
                |row| rows::capture(row, hour),
            );
            if value.through_hour != Some(hour) {
                value = LedgerCoverage {
                    streams: 0,
                    since_hour: Some(hour),
                    through_hour: Some(hour),
                    reason: Some("politics coverage is stale at this read".into()),
                    ..value
                };
            }
            let raw = witness.as_ref().map_or(country.id, |row| row.country);
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
                value.reason.clone()
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
                    if adjacent && {
                        let mut lazy = old.clone();
                        lazy.through_hour = Some(hour - 1);
                        rows::continues_coverage(&lazy, &value)
                    } =>
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
            .as_ref()
            .filter(|row| row.through_hour == Some(hour))
            .and_then(identity_of);
        let starts = !adjacent || identity != self.countries[tag as usize].identity;
        self.write_coverage(writer, tag, hour, starts, coverage)?;
        self.write_spending(writer, tag, hour, adjacent, starts, spending)?;
        self.write_holds(writer, tag, hour, adjacent, active, joined)?;
        self.write_income(writer, tag, hour)?;
        self.write_parts(writer, tag, hour, previous, starts, identity, joined)?;
        self.countries[tag as usize].identity = identity;
        Ok(())
    }

    /// `tag`'s capture coverage at `hour`: the open piece, its fact written when it changes.
    fn write_coverage(
        &mut self,
        writer: &mut Writer<'_>,
        tag: u32,
        hour: i32,
        starts: bool,
        coverage: Option<LedgerCoverage>,
    ) -> io::Result<()> {
        let key = self.tag_key(writer, tag, CAPTURE, None)?;
        let Some(merged) = coverage else {
            let country = &mut self.countries[tag as usize];
            country.coverage = None;
            self.carry.value(writer, key, hour, None, false)?;
            return Ok(());
        };
        let mut signature = merged.clone();
        signature.through_hour = None;
        let held = self.countries[tag as usize]
            .capture
            .as_ref()
            .filter(|(_, held)| *held == signature)
            .map(|(sequence, _)| *sequence);
        let fact = match held {
            Some(sequence) => sequence,
            None => {
                let facts_key = self.tag_key(writer, tag, CAPTURE_FACTS, None)?;
                let cells = vec![
                    unsigned(merged.country),
                    unsigned(merged.instance),
                    Cell::Integer(i128::from(merged.read_hour)),
                    integer(merged.since_hour),
                    unsigned(
                        merged
                            .epoch
                            .as_deref()
                            .and_then(|epoch| epoch.parse::<u64>().ok()),
                    ),
                    Cell::Unsigned(u64::from(merged.streams)),
                    optional_text(writer, merged.reason.as_deref().map(str::as_bytes))?,
                ];
                let sequence = Self::event(writer, facts_key, (hour, None), CAPTURE_KIND, cells)?;
                self.countries[tag as usize].capture = Some((sequence, signature));
                sequence
            }
        };
        self.carry
            .value(writer, key, hour, Some(i128::from(fact)), starts)?;
        self.countries[tag as usize].coverage = Some(merged);
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
        let cover = (merged.political, merged.command, merged.reason.clone());
        if self.countries[tag as usize].cover.as_ref() != Some(&cover) {
            let key = self.tag_key(writer, tag, COVER_REASONS, None)?;
            let cells = vec![
                Cell::Integer(i128::from(merged.read_hour)),
                integer(merged.since_hour),
                integer(merged.through_hour),
                Cell::Boolean(merged.political),
                Cell::Boolean(merged.command),
                optional_text(writer, merged.reason.as_deref().map(str::as_bytes))?,
            ];
            Self::event(writer, key, (hour, None), COVER_KIND, cells)?;
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
        let snapshot = if active {
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
            snapshot(holds, total, "CountryCommandPower row unavailable")
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
            snapshot(
                holds,
                total,
                refusal.as_deref().unwrap_or("Countries unavailable"),
            )
        };
        let holds = &mut self.countries[tag as usize].holds;
        let continues =
            adjacent && holds.last.is_some() && holds.snapshot.as_ref() == Some(&snapshot);
        holds.last = Some(hour);
        if continues {
            return Ok(());
        }
        holds.snapshot = Some(snapshot.clone());
        let key = self.tag_key(writer, tag, HOLDS, None)?;
        let mut holders = Vec::with_capacity(snapshot.holders.len());
        for (id, holder) in &snapshot.holders {
            let (script, saved) = match &holder.character {
                Some((script, saved)) => (script.as_deref(), Some(saved.as_str())),
                None => (None, None),
            };
            let cells = vec![
                text(writer, id.as_bytes())?,
                text(writer, holder.kind.as_bytes())?,
                Cell::Boolean(holder.ability),
                optional_text(writer, holder.token.as_deref().map(str::as_bytes))?,
                optional_text(writer, script.map(str::as_bytes))?,
                optional_text(writer, saved.map(str::as_bytes))?,
                Cell::Integer(holder.amount),
            ];
            holders.push(Cell::List(writer.cells(cells)?));
        }
        let cells = vec![
            integer(snapshot.total),
            Cell::List(writer.cells(holders)?),
            optional_text(writer, snapshot.missing.as_deref().map(str::as_bytes))?,
        ];
        Self::event(writer, key, (hour, None), HOLD_KIND, cells)?;
        Ok(())
    }

    /// `tag`'s current daily income by currency: its latest recorded day the daily table holds.
    fn write_income(&mut self, writer: &mut Writer<'_>, tag: u32, hour: i32) -> io::Result<()> {
        let current = self
            .tag_days
            .range((tag, i32::MIN, 0, 0)..=(tag, i32::MAX, u32::MAX, u64::MAX))
            .rev()
            .find(|&&(_, _, country, _)| self.daily.contains_key(&country))
            .and_then(|&(_, day_hour, country, epoch)| self.days.get(&(country, epoch, day_hour)))
            .map(|day| (day.row.pp_net_gain_raw, day.row.cp_gain_raw));
        let values = [
            current.and_then(|(pp, _)| pp),
            current.and_then(|(_, cp)| cp),
        ];
        for (measure, value) in INCOME.into_iter().zip(values) {
            let key = self.tag_key(writer, tag, measure, None)?;
            self.carry
                .value(writer, key, hour, value.map(i128::from), false)?;
        }
        Ok(())
    }

    /// The part dimension of `category` in `currency`, numbered and named when new.
    fn category(
        &mut self,
        writer: &mut Writer<'_>,
        currency: PowerCurrency,
        category: &str,
    ) -> io::Result<u64> {
        let index = usize::from(currency == PowerCurrency::Command);
        let spelling = format!("{}:{category}", CURRENCIES[index]);
        if let Some(&dimension) = self.categories.get(&spelling) {
            return Ok(dimension);
        }
        let dimension = CATEGORY_PARTS + self.categories.len() as u64;
        let cell = text(writer, spelling.as_bytes())?;
        writer.catalogue(CATEGORY_NAMES + dimension, cell)?;
        self.categories.insert(spelling, dimension);
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
        let mut values = BTreeMap::new();
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
            values = parts.values.clone();
            since = parts.since;
            if first <= day {
                let held: Vec<&Day> = self
                    .days
                    .range((country, 0, day)..=(country, u64::MAX, day))
                    .map(|(_, held)| held)
                    .filter(|held| held.tag == tag && held.row.day_hour == day)
                    .collect();
                // One capture's later read revises its day; only separate captures are ambiguous.
                let supplied = |command: bool| {
                    let mut found = held.iter().filter(|held| {
                        if command { held.reads.1 } else { held.reads.0 }
                            .is_some_and(|read| read <= hour)
                    });
                    let first = found.next().copied();
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
        let categories: Vec<u64> = {
            let country = &mut self.countries[tag as usize];
            for &dimension in values.keys() {
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    country.categories.entry(dimension)
                {
                    entry.insert(hour);
                    country.categories_changed = true;
                }
            }
            country.categories.keys().copied().collect()
        };
        for index in 0..2 {
            let covered = since[index].is_some_and(|first| first <= day);
            let mine: Vec<u64> = categories
                .iter()
                .copied()
                .filter(|dimension| self.category_currency.get(dimension) == Some(&index))
                .collect();
            let total = if covered {
                rows::sum_raw(
                    mine.iter()
                        .map(|dimension| values.get(dimension).copied().unwrap_or(Some(0))),
                )?
            } else {
                None
            };
            let shown = covered && total.is_some();
            let key = self.tag_key(writer, tag, PARTS, Some(ZERO[index]))?;
            self.carry
                .value(writer, key, hour, shown.then_some(0), starts)?;
            for dimension in mine {
                let key = self.tag_key(writer, tag, PARTS, Some(dimension))?;
                let value = shown
                    .then(|| values.get(&dimension).copied().unwrap_or(Some(0)))
                    .flatten();
                self.carry.value(writer, key, hour, value, starts)?;
            }
            let key = self.tag_key(writer, tag, PARTS, Some(TOTAL[index]))?;
            self.carry.value(writer, key, hour, total, starts)?;
        }
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
            census: self.census.values().map(SavedCensus::from).collect(),
            census_complete: self.census_complete,
            known: self.known.iter().map(|(&slot, &tag)| (slot, tag)).collect(),
            tags: self.tags.iter().map(|tag| tag.to_string()).collect(),
            countries: &self.countries,
            refused: self.refused.clone(),
            links: self.links.iter().map(|(&raw, &slot)| (raw, slot)).collect(),
            extended: self.extended.to_vec(),
            extended_through: self.extended_through,
            advanced: self.advanced.clone(),
            volatile: self.volatile.iter().copied().collect(),
            identities: self
                .identities
                .iter()
                .map(|(&identity, tags)| (identity, tags.clone()))
                .collect(),
            complete: self.complete,
            recorded: self.recorded.iter().copied().collect(),
            days: self
                .days
                .iter()
                .map(|(&key, day)| (key, day.reads, day.tag))
                .collect(),
            recorded_costs: self
                .recorded_costs
                .iter()
                .map(|(&key, cost)| (key, cost.covered, cost.dated, cost.item))
                .collect(),
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
                .map(|(spelling, &dimension)| (spelling.clone(), dimension))
                .collect(),
            tomorrow: self.tomorrow.clone(),
        };
        let json = serde_json::to_vec(&state)?;
        let rows = serde_json::to_vec(&SavedRows {
            witnesses: self.witnesses.values().map(SavedCapture::from).collect(),
            daily: self.daily.values().map(SavedDaily::from).collect(),
            days: self
                .days
                .values()
                .map(|day| SavedDaily::from(&day.row))
                .collect(),
            costs: self.costs.values().map(SavedCost::from).collect(),
            recorded: self
                .recorded_costs
                .values()
                .map(|cost| SavedCost::from(&cost.row))
                .collect(),
        })?;
        for part in [&json, &rows] {
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
        let mut parts = Vec::with_capacity(2);
        for _ in 0..2 {
            let length = usize::try_from(d.u64()?).map_err(|_| invalid("saved part length"))?;
            parts.push(d.bytes(length)?);
        }
        let at = d.position();
        let saved: Loaded = serde_json::from_slice(parts[0])?;
        let rows: SavedRows = serde_json::from_slice(parts[1])?;
        let witnesses: Vec<PoliticsCaptureCoverageRow> =
            rows.witnesses.into_iter().map(Into::into).collect();
        let daily: Vec<CountryPowerDailyRow> = rows.daily.into_iter().map(Into::into).collect();
        let days: Vec<CountryPowerDailyRow> = rows.days.into_iter().map(Into::into).collect();
        let costs = rows
            .costs
            .into_iter()
            .map(CountryPowerDailyCostRow::try_from)
            .collect::<io::Result<Vec<_>>>()?;
        let recorded = rows
            .recorded
            .into_iter()
            .map(CountryPowerDailyCostRow::try_from)
            .collect::<io::Result<Vec<_>>>()?;
        let mut emit = Self::new(content);
        emit.actors = Actors::resume(SCHEMA, directory)?;
        emit.carry = Carry::load(&bytes[at..])?;
        emit.last = saved.last;
        emit.known_from = saved.known_from;
        emit.begun = saved.begun;
        emit.census = saved
            .census
            .into_iter()
            .map(|row| (row.slot, row.into()))
            .collect();
        emit.census_complete = saved.census_complete;
        emit.known = saved.known.into_iter().collect();
        emit.tags = saved.tags.into_iter().map(Into::into).collect();
        emit.numbers = emit
            .tags
            .iter()
            .enumerate()
            .map(|(at, tag)| (tag.clone(), at as u32))
            .collect();
        emit.countries = saved.countries;
        for country in &mut emit.countries {
            country.item_index = country
                .items
                .iter()
                .enumerate()
                .map(|(ordinal, item)| (item.key, ordinal as u32))
                .collect();
        }
        if emit.countries.len() != emit.tags.len() {
            return Err(invalid("saved countries and tags disagree"));
        }
        emit.refused = saved.refused;
        emit.links = saved.links.into_iter().collect();
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
        emit.witnesses = witnesses
            .into_iter()
            .map(|row| (row.country, row))
            .collect();
        emit.extended = Arc::from(saved.extended);
        emit.extended_through = saved.extended_through;
        emit.advanced = saved.advanced;
        emit.volatile = saved.volatile.into_iter().collect();
        emit.identities = saved.identities.into_iter().collect();
        emit.complete = saved.complete;
        emit.recorded = saved.recorded.into_iter().collect();
        emit.daily = daily.into_iter().map(|row| (row.country, row)).collect();
        if days.len() != saved.days.len() || recorded.len() != saved.recorded_costs.len() {
            return Err(invalid("saved rows and their records disagree"));
        }
        for (row, (key, reads, tag)) in days.into_iter().zip(saved.days) {
            emit.tag_days.insert((tag, key.2, key.0, key.1));
            emit.days.insert(key, Day { row, reads, tag });
        }
        for row in costs {
            let sum = emit
                .sums
                .entry((row.country, row.epoch, row.day_hour, row.pp_sequence))
                .or_default();
            sum.0 = sum.0.wrapping_add(row.cost_raw);
            sum.1 += 1;
            emit.costs.insert(row.key(), row);
        }
        for (row, (key, covered, dated, item)) in recorded.into_iter().zip(saved.recorded_costs) {
            emit.recorded_costs.insert(
                key,
                Recorded {
                    row,
                    covered,
                    dated,
                    item,
                },
            );
        }
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
            let index = usize::from(spelling.starts_with("Command:"));
            emit.category_currency.insert(dimension, index);
            emit.categories.insert(spelling, dimension);
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

/// The holder a command power row's allocation names, by the operands its facts recorded.
fn holder(row: &CountryPowerChangeRow, dated: Option<&[u8]>) -> Option<Holder> {
    let facts = dated
        .and_then(|bytes| serde_json::from_slice::<items::Dated>(bytes).ok())
        .map(|dated| dated.facts)
        .unwrap_or_else(|| items::event_facts(None, &row.identity, row.cause, None));
    Some(Holder {
        kind: rows::holder_kind(row),
        amount: 0,
        ability: row.cause == PowerCause::AbilityActivation
            || row.identity.object_kind == PowerObjectKind::Ability,
        token: facts.token,
        character: facts
            .character
            .map(|character| (character.script_token, character.saved_name)),
    })
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

/// The snapshot of `holds` against the hour's total: an observed total never keeps stale
/// identities or invents a negative remainder.
fn snapshot(holds: &mut Holds, total: Option<i64>, missing: &str) -> Snapshot {
    let Some(value) = total else {
        holds.active.clear();
        return Snapshot {
            total: None,
            holders: Vec::new(),
            missing: Some(missing.to_owned()),
        };
    };
    let attributed: i128 = holds.active.values().map(|holder| holder.amount).sum();
    if value == 0 || attributed > i128::from(value) {
        holds.active.clear();
    }
    Snapshot {
        total,
        holders: holds
            .active
            .iter()
            .map(|(id, holder)| (id.clone(), holder.clone()))
            .collect(),
        missing: None,
    }
}

#[derive(Serialize, Deserialize)]
struct SavedCensus {
    id: u32,
    slot: u32,
    tag: Vec<u8>,
}

impl From<&CensusRow> for SavedCensus {
    fn from(row: &CensusRow) -> Self {
        Self {
            id: row.id,
            slot: row.slot,
            tag: row.tag.to_vec(),
        }
    }
}

impl From<SavedCensus> for CensusRow {
    fn from(row: SavedCensus) -> Self {
        Self {
            id: row.id,
            slot: row.slot,
            tag: Arc::from(row.tag),
        }
    }
}

type RecordedCost = ((u32, u64, i32, u32), bool, Option<(i32, u64)>, u32);
type RecordedDay = ((u32, u64, i32), (Option<i32>, Option<i32>), u32);

#[derive(Serialize)]
struct Saved<'a> {
    last: Option<i32>,
    known_from: Option<i32>,
    begun: bool,
    census: Vec<SavedCensus>,
    census_complete: bool,
    known: Vec<(u32, u32)>,
    tags: Vec<String>,
    countries: &'a [Country],
    refused: BTreeMap<u32, String>,
    links: Vec<(u32, u32)>,
    extended: Vec<u32>,
    extended_through: i32,
    advanced: Vec<u32>,
    volatile: Vec<u32>,
    identities: Vec<((u32, u64), Vec<(i32, u32)>)>,
    complete: u16,
    recorded: Vec<(u32, u64, u64)>,
    days: Vec<RecordedDay>,
    recorded_costs: Vec<RecordedCost>,
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
    census: Vec<SavedCensus>,
    census_complete: bool,
    known: Vec<(u32, u32)>,
    tags: Vec<String>,
    countries: Vec<Country>,
    refused: BTreeMap<u32, String>,
    links: Vec<(u32, u32)>,
    extended: Vec<u32>,
    extended_through: i32,
    advanced: Vec<u32>,
    volatile: Vec<u32>,
    identities: Vec<((u32, u64), Vec<(i32, u32)>)>,
    complete: u16,
    recorded: Vec<(u32, u64, u64)>,
    days: Vec<RecordedDay>,
    recorded_costs: Vec<RecordedCost>,
    command: Vec<(u32, i64)>,
    presence: Option<(Vec<String>, Vec<String>)>,
    settled: Option<i32>,
    settled_written: Option<i32>,
    warnings: BTreeMap<String, u64>,
    categories: Vec<(String, u64)>,
    tomorrow: BTreeMap<i32, BTreeSet<u32>>,
}

/// A capture row as the saved state keeps it.
#[derive(Serialize, Deserialize)]
struct SavedCapture(u32, u32, u64, i32, i32, u8);

impl From<&PoliticsCaptureCoverageRow> for SavedCapture {
    fn from(row: &PoliticsCaptureCoverageRow) -> Self {
        Self(
            row.country,
            row.instance,
            row.epoch,
            row.since_hour,
            row.through_hour,
            row.streams,
        )
    }
}

impl From<SavedCapture> for PoliticsCaptureCoverageRow {
    fn from(row: SavedCapture) -> Self {
        Self {
            country: row.0,
            instance: row.1,
            epoch: row.2,
            since_hour: row.3,
            through_hour: row.4,
            streams: row.5,
        }
    }
}

type Political = (
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<u64>,
);

/// A daily row as the saved state keeps it.
#[derive(Serialize, Deserialize)]
struct SavedDaily {
    country: u32,
    instance: u32,
    epoch: u64,
    day_hour: i32,
    pp: Political,
    cp: (Option<i64>, Option<i64>, Option<u64>),
}

impl From<&CountryPowerDailyRow> for SavedDaily {
    fn from(row: &CountryPowerDailyRow) -> Self {
        Self {
            country: row.country,
            instance: row.instance,
            epoch: row.epoch,
            day_hour: row.day_hour,
            pp: rows::daily_political(row),
            cp: rows::daily_command(row),
        }
    }
}

impl From<SavedDaily> for CountryPowerDailyRow {
    fn from(row: SavedDaily) -> Self {
        Self {
            country: row.country,
            instance: row.instance,
            epoch: row.epoch,
            day_hour: row.day_hour,
            pp_gross_gain_raw: row.pp.0,
            pp_cost_raw: row.pp.1,
            pp_net_gain_raw: row.pp.2,
            pp_realized_delta_raw: row.pp.3,
            pp_sequence: row.pp.4,
            cp_gain_raw: row.cp.0,
            cp_realized_delta_raw: row.cp.1,
            cp_sequence: row.cp.2,
        }
    }
}

/// An item identity as the saved state keeps it: its kinds by their recorded wire values.
#[derive(Serialize, Deserialize)]
struct SavedIdentity {
    action: u8,
    item_token: Option<u32>,
    item_key: Vec<u8>,
    slot_key: Vec<u8>,
    character: Option<u32>,
    object_kind: u8,
    object_reference: Option<u64>,
    target_country: Option<u32>,
    target_state: Option<u32>,
    target_region: Option<u32>,
    origin_kind: u8,
    origin_token: Option<u32>,
    origin_key: Vec<u8>,
}

impl From<&PowerIdentity> for SavedIdentity {
    fn from(identity: &PowerIdentity) -> Self {
        Self {
            action: identity.action as u8,
            item_token: identity.item_token,
            item_key: identity.item_key.clone(),
            slot_key: identity.slot_key.clone(),
            character: identity.character,
            object_kind: identity.object_kind as u8,
            object_reference: identity.object_reference,
            target_country: identity.target_country,
            target_state: identity.target_state,
            target_region: identity.target_region,
            origin_kind: identity.origin_kind as u8,
            origin_token: identity.origin_token,
            origin_key: identity.origin_key.clone(),
        }
    }
}

impl TryFrom<SavedIdentity> for PowerIdentity {
    type Error = io::Error;
    fn try_from(identity: SavedIdentity) -> io::Result<Self> {
        Ok(Self {
            action: PowerAction::try_from(identity.action)?,
            item_token: identity.item_token,
            item_key: identity.item_key,
            slot_key: identity.slot_key,
            character: identity.character,
            object_kind: PowerObjectKind::try_from(identity.object_kind)?,
            object_reference: identity.object_reference,
            target_country: identity.target_country,
            target_state: identity.target_state,
            target_region: identity.target_region,
            origin_kind: PowerOriginKind::try_from(identity.origin_kind)?,
            origin_token: identity.origin_token,
            origin_key: identity.origin_key,
        })
    }
}

/// A cost row as the saved state keeps it.
#[derive(Serialize, Deserialize)]
struct SavedCost {
    country: u32,
    instance: u32,
    ordinal: u32,
    epoch: u64,
    day_hour: i32,
    pp_sequence: u64,
    component: u8,
    cost_raw: i64,
    cause: u16,
    site: u16,
    identity: SavedIdentity,
}

impl From<&CountryPowerDailyCostRow> for SavedCost {
    fn from(row: &CountryPowerDailyCostRow) -> Self {
        Self {
            country: row.country,
            instance: row.instance,
            ordinal: row.ordinal,
            epoch: row.epoch,
            day_hour: row.day_hour,
            pp_sequence: row.pp_sequence,
            component: row.component as u8,
            cost_raw: row.cost_raw,
            cause: row.cause as u16,
            site: row.site as u16,
            identity: (&row.identity).into(),
        }
    }
}

impl TryFrom<SavedCost> for CountryPowerDailyCostRow {
    type Error = io::Error;
    fn try_from(row: SavedCost) -> io::Result<Self> {
        Ok(Self {
            country: row.country,
            instance: row.instance,
            ordinal: row.ordinal,
            epoch: row.epoch,
            day_hour: row.day_hour,
            pp_sequence: row.pp_sequence,
            component: PowerDailyCostKind::try_from(row.component)?,
            cost_raw: row.cost_raw,
            cause: PowerCause::try_from(row.cause)?,
            site: PowerSite::try_from(row.site)?,
            identity: row.identity.try_into()?,
        })
    }
}

/// The rows the saved state holds beside its records.
#[derive(Serialize, Deserialize)]
struct SavedRows {
    witnesses: Vec<SavedCapture>,
    daily: Vec<SavedDaily>,
    days: Vec<SavedDaily>,
    costs: Vec<SavedCost>,
    recorded: Vec<SavedCost>,
}
