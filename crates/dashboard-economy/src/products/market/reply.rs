//! The market's replies, read from a publication of its keys ([`Read`]): the series publication
//! the segment driver prepares ([`super::query`]), or the row preparation kept as its oracle.

use super::{
    AgreementHistory, CHUNK, ContractHistory, Field, LEDGER_KINDS, MARKET_EVENTS,
    MarketHistoryParams, MarketHistoryReply, MarketLedgerParams, MarketLedgerReply,
    MarketLedgerRow, MarketPoolItem, MarketView, OfferHistory,
};
use crate::products::keyed::{EconomyRun, HolderTag, clip_text, selected_spans};
use dashboard_base::{
    invalid,
    prepared::series::Run,
    products::{
        country_groups::HourSpan,
        equipment::{StockVariantName, Texts, VariantCatalogue, VariantVersion},
    },
};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    io,
};

/// A chain entry's payload: a stream's occurrence, or an index's position in its stream.
#[derive(Clone, Debug)]
pub(crate) enum Payload {
    Row(Box<MarketLedgerRow>),
    Position(u64),
}

/// A publication of the market's keys, as its replies read it.
pub(crate) trait Read {
    /// Every key the family named, with its field, by key.
    fn fields(&self) -> Cow<'_, [(u64, Field)]>;
    /// Fails once the request is cancelled.
    fn check(&self) -> io::Result<()>;
    /// The key's runs through the publication's last hour read, clipped to `spans`.
    fn runs(&mut self, key: u64, spans: &[(i32, i32)]) -> io::Result<Vec<Run>>;
    /// [`Self::runs`] as exact decimal text.
    fn text(&mut self, key: u64, spans: &[(i32, i32)]) -> io::Result<Vec<EconomyRun>>;
    /// The holder's tag runs.
    fn tags(&self, slot: u32) -> Vec<HolderTag>;
    /// The hours each holder had one of `tags`.
    fn holder_spans(&self, tags: &[String]) -> BTreeMap<u32, Vec<(i32, i32)>>;
    /// The refusals as warning lines.
    fn warnings(&self) -> Vec<String>;
    /// Each variant the market named, with its naming reads.
    fn variants(&self) -> &BTreeMap<u64, Vec<VariantVersion>>;
    /// The most hours an occurrence was supplied after its own hour.
    fn lag(&self) -> i32;
    /// The hours whose country identities were read, merged.
    fn identified(&self) -> &[(i32, i32)];
    /// The entries of chain key `key` supplied through `until`.
    fn supplied(&mut self, key: u64, until: i32) -> io::Result<u64>;
    /// The entry at `ordinal` of chain key `key` (of an index when `index`): its supplying read,
    /// sequence, own hour and payload.
    fn entry(&mut self, key: u64, ordinal: u64, index: bool)
    -> io::Result<(i32, u64, i32, Payload)>;
}

type Gathered = BTreeMap<(u32, u64, u8), Vec<(u64, Field)>>;

/// A view's history, from `read` when the family published the branch.
pub(crate) fn history(
    read: Option<&mut dyn Read>,
    params: &MarketHistoryParams,
    through: Option<i32>,
    texts: Option<&dyn Texts>,
    mut warnings: Vec<String>,
) -> io::Result<MarketHistoryReply> {
    let mut reply = MarketHistoryReply {
        branch_id: params.branch_id.clone(),
        view: params.view,
        last_hour: through,
        known: Vec::new(),
        contracts: Vec::new(),
        offers: Vec::new(),
        agreements: Vec::new(),
        holders: BTreeMap::new(),
        variants: BTreeMap::new(),
        warnings: Vec::new(),
    };
    let (Some(_), Some(published)) = (through, read) else {
        reply.warnings = warnings;
        return Ok(reply);
    };
    let chosen: Option<BTreeMap<u32, Vec<(i32, i32)>>> =
        (!params.tags.is_empty()).then(|| published.holder_spans(&params.tags));
    let all = [(i32::MIN, i32::MAX)];
    let table = match params.view {
        MarketView::Contracts | MarketView::Requests => 1,
        MarketView::Inventory => 0,
        MarketView::Agreements => 2,
    };
    // Group every key under its owner (holder, handle, kind).
    let mut owners = Gathered::new();
    let mut purchases = BTreeMap::<u64, Vec<(u8, u64)>>::new();
    let mut known = None;
    for &(id, field) in published.fields().iter() {
        let owner = match (field, params.view) {
            (Field::Known(held), _) if held == table => {
                known = Some(id);
                continue;
            }
            (Field::Purchase(handle, which), MarketView::Contracts) => {
                purchases.entry(handle).or_default().push((which, id));
                continue;
            }
            (
                Field::ContractBuyer(seller, handle, kind)
                | Field::ContractUnidentified(seller, handle, kind)
                | Field::ContractSpeed(seller, handle, kind)
                | Field::ContractPool(seller, handle, kind, ..)
                | Field::ContractLevel(seller, handle, kind, _),
                MarketView::Contracts | MarketView::Requests,
            ) if (kind == 1) == (params.view == MarketView::Contracts) => (seller, handle, kind),
            (Field::Inventory(seller, ..) | Field::OfferLevel(seller, _), MarketView::Inventory) => {
                (seller, 0, 0)
            }
            (
                Field::LeaseReceiver(giver, handle)
                | Field::LeaseUnidentified(giver, handle)
                | Field::LeaseRevision(giver, handle)
                | Field::LeaseSince(giver, handle)
                | Field::LeaseLast(giver, handle)
                | Field::LeasePool(giver, handle, ..)
                | Field::LeaseFuel(giver, handle, _),
                MarketView::Agreements,
            ) => (giver, handle, 0),
            _ => continue,
        };
        owners.entry(owner).or_default().push((id, field));
    }
    if let Some(id) = known {
        reply.known = published.text(id, &all)?;
    }
    let mut named = BTreeSet::new();
    let mut holders = BTreeSet::new();
    let item = |published: &mut dyn Read,
                named: &mut BTreeSet<u64>,
                spans: &[(i32, i32)],
                id: u64,
                pool: u8,
                ordinal: u32,
                variant: u64|
     -> io::Result<MarketPoolItem> {
        named.insert(variant);
        Ok(MarketPoolItem {
            pool,
            ordinal,
            variant: format!("{variant:x}"),
            amounts: published.text(id, spans)?,
        })
    };
    let party = |runs: &[EconomyRun]| -> Vec<u32> {
        runs.iter().filter_map(|run| run.2.parse().ok()).collect()
    };
    // The other party's runs, read first so a row no chosen country is party
    // to is skipped before its pools are read; with a selection, also the
    // hours that party had no identity.
    let counterpart = |published: &mut dyn Read,
                       fields: &[(u64, Field)],
                       party: fn(&Field) -> bool,
                       unidentified: fn(&Field) -> bool|
     -> io::Result<(Vec<EconomyRun>, Vec<EconomyRun>)> {
        let mut read = |wanted: fn(&Field) -> bool| match fields.iter().find(|(_, field)| wanted(field)) {
            Some(&(id, _)) => published.text(id, &all),
            None => Ok(Vec::new()),
        };
        let runs = read(party)?;
        let unknown = if chosen.is_some() { read(unidentified)? } else { Vec::new() };
        Ok((runs, unknown))
    };
    for ((owner, handle, _kind), fields) in owners {
        published.check()?;
        match params.view {
            MarketView::Contracts | MarketView::Requests => {
                let (buyer, unidentified) = counterpart(
                    &mut *published,
                    &fields,
                    |field| matches!(field, Field::ContractBuyer(..)),
                    |field| matches!(field, Field::ContractUnidentified(..)),
                )?;
                let spans = selected_spans(chosen.as_ref(), &[owner], &buyer, &unidentified);
                if spans.is_empty() {
                    continue;
                }
                let buyer = clip_text(&buyer, &spans);
                let buyers = party(&buyer);
                let mut buyer_runs = Vec::new();
                let mut contract = ContractHistory {
                    seller: owner.to_string(),
                    id: format!("{handle:016x}"),
                    buyer,
                    speed: Vec::new(),
                    items: Vec::new(),
                    levels: Vec::new(),
                    purchase: Vec::new(),
                };
                for (id, field) in fields {
                    match field {
                        Field::ContractBuyer(..) => buyer_runs = published.runs(id, &spans)?,
                        Field::ContractSpeed(..) => contract.speed = published.text(id, &spans)?,
                        Field::ContractPool(_, _, _, pool, ordinal, variant) => contract
                            .items
                            .push(item(&mut *published, &mut named, &spans, id, pool, ordinal, variant)?),
                        Field::ContractLevel(_, _, _, variant) => contract
                            .levels
                            .push(item(&mut *published, &mut named, &spans, id, 2, 0, variant)?),
                        _ => {}
                    }
                }
                if let Some(keys) = purchases.get(&handle) {
                    contract.purchase = join_purchase(&mut *published, keys, owner, &buyer_runs)?;
                }
                holders.insert(owner);
                holders.extend(buyers);
                reply.contracts.push(contract);
            }
            MarketView::Inventory => {
                let spans = selected_spans(chosen.as_ref(), &[owner], &[], &[]);
                if spans.is_empty() {
                    continue;
                }
                let mut offer = OfferHistory {
                    seller: owner.to_string(),
                    inventory: Vec::new(),
                    levels: Vec::new(),
                };
                for (id, field) in fields {
                    match field {
                        Field::Inventory(_, ordinal, variant) => offer
                            .inventory
                            .push(item(&mut *published, &mut named, &spans, id, 0, ordinal, variant)?),
                        Field::OfferLevel(_, variant) => offer
                            .levels
                            .push(item(&mut *published, &mut named, &spans, id, 2, 0, variant)?),
                        _ => {}
                    }
                }
                holders.insert(owner);
                reply.offers.push(offer);
            }
            MarketView::Agreements => {
                let (receiver, unidentified) = counterpart(
                    &mut *published,
                    &fields,
                    |field| matches!(field, Field::LeaseReceiver(..)),
                    |field| matches!(field, Field::LeaseUnidentified(..)),
                )?;
                let spans = selected_spans(chosen.as_ref(), &[owner], &receiver, &unidentified);
                if spans.is_empty() {
                    continue;
                }
                let receiver = clip_text(&receiver, &spans);
                let receivers = party(&receiver);
                let mut agreement = AgreementHistory {
                    giver: owner.to_string(),
                    id: format!("{handle:016x}"),
                    receiver,
                    revision: Vec::new(),
                    since: Vec::new(),
                    last: Vec::new(),
                    items: Vec::new(),
                    fuel: vec![Vec::new(); 5],
                };
                for (id, field) in fields {
                    match field {
                        Field::LeaseRevision(..) => agreement.revision = published.text(id, &spans)?,
                        Field::LeaseSince(..) => agreement.since = published.text(id, &spans)?,
                        Field::LeaseLast(..) => agreement.last = published.text(id, &spans)?,
                        Field::LeaseFuel(_, _, which) if which < 5 => {
                            agreement.fuel[usize::from(which)] = published.text(id, &spans)?;
                        }
                        Field::LeasePool(_, _, pool, ordinal, variant) => agreement
                            .items
                            .push(item(&mut *published, &mut named, &spans, id, pool, ordinal, variant)?),
                        _ => {}
                    }
                }
                holders.insert(owner);
                holders.extend(receivers);
                reply.agreements.push(agreement);
            }
        }
    }
    for list in reply
        .contracts
        .iter_mut()
        .flat_map(|row| [&mut row.items, &mut row.levels])
        .chain(reply.offers.iter_mut().flat_map(|row| [&mut row.inventory, &mut row.levels]))
        .chain(reply.agreements.iter_mut().map(|row| &mut row.items))
    {
        list.sort_by_key(|item| (item.pool, item.ordinal));
    }
    reply.holders = holders
        .into_iter()
        .map(|slot| (slot.to_string(), published.tags(slot)))
        .collect();
    reply.variants = variant_names(published.variants(), &named, texts, &mut warnings);
    warnings.extend(published.warnings());
    reply.warnings = warnings;
    Ok(reply)
}

/// The purchase contract's runs while its parties match the contract's.
fn join_purchase(
    published: &mut dyn Read,
    keys: &[(u8, u64)],
    seller: u32,
    buyers: &[Run],
) -> io::Result<Vec<Vec<EconomyRun>>> {
    let all = [(i32::MIN, i32::MAX)];
    let mut parties = [Vec::new(), Vec::new()];
    let mut values = vec![Vec::new(); 4];
    for &(which, id) in keys {
        match which {
            0 | 1 => parties[usize::from(which)] = published.runs(id, &all)?,
            2..=5 => values[usize::from(which - 2)] = published.text(id, &all)?,
            _ => {}
        }
    }
    // Hours the purchase records the contract's seller and the buyer the
    // contract itself records at that same hour: a buyer from another stretch
    // of the contract's history never joins.
    let matching: Vec<(i32, i32)> = parties[0]
        .iter()
        .filter(|run| run.value == i128::from(seller))
        .flat_map(|sold| {
            parties[1].iter().flat_map(move |bought| {
                buyers.iter().filter_map(move |held| {
                    let first = sold.first_hour.max(bought.first_hour).max(held.first_hour);
                    let last = sold.last_hour.min(bought.last_hour).min(held.last_hour);
                    (first <= last && bought.value == held.value).then_some((first, last))
                })
            })
        })
        .collect();
    Ok(values
        .into_iter()
        .map(|runs| {
            runs.into_iter()
                .flat_map(|EconomyRun(first, last, value)| {
                    matching.iter().filter_map(move |&(from, to)| {
                        let (first, last) = (first.max(from), last.min(to));
                        (first <= last).then(|| EconomyRun(first, last, value.clone()))
                    })
                })
                .collect()
        })
        .collect())
}

/// Each named variant's names at each read, and a warning counting those never named.
pub(crate) fn variant_names(
    variants: &BTreeMap<u64, Vec<VariantVersion>>,
    named: &BTreeSet<u64>,
    texts: Option<&dyn Texts>,
    warnings: &mut Vec<String>,
) -> BTreeMap<String, Vec<StockVariantName>> {
    let catalogue = VariantCatalogue::seeded(
        variants
            .iter()
            .filter(|(id, _)| named.contains(id))
            .map(|(&id, versions)| (id, versions.clone()))
            .collect(),
    );
    let mut missing = 0_usize;
    let names = named
        .iter()
        .map(|&id| {
            let names = catalogue.names(id, texts);
            if names.iter().all(|name| name.name.is_none()) {
                missing += 1;
            }
            (format!("{id:x}"), names)
        })
        .collect();
    if missing > 0 {
        warnings.push(format!("Equipment names unavailable for {missing} market variants"));
    }
    names
}

/// A ledger page, from `read` when the family published the branch.
pub(crate) fn ledger(
    read: Option<&mut dyn Read>,
    params: &MarketLedgerParams,
    through: Option<i32>,
    texts: Option<&dyn Texts>,
    mut warnings: Vec<String>,
) -> io::Result<MarketLedgerReply> {
    let mut reply = MarketLedgerReply {
        branch_id: params.branch_id.clone(),
        last_hour: through,
        rows: Vec::new(),
        total: None,
        next: None,
        known: Vec::new(),
        holders: BTreeMap::new(),
        identified: Vec::new(),
        variants: BTreeMap::new(),
        warnings: Vec::new(),
    };
    let (Some(through), Some(published)) = (through, read) else {
        reply.warnings = warnings;
        return Ok(reply);
    };
    let kinds: BTreeSet<u8> = if params.kinds.is_empty() {
        LEDGER_KINDS.into_iter().collect()
    } else {
        params.kinds.iter().copied().collect()
    };
    // Each chosen country's holders with the hours they had one of its tags
    // (and, for a group's member, were a member): an occurrence matches by a
    // party's tag at its own hour.
    let spans = (!params.tags.is_empty()
        || params.members.as_ref().is_some_and(|members| !members.is_empty()))
    .then(|| chosen_spans(&*published, params));
    // Each chain's keys: a kind's stream, and each holder's index of a kind.
    let mut streams = BTreeMap::<u8, Vec<(u32, i32, u64)>>::new();
    let mut parties = BTreeMap::<u32, BTreeMap<u8, Vec<(u32, i32, u64)>>>::new();
    let (mut known, mut identified) = (None, None);
    for &(id, field) in published.fields().iter() {
        match field {
            Field::Known(MARKET_EVENTS) => known = Some(id),
            Field::Identified => identified = Some(id),
            Field::Stream(kind, chunk, first) if kinds.contains(&kind) => {
                streams.entry(kind).or_default().push((chunk, first, id));
            }
            Field::Party(holder, kind, chunk, first) if kinds.contains(&kind) => {
                parties.entry(holder).or_default().entry(kind).or_default().push((chunk, first, id));
            }
            _ => {}
        }
    }
    if let Some(id) = known {
        reply.known = published.text(id, &[(i32::MIN, i32::MAX)])?;
        // An occurrence without its parties' identities may be a chosen country's.
        if spans.is_some() {
            let read: Vec<(i32, i32)> = reply.known.iter().map(|run| (run.0, run.1)).collect();
            reply.known = match identified {
                Some(id) => published.text(id, &read)?,
                None => Vec::new(),
            };
        }
    }
    let to = params.last_hour.min(through);
    let first = params.first_hour.unwrap_or(i32::MIN);
    if streams.is_empty() {
        reply.total = spans.is_none().then(|| "0".to_owned());
        warnings.extend(published.warnings());
        reply.warnings = warnings;
        return Ok(reply);
    }
    let mut chains = Vec::new();
    let mut stream_of = BTreeMap::new();
    for (kind, chunks) in streams {
        stream_of.insert(kind, chains.len());
        chains.push(Chain::new(chunks, false)?);
    }
    // Every occurrence once: each kind's stream through the hour. Chosen
    // countries: each chosen holder's index of each kind over each span it had
    // a chosen tag. Spans are occurrence hours; `to` gates supplying reads.
    let mut segments = Vec::new();
    match &spans {
        None => segments.extend(stream_of.iter().map(|(&kind, &chain)| (chain, kind, first, to))),
        Some(spans) => {
            for (holder, spans) in spans {
                for (&kind, chunks) in parties.get(holder).into_iter().flatten() {
                    let chain = chains.len();
                    chains.push(Chain::new(chunks.clone(), true)?);
                    for &(from, until) in spans {
                        let (from, until) = (from.max(first), until.min(to));
                        if from <= until {
                            segments.push((chain, kind, from, until));
                        }
                    }
                }
            }
        }
    }
    let lag = published.lag();
    let mut ledger = Ledger::open(&mut *published, chains, segments, to, lag, params.before.as_deref())?;
    let mut taken = Vec::new();
    while taken.len() < params.page_size as usize {
        published.check()?;
        match ledger.next(&mut *published)? {
            Some(entry) => taken.push(entry),
            None => break,
        }
    }
    reply.next = ledger.cursor();
    if spans.is_none() {
        reply.total = Some(ledger.total(&mut *published)?.to_string());
    }
    let mut rows: Vec<MarketLedgerRow> = Vec::with_capacity(taken.len());
    for (kind, payload) in taken {
        let row = match payload {
            Payload::Row(row) => row,
            Payload::Position(position) => {
                let stream = stream_of
                    .get(&kind)
                    .ok_or_else(|| invalid("A market occurrence index names no stream"))?;
                match ledger.chains[*stream].entry(&mut *published, position)?.3 {
                    Payload::Row(row) => row,
                    Payload::Position(_) => {
                        return Err(invalid("A market occurrence stream holds a position"));
                    }
                }
            }
        };
        rows.push(*row);
    }
    let mut named = BTreeSet::new();
    let mut holders = BTreeSet::new();
    for row in &rows {
        if let Ok(variant) = u64::from_str_radix(&row.variant, 16)
            && variant != 0
        {
            named.insert(variant);
        }
        holders.extend(
            [&row.seller, &row.buyer]
                .into_iter()
                .flatten()
                .filter_map(|holder| holder.parse::<u32>().ok()),
        );
    }
    if let (Some(first), Some(last)) = (
        rows.iter().map(|row| row.hour).min(),
        rows.iter().map(|row| row.hour).max(),
    ) {
        reply.identified = intersect(published.identified(), &[(first, last)])
            .into_iter()
            .map(|(first_hour, last_hour)| HourSpan { first_hour, last_hour })
            .collect();
    }
    reply.rows = rows;
    reply.holders = holders
        .into_iter()
        .map(|slot| (slot.to_string(), published.tags(slot)))
        .collect();
    reply.variants = variant_names(published.variants(), &named, texts, &mut warnings);
    warnings.extend(published.warnings());
    reply.warnings = warnings;
    Ok(reply)
}

// Stream entries and party index entries read, counted in tests to bound a
// page's work.
#[cfg(test)]
thread_local! {
    pub(crate) static LEDGER_ROWS_READ: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static LEDGER_INDEX_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A stream or party index: its keys in chunk order, each with its first hour.
struct Chain {
    chunks: Vec<(i32, u64)>,
    /// A party index rather than a stream.
    index: bool,
}

impl Chain {
    fn new(mut chunks: Vec<(u32, i32, u64)>, index: bool) -> io::Result<Self> {
        chunks.sort_unstable();
        if chunks.iter().enumerate().any(|(at, &(chunk, ..))| chunk as usize != at) {
            return Err(invalid("A market occurrence chain misses a key"));
        }
        Ok(Self {
            chunks: chunks.into_iter().map(|(_, first, key)| (first, key)).collect(),
            index,
        })
    }

    /// The chain's name in a cursor: its first key.
    fn name(&self) -> u64 {
        self.chunks[0].1
    }

    /// The position after the chain's last entry through `until`: its count
    /// there, read from the one key whose chunk holds that hour.
    fn end(&self, published: &mut dyn Read, until: i32) -> io::Result<u64> {
        let Some(last) = self.chunks.partition_point(|&(first, _)| first <= until).checked_sub(1) else {
            return Ok(0);
        };
        let held = published.supplied(self.chunks[last].1, until)?;
        self.read();
        Ok(last as u64 * CHUNK + held)
    }

    /// The entry at `position`: its supplying read, sequence, own hour and payload.
    fn entry(&self, published: &mut dyn Read, position: u64) -> io::Result<(i32, u64, i32, Payload)> {
        let key = usize::try_from(position / CHUNK)
            .ok()
            .and_then(|chunk| self.chunks.get(chunk))
            .map(|&(_, key)| key)
            .ok_or_else(|| invalid("A market occurrence position lies past its chain"))?;
        let entry = published.entry(key, position % CHUNK, self.index)?;
        self.read();
        Ok(entry)
    }

    fn read(&self) {
        #[cfg(test)]
        {
            let counter = if self.index { &LEDGER_INDEX_READS } else { &LEDGER_ROWS_READ };
            counter.with(|read| read.set(read.get() + 1));
        }
    }
}

/// A chain's occurrences of the hours `from..=until`, supplied through
/// `through`: the position its unpaged entries end at, and the newest of them.
struct Segment {
    chain: usize,
    kind: u8,
    from: i32,
    until: i32,
    /// The last supplying read that can hold an occurrence of its hours.
    through: i32,
    /// The position after its newest unpaged entry; entries supplied from
    /// `from` up to it remain.
    position: u64,
    /// Its chain's entries supplied through `through`.
    end: u64,
    /// The newest unpaged entry: its sequence and payload.
    head: Option<(u64, Payload)>,
}

/// The segments merged newest first by sequence.
struct Ledger {
    chains: Vec<Chain>,
    segments: Vec<Segment>,
    /// The most hours an occurrence was supplied after its own hour.
    lag: i32,
}

impl Ledger {
    /// Each segment `(chain, kind, from, until)`, supplied through `to`, at
    /// its position from `cursor`, else at its end, with its head. The cursor
    /// names each segment by its chain and first hour; a segment it doesn't
    /// name had no entries left then, or gained its first ones after that page
    /// was read, so they are all newer than it and none remain.
    fn open(
        published: &mut dyn Read,
        chains: Vec<Chain>,
        wanted: Vec<(usize, u8, i32, i32)>,
        to: i32,
        lag: i32,
        cursor: Option<&str>,
    ) -> io::Result<Self> {
        let positions = cursor.map(parse_cursor).transpose()?;
        let mut segments = Vec::with_capacity(wanted.len());
        for (chain, kind, from, until) in wanted {
            let through = until.saturating_add(lag).min(to);
            let end = chains[chain].end(published, through)?;
            let position = match &positions {
                None => end,
                Some(positions) => positions
                    .get(&(chains[chain].name(), from))
                    .copied()
                    .unwrap_or(0)
                    .min(end),
            };
            let mut segment = Segment { chain, kind, from, until, through, position, end, head: None };
            segment.head = Self::head(published, &chains[chain], &mut segment)?;
            segments.push(segment);
        }
        Ok(Self { chains, segments, lag })
    }

    /// The newest entry before `position` that occurred in the segment's
    /// hours, leaving `position` just after it.
    fn head(
        published: &mut dyn Read,
        chain: &Chain,
        segment: &mut Segment,
    ) -> io::Result<Option<(u64, Payload)>> {
        while let Some(last) = segment.position.checked_sub(1) {
            let (supplied, sequence, hour, payload) = chain.entry(published, last)?;
            // Every earlier entry was supplied, so occurred, before `from` too.
            if supplied < segment.from {
                return Ok(None);
            }
            if (segment.from..=segment.until).contains(&hour) {
                return Ok(Some((sequence, payload)));
            }
            segment.position = last;
        }
        Ok(None)
    }

    /// The newest unpaged occurrence's kind and payload, taken from every
    /// segment that holds it.
    fn next(&mut self, published: &mut dyn Read) -> io::Result<Option<(u8, Payload)>> {
        let Some((sequence, kind, payload)) = self
            .segments
            .iter()
            .filter_map(|segment| {
                segment
                    .head
                    .as_ref()
                    .map(|(sequence, payload)| (*sequence, segment.kind, payload))
            })
            .max_by_key(|&(sequence, ..)| sequence)
            .map(|(sequence, kind, payload)| (sequence, kind, payload.clone()))
        else {
            return Ok(None);
        };
        let chains = &self.chains;
        for segment in &mut self.segments {
            if segment.head.as_ref().is_some_and(|(held, _)| *held == sequence) {
                segment.position -= 1;
                segment.head = Self::head(published, &chains[segment.chain], segment)?;
            }
        }
        Ok(Some((kind, payload)))
    }

    /// Every segment's entries that occurred in its hours: the stream
    /// ledger's total. Only entries supplied within the lag after a bound can
    /// have occurred outside it, so only those are read.
    fn total(&self, published: &mut dyn Read) -> io::Result<u64> {
        let mut total = 0;
        for segment in &self.segments {
            let chain = &self.chains[segment.chain];
            let (first, settled) = match segment.from.checked_sub(1) {
                Some(before) => {
                    let first = chain.end(published, before)?;
                    let mixed = before.saturating_add(self.lag).min(segment.until);
                    (first, if mixed == before { first } else { chain.end(published, mixed)? })
                }
                None => (0, 0),
            };
            let supplied = if segment.through == segment.until {
                segment.end
            } else {
                chain.end(published, segment.until)?
            };
            let mut count = segment.end.saturating_sub(first);
            for position in (first..settled).chain(supplied..segment.end) {
                let (_, _, hour, _) = chain.entry(published, position)?;
                if !(segment.from..=segment.until).contains(&hour) {
                    count -= 1;
                }
            }
            total += count;
        }
        Ok(total)
    }

    /// The positions to continue from, or None when every segment is paged.
    fn cursor(&self) -> Option<String> {
        let parts: Vec<String> = self
            .segments
            .iter()
            .filter(|segment| segment.head.is_some())
            .map(|segment| {
                format!(
                    "{}:{}:{}",
                    self.chains[segment.chain].name(),
                    segment.from,
                    segment.position
                )
            })
            .collect();
        (!parts.is_empty()).then(|| parts.join("."))
    }
}

/// Each chosen holder's hours, merged: the hours it held a chosen tag, and the
/// hours it held a member's tag inside that member's own hours.
fn chosen_spans(published: &dyn Read, params: &MarketLedgerParams) -> BTreeMap<u32, Vec<(i32, i32)>> {
    let mut spans = if params.tags.is_empty() {
        BTreeMap::new()
    } else {
        published.holder_spans(&params.tags)
    };
    for member in params.members.iter().flatten() {
        for (holder, held) in published.holder_spans(std::slice::from_ref(&member.tag)) {
            let clipped = held.into_iter().filter_map(|(from, until)| {
                let (from, until) = (from.max(member.first_hour), until.min(member.last_hour));
                (from <= until).then_some((from, until))
            });
            spans.entry(holder).or_default().extend(clipped);
        }
    }
    spans.retain(|_, held| {
        held.sort_unstable();
        let mut merged: Vec<(i32, i32)> = Vec::with_capacity(held.len());
        for &(from, until) in held.iter() {
            match merged.last_mut() {
                Some(last) if from <= last.1.saturating_add(1) => last.1 = last.1.max(until),
                _ => merged.push((from, until)),
            }
        }
        // A tag proves membership only at hours whose identities were read.
        *held = intersect(&merged, published.identified());
        !held.is_empty()
    });
    spans
}

/// The hours both sorted, disjoint span lists cover.
pub(crate) fn intersect(a: &[(i32, i32)], b: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let (mut out, mut i, mut j) = (Vec::new(), 0, 0);
    while let (Some(&(a_from, a_until)), Some(&(b_from, b_until))) = (a.get(i), b.get(j)) {
        let (from, until) = (a_from.max(b_from), a_until.min(b_until));
        if from <= until {
            out.push((from, until));
        }
        if a_until < b_until { i += 1 } else { j += 1 }
    }
    out
}

/// A cursor: each segment's (chain, first hour) and the position its unpaged
/// entries end at.
pub(crate) fn parse_cursor(cursor: &str) -> io::Result<BTreeMap<(u64, i32), u64>> {
    cursor
        .split('.')
        .map(|part| {
            let mut fields = part.splitn(3, ':');
            let key = fields.next().and_then(|key| key.parse::<u64>().ok());
            let from = fields.next().and_then(|from| from.parse::<i32>().ok());
            let position = fields.next().and_then(|position| position.parse::<u64>().ok());
            match (key, from, position) {
                (Some(key), Some(from), Some(position)) => Ok(((key, from), position)),
                _ => Err(invalid("Invalid market ledger cursor")),
            }
        })
        .collect()
}
