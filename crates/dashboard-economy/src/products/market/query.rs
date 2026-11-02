//! The market's replies, read from the family's series publication ([`super::emit`]): a view's
//! history and a ledger page, through the readers both preparations share ([`super::reply`]).

use super::{
    Field, MarketHistoryParams, MarketHistoryReply, MarketLedgerParams, MarketLedgerReply,
    emit::{IDENTIFIED, LAG, REFUSALS, TAGS, VALUE, VARIANTS, kind, versions},
    ledger_row,
    reply::{self, Payload, Read},
};
use crate::products::keyed::{EconomyRun, HolderTag, clip, holder_tags, tag_spans};
use dashboard_base::{
    prepared::{engine, series::Run},
    products::equipment::{Texts, VariantVersion},
};
use dashboard_series::{
    Cell, EventOrder, EventQuery, PageRead, Selection, Snapshot, Span,
    codec::Binary,
    family::{Directory, Scalar},
    query::EventView,
};
use replay_data::rows::MarketEventRow;
use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap},
    io,
    sync::atomic::{AtomicBool, Ordering},
};

/// Bytes one event page of a chain key reads at most.
const PAGE: usize = 1 << 20;

fn invalid(message: &'static str) -> io::Error {
    dashboard_core::invalid(message)
}

/// What a publication's directory says of the market, read once per publication: each key's
/// field, by key, each holder's tags, the refusals, the lag, the hours whose identities were
/// read and each named variant's naming reads.
#[derive(Debug, Default)]
pub struct Catalogue {
    fields: Vec<(u64, Field)>,
    tags: BTreeMap<u32, Vec<(i32, String)>>,
    refusals: BTreeMap<String, u64>,
    lag: i32,
    identified: Vec<(i32, i32)>,
    variants: BTreeMap<u64, Vec<VariantVersion>>,
    bytes: usize,
}

impl Catalogue {
    pub fn read(directory: &Directory) -> io::Result<Self> {
        let malformed = || invalid("Prepared market catalogue is not valid");
        let mut catalogue = Self::default();
        for (actor, named) in directory.actors.iter().enumerate() {
            let field: Field = serde_json::from_slice(&named.name).map_err(|_| malformed())?;
            let key = directory
                .key(actor as u64, VALUE, None)
                .ok_or_else(malformed)?;
            catalogue.fields.push((key, field));
        }
        catalogue.fields.sort_unstable_by_key(|&(key, _)| key);
        let integer = |scalar: &Scalar| scalar.integer().ok_or_else(malformed);
        let hour = |scalar: &Scalar| {
            integer(scalar).and_then(|hour| i32::try_from(hour).map_err(|_| malformed()))
        };
        let text = |scalar: &Scalar| {
            scalar
                .text()
                .and_then(|text| std::str::from_utf8(text).ok())
                .map(str::to_owned)
                .ok_or_else(malformed)
        };
        for pair in directory.scalar(REFUSALS).map_or(&[][..], Scalar::list) {
            let [reason, hours] = pair.list() else {
                return Err(malformed());
            };
            let hours = u64::try_from(integer(hours)?).map_err(|_| malformed())?;
            catalogue.refusals.insert(text(reason)?, hours);
        }
        if let Some(lag) = directory.scalar(LAG) {
            catalogue.lag = hour(lag)?;
        }
        for span in directory.scalar(IDENTIFIED).map_or(&[][..], Scalar::list) {
            let [first, last] = span.list() else {
                return Err(malformed());
            };
            catalogue.identified.push((hour(first)?, hour(last)?));
        }
        let mut ordinal = 0;
        while let Some(holder) = directory.scalar(TAGS + ordinal) {
            let [slot, runs] = holder.list() else {
                return Err(malformed());
            };
            let slot = u32::try_from(integer(slot)?).map_err(|_| malformed())?;
            let runs = runs
                .list()
                .iter()
                .map(|run| match run.list() {
                    [at, tag] => Ok((hour(at)?, text(tag)?)),
                    _ => Err(malformed()),
                })
                .collect::<io::Result<Vec<_>>>()?;
            catalogue.bytes += runs.iter().map(|(_, tag)| tag.len() + 32).sum::<usize>();
            catalogue.tags.insert(slot, runs);
            ordinal += 1;
        }
        let mut ordinal = 0;
        while let Some(named) = directory.scalar(VARIANTS + ordinal) {
            let [variant, spelled] = named.list() else {
                return Err(malformed());
            };
            let variant = u64::try_from(integer(variant)?).map_err(|_| malformed())?;
            let spelled = spelled.text().ok_or_else(malformed)?;
            catalogue.bytes += spelled.len() * 2;
            catalogue.variants.insert(variant, versions(spelled)?);
            ordinal += 1;
        }
        catalogue.bytes += catalogue.fields.len() * std::mem::size_of::<(u64, Field)>()
            + catalogue
                .refusals
                .keys()
                .map(|reason| reason.len() + 32)
                .sum::<usize>();
        Ok(catalogue)
    }

    /// The bytes it holds, for the cache keeping it.
    pub fn bytes(&self) -> usize {
        self.bytes + std::mem::size_of::<Self>()
    }
}

/// A chain key's entries, as [`Read::entry`] gives them.
type Entries = Vec<(i32, u64, i32, Payload)>;

/// The publication pinned for one reply, as the shared readers read it.
struct Series<'s, 'a, P> {
    snapshot: &'s mut Snapshot<'a, P>,
    catalogue: &'s Catalogue,
    /// The last hour a reply reads: its prefix's, within what the publication accepted.
    through: i32,
    first: i32,
    cancelled: &'s AtomicBool,
    /// Each chain key's entries, decoded at its first entry read.
    entries: HashMap<u64, Entries>,
}

impl<P: PageRead> Series<'_, '_, P> {
    fn runs_of(&mut self, key: u64) -> io::Result<Vec<Run>> {
        if self.first > self.through {
            return Ok(Vec::new());
        }
        Ok(engine::runs(self.snapshot, key, self.first, self.through)?
            .into_iter()
            .map(|run| Run::new(run.span.first, run.span.last, run.value, run.break_before))
            .collect())
    }

    fn decode(&mut self, key: u64, index: bool) -> io::Result<Entries> {
        let selection = [Selection {
            key,
            membership: None,
        }];
        let query = EventQuery {
            selection: &selection,
            native: Span {
                first: i32::MIN,
                last: i32::MAX,
            },
            as_of: None,
            order: EventOrder::Native,
        };
        let mut events = Vec::new();
        let mut cursor = None;
        loop {
            let page = self.snapshot.event_page(query, cursor, PAGE, |view| {
                events.push(view);
                Ok(())
            })?;
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        let malformed = || invalid("Prepared market occurrence is not valid");
        let mut entries = Vec::with_capacity(events.len());
        for view in events {
            let EventView::Family(event) = view else {
                return Err(malformed());
            };
            let cells = event.fields.len / Cell::SIZE as u64;
            let mut field = |at: u64| self.snapshot.field(event.fields, at);
            let unsigned = |cell: Cell| match cell {
                Cell::Unsigned(value) => Ok(value),
                _ => Err(malformed()),
            };
            let integer = |cell: Cell| match cell {
                Cell::Integer(value) => Ok(value),
                _ => Err(malformed()),
            };
            let sequence = unsigned(field(0)?)?;
            let hour = i32::try_from(integer(field(1)?)?).map_err(|_| malformed())?;
            let payload = if index {
                if event.kind != kind::INDEX || cells != 3 {
                    return Err(malformed());
                }
                Payload::Position(unsigned(field(2)?)?)
            } else {
                if event.kind != kind::OCCURRENCE || cells != 13 {
                    return Err(malformed());
                }
                let party = |cell: Cell| match cell {
                    Cell::Null => Ok(None),
                    Cell::Unsigned(slot) => u32::try_from(slot).map(Some).map_err(|_| malformed()),
                    _ => Err(malformed()),
                };
                let narrow = |value: u64| u8::try_from(value).map_err(|_| malformed());
                let row = MarketEventRow {
                    id: unsigned(field(2)?)?,
                    occurrence: unsigned(field(3)?)?,
                    hour,
                    kind: narrow(unsigned(field(4)?)?)?,
                    mode: narrow(unsigned(field(5)?)?)?,
                    agreement: unsigned(field(6)?)?,
                    variant: unsigned(field(9)?)?,
                    amount_raw: i64::try_from(integer(field(10)?)?).map_err(|_| malformed())?,
                    total_raw: i64::try_from(integer(field(11)?)?).map_err(|_| malformed())?,
                    ordinal: u32::try_from(unsigned(field(12)?)?).map_err(|_| malformed())?,
                    ..MarketEventRow::default()
                };
                let (seller, buyer) = (party(field(7)?)?, party(field(8)?)?);
                Payload::Row(Box::new(ledger_row(&row, seller, buyer)))
            };
            entries.push((event.native_hour, sequence, hour, payload));
        }
        Ok(entries)
    }
}

impl<P: PageRead> Read for Series<'_, '_, P> {
    fn fields(&self) -> Cow<'_, [(u64, Field)]> {
        Cow::Borrowed(&self.catalogue.fields)
    }

    fn check(&self) -> io::Result<()> {
        if self.cancelled.load(Ordering::Acquire) {
            Err(invalid("Economy history request cancelled"))
        } else {
            Ok(())
        }
    }

    fn runs(&mut self, key: u64, spans: &[(i32, i32)]) -> io::Result<Vec<Run>> {
        let runs = self.runs_of(key)?;
        Ok(clip(runs, spans, self.through))
    }

    fn text(&mut self, key: u64, spans: &[(i32, i32)]) -> io::Result<Vec<EconomyRun>> {
        let runs = self.runs(key, spans)?;
        Ok(runs
            .into_iter()
            .map(|run| EconomyRun(run.first_hour, run.last_hour, run.value.to_string()))
            .collect())
    }

    fn tags(&self, slot: u32) -> Vec<HolderTag> {
        holder_tags(
            self.catalogue
                .tags
                .get(&slot)
                .map(Vec::as_slice)
                .unwrap_or_default(),
        )
    }

    fn holder_spans(&self, tags: &[String]) -> BTreeMap<u32, Vec<(i32, i32)>> {
        self.catalogue
            .tags
            .iter()
            .filter_map(|(&slot, runs)| {
                let spans = if tags.is_empty() {
                    vec![(i32::MIN, i32::MAX)]
                } else {
                    tag_spans(runs, tags)
                };
                (!spans.is_empty()).then_some((slot, spans))
            })
            .collect()
    }

    fn warnings(&self) -> Vec<String> {
        self.catalogue
            .refusals
            .iter()
            .map(|(reason, hours)| format!("Market: {reason} ({hours} observed hours)"))
            .collect()
    }

    fn variants(&self) -> &BTreeMap<u64, Vec<VariantVersion>> {
        &self.catalogue.variants
    }

    fn lag(&self) -> i32 {
        self.catalogue.lag
    }

    fn identified(&self) -> &[(i32, i32)] {
        &self.catalogue.identified
    }

    fn supplied(&mut self, key: u64, until: i32) -> io::Result<u64> {
        if let Some(entries) = self.entries.get(&key) {
            return Ok(entries.partition_point(|&(supplied, ..)| supplied <= until) as u64);
        }
        self.snapshot.event_count_through(key, until, None)
    }

    fn entry(
        &mut self,
        key: u64,
        ordinal: u64,
        index: bool,
    ) -> io::Result<(i32, u64, i32, Payload)> {
        if !self.entries.contains_key(&key) {
            let entries = self.decode(key, index)?;
            self.entries.insert(key, entries);
        }
        usize::try_from(ordinal)
            .ok()
            .and_then(|at| self.entries[&key].get(at))
            .cloned()
            .ok_or_else(|| invalid("A market occurrence chain is shorter than its count"))
    }
}

/// The reader of the publication `snapshot` pins, through `through`.
fn series<'s, 'a, P: PageRead>(
    snapshot: &'s mut Snapshot<'a, P>,
    catalogue: &'s Catalogue,
    through: i32,
    cancelled: &'s AtomicBool,
) -> Series<'s, 'a, P> {
    let accepted = snapshot.publication().accepted;
    Series {
        through: through.min(accepted.last),
        first: accepted.first,
        snapshot,
        catalogue,
        cancelled,
        entries: HashMap::new(),
    }
}

/// A view's history from the publication `snapshot` pins, through the prefix `through`; empty
/// before the prefix holds an hour, and pending while `snapshot` is none.
pub fn history<P: PageRead>(
    snapshot: Option<(&mut Snapshot<'_, P>, &Catalogue)>,
    params: &MarketHistoryParams,
    through: Option<i32>,
    texts: Option<&dyn Texts>,
    warnings: Vec<String>,
    cancelled: &AtomicBool,
) -> io::Result<MarketHistoryReply> {
    let mut read = match (snapshot, through) {
        (Some((snapshot, catalogue)), Some(through)) => {
            Some(series(snapshot, catalogue, through, cancelled))
        }
        _ => None,
    };
    reply::history(
        read.as_mut().map(|read| read as &mut dyn Read),
        params,
        through,
        texts,
        warnings,
    )
}

/// A ledger page from the publication `snapshot` pins, through the prefix `through`.
pub fn ledger<P: PageRead>(
    snapshot: Option<(&mut Snapshot<'_, P>, &Catalogue)>,
    params: &MarketLedgerParams,
    through: Option<i32>,
    texts: Option<&dyn Texts>,
    warnings: Vec<String>,
    cancelled: &AtomicBool,
) -> io::Result<MarketLedgerReply> {
    let mut read = match (snapshot, through) {
        (Some((snapshot, catalogue)), Some(through)) => {
            Some(series(snapshot, catalogue, through, cancelled))
        }
        _ => None,
    };
    reply::ledger(
        read.as_mut().map(|read| read as &mut dyn Read),
        params,
        through,
        texts,
        warnings,
    )
}
