//! A branch's newest naval battle window prepared first, from its last block
//! and the shortest look-back proven to hold every fact its page carries, so
//! the window and the manifest answer before forward preparation reaches
//! them.
//!
//! A look-back observes the branch's blocks from one of them through the
//! last, each read alone with the battle census tables and a change tracker
//! of its own, on a fresh scan, as the naval family observes them, and builds
//! the last window's page as forward preparation's publication of the last
//! block does. That page is forward preparation's when the look-back starts at
//! the branch's first block (it is then forward preparation's walk of the
//! branch), or when every fact the page carries that the earlier blocks could
//! decide is proven absent from them. The earlier blocks are read once, by
//! the changes of the combat and result keys and of every source field alone
//! ([`Replay::read_block_changes`]), each block on its own on spare cores and
//! joined in block order, an open combat's presence continuing across a
//! block boundary without a gap:
//! - Results. A result's episode, its id, first read and observed battle
//!   depend on every earlier read of its reference, so no result on the page
//!   has a reference read, or named by a source, in an earlier block.
//! - Sources. A source's validity depends on the sources before it: each one
//!   the look-back read has an id above, and a game hour at or after, every
//!   earlier source's; a persistent one names a combat no earlier persistent
//!   source named, and a result no earlier source named and no earlier block
//!   read. Each source read at the look-back's first hour dates at or after
//!   the hour forward preparation observed before it.
//! - Combats. Without a recorded gap before the look-back, forward
//!   preparation carries the combats open at the hour before its first: no
//!   combat on the page opening at the look-back's first hour, and none a
//!   source the look-back read names, is open there. After a gap forward
//!   preparation closes every combat, as the look-back starts.
//! - Places and availability. A result named by its date reads whether that
//!   hour was recorded, and the manifest whether any hour was available: both
//!   are taken from every hour of the earlier blocks with the look-back's.
//!
//! A look-back not proven moves back to the block of the earliest earlier
//! fact its failure names (for an open combat, the first hour of its
//! presence since the last recorded gap), and after its first retry at least
//! doubles its length. A look-back that would start at the branch's first
//! block would be forward preparation's walk of the branch: none is tried,
//! and the window is left to forward preparation. A block that cannot be
//! read publishes nothing: forward preparation reports the error itself. The page is saved under
//! names of its own, with the bytes forward preparation writes for the
//! window.
use super::*;
use dashboard_base::prepared::{
    frame_changes::FrameChangeTracker,
    newest::{Outcome, Report, Tried, Unproven},
    series,
};
use replay::{
    container::{BlockChange, TableFields},
    rows::NavalResultSourceRow,
};
use serde_json::Value;

/// What is saved beside a newest window's page.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Meta {
    /// The window's position in its branch.
    window: usize,
    interval: BattleWindow,
    /// Whether any hour of the branch is available, as forward preparation's
    /// manifest reads it after the last block.
    available: bool,
}

/// A branch's newest window, proven equal to forward preparation's.
pub struct Newest {
    branch: u64,
    reply: NavalBattlesReply,
    meta: Meta,
    /// Blocks observed over every look-back tried, the last block's included.
    #[cfg(any(test, feature = "test-support"))]
    pub blocks_read: usize,
    /// Blocks before the first look-back's read by their changes.
    #[cfg(any(test, feature = "test-support"))]
    pub earlier_blocks: usize,
}

fn page_section(branch: u64) -> String {
    format!("naval-battles-newest-{branch}")
}

fn meta_section(branch: u64) -> String {
    format!("naval-battle-newest-{branch}")
}

/// A source row read complete: its block among the earlier blocks read, and
/// the hour of the read.
#[derive(Debug, PartialEq)]
pub(super) struct SourceRead {
    block: usize,
    hour: i32,
    row: NavalResultSourceRow,
}

/// What the branch's blocks before a look-back hold that its page depends
/// on, each block's own facts at its position among the blocks read, so
/// every shorter prefix answers a later look-back.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Earlier {
    /// Each block's recorded hours.
    recorded: Vec<Vec<HourSpan>>,
    /// Each block's last recorded hour.
    last_hours: Vec<Option<i32>>,
    /// Whether any hour of each block was available.
    available: Vec<bool>,
    /// The combats a complete census held at each block's last hour, each
    /// with the first hour of its presence since the last recorded gap or
    /// incomplete census.
    pub(super) open: Vec<HashMap<u64, i32>>,
    /// Each result reference's first block and hour read in a complete
    /// census.
    results: HashMap<u64, (usize, i32)>,
    /// Every source row read in a complete source table.
    sources: Vec<SourceRead>,
}

impl Earlier {
    /// The last hour recorded before the prefix's end.
    fn last_hour(&self, blocks: usize) -> Option<i32> {
        self.last_hours[..blocks]
            .iter()
            .rev()
            .find_map(|hour| *hour)
    }
}

/// The changes the earlier blocks are read by: the combat and result keys,
/// and every source field.
fn earlier_fields() -> [TableFields; 3] {
    [
        TableFields::keys(TableId::NavalCombats),
        TableFields::keys(TableId::NavalResults),
        TableFields::all(TableId::NavalResultSources),
    ]
}

/// One block's facts, read alone.
struct BlockFacts {
    recorded: Vec<HourSpan>,
    available: bool,
    /// The combats a complete census held at the block's last hour, each
    /// with the first hour of its presence in the block since its last
    /// recorded gap or incomplete census: the block's first hour when present
    /// since then.
    open: HashMap<u64, i32>,
    /// Each result reference's first hour read in a complete census.
    results: HashMap<u64, i32>,
    /// Every source row read in a complete source table, with the hour.
    sources: Vec<(i32, NavalResultSourceRow)>,
}

/// `index`'s facts, read only at the hours its combat keys, result keys or
/// source rows change, with no fact of the blocks before it.
fn block_facts(
    replay: &Replay,
    index: &replay::container::BlockIndex,
    fields: &[TableFields],
    cancelled: &AtomicBool,
) -> io::Result<BlockFacts> {
    let mut facts = BlockFacts {
        recorded: Vec::new(),
        available: false,
        open: HashMap::new(),
        results: HashMap::new(),
        sources: Vec::new(),
    };
    let mut previous: Option<i32> = None;
    replay.read_block_changes(
        index,
        fields,
        |change| {
            match change {
                BlockChange::Changed {
                    frame,
                    tables: changed,
                    ..
                } => {
                    // Forward preparation closes every combat after a gap,
                    // and opens again those its census holds there, whether
                    // or not their keys changed.
                    let gap = previous.and_then(|hour| hour.checked_add(1)) != Some(frame.hour);
                    if gap {
                        facts.open.clear();
                    }
                    previous = Some(frame.hour);
                    span(&mut facts.recorded, frame.hour);
                    // Quiet hours hold the availability of the hour before.
                    facts.available |= frame_available(frame);
                    if gap || changed.contains(&TableId::NavalCombats) {
                        // An incomplete census closes every combat.
                        let present = frame.complete::<tables::NavalCombats>().unwrap_or_default();
                        let open = std::mem::take(&mut facts.open);
                        facts.open = present
                            .iter()
                            .map(|row| (row.id, open.get(&row.id).copied().unwrap_or(frame.hour)))
                            .collect();
                    }
                    if changed.contains(&TableId::NavalResults)
                        && let Ok(rows) = frame.complete::<tables::NavalResults>()
                    {
                        for row in rows {
                            facts.results.entry(row.id).or_insert(frame.hour);
                        }
                    }
                    if changed.contains(&TableId::NavalResultSources)
                        && let Ok(rows) = frame.complete::<tables::NavalResultSources>()
                    {
                        facts
                            .sources
                            .extend(rows.iter().map(|row| (frame.hour, row.clone())));
                    }
                }
                BlockChange::Quiet(quiet) => {
                    for &hour in quiet.hours() {
                        if previous.and_then(|last| last.checked_add(1)) != Some(hour) {
                            // The combats still present open again after the
                            // gap.
                            for first in facts.open.values_mut() {
                                *first = hour;
                            }
                        }
                        previous = Some(hour);
                        span(&mut facts.recorded, hour);
                    }
                }
            }
            Ok(())
        },
        || check_cancel(cancelled),
    )?;
    Ok(facts)
}

/// The blocks' facts joined in block order. A block's combat present since
/// its first hour continues the presence the block before held at its last
/// hour when that hour is the one before; every other fact is the block's
/// own.
fn join_blocks(blocks: impl Iterator<Item = io::Result<BlockFacts>>) -> io::Result<Earlier> {
    let mut facts = Earlier::default();
    let mut open: HashMap<u64, i32> = HashMap::new();
    let mut previous: Option<i32> = None;
    for (at, block) in blocks.enumerate() {
        let block = block?;
        if let Some(first) = block.recorded.first().map(|span| span.first_hour) {
            let continues = previous.and_then(|hour| hour.checked_add(1)) == Some(first);
            open = block
                .open
                .into_iter()
                .map(|(id, opened)| {
                    let carried = open
                        .get(&id)
                        .copied()
                        .filter(|_| continues && opened == first);
                    (id, carried.unwrap_or(opened))
                })
                .collect();
            previous = block.recorded.last().map(|span| span.last_hour);
        }
        for (id, hour) in block.results {
            facts.results.entry(id).or_insert((at, hour));
        }
        facts
            .sources
            .extend(block.sources.into_iter().map(|(hour, row)| SourceRead {
                block: at,
                hour,
                row,
            }));
        facts
            .last_hours
            .push(block.recorded.last().map(|span| span.last_hour));
        facts.recorded.push(block.recorded);
        facts.available.push(block.available);
        facts.open.push(open.clone());
    }
    Ok(facts)
}

/// The facts of `blocks`, each read alone on `workers` threads within the
/// shared block-read core budget ([`dashboard_core::blocks::scan_blocks_by`])
/// and joined in block order: the facts of reading them in order, and the
/// first error in block order.
pub(super) fn read_earlier(
    replay: &Replay,
    blocks: &[usize],
    workers: usize,
    cancelled: &AtomicBool,
) -> io::Result<Earlier> {
    let fields = earlier_fields();
    let indices = blocks.iter().map(|&block| &replay.index[block]).collect();
    dashboard_core::blocks::scan_blocks_by(
        indices,
        workers.saturating_sub(1),
        cancelled,
        &|index| block_facts(replay, index, &fields, cancelled),
        |reads| join_blocks(reads.map(|(_, read)| read)),
    )
}

/// Threads the earlier blocks are read on: one per core.
fn workers() -> usize {
    std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
}

/// [`read_earlier`] block after block on one thread, carrying the open
/// combats from each block into the next: the scan the parallel one equals.
#[cfg(test)]
pub(super) fn read_earlier_in_order(
    replay: &Replay,
    blocks: &[usize],
    cancelled: &AtomicBool,
) -> io::Result<Earlier> {
    let fields = earlier_fields();
    let mut facts = Earlier::default();
    // The combats open at the last hour visited, and that hour.
    let mut open: HashMap<u64, i32> = HashMap::new();
    let mut previous: Option<i32> = None;
    for (at, &block) in blocks.iter().enumerate() {
        let mut recorded = Vec::new();
        let mut available = false;
        replay.read_block_changes(
            &replay.index[block],
            &fields,
            |change| {
                match change {
                    BlockChange::Changed {
                        frame,
                        tables: changed,
                        ..
                    } => {
                        // Forward preparation closes every combat after a
                        // gap, and opens again those its census holds there.
                        let gap = previous.and_then(|hour| hour.checked_add(1)) != Some(frame.hour);
                        if gap {
                            open.clear();
                        }
                        previous = Some(frame.hour);
                        span(&mut recorded, frame.hour);
                        // Quiet hours hold the availability of the hour
                        // before.
                        available |= frame_available(frame);
                        if gap || changed.contains(&TableId::NavalCombats) {
                            // An incomplete census closes every combat.
                            let present =
                                frame.complete::<tables::NavalCombats>().unwrap_or_default();
                            open = present
                                .iter()
                                .map(|row| {
                                    (row.id, open.get(&row.id).copied().unwrap_or(frame.hour))
                                })
                                .collect();
                        }
                        if changed.contains(&TableId::NavalResults)
                            && let Ok(rows) = frame.complete::<tables::NavalResults>()
                        {
                            for row in rows {
                                facts.results.entry(row.id).or_insert((at, frame.hour));
                            }
                        }
                        if changed.contains(&TableId::NavalResultSources)
                            && let Ok(rows) = frame.complete::<tables::NavalResultSources>()
                        {
                            facts.sources.extend(rows.iter().map(|row| SourceRead {
                                block: at,
                                hour: frame.hour,
                                row: row.clone(),
                            }));
                        }
                    }
                    BlockChange::Quiet(quiet) => {
                        for &hour in quiet.hours() {
                            if previous.and_then(|last| last.checked_add(1)) != Some(hour) {
                                // The combats still present open again after
                                // the gap.
                                for first in open.values_mut() {
                                    *first = hour;
                                }
                            }
                            previous = Some(hour);
                            span(&mut recorded, hour);
                        }
                    }
                }
                Ok(())
            },
            || check_cancel(cancelled),
        )?;
        facts
            .last_hours
            .push(recorded.last().map(|span| span.last_hour));
        facts.recorded.push(recorded);
        facts.available.push(available);
        facts.open.push(open.clone());
    }
    Ok(facts)
}

/// One look-back's observation: its scan, every source row it read with the
/// hour of the read, and its first hour.
struct LookBack {
    scan: Scan,
    sources: Vec<(i32, NavalResultSourceRow)>,
    first_hour: Option<i32>,
}

/// Observes `blocks` (the branch's, from one through its last) as the naval
/// family does: each block read alone, with change facts of its own, and the
/// results extended at its end, where forward preparation publishes it.
fn look_back(replay: &Replay, blocks: &[usize], cancelled: &AtomicBool) -> io::Result<LookBack> {
    let format = replay.format;
    let mut scan = Scan::default();
    let mut sources = Vec::new();
    let mut first_hour = None;
    for &block in blocks {
        let mut tracker = FrameChangeTracker::new();
        replay.read_block_alone(
            &replay.index[block],
            &CENSUS_TABLES,
            |_, frame| {
                first_hour.get_or_insert(frame.hour);
                if let Ok(rows) = frame.complete::<tables::NavalResultSources>() {
                    sources.extend(rows.iter().map(|row| (frame.hour, row.clone())));
                }
                let changes = tracker.observe(frame, format);
                scan.observe_prepared(frame, &changes);
                Ok(())
            },
            || check_cancel(cancelled),
        )?;
        // As forward preparation's publication after each block.
        scan.extend_results();
        scan.dirty.clear();
    }
    Ok(LookBack {
        scan,
        sources,
        first_hour,
    })
}

/// Appends `span` to `spans`, joining it to the last when it continues it.
fn join(spans: &mut Vec<HourSpan>, span: &HourSpan) {
    if let Some(last) = spans
        .last_mut()
        .filter(|last| last.last_hour.checked_add(1) == Some(span.first_hour))
    {
        last.last_hour = span.last_hour;
    } else {
        spans.push(HourSpan {
            first_hour: span.first_hour,
            last_hour: span.last_hour,
        });
    }
}

/// A result on the page was read before the look-back.
const RESULT_READ: &str = "a result read before the look-back";
/// A result on the page, or one a source the look-back read names, was named
/// by an earlier source.
const RESULT_NAMED: &str = "a result an earlier source named";
/// A source the look-back read is not after every earlier source.
const SOURCE_ORDER: &str = "a source not after every earlier source";
/// A persistent source the look-back read names a combat an earlier
/// persistent source named.
const COMBAT_NAMED: &str = "a combat an earlier persistent source named";
/// A source read at the look-back's first hour dates before the hour
/// observed before it.
const SOURCE_DATE: &str = "a source dated before the hour observed before the look-back";
/// A combat on the page, or one a source the look-back read names, was open
/// at the hour before the look-back.
const OPEN_COMBAT: &str = "a combat open at the hour before the look-back";

/// The earliest hour a look-back whose first hour is `first` must start by,
/// for a fact of its page that `earlier`'s first `from` blocks decide, with
/// that fact; None when every such fact is proven.
fn unproven(
    look: &LookBack,
    reply: &NavalBattlesReply,
    earlier: &Earlier,
    from: usize,
    first: i32,
) -> Option<Unproven> {
    let mut before: Option<Unproven> = None;
    let mut back = |check: &'static str, hour: i32| {
        if before.as_ref().is_none_or(|earlier| hour < earlier.hour) {
            before = Some(Unproven { check, hour });
        }
    };
    let read_before = |reference: &u64| {
        earlier
            .results
            .get(reference)
            .filter(|(block, _)| *block < from)
            .map(|(_, hour)| *hour)
    };
    let sources: Vec<&SourceRead> = earlier
        .sources
        .iter()
        .filter(|source| source.block < from)
        .collect();
    // Each reference's first earlier read: results named by any source (an
    // invalid one marks its result too), combats by a persistent one.
    let mut named_results: HashMap<u64, i32> = HashMap::new();
    let mut named_combats: HashMap<u64, i32> = HashMap::new();
    let mut highest: Option<(u64, i32)> = None;
    for source in &sources {
        named_results
            .entry(source.row.result)
            .or_insert(source.hour);
        if !source.row.transient {
            named_combats
                .entry(source.row.combat)
                .or_insert(source.hour);
        }
        highest = Some(
            highest.map_or((source.row.id, source.row.game_hour), |(id, hour)| {
                (id.max(source.row.id), hour.max(source.row.game_hour))
            }),
        );
    }
    let on_page: HashSet<&str> = reply.rows.iter().map(|row| row.id.as_str()).collect();
    let page = || {
        look.scan
            .episodes
            .iter()
            .filter(|episode| on_page.contains(episode.id.as_str()))
    };
    for episode in page().filter(|episode| episode.kind == Kind::Result) {
        if let Some(hour) = read_before(&episode.reference) {
            back(RESULT_READ, hour);
        }
        if let Some(&hour) = named_results.get(&episode.reference) {
            back(RESULT_NAMED, hour);
        }
    }
    let previous = earlier.last_hour(from);
    for (hour, row) in &look.sources {
        if let Some((id, game_hour)) = highest
            && (row.id <= id || row.game_hour < game_hour)
        {
            for source in sources
                .iter()
                .filter(|source| source.row.id >= row.id || source.row.game_hour > row.game_hour)
            {
                back(SOURCE_ORDER, source.hour);
            }
        }
        if !row.transient {
            if let Some(&hour) = named_combats.get(&row.combat) {
                back(COMBAT_NAMED, hour);
            }
            if let Some(&hour) = named_results.get(&row.result) {
                back(RESULT_NAMED, hour);
            }
            if let Some(hour) = read_before(&row.result) {
                back(RESULT_READ, hour);
            }
        }
        if *hour == first
            && let Some(previous) = previous
            && row.game_hour < previous
        {
            back(SOURCE_DATE, previous);
        }
    }
    if previous.and_then(|hour| hour.checked_add(1)) == Some(first) {
        let open = &earlier.open[from - 1];
        let opening = page()
            .filter(|episode| episode.kind == Kind::Combat && episode.first == first)
            .map(|episode| episode.reference);
        for combat in opening.chain(look.sources.iter().map(|(_, row)| row.combat)) {
            if let Some(&opened) = open.get(&combat) {
                back(OPEN_COMBAT, opened);
            }
        }
    }
    before
}

/// A look-back's outcome.
enum Attempt {
    /// Its page is forward preparation's, with the branch's availability.
    Proven(NavalBattlesReply, bool),
    /// The look-back must start at or before the hour its failed check
    /// names.
    Before(Unproven),
}

/// One look-back from the branch's block `from` (of `blocks`) through its
/// last, `ordinal`, against `earlier`, the facts of the blocks before the
/// last.
fn attempt(
    replay: &Replay,
    blocks: &[usize],
    from: usize,
    window: &BattleWindow,
    earlier: &Earlier,
    cancelled: &AtomicBool,
) -> io::Result<Attempt> {
    let ordinal = blocks.len() - 1;
    let mut look = look_back(replay, &blocks[from..], cancelled)?;
    let facts = (from != 0).then_some(earlier);
    let mut available = look.scan.available;
    if let Some(facts) = facts {
        available |= facts.available[..from].iter().any(|available| *available);
        let mut recorded = Vec::new();
        for span in facts.recorded[..from]
            .iter()
            .flatten()
            .chain(&look.scan.recorded_hours)
        {
            join(&mut recorded, span);
        }
        look.scan.recorded_hours = recorded;
    }
    let reply = look
        .scan
        .window_records(window)
        .reply_window(ordinal, window)?;
    if let Some(facts) = facts {
        let first = look
            .first_hour
            .unwrap_or(replay.index[blocks[from]].first_hour);
        if let Some(why) = unproven(&look, &reply, facts, from, first) {
            return Ok(Attempt::Before(why));
        }
    }
    Ok(Attempt::Proven(reply, available))
}

/// The branch's newest window and availability, when a look-back proves
/// them; None when the branch has no block, the look-back would start at the
/// branch's first block, or a block a look-back or the earlier blocks' facts
/// read cannot be read.
pub fn prepare(source: &Source, branch: u64, cancelled: &AtomicBool) -> io::Result<Option<Newest>> {
    Ok(prepare_reported(source, branch, cancelled)?.0)
}

/// [`prepare`], with what it did: the earlier blocks read, every look-back
/// tried and the outcome.
pub fn prepare_reported(
    source: &Source,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<(Option<Newest>, Report)> {
    let mut report = Report::default();
    let newest = prepare_into(source, branch, cancelled, &mut report)?;
    if newest.is_some() {
        report.outcome = Outcome::Proven;
    }
    Ok((newest, report))
}

fn prepare_into(
    source: &Source,
    branch: u64,
    cancelled: &AtomicBool,
    report: &mut Report,
) -> io::Result<Option<Newest>> {
    let replay = source.replay_ref();
    let blocks: Vec<usize> = replay
        .index
        .iter()
        .enumerate()
        .filter(|(_, index)| index.branch == branch)
        .map(|(at, _)| at)
        .collect();
    report.blocks = blocks.len();
    let Some(last) = blocks.len().checked_sub(1) else {
        return Ok(None);
    };
    let windows = windows(replay, branch);
    let window = &windows[last];
    let mut from = last;
    // From the branch's first block the look-back is forward preparation's
    // walk of the branch: the window is left to it.
    if from == 0 {
        report.outcome = Outcome::BranchStart(None);
        return Ok(None);
    }
    // Every look-back starts after the first block, so each needs the
    // earlier blocks' facts: read once, before the first.
    let started = std::time::Instant::now();
    let earlier = match read_earlier(replay, &blocks[..last], workers(), cancelled) {
        Ok(earlier) => earlier,
        Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
        Err(error) => {
            report.outcome = Outcome::Unreadable(error.to_string());
            return Ok(None);
        }
    };
    report.earlier = last;
    report.earlier_took = started.elapsed();
    #[cfg(any(test, feature = "test-support"))]
    let mut read = 0;
    let mut retries = 0;
    loop {
        check_cancel(cancelled)?;
        let started = std::time::Instant::now();
        let outcome = attempt(replay, &blocks, from, window, &earlier, cancelled);
        #[cfg(any(test, feature = "test-support"))]
        {
            read += blocks.len() - from;
        }
        report.tried.push(Tried {
            from,
            blocks: blocks.len() - from,
            took: started.elapsed(),
            unproven: match &outcome {
                Ok(Attempt::Before(why)) => Some(why.clone()),
                _ => None,
            },
        });
        let before = match outcome {
            Ok(Attempt::Proven(reply, available)) => {
                return Ok(Some(Newest {
                    branch,
                    reply,
                    meta: Meta {
                        window: last,
                        interval: window.clone(),
                        available,
                    },
                    #[cfg(any(test, feature = "test-support"))]
                    blocks_read: read,
                    #[cfg(any(test, feature = "test-support"))]
                    earlier_blocks: earlier.recorded.len(),
                }));
            }
            Ok(Attempt::Before(why)) => why,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
            Err(error) => {
                report.outcome = Outcome::Unreadable(error.to_string());
                return Ok(None);
            }
        };
        // The block holding that hour, or the first after the gap holding it.
        let hinted = blocks.partition_point(|&at| replay.index[at].last_hour < before.hour);
        let doubled = blocks.len().saturating_sub(2 * (blocks.len() - from));
        from = if retries == 0 {
            hinted
        } else {
            hinted.min(doubled)
        }
        .min(from - 1);
        if from == 0 {
            report.outcome = Outcome::BranchStart(Some(before));
            return Ok(None);
        }
        retries += 1;
    }
}

impl Newest {
    /// Writes the page, with the bytes forward preparation writes for the
    /// window, and what the query checks it by.
    pub fn write(&self, writer: &mut store::Writer) -> io::Result<()> {
        series::write_reply(writer, &page_section(self.branch), 0, &self.reply)?;
        series::write_reply(writer, &meta_section(self.branch), 0, &self.meta)
    }
}

/// Whether `meta` names the branch's current last window.
fn current(replay: &Replay, branch: u64, meta: &Meta) -> bool {
    let windows = windows(replay, branch);
    windows.len().checked_sub(1) == Some(meta.window)
        && windows.last().is_some_and(|window| {
            window.first_hour == meta.interval.first_hour
                && window.last_hour == meta.interval.last_hour
        })
}

/// Whether the store holds the newest window of the branch's current last
/// block.
pub fn published(
    store: &store::Store,
    key: &store::SourceKey,
    replay: &Replay,
    branch: u64,
) -> io::Result<bool> {
    Ok(
        series::read_reply::<Meta>(store, key, &meta_section(branch), 0)?
            .is_some_and(|meta| current(replay, branch, &meta)),
    )
}

/// A naval battle query answered from the branch's newest window before
/// forward preparation covers the branch: its manifest, and its newest
/// window's page. None for every other query, and before the newest window of
/// the branch's current last block is published.
pub fn query(
    source: &Source,
    product: &str,
    branch: u64,
    params: &Value,
    store: &store::Store,
    key: &store::SourceKey,
) -> io::Result<Option<Value>> {
    if !matches!(product, "naval-battles-manifest" | "naval-battles") {
        return Ok(None);
    }
    let replay = source.replay_ref();
    let Some(meta) = series::read_reply::<Meta>(store, key, &meta_section(branch), 0)? else {
        return Ok(None);
    };
    if !current(replay, branch, &meta) {
        return Ok(None);
    }
    if product == "naval-battles-manifest" {
        let params: NavalBattlesManifestParams = serde_json::from_value(params.clone())?;
        NavalBattlesManifest::validate(&params).map_err(dashboard_core::invalid)?;
        return Ok(Some(serde_json::to_value(bounded(
            NavalBattlesManifestReply {
                windows: windows(replay, branch),
                available: meta.available,
                unavailable: (!meta.available).then(|| "Not in this recording".into()),
                content_warnings: Vec::new(),
            },
        )?)?));
    }
    let params: NavalBattlesParams = serde_json::from_value(params.clone())?;
    NavalBattles::validate(&params).map_err(dashboard_core::invalid)?;
    if params.window != meta.window {
        return Ok(None);
    }
    let Some(reply) =
        series::read_reply::<NavalBattlesReply>(store, key, &page_section(branch), 0)?
    else {
        return Ok(None);
    };
    Ok(Some(serde_json::to_value(bounded(reply)?)?))
}
