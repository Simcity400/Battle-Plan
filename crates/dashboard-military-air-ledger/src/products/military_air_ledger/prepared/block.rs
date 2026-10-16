//! The military air ledger prepared block by block, on its own. A pre-pass
//! worker reads a block alone, only at the hours the fields the ledger reads
//! change ([`FAMILY`]'s fields), and runs the ledger on it from the block's
//! first hour with no state of earlier blocks, recording the calls it makes
//! on its series after that hour. The hours between are quiet: the event
//! tables are unchanged (so empty, or a repeated ordinal), the countries'
//! keys and links too, so each extends what the hour before held.
//!
//! The fold observes the block's first hour on the carried ledger, as hourly
//! preparation does, and then continues from the extract: it makes the
//! recorded calls again under the carried keys and carries every span and
//! count forward. After the first hour the carried ledger differs from the
//! extract's only in the event ordinal and unread interval it carried into
//! the block, which the extract followed: until the block's first read with
//! events, no hour reads them but to start the unread interval, and that read
//! starts a new identity epoch or checks its first ordinal against them. When
//! the carried ledger would refuse that read, or an ordinal is checked
//! outside a read, the fold observes the rest of the block hour by hour.

use super::*;
use dashboard_base::prepared::{
    frame_changes::{FrameChangeTracker, FrameChanges},
    prepass::{Extract, Family, Folded, Held, HeldBytes},
    series::{Recording, ReplayTarget},
};
use replay::{
    container::{BlockIndex, QuietHours, Replay, TableFields},
    format::FormatVersion,
};
use std::{any::Any, ops::Range, sync::Arc};

/// The tables the ledger reads.
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::LexicalTokens,
    TableId::AirLossEvents,
    TableId::BuildingBombingDamage,
];

pub static FAMILY: Family = Family {
    name: "air_ledger",
    extract: "air_ledger.extract",
    sequential: "air_ledger.sequential",
    fallback: "air_ledger.fallback",
    tables: TABLES,
    fields: Some(fields),
    start,
};

/// The fields of the ledger's tables its scans read. Campaign is checked on
/// every read and read by nothing else here.
fn fields() -> io::Result<Vec<TableFields>> {
    Ok(vec![
        // Country identity reads the keys (`CountryKeys`) and tags slots
        // through the links (`tagged_countries`): the id (the row key),
        // canonical slot and tag. The balances change every hour and are
        // not read.
        TableFields::named(TableId::Countries, &["canonical_slot", "tag"])?,
        // Every link maps an event's raw country to its slot.
        TableFields::all(TableId::CountryLinks),
        // Event tables: every row of an hour with events is read, and an
        // equal table read again (a new allocation) is a repeated ordinal,
        // so every field is watched and every decoding is visited.
        TableFields::all(TableId::AirLossEvents),
        TableFields::all(TableId::BuildingBombingDamage),
        // Building names are spelled at hours with bombing events, which
        // are observed anyway; the dictionary is static.
        TableFields::all(TableId::LexicalTokens),
    ])
}

fn start(_: FormatVersion) -> Box<dyn Extract> {
    Box::new(BlockExtract {
        prepared: Box::default(),
        recording: Recording::default(),
        first: None,
        last: None,
        hour: None,
        #[cfg(any(test, feature = "test-support"))]
        frames: 0,
    })
}

/// A scan's ordinal state and warnings right after the block's first hour,
/// as the extract held it.
struct Entered {
    last_event: Option<u64>,
    unread_since: Option<i32>,
    warnings: BTreeMap<String, usize>,
}

impl Entered {
    fn of(scan: &FamilyScan) -> Self {
        Self {
            last_event: scan.last_event,
            unread_since: scan.unread_since,
            warnings: scan.warnings.clone(),
        }
    }
}

/// The block's first hour, which the fold observes on the carried ledger,
/// and what the extract held after it.
struct First {
    frame: DecodedFrame,
    air: Entered,
    bombing: Entered,
}

/// One scan's block: its state at the block's end, its series' keys in key
/// order and the leaves it named.
struct Followed {
    scan: FamilyScan,
    names: Vec<String>,
    leaves: BTreeSet<LeafKey>,
}

impl Followed {
    fn of(events: Events, scan: FamilyScan) -> io::Result<Self> {
        Ok(Self {
            scan,
            names: series::catalogue_names(&events.keys)?,
            leaves: events.leaves,
        })
    }
}

struct BlockFacts {
    first: First,
    recording: Recording,
    air: Followed,
    bombing: Followed,
    /// The last hour observed or quiet.
    last: i32,
    /// Frames the extract was given or built.
    #[cfg(any(test, feature = "test-support"))]
    frames: usize,
}

struct BlockExtract {
    prepared: Box<Prepared>,
    recording: Recording,
    first: Option<First>,
    /// The last frame observed, which the quiet hours after it repeat.
    last: Option<DecodedFrame>,
    hour: Option<i32>,
    #[cfg(any(test, feature = "test-support"))]
    frames: usize,
}

impl Held for LeafKey {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .heap(&self.country)
            .heap(&self.opponent)
            .heap(&self.building);
    }
}

impl Held for IdentityOperands {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.heap(&self.by_id).heap(&self.present);
    }
}

/// A scan: its spans, each country's spans, its pending leaves, warnings,
/// and the identities and country keys it read.
impl Held for FamilyScan {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .heap(&self.observed)
            .heap(&self.incomplete)
            .heap(&self.countries)
            .heap(&self.leaves)
            .heap(&self.warnings)
            .heap(&self.identities)
            .heap(&self.country_keys);
    }
}

impl Held for Entered {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.heap(&self.warnings);
    }
}

impl Extract for BlockExtract {
    /// What the block's facts hold: its first frame and each scan's warnings
    /// then, its recorded calls, each scan at the block's end, and each
    /// series' keys by name and leaves.
    fn bytes(&self) -> u64 {
        let mut held = HeldBytes::default();
        if let Some(first) = &self.first {
            held.frame(&first.frame)
                .heap(&first.air)
                .heap(&first.bombing);
        }
        held.heap(&self.recording);
        for events in [&self.prepared.air, &self.prepared.bombing] {
            // The keys by number, as the facts name them.
            held.add(events.keys.len() * size_of::<String>())
                .add(events.keys.keys().map(String::capacity).sum())
                .heap(&events.leaves);
        }
        held.heap(&self.prepared.scan.air)
            .heap(&self.prepared.scan.bombing)
            .total()
    }

    fn observe(
        &mut self,
        _: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        #[cfg(any(test, feature = "test-support"))]
        {
            self.frames += 1;
        }
        if self.first.is_none() {
            self.prepared.observe_changed(frame, changes)?;
            let scan = &mut self.prepared.scan;
            self.first = Some(First {
                frame: frame.clone(),
                air: Entered::of(&scan.air),
                bombing: Entered::of(&scan.bombing),
            });
            scan.air.entry = Some(Entry::default());
            scan.bombing.entry = Some(Entry::default());
            self.prepared.record_calls();
        } else {
            let prepared = &mut self.prepared;
            series::with_recording(&mut self.recording, || {
                prepared.observe_changed(frame, changes)
            })?;
        }
        self.last = Some(frame.clone());
        self.hour = Some(frame.hour);
        Ok(())
    }

    /// While a scan reads its identities again every hour, the next hour.
    fn deadline(&self) -> Option<i32> {
        let scan = &self.prepared.scan;
        (scan.air.retry || scan.bombing.retry)
            .then(|| self.hour.and_then(|hour| hour.checked_add(1)))
            .flatten()
    }

    fn quiet(&mut self, _: &Replay, hours: &QuietHours, span: Range<usize>) -> io::Result<()> {
        let span = &hours.hours()[span];
        let (Some(&first), Some(&last)) = (span.first(), span.last()) else {
            return Ok(());
        };
        let BlockExtract {
            prepared,
            recording,
            last: frame,
            hour,
            ..
        } = self;
        let frame = frame
            .as_ref()
            .ok_or_else(|| io::Error::other("Ledger hours were quiet before the block's first"))?;
        series::with_recording(recording, || prepared.quiet(frame, first, last))?;
        *hour = Some(last);
        Ok(())
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        let BlockExtract {
            prepared,
            recording,
            first,
            hour,
            #[cfg(any(test, feature = "test-support"))]
            frames,
            ..
        } = *self;
        let first = first.ok_or_else(|| io::Error::other("Ledger block has no first hour"))?;
        let last = hour.ok_or_else(|| io::Error::other("Ledger block has no hours"))?;
        let Prepared { scan, air, bombing } = *prepared;
        Ok(Box::new(BlockFacts {
            first,
            recording,
            air: Followed::of(air, scan.air)?,
            bombing: Followed::of(bombing, scan.bombing)?,
            last,
            #[cfg(any(test, feature = "test-support"))]
            frames,
        }))
    }
}

impl FamilyScan {
    /// Hours `first..=last`, after an observed `frame` they repeat in every
    /// field the scan reads (the event tables unchanged): what observing
    /// each would do, at once. An hour this cannot follow is an error, and
    /// the block is then observed hour by hour.
    fn quiet(
        &mut self,
        frame: &DecodedFrame,
        first: i32,
        last: i32,
        bombing: bool,
    ) -> io::Result<()> {
        let rows = if bombing {
            frame
                .complete::<tables::BuildingBombingDamage>()
                .ok()
                .map(|rows| rows.first().map(|row| row.id))
        } else {
            frame
                .complete::<tables::AirLossEvents>()
                .ok()
                .map(|rows| rows.first().map(|row| row.id))
        };
        let identities = frame.complete::<tables::Countries>().is_ok()
            && frame.complete::<tables::CountryLinks>().is_ok();
        let Some(first_event) = rows.filter(|_| identities) else {
            // The observed hour already ended presence and identities.
            if rows.is_none() {
                self.unread(first);
            }
            self.seal_presence();
            self.identities = None;
            self.presence_through = None;
            return Ok(());
        };
        // Every quiet hour continues the last: a gap or a failed identity
        // read is observed instead.
        let continues = self.presence_through.and_then(|hour| hour.checked_add(1)) == Some(first);
        let Some(invalid_tags) = self
            .identities
            .as_ref()
            .filter(|_| continues)
            .map(|identities| identities.invalid_tags)
        else {
            return Err(io::Error::other(
                "Ledger quiet hours follow no identity read",
            ));
        };
        let hours = usize::try_from(i64::from(last) - i64::from(first) + 1)
            .map_err(|_| io::Error::other("Ledger quiet hours reversed"))?;
        if invalid_tags != 0 {
            let count = invalid_tags
                .checked_mul(hours)
                .ok_or_else(|| io::Error::other("Ledger warning count overflow"))?;
            *self.warnings.entry(UNNAMED_TAG.into()).or_default() += count;
        }
        self.presence_through = Some(last);
        self.available = true;
        append_span(&mut self.observed, first, last);
        // An unchanged nonempty event table repeats its first ordinal every
        // hour, which the second check at the latest refuses.
        if let Some(id) = first_event {
            self.event(id)?;
            if last > first {
                self.event(id)?;
            }
        }
        Ok(())
    }

    /// Carry the extract's block `block` forward after the block's first
    /// hour `first`, which this scan observed: its ordinal state becomes
    /// `entry` (from [`continuation`]), its spans after `first` and its
    /// counts since `entered` add to this scan's, and its identities are
    /// this scan's from here.
    fn carry(&mut self, mut block: FamilyScan, entered: &Entered, first: i32, entry: Ordinals) {
        self.available |= block.available;
        append_after(&mut self.observed, &block.observed, first);
        append_after(&mut self.incomplete, &block.incomplete, first);
        // Presence is deferred to the last hour present; seal both so each
        // span list holds its hours, then join the block's after `first`.
        self.seal_presence();
        block.seal_presence();
        for (key, spans) in std::mem::take(&mut block.countries) {
            if spans.iter().any(|span| span.last_hour > first) {
                append_after(self.countries.entry(key).or_default(), &spans, first);
            }
        }
        for (reason, count) in std::mem::take(&mut block.warnings) {
            let before = entered.warnings.get(&reason).copied().unwrap_or(0);
            if count > before {
                *self.warnings.entry(reason).or_default() += count - before;
            }
        }
        (self.last_event, self.unread_since) = entry;
        self.presence_through = block.presence_through;
        self.identities = block.identities;
        self.country_keys = block.country_keys;
        self.retry = block.retry;
        self.entry = None;
    }
}

/// A scan's event ordinal and unread interval.
type Ordinals = (Option<u64>, Option<i32>);

/// The carried scan's ordinal state at the block's end, when the rest of the
/// block reads the state carried into it as the extract did; None when it
/// would not (the fold then observes the block hour by hour).
fn continuation(carried: &FamilyScan, entered: &Entered, block: &FamilyScan) -> Option<Ordinals> {
    let at_end = (block.last_event, block.unread_since);
    if (carried.last_event, carried.unread_since) == (entered.last_event, entered.unread_since) {
        return Some(at_end);
    }
    let entry = block.entry.as_ref()?;
    let since = carried.unread_since.or(entry.unread);
    match entry.used {
        // Nothing after the first hour read the ordinal: it is the carried
        // one, and an unread interval joins the carried one.
        None => Some((carried.last_event, since)),
        // The first read with events: its epoch and first ordinal check on
        // the carried state; after it both scans hold its last ordinal and
        // no unread interval.
        Some(EntryUse::Read { first, earliest }) => {
            let mut last = carried.last_event;
            if let Some(since) = since
                && last.is_some_and(|last| first <= last)
                && earliest >= since
            {
                last = None;
            }
            (!last.is_some_and(|last| first <= last)).then_some(at_end)
        }
        Some(EntryUse::Event) => None,
    }
}

/// Append `spans`' hours after `after` to `target`, joining adjacent hours.
fn append_after(target: &mut Vec<HourSpan>, spans: &[HourSpan], after: i32) {
    for span in spans.iter().filter(|span| span.last_hour > after) {
        append_span(target, span.first_hour.max(after + 1), span.last_hour);
    }
}

/// Append hours `first..=last` to `spans`, as appending each in turn does.
fn append_span(spans: &mut Vec<HourSpan>, first: i32, last: i32) {
    append_hour(spans, first);
    if let Some(span) = spans.last_mut() {
        span.last_hour = last;
    }
}

impl Prepared {
    /// Record the series' calls as 0 (air losses) and 1 (bombing) while a
    /// block's extract records them.
    fn record_calls(&mut self) {
        self.air.series.record_as(0);
        self.bombing.series.record_as(1);
    }

    /// Quiet hours `first..=last` after the observed `frame`, as observing
    /// each does: each scan, then each series through them.
    fn quiet(&mut self, frame: &DecodedFrame, first: i32, last: i32) -> io::Result<()> {
        self.scan.air.quiet(frame, first, last, false)?;
        self.scan.bombing.quiet(frame, first, last, true)?;
        for events in [&mut self.air, &mut self.bombing] {
            events.series.through(first);
            events.series.through(last);
        }
        Ok(())
    }
}

struct Target<'a> {
    prepared: &'a mut Prepared,
    keys: [Vec<u64>; 2],
}

impl ReplayTarget for Target<'_> {
    fn series(&mut self, series: u8) -> &mut Accumulator {
        if series == 0 {
            &mut self.prepared.air.series
        } else {
            &mut self.prepared.bombing.series
        }
    }
    fn key(&self, series: u8, key: u64) -> u64 {
        self.keys[usize::from(series.min(1))][key as usize]
    }
    fn family(&mut self, _: usize) -> io::Result<()> {
        Err(io::Error::other("The ledger records no calls of its own"))
    }
}

/// The ledger's observation of one hour's outcome: `Ok(true)` while it goes
/// on, `Ok(false)` once it refused the branch.
pub type Observed<'a> = dyn FnMut(i32, io::Result<()>) -> io::Result<bool> + 'a;

/// Fold one block's facts into the carried ledger. Returns how it folded and
/// the last hour it reached.
pub fn fold(
    prepared: &mut Prepared,
    facts: Box<dyn Any + Send>,
    replay: &Replay,
    index: &BlockIndex,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<(Folded, Option<i32>)> {
    let facts = facts
        .downcast::<BlockFacts>()
        .map_err(|_| io::Error::other("Pre-pass facts of another family"))?;
    let BlockFacts {
        first,
        recording,
        air,
        bombing,
        last,
        #[cfg(any(test, feature = "test-support"))]
        frames,
    } = *facts;
    #[cfg(any(test, feature = "test-support"))]
    count_frames(frames);
    check()?;
    let hour = first.frame.hour;
    let changes = FrameChanges::alone(&first.frame, replay.format);
    #[cfg(any(test, feature = "test-support"))]
    count_observed(1);
    let result = prepared.observe_changed(&first.frame, &changes);
    if !observed(hour, result)? {
        return Ok((Folded::Fast, Some(hour)));
    }
    let (Some(air_entry), Some(bombing_entry)) = (
        continuation(&prepared.scan.air, &first.air, &air.scan),
        continuation(&prepared.scan.bombing, &first.bombing, &bombing.scan),
    ) else {
        let reached = observe_block(prepared, replay, index, Some(hour), observed, check)?;
        return Ok((Folded::Fallback, reached.or(Some(hour))));
    };
    check()?;
    let keys = [
        series::number_names(&mut prepared.air.keys, &air.names),
        series::number_names(&mut prepared.bombing.keys, &bombing.names),
    ];
    let result = recording.replay(&mut Target {
        prepared: &mut *prepared,
        keys,
    });
    if !observed(last, result)? {
        return Ok((Folded::Fast, Some(last)));
    }
    prepared
        .scan
        .air
        .carry(air.scan, &first.air, hour, air_entry);
    prepared
        .scan
        .bombing
        .carry(bombing.scan, &first.bombing, hour, bombing_entry);
    prepared.air.leaves.extend(air.leaves);
    prepared.bombing.leaves.extend(bombing.leaves);
    Ok((Folded::Fast, Some(last)))
}

/// Observe the block hour by hour after `after` (every hour without it),
/// with the change facts of the block alone, as hourly preparation does.
/// Returns the last hour observed.
pub fn observe_block(
    prepared: &mut Prepared,
    replay: &Replay,
    index: &BlockIndex,
    after: Option<i32>,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<Option<i32>> {
    let mut tracker = FrameChangeTracker::new();
    let mut reached = None;
    let mut stopped = false;
    let mut failure = None;
    let read = replay.read_block_alone(
        index,
        TABLES,
        |raw, frame| {
            let changes = tracker.observe_updates(frame, Arc::new(raw), replay.format);
            if stopped || after.is_some_and(|after| frame.hour <= after) {
                return Ok(());
            }
            reached = Some(frame.hour);
            #[cfg(any(test, feature = "test-support"))]
            count_observed(1);
            let result = prepared.observe_changed(frame, &changes);
            match observed(frame.hour, result) {
                Ok(live) => {
                    stopped = !live;
                    Ok(())
                }
                Err(error) => {
                    failure = Some(error);
                    Err(io::Error::other("Ledger observation stopped"))
                }
            }
        },
        || check(),
    );
    if let Some(error) = failure {
        return Err(error);
    }
    read?;
    Ok(reached)
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static OBSERVED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static FRAMES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(any(test, feature = "test-support"))]
fn count_observed(hours: usize) {
    OBSERVED.with(|count| count.set(count.get() + hours));
}

#[cfg(any(test, feature = "test-support"))]
fn count_frames(frames: usize) {
    FRAMES.with(|count| count.set(count.get() + frames));
}

/// Hours the fold observed on the carried ledger (first hours and hours
/// observed hour by hour), and frames the folded blocks' extracts were given
/// or built, on this thread.
#[cfg(any(test, feature = "test-support"))]
pub fn counts() -> (usize, usize) {
    (
        OBSERVED.with(std::cell::Cell::get),
        FRAMES.with(std::cell::Cell::get),
    )
}

/// Add counts made on another thread to this thread's.
#[cfg(any(test, feature = "test-support"))]
pub fn add_counts((observed, frames): (usize, usize)) {
    count_observed(observed);
    count_frames(frames);
}
