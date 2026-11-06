//! The market's facts, projected hour by hour from a columnar walk ([`replay::segments`]). An
//! hour that writes a field the market reads, or changes the countries, is observed: it hands
//! the ordered emit ([`super::emit`]) the rows of each state table it wrote (every row at the
//! segment's first hour, where a table's status or the links changed, or where the identities
//! came back), each with the values the row shows and its holders resolved through the links;
//! each table's refusal where it is not complete; the occurrence table's rows where it changed;
//! and the countries' tags where they changed. Every hour hands the variant catalogue's
//! evidence and the operands its complete reads build
//! ([`dashboard_base::products::equipment::columnar`]). What came before the segment is the
//! emit's.

use super::{Field, STATE, ledger_kind};
use dashboard_base::products::equipment::{
    RawVariant, VariantCatalogue, VariantHour,
    columnar::{Naming, slots},
};
use dashboard_world::{Hold, Needs, Roster, World};
use replay::{
    columnar_frames::metrics,
    columns,
    segments::{self, Operator, Sink, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{
        CountryMarketRow, LendLeaseAgreementRow, MarketContractTermsRow, MarketEventRow,
        MarketPool, PurchaseContractRow, RowCodec, TABLE_COUNT, TableId, tables,
    },
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, Metrics, not_selected};
use std::{
    collections::BTreeMap,
    io,
    mem::size_of,
    sync::{Arc, LazyLock},
};

/// The tables a projection reads: the market's, the countries naming its holders, the variant
/// catalogue's, and Campaign, which every frame of the walk's world holds.
pub fn tables() -> &'static [TableId] {
    static TABLES: LazyLock<Vec<TableId>> = LazyLock::new(|| {
        let mut tables = vec![TableId::Campaign, TableId::Countries, TableId::CountryLinks];
        tables.extend(STATE);
        tables.extend(VariantCatalogue::TABLES);
        tables.sort_unstable();
        tables.dedup();
        tables
    });
    &TABLES
}

/// The fields of each market table the observer reads, by name; every field of one naming none.
const READ: [&[&str]; 5] = [
    &["inventory", "prices"],
    &["buyer", "speed", "equipment", "prices", "price_levels"],
    &[],
    &[
        "seller",
        "buyer",
        "convoys_assigned",
        "convoys_total",
        "collected_raw",
        "completed_deliveries",
    ],
    &[],
];

/// Each market table's fields the observer reads, as a change mask.
fn masks() -> io::Result<[u64; 5]> {
    let mut masks = [0; 5];
    for (mask, (&table, names)) in masks.iter_mut().zip(STATE.iter().zip(READ)) {
        *mask = if names.is_empty() {
            table.full_mask()
        } else {
            let mut fields = 0;
            for name in names {
                fields |= 1_u64 << columns::field(table, name)?;
            }
            fields
        };
    }
    Ok(masks)
}

/// How a projection asks the world for the market's tables: each in the fields it reads.
fn asks(masks: &[u64; 5]) -> Vec<(TableId, Hold)> {
    STATE
        .iter()
        .zip(masks)
        .map(|(&table, &mask)| (table, Hold::Fields(mask).of(table)))
        .collect()
}

/// What a projection reads of its walk's world: the countries, their naming, and its tables.
pub fn needs() -> io::Result<Needs> {
    Ok(Needs {
        countries: true,
        naming: true,
        frame: asks(&masks()?),
        ..Needs::default()
    })
}

/// Why a table is not complete, as the row decoder names it.
pub type Refusal = Box<str>;

/// The reason the row decoder gives `table`, not complete at `hour`.
fn refusal(state: &State, table: TableId, mode: CaptureMode, hour: i32) -> io::Result<Refusal> {
    Ok(match columns::status(state, table) {
        columns::Status::Unavailable { reason } => {
            let reason = state
                .dictionary()
                .get(reason)
                .ok_or_else(|| io::Error::other("A market reason names no dictionary entry"))?;
            format!(
                "hour {hour} {table:?} is unavailable: {}",
                String::from_utf8_lossy(reason)
            )
        }
        columns::Status::Truncated { missing } => {
            format!("hour {hour} {table:?} is truncated ({missing} missing rows)")
        }
        columns::Status::NotRecorded => {
            format!("hour {hour} {table:?} is not recorded in a {mode} recording")
        }
        columns::Status::Complete => {
            return Err(io::Error::other("A complete market table has no reason"));
        }
    }
    .into())
}

/// A state row as the observer reads it: the values it shows, in the order it reads them, the
/// variants it names, whether its owner has no identity (it shows nothing then), and how many
/// of its holders have none.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct Row {
    pub values: Vec<(Field, i128)>,
    pub variants: Vec<u64>,
    pub orphan: bool,
    pub unresolved: u32,
}

impl Row {
    fn orphan() -> Self {
        Self {
            orphan: true,
            unresolved: 1,
            ..Self::default()
        }
    }

    fn heap(&self) -> usize {
        self.values.capacity() * size_of::<(Field, i128)>() + self.variants.capacity() * 8
    }
}

/// What an hour hands of a state table.
#[derive(Debug)]
#[derive(replay_spool::Spooled)]
pub enum Table {
    /// The countries' identities stopped being read: nothing of it is observed.
    Skipped,
    /// It is not complete.
    Refused(Refusal),
    /// Its rows by key, ascending: every one when `whole`, else those the hour wrote in a field
    /// the observer reads (None: deleted).
    Rows {
        whole: bool,
        rows: Vec<(u128, Option<Row>)>,
    },
}

impl Table {
    fn heap(&self) -> usize {
        match self {
            Self::Skipped => 0,
            Self::Refused(refusal) => refusal.len(),
            Self::Rows { rows, .. } => {
                rows.capacity() * size_of::<(u128, Option<Row>)>()
                    + rows.iter().flat_map(|(_, row)| row).map(Row::heap).sum::<usize>()
            }
        }
    }
}

/// An occurrence the ledger keeps, its parties resolved at the hour it was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct Occurrence {
    pub row: MarketEventRow,
    pub seller: Option<u32>,
    pub buyer: Option<u32>,
}

/// What an hour hands of the occurrence table, where it changed.
#[derive(Debug)]
#[derive(replay_spool::Spooled)]
pub enum Events {
    Refused(Refusal),
    /// Every row's id, and the rows of the ledger's kinds, in table order.
    Rows {
        ids: Vec<u64>,
        kept: Vec<Occurrence>,
    },
}

/// What the ordered emit reads of an observed hour.
#[derive(Debug, Default)]
#[derive(replay_spool::Spooled)]
pub struct Facts {
    /// Whether the countries' identities are read, and why not, where that changed.
    pub identities: Option<Result<(), Refusal>>,
    /// Every holder slot's tag, where they changed while the identities are read.
    pub tags: Option<Vec<(u32, Box<str>)>>,
    pub tables: [Option<Table>; 4],
    pub events: Option<Events>,
}

impl Facts {
    fn heap(&self) -> usize {
        size_of::<Self>()
            + match &self.identities {
                Some(Err(refusal)) => refusal.len(),
                _ => 0,
            }
            + self.tags.as_ref().map_or(0, |tags| {
                tags.capacity() * size_of::<(u32, Box<str>)>()
                    + tags.iter().map(|(_, tag)| tag.len()).sum::<usize>()
            })
            + self.tables.iter().flatten().map(Table::heap).sum::<usize>()
            + match &self.events {
                Some(Events::Refused(refusal)) => refusal.len(),
                Some(Events::Rows { ids, kept }) => {
                    ids.capacity() * 8 + kept.capacity() * size_of::<Occurrence>()
                }
                None => 0,
            }
    }
}

/// One hour of a segment.
#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub hour: i32,
    /// Whether Countries and CountryLinks are complete.
    pub identities: bool,
    /// Whether the hour wrote any field of a table the market reads, or changed its status: an
    /// hour the row preparation observed.
    pub woke: bool,
    /// The variant catalogue's evidence, and the operands its complete read built.
    pub variants: VariantHour,
    pub raws: Vec<(u64, RawVariant)>,
    /// None where the hour changes nothing the market reads.
    pub facts: Option<Box<Facts>>,
    bytes: usize,
}

#[cfg(test)]
impl Hour {
    /// An hour as a projector hands it on.
    pub(crate) fn of(
        hour: i32,
        identities: bool,
        (variants, raws): (VariantHour, Vec<(u64, RawVariant)>),
        facts: Option<Facts>,
    ) -> Self {
        Self {
            hour,
            identities,
            woke: facts.is_some(),
            variants,
            raws,
            facts: facts.map(Box::new),
            bytes: 0,
        }
    }

    /// The hour, waking the row preparation though it changes nothing the market reads.
    pub(crate) fn woken(self) -> Self {
        Self { woke: true, ..self }
    }
}

/// The frame the world's views are lent into for an hour, given back when dropped.
struct Lent<'o> {
    frame: &'o mut DecodedFrame,
    asks: &'o [(TableId, Hold)],
}

impl Drop for Lent<'_> {
    fn drop(&mut self) {
        dashboard_world::release(self.frame, self.asks);
    }
}

/// The bytes a built variant's operands hold.
fn raws_heap(raws: &Vec<(u64, RawVariant)>) -> usize {
    raws.capacity() * size_of::<(u64, RawVariant)>()
        + raws.iter().map(|(_, raw)| raw.heap()).sum::<usize>()
}

/// The keys of `table` the hour wrote in a field of `mask` or inserted, and those it deleted,
/// ascending; a key deleted and inserted again within the hour is written.
fn edited(state: &State, table: TableId, mask: u64) -> io::Result<(Vec<u128>, Vec<u128>)> {
    let held = columns::table(state, table)?;
    let changes = columns::changes(state, table)?;
    let mut written: Vec<u128> = changes
        .changed
        .iter()
        .zip(&changes.masks)
        .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
        .filter(|&(_, &fields)| fields & mask != 0)
        .map(|(&slot, _)| slot)
        .chain(changes.inserted.iter().copied())
        .filter_map(|slot| held.key(slot))
        .collect();
    written.sort_unstable();
    written.dedup();
    let mut deleted = changes.deleted_keys.clone();
    deleted.sort_unstable();
    deleted.dedup();
    deleted.retain(|key| written.binary_search(key).is_err());
    Ok((written, deleted))
}

/// Each entry of an ordered pool under the field `field` names.
fn pool(row: &mut Row, items: &MarketPool, field: impl Fn(u32, u64) -> Field) {
    for (ordinal, item) in items.items.iter().enumerate() {
        row.variants.push(item.variant);
        row.values
            .push((field(ordinal as u32, item.variant), i128::from(item.amount_raw)));
    }
}

fn offers(row: &CountryMarketRow, roster: &Roster) -> Row {
    let Some(seller) = roster.slot_of(row.country) else {
        return Row::orphan();
    };
    let mut read = Row::default();
    pool(&mut read, &row.inventory, |ordinal, variant| {
        Field::Inventory(seller, ordinal, variant)
    });
    for level in row.prices.iter() {
        read.variants.push(level.variant);
        read.values
            .push((Field::OfferLevel(seller, level.variant), i128::from(level.level)));
    }
    read
}

fn terms(row: &MarketContractTermsRow, roster: &Roster) -> Row {
    let Some(seller) = roster.slot_of(row.country) else {
        return Row::orphan();
    };
    let (id, kind) = (row.id, row.kind);
    let mut read = Row::default();
    read.values.push(match roster.slot_of(row.buyer) {
        Some(buyer) => (Field::ContractBuyer(seller, id, kind), i128::from(buyer)),
        None => {
            read.unresolved += 1;
            (Field::ContractUnidentified(seller, id, kind), 1)
        }
    });
    read.values
        .push((Field::ContractSpeed(seller, id, kind), i128::from(row.speed)));
    for (which, items) in [(0, &row.equipment), (1, &row.prices)] {
        pool(&mut read, items, |ordinal, variant| {
            Field::ContractPool(seller, id, kind, which, ordinal, variant)
        });
    }
    for level in row.price_levels.iter() {
        read.variants.push(level.variant);
        read.values.push((
            Field::ContractLevel(seller, id, kind, level.variant),
            i128::from(level.level),
        ));
    }
    read
}

fn agreements(row: &LendLeaseAgreementRow, roster: &Roster) -> Row {
    let Some(giver) = roster.slot_of(row.country) else {
        return Row::orphan();
    };
    let id = row.id;
    let mut read = Row::default();
    read.values.push(match roster.slot_of(row.receiver) {
        Some(receiver) => (Field::LeaseReceiver(giver, id), i128::from(receiver)),
        None => {
            read.unresolved += 1;
            (Field::LeaseUnidentified(giver, id), 1)
        }
    });
    read.values.extend([
        (Field::LeaseRevision(giver, id), i128::from(row.revision)),
        (Field::LeaseSince(giver, id), i128::from(row.active_since)),
        (Field::LeaseLast(giver, id), i128::from(row.last_delivery)),
        (Field::LeaseFuel(giver, id, 0), i128::from(row.fuel_daily_raw)),
        (Field::LeaseFuel(giver, id, 1), i128::from(row.fuel_percentage_raw)),
        (Field::LeaseFuel(giver, id, 2), i128::from(row.fuel_sent_raw)),
        (Field::LeaseFuel(giver, id, 3), i128::from(row.fuel_sunk_raw)),
        (Field::LeaseFuel(giver, id, 4), i128::from(row.last_fuel_delivered_raw)),
    ]);
    for (which, items) in [&row.monthly, &row.percentage, &row.once, &row.delivered, &row.sunk]
        .into_iter()
        .enumerate()
    {
        pool(&mut read, items, |ordinal, variant| {
            Field::LeasePool(giver, id, which as u8, ordinal, variant)
        });
    }
    read
}

fn purchases(row: &PurchaseContractRow, roster: &Roster) -> Row {
    let (seller, buyer) = (roster.slot_of(row.seller), roster.slot_of(row.buyer));
    let mut read = Row {
        unresolved: u32::from(seller.is_none()) + u32::from(buyer.is_none()),
        ..Row::default()
    };
    for (which, value) in [
        (0, seller.map(i128::from)),
        (1, buyer.map(i128::from)),
        (2, Some(i128::from(row.convoys_assigned))),
        (3, Some(i128::from(row.convoys_total))),
        (4, Some(i128::from(row.collected_raw))),
        (5, Some(i128::from(row.completed_deliveries))),
    ] {
        if let Some(value) = value {
            read.values.push((Field::Purchase(row.id, which), value));
        }
    }
    read
}

/// The rows of a complete table, sorted by key: every one when `edits` is None, else the
/// hour's written and deleted keys.
fn read_rows<R: RowCodec>(
    rows: &[R],
    edits: Option<(Vec<u128>, Vec<u128>)>,
    read: impl Fn(&R) -> Row,
) -> io::Result<Vec<(u128, Option<Row>)>> {
    let Some((written, deleted)) = edits else {
        return Ok(rows.iter().map(|row| (RowCodec::key(row), Some(read(row)))).collect());
    };
    let mut out = Vec::with_capacity(written.len() + deleted.len());
    for key in written {
        let at = rows
            .binary_search_by_key(&key, RowCodec::key)
            .map_err(|_| io::Error::other("A written market row is not in its table"))?;
        out.push((key, Some(read(&rows[at]))));
    }
    out.extend(deleted.into_iter().map(|key| (key, None)));
    out.sort_unstable_by_key(|&(key, _)| key);
    Ok(out)
}

/// Every holder slot's tag as the observer reads the countries: each slot's last country by id
/// names it, when its tag is UTF-8 and not empty.
fn tags(roster: &Roster) -> Vec<(u32, Box<str>)> {
    let mut slots = BTreeMap::new();
    for country in roster.iter() {
        slots.insert(country.slot, roster.tag(country.tag));
    }
    slots
        .into_iter()
        .filter_map(|(slot, tag)| {
            std::str::from_utf8(tag)
                .ok()
                .filter(|tag| !tag.is_empty())
                .map(|tag| (slot, Box::from(tag)))
        })
        .collect()
}

/// One segment's projection: every segment starts cold.
pub struct Projector {
    format: FormatVersion,
    mode: CaptureMode,
    blocks: Arc<BTreeMap<u64, Arc<[i32]>>>,
    block_starts: Arc<[i32]>,
    next_block: usize,
    masks: [u64; 5],
    asks: Vec<(TableId, Hold)>,
    /// The frame the world's views are lent into for an hour, given back before it ends.
    frame: DecodedFrame,
    naming: Naming,
    /// Nothing is held: the next hour is read whole, as a segment's first.
    cold: bool,
    last: Option<i32>,
    /// Each table's status the hour before, by table index.
    shapes: [Option<columns::Status>; TABLE_COUNT],
    identities: bool,
    /// The tags last handed on in this segment.
    tags: Vec<(u32, Box<str>)>,
}

/// What [`Projector::save`] writes: nothing but that it restarts, since everything it holds is
/// read again from the walker's tables, as at a segment's first hour.
const SAVED: &[u8] = b"market projector: cold";

impl Projector {
    pub fn new(
        format: FormatVersion,
        mode: CaptureMode,
        blocks: Arc<BTreeMap<u64, Arc<[i32]>>>,
    ) -> io::Result<Self> {
        let masks = masks()?;
        Ok(Self {
            format,
            mode,
            blocks,
            block_starts: Arc::from([]),
            next_block: 0,
            asks: asks(&masks),
            masks,
            frame: DecodedFrame {
                branch: 0,
                hour: 0,
                tables: std::array::from_fn(|_| not_selected()),
                metrics: Metrics::default(),
            },
            naming: Naming::new()?,
            cold: true,
            last: None,
            shapes: [None; TABLE_COUNT],
            identities: false,
            tags: Vec::new(),
        })
    }

    fn heap(&self) -> usize {
        self.naming.heap()
            + self.tags.capacity() * size_of::<(u32, Box<str>)>()
            + self.tags.iter().map(|(_, tag)| tag.len()).sum::<usize>()
    }
}

// Every segment starts cold: its first hour reads every row and country again; what came before
// is the emit's.
impl Operator<World> for Projector {
    type Output = Hour;

    fn start(&mut self, start: segments::Start<'_>, branch: u64, first: i32) -> io::Result<()> {
        self.cold = true;
        // A branch the row recording holds no block of has no row block start to observe.
        self.block_starts = self.blocks.get(&branch).cloned().unwrap_or_else(|| Arc::from([]));
        self.next_block = self.block_starts.partition_point(|&hour| hour < first);
        // A restart holds the hour before's statuses, which this hour's may have moved from.
        if let segments::Start::Restart(before) = start {
            for &table in tables() {
                self.shapes[table.index()] = Some(columns::status(before, table));
            }
        }
        Ok(())
    }

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("market.columnar", Some("market"));
        let state = hour.state;
        let at = hour.hour;
        let (roster, names) = (hour.world.countries()?, hour.world.naming()?);
        let opens = self.cold
            || hour.starts
            || hour.opens
            || self.last.and_then(|last| last.checked_add(1)) != Some(at);
        self.cold = false;
        self.last = Some(at);
        let read = dashboard_text::metadata::slot_read(
            self.format,
            &metrics(hour.manifest),
            TableId::VariantDefinitions,
            at,
        );
        // Whether a table's status moved since the hour before, and whether it is restated.
        let mut moved = [false; TABLE_COUNT];
        let mut shaped = [false; TABLE_COUNT];
        for &table in tables() {
            let (index, status) = (table.index(), columns::status(state, table));
            moved[index] = self.shapes[index].is_some_and(|held| held != status);
            shaped[index] = opens || self.shapes[index] != Some(status);
            self.shapes[index] = Some(status);
        }
        let (variants, raws) = {
            let _cpu = replay_frame::cpu::scope("market.columnar.naming", None);
            let mut written = [false; TABLE_COUNT];
            for table in VariantCatalogue::TABLES {
                let index = table.index();
                written[index] = shaped[index] || slots::relevant(state, table, u64::MAX)?;
            }
            let world = (roster, names);
            self.naming
                .project(state, world, self.format, at, opens, &shaped, &written, read)?
        };
        let restated: [bool; 5] = std::array::from_fn(|index| shaped[STATE[index].index()]);
        let mut changed = [false; 5];
        // The row preparation starts a fresh change tracker at each row block.
        while self.block_starts.get(self.next_block).is_some_and(|&first| first < at) {
            self.next_block += 1;
        }
        let mut woke = self.block_starts.get(self.next_block) == Some(&at);
        self.next_block += usize::from(woke);
        for (index, &table) in STATE.iter().enumerate() {
            woke |= moved[table.index()]
                || self.shapes[table.index()] != Some(columns::Status::Complete);
            changed[index] = restated[index]
                || (self.shapes[table.index()] == Some(columns::Status::Complete)
                    && slots::relevant(state, table, self.masks[index])?);
        }
        let mut reshaped = false;
        for table in [TableId::Countries, TableId::CountryLinks] {
            woke |= moved[table.index()]
                || self.shapes[table.index()] != Some(columns::Status::Complete);
            reshaped |= shaped[table.index()];
            woke |= slots::relevant(state, table, u64::MAX)?;
        }
        for &table in &STATE {
            woke = woke || slots::relevant(state, table, u64::MAX)?;
        }
        let identities = roster.complete();
        let flipped = opens || identities != self.identities;
        self.identities = identities;
        let relinked = flipped || roster.rebuilt() || !roster.changed_links().is_empty();
        if !(woke || reshaped || roster.changed() || changed.contains(&true)) {
            let bytes = raws_heap(&raws);
            return out.push(Hour {
                hour: at,
                identities,
                woke,
                variants,
                raws,
                facts: None,
                bytes,
            });
        }
        let mut facts = Facts::default();
        if flipped || (woke && !identities) {
            facts.identities = Some(if identities {
                Ok(())
            } else {
                let table = if columns::complete(state, TableId::Countries) {
                    TableId::CountryLinks
                } else {
                    TableId::Countries
                };
                Err(refusal(state, table, self.mode, at)?)
            });
        }
        if identities && (flipped || roster.changed()) {
            let now = tags(roster);
            if flipped || now != self.tags {
                facts.tags = Some(now.clone());
                self.tags = now;
            }
        }
        // The views go back before the hour ends, however it ends, so the next hour's builds
        // change them in place.
        let lent = Lent {
            frame: &mut self.frame,
            asks: &self.asks,
        };
        let wanted = changed[4] || (identities && (relinked || changed[..4].contains(&true)));
        if wanted {
            hour.world.frame_held_into(state, lent.asks, lent.frame)?;
        }
        let frame = &*lent.frame;
        for (index, &table) in STATE[..4].iter().enumerate() {
            let whole = restated[index] || relinked;
            facts.tables[index] = if !identities {
                flipped.then_some(Table::Skipped)
            } else if !columns::complete(state, table) {
                (whole || woke)
                    .then(|| refusal(state, table, self.mode, at).map(Table::Refused))
                    .transpose()?
            } else if whole || changed[index] {
                let edits = (!whole)
                    .then(|| edited(state, table, self.masks[index]))
                    .transpose()?;
                let rows = match table {
                    TableId::CountryMarket => {
                        read_rows(frame.complete::<tables::CountryMarket>()?, edits, |row| {
                            offers(row, roster)
                        })?
                    }
                    TableId::MarketContractTerms => read_rows(
                        frame.complete::<tables::MarketContractTerms>()?,
                        edits,
                        |row| terms(row, roster),
                    )?,
                    TableId::LendLeaseAgreements => read_rows(
                        frame.complete::<tables::LendLeaseAgreements>()?,
                        edits,
                        |row| agreements(row, roster),
                    )?,
                    _ => read_rows(
                        frame.complete::<tables::PurchaseContracts>()?,
                        edits,
                        |row| purchases(row, roster),
                    )?,
                };
                Some(Table::Rows { whole, rows })
            } else {
                None
            };
        }
        if changed[4] || (woke && !columns::complete(state, TableId::MarketEvents)) {
            let table = TableId::MarketEvents;
            facts.events = Some(if columns::complete(state, table) {
                let rows = frame.complete::<tables::MarketEvents>()?;
                let party = |raw: u32| identities.then(|| roster.slot_of(raw)).flatten();
                Events::Rows {
                    ids: rows.iter().map(|row| row.id).collect(),
                    kept: rows
                        .iter()
                        .filter(|row| ledger_kind(row))
                        .map(|&row| Occurrence {
                            row,
                            seller: party(row.country),
                            buyer: party(row.receiver),
                        })
                        .collect(),
                }
            } else {
                Events::Refused(refusal(state, table, self.mode, at)?)
            });
        }
        drop(lent);
        let bytes = facts.heap() + raws_heap(&raws);
        out.push(Hour {
            hour: at,
            identities,
            woke,
            variants,
            raws,
            facts: Some(Box::new(facts)),
            bytes,
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
                "Not a saved market projector",
            ));
        }
        *self = Self::new(self.format, self.mode, Arc::clone(&self.blocks))?;
        Ok(())
    }
}
