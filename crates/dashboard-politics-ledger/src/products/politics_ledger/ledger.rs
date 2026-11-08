//! The ledger's accounting, apart from where it is written ([`Out`]). What joins countries is the
//! branch's: the census, links and captures, the tag each capture identity names, the settled
//! frontier, the categories. Each raw country's rows (held, recorded, waiting for a tag, matched
//! to their day's application) are its own ([`Raw`]), and each tag's accounting of them is the
//! tag's ([`Country`]). The emit runs it for every country ([`super::emit`]).

use super::emit::{
    BALANCE, CATEGORY_PARTS, COSTS, COST_KIND, COVERED, COVER_KIND, COVER_REASONS, CREDIT,
    CURRENCIES, DAILY, DAILY_KIND, GOVERNMENTS, GOVERNMENT_KIND, HOLDS, HOLD_KIND, LATE,
    PARTS, PAYMENT, POWER, POWER_KIND, SKIPPED, TOTAL, UNMATCHED, ZERO,
};
use super::facts::{self, CensusRow, Cost, Government, Hour, Power};
use super::records::{CostRecord, DailyRecord, PowerRecord};
use super::rows::Holder;
use super::tally::{self, Contribution};
use super::*;
use super::{items, present};
use dashboard_base::prepared::codec;
use dashboard_base::products::politics_content::PoliticsContent;
use dashboard_series::Cell;
use dashboard_series::hash::{FastMap, Words};
use std::{collections::HashSet, hash::BuildHasherDefault};

/// A set keyed by [`Words`], as [`FastMap`] is.
pub(super) type FastSet<K> = HashSet<K, BuildHasherDefault<Words>>;

fn invalid(what: &str) -> io::Error {
    dashboard_core::invalid(format!("Politics ledger: {what}"))
}

/// The native day a power row closes: daily upkeep and gains only.
pub(super) fn power_day(row: &CountryPowerChangeRow) -> Option<i32> {
    matches!(row.cause, PowerCause::AbilityUpkeep | PowerCause::DailyGain)
        .then(|| row.hour.div_euclid(24) * 24)
}

/// A field of an event, as the accounting hands it to be written.
#[derive(Clone, Copy)]
pub(super) enum Field<'a> {
    Plain(Cell),
    /// A record's encoding ([`super::records`]), written as a text cell.
    Record(&'a [u8]),
    /// An item's ordinal, as the tag's accounting numbers it.
    Item(u32),
}

/// Where the accounting is written.
pub(super) trait Out {
    /// Tag `tag`, spelled `spelling`, is first numbered at `hour`.
    fn tagged(&mut self, tag: u32, spelling: &str, hour: i32) -> io::Result<()>;
    /// An event of `tag`'s `measure`: its sequence, which orders the tag's payments.
    fn event(
        &mut self,
        tag: u32,
        measure: u64,
        hours: (i32, Option<i32>),
        kind: u64,
        fields: &[Field<'_>],
    ) -> io::Result<u64>;
    /// `tag`'s value of `measure` (and `dimension`) from `hour` on, as [`dashboard_series::carry::Carry::value`].
    fn value(
        &mut self,
        tag: u32,
        measure: u64,
        dimension: Option<u64>,
        hour: i32,
        value: Option<i128>,
        starts: bool,
    ) -> io::Result<()>;
    /// Category dimension `dimension` of currency `currency` is first named.
    fn named(&mut self, dimension: u64, currency: usize, category: &str) -> io::Result<()>;
    /// A payment of `tag` names item `ordinal`, with its record `shown` where it is the first.
    fn item(
        &mut self,
        tag: u32,
        ordinal: u32,
        new: bool,
        shown: Option<&[u8]>,
        hours: (i32, Option<i32>),
        kind: u64,
    ) -> io::Result<()>;
    /// `tag`'s periods the settled frontier closed at `at`.
    fn settled(
        &mut self,
        tag: u32,
        at: i32,
        closed: Vec<(usize, i32, tally::Totals)>,
    ) -> io::Result<()>;
}

/// A tag's capture coverage at a read, as its account reads and writes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
pub(super) struct Coverage {
    pub(super) country: Option<u32>,
    pub(super) instance: Option<u32>,
    pub(super) since_hour: Option<i32>,
    pub(super) through_hour: Option<i32>,
    pub(super) epoch: Option<u64>,
    pub(super) streams: u8,
    pub(super) reason: Option<Gap>,
}

/// Why a read holds no current capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
pub(super) enum Gap {
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
#[derive(Clone, replay_spool::Spooled)]
pub(super) enum Waiting {
    Power(Power),
    Cost(Cost),
    Government(Government),
}

impl Waiting {
    fn identity(&self) -> (u32, u64) {
        match self {
            Self::Power(power) => (power.row.country, power.row.epoch),
            Self::Cost(cost) => (cost.row.country, cost.row.epoch),
            Self::Government(government) => (government.row.country, government.row.epoch),
        }
    }
}

/// A recorded daily row.
#[derive(Clone, Debug, PartialEq, Eq, replay_spool::Spooled)]
pub(super) struct Day {
    pub(super) row: CountryPowerDailyRow,
    pub(super) reads: (Option<i32>, Option<i32>),
    pub(super) tag: u32,
}

/// A held power row: what its deletion and the account's indexes read of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, replay_spool::Spooled)]
pub(super) struct HeldPower {
    pub(super) epoch: u64,
    pub(super) sequence: u64,
    pub(super) hour: i32,
    index: Indexed,
}

/// The index of its raw country a held power row is in ([`Raw::index_power`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, replay_spool::Spooled)]
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
            epoch: row.epoch,
            sequence: row.sequence,
            hour: row.hour,
            index,
        }
    }
}

/// A held cost: what its day's sums and records read of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, replay_spool::Spooled)]
pub(super) struct HeldCost {
    pub(super) country: u32,
    pub(super) epoch: u64,
    pub(super) day_hour: i32,
    pub(super) pp_sequence: u64,
    pub(super) ordinal: u32,
    pub(super) cost_raw: i64,
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
    pub(super) fn key(&self) -> u128 {
        Self::slot(self.country, self.ordinal)
    }

    /// The cost table's key of `country`'s cost `ordinal`.
    pub(super) fn slot(country: u32, ordinal: u32) -> u128 {
        CountryPowerDailyCostRow {
            country,
            ordinal,
            ..CountryPowerDailyCostRow::default()
        }
        .key()
    }

    /// Its application, within its raw country.
    fn application(&self) -> (u64, i32, u64) {
        (self.epoch, self.day_hour, self.pp_sequence)
    }
}

/// What one country's costs of a day and application read alike at an hour
/// ([`Ledger::cost_day`]).
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
#[derive(Clone, Debug, PartialEq, Eq, replay_spool::Spooled)]
pub(super) struct Recorded {
    pub(super) row: CountryPowerDailyCostRow,
    pub(super) covered: bool,
    pub(super) item: u32,
}

/// A hold snapshot's operands: its total, holders and why its total is missing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
pub(super) struct Snapshot {
    total: Option<i64>,
    holders: Vec<(String, Holder)>,
    missing: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
pub(super) struct Holds {
    pub(super) identity: Option<(u32, u64)>,
    pub(super) sequence: Option<u64>,
    pub(super) last: Option<i32>,
    pub(super) active: BTreeMap<String, Holder>,
    pub(super) snapshot: Option<Snapshot>,
}

/// A capture identity's spending parts of one day.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
pub(super) struct DayParts {
    /// The capture epoch they are of: another's start over.
    pub(super) epoch: Option<u64>,
    pub(super) day: i32,
    pub(super) values: BTreeMap<u64, Option<i128>>,
    pub(super) since: [Option<i32>; 2],
    pub(super) last: Option<i32>,
    pub(super) first: Option<i32>,
}

impl DayParts {
}

/// One tag's accounting.
#[derive(Clone, Default, Serialize, Deserialize, replay_spool::Spooled)]
pub(super) struct Country {
    #[serde(skip)]
    pub(super) slots: Vec<u32>,
    pub(super) active: bool,
    pub(super) coverage: Option<Coverage>,
    pub(super) spending: Option<LedgerSpendCoverage>,
    pub(super) cover: Option<(bool, bool, Option<String>)>,
    pub(super) identity: Option<(u32, u64)>,
    pub(super) holds: Holds,
    pub(super) parts: BTreeMap<u32, DayParts>,
    /// Each category's first hour, by part dimension.
    pub(super) categories: BTreeMap<u64, i32>,
    pub(super) categories_changed: bool,
    /// Warning rates counted for every accepted hour after `rated`.
    pub(super) rates: BTreeMap<String, u64>,
    pub(super) rated: Option<i32>,
    pub(super) warnings: BTreeMap<String, u64>,
    pub(super) warnings_changed: bool,
    /// Whether the census last listed it present, and refused.
    #[serde(default)]
    pub(super) presence: (bool, bool),
    /// Its items' keys in the order first paid.
    #[serde(skip)]
    pub(super) items: Vec<[u8; 32]>,
    #[serde(skip)]
    pub(super) item_index: FastMap<[u8; 32], u32>,
    /// Saved in the binary part with `items` ([`super::emit::Emit::save`]).
    #[serde(skip)]
    pub(super) rollups: tally::Rollups,
    /// The least settled frontier its rollups settle at.
    #[serde(skip)]
    #[spool(skip)]
    pub(super) due: Option<i32>,
}

impl Country {
    /// Counts its rates through `through`.
    pub(super) fn materialize(&mut self, through: i32) {
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

/// One raw country's rows: those its tables hold, those recorded, those waiting for a tag, and
/// how its costs match their day's application.
#[derive(Clone, Default, replay_spool::Spooled)]
pub(super) struct Raw {
    pub(super) power: FastMap<u128, HeldPower>,
    /// Of `power`, the political daily gains by hour, and the additions without an evaluated
    /// operand by epoch, hour and currency bit: what an account asks, found without walking.
    pub(super) daily_gains: BTreeSet<(i32, u128)>,
    pub(super) unknown_adds: BTreeSet<(u64, i32, u8, u128)>,
    /// The held government rows' epochs and sequences.
    pub(super) governments: FastMap<u128, (u64, u64)>,
    /// The power and government rows recorded, by epoch and sequence.
    pub(super) recorded: FastSet<(u64, u64)>,
    pub(super) waiting: BTreeMap<(u8, u128), Waiting>,
    pub(super) daily: Option<CountryPowerDailyRow>,
    /// Its recorded daily rows by epoch and day.
    pub(super) days: BTreeMap<(u64, i32), Day>,
    pub(super) costs: FastMap<u128, HeldCost>,
    pub(super) sums: FastMap<(u64, i32, u64), (i64, u32)>,
    /// Its recorded costs by epoch, day and ordinal.
    pub(super) recorded_costs: FastMap<(u64, i32, u32), Recorded>,
}

impl Raw {
    /// Adds held power row `row`, under `key`, to the indexes an account asks, or takes it out.
    fn index_power(&mut self, key: u128, row: HeldPower, held: bool) {
        match row.index {
            Indexed::DailyGain => {
                let entry = (row.hour, key);
                if held {
                    self.daily_gains.insert(entry);
                } else {
                    self.daily_gains.remove(&entry);
                }
            }
            Indexed::UnknownAdd(bit) => {
                let entry = (row.epoch, row.hour, bit, key);
                if held {
                    self.unknown_adds.insert(entry);
                } else {
                    self.unknown_adds.remove(&entry);
                }
            }
            Indexed::None => {}
        }
    }

    fn sum(&mut self, row: HeldCost) {
        let sum = self.sums.entry(row.application()).or_default();
        sum.0 = sum.0.wrapping_add(row.cost_raw);
        sum.1 += 1;
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

    /// Whether its tables hold a row an account reads.
    fn holds_rows(&self) -> bool {
        self.daily.is_some() || !self.power.is_empty() || !self.costs.is_empty()
    }

    /// Its tables as a whole read finds them: none held before.
    fn clear_tables(&mut self) {
        self.waiting.clear();
        self.power.clear();
        self.daily_gains.clear();
        self.unknown_adds.clear();
        self.governments.clear();
        self.daily = None;
        self.costs.clear();
        self.sums.clear();
    }

    /// Whether its daily table holds the political application (`epoch`, `day_hour`,
    /// `sequence`) now: the gains and costs it applies match it.
    fn applied(&self, epoch: u64, day_hour: i32, sequence: u64) -> bool {
        self.daily.as_ref().is_some_and(|row| {
            row.epoch == epoch && row.day_hour == day_hour && row.pp_sequence == Some(sequence)
        })
    }

    /// Whether its daily political cost differs from its components' sum.
    fn unreconciled(&self) -> bool {
        self.daily.as_ref().is_some_and(|row| {
            row.pp_sequence.is_some_and(|sequence| {
                let key = (row.epoch, row.day_hour, sequence);
                row.pp_cost_raw
                    .is_none_or(|cost| self.sums.get(&key).map_or(0, |sum| sum.0) != cost)
            })
        })
    }
}

/// Whether a read's spending is recorded in each currency, and why not.
#[derive(Clone, Copy)]
struct Spending {
    political: bool,
    command: bool,
    reason: Option<&'static str>,
}

/// A payment recorded this hour, for its country's day parts.
struct Paid {
    currency: PowerCurrency,
    category: Arc<str>,
    hour: i32,
    charge: Option<i128>,
}

/// Tags by number, as one bit each: the accounting takes them in ascending order.
#[derive(Clone, Default)]
pub(super) struct Tags(Vec<u64>);

impl Tags {
    pub(super) fn insert(&mut self, tag: u32) {
        let word = (tag / 64) as usize;
        if word >= self.0.len() {
            self.0.resize(word + 1, 0);
        }
        self.0[word] |= 1 << (tag % 64);
    }

    pub(super) fn iter(&self) -> Ascending {
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
pub(super) struct Ascending {
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
pub(super) struct Joined {
    /// The hour before the hour, and whether the hour follows it.
    pub(super) previous: Option<i32>,
    pub(super) adjacent: bool,
    /// Tags whose inputs the hour changed.
    pub(super) dirty: Tags,
    /// The capture identities the hour named their first tag.
    pub(super) tagged: Vec<(u32, u64)>,
    /// Command power rows recorded this hour by identity: covered, and their holder's operands.
    command: FastMap<(u32, u64), Vec<(CountryPowerChangeRow, bool, Option<Holder>)>>,
    /// Costs recorded this hour by tag: their amount and whether their application matched.
    costs: FastMap<u32, Vec<(i64, bool)>>,
    payments: FastMap<u32, Vec<Paid>>,
    /// Raw countries with a political daily gain read before its application.
    unmatched: FastSet<u32>,
}

/// The buffers each account reuses.
#[derive(Default)]
struct Scratch {
    values: Vec<(u64, Option<i128>)>,
    categories: Vec<(u64, Option<usize>)>,
    record: Vec<u8>,
}

/// The accounting of one branch.
pub(super) struct Ledger {
    pub(super) content: Option<Arc<PoliticsContent>>,
    pub(super) last: Option<i32>,
    pub(super) census: BTreeMap<u32, CensusRow>,
    pub(super) census_complete: bool,
    /// Each slot's last valid tag.
    pub(super) known: FastMap<u32, u32>,
    pub(super) tags: Vec<Box<str>>,
    pub(super) numbers: FastMap<Box<str>, u32>,
    pub(super) countries: Vec<Country>,
    pub(super) slot_tags: FastMap<u32, u32>,
    pub(super) refused: BTreeMap<u32, String>,
    pub(super) links: FastMap<u32, u32>,
    pub(super) raw_slots: FastMap<u32, Vec<u32>>,
    pub(super) witnesses: FastMap<u32, PoliticsCaptureCoverageRow>,
    pub(super) extended: Arc<[u32]>,
    pub(super) extended_through: i32,
    /// The raw countries whose capture moved on at the last hour, ascending.
    pub(super) advanced: Vec<u32>,
    /// The extended list and the captures that followed, in the order handed, `advanced` was
    /// made of: an hour handing both again leaves it as it was.
    advanced_from: Option<(Arc<[u32]>, Vec<u32>)>,
    /// Tags whose last account read inputs of its hour alone: accounted again at the next.
    pub(super) volatile: Tags,
    pub(super) identities: BTreeMap<(u32, u64), Vec<(i32, u32)>>,
    pub(super) complete: u16,
    pub(super) raws: FastMap<u32, Raw>,
    /// Raw countries whose cost table lost a row since costs were last matched to records.
    pub(super) stale_costs: FastSet<u32>,
    /// Each tag's recorded days, counted.
    pub(super) day_counts: Vec<u32>,
    pub(super) command: FastMap<u32, i64>,
    pub(super) settled: Option<i32>,
    /// The frontier the rollups were last settled at.
    pub(super) rolled: Option<i32>,
    /// The tags whose rollups settle at each frontier ([`Country::due`]).
    pub(super) due: BTreeSet<(i32, u32)>,
    pub(super) warnings: BTreeMap<String, u64>,
    pub(super) warnings_changed: bool,
    /// Each currency's categories by name, with their part dimension.
    pub(super) categories: [FastMap<Box<str>, u64>; 2],
    pub(super) category_currency: FastMap<u64, usize>,
    pub(super) tomorrow: BTreeMap<i32, BTreeSet<u32>>,
    scratch: Scratch,
}

impl Ledger {
    pub(super) fn new(content: Option<Arc<PoliticsContent>>) -> Self {
        Self {
            content,
            last: None,
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
            identities: BTreeMap::new(),
            complete: 0,
            raws: FastMap::default(),
            stale_costs: FastSet::default(),
            day_counts: Vec::new(),
            command: FastMap::default(),
            settled: None,
            rolled: None,
            due: BTreeSet::new(),
            warnings: BTreeMap::new(),
            warnings_changed: false,
            categories: [FastMap::default(), FastMap::default()],
            category_currency: FastMap::default(),
            tomorrow: BTreeMap::new(),
            scratch: Scratch::default(),
        }
    }

    /// The tag spelled `spelling`, numbered when new.
    pub(super) fn tag(&mut self, out: &mut impl Out, spelling: &str, hour: i32) -> io::Result<u32> {
        if let Some(&tag) = self.numbers.get(spelling) {
            return Ok(tag);
        }
        let tag = u32::try_from(self.tags.len()).map_err(|_| invalid("tags exhausted"))?;
        out.tagged(tag, spelling, hour)?;
        self.tags.push(spelling.into());
        self.numbers.insert(spelling.into(), tag);
        self.countries.push(Country::default());
        self.day_counts.push(0);
        Ok(tag)
    }

    /// The tag of capture identity (`country`, `epoch`) at `hour`: the latest begun by then,
    /// else its first.
    pub(super) fn tag_at(&self, country: u32, epoch: u64, hour: i32) -> Option<u32> {
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
    pub(super) fn raw_tag(&self, raw: u32) -> Option<u32> {
        self.links
            .get(&raw)
            .and_then(|slot| self.slot_tags.get(slot))
            .copied()
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

    /// Writes the branch's warning `reason`, `count` more times.
    fn warn(&mut self, reason: String, count: u64) {
        *self.warnings.entry(reason).or_default() += count;
        self.warnings_changed = true;
    }

    /// An hour not following `last`, the last kept: every country's open pieces end.
    pub(super) fn gap(&mut self, last: i32) {
        for country in &mut self.countries {
            country.materialize(last);
            country.rated = None;
            country.coverage = None;
            country.spending = None;
            country.cover = None;
            country.identity = None;
            country.holds.last = None;
        }
    }

    /// Opens `hour`: the tags it accounts before its own rows dirty any.
    pub(super) fn open(&mut self, hour: &Hour) -> Joined {
        let at = hour.hour;
        let previous = self.last;
        let adjacent = previous.and_then(|last| last.checked_add(1)) == Some(at);
        self.last = Some(at);
        let mut joined = Joined {
            previous,
            adjacent,
            ..Joined::default()
        };
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
        joined
    }

    /// The hour's census, links, captures and the tags they name; whether its census changed.
    pub(super) fn join(
        &mut self,
        out: &mut impl Out,
        hour: &Hour,
        joined: &mut Joined,
    ) -> io::Result<bool> {
        let at = hour.hour;
        let census_complete = hour.complete & facts::bit(TableId::Countries) != 0;
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
            self.apply_census(out, &census, at, joined)?;
        }
        let links_changed = hour.links.is_some();
        if let Some(links) = hour.links.as_deref() {
            self.apply_links(links, joined);
        }
        let touched = self.apply_captures(&hour.captures, at, hour.whole, joined);
        let raws: Vec<u32> = if census_changed || links_changed {
            self.witnesses.keys().copied().collect()
        } else {
            touched
        };
        joined.tagged = self.tag_identities(raws, at);
        Ok(census_changed)
    }

    /// The census of the hour, every row by id (none while it is not complete): each slot's
    /// last row, its tag valid or refused.
    fn apply_census(
        &mut self,
        out: &mut impl Out,
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
                    let tag = self.tag(out, &spelling, hour)?;
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

    /// The hour's native rows: each table's held rows changed, and the rows recorded.
    pub(super) fn rows(
        &mut self,
        out: &mut impl Out,
        hour: &Hour,
        joined: &mut Joined,
    ) -> io::Result<()> {
        let at = hour.hour;
        let [power_deleted, government_deleted, daily_deleted, cost_deleted] = &hour.deleted;
        let mut newest = None::<i32>;
        if hour.whole {
            // A table handed whole replaces what was held: rows it no longer holds are gone.
            let mut current: FastMap<u32, FastSet<(u64, u64)>> = FastMap::default();
            for power in &hour.power {
                let row = &power.row;
                current
                    .entry(row.country)
                    .or_default()
                    .insert((row.epoch, row.sequence));
            }
            for government in &hour.governments {
                let row = &government.row;
                current
                    .entry(row.country)
                    .or_default()
                    .insert((row.epoch, row.sequence));
            }
            for (raw, held) in &mut self.raws {
                match current.get(raw) {
                    Some(current) => held.recorded.retain(|key| current.contains(key)),
                    None => held.recorded.clear(),
                }
            }
            // What the replacement leaves out changes the accounts of the tags holding it.
            let mut raws: Vec<u32> = self
                .raws
                .iter()
                .filter(|(_, held)| held.holds_rows())
                .map(|(&raw, _)| raw)
                .collect();
            raws.sort_unstable();
            for raw in raws {
                self.dirty_days(raw, joined);
            }
            for held in self.raws.values_mut() {
                held.clear_tables();
            }
            self.stale_costs.clear();
        }
        // A held power row an account reads changing re-accounts its raw country's current tag,
        // whether or not the row is recorded again.
        for &key in power_deleted {
            let raw = (key >> 64) as u32;
            let tag = self.raw_tag(raw);
            if let Some(held) = self.raws.get_mut(&raw) {
                if let Some(row) = held.power.remove(&key) {
                    held.index_power(key, row, false);
                    held.recorded.remove(&(row.epoch, row.sequence));
                    joined.dirty.extend(tag);
                }
                held.waiting.remove(&(0, key));
            }
        }
        for power in &hour.power {
            let row = &power.row;
            newest = newest.max(power_day(row));
            let key = row.key();
            let held = HeldPower::of(row);
            let tag = self.raw_tag(row.country);
            let raw = self.raws.entry(row.country).or_default();
            if let Some(old) = raw.power.insert(key, held) {
                raw.index_power(key, old, false);
            }
            raw.index_power(key, held, true);
            joined.dirty.extend(tag);
        }
        for &key in government_deleted {
            let raw = (key >> 64) as u32;
            if let Some(held) = self.raws.get_mut(&raw) {
                if let Some(identity) = held.governments.remove(&key) {
                    held.recorded.remove(&identity);
                }
                held.waiting.remove(&(2, key));
            }
        }
        for government in &hour.governments {
            let row = &government.row;
            newest = newest.max(Some(row.hour.div_euclid(24) * 24));
            self.raws
                .entry(row.country)
                .or_default()
                .governments
                .insert(row.key(), (row.epoch, row.sequence));
        }
        // A raw country's daily row is its current tag's reconciliation, and decides whether the
        // days it holds under any tag count as that tag's income.
        for &key in daily_deleted {
            let raw = key as u32;
            if let Some(held) = self.raws.get_mut(&raw) {
                held.daily = None;
            }
            self.dirty_days(raw, joined);
        }
        for row in &hour.daily {
            newest = newest.max(Some(row.day_hour));
            self.raws.entry(row.country).or_default().daily = Some(row.clone());
            self.dirty_days(row.country, joined);
        }
        for &key in cost_deleted {
            let raw = (key >> 32) as u32;
            let tag = self.raw_tag(raw);
            if let Some(held) = self.raws.get_mut(&raw) {
                if let Some(row) = held.costs.remove(&key) {
                    held.unsum(row);
                    joined.dirty.extend(tag);
                    self.stale_costs.insert(raw);
                }
                held.waiting.remove(&(1, key));
            }
        }
        let mut costed = Vec::new();
        for cost in &hour.costs {
            joined.dirty.extend(self.raw_tag(cost.row.country));
            let held = HeldCost::of(&cost.row);
            let raw = self.raws.entry(cost.row.country).or_default();
            if let Some(old) = raw.costs.insert(held.key(), held) {
                raw.unsum(old);
            }
            raw.sum(held);
            costed.push(cost.row.country);
        }
        if hour.whole || !hour.costs.is_empty() {
            // A cost key a complete publication removed cannot return.
            let stale: Vec<u32> = if hour.whole {
                self.raws.keys().copied().collect()
            } else {
                costed.extend(self.stale_costs.drain());
                costed
            };
            for country in stale {
                let Some(raw) = self.raws.get_mut(&country) else {
                    continue;
                };
                let Raw {
                    costs,
                    recorded_costs,
                    ..
                } = raw;
                recorded_costs.retain(|&(epoch, day_hour, ordinal), _| {
                    costs
                        .get(&HeldCost::slot(country, ordinal))
                        .is_some_and(|row| row.epoch == epoch && row.day_hour == day_hour)
                });
            }
        }
        // Totals the table does not hold whole this hour are unknown; a whole read replaces them.
        let command_known = self.complete & facts::bit(TableId::CountryCommandPower) != 0;
        if !command_known || hour.command_whole {
            for slot in std::mem::take(&mut self.command).into_keys() {
                joined.dirty.extend(self.slot_tags.get(&slot).copied());
            }
        }
        for &(slot, value) in &hour.command {
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
        let _cpu = ::replay_frame::cpu::scope("politics_ledger.rows.record", None);
        if !joined.tagged.is_empty() {
            let tagged: FastSet<(u32, u64)> = joined.tagged.iter().copied().collect();
            let mut raws: Vec<u32> = joined.tagged.iter().map(|&(raw, _)| raw).collect();
            raws.sort_unstable();
            raws.dedup();
            let mut released: Vec<((u8, u128), Waiting)> = Vec::new();
            for raw in raws {
                let Some(held) = self.raws.get_mut(&raw) else {
                    continue;
                };
                let keys: Vec<(u8, u128)> = held
                    .waiting
                    .iter()
                    .filter(|(_, row)| tagged.contains(&row.identity()))
                    .map(|(key, _)| *key)
                    .collect();
                for key in keys {
                    if let Some(row) = held.waiting.remove(&key) {
                        released.push((key, row));
                    }
                }
            }
            released.sort_unstable_by_key(|(key, _)| *key);
            let previous = joined.previous;
            for (_, mut row) in released {
                // Facts are the event hour's or the read before's; any other row's are not.
                let frameless = |date: i32| date != at && Some(date) != previous;
                match &mut row {
                    Waiting::Power(power) if frameless(power.row.hour) => power.undate(),
                    Waiting::Cost(cost) if frameless(cost.row.day_hour) => cost.undate(),
                    _ => {}
                }
                match &row {
                    Waiting::Power(power) => self.record_power(out, power, at, joined)?,
                    Waiting::Government(government) => {
                        self.record_government(out, government, at, joined)?
                    }
                    Waiting::Cost(cost) => self.record_cost(out, cost, at, &mut None, joined)?,
                }
            }
        }
        for power in &hour.power {
            self.record_power(out, power, at, joined)?;
        }
        for government in &hour.governments {
            self.record_government(out, government, at, joined)?;
        }
        drop(_cpu);
        let _cpu = ::replay_frame::cpu::scope("politics_ledger.rows.daily", None);
        for row in &hour.daily {
            self.record_daily(out, row, at, joined)?;
        }
        drop(_cpu);
        // The hour's rows are all in place: a country's costs of one day read alike.
        let _cpu = ::replay_frame::cpu::scope("politics_ledger.rows.costs", None);
        let mut run = None;
        for cost in &hour.costs {
            self.record_cost(out, cost, at, &mut run, joined)?;
        }
        Ok(())
    }

    /// Dirties raw country `raw`'s current tag and every tag holding a day of it.
    fn dirty_days(&self, raw: u32, joined: &mut Joined) {
        joined.dirty.extend(self.raw_tag(raw));
        if let Some(held) = self.raws.get(&raw) {
            joined.dirty.extend(held.days.values().map(|day| day.tag));
        }
    }

    /// Whether a contribution dated `hour` reaches a native day already settled: a reply shows
    /// it on its own, beside its month.
    fn late(&self, hour: i32) -> bool {
        self.settled
            .is_some_and(|settled| hour.div_euclid(24) * 24 < settled)
    }

    /// `tag`'s settle frontier is due at `due` too.
    fn schedule(&mut self, tag: u32, due: i32) {
        let country = &mut self.countries[tag as usize];
        if country.due.is_some_and(|held| held <= due) {
            return;
        }
        if let Some(held) = country.due.replace(due) {
            self.due.remove(&(held, tag));
        }
        self.due.insert((due, tag));
    }

    /// `tag`'s settle frontier, as its rollups hold now.
    pub(super) fn reschedule(&mut self, tag: u32) {
        let country = &mut self.countries[tag as usize];
        if let Some(held) = country.due.take() {
            self.due.remove(&(held, tag));
        }
        if let Some(due) = country.rollups.due() {
            country.due = Some(due);
            self.due.insert((due, tag));
        }
    }

    /// The frontier a payment dated `hour` in its period, recurring or not, settles at.
    fn payment_due(recurring: bool, hour: i32) -> i32 {
        let day = hour.div_euclid(24);
        if recurring {
            return (day * 24).saturating_add(1);
        }
        let (index, _, _) = tally::period(0, day);
        (index + 1)
            .saturating_mul(tally::PERIOD_DAYS[0])
            .saturating_mul(24)
    }

    /// `value` as a record.
    fn encoded<T: Serialize>(&mut self, value: &T) -> io::Result<()> {
        self.scratch.record.clear();
        codec::encode_into(&mut self.scratch.record, value)
    }

    fn record_power(
        &mut self,
        out: &mut impl Out,
        power: &Power,
        at: i32,
        joined: &mut Joined,
    ) -> io::Result<()> {
        let row = &power.row;
        let identity = (row.epoch, row.sequence);
        if self
            .raws
            .get(&row.country)
            .is_some_and(|held| held.recorded.contains(&identity))
        {
            return Ok(());
        }
        if row.cause == PowerCause::DailyGain
            && row.currency == PowerCurrency::Political
            && !self
                .raws
                .get(&row.country)
                .is_some_and(|held| held.applied(row.epoch, row.hour, row.sequence))
        {
            joined.unmatched.insert(row.country);
        }
        let Some(tag) = self.tag_at(row.country, row.epoch, row.hour) else {
            self.raws
                .entry(row.country)
                .or_default()
                .waiting
                .insert((0, row.key()), Waiting::Power(power.clone()));
            return Ok(());
        };
        self.raws
            .entry(row.country)
            .or_default()
            .recorded
            .insert(identity);
        let bit = match row.currency {
            PowerCurrency::Political => PoliticsCaptureCoverageRow::PP,
            PowerCurrency::Command => PoliticsCaptureCoverageRow::CP,
        };
        let covered = self.covered(row.country, row.epoch, row.hour, bit, at);
        let measure = POWER[usize::from(row.currency == PowerCurrency::Command)];
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
        let resolved = match &power.resolved {
            Some(resolved) => resolved.clone(),
            None => power.resolve(),
        };
        let (ordinal, new) = self.item(tag, resolved.item);
        let shown = if new {
            let first = rows::mutation_id(row.country, row.epoch, row.sequence);
            Some(self.show(
                first,
                &rows::paid(row),
                (&row.identity, row.cause, None),
                power.dating.dated.as_deref(),
            )?)
        } else {
            None
        };
        out.item(
            tag,
            ordinal,
            new,
            shown.as_deref(),
            (row.hour, Some(at)),
            POWER_KIND,
        )?;
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
        self.encoded(&record)?;
        let fields = [Field::Record(&self.scratch.record), Field::Item(ordinal)];
        let sequence = out.event(tag, measure, (row.hour, Some(at)), POWER_KIND, &fields)?;
        if late && class != SKIPPED {
            out.event(tag, LATE, (row.hour, Some(at)), POWER_KIND, &fields)?;
        } else if class == BALANCE {
            self.countries[tag as usize].rollups.count(
                Some((ordinal, row.country, row.epoch)),
                row.hour,
                false,
            );
            self.schedule(tag, Self::payment_due(false, row.hour));
        } else if class == PAYMENT {
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
            self.schedule(tag, Self::payment_due(recurring, row.hour));
        }
        joined.dirty.insert(tag);
        if let (Some(charge), Some(category)) = (charge, power.dating.category.as_ref())
            && &**category != "dailyDeficit"
        {
            joined.payments.entry(tag).or_default().push(Paid {
                currency: row.currency,
                category: Arc::clone(category),
                hour: row.hour,
                charge,
            });
        }
        if row.currency == PowerCurrency::Command {
            joined
                .command
                .entry((row.country, row.epoch))
                .or_default()
                .push((row.clone(), covered, resolved.holder));
        }
        Ok(())
    }

    /// The record of an item first paid by `first` (`row`, of `cause` and `component`), as
    /// replies show it, named from the facts its hour recorded.
    fn show(
        &self,
        first: String,
        row: &PaidRow,
        (identity, cause, component): (&PowerIdentity, PowerCause, Option<PowerDailyCostKind>),
        dated: Option<&facts::DatedFacts>,
    ) -> io::Result<Vec<u8>> {
        let dated = dated
            .map(|dated| codec::decode::<items::Dated>(&dated.bytes))
            .transpose()?;
        let shown = present::show(
            first,
            row,
            identity,
            cause,
            dated.as_ref(),
            component,
            self.content.as_deref(),
        );
        codec::encode(&shown)
    }

    fn record_government(
        &mut self,
        out: &mut impl Out,
        government: &Government,
        at: i32,
        joined: &mut Joined,
    ) -> io::Result<()> {
        let row = &government.row;
        let identity = (row.epoch, row.sequence);
        if self
            .raws
            .get(&row.country)
            .is_some_and(|held| held.recorded.contains(&identity))
        {
            return Ok(());
        }
        let Some(tag) = self.tag_at(row.country, row.epoch, row.hour) else {
            self.raws
                .entry(row.country)
                .or_default()
                .waiting
                .insert((2, row.key()), Waiting::Government(government.clone()));
            return Ok(());
        };
        self.raws
            .entry(row.country)
            .or_default()
            .recorded
            .insert(identity);
        let covered = self.covered(
            row.country,
            row.epoch,
            row.hour,
            PoliticsCaptureCoverageRow::GOVERNMENT,
            at,
        );
        let shown = rows::government(row, at, government.tokens.clone(), covered);
        self.encoded(&shown)?;
        let fields = [Field::Record(&self.scratch.record)];
        out.event(tag, GOVERNMENTS, (row.hour, Some(at)), GOVERNMENT_KIND, &fields)?;
        joined.dirty.insert(tag);
        Ok(())
    }

    fn record_daily(
        &mut self,
        out: &mut impl Out,
        row: &CountryPowerDailyRow,
        at: i32,
        joined: &mut Joined,
    ) -> io::Result<()> {
        let Some(tag) = self.tag_at(row.country, row.epoch, row.day_hour) else {
            return Ok(());
        };
        let key = (row.epoch, row.day_hour);
        let (pp, cp) = rows::daily_parts(row);
        let mut reads = (pp.then_some(at), cp.then_some(at));
        if let Some(old) = self.raws.get(&row.country).and_then(|held| held.days.get(&key)) {
            if old.row == *row {
                return Ok(());
            }
            // An application commits its matching political operands together,
            // independently of command power.
            if rows::daily_political(row) == rows::daily_political(&old.row) {
                reads.0 = old.reads.0;
            }
            if rows::daily_command(row) == rows::daily_command(&old.row) {
                reads.1 = old.reads.1;
            }
        }
        let record = DailyRecord {
            epoch: row.epoch,
            daily: rows::daily(row, at, reads),
        };
        let late = self.late(row.day_hour);
        self.encoded(&record)?;
        {
            let fields = [Field::Record(&self.scratch.record)];
            // A revision of a settled day is also kept apart: no rollup counts it.
            if late {
                out.event(tag, LATE, (row.day_hour, Some(at)), DAILY_KIND, &fields)?;
            }
            out.event(tag, DAILY, (row.day_hour, Some(at)), DAILY_KIND, &fields)?;
        }
        if !late {
            let gains = [
                rows::native_gain(row, reads, false),
                rows::native_gain(row, reads, true),
            ];
            self.countries[tag as usize]
                .rollups
                .add_daily((row.day_hour, row.country, row.epoch), gains);
            self.schedule(tag, row.day_hour.saturating_add(1));
        }
        // The daily table holds one day per country: its earlier days are of no further hour.
        let held = self.raws.entry(row.country).or_default();
        let earlier: Vec<(u64, i32)> = held
            .days
            .keys()
            .copied()
            .filter(|key| key.1 < row.day_hour)
            .collect();
        let mut gone = Vec::new();
        for key in earlier {
            if let Some(day) = held.days.remove(&key) {
                gone.push(day.tag);
            }
        }
        if let Some(old) = held.days.insert(
            key,
            Day {
                row: row.clone(),
                reads,
                tag,
            },
        ) {
            gone.push(old.tag);
        }
        for old in gone {
            joined.dirty.insert(old);
            if let Some(count) = self.day_counts.get_mut(old as usize) {
                *count = count.saturating_sub(1);
            }
        }
        if let Some(count) = self.day_counts.get_mut(tag as usize) {
            *count += 1;
        }
        joined.dirty.insert(tag);
        Ok(())
    }

    /// What every cost of `row`'s country, day and application reads alike at hour `at`.
    fn cost_day(&self, row: &CountryPowerDailyCostRow, at: i32) -> CostDay {
        let held = self.raws.get(&row.country);
        let day = held.and_then(|held| held.days.get(&(row.epoch, row.day_hour)));
        CostDay {
            of: (row.country, row.epoch, row.day_hour, row.pp_sequence),
            tag: self.tag_at(row.country, row.epoch, row.day_hour),
            covered: self.covered(
                row.country,
                row.epoch,
                row.day_hour,
                PoliticsCaptureCoverageRow::PP | PoliticsCaptureCoverageRow::PP_DAILY_COSTS,
                at,
            ) && !held.is_some_and(Raw::unreconciled),
            late: self.late(row.day_hour),
            matched: held.is_some_and(|held| held.applied(row.epoch, row.day_hour, row.pp_sequence))
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
        out: &mut impl Out,
        cost: &Cost,
        at: i32,
        run: &mut Option<CostDay>,
        joined: &mut Joined,
    ) -> io::Result<()> {
        let row = &cost.row;
        let key = (row.epoch, row.day_hour, row.ordinal);
        let of = (row.country, row.epoch, row.day_hour, row.pp_sequence);
        let day = match run.filter(|day| day.of == of) {
            Some(day) => day,
            None => *run.insert(self.cost_day(row, at)),
        };
        let Some(tag) = day.tag else {
            self.raws
                .entry(row.country)
                .or_default()
                .waiting
                .insert((1, row.key()), Waiting::Cost(cost.clone()));
            return Ok(());
        };
        let held = self
            .raws
            .get(&row.country)
            .and_then(|held| held.recorded_costs.get(&key));
        let (covered, revision, held_item) = match held {
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
        let ordinal = match held_item {
            Some(ordinal) => ordinal,
            None => {
                let item = cost.item.unwrap_or_else(|| cost.resolve());
                let (ordinal, new) = self.item(tag, item);
                let shown = if new {
                    let first = rows::cost_id(row.country, row.epoch, row.day_hour, row.ordinal);
                    Some(self.show(
                        first,
                        &rows::paid_cost(row),
                        (&row.identity, row.cause, Some(row.component)),
                        cost.dating.dated.as_deref(),
                    )?)
                } else {
                    None
                };
                out.item(
                    tag,
                    ordinal,
                    new,
                    shown.as_deref(),
                    (row.day_hour, Some(at)),
                    COST_KIND,
                )?;
                ordinal
            }
        };
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
            self.encoded(&record)?;
            let fields = [Field::Record(&self.scratch.record), Field::Item(ordinal)];
            let sequence = out.event(tag, COSTS, (row.day_hour, Some(at)), COST_KIND, &fields)?;
            let field = (ordinal, row.country, row.epoch);
            if late {
                out.event(tag, LATE, (row.day_hour, Some(at)), COST_KIND, &fields)?;
            } else if class == UNMATCHED || class == CREDIT {
                let field = (class == CREDIT).then_some(field);
                self.countries[tag as usize]
                    .rollups
                    .count(field, row.day_hour, true);
                self.schedule(tag, Self::payment_due(false, row.day_hour));
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
                self.schedule(tag, Self::payment_due(recurring, row.day_hour));
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
                && let Some(category) = cost.dating.category.as_ref()
            {
                joined.payments.entry(tag).or_default().push(Paid {
                    currency: PowerCurrency::Political,
                    category: Arc::clone(category),
                    hour: row.day_hour,
                    charge: Some(i128::from(row.cost_raw)),
                });
            }
        }
        self.raws.entry(row.country).or_default().recorded_costs.insert(
            key,
            Recorded {
                row: row.clone(),
                covered,
                item: ordinal,
            },
        );
        joined.dirty.insert(tag);
        Ok(())
    }

    /// The currencies an addition without an evaluated operand leaves unknown, among the
    /// hour's power rows of `witness`'s capture.
    fn unknown_additions(&self, witness: &PoliticsCaptureCoverageRow) -> u8 {
        if witness.since_hour > witness.through_hour {
            return 0;
        }
        let Some(held) = self.raws.get(&witness.country) else {
            return 0;
        };
        let epoch = witness.epoch;
        held.unknown_adds
            .range(
                (epoch, witness.since_hour, 0, 0)..=(epoch, witness.through_hour, u8::MAX, u128::MAX),
            )
            .fold(0, |bits, &(.., bit, _)| bits | bit)
    }

    /// Whether `raw`'s hour holds a political daily gain within `since..=hour`.
    fn has_daily(&self, raw: u32, since: i32, hour: i32) -> bool {
        since <= hour
            && self.raws.get(&raw).is_some_and(|held| {
                held.daily_gains
                    .range((since, 0)..=(hour, u128::MAX))
                    .next()
                    .is_some()
            })
    }

    /// One tag's accounting at `hour`: its coverage, spending coverage, holds, income, parts
    /// and warning rates, written where they change.
    pub(super) fn account(
        &mut self,
        out: &mut impl Out,
        tag: u32,
        hour: i32,
        joined: &Joined,
    ) -> io::Result<()> {
        let adjacent = joined.adjacent;
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
            let steady = self.raws.get(&raw).is_some_and(Raw::unreconciled);
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
                Some("Spending events were not recorded at this read")
            } else if !daily_known
                || unreconciled
                || has_daily && value.streams & PoliticsCaptureCoverageRow::PP_DAILY_COSTS == 0
            {
                Some("Daily political power costs were not recorded at this read")
            } else if unknown != 0 {
                Some(match unknown {
                    PoliticsCaptureCoverageRow::PP => {
                        "A political power adjustment has no recorded evaluated operand"
                    }
                    PoliticsCaptureCoverageRow::CP => {
                        "A command power adjustment has no recorded evaluated operand"
                    }
                    _ => {
                        "Political and command power adjustments have no recorded evaluated operands"
                    }
                })
            } else {
                value.reason.map(Gap::text)
            };
            spending = Some(Spending {
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
                    spending.reason =
                        Some("Daily political power costs have no matched payment at this read");
                } else if amount < 0 {
                    spending.political = false;
                    spending.reason = Some(
                        "Negative daily political power cost components are not included in spending",
                    );
                }
            }
        }
        let identity = coverage
            .filter(|row| row.through_hour == Some(hour))
            .and_then(|row| row.identity());
        let starts = !adjacent || identity != self.countries[tag as usize].identity;
        self.countries[tag as usize].coverage = coverage;
        self.write_spending(out, tag, hour, adjacent, starts, spending)?;
        self.write_holds(out, tag, hour, adjacent, active, joined)?;
        self.write_parts(out, tag, hour, joined.previous, starts, identity, joined)?;
        self.countries[tag as usize].identity = identity;
        Ok(())
    }

    /// Whether `tag`'s spending is recorded at `hour` in each currency, and why not.
    fn write_spending(
        &mut self,
        out: &mut impl Out,
        tag: u32,
        hour: i32,
        adjacent: bool,
        starts: bool,
        value: Option<Spending>,
    ) -> io::Result<()> {
        let Some(value) = value else {
            for measure in COVERED {
                out.value(tag, measure, None, hour, None, false)?;
            }
            self.countries[tag as usize].spending = None;
            return Ok(());
        };
        let merged = match self.countries[tag as usize].spending.take() {
            Some(mut old)
                if adjacent
                    && old.political == value.political
                    && old.command == value.command
                    && old.reason.as_deref() == value.reason =>
            {
                old.through_hour = Some(hour);
                old
            }
            _ => LedgerSpendCoverage {
                read_hour: hour,
                since_hour: Some(hour),
                through_hour: Some(hour),
                political: value.political,
                command: value.command,
                reason: value.reason.map(str::to_owned),
            },
        };
        for (measure, known) in COVERED.into_iter().zip([merged.political, merged.command]) {
            out.value(tag, measure, None, hour, known.then_some(1), starts)?;
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
            self.encoded(&merged)?;
            let fields = [Field::Record(&self.scratch.record)];
            out.event(tag, COVER_REASONS, (hour, None), COVER_KIND, &fields)?;
            self.countries[tag as usize].cover = Some(cover);
        }
        self.countries[tag as usize].spending = Some(merged);
        Ok(())
    }

    /// `tag`'s command power holds at `hour`: the allocators carried on, the hour's command rows
    /// applied, and a snapshot of the hour's total, written when it differs.
    fn write_holds(
        &mut self,
        out: &mut impl Out,
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
        let shown = present::hold(
            hour,
            snapshot.total,
            &snapshot.holders,
            snapshot.missing.as_deref(),
            self.content.as_deref(),
        );
        self.encoded(&shown)?;
        let fields = [Field::Record(&self.scratch.record)];
        out.event(tag, HOLDS, (hour, None), HOLD_KIND, &fields)?;
        Ok(())
    }

    /// The part dimension of `category` in `currency`, numbered and named when new.
    pub(super) fn category(
        &mut self,
        out: &mut impl Out,
        currency: usize,
        category: &str,
    ) -> io::Result<u64> {
        if let Some(&dimension) = self.categories[currency].get(category) {
            return Ok(dimension);
        }
        let held = self.categories.iter().map(FastMap::len).sum::<usize>();
        let dimension = CATEGORY_PARTS + held as u64;
        out.named(dimension, currency, category)?;
        self.categories[currency].insert(category.into(), dimension);
        self.category_currency.insert(dimension, currency);
        Ok(dimension)
    }

    /// `tag`'s spending parts at `hour`: its capture identity's income of the day and its spending
    /// by category, each currency's while its spending was recorded since the day began.
    #[allow(clippy::too_many_arguments)]
    fn write_parts(
        &mut self,
        out: &mut impl Out,
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
            let currency = usize::from(payment.currency == PowerCurrency::Command);
            let dimension = self.category(out, currency, &payment.category)?;
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
                let held = self
                    .raws
                    .get(&country)
                    .into_iter()
                    .flat_map(|held| held.days.range((0, day)..=(u64::MAX, day)))
                    .map(|(_, held)| held)
                    .filter(|held| held.tag == tag && held.row.day_hour == day);
                // One capture's later read revises its day; only separate captures are ambiguous.
                let supplied = |command: bool| {
                    let mut found = held.clone().filter(|held| {
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
            out.value(tag, PARTS, Some(dimension as u64), hour, value, starts)?;
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
            out.value(tag, PARTS, Some(ZERO[index]), hour, shown.then_some(0), starts)?;
            for dimension in mine() {
                let value = shown.then(|| value(dimension)).flatten();
                out.value(tag, PARTS, Some(dimension), hour, value, starts)?;
            }
            out.value(tag, PARTS, Some(TOTAL[index]), hour, total, starts)?;
        }
        self.scratch.values = values;
        self.scratch.categories = categories;
        Ok(())
    }

    /// Writes the periods the settled frontier closed since it last moved, of each tag whose
    /// rollups it reached.
    pub(super) fn settle(&mut self, out: &mut impl Out, at: i32) -> io::Result<()> {
        let Some(settled) = self.settled.filter(|settled| self.rolled != Some(*settled)) else {
            return Ok(());
        };
        self.rolled = Some(settled);
        while let Some(&(due, tag)) = self.due.first() {
            if due > settled {
                break;
            }
            self.due.pop_first();
            let country = &mut self.countries[tag as usize];
            country.due = None;
            let closed = country.rollups.settle(settled)?;
            self.reschedule(tag);
            out.settled(tag, at, closed)?;
        }
        Ok(())
    }
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

/// The category name a dimension of `categories` spells.
pub(super) fn spelling(currency: usize, category: &str) -> String {
    format!("{}:{category}", CURRENCIES[currency])
}
