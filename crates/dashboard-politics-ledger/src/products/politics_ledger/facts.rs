//! The ledger's facts, projected hour by hour from a columnar walk ([`replay::segments`]): the
//! native power, government, daily and cost rows the hour inserted or rewrote, typed from their
//! columns; the captures it changed, with those it only extended to the hour as one list; the
//! command power allocations it changed; and the census and links from the walk's world
//! ([`dashboard_world`]). A segment's first hour, and any after a skipped hour, hands every row.
//!
//! What only an hour's tables say of a row (its item's operands, names and scripted text,
//! [`items`]) is read for the rows dated at the hour, from the walk's views at that hour alone,
//! scripted text and trigger conditions with the walk's lexer folds ([`items::TextFrame`]).
//! Rows a capture may publish a read late are dated
//! the hour before: after a read whose capture interval starts at the hour, the kit is copied
//! for the next hour, and copied again only where a following such hour moved it.

use super::items::{self, Dated, FrameTexts, Item, NamedCountry, TextFrame};
use super::*;
use dashboard_base::products::idea_format;
use dashboard_base::products::politics_content::PoliticsContent;
use dashboard_world::{Copies, Hold, Needs, Roster, World};
use replay::{
    columns,
    segments::{self, Operator, Sink, State},
};
use replay_data::format::FormatVersion;
use replay_frame::frame::Tables;
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    sync::LazyLock,
};

/// The ledger's own tables: what the accounting reads besides the world's roster.
const NATIVE: [TableId; 8] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::PoliticsCaptureCoverage,
    TableId::CountryPowerChanges,
    TableId::GovernmentChanges,
    TableId::CountryPowerDaily,
    TableId::CountryPowerDailyCosts,
    TableId::CountryCommandPower,
];

/// The tables an item's operands, names and scripted text read at its hour.
pub(super) fn kit() -> &'static [TableId] {
    static KIT: LazyLock<Vec<TableId>> = LazyLock::new(|| {
        let mut tables = vec![
            TableId::CharacterPortraits,
            TableId::AdvisorRoles,
            TableId::Characters,
            TableId::UnitLeaders,
            TableId::Countries,
            TableId::CountryLinks,
            TableId::LexicalTokens,
            TableId::ModifierDefinitions,
            TableId::CountryAppearance,
            TableId::Politics,
            TableId::PoliticalParties,
            TableId::Autonomy,
            TableId::CountryCivilWars,
            TableId::CountryCounterInputs,
            TableId::CountryWars,
            TableId::WarRelations,
            TableId::IndustrialOrganisations,
            TableId::States,
        ];
        tables.extend(installed_content_country_names::country_names::TABLES);
        tables.extend_from_slice(dashboard_text::triggers::RECORDED_TABLES);
        tables.sort_unstable();
        tables.dedup();
        tables
    });
    &KIT
}

/// Every table the family's operators read.
pub fn tables() -> &'static [TableId] {
    static TABLES: LazyLock<Vec<TableId>> = LazyLock::new(|| {
        let mut tables = vec![TableId::Campaign];
        tables.extend(NATIVE);
        tables.extend_from_slice(kit());
        tables.sort_unstable();
        tables.dedup();
        tables
    });
    &TABLES
}

/// What the family reads of its walk's world: the countries, and views of its item tables.
pub fn needs() -> Needs {
    Needs {
        countries: true,
        lexicon: true,
        frame: kit().iter().map(|&table| (table, Hold::Whole)).collect(),
        ..Needs::default()
    }
}

/// A table's completeness bit in [`Hour::complete`], in [`NATIVE`] order.
pub(super) fn bit(table: TableId) -> u16 {
    NATIVE
        .iter()
        .position(|&native| native == table)
        .map_or(0, |at| 1 << at)
}

/// LexicalTokens completeness, after the native tables' bits.
pub(super) const LEXICAL: u16 = 1 << 8;

/// A Countries row's identity.
#[derive(Clone, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct CensusRow {
    pub id: u32,
    pub slot: u32,
    pub tag: Arc<[u8]>,
}

/// What the hour changed of the captures.
#[derive(Clone, Debug, Default)]
#[derive(replay_spool::Spooled)]
pub struct Captures {
    /// Rows the hour wrote other than by extending them to the hour; every row when whole.
    pub changed: Vec<PoliticsCaptureCoverageRow>,
    /// The raw countries whose capture the hour removed.
    pub deleted: Vec<u32>,
    /// The raw countries whose capture the hour only extended to the hour, ascending: the hour
    /// before's list while it stays the same.
    pub extended: Arc<[u32]>,
}

/// A power or cost row and what its hour says of its item.
#[derive(Clone, Debug, Default)]
#[derive(replay_spool::Spooled)]
pub struct Dating {
    /// Its item's facts at its hour; None where no frame of its hour was read.
    pub dated: Option<Arc<DatedFacts>>,
    /// The category its payment is spent in, for a payment.
    pub category: Option<Arc<str>>,
}

/// An item's facts at its hour, MessagePack, and their digest: one allocation for the rows
/// repeating them.
#[derive(Debug)]
#[derive(replay_spool::Spooled)]
pub struct DatedFacts {
    pub bytes: Vec<u8>,
    pub digest: [u8; 32],
}

impl Dating {
    fn digest(&self) -> Option<&[u8; 32]> {
        self.dated.as_ref().map(|dated| &dated.digest)
    }

    /// Whether it reads as `other` does.
    fn same(&self, other: &Self) -> bool {
        let dated = match (&self.dated, &other.dated) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b) || (a.digest == b.digest && a.bytes == b.bytes),
            (None, None) => true,
            _ => false,
        };
        dated && self.category == other.category
    }
}

/// A power row and what its hour says of its item. The spool writes its sequence, hour and
/// amounts, and the rest as a [`PowerShape`] the rows repeating it share.
#[derive(Clone, Debug)]
pub struct Power {
    pub row: CountryPowerChangeRow,
    pub dating: Dating,
    /// Its item and holder, worked out with its facts; None to work them out when recorded.
    pub resolved: Option<Resolved>,
    /// What it shares with the rows repeating it, where the projection shared it.
    pub shape: Option<Arc<PowerShape>>,
}

/// What a power row's facts make of it ([`rows::power_item`], [`rows::holder`]).
#[derive(Clone, Debug, PartialEq)]
#[derive(replay_spool::Spooled)]
pub struct Resolved {
    pub(super) item: [u8; 32],
    pub(super) holder: Option<rows::Holder>,
}

/// What a power row holds but for its sequence, hour and amounts.
#[derive(Debug)]
#[derive(replay_spool::Spooled)]
pub struct PowerShape {
    instance: u32,
    epoch: u64,
    currency: PowerCurrency,
    operation: PowerOperation,
    cause: PowerCause,
    site: PowerSite,
    identity: PowerIdentity,
    dating: Dating,
    resolved: Option<Resolved>,
}

impl PowerShape {
    fn of(power: &Power) -> Self {
        let row = &power.row;
        Self {
            instance: row.instance,
            epoch: row.epoch,
            currency: row.currency,
            operation: row.operation,
            cause: row.cause,
            site: row.site,
            identity: row.identity.clone(),
            dating: power.dating.clone(),
            resolved: power.resolved.clone(),
        }
    }

    fn fits(&self, power: &Power) -> bool {
        let row = &power.row;
        self.instance == row.instance
            && self.epoch == row.epoch
            && self.currency == row.currency
            && self.operation == row.operation
            && self.cause == row.cause
            && self.site == row.site
            && self.identity == row.identity
            && self.dating.same(&power.dating)
            && self.resolved == power.resolved
    }
}

impl replay_spool::Spooled for Power {
    fn spool(&self, out: &mut replay_spool::Writer<'_>) -> io::Result<()> {
        let row = &self.row;
        row.country.spool(out)?;
        row.sequence.spool(out)?;
        row.hour.spool(out)?;
        row.balance_before_raw.spool(out)?;
        row.balance_after_raw.spool(out)?;
        row.requested_delta_raw.spool(out)?;
        match self.shape.as_ref().filter(|shape| shape.fits(self)) {
            Some(shape) => shape.spool(out),
            None => Arc::new(PowerShape::of(self)).spool(out),
        }
    }

    fn unspool(input: &mut replay_spool::Reader<'_>) -> io::Result<Self> {
        let country = u32::unspool(input)?;
        let sequence = u64::unspool(input)?;
        let hour = i32::unspool(input)?;
        let balance_before_raw = i64::unspool(input)?;
        let balance_after_raw = i64::unspool(input)?;
        let requested_delta_raw = Option::<i64>::unspool(input)?;
        let shape = Arc::<PowerShape>::unspool(input)?;
        Ok(Self {
            row: CountryPowerChangeRow {
                country,
                instance: shape.instance,
                sequence,
                epoch: shape.epoch,
                hour,
                currency: shape.currency,
                operation: shape.operation,
                balance_before_raw,
                balance_after_raw,
                requested_delta_raw,
                cause: shape.cause,
                site: shape.site,
                identity: shape.identity.clone(),
            },
            dating: shape.dating.clone(),
            resolved: shape.resolved.clone(),
            shape: Some(shape),
        })
    }
}

impl Power {
    pub fn resolve(&self) -> Resolved {
        Resolved {
            item: rows::power_item(&self.row, self.dating.digest()),
            holder: rows::holder(
                &self.row,
                self.dating.dated.as_ref().map(|dated| &dated.bytes[..]),
            ),
        }
    }

    /// Its row as no frame of its hour dated it.
    pub fn undate(&mut self) {
        self.dating.dated = None;
        self.resolved = None;
        self.shape = None;
    }
}

/// A cost row and what its hour says of its item. The spool writes its key, day and sequence,
/// and the rest as a [`CostShape`] the rows repeating it share.
#[derive(Clone, Debug)]
pub struct Cost {
    pub row: CountryPowerDailyCostRow,
    pub dating: Dating,
    /// Its item ([`rows::cost_item`]), worked out with its facts; None to work it out when
    /// recorded.
    pub item: Option<[u8; 32]>,
    /// What it shares with the rows repeating it, where the projection shared it.
    pub shape: Option<Arc<CostShape>>,
}

/// What a cost row holds but for its key, day and sequence.
#[derive(Debug)]
#[derive(replay_spool::Spooled)]
pub struct CostShape {
    instance: u32,
    epoch: u64,
    component: PowerDailyCostKind,
    cost_raw: i64,
    cause: PowerCause,
    site: PowerSite,
    identity: PowerIdentity,
    dating: Dating,
    item: Option<[u8; 32]>,
}

impl CostShape {
    fn of(cost: &Cost) -> Self {
        let row = &cost.row;
        Self {
            instance: row.instance,
            epoch: row.epoch,
            component: row.component,
            cost_raw: row.cost_raw,
            cause: row.cause,
            site: row.site,
            identity: row.identity.clone(),
            dating: cost.dating.clone(),
            item: cost.item,
        }
    }

    fn fits(&self, cost: &Cost) -> bool {
        let row = &cost.row;
        self.instance == row.instance
            && self.epoch == row.epoch
            && self.component == row.component
            && self.cost_raw == row.cost_raw
            && self.cause == row.cause
            && self.site == row.site
            && self.identity == row.identity
            && self.dating.same(&cost.dating)
            && self.item == cost.item
    }
}

impl replay_spool::Spooled for Cost {
    fn spool(&self, out: &mut replay_spool::Writer<'_>) -> io::Result<()> {
        let row = &self.row;
        row.country.spool(out)?;
        row.ordinal.spool(out)?;
        row.day_hour.spool(out)?;
        row.pp_sequence.spool(out)?;
        match self.shape.as_ref().filter(|shape| shape.fits(self)) {
            Some(shape) => shape.spool(out),
            None => Arc::new(CostShape::of(self)).spool(out),
        }
    }

    fn unspool(input: &mut replay_spool::Reader<'_>) -> io::Result<Self> {
        let country = u32::unspool(input)?;
        let ordinal = u32::unspool(input)?;
        let day_hour = i32::unspool(input)?;
        let pp_sequence = u64::unspool(input)?;
        let shape = Arc::<CostShape>::unspool(input)?;
        Ok(Self {
            row: CountryPowerDailyCostRow {
                country,
                instance: shape.instance,
                ordinal,
                epoch: shape.epoch,
                day_hour,
                pp_sequence,
                component: shape.component,
                cost_raw: shape.cost_raw,
                cause: shape.cause,
                site: shape.site,
                identity: shape.identity.clone(),
            },
            dating: shape.dating.clone(),
            item: shape.item,
            shape: Some(shape),
        })
    }
}

impl Cost {
    pub fn resolve(&self) -> [u8; 32] {
        rows::cost_item(&self.row, self.dating.digest())
    }

    /// Its row as no frame of its hour dated it.
    pub fn undate(&mut self) {
        self.dating.dated = None;
        self.item = None;
        self.shape = None;
    }
}

/// Values met on the native day of `day` and on the day before, by key: a row repeating one
/// hands on the same allocation, which the spool writes once.
struct Recent<K, V> {
    day: Option<i32>,
    held: [HashMap<K, Arc<V>>; 2],
}

impl<K, V> Default for Recent<K, V> {
    fn default() -> Self {
        Self {
            day: None,
            held: [HashMap::new(), HashMap::new()],
        }
    }
}

impl<K: std::hash::Hash + Eq, V> Recent<K, V> {
    /// The value held for `key` that `fits`, as met at `hour`.
    fn get(&mut self, hour: i32, key: &K, fits: impl Fn(&V) -> bool) -> Option<Arc<V>> {
        let day = hour.div_euclid(24);
        if self.day != Some(day) {
            if self.day.and_then(|held| held.checked_add(1)) == Some(day) {
                self.held.swap(0, 1);
                self.held[0].clear();
            } else {
                self.held = [HashMap::new(), HashMap::new()];
            }
            self.day = Some(day);
        }
        if let Some(held) = self.held[0].get(key) {
            return fits(&**held).then(|| Arc::clone(held));
        }
        let (key, held) = self.held[1].remove_entry(key)?;
        if !fits(&*held) {
            return None;
        }
        self.held[0].insert(key, Arc::clone(&held));
        Some(held)
    }

    /// `value`, held for `key` from now.
    fn put(&mut self, key: K, value: V) -> Arc<V> {
        let value = Arc::new(value);
        self.held[0].insert(key, Arc::clone(&value));
        value
    }
}

#[derive(Clone, Debug)]
#[derive(replay_spool::Spooled)]
pub struct Government {
    pub row: GovernmentChangeRow,
    /// Its ideologies' spellings at its hour.
    pub tokens: (Option<String>, Option<String>),
}

/// One projected hour.
#[derive(Default)]
#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// Every table is handed whole: one left out holds no row.
    pub whole: bool,
    /// The native tables complete this hour ([`bit`]), and LexicalTokens ([`LEXICAL`]).
    pub complete: u16,
    /// Every Countries row by id, when the census changed or `whole`.
    pub census: Option<Arc<[CensusRow]>>,
    /// Every link by raw id, when the links changed or `whole`.
    pub links: Option<Arc<[(u32, u32)]>>,
    pub captures: Captures,
    pub power: Vec<Power>,
    pub governments: Vec<Government>,
    pub daily: Vec<CountryPowerDailyRow>,
    pub costs: Vec<Cost>,
    /// The keys the hour deleted from the power, government, daily and cost tables.
    pub deleted: [Vec<u128>; 4],
    /// Each changed command power allocation by canonical slot; None where its row left.
    pub command: Vec<(u32, Option<i64>)>,
    /// `command` holds every live allocation: one held before that it omits left.
    pub command_whole: bool,
}

/// Columns read in place.
struct Fields {
    through: usize,
    /// A capture's epoch, first hour, streams and instance.
    capture: [usize; 4],
    allocated: usize,
    literal: usize,
}

/// One segment's projection.
pub struct Projector {
    format: FormatVersion,
    content: Option<Arc<PoliticsContent>>,
    fields: Fields,
    last: Option<i32>,
    /// The native tables' completeness the hour before, in [`NATIVE`] order.
    complete: u16,
    /// The last extension list handed.
    extended: Arc<[u32]>,
    /// The kit copied at the hour `carried_at` names, after a read whose capture may publish
    /// rows a read late; cleared at the next hour that is not such a read.
    carried: Option<Copies>,
    carried_at: Option<(u64, i32)>,
    /// The recorded modifier definitions by name, as the walk's views last indexed them, while
    /// neither table moves; None inside where the views hold them in part.
    modifiers: Option<Option<Arc<BTreeMap<String, ModifierDefinitionRow>>>>,
    /// The item facts dated lately, by digest, and the shapes of the rows handed lately, by
    /// item and capture.
    facts: Recent<[u8; 32], DatedFacts>,
    powers: Recent<([u8; 32], u32, u64), PowerShape>,
    costs: Recent<([u8; 32], u32, u64), CostShape>,
    /// Every category spelling handed on, shared alike.
    categories: HashSet<Arc<str>>,
}

/// The slots `table`'s hour inserted, or wrote any of `mask` at, ascending.
fn written(state: &State, table: TableId, mask: u64) -> io::Result<Vec<u32>> {
    let changes = columns::changes(state, table)?;
    let mut slots: Vec<u32> = changes
        .changed
        .iter()
        .zip(&changes.masks)
        .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
        .filter(|&(_, &written)| written & mask != 0)
        .map(|(&slot, _)| slot)
        .chain(changes.inserted.iter().copied())
        .collect();
    slots.sort_unstable();
    slots.dedup();
    Ok(slots)
}

/// Every live slot of `table`, ascending.
fn live(state: &State, table: TableId) -> io::Result<Vec<u32>> {
    Ok(columns::table(state, table)?.live().ones().collect())
}

/// Whether a payment of this power row resolves an item a reply shows: a charge, a balance
/// line resolved by its item, or a hold naming its allocator.
fn named(row: &CountryPowerChangeRow) -> bool {
    !matches!(
        row.cause,
        PowerCause::CapCorrection | PowerCause::ResetSetup | PowerCause::Console
    ) && !(row.cause == PowerCause::DailyGain && row.currency == PowerCurrency::Political)
}

impl Projector {
    pub fn new(
        format: FormatVersion,
        content: &Result<Option<Arc<PoliticsContent>>, String>,
    ) -> io::Result<Self> {
        Ok(Self {
            format,
            content: content.as_ref().ok().cloned().flatten(),
            fields: Fields {
                through: columns::field(TableId::PoliticsCaptureCoverage, "through_hour")?,
                capture: [
                    columns::field(TableId::PoliticsCaptureCoverage, "epoch")?,
                    columns::field(TableId::PoliticsCaptureCoverage, "since_hour")?,
                    columns::field(TableId::PoliticsCaptureCoverage, "streams")?,
                    columns::field(TableId::PoliticsCaptureCoverage, "instance")?,
                ],
                allocated: columns::field(
                    TableId::CountryCommandPower,
                    "allocated_command_power_raw",
                )?,
                literal: columns::field(TableId::LexicalTokens, "literal")?,
            },
            last: None,
            complete: 0,
            extended: Arc::from(Vec::new()),
            carried: None,
            carried_at: None,
            modifiers: None,
            facts: Recent::default(),
            powers: Recent::default(),
            costs: Recent::default(),
            categories: HashSet::new(),
        })
    }

    /// Item facts dated at `hour`, sharing the allocation of equal facts dated that native day
    /// or the day before.
    fn dated(&mut self, hour: i32, bytes: Vec<u8>) -> Arc<DatedFacts> {
        let digest = *blake3::hash(&bytes).as_bytes();
        match self.facts.get(hour, &digest, |held| held.bytes == bytes) {
            Some(held) => held,
            None => self.facts.put(digest, DatedFacts { bytes, digest }),
        }
    }

    /// `text` as the category spelling every row handed on shares.
    fn category(&mut self, text: &str) -> Arc<str> {
        if let Some(held) = self.categories.get(text) {
            return Arc::clone(held);
        }
        let shared: Arc<str> = Arc::from(text);
        self.categories.insert(Arc::clone(&shared));
        shared
    }

    /// `table`'s rows the hour inserted or wrote, every live row when `whole`, and the keys it
    /// deleted; none of a table that is not complete.
    fn rows<R: RowCodec>(
        &self,
        state: &State,
        table: TableId,
        whole: bool,
    ) -> io::Result<(Vec<R>, Vec<u128>)> {
        if !columns::complete(state, table) {
            return Ok((Vec::new(), Vec::new()));
        }
        let slots = if whole {
            live(state, table)?
        } else {
            written(state, table, u64::MAX)?
        };
        let mut rows = Vec::with_capacity(slots.len());
        for slot in slots {
            rows.push(columns::row::<R>(
                state,
                table,
                slot,
                self.format.version(),
            )?);
        }
        let deleted = if whole {
            Vec::new()
        } else {
            columns::changes(state, table)?.deleted_keys.clone()
        };
        Ok((rows, deleted))
    }

    /// The capture `slot` holds, read in place where its columns are recorded and valid, else
    /// as the row decoder reads it.
    fn capture(&self, state: &State, slot: u32) -> io::Result<PoliticsCaptureCoverageRow> {
        let table = TableId::PoliticsCaptureCoverage;
        let held = columns::table(state, table)?;
        let [epoch, since, streams, instance] = self.fields.capture;
        let at = slot as usize;
        let fast = (|| {
            if ![epoch, since, self.fields.through, streams, instance]
                .iter()
                .all(|&field| held.recorded(field, slot))
            {
                return None;
            }
            let row = PoliticsCaptureCoverageRow {
                country: u32::try_from(held.key(slot)?).ok()?,
                instance: *columns::plain::<u32>(held, instance).ok()?.get(at)?,
                epoch: *columns::plain::<u64>(held, epoch).ok()?.get(at)?,
                since_hour: *columns::plain::<i32>(held, since).ok()?.get(at)?,
                through_hour: *columns::plain::<i32>(held, self.fields.through)
                    .ok()?
                    .get(at)?,
                streams: *columns::plain::<u8>(held, streams).ok()?.get(at)?,
            };
            (row.since_hour <= row.through_hour
                && row.streams & !PoliticsCaptureCoverageRow::ALL_STREAMS == 0)
                .then_some(row)
        })();
        match fast {
            Some(row) => Ok(row),
            None => columns::row(state, table, slot, self.format.version()),
        }
    }

    /// The captures the hour changed, and those it only extended to the hour.
    fn captures(&mut self, state: &State, hour: i32, whole: bool) -> io::Result<Captures> {
        let table = TableId::PoliticsCaptureCoverage;
        if !columns::complete(state, table) {
            self.extended = Arc::from(Vec::new());
            return Ok(Captures {
                extended: Arc::clone(&self.extended),
                ..Captures::default()
            });
        }
        if whole {
            let mut changed = Vec::new();
            for slot in live(state, table)? {
                changed.push(self.capture(state, slot)?);
            }
            self.extended = Arc::from(Vec::new());
            return Ok(Captures {
                changed,
                deleted: Vec::new(),
                extended: Arc::clone(&self.extended),
            });
        }
        let held = columns::table(state, table)?;
        let changes = columns::changes(state, table)?;
        let through = columns::plain::<i32>(held, self.fields.through)?;
        let only = 1_u64 << self.fields.through;
        // A slot's columns written this hour, by its runs and fills together.
        let mut masks: Vec<(u32, u64)> = changes
            .changed
            .iter()
            .copied()
            .zip(changes.masks.iter().copied())
            .chain(
                changes
                    .fill_slots
                    .iter()
                    .copied()
                    .zip(changes.fill_masks.iter().copied()),
            )
            .collect();
        masks.sort_unstable_by_key(|&(slot, _)| slot);
        let mut extended = Vec::new();
        let mut changed = Vec::new();
        let mut at = 0;
        while let Some(&(slot, _)) = masks.get(at) {
            let mut mask = 0;
            while let Some(&(_, bits)) = masks.get(at).filter(|&&(held, _)| held == slot) {
                mask |= bits;
                at += 1;
            }
            if mask == 0 {
                continue;
            }
            if mask == only
                && through.get(slot as usize) == Some(&hour)
                && changes.inserted.binary_search(&slot).is_err()
            {
                let key = held
                    .key(slot)
                    .ok_or_else(|| dashboard_core::invalid("A changed capture holds no row"))?;
                extended.push(key as u32);
            } else {
                changed.push(self.capture(state, slot)?);
            }
        }
        for &slot in &changes.inserted {
            if masks
                .binary_search_by_key(&slot, |&(slot, _)| slot)
                .is_err()
            {
                changed.push(self.capture(state, slot)?);
            }
        }
        extended.sort_unstable();
        if *self.extended != *extended {
            self.extended = Arc::from(extended);
        }
        Ok(Captures {
            changed,
            deleted: changes.deleted_keys.iter().map(|&key| key as u32).collect(),
            extended: Arc::clone(&self.extended),
        })
    }

    /// The command power allocations the hour changed, every one when `whole`.
    fn command(&self, state: &State, whole: bool) -> io::Result<Vec<(u32, Option<i64>)>> {
        let table = TableId::CountryCommandPower;
        if !columns::complete(state, table) {
            return Ok(Vec::new());
        }
        let held = columns::table(state, table)?;
        let allocated = columns::plain::<i64>(held, self.fields.allocated)?;
        let slots = if whole {
            live(state, table)?
        } else {
            written(state, table, 1 << self.fields.allocated)?
        };
        let mut command = Vec::with_capacity(slots.len());
        for slot in slots {
            let key = held
                .key(slot)
                .ok_or_else(|| dashboard_core::invalid("A command power row holds no key"))?;
            let value = if held.recorded(self.fields.allocated, slot) {
                allocated.get(slot as usize).copied().unwrap_or_default()
            } else {
                columns::row::<CountryCommandPowerRow>(state, table, slot, self.format.version())?
                    .allocated_command_power_raw
            };
            command.push((key as u32, Some(value)));
        }
        if !whole {
            command.extend(
                columns::changes(state, table)?
                    .deleted_keys
                    .iter()
                    .map(|&key| (key as u32, None)),
            );
        }
        Ok(command)
    }

    /// `token`'s spelling at the hour, as LexicalTokens holds it.
    fn spelling(&self, state: &State, token: u32) -> io::Result<Option<String>> {
        if !columns::complete(state, TableId::LexicalTokens) {
            return Ok(None);
        }
        let held = columns::table(state, TableId::LexicalTokens)?;
        let Some(slot) = held.slot(u128::from(token)) else {
            return Ok(None);
        };
        let bytes = if held.recorded(self.fields.literal, slot) {
            let id = *columns::text(held, self.fields.literal)?
                .get(slot as usize)
                .ok_or_else(|| dashboard_core::invalid("A token holds no spelling cell"))?;
            state
                .dictionary()
                .get(id)
                .ok_or_else(|| dashboard_core::invalid("A spelling names no dictionary entry"))?
                .to_vec()
        } else {
            columns::row::<LexicalTokenRow>(
                state,
                TableId::LexicalTokens,
                slot,
                self.format.version(),
            )?
            .literal
        };
        Ok(String::from_utf8(bytes).ok())
    }

    /// The modifier definitions `frame` records by name; None unless it holds both tables
    /// whole. The walk's views' index is kept while neither table moves.
    fn modifier_index<F: Tables + ?Sized>(
        &mut self,
        frame: &F,
        walked: bool,
    ) -> io::Result<Option<Arc<BTreeMap<String, ModifierDefinitionRow>>>> {
        if walked && let Some(index) = &self.modifiers {
            return Ok(index.clone());
        }
        let index = match (
            frame.complete_rows::<tables::ModifierDefinitions>(u64::MAX),
            frame.complete_rows::<tables::LexicalTokens>(items::LITERAL),
        ) {
            (Ok(rows), Ok(tokens)) => Some(Arc::new(idea_format::index_rowsets(rows, tokens)?)),
            _ => None,
        };
        if walked {
            self.modifiers = Some(index.clone());
        }
        Ok(index)
    }

    /// What `frame`, the read at the rows' own hour, and `scripted`, its scripted text's frame, say
    /// of `rows`' items; `current`, the walk's views, names countries through the walk's
    /// roster, the hour before's copies through their own links.
    fn date<F: Tables + ?Sized>(
        &mut self,
        frame: &F,
        scripted: &TextFrame<'_>,
        roster: &Roster,
        current: bool,
        rows: &mut [(
            Item<'_>,
            Option<u32>,
            u32,
            Option<PowerDailyCostKind>,
            &mut Dating,
        )],
    ) -> io::Result<()> {
        let content = self.content.clone();
        let mut raws = BTreeSet::new();
        for (item, _, country, _, _) in rows.iter() {
            raws.insert(*country);
            raws.extend(item.identity.target_country);
        }
        let names = self.names(frame, roster, current, &raws);
        let modifiers = if rows.iter().any(|(item, ..)| {
            matches!(
                item.cause,
                PowerCause::Idea | PowerCause::Advisor | PowerCause::AdvisorRemoval
            )
        }) {
            self.modifier_index(frame, current)?
        } else {
            None
        };
        let texts = FrameTexts::default();
        for (item, _, country, component, dating) in rows.iter_mut() {
            let mut facts = items::event_facts(Some(frame), item.identity, item.cause, *component);
            let own = names.get(country).cloned();
            let target = item
                .identity
                .target_country
                .and_then(|raw| names.get(&raw).cloned());
            let mut modifier_rows = None;
            if let Some(content) = content.as_deref() {
                facts.text =
                    items::item_text(own.as_ref(), item, &facts, content, Some(scripted), &texts);
                if matches!(
                    item.cause,
                    PowerCause::Idea | PowerCause::Advisor | PowerCause::AdvisorRemoval
                ) {
                    modifier_rows = items::tooltip_modifiers(
                        content,
                        &facts,
                        own.as_ref(),
                        item.identity,
                        modifiers.as_deref(),
                    );
                    if modifiers.is_none() {
                        modifier_rows = None;
                    }
                }
                if dating.category.is_some() {
                    let text = category(item, &facts, *component, Some(content));
                    dating.category = Some(self.category(&text));
                }
            }
            let value = Dated {
                facts,
                own,
                target,
                modifiers: modifier_rows,
            };
            let bytes = dashboard_base::prepared::codec::encode(&value)?;
            dating.dated = Some(self.dated(frame.at_hour(), bytes));
        }
        Ok(())
    }

    /// The names at `frame`'s hour of `raws`.
    fn names<F: Tables + ?Sized>(
        &self,
        frame: &F,
        roster: &Roster,
        current: bool,
        raws: &BTreeSet<u32>,
    ) -> BTreeMap<u32, NamedCountry> {
        let links = frame
            .complete_rows::<tables::CountryLinks>(items::LINK)
            .ok();
        let countries = frame
            .complete_rows::<tables::Countries>(items::COUNTRY)
            .ok();
        let slots: Vec<(u32, CountryRow)> = raws
            .iter()
            .filter_map(|&raw| {
                let country = if current {
                    let id = roster.at_slot(roster.slot_of(raw)?).last()?.id;
                    countries?.get(u128::from(id)).ok().flatten()?.into_owned()
                } else {
                    let slot = links
                        .as_ref()
                        .and_then(|links| dashboard_text::metadata::link_slot_in(links, raw))?;
                    // A canonical slot's last row, in key order.
                    countries?
                        .iter()
                        .filter_map(Result::ok)
                        .filter(|row| row.canonical_slot == slot)
                        .last()?
                        .into_owned()
                };
                Some((raw, country))
            })
            .collect();
        items::named_countries(frame, self.content.as_deref(), &slots)
            .into_iter()
            .collect()
    }
}

/// The rows `power` and `costs` date at `at`, as [`Projector::date`] reads them.
#[allow(clippy::type_complexity)]
fn dated_at<'r>(
    power: &'r mut [Power],
    costs: &'r mut [Cost],
    at: i32,
) -> Vec<(
    Item<'r>,
    Option<u32>,
    u32,
    Option<PowerDailyCostKind>,
    &'r mut Dating,
)> {
    let mut dated = Vec::new();
    for power in power
        .iter_mut()
        .filter(|power| power.row.hour == at && named(&power.row))
    {
        dated.push((
            Item {
                cause: power.row.cause,
                identity: &power.row.identity,
            },
            power.row.identity.target_country,
            power.row.country,
            None,
            &mut power.dating,
        ));
    }
    for cost in costs
        .iter_mut()
        .filter(|cost| cost.row.day_hour == at && cost.row.cost_raw != 0)
    {
        dated.push((
            Item {
                cause: cost.row.cause,
                identity: &cost.row.identity,
            },
            cost.row.identity.target_country,
            cost.row.country,
            Some(cost.row.component),
            &mut cost.dating,
        ));
    }
    dated
}

/// What the late rows' copies hold: the kit and Campaign, whole, as the walk's frame held them.
fn carried_asks() -> Vec<(TableId, Hold)> {
    kit()
        .iter()
        .chain(&[TableId::Campaign])
        .map(|&table| (table, Hold::Whole))
        .collect()
}

/// The category a payment of `item` is spent in.
fn category(
    item: &Item<'_>,
    facts: &items::EventFacts,
    component: Option<PowerDailyCostKind>,
    content: Option<&PoliticsContent>,
) -> Cow<'static, str> {
    if component.is_some() && item.cause == PowerCause::DailyGain {
        return Cow::Borrowed(match component {
            Some(PowerDailyCostKind::MioResearch | PowerDailyCostKind::MioProduction) => {
                "organisationUpkeep"
            }
            _ => "dailyCosts",
        });
    }
    let token = items::recorded_token(facts, item.identity);
    Cow::Owned(present::category_of(item.cause, token, content).0)
}

impl Projector {
    /// Hour `walked`'s facts; `whole` hands every table.
    fn project(&mut self, walked: &segments::Hour<'_, World>, whole: bool) -> io::Result<Hour> {
        let state = walked.state;
        let hour = walked.hour;
        // The copies carried from the hour before date its late rows; an hour that refuses
        // carries none on.
        let follows = self
            .carried_at
            .take()
            .is_some_and(|(branch, at)| branch == walked.branch && at.checked_add(1) == Some(hour));
        let mut carried = self.carried.take();
        // The modifier index kept from the views lasts while neither of its tables moves.
        if whole
            || walked.world.touched(TableId::ModifierDefinitions, u64::MAX)
            || walked.world.touched(TableId::LexicalTokens, u64::MAX)
        {
            self.modifiers = None;
        }
        self.last = Some(hour);
        let roster = walked.world.countries()?;
        let mut complete = 0;
        for table in NATIVE {
            if columns::complete(state, table) {
                complete |= bit(table);
            }
        }
        if columns::complete(state, TableId::LexicalTokens) {
            complete |= LEXICAL;
        }
        // A table that becomes complete is read whole: its rows were not followed.
        let regained = complete & !self.complete;
        self.complete = complete & !LEXICAL;
        let whole_of = |table: TableId| whole || regained & bit(table) != 0;
        let census = (whole || roster.changed() || regained & bit(TableId::Countries) != 0).then(
            || -> Arc<[CensusRow]> {
                roster
                    .iter()
                    .map(|country| CensusRow {
                        id: country.id,
                        slot: country.slot,
                        tag: Arc::from(roster.tag(country.tag)),
                    })
                    .collect()
            },
        );
        let links = (whole
            || roster.rebuilt()
            || !roster.changed_links().is_empty()
            || regained & bit(TableId::CountryLinks) != 0)
            .then(|| -> Arc<[(u32, u32)]> {
                roster
                    .links()
                    .iter()
                    .map(|link| (link.id, link.canonical_slot))
                    .collect()
            });
        let captures = self.captures(state, hour, whole_of(TableId::PoliticsCaptureCoverage))?;
        let (power, power_deleted) = self.rows::<CountryPowerChangeRow>(
            state,
            TableId::CountryPowerChanges,
            whole_of(TableId::CountryPowerChanges),
        )?;
        let (governments, government_deleted) = self.rows::<GovernmentChangeRow>(
            state,
            TableId::GovernmentChanges,
            whole_of(TableId::GovernmentChanges),
        )?;
        let (daily, daily_deleted) = self.rows::<CountryPowerDailyRow>(
            state,
            TableId::CountryPowerDaily,
            whole_of(TableId::CountryPowerDaily),
        )?;
        let (costs, cost_deleted) = self.rows::<CountryPowerDailyCostRow>(
            state,
            TableId::CountryPowerDailyCosts,
            whole_of(TableId::CountryPowerDailyCosts),
        )?;
        let command_whole = whole_of(TableId::CountryCommandPower);
        let command = self.command(state, command_whole)?;
        let unnamed = self.category("");
        let mut power: Vec<Power> = power
            .into_iter()
            .map(|row| Power {
                dating: Dating {
                    dated: None,
                    category: rows::charge(&row).map(|_| Arc::clone(&unnamed)),
                },
                row,
                resolved: None,
                shape: None,
            })
            .collect();
        let mut costs: Vec<Cost> = costs
            .into_iter()
            .map(|row| Cost {
                dating: Dating {
                    dated: None,
                    category: (row.cost_raw != 0).then(|| Arc::clone(&unnamed)),
                },
                row,
                item: None,
                shape: None,
            })
            .collect();
        let held = carried
            .as_ref()
            .filter(|_| follows)
            .map(Copies::frame);
        let wanted = |at: i32| {
            power
                .iter()
                .any(|power| power.row.hour == at && named(&power.row))
                || costs
                    .iter()
                    .any(|cost| cost.row.day_hour == at && cost.row.cost_raw != 0)
        };
        let before = hour.saturating_sub(1);
        let (now, late) = (wanted(hour), before != hour && wanted(before));
        // The rows dated at the hour, from its views.
        if now {
            let columns = walked.world.columns(state);
            let text = TextFrame::walked(walked.world, state);
            let mut dated = dated_at(&mut power, &mut costs, hour);
            self.date(&columns, &text, roster, true, &mut dated)?;
        }
        // And at the hour before, from the copies carried from it.
        if let Some(frame) = held.filter(|_| late) {
            let text = TextFrame::Carried(frame);
            let mut dated = dated_at(&mut power, &mut costs, before);
            self.date(frame, &text, roster, false, &mut dated)?;
        }
        // A frameless payment's category reads its recorded key alone.
        let content = self.content.clone();
        for (row, dating) in power
            .iter_mut()
            .map(|power| {
                (
                    (
                        power.row.cause,
                        &power.row.identity,
                        None::<PowerDailyCostKind>,
                    ),
                    &mut power.dating,
                )
            })
            .chain(costs.iter_mut().map(|cost| {
                (
                    (cost.row.cause, &cost.row.identity, Some(cost.row.component)),
                    &mut cost.dating,
                )
            }))
        {
            if dating.dated.is_none() && dating.category.is_some() {
                let (cause, identity, component) = row;
                let facts = items::event_facts::<DecodedFrame>(None, identity, cause, component);
                let text = category(&Item { cause, identity }, &facts, component, content.as_deref());
                dating.category = Some(self.category(&text));
            }
        }
        for power in &mut power {
            let resolved = power.resolve();
            let key = (resolved.item, power.row.instance, power.row.epoch);
            power.resolved = Some(resolved);
            let shape = match self.powers.get(hour, &key, |held| held.fits(power)) {
                Some(held) => held,
                None => self.powers.put(key, PowerShape::of(power)),
            };
            power.shape = Some(shape);
        }
        for cost in &mut costs {
            let item = cost.resolve();
            let key = (item, cost.row.instance, cost.row.epoch);
            cost.item = Some(item);
            let shape = match self.costs.get(hour, &key, |held| held.fits(cost)) {
                Some(held) => held,
                None => self.costs.put(key, CostShape::of(cost)),
            };
            cost.shape = Some(shape);
        }
        let mut handed_governments = Vec::with_capacity(governments.len());
        for row in governments {
            let spell = |token: Option<u32>| -> io::Result<Option<String>> {
                let Some(token) = token else {
                    return Ok(None);
                };
                if row.hour == hour {
                    self.spelling(state, token)
                } else if let Some(frame) = held.filter(|frame| frame.hour == row.hour) {
                    Ok(items::lexical_token(frame, token))
                } else {
                    Ok(None)
                }
            };
            let tokens = (spell(row.before_ideology)?, spell(row.after_ideology)?);
            handed_governments.push(Government { row, tokens });
        }
        // A capture interval starting at the hour may leave rows of the hour to the next read.
        let discontinuous = complete & bit(TableId::PoliticsCaptureCoverage) == 0
            || captures
                .changed
                .iter()
                .any(|row| row.since_hour == hour && row.through_hour == hour);
        // Copies are brought on only from the hour before; any others are let go.
        if let Some(copies) = carried.as_mut()
            && !(discontinuous && follows)
        {
            copies.clear();
        }
        if discontinuous {
            let mut copies = match carried {
                Some(copies) => copies,
                None => Copies::new(carried_asks())?,
            };
            copies.hour(walked.world, state)?;
            self.carried = Some(copies);
            self.carried_at = Some((walked.branch, hour));
        } else {
            self.carried = carried;
            self.carried_at = None;
        }
        Ok(Hour {
            branch: walked.branch,
            hour,
            whole,
            complete,
            census,
            links,
            captures,
            power,
            governments: handed_governments,
            daily,
            costs,
            deleted: [
                power_deleted,
                government_deleted,
                daily_deleted,
                cost_deleted,
            ],
            command,
            command_whole,
        })
    }
}

impl Operator<World> for Projector {
    type Output = Hour;

    fn hour(
        &mut self,
        walked: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("politics_ledger.facts", None);
        let hour = walked.hour;
        let whole = walked.starts
            || self
                .last
                .is_none_or(|last| last.checked_add(1) != Some(hour));
        let projected = self.project(walked, whole)?;
        out.push(projected).map_err(segments::family_failure)
    }

    fn carried(&self) -> Option<usize> {
        None
    }
}
