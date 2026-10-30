//! Each family's publication on a branch: what the driver's publisher writes in the family's own
//! stage and the gates read under a query's pinned manifest.
//!
//! Its root is a series root ([`dashboard_series`]). The root's identity carries the family's
//! version stamp, so a family whose version changed is prepared again alone; its checkpoint is
//! the family's [`Frontier`] on the branch, of a size independent of the branch's length. Beside
//! it are the paged ledger of what each processed hour holds, and the emit state and projector
//! carry the family goes on with, named by their content. A family that moves its output onto
//! the series engine writes it into this same hierarchy.

use dashboard_series::{self as series, Budget, Cache, PageRead, StoreRead, StoreWrite};
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Read},
    sync::LazyLock,
};

const ROOT: &str = "preparation-root-";
const LEDGER: &str = "preparation-ledger-";
const STATE: &str = "preparation-state-";
const CARRY: &str = "preparation-carry-";

/// Intervals one ledger page holds at most.
pub const PAGE_INTERVALS: usize = 256;

// Root, node and heap pages of publications read and written; frontiers are small.
static BUDGET: Budget = Budget::new(64 << 20);
static CACHE: LazyLock<Cache<'static>> = LazyLock::new(|| Cache::new(&BUDGET));

fn cache() -> &'static Cache<'static> {
    &CACHE
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn refused(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// Ledger pages this thread read, for bounds on detailed range reads.
    pub static LEDGER_READS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// The section of a family's series pages on a branch: its last range is the root.
pub fn root_section(family: &str, branch: u64) -> String {
    format!("{ROOT}{family}-{branch}")
}

/// The section of a family's ledger pages on a branch.
pub fn ledger_section(family: &str, branch: u64) -> String {
    format!("{LEDGER}{family}-{branch}")
}

/// The section of the emit state a family goes on with on a branch.
pub fn state_section(family: &str, branch: u64) -> String {
    format!("{STATE}{family}-{branch}")
}

/// The section of the projector carry a family goes on with on a branch.
pub fn carry_section(family: &str, branch: u64) -> String {
    format!("{CARRY}{family}-{branch}")
}

/// The family and branch whose publication `section` is part of.
pub fn owner(section: &str) -> Option<(&str, u64)> {
    [ROOT, LEDGER, STATE, CARRY]
        .into_iter()
        .find_map(|prefix| named(section.strip_prefix(prefix)?))
}

/// The family and branch whose root section `section` is.
pub fn root_of(section: &str) -> Option<(&str, u64)> {
    named(section.strip_prefix(ROOT)?)
}

fn named(rest: &str) -> Option<(&str, u64)> {
    let (family, branch) = rest.rsplit_once('-')?;
    if family.is_empty() || branch.is_empty() || !branch.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some((family, branch.parse().ok()?))
}

/// What a family's publications are bound to beside its version: the recording and its content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    pub source: [u8; 32],
    pub content: [u8; 32],
}

impl Binding {
    /// The binding of the recording and content `source` and `content` identify.
    pub fn new(source: &[u8], content: &[u8]) -> Self {
        Self {
            source: *blake3::hash(source).as_bytes(),
            content: *blake3::hash(content).as_bytes(),
        }
    }

    /// The identity of a family's publication on `branch` whose version stamp is `stamp`.
    pub fn identity(&self, branch: u64, stamp: u64) -> series::Identity {
        let mut bytes = [0; 32];
        bytes[..8].copy_from_slice(&branch.to_le_bytes());
        series::Identity {
            source: self.source,
            content: self.content,
            branch: bytes,
            schema: stamp,
        }
    }
}

/// What a processed hour holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Held {
    Prepared,
    /// The recording there could not be read, or the family's history there has a gap.
    Lost,
    /// The family refused its branch's data from these hours on.
    Refused,
}

/// Processed hours holding the same, in order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Interval {
    pub first_hour: i32,
    pub last_hour: i32,
    pub held: Held,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
}

/// The segment a family goes on with on a branch: its run, and the ordinal it starts at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Cursor {
    pub run: u32,
    pub ordinal: u32,
}

/// Why a family's branch ended before the recording's: every later hour holds `held`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct End {
    pub held: Held,
    pub hour: i32,
    pub reason: String,
    /// The family whose refusal or loss ended it, when another's did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

/// Bytes named by what they hold, wherever compaction moves the range holding them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Content {
    pub len: u64,
    pub digest: [u8; 32],
}

impl Content {
    pub fn of(bytes: &[u8]) -> Self {
        Self {
            len: bytes.len() as u64,
            digest: *blake3::hash(bytes).as_bytes(),
        }
    }
}

/// Where a family's publication on a branch stands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Frontier {
    /// The family's own version, as the registry gives it.
    pub version: u32,
    /// The hour after the last it processed, by its run and ordinal.
    pub next: Cursor,
    /// The branch's segments it processed whole, counted from the branch's first: an open
    /// tail's hours advance `next` and the hours, never this.
    pub segments: u64,
    /// The hours it processed, each prepared, lost or refused as its ledger says.
    pub first_hour: i32,
    pub last_hour: i32,
    /// How many of them it prepared.
    pub prepared: u64,
    /// Proof of the recorded input it accepted, through its last hour.
    pub proof: [u8; 32],
    /// Its ledger's pages, every one full but the last.
    pub ledger_pages: u64,
    /// Its earliest hours that are not prepared, as far as they reach: no reply needing earlier
    /// observations answers an hour from them on.
    pub first_hole: Option<Interval>,
    /// The emit state and projector carry it goes on with.
    pub state: Option<Content>,
    pub carry: Option<Content>,
    pub end: Option<End>,
    /// It processed the branch's last segment of the recording it was prepared from.
    pub complete: bool,
}

impl Frontier {
    /// Whether its branch has nothing more for the family in the recording it was prepared from.
    pub fn terminal(&self) -> bool {
        self.complete || self.end.is_some()
    }
}

/// A family's publication on a branch, as a manifest or a transaction holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct Published {
    /// The root's range in its section.
    pub root: u64,
    pub identity: series::Identity,
    pub publication: series::Publication,
    pub frontier: Frontier,
}

/// Where publications are read from: a pinned publication, or a writer's transaction.
pub trait View {
    fn count(&mut self, section: &str) -> io::Result<Option<u64>>;
    fn range(&mut self, section: &str, range: u64) -> io::Result<Option<store::Range>>;
    /// The identity, publication and checkpoint bytes of the series root at `root`.
    fn root(&mut self, section: &str, root: u64) -> io::Result<Root>;
}

/// What a series root holds for its owner.
pub type Root = (series::Identity, series::Publication, Vec<u8>);

impl View for &store::Snapshot {
    fn count(&mut self, section: &str) -> io::Result<Option<u64>> {
        Ok(store::Snapshot::range_count(self, section))
    }

    fn range(&mut self, section: &str, range: u64) -> io::Result<Option<store::Range>> {
        store::Snapshot::read(self, section, range)
    }

    fn root(&mut self, section: &str, root: u64) -> io::Result<Root> {
        checkpoint(
            StoreRead {
                snapshot: *self,
                section,
            },
            root,
        )
    }
}

impl View for store::Writer {
    fn count(&mut self, section: &str) -> io::Result<Option<u64>> {
        store::Writer::range_count(self, section)
    }

    fn range(&mut self, section: &str, range: u64) -> io::Result<Option<store::Range>> {
        store::Writer::read_at(self, section, range)
    }

    fn root(&mut self, section: &str, root: u64) -> io::Result<Root> {
        checkpoint(
            StoreWrite {
                writer: self,
                section,
            },
            root,
        )
    }
}

fn checkpoint(mut io: impl PageRead, root: u64) -> io::Result<Root> {
    let identity = series::query::root_identity(&mut io, cache(), root)?;
    let mut snapshot = series::Snapshot::open(io, cache(), root, identity)?;
    let blob = snapshot.checkpoint();
    let length = usize::try_from(blob.len).map_err(|_| invalid("Frontier length"))?;
    let mut bytes = engine_allocation::filled(length, 0_u8)?;
    snapshot.cell_bytes(blob, 0, &mut bytes)?;
    Ok((identity, snapshot.publication(), bytes))
}

/// A family's publication on `branch` as `view` holds it; None before its first, or once cleared.
pub fn read(view: &mut impl View, family: &str, branch: u64) -> io::Result<Option<Published>> {
    let section = root_section(family, branch);
    let Some(count) = view.count(&section)?.filter(|&count| count > 0) else {
        return Ok(None);
    };
    let root = count - 1;
    let (identity, publication, bytes) = view.root(&section, root)?;
    let frontier: Frontier = serde_json::from_slice(&bytes)?;
    if publication.accepted.first != frontier.first_hour
        || publication.accepted.last != frontier.last_hour
    {
        return Err(invalid("A family's root and frontier disagree"));
    }
    Ok(Some(Published {
        root,
        identity,
        publication,
        frontier,
    }))
}

fn ledger_page(
    view: &mut impl View,
    section: &str,
    index: u64,
    published: &Published,
) -> io::Result<Vec<Interval>> {
    #[cfg(any(test, feature = "test-support"))]
    LEDGER_READS.with(|reads| reads.set(reads.get() + 1));
    let range = view
        .range(section, index)?
        .ok_or_else(|| invalid("A ledger page its frontier counts is missing"))?;
    let intervals: Vec<Interval> = serde_json::from_reader(io::BufReader::new(range))?;
    let full = index + 1 < published.frontier.ledger_pages;
    if intervals.is_empty() || (full && intervals.len() != PAGE_INTERVALS) {
        return Err(invalid("A ledger page does not match its frontier"));
    }
    Ok(intervals)
}

/// The intervals of `published`'s ledger that meet `first..=last`, in hour order: the pages
/// before them are passed over by halving, the rest read in order.
pub fn ledger(
    view: &mut impl View,
    family: &str,
    branch: u64,
    published: &Published,
    first: i32,
    last: i32,
) -> io::Result<Vec<Interval>> {
    let section = ledger_section(family, branch);
    let pages = published.frontier.ledger_pages;
    // The first page whose last interval reaches `first`.
    let (mut low, mut high) = (0, pages);
    while low < high {
        let middle = low + (high - low) / 2;
        let page = ledger_page(view, &section, middle, published)?;
        if page
            .last()
            .is_some_and(|interval| interval.last_hour < first)
        {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    let mut found = Vec::new();
    for index in low..pages {
        let page = ledger_page(view, &section, index, published)?;
        if page
            .first()
            .is_some_and(|interval| interval.first_hour > last)
        {
            break;
        }
        found.extend(
            page.into_iter()
                .filter(|interval| interval.last_hour >= first && interval.first_hour <= last),
        );
    }
    Ok(found)
}

/// What `published` leaves unanswered of `first..=last`: its first hole, from which no reply
/// needing earlier observations answers, and every hour after its branch ended for it.
pub fn holes(published: &Published, first: i32, last: i32) -> Vec<Interval> {
    let frontier = &published.frontier;
    let mut holes: Vec<Interval> = frontier
        .first_hole
        .iter()
        .map(|hole| Interval {
            first_hour: hole.first_hour.max(first),
            last_hour: hole.last_hour.min(last),
            ..hole.clone()
        })
        .filter(|hole| hole.first_hour <= hole.last_hour)
        .collect();
    if let Some(end) = &frontier.end
        && let Some(after) = frontier.last_hour.checked_add(1)
        && after.max(first) <= last
    {
        holes.push(Interval {
            first_hour: after.max(first),
            last_hour: last,
            held: end.held,
            reason: end.reason.clone(),
        });
    }
    holes
}

/// The emit state `published` names on its branch; None when it names none.
pub fn state(
    view: &mut impl View,
    family: &str,
    branch: u64,
    published: &Published,
) -> io::Result<Option<Vec<u8>>> {
    named_bytes(
        view,
        &state_section(family, branch),
        published.frontier.state,
    )
}

/// The projector carry `published` names on its branch; None when it names none.
pub fn carry(
    view: &mut impl View,
    family: &str,
    branch: u64,
    published: &Published,
) -> io::Result<Option<Vec<u8>>> {
    named_bytes(
        view,
        &carry_section(family, branch),
        published.frontier.carry,
    )
}

fn named_bytes(
    view: &mut impl View,
    section: &str,
    content: Option<Content>,
) -> io::Result<Option<Vec<u8>>> {
    let Some(content) = content else {
        return Ok(None);
    };
    let mut range = view
        .range(section, 0)?
        .ok_or_else(|| invalid("A family's state is missing"))?;
    let mut bytes = Vec::new();
    range.read_to_end(&mut bytes)?;
    if Content::of(&bytes) != content {
        return Err(invalid(
            "A family's state is not the one its frontier names",
        ));
    }
    Ok(Some(bytes))
}

/// What one batch of a family did on a branch.
pub struct Step<'a> {
    /// Where it leaves the family; its ledger pages, prepared hours, first hole and state
    /// contents are filled in from what it publishes.
    pub frontier: Frontier,
    /// The hours it processed, in order, after the previous frontier's.
    pub intervals: &'a [Interval],
    pub state: Option<&'a [u8]>,
    pub carry: Option<&'a [u8]>,
}

/// Publishes `step` of `family` on `branch` into `writer` after `previous`, the family's last
/// publication there: its ledger pages, emit state and carry, then its root, last. The root's
/// publication sequence counts the family's publications on the branch; its hours and its
/// cursor must advance, an open tail's within its segment. A step that repeats or reverses
/// hours, or does not follow `previous`, is refused (`InvalidInput`).
pub fn publish(
    writer: &mut store::Writer,
    family: &str,
    branch: u64,
    identity: series::Identity,
    previous: Option<&Published>,
    mut step: Step<'_>,
) -> io::Result<Published> {
    let frontier = &mut step.frontier;
    if let Some(previous) = previous {
        if previous.identity != identity {
            return Err(refused(
                "A family's publication goes on under another identity",
            ));
        }
        let before = &previous.frontier;
        if before.end.is_some() {
            return Err(refused("A family's branch goes on after it ended"));
        }
        if frontier.first_hour != before.first_hour
            || frontier.last_hour <= before.last_hour
            || frontier.next <= before.next
            || frontier.segments < before.segments
        {
            return Err(refused("A family's step would publish hours twice"));
        }
    }
    let before = previous.map(|previous| &previous.frontier);
    if step
        .intervals
        .first()
        .is_some_and(|first| first.first_hour < frontier.first_hour)
        || step
            .intervals
            .last()
            .is_some_and(|last| last.last_hour > frontier.last_hour)
    {
        return Err(refused("A family's step holds hours outside its frontier"));
    }
    frontier.ledger_pages = append_ledger(
        writer,
        &ledger_section(family, branch),
        before.map_or(0, |before| before.ledger_pages),
        before.map(|before| before.last_hour),
        step.intervals,
    )?;
    frontier.prepared = before.map_or(0, |before| before.prepared);
    frontier.first_hole = before.and_then(|before| before.first_hole.clone());
    for interval in step.intervals {
        if interval.held == Held::Prepared {
            frontier.prepared += span(interval.first_hour, interval.last_hour);
            continue;
        }
        match &mut frontier.first_hole {
            None => frontier.first_hole = Some(interval.clone()),
            Some(hole)
                if hole.held == interval.held
                    && hole.reason == interval.reason
                    && i64::from(hole.last_hour) + 1 == i64::from(interval.first_hour) =>
            {
                hole.last_hour = interval.last_hour;
            }
            Some(_) => {}
        }
    }
    frontier.state = step
        .state
        .map(|bytes| replace(writer, &state_section(family, branch), bytes))
        .transpose()?;
    frontier.carry = step
        .carry
        .map(|bytes| replace(writer, &carry_section(family, branch), bytes))
        .transpose()?;
    let (generation, sequence) = match previous {
        Some(previous) => (
            previous.publication.generation.checked_add(1),
            previous.publication.segment.checked_add(1),
        ),
        None => (Some(1), Some(1)),
    };
    let refusal = frontier
        .end
        .as_ref()
        .filter(|end| end.held == Held::Refused)
        .map(|end| u64::from(end.hour.cast_unsigned()));
    let publication = series::Publication {
        generation: generation.ok_or_else(|| invalid("Publication generation overflow"))?,
        segment: sequence.ok_or_else(|| invalid("Publication sequence overflow"))?,
        accepted: series::Span::new(frontier.first_hour, frontier.last_hour)?,
        reached: frontier.last_hour,
        refusal,
    };
    let bytes = serde_json::to_vec(&*frontier)?;
    let section = root_section(family, branch);
    let mut io = StoreWrite {
        writer,
        section: &section,
    };
    let mut series = match previous {
        Some(previous) => series::Writer::resume(&mut io, cache(), previous.root, identity)?,
        None => series::Writer::new(&BUDGET, identity, frontier.first_hour)?,
    };
    let checkpoint = series.blob(&mut bytes.as_slice(), bytes.len() as u64)?;
    let pending = series.cut(publication, checkpoint)?;
    let sealed = pending.seal(&mut io)?;
    let root = series.publish(&mut io, cache(), sealed)?;
    Ok(Published {
        root,
        identity,
        publication,
        frontier: step.frontier,
    })
}

fn span(first: i32, last: i32) -> u64 {
    (i64::from(last) - i64::from(first) + 1) as u64
}

fn replace(writer: &mut store::Writer, section: &str, bytes: &[u8]) -> io::Result<Content> {
    writer.write_at(section, 0, bytes)?;
    Ok(Content::of(bytes))
}

/// Appends `intervals`, which follow hour `after`, to the ledger of `pages` pages; returns its
/// page count. Its last page is rewritten until it is full, and an interval continuing the last
/// held alike extends it.
fn append_ledger(
    writer: &mut store::Writer,
    section: &str,
    pages: u64,
    after: Option<i32>,
    intervals: &[Interval],
) -> io::Result<u64> {
    let mut last = after;
    for interval in intervals {
        if interval.first_hour > interval.last_hour
            || last.is_some_and(|last| interval.first_hour <= last)
        {
            return Err(refused("A ledger interval repeats or reverses hours"));
        }
        last = Some(interval.last_hour);
    }
    if intervals.is_empty() {
        return Ok(pages);
    }
    let mut at = pages;
    let mut tail: Vec<Interval> = Vec::new();
    if let Some(index) = pages.checked_sub(1) {
        let range = writer
            .read_at(section, index)?
            .ok_or_else(|| invalid("A ledger page its frontier counts is missing"))?;
        let page: Vec<Interval> = serde_json::from_reader(io::BufReader::new(range))?;
        if page.len() < PAGE_INTERVALS {
            at = index;
            tail = page;
        }
    }
    for interval in intervals {
        if let Some(held) = tail.last_mut()
            && held.held == interval.held
            && held.reason == interval.reason
            && i64::from(held.last_hour) + 1 == i64::from(interval.first_hour)
        {
            held.last_hour = interval.last_hour;
            continue;
        }
        if tail.len() == PAGE_INTERVALS {
            writer.write_at(section, at, serde_json::to_vec(&tail)?.as_slice())?;
            at += 1;
            tail.clear();
        }
        tail.push(interval.clone());
    }
    writer.write_at(section, at, serde_json::to_vec(&tail)?.as_slice())?;
    Ok(at + 1)
}
