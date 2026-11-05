//! The market as the row walk prepared it, kept as the oracle the series family is held to: its
//! observer (each observed hour's whole state, keyed, with the occurrences' chains) and its
//! publication as the replies read it ([`super::reply::Read`]).

use super::{
    CHUNK, Field, MARKET_EVENTS, STATE, MarketHistoryParams, MarketHistoryReply, MarketLedgerParams,
    MarketLedgerReply, ledger_kind, ledger_row,
    reply::{self, Payload, Read},
};
use crate::products::keyed::{
    EconomyRun, HolderTag, Keyed, Observed, Published, Tags, branch_of,
};
use dashboard_base::{
    invalid,
    prepared::{frame_changes::FrameChanges, series::Run},
    products::equipment::{Texts, VariantCatalogue, VariantVersion},
};
use replay_data::{
    format::FormatVersion,
    rows::{MarketPool, TableId, tables},
};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{LazyLock, atomic::AtomicBool},
};

/// The tables the observer reads: the market's, the identities naming its
/// countries and the variant catalogue naming its equipment.
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

/// A chain's length and its open chunk's first hour.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
struct ChainState {
    count: u64,
    first: i32,
}

/// Appends `entry` to a chain at `at`, opening its next key every [`CHUNK`]
/// entries; the entry's position in the chain.
fn append(
    keyed: &mut Keyed<Field, Extra>,
    chain: &mut ChainState,
    field: impl Fn(u32, i32) -> Field,
    at: i32,
    entry: &impl Serialize,
) -> io::Result<u64> {
    let first = if chain.count.is_multiple_of(CHUNK) { at } else { chain.first };
    let chunk = u32::try_from(chain.count / CHUNK)
        .map_err(|_| invalid("a market occurrence chain outgrew its keys"))?;
    let key = keyed.key(field(chunk, first));
    keyed.values.event(key, at, entry)?;
    chain.first = first;
    chain.count += 1;
    Ok(chain.count - 1)
}

/// The variants the market names, with their naming reads.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Extra {
    pub variants: BTreeMap<u64, Vec<VariantVersion>>,
    /// The most hours an occurrence was supplied after its own hour.
    pub lag: i32,
    /// The hours whose country identities were read, merged: a holder's tag is
    /// proved only inside them.
    pub identified: Vec<(i32, i32)>,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    keyed: Keyed<Field, Extra>,
    variants: VariantCatalogue,
    /// Variants the market named, published with their naming reads.
    named: BTreeSet<u64>,
    /// The previous hour's occurrence ids, so one carried twice is kept once.
    last_events: BTreeSet<u64>,
    /// Each kind's stream.
    streams: BTreeMap<u8, ChainState>,
    /// Each party holder's index of each kind.
    indexes: BTreeMap<u32, BTreeMap<u8, ChainState>>,
    /// Occurrences kept so far, across kinds: the next one's sequence, which
    /// orders every stream and index alike.
    sequence: u64,
}

impl Prepared {
    pub fn observe_changes(
        &mut self,
        format: FormatVersion,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        let gap = self.keyed.step(frame.hour)?;
        crate::products::keyed::observe_variants(&mut self.variants, format, frame, Some(changes));
        // Every admitted hour, unchanged ones included, proves its identities or not.
        self.note_identities(frame);
        let watched = [TableId::Countries, TableId::CountryLinks]
            .into_iter()
            .chain(STATE)
            .any(|table| changes.rows_changed[table.index()]);
        if !gap && !watched {
            return Ok(());
        }
        self.observe(frame);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn step(&mut self, frame: &DecodedFrame) {
        self.keyed.step(frame.hour).unwrap();
        crate::products::keyed::observe_variants(&mut self.variants, FormatVersion::CURRENT, frame, None);
        self.note_identities(frame);
        self.observe(frame);
    }

    /// Extends the hours whose country identities were read by this one, if it read them.
    fn note_identities(&mut self, frame: &DecodedFrame) {
        if frame.complete::<tables::Countries>().is_err() || frame.complete::<tables::CountryLinks>().is_err() {
            return;
        }
        let identified = &mut self.keyed.meta.extra.identified;
        match identified.last_mut() {
            Some(last) if last.1.checked_add(1) == Some(frame.hour) => last.1 = frame.hour,
            Some(last) if last.1 >= frame.hour => {}
            _ => {
                identified.push((frame.hour, frame.hour));
                self.keyed.note_change(8);
            }
        }
    }

    fn observe(&mut self, frame: &DecodedFrame) {
        let mut observed = Observed::new();
        let mut named = Vec::new();
        // No later read repeats a drained occurrence, so it is kept without identities.
        match Tags::new(frame) {
            Ok(tags) => {
                // Every country's tags, not only active traders': an occurrence read
                // later matches its parties' tags at its own, earlier hour.
                self.keyed.tag_all(&tags, frame.hour);
                self.observe_state(frame, &tags, &mut observed, &mut named);
                self.observe_events(frame, Some(&tags), &mut observed, &mut named);
            }
            Err(error) => {
                self.keyed
                    .refuse(format!("country identities unavailable: {error}"));
                self.observe_events(frame, None, &mut observed, &mut named);
            }
        }
        self.named.extend(named.into_iter().filter(|&variant| variant != 0));
        // Occurrence keys hold no state; state keys end when not shown.
        self.keyed.commit(frame.hour, &observed, |field| {
            !matches!(field, Field::Stream(..) | Field::Party(..))
        });
    }

    fn observe_state(
        &mut self,
        frame: &DecodedFrame,
        tags: &Tags<'_>,
        observed: &mut Observed,
        named: &mut Vec<u64>,
    ) {
        let slot = |keyed: &mut Keyed<Field, Extra>, raw: u32| {
            let slot = tags.slot(raw);
            if slot.is_none() {
                keyed.refuse("a market country has no identity");
            }
            slot
        };
        let keyed = &mut self.keyed;
        let known = |keyed: &mut Keyed<Field, Extra>, observed: &mut Observed, table: u8| {
            observed.insert(keyed.key(Field::Known(table)), 1);
        };
        // A row whose owner has no identity is left out, so the table's roster is not known whole.
        let unknown = |keyed: &mut Keyed<Field, Extra>, observed: &mut Observed, table: u8| {
            observed.remove(&keyed.key(Field::Known(table)));
        };
        match frame.complete::<tables::CountryMarket>() {
            Ok(rows) => {
                known(keyed, observed, 0);
                let mut orphaned = false;
                for row in rows {
                    let Some(seller) = slot(keyed, row.country) else {
                        orphaned = true;
                        continue;
                    };
                    pool(keyed, observed, named, &row.inventory, &|ordinal, variant| {
                        Field::Inventory(seller, ordinal, variant)
                    });
                    for level in row.prices.iter() {
                        named.push(level.variant);
                        observed.insert(
                            keyed.key(Field::OfferLevel(seller, level.variant)),
                            i128::from(level.level),
                        );
                    }
                }
                if orphaned {
                    unknown(keyed, observed, 0);
                }
            }
            Err(error) => keyed.refuse(format!("market inventory unavailable: {error}")),
        }
        match frame.complete::<tables::MarketContractTerms>() {
            Ok(rows) => {
                known(keyed, observed, 1);
                let mut orphaned = false;
                for row in rows {
                    let Some(seller) = slot(keyed, row.country) else {
                        orphaned = true;
                        continue;
                    };
                    let (id, kind) = (row.id, row.kind);
                    let buyer = match slot(keyed, row.buyer) {
                        Some(buyer) => (Field::ContractBuyer(seller, id, kind), i128::from(buyer)),
                        None => (Field::ContractUnidentified(seller, id, kind), 1),
                    };
                    observed.insert(keyed.key(buyer.0), buyer.1);
                    observed.insert(keyed.key(Field::ContractSpeed(seller, id, kind)), i128::from(row.speed));
                    for (which, items) in [(0, &row.equipment), (1, &row.prices)] {
                        pool(keyed, observed, named, items, &|ordinal, variant| {
                            Field::ContractPool(seller, id, kind, which, ordinal, variant)
                        });
                    }
                    for level in row.price_levels.iter() {
                        named.push(level.variant);
                        observed.insert(
                            keyed.key(Field::ContractLevel(seller, id, kind, level.variant)),
                            i128::from(level.level),
                        );
                    }
                }
                if orphaned {
                    unknown(keyed, observed, 1);
                }
            }
            Err(error) => keyed.refuse(format!("market contract terms unavailable: {error}")),
        }
        match frame.complete::<tables::LendLeaseAgreements>() {
            Ok(rows) => {
                known(keyed, observed, 2);
                let mut orphaned = false;
                for row in rows {
                    let Some(giver) = slot(keyed, row.country) else {
                        orphaned = true;
                        continue;
                    };
                    let id = row.id;
                    let receiver = match slot(keyed, row.receiver) {
                        Some(receiver) => (Field::LeaseReceiver(giver, id), i128::from(receiver)),
                        None => (Field::LeaseUnidentified(giver, id), 1),
                    };
                    observed.insert(keyed.key(receiver.0), receiver.1);
                    for (field, value) in [
                        (Field::LeaseRevision(giver, id), i128::from(row.revision)),
                        (Field::LeaseSince(giver, id), i128::from(row.active_since)),
                        (Field::LeaseLast(giver, id), i128::from(row.last_delivery)),
                        (Field::LeaseFuel(giver, id, 0), i128::from(row.fuel_daily_raw)),
                        (Field::LeaseFuel(giver, id, 1), i128::from(row.fuel_percentage_raw)),
                        (Field::LeaseFuel(giver, id, 2), i128::from(row.fuel_sent_raw)),
                        (Field::LeaseFuel(giver, id, 3), i128::from(row.fuel_sunk_raw)),
                        (Field::LeaseFuel(giver, id, 4), i128::from(row.last_fuel_delivered_raw)),
                    ] {
                        observed.insert(keyed.key(field), value);
                    }
                    for (which, items) in
                        [&row.monthly, &row.percentage, &row.once, &row.delivered, &row.sunk]
                            .into_iter()
                            .enumerate()
                    {
                        pool(keyed, observed, named, items, &|ordinal, variant| {
                            Field::LeasePool(giver, id, which as u8, ordinal, variant)
                        });
                    }
                }
                if orphaned {
                    unknown(keyed, observed, 2);
                }
            }
            Err(error) => keyed.refuse(format!("lend-lease agreements unavailable: {error}")),
        }
        match frame.complete::<tables::PurchaseContracts>() {
            Ok(rows) => {
                known(keyed, observed, 3);
                for row in rows {
                    let (seller, buyer) = (slot(keyed, row.seller), slot(keyed, row.buyer));
                    for (which, value) in [
                        (0, seller.map(i128::from)),
                        (1, buyer.map(i128::from)),
                        (2, Some(i128::from(row.convoys_assigned))),
                        (3, Some(i128::from(row.convoys_total))),
                        (4, Some(i128::from(row.collected_raw))),
                        (5, Some(i128::from(row.completed_deliveries))),
                    ] {
                        if let Some(value) = value {
                            observed.insert(keyed.key(Field::Purchase(row.id, which)), value);
                        }
                    }
                }
            }
            Err(error) => keyed.refuse(format!("purchase contracts unavailable: {error}")),
        }
    }

    /// The hour's new occurrences, each kept once at the read that supplied it;
    /// without `tags` none has a party.
    fn observe_events(
        &mut self,
        frame: &DecodedFrame,
        tags: Option<&Tags<'_>>,
        observed: &mut Observed,
        named: &mut Vec<u64>,
    ) {
        let hour = frame.hour;
        let party = |keyed: &mut Keyed<Field, Extra>, raw: u32| {
            let slot = tags.and_then(|tags| tags.slot(raw));
            if slot.is_none() {
                keyed.refuse("a market occurrence's party has no identity");
            }
            slot
        };
        let keyed = &mut self.keyed;
        let mut seen = BTreeSet::new();
        match frame.complete::<tables::MarketEvents>() {
            Ok(rows) => {
                observed.insert(keyed.key(Field::Known(MARKET_EVENTS)), 1);
                let mut identified = true;
                for row in rows {
                    seen.insert(row.id);
                    if self.last_events.contains(&row.id) || !ledger_kind(row) {
                        continue;
                    }
                    let (seller, buyer) = (party(keyed, row.country), party(keyed, row.receiver));
                    // A chosen country's absence also needs its parties' tags at the occurrence's own hour.
                    let spans = &keyed.meta.extra.identified;
                    let proved = spans
                        .get(spans.partition_point(|span| span.0 <= row.hour).wrapping_sub(1))
                        .is_some_and(|span| row.hour <= span.1);
                    identified &= seller.is_some() && buyer.is_some() && proved;
                    if row.variant != 0 {
                        named.push(row.variant);
                    }
                    let entry = ledger_row(row, seller, buyer);
                    let at = row.hour.max(hour);
                    let (kind, sequence) = (row.kind, self.sequence);
                    let stream = self.streams.entry(kind).or_default();
                    let position = match append(
                        keyed,
                        stream,
                        |chunk, first| Field::Stream(kind, chunk, first),
                        at,
                        &(sequence, row.hour, &entry),
                    ) {
                        Ok(position) => position,
                        Err(error) => {
                            keyed.refuse(format!("a market occurrence could not be kept: {error}"));
                            continue;
                        }
                    };
                    self.sequence += 1;
                    let lag = i32::try_from(i64::from(at) - i64::from(row.hour)).unwrap_or(i32::MAX);
                    if lag > keyed.meta.extra.lag {
                        keyed.meta.extra.lag = lag;
                        keyed.note_change(16);
                    }
                    let parties = [seller, buyer.filter(|buyer| Some(*buyer) != seller)];
                    for holder in parties.into_iter().flatten() {
                        let index = self.indexes.entry(holder).or_default().entry(kind).or_default();
                        if let Err(error) = append(
                            keyed,
                            index,
                            |chunk, first| Field::Party(holder, kind, chunk, first),
                            at,
                            &(sequence, row.hour, position),
                        ) {
                            keyed.refuse(format!("a market occurrence's party index could not be kept: {error}"));
                        }
                    }
                }
                if identified {
                    observed.insert(keyed.key(Field::Identified), 1);
                }
            }
            Err(error) => keyed.refuse(format!("market occurrences unavailable: {error}")),
        }
        self.last_events = seen;
    }

    /// Bytes the next commit publishes and releases.
    pub fn bytes(&self) -> usize {
        self.keyed.pending()
    }

    pub fn flush(&mut self, writer: &mut store::Writer, branch: u64, cancelled: &AtomicBool) -> io::Result<()> {
        self.keyed.meta.extra.variants = self
            .named
            .iter()
            .filter_map(|&id| Some((id, self.variants.versions.get(&id)?.clone())))
            .collect();
        self.keyed.flush(writer, super::FAMILY, branch, cancelled)
    }
}

/// Each entry of an ordered pool under the key `field` names.
fn pool(
    keyed: &mut Keyed<Field, Extra>,
    observed: &mut Observed,
    named: &mut Vec<u64>,
    pool: &MarketPool,
    field: &dyn Fn(u32, u64) -> Field,
) {
    for (ordinal, item) in pool.items.iter().enumerate() {
        named.push(item.variant);
        observed.insert(
            keyed.key(field(ordinal as u32, item.variant)),
            i128::from(item.amount_raw),
        );
    }
}

impl Read for Published<'_, Field, Extra> {
    fn fields(&self) -> Cow<'_, [(u64, Field)]> {
        Cow::Owned(self.meta.fields.iter().map(|(&key, &field)| (key, field)).collect())
    }

    fn check(&self) -> io::Result<()> {
        Published::check(self)
    }

    fn runs(&mut self, key: u64, spans: &[(i32, i32)]) -> io::Result<Vec<Run>> {
        Published::runs(self, key, spans)
    }

    fn text(&mut self, key: u64, spans: &[(i32, i32)]) -> io::Result<Vec<EconomyRun>> {
        Published::text(self, key, spans)
    }

    fn tags(&self, slot: u32) -> Vec<HolderTag> {
        Published::tags(self, slot)
    }

    fn holder_spans(&self, tags: &[String]) -> BTreeMap<u32, Vec<(i32, i32)>> {
        Published::holder_spans(self, tags)
    }

    fn warnings(&self) -> Vec<String> {
        Published::warnings(self, "Market")
    }

    fn variants(&self) -> &BTreeMap<u64, Vec<VariantVersion>> {
        &self.meta.extra.variants
    }

    fn lag(&self) -> i32 {
        self.meta.extra.lag
    }

    fn identified(&self) -> &[(i32, i32)] {
        &self.meta.extra.identified
    }

    fn supplied(&mut self, key: u64, until: i32) -> io::Result<u64> {
        Ok(self.reader.events(&[key], i32::MIN, until, 0, 1)?.total)
    }

    fn entry(&mut self, key: u64, ordinal: u64, index: bool) -> io::Result<(i32, u64, i32, Payload)> {
        let page = self.reader.events(&[key], i32::MIN, i32::MAX, ordinal, 1)?;
        let row = page
            .rows
            .into_iter()
            .next()
            .ok_or_else(|| invalid("A market occurrence chain is shorter than its count"))?;
        let (sequence, hour, payload): (u64, i32, Value) = serde_json::from_slice(&row.row)?;
        let payload = if index {
            Payload::Position(serde_json::from_value(payload)?)
        } else {
            Payload::Row(Box::new(serde_json::from_value(payload)?))
        };
        Ok((row.hour, sequence, hour, payload))
    }
}

/// A view's history from the row preparation in `store`.
pub fn history(
    store: &store::Store,
    key: &store::SourceKey,
    params: &MarketHistoryParams,
    through: Option<i32>,
    texts: Option<&dyn Texts>,
    warnings: Vec<String>,
    cancelled: &AtomicBool,
) -> io::Result<MarketHistoryReply> {
    let branch = branch_of(&params.branch_id)?;
    let mut published = match through {
        Some(through) => {
            Published::<Field, Extra>::open(store, key, super::FAMILY, branch, through, cancelled)?
        }
        None => None,
    };
    let read = published.as_mut().map(|published| published as &mut dyn Read);
    reply::history(read, params, through, texts, warnings)
}

/// A ledger page from the row preparation in `store`.
pub fn ledger(
    store: &store::Store,
    key: &store::SourceKey,
    params: &MarketLedgerParams,
    through: Option<i32>,
    texts: Option<&dyn Texts>,
    warnings: Vec<String>,
    cancelled: &AtomicBool,
) -> io::Result<MarketLedgerReply> {
    let branch = branch_of(&params.branch_id)?;
    let mut published = match through {
        Some(through) => {
            Published::<Field, Extra>::open(store, key, super::FAMILY, branch, through, cancelled)?
        }
        None => None,
    };
    let read = published.as_mut().map(|published| published as &mut dyn Read);
    reply::ledger(read, params, through, texts, warnings)
}

#[cfg(test)]
#[path = "row_tests.rs"]
mod tests;
