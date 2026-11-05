//! National focus replies, read from the family's series publication ([`super::emit`]): a
//! country's focus at an hour, its completed focuses page by page, its dated completions, the
//! four tables' refusals at an hour, and the event feed's completions.

use super::{
    FocusEntry, Hour, PAGE_ROWS, Pages, StateSchema, TableRefusal, UnavailableHour,
    emit::{CHAIN, COMPLETIONS, FAMILY, GAIN, OBSERVED, ORDER, PROGRESS, REASONS, REFUSALS, UNAVAILABLE, kind},
};
use super::art::CompletionArt;
use crate::products::national_focus::FocusCompletion;
use dashboard_base::prepared::engine;
use dashboard_series::{
    Blob, Cell, Event, EventOrder, EventQuery, PageRead, Selection, Snapshot, Span,
    codec::Binary,
    family::{Directory, Scalar},
    query::EventView,
};
use std::{collections::BTreeMap, io, sync::atomic::AtomicBool};

/// Rows an event page reads at once.
const PAGE_BYTES: usize = 1 << 16;

fn invalid(message: &'static str) -> io::Error {
    crate::invalid(message)
}

fn malformed() -> io::Error {
    invalid("A prepared focus event is not valid")
}

/// A text cell's bytes; none for a null one.
fn bytes<P: PageRead>(snapshot: &mut Snapshot<'_, P>, cell: Cell) -> io::Result<Option<Vec<u8>>> {
    match cell {
        Cell::Null => Ok(None),
        Cell::Text(blob) => {
            let length = usize::try_from(blob.len).map_err(|_| malformed())?;
            let mut bytes = vec![0; length];
            if length > 0 {
                snapshot.cell_bytes(blob, 0, &mut bytes)?;
            }
            Ok(Some(bytes))
        }
        _ => Err(malformed()),
    }
}

/// A text cell's text; none for a null one.
fn text<P: PageRead>(snapshot: &mut Snapshot<'_, P>, cell: Cell) -> io::Result<Option<String>> {
    bytes(snapshot, cell)?
        .map(|bytes| String::from_utf8(bytes).map_err(|_| malformed()))
        .transpose()
}

fn unsigned<P: PageRead>(snapshot: &mut Snapshot<'_, P>, event: &Event, at: u64) -> io::Result<u64> {
    match snapshot.field(event.fields, at)? {
        Cell::Unsigned(value) => Ok(value),
        _ => Err(malformed()),
    }
}

fn family_event(event: Option<EventView>, kind: u64, fields: u64) -> io::Result<Option<Event>> {
    match event {
        None => Ok(None),
        Some(EventView::Family(event))
            if event.kind == kind && event.fields.len == fields * Cell::SIZE as u64 =>
        {
            Ok(Some(event))
        }
        Some(_) => Err(malformed()),
    }
}

/// A tag's order as its ORDER event published it.
struct Order {
    slot: u32,
    country_id: u32,
    order_first_hour: i32,
    current: Option<String>,
    continuous: Option<String>,
    paused: bool,
    chain: u64,
    len: u64,
}

impl Order {
    fn read<P: PageRead>(snapshot: &mut Snapshot<'_, P>, event: &Event) -> io::Result<Self> {
        let slot = u32::try_from(unsigned(snapshot, event, 0)?).map_err(|_| malformed())?;
        let country_id = u32::try_from(unsigned(snapshot, event, 1)?).map_err(|_| malformed())?;
        let order_first_hour = match snapshot.field(event.fields, 2)? {
            Cell::Integer(hour) => i32::try_from(hour).map_err(|_| malformed())?,
            _ => return Err(malformed()),
        };
        let current = snapshot.field(event.fields, 3)?;
        let current = text(snapshot, current)?;
        let continuous = snapshot.field(event.fields, 4)?;
        let continuous = text(snapshot, continuous)?;
        let paused = match snapshot.field(event.fields, 5)? {
            Cell::Boolean(paused) => paused,
            _ => return Err(malformed()),
        };
        Ok(Self {
            slot,
            country_id,
            order_first_hour,
            current,
            continuous,
            paused,
            chain: unsigned(snapshot, event, 6)?,
            len: unsigned(snapshot, event, 7)?,
        })
    }
}

/// A completion chain row as its CHAIN event published it.
fn entry<P: PageRead>(snapshot: &mut Snapshot<'_, P>, event: EventView) -> io::Result<FocusEntry> {
    let event = family_event(Some(event), kind::ENTRY, 3)?.ok_or_else(malformed)?;
    let ordinal = u32::try_from(unsigned(snapshot, &event, 0)?).map_err(|_| malformed())?;
    let token = snapshot.field(event.fields, 1)?;
    let token = text(snapshot, token)?.ok_or_else(malformed)?;
    let completed_by_hour = match snapshot.field(event.fields, 2)? {
        Cell::Null => None,
        Cell::Integer(hour) => Some(i32::try_from(hour).map_err(|_| malformed())?),
        _ => return Err(malformed()),
    };
    Ok(FocusEntry {
        ordinal,
        token,
        completed_by_hour,
    })
}

/// Each of `key`'s events from its `skip`th of `from` on, in native order, while `take` asks
/// for more.
fn events<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    from: i32,
    mut skip: u64,
    mut take: impl FnMut(&mut Snapshot<'_, P>, EventView) -> io::Result<bool>,
) -> io::Result<()> {
    let accepted = snapshot.publication().accepted;
    if from > accepted.last {
        return Ok(());
    }
    let selection = [Selection {
        key,
        membership: None,
    }];
    let query = EventQuery {
        selection: &selection,
        native: Span::new(from.max(accepted.first), accepted.last)?,
        as_of: None,
        order: EventOrder::Native,
    };
    let mut cursor = None;
    loop {
        let mut page = Vec::new();
        let read = snapshot.event_page(query, cursor, PAGE_BYTES, |event| {
            page.push(event);
            Ok(())
        })?;
        for event in page {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            if !take(snapshot, event)? {
                return Ok(());
            }
        }
        cursor = read.next;
        if cursor.is_none() {
            return Ok(());
        }
    }
}

fn pages(first: u64, rows: u64) -> Pages {
    Pages {
        first,
        count: rows.div_ceil(PAGE_ROWS as u64),
        rows,
    }
}

/// `tag`'s ORDER key, when the family ever named it.
fn order_key(directory: &Directory, tag: &str) -> Option<(u64, u64)> {
    let actor = directory.actor(tag.as_bytes())?;
    Some((actor, directory.key(actor, ORDER, None)?))
}

/// The order holding at `hour` while `tag` is observed there: its event's hour and ordinal, the
/// order and the presence run holding the hour.
fn order_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    hour: i32,
) -> io::Result<Option<(i32, u64, Order, dashboard_series::Run)>> {
    let Some((presence, _)) = engine::run_at(snapshot, key, hour)? else {
        return Ok(None);
    };
    let selection = Selection {
        key,
        membership: None,
    };
    let latest = snapshot.latest_event(selection, hour, None)?;
    let Some(event) = family_event(latest, kind::ORDER, 8)? else {
        return Ok(None);
    };
    let ordinal = snapshot
        .event_count_through(key, hour, None)?
        .checked_sub(1)
        .ok_or_else(|| invalid("Prepared focus order is missing"))?;
    let order = Order::read(snapshot, &event)?;
    Ok(Some((event.native_hour, ordinal, order, presence)))
}

/// A country's focus at `hour`, None where it was not observed.
pub fn hour<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    tag: &str,
    hour: i32,
) -> io::Result<Option<Hour>> {
    let Some((actor, key)) = order_key(directory, tag) else {
        return Ok(None);
    };
    let Some((first, ordinal, order, presence)) = order_at(snapshot, key, hour)? else {
        return Ok(None);
    };
    let selection = Selection {
        key,
        membership: None,
    };
    let next = snapshot.event_date(selection, None, ordinal + 1)?;
    let value = |snapshot: &mut Snapshot<'_, P>, measure| -> io::Result<Option<i128>> {
        Ok(match directory.key(actor, measure, None) {
            Some(key) => engine::run_at(snapshot, key, hour)?.map(|(run, _)| run.value),
            None => None,
        })
    };
    let progress = value(snapshot, PROGRESS)?.ok_or_else(|| {
        invalid("Prepared focus progress is not observed at this hour; prepare again")
    })?;
    let gain = value(snapshot, GAIN)?;
    Ok(Some(Hour {
        schema: StateSchema,
        slot: order.slot,
        country_id: order.country_id,
        hour: first.max(presence.span.first),
        last_hour: next.map_or(presence.span.last, |next| presence.span.last.min(next - 1)),
        order_first_hour: Some(order.order_first_hour),
        current: order.current,
        continuous: order.continuous,
        paused: order.paused,
        completed: pages(ordinal, order.len),
        at: hour,
        progress_raw: progress.to_string(),
        daily_gain_raw: gain.map(|gain| gain.to_string()),
    }))
}

/// The four focus tables' refusals at `hour`, with the hours they hold.
pub fn unavailable<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    hour: i32,
) -> io::Result<Option<UnavailableHour>> {
    let Some(key) = directory.key(FAMILY, REFUSALS, None) else {
        return Ok(None);
    };
    let Some((run, _)) = engine::run_at(snapshot, key, hour)? else {
        return Ok(None);
    };
    let id = u64::try_from(run.value).map_err(|_| malformed())?;
    let set = directory
        .scalar(REASONS + id)
        .and_then(Scalar::text)
        .ok_or_else(|| invalid("A prepared focus refusal names no reasons"))?;
    let reasons: Vec<TableRefusal> = serde_json::from_slice(set)?;
    Ok((!reasons.is_empty()).then(|| UnavailableHour {
        schema: StateSchema,
        first_hour: run.span.first,
        last_hour: run.span.last,
        reasons,
    }))
}

/// One page of `tag`'s completed focuses at `hour`, read from its completion chain; `requested`
/// must be the pages `focus-hour` gave for that hour.
pub fn order_page<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    tag: &str,
    hour: i32,
    requested: &Pages,
    page: u64,
) -> io::Result<serde_json::Value> {
    let unavailable = || dashboard_core::refused("Prepared focus data is unavailable at this hour");
    let Some((actor, key)) = order_key(directory, tag) else {
        return Err(unavailable());
    };
    let Some((_, ordinal, order, _)) = order_at(snapshot, key, hour)? else {
        return Err(unavailable());
    };
    let actual = pages(ordinal, order.len);
    if (actual.first, actual.count, actual.rows)
        != (requested.first, requested.count, requested.rows)
    {
        return Err(invalid("Prepared focus page changed. Refresh this view."));
    }
    if page >= actual.count {
        return Ok(serde_json::Value::Null);
    }
    let first = page
        .checked_mul(PAGE_ROWS as u64)
        .ok_or_else(|| invalid("Focus page index overflow"))?;
    let rows = (order.len - first).min(PAGE_ROWS as u64);
    let chain = directory
        .key(actor, CHAIN, None)
        .ok_or_else(|| invalid("Prepared focus order is incomplete"))?;
    let at = order
        .chain
        .checked_add(first)
        .ok_or_else(|| invalid("Focus page index overflow"))?;
    let selection = Selection {
        key: chain,
        membership: None,
    };
    let mut entries = Vec::new();
    if let Some(from) = snapshot.event_date(selection, None, at)? {
        let before = match from.checked_sub(1) {
            Some(hour) => snapshot.event_count_through(chain, hour, None)?,
            None => 0,
        };
        let skip = at
            .checked_sub(before)
            .ok_or_else(|| invalid("Prepared focus order is incomplete"))?;
        events(snapshot, chain, from, skip, |snapshot, event| {
            entries.push(entry(snapshot, event)?);
            Ok((entries.len() as u64) < rows)
        })?;
    }
    if entries.len() as u64 != rows
        || entries
            .iter()
            .enumerate()
            .any(|(index, entry)| u64::from(entry.ordinal) != first + index as u64)
    {
        return Err(invalid("Prepared focus order is incomplete"));
    }
    Ok(serde_json::to_value(entries)?)
}

/// Every completion `tag`'s order gained while it was observed, each dated by the hour that saw
/// it, by date and ordinal: its completion chain without the undated rows a reset or a first
/// read publishes. A country never observed has none.
pub fn completions<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    tag: &str,
) -> io::Result<Vec<FocusEntry>> {
    let Some(key) = directory
        .actor(tag.as_bytes())
        .and_then(|actor| directory.key(actor, CHAIN, None))
    else {
        return Ok(Vec::new());
    };
    let mut entries = Vec::new();
    events(snapshot, key, i32::MIN, 0, |snapshot, event| {
        let entry = entry(snapshot, event)?;
        if entry.completed_by_hour.is_some() {
            entries.push(entry);
        }
        Ok(true)
    })?;
    entries.sort_by_key(|entry| (entry.completed_by_hour, entry.ordinal));
    Ok(entries)
}

/// The event feed's view of the publication: every completion in the order it lists them, where
/// each one's art is stored (read only for the completions a page shows, [`CompletionArts`]),
/// whether any country was observed, and the countries and completions without an observation
/// or a date, by reason.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Feed {
    pub completions: Vec<FocusCompletion>,
    /// Each completion's stored art, none where it was prepared without the game's content.
    pub art: Vec<Option<Blob>>,
    pub observed: bool,
    pub unavailable: BTreeMap<String, u64>,
}

impl Feed {
    /// The bytes it holds, for the cache keeping it.
    pub fn bytes(&self) -> usize {
        self.completions
            .iter()
            .map(|completion| 64 + completion.tag.capacity() + completion.token.capacity())
            .sum::<usize>()
            + self.art.capacity() * std::mem::size_of::<Option<Blob>>()
            + self
                .unavailable
                .keys()
                .map(|reason| 64 + reason.capacity())
                .sum::<usize>()
    }
}

/// Where the event feed reads the art of the completions a page shows.
pub(crate) trait CompletionArts {
    /// The art kept with each of the feed's completions at `ordinals`, none where it was
    /// prepared without the game's content.
    fn art(&self, ordinals: &[u64], cancelled: &AtomicBool) -> io::Result<Vec<Option<CompletionArt>>>;
}

/// A feed, its completions' art read from the publication it was read from.
pub(crate) struct Stored {
    pub feed: std::sync::Arc<Feed>,
    pub opened: dashboard_base::prepared::published::Opened,
}

impl CompletionArts for Stored {
    fn art(&self, ordinals: &[u64], cancelled: &AtomicBool) -> io::Result<Vec<Option<CompletionArt>>> {
        let stored = ordinals
            .iter()
            .map(|&ordinal| {
                usize::try_from(ordinal)
                    .ok()
                    .and_then(|ordinal| self.feed.art.get(ordinal))
                    .copied()
                    .ok_or_else(|| invalid("A focus completion's art reference is missing"))
            })
            .collect::<io::Result<Vec<_>>>()?;
        if stored.iter().all(Option::is_none) {
            return Ok(vec![None; stored.len()]);
        }
        self.opened.read(cancelled, |snapshot, _| {
            stored
                .into_iter()
                .map(|blob| {
                    blob.map(|blob| -> io::Result<CompletionArt> {
                        let art = bytes(snapshot, Cell::Text(blob))?.ok_or_else(malformed)?;
                        dashboard_base::prepared::codec::decode(&art)
                    })
                    .transpose()
                })
                .collect()
        })
    }
}

/// A test feed's completions' art, by ordinal.
#[cfg(test)]
impl CompletionArts for Vec<Option<CompletionArt>> {
    fn art(&self, ordinals: &[u64], _: &AtomicBool) -> io::Result<Vec<Option<CompletionArt>>> {
        ordinals
            .iter()
            .map(|&ordinal| {
                usize::try_from(ordinal)
                    .ok()
                    .and_then(|ordinal| self.get(ordinal))
                    .cloned()
                    .ok_or_else(|| invalid("A focus completion's art reference is missing"))
            })
            .collect()
    }
}

pub(crate) fn feed<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
) -> io::Result<Feed> {
    let mut completions = Vec::new();
    let mut art = Vec::new();
    if let Some(key) = directory.key(FAMILY, COMPLETIONS, None) {
        let tag = |actor: u64| {
            directory
                .actors
                .get(actor as usize)
                .map(|actor| String::from_utf8_lossy(&actor.name).into_owned())
                .ok_or_else(|| invalid("A prepared focus completion names an unknown country"))
        };
        events(snapshot, key, i32::MIN, 0, |snapshot, event| {
            let event = family_event(Some(event), kind::COMPLETION, 3)?.ok_or_else(malformed)?;
            let actor = unsigned(snapshot, &event, 0)?;
            let token = snapshot.field(event.fields, 1)?;
            let token = text(snapshot, token)?.ok_or_else(malformed)?;
            art.push(match snapshot.field(event.fields, 2)? {
                Cell::Null => None,
                Cell::Text(blob) => Some(blob),
                _ => return Err(malformed()),
            });
            completions.push(FocusCompletion {
                tag: tag(actor)?,
                token,
                hour: event.native_hour,
            });
            Ok(true)
        })?;
    }
    let observed = matches!(directory.scalar(OBSERVED), Some(Scalar::Boolean(true)));
    let unavailable = directory
        .scalar(UNAVAILABLE)
        .map_or(&[][..], Scalar::list)
        .iter()
        .map(|pair| match pair.list() {
            [reason, count] => Some((
                String::from_utf8(reason.text()?.to_vec()).ok()?,
                u64::try_from(count.integer()?).ok()?,
            )),
            _ => None,
        })
        .collect::<Option<BTreeMap<_, _>>>()
        .ok_or_else(|| invalid("Prepared focus counts are not valid"))?;
    Ok(Feed {
        completions,
        art,
        observed,
        unavailable,
    })
}
