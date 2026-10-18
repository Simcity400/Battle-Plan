//! A branch's newest block window prepared first, from its last block and a
//! look-back proven to hold every fact its page carries, so the window
//! answers before forward preparation reaches it.
//!
//! A look-back observes the branch's blocks from one of them through the last,
//! read ahead alone and observed in order on a fresh census configured as forward
//! preparation's, and
//! builds the last window's page as forward preparation's publication of the
//! last block does. That page is forward preparation's when the look-back
//! starts at the branch's first block (it is then forward preparation's walk
//! of the branch), or when it observed every fact the page carries:
//! - Battles. After a recorded gap (the branch's previous block ends before
//!   the hour before the look-back's first), forward preparation closes every
//!   battle and starts the look-back's first hour as the look-back does.
//!   Otherwise the look-back starts before the window, and every battle on
//!   the page started after its first hour, so it saw each one's whole
//!   course: its id and first hour, `Exact` or `By` start, counter base, runs,
//!   loss reads (the one carried into the window included) and prior peaks.
//!   A battle on the page already open at its first hour fails the proof,
//!   naming the hour before that battle's native duration counter began.
//! - Results. Every finalizer result the window observes covers no hour
//!   before the look-back's first, so the battle it closes, if any, is one
//!   the look-back saw.
//! - Equipment names. Outside a look-back starting after a gap, every loss
//!   read dated in the window names a variant whose naming the look-back read,
//!   or follows an hour whose variant table was not complete, where forward
//!   preparation forgets every name too.
//! - Availability. Only an available hour inside the look-back proves the
//!   manifest's availability; without one nothing is published.
//!
//! The window's reply carries the branch's diagnostics, which forward
//! preparation gathers over every block. Its own content warnings and the
//! catalogue's are the look-back's, configured alike. Its variant naming
//! refusals are those of every variant read on the branch, in whatever order:
//! the look-back's reads, and the earlier blocks' read by the changes of the
//! naming tables alone, each block on its own on spare cores. Its census's
//! own text refusals come from naming the
//! variants its loss reads use, each one a variant read on the branch; when
//! naming every variant read on the branch refuses nothing, forward
//! preparation's naming refuses nothing either. Otherwise the proven page
//! remains available and only its diagnostics are withheld.
//!
//! One read of the branch's blocks by their changes alone decides where the
//! look-back starts ([`start`]): the last block's by the combat and naming
//! tables' keys and every field of the progress, results and event coverage,
//! then each earlier block's, from the last back, by the combat and naming
//! tables' keys, each block read alone on spare cores and taken in that
//! order. It finds the latest block from which every check above holds as
//! far as the blocks' changes decide them: each battle on the page (every
//! combat the last block holds, those the block before holds at its last
//! hour when it adjoins the window, and those whose presence a result read
//! in the window may close) traced back through its presence in a complete,
//! read census, across adjoining blocks, to the hour forward preparation
//! opens it; the window's results' coverage; and each variant a progress row
//! or result in the window names, read in a complete variant read, or
//! forgotten at an incomplete variant table, before the read naming it. The
//! census look-back from that block, its blocks read ahead on spare cores,
//! is then proven by the checks above, which decide; one not proven
//! publishes nothing. A look-back that would start at the branch's first
//! block would be forward preparation's walk of the branch: none is tried,
//! and the window is left to forward preparation. A census refusal or a
//! block that cannot be read publishes nothing: forward preparation refuses
//! the branch or reports the error itself.
//!
//! The page is saved under names of its own, with the bytes forward
//! preparation writes for the window, as soon as it is proven ([`prove`],
//! [`Proven::write`]); the diagnostics follow ([`Proven::diagnose`],
//! [`Diagnosed::write`]) beside the families' preparation. Until they are
//! saved, the window's reply carries no content warnings and says so
//! (`warningsComplete: false`). A variant naming refusal withholds only
//! those diagnostics.
use super::*;
use dashboard_base::{
    prepared::{
        newest::{Outcome, Report, Tried, Unproven},
        series,
    },
    products::equipment::VariantCatalogue,
};
use replay::{container::BlockChange, container::BlockIndex, container::TableFields};
use replay_data::rows::Recorded;
use replay_frame::frame::Metrics;
use serde_json::Value;

/// What a look-back's census notes while it observes, for its proof.
#[derive(Default)]
pub(super) struct Check {
    /// The newest window's first hour.
    pub(super) window_first: i32,
    /// The earliest coverage start of a result matched at an hour of the
    /// window.
    pub(super) results_from: Option<i32>,
    /// The look-back observed an hour whose variant table was not complete.
    pub(super) forgot: bool,
    /// A loss read dated in the window named a variant whose naming the
    /// look-back neither read nor forgot.
    pub(super) unnamed: bool,
}

/// Which window a newest page is, saved beside it when it is proven.
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Position {
    /// The window's position in its branch.
    window: usize,
    interval: BattleWindow,
}

/// What is saved beside a newest window's page once its diagnostics are read.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Meta {
    /// The window's position in its branch.
    window: usize,
    interval: BattleWindow,
    /// The branch's content warnings as forward preparation publishes them,
    /// which the window's reply carries until forward preparation covers the
    /// branch.
    content_warnings: Vec<String>,
}

/// A branch's newest window, proven equal to forward preparation's, with its
/// diagnostics.
pub struct Newest {
    branch: u64,
    reply: LandBattlesReply,
    position: Position,
    meta: Meta,
    /// Blocks the look-back observed, the last block's included.
    #[cfg(any(test, feature = "test-support"))]
    pub blocks_read: usize,
    /// Blocks before the proven look-back's whose naming tables the
    /// diagnostics read.
    #[cfg(any(test, feature = "test-support"))]
    pub naming_blocks: usize,
}

fn page_section(branch: u64) -> String {
    format!("land-battles-newest-{branch}")
}

fn position_section(branch: u64) -> String {
    format!("land-battles-newest-window-{branch}")
}

fn meta_section(branch: u64) -> String {
    format!("land-battle-newest-diagnostics-{branch}")
}

/// A branch's newest window proven equal to forward preparation's, before
/// its diagnostics are read.
pub struct Proven {
    branch: u64,
    reply: LandBattlesReply,
    position: Position,
    census: Box<Census>,
    /// The branch's blocks before the proven look-back's.
    earlier: Vec<usize>,
    /// Blocks read over every look-back tried, the last block's included.
    #[cfg(any(test, feature = "test-support"))]
    pub blocks_read: usize,
}

/// A newest window's diagnostics, as forward preparation publishes them.
pub struct Diagnosed {
    branch: u64,
    meta: Meta,
}

/// A look-back's outcome.
enum Attempt {
    /// Its page is forward preparation's.
    Proven(Page),
    /// The look-back must start at or before the hour its failed check
    /// names.
    Before(Unproven),
}

/// A result read in the window covers hours before the look-back's first.
const RESULT_COVERAGE: &str = "a result's coverage";
/// The look-back starts at the window's first hour, after none of the
/// battles the window's first hour closes.
const WINDOW_START: &str = "the window's first hour";
/// A loss read in the window names a variant whose naming the look-back
/// neither read nor forgot.
const VARIANT_NAMING: &str = "an unread variant's naming";
/// A battle on the page was open at the look-back's first hour.
const OPEN_BATTLE: &str = "a battle open at the look-back's first hour";

/// A proven look-back's page and the census that built it, which holds the
/// availability it proved and the diagnostics of the blocks it read.
struct Page {
    reply: LandBattlesReply,
    census: Box<Census>,
}

/// The branch's newest window, when a look-back proves it, its availability
/// and its diagnostics; None when the branch has no block, the look-back
/// would start at the branch's first block, forward preparation would refuse
/// or fail where the look-back or the diagnostics read, no hour read is
/// available, or naming the variants read refuses text.
pub fn prepare(
    source: &Source,
    branch: u64,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Option<Newest>> {
    Ok(prepare_reported(source, branch, context, cancelled)?.0)
}

/// [`prepare`], with what it did: the look-back and its diagnostics.
pub fn prepare_reported(
    source: &Source,
    branch: u64,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<(Option<Newest>, Report)> {
    let (proven, mut report) = prove_reported(source, branch, context, cancelled)?;
    let Some(proven) = proven else {
        return Ok((None, report));
    };
    #[cfg(any(test, feature = "test-support"))]
    let (blocks_read, naming_blocks) = (proven.blocks_read, proven.earlier.len());
    let (reply, position) = (proven.reply.clone(), proven.position.clone());
    let Some(diagnosed) =
        proven.diagnose_reported(source.replay_ref(), workers(), cancelled, &mut report)?
    else {
        return Ok((None, report));
    };
    Ok((
        Some(Newest {
            branch,
            reply,
            meta: diagnosed.meta,
            position,
            #[cfg(any(test, feature = "test-support"))]
            blocks_read,
            #[cfg(any(test, feature = "test-support"))]
            naming_blocks,
        }),
        report,
    ))
}

/// The branch's newest window, when the look-back proves it and an hour it
/// read is available; None where the look-back leaves it to forward preparation.
pub fn prove(
    source: &Source,
    branch: u64,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Option<Proven>> {
    Ok(prove_reported(source, branch, context, cancelled)?.0)
}

/// [`prove`], with its change-read start, look-back and outcome.
pub fn prove_reported(
    source: &Source,
    branch: u64,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<(Option<Proven>, Report)> {
    let mut report = Report::default();
    let proven = prove_into(source, branch, context, cancelled, &mut report)?;
    Ok((proven, report))
}

fn prove_into(
    source: &Source,
    branch: u64,
    context: &LoadContext,
    cancelled: &AtomicBool,
    report: &mut Report,
) -> io::Result<Option<Proven>> {
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
    // One read of the blocks' changes, from the last back, decides where the
    // look-back starts.
    let started = std::time::Instant::now();
    let decided = start(replay, &blocks, window, cancelled, &mut report.earlier);
    report.earlier_took = started.elapsed();
    let from = match decided {
        Ok(Start::Block(from)) => from,
        // From the branch's first block the look-back is forward
        // preparation's walk of the branch: the window is left to it.
        Ok(Start::BranchStart(why)) => {
            report.outcome = Outcome::BranchStart(why);
            return Ok(None);
        }
        Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
        Err(error) => {
            report.outcome = Outcome::Unreadable(error.to_string());
            return Ok(None);
        }
    };
    check_cancel(cancelled)?;
    let started = std::time::Instant::now();
    let attempt = look_back(
        source,
        context,
        &blocks[from..],
        false,
        after_gap(replay, &blocks, from),
        last,
        window,
        cancelled,
    );
    report.tried.push(Tried {
        from,
        blocks: blocks.len() - from,
        took: started.elapsed(),
        unproven: match &attempt {
            Ok(Attempt::Before(why)) => Some(why.clone()),
            _ => None,
        },
    });
    let page = match attempt {
        Ok(Attempt::Proven(page)) => page,
        // The checks decide: a fact the blocks' changes did not show leaves
        // the window to forward preparation.
        Ok(Attempt::Before(why)) => {
            report.outcome = Outcome::Unproven(why);
            return Ok(None);
        }
        Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
        Err(error) => {
            report.outcome = Outcome::Unreadable(error.to_string());
            return Ok(None);
        }
    };
    if !page.census.available {
        report.outcome = Outcome::Unavailable;
        return Ok(None);
    }
    report.outcome = Outcome::Proven;
    Ok(Some(Proven {
        branch,
        reply: page.reply,
        position: Position {
            window: last,
            interval: window.clone(),
        },
        census: page.census,
        earlier: blocks[..from].to_vec(),
        #[cfg(any(test, feature = "test-support"))]
        blocks_read: blocks.len() - from,
    }))
}

/// Whether the branch's block `at` (of `blocks`) follows a recorded gap:
/// forward preparation observes no hour just before its first.
fn after_gap(replay: &Replay, blocks: &[usize], at: usize) -> bool {
    at > 0
        && replay.index[blocks[at - 1]]
            .last_hour
            .checked_add(1)
            .is_some_and(|next| next < replay.index[blocks[at]].first_hour)
}

/// What one block holds that the look-back's start depends on, read by its
/// changes alone.
#[derive(Default)]
struct Facts {
    first_hour: i32,
    last_hour: i32,
    /// Each combat's presence, in hour order: runs of read hours at which a
    /// complete census holds it, ended by a recorded gap, an hour whose
    /// census is unread or incomplete, or its absence, where forward
    /// preparation closes its battle.
    present: HashMap<u64, Vec<(i32, i32)>>,
    /// Each variant's first read in a complete variant read: of every
    /// variant in the last block, of the variants the window names before.
    read: HashMap<u64, i32>,
    /// The first hour whose variant table was not complete, where a
    /// look-back forgets every name.
    forgot: Option<i32>,
}

impl Facts {
    /// One hour, `hour`, read with `metrics` from `frame`, the frame holding
    /// the watched tables there.
    #[allow(clippy::too_many_arguments)]
    fn hour(
        &mut self,
        open: &mut HashMap<u64, usize>,
        previous: &mut Option<i32>,
        format: replay_data::format::FormatVersion,
        frame: &DecodedFrame,
        hour: i32,
        metrics: &Metrics,
        tracked: Option<&HashSet<u64>>,
    ) {
        // A recorded gap, or an hour whose census is unread or incomplete,
        // closes every battle; the census opens those it holds again after.
        let adjacent = previous.and_then(|before| before.checked_add(1)) == Some(hour);
        *previous = Some(hour);
        let rows = slot_read(format, metrics, TableId::LandCombats, hour)
            .then(|| frame.complete::<tables::LandCombats>().ok())
            .flatten();
        if !adjacent || rows.is_none() {
            open.clear();
        }
        if let Some(rows) = rows {
            let mut still = HashMap::with_capacity(rows.len());
            for row in rows {
                let spans = self.present.entry(row.id).or_default();
                match open.get(&row.id) {
                    Some(&at) => spans[at].1 = hour,
                    None => spans.push((hour, hour)),
                }
                still.insert(row.id, spans.len() - 1);
            }
            *open = still;
        }
        // As the look-back's catalogue reads and forgets names.
        if frame.complete::<tables::VariantDefinitions>().is_err() {
            self.forgot.get_or_insert(hour);
        }
        if slot_read(format, metrics, TableId::VariantDefinitions, hour)
            && frame.complete::<tables::Countries>().is_ok()
            && frame.complete::<tables::CountryLinks>().is_ok()
            && frame.complete::<tables::LexicalTokens>().is_ok()
            && let Ok(definitions) = frame.complete::<tables::VariantDefinitions>()
        {
            for definition in definitions {
                if tracked.is_none_or(|tracked| tracked.contains(&definition.id)) {
                    self.read.entry(definition.id).or_insert(hour);
                }
            }
        }
    }
}

/// What the last block's window reads that the look-back's start depends
/// on.
#[derive(Default)]
struct Window {
    /// The earliest coverage start of a result the census matches in the
    /// window.
    results_from: Option<i32>,
    /// Each such result's combat and coverage start.
    results: Vec<(u64, i32)>,
    /// Each variant a loss read in the window may name, at the first hour
    /// one may.
    named: HashMap<u64, i32>,
}

impl Window {
    /// A changed hour of the last block, whose first hour is `first`.
    fn observe(
        &mut self,
        format: replay_data::format::FormatVersion,
        frame: &DecodedFrame,
        first: i32,
    ) {
        let hour = frame.hour;
        // Every variant a progress row's losses name, read or not.
        if let Ok(rows) = frame.complete::<tables::LandCombatProgress>() {
            for row in rows {
                for losses in [
                    &row.attacker_equipment_losses,
                    &row.defender_equipment_losses,
                ] {
                    if let Recorded::Value(losses) = losses {
                        for loss in losses {
                            self.named.entry(loss.variant).or_insert(hour);
                        }
                    }
                }
            }
        }
        // The results the census matches: read in their slot, complete, with
        // the hour's complete event coverage, ending within it. Quiet hours
        // hold an earlier hour's coverage, which matches none.
        if !slot_read(format, &frame.metrics, TableId::LandCombatResults, hour) {
            return;
        }
        let Ok(results) = frame.complete::<tables::LandCombatResults>() else {
            return;
        };
        let Some(coverage) = frame
            .complete::<tables::LandEventCoverage>()
            .ok()
            .and_then(|rows| rows.first())
            .filter(|row| row.complete && row.to_hour == hour)
        else {
            return;
        };
        for result in results {
            if result.end_hour <= coverage.from_hour || result.end_hour > coverage.to_hour {
                continue;
            }
            self.results_from = Some(
                self.results_from
                    .map_or(coverage.from_hour, |from| from.min(coverage.from_hour)),
            );
            self.results.push((result.combat, coverage.from_hour));
            if result.end_hour >= first {
                for side in &result.losses {
                    for loss in &side.equipment {
                        self.named.entry(loss.variant).or_insert(hour);
                    }
                }
            }
        }
    }
}

/// `index`'s facts, read alone by its changes: the combat and naming tables'
/// keys, with every field of the progress, results and event coverage for
/// the last block's `window` (and the window's first hour). Earlier blocks
/// note the reads of the `tracked` variants alone.
fn read_facts(
    replay: &Replay,
    index: &BlockIndex,
    mut window: Option<(&mut Window, i32)>,
    tracked: Option<&HashSet<u64>>,
    cancelled: &AtomicBool,
) -> io::Result<Facts> {
    let format = replay.format;
    let mut fields = vec![
        TableFields::keys(TableId::LandCombats),
        TableFields::keys(TableId::Countries),
        TableFields::keys(TableId::CountryLinks),
        TableFields::keys(TableId::VariantDefinitions),
        TableFields::keys(TableId::LexicalTokens),
    ];
    if window.is_some() {
        fields.extend([
            TableFields::all(TableId::LandCombatProgress),
            TableFields::all(TableId::LandCombatResults),
            TableFields::all(TableId::LandEventCoverage),
        ]);
    }
    let mut facts = Facts {
        first_hour: index.first_hour,
        last_hour: index.last_hour,
        ..Facts::default()
    };
    // The combats present at the hour before, each with its run's position.
    let mut open: HashMap<u64, usize> = HashMap::new();
    let mut previous: Option<i32> = None;
    // The block's last changed frame, which every quiet hour after it holds
    // for the watched tables.
    let mut last: Option<DecodedFrame> = None;
    replay.read_block_changes(
        index,
        &fields,
        |change| {
            match change {
                BlockChange::Changed { frame, .. } => {
                    facts.hour(
                        &mut open,
                        &mut previous,
                        format,
                        frame,
                        frame.hour,
                        &frame.metrics,
                        tracked,
                    );
                    if let Some((window, first)) = window.as_mut() {
                        window.observe(format, frame, *first);
                    }
                    last = Some(frame.clone());
                }
                BlockChange::Quiet(quiet) => {
                    let frame = last.as_ref().ok_or_else(|| {
                        io::Error::other("A quiet hour came before the block's first")
                    })?;
                    for (&hour, metrics) in quiet.hours().iter().zip(quiet.metrics()) {
                        facts.hour(
                            &mut open,
                            &mut previous,
                            format,
                            frame,
                            hour,
                            metrics,
                            tracked,
                        );
                    }
                }
            }
            Ok(())
        },
        || check_cancel(cancelled),
    )?;
    Ok(facts)
}

/// Where the look-back starts.
enum Start {
    /// At this block of the branch, after its first.
    Block(usize),
    /// At the branch's first block, as the check failing at the block after
    /// it names (none for a branch of one block).
    BranchStart(Option<Unproven>),
}

/// What the blocks taken so far, from the last back, leave to prove for a
/// look-back starting at the earliest of them.
struct Search {
    results_from: Option<i32>,
    results: Vec<(u64, i32)>,
    /// Each variant the window names, and whether a block taken read or
    /// forgot its naming before the window named it.
    named: HashMap<u64, bool>,
    /// The battles on the page present since the earliest block's first
    /// hour, whose opening lies further back.
    open: HashSet<u64>,
}

impl Search {
    /// After the last block: every battle it holds is on the page.
    fn new(last: &Facts, window: Window) -> Self {
        let named = window
            .named
            .iter()
            .map(|(variant, &hour)| {
                let read = last.read.get(variant).is_some_and(|&read| read <= hour)
                    || last.forgot.is_some_and(|forgot| forgot <= hour);
                (*variant, read)
            })
            .collect();
        let open = last
            .present
            .iter()
            .filter(|(_, spans)| spans.first().is_some_and(|span| span.0 == last.first_hour))
            .map(|(&combat, _)| combat)
            .collect();
        Self {
            results_from: window.results_from,
            results: window.results,
            named,
            open,
        }
    }

    /// Takes `block`, the block before the earliest taken, which `adjoins`
    /// it (no recorded gap between), and is the block before the window's
    /// when `window_next`.
    fn take(&mut self, block: &Facts, adjoins: bool, window_next: bool) {
        let mut open = HashSet::new();
        let mut on_page = |combat: u64, span: (i32, i32)| {
            if span.0 == block.first_hour {
                open.insert(combat);
            }
        };
        let at_end = |combat: &u64| {
            block
                .present
                .get(combat)
                .and_then(|spans| spans.last())
                .filter(|span| span.1 == block.last_hour)
                .copied()
        };
        if adjoins {
            // A battle open at the next block's first hour, present here
            // through the last hour, continues.
            for combat in &self.open {
                if let Some(span) = at_end(combat) {
                    on_page(*combat, span);
                }
            }
            // The window's first hour closes the battles of the hour
            // before it.
            if window_next {
                for combat in block.present.keys() {
                    if let Some(span) = at_end(combat) {
                        on_page(*combat, span);
                    }
                }
            }
        }
        // A result in the window closes a battle present from its coverage
        // start on.
        for &(combat, from) in &self.results {
            for &span in block.present.get(&combat).into_iter().flatten() {
                if span.1 >= from {
                    on_page(combat, span);
                }
            }
        }
        self.open = open;
        for (variant, read) in self.named.iter_mut().filter(|(_, read)| !**read) {
            *read = block.read.contains_key(variant) || block.forgot.is_some();
        }
    }

    /// The check a look-back starting at the earliest block taken fails,
    /// whose first hour is `first`, after a recorded gap when `after_gap`,
    /// with the hour it names; None when every check holds.
    fn fails(&self, first: i32, after_gap: bool) -> Option<Unproven> {
        if let Some(from) = self.results_from.filter(|from| *from < first) {
            return Some(Unproven {
                check: RESULT_COVERAGE,
                hour: from,
            });
        }
        if after_gap {
            return None;
        }
        let before = first.saturating_sub(1);
        if !self.open.is_empty() {
            return Some(Unproven {
                check: OPEN_BATTLE,
                hour: before,
            });
        }
        if self.named.values().any(|read| !read) {
            return Some(Unproven {
                check: VARIANT_NAMING,
                hour: before,
            });
        }
        None
    }
}

/// The latest block of the branch's `blocks` from which a look-back holds
/// every fact of `window`'s page that the blocks' changes decide: the last
/// block read by its changes, then each earlier block's, from the last back,
/// on spare cores ([`dashboard_core::blocks::scan_blocks_by`]), until one
/// holds them. The branch's first block is never a start. Counts the earlier
/// blocks taken in `read`.
fn start(
    replay: &Replay,
    blocks: &[usize],
    window: &BattleWindow,
    cancelled: &AtomicBool,
    read: &mut usize,
) -> io::Result<Start> {
    let last = blocks.len() - 1;
    if last == 0 {
        return Ok(Start::BranchStart(None));
    }
    let first_hour = |at: usize| replay.index[blocks[at]].first_hour;
    let mut seen = Window::default();
    let tail = read_facts(
        replay,
        &replay.index[blocks[last]],
        Some((&mut seen, window.first_hour)),
        None,
        cancelled,
    )?;
    let mut search = Search::new(&tail, seen);
    // The last block alone only after a gap: the window's first hour closes
    // the battles of the hour before it otherwise.
    let mut failed = if after_gap(replay, blocks, last) {
        search.fails(first_hour(last), true)
    } else {
        Some(Unproven {
            check: WINDOW_START,
            hour: first_hour(last).saturating_sub(1),
        })
    };
    if failed.is_none() {
        return Ok(Start::Block(last));
    }
    let tracked: HashSet<u64> = search.named.keys().copied().collect();
    let earlier: Vec<&BlockIndex> = (1..last)
        .rev()
        .map(|at| &replay.index[blocks[at]])
        .collect();
    if earlier.is_empty() {
        return Ok(Start::BranchStart(failed));
    }
    dashboard_core::blocks::scan_blocks_by(
        earlier,
        workers().saturating_sub(1),
        cancelled,
        &|index| read_facts(replay, index, None, Some(&tracked), cancelled),
        |reads| {
            for (at, (_, facts)) in (1..last).rev().zip(reads) {
                let facts = facts?;
                *read += 1;
                search.take(&facts, !after_gap(replay, blocks, at + 1), at + 1 == last);
                failed = search.fails(first_hour(at), after_gap(replay, blocks, at));
                if failed.is_none() {
                    return Ok(Start::Block(at));
                }
            }
            Ok(Start::BranchStart(failed))
        },
    )
}

/// The first block of the look-back [`start`] decides for the branch, the
/// branch's first block when it reaches it.
#[cfg(test)]
pub(super) fn single_pass_start(
    source: &Source,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<usize> {
    let replay = source.replay_ref();
    let blocks = branch_blocks(replay, branch);
    let window = &windows(replay, branch)[blocks.len() - 1];
    Ok(match start(replay, &blocks, window, cancelled, &mut 0)? {
        Start::Block(at) => at,
        Start::BranchStart(_) => 0,
    })
}

/// The first block of the look-back retried look-backs accept, as newest
/// windows were proven before one read of the blocks' changes decided the
/// start: the first a block before the last (the last after a gap), each
/// one not proven moving back to the block holding the hour its failure
/// names (the first after the gap holding it), from its second retry also at
/// least doubling its length, through the branch's first block, from which
/// one is always proven.
#[cfg(test)]
pub(super) fn retried_start(
    source: &Source,
    branch: u64,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<usize> {
    let replay = source.replay_ref();
    let blocks = branch_blocks(replay, branch);
    let last = blocks.len() - 1;
    let window = &windows(replay, branch)[last];
    let mut from = if last == 0 || after_gap(replay, &blocks, last) {
        last
    } else {
        last - 1
    };
    let mut retries = 0;
    loop {
        let before = match look_back(
            source,
            context,
            &blocks[from..],
            from == 0,
            after_gap(replay, &blocks, from),
            last,
            window,
            cancelled,
        )? {
            Attempt::Proven(_) => return Ok(from),
            Attempt::Before(why) => why,
        };
        let hinted = blocks.partition_point(|&at| replay.index[at].last_hour < before.hour);
        let doubled = blocks.len().saturating_sub(2 * (blocks.len() - from));
        from = if retries == 0 {
            hinted
        } else {
            hinted.min(doubled)
        }
        .min(from - 1);
        retries += 1;
    }
}

/// The branch's blocks, by position in the recording.
#[cfg(test)]
fn branch_blocks(replay: &Replay, branch: u64) -> Vec<usize> {
    replay
        .index
        .iter()
        .enumerate()
        .filter(|(_, index)| index.branch == branch)
        .map(|(at, _)| at)
        .collect()
}

/// One look-back over `blocks` (the branch's, from one through its last),
/// its blocks read ahead on spare cores and each reconstructed alone on this
/// thread, as [`Replay::read_block_alone`] reads it; `first` when they start
/// at the branch's first block, `after_gap` when the first follows a recorded
/// gap.
#[allow(clippy::too_many_arguments)]
fn look_back(
    source: &Source,
    context: &LoadContext,
    blocks: &[usize],
    first: bool,
    after_gap: bool,
    ordinal: usize,
    window: &BattleWindow,
    cancelled: &AtomicBool,
) -> io::Result<Attempt> {
    let replay = source.replay_ref();
    let start = replay.index[blocks[0]].first_hour;
    let mut census = Census::new(replay.format);
    census.configure(source, context);
    census.newest = Some(Check {
        window_first: window.first_hour,
        ..Check::default()
    });
    let consumer = replay.section_consumer(blocks[0]);
    let indices = blocks.iter().map(|&block| &replay.index[block]).collect();
    dashboard_core::blocks::scan_blocks_by(
        indices,
        workers().saturating_sub(1),
        cancelled,
        &|index| {
            if let Some(position) = replay.block_position(index) {
                consumer.reading(position);
            }
            replay.read_block_recorded_selected(index, &CENSUS_TABLES)
        },
        |reads| {
            for (index, read) in reads {
                if let Some(position) = replay.block_position(index) {
                    consumer.at(position);
                }
                let frames = read?;
                if !replay.reconstruct_alone(
                    &frames,
                    &CENSUS_TABLES,
                    |_, frame| census.observe(frame),
                    || check_cancel(cancelled),
                )? {
                    replay.read_block_recorded_alone(
                        index,
                        &CENSUS_TABLES,
                        |_, frame| census.observe(frame),
                        || check_cancel(cancelled),
                    )?;
                }
                // As forward preparation's publication after each block.
                census.materialize()?;
                census.dirty.clear();
            }
            Ok(())
        },
    )?;
    let reply = census.newest_page(ordinal, window)?;
    let check = census.newest.take().unwrap_or_default();
    // The earliest hour any unproven fact needs the look-back to start by,
    // with that fact.
    let mut before: Option<Unproven> = None;
    let mut back = |check: &'static str, hour: i32| {
        if before.as_ref().is_none_or(|earlier| hour < earlier.hour) {
            before = Some(Unproven { check, hour });
        }
    };
    if !first {
        if let Some(from) = check.results_from.filter(|from| *from < start) {
            back(RESULT_COVERAGE, from);
        }
        if !after_gap {
            if start >= window.first_hour {
                back(WINDOW_START, start.saturating_sub(1));
            }
            if check.unnamed {
                back(VARIANT_NAMING, start.saturating_sub(1));
            }
            // Each battle open at the look-back's first hour, with its native
            // counter there.
            let counters: HashMap<&str, i32> = census
                .episodes
                .iter()
                .filter(|episode| episode.start.hour <= start)
                .filter_map(|episode| Some((episode.id.as_str(), episode.runs.first()?.duration)))
                .collect();
            for row in reply.rows.iter().filter(|row| row.start.hour <= start) {
                let counter = counters.get(row.id.as_str()).copied().unwrap_or(0);
                back(
                    OPEN_BATTLE,
                    start.saturating_sub(counter.max(0)).saturating_sub(1),
                );
            }
        }
    }
    if let Some(why) = before {
        return Ok(Attempt::Before(why));
    }
    Ok(Attempt::Proven(Page {
        reply,
        census: Box::new(census),
    }))
}

impl Census {
    /// The window's page as forward preparation's publication of the last
    /// block writes it, as its own consumer of the shared content.
    fn newest_page(
        &mut self,
        ordinal: usize,
        window: &BattleWindow,
    ) -> io::Result<LandBattlesReply> {
        let mut own = std::mem::take(&mut self.refusals);
        let page = dashboard_base::catalogue::own_refusals(&mut own, || -> io::Result<_> {
            self.materialize()?;
            self.window_records(window).reply_window(ordinal, window)
        });
        self.refusals = own;
        page
    }
}

/// The branch's content warnings as forward preparation publishes them after
/// its last block, from a proven look-back's census and `earlier`, the
/// branch's blocks before the look-back's. None when naming the variants read
/// on the branch refuses text, which forward preparation's own naming may or
/// may not meet.
fn diagnostics(
    replay: &Replay,
    earlier: &[usize],
    mut census: Box<Census>,
    workers: usize,
    cancelled: &AtomicBool,
) -> io::Result<Option<Vec<String>>> {
    // Every variant read on the branch: its refusals do not depend on the
    // order of the reads, nor on the forgetting between them.
    let mut variants = variant_reads(replay, earlier, workers, cancelled)?;
    for (id, versions) in std::mem::take(&mut census.variants.versions) {
        variants.versions.entry(id).or_default().extend(versions);
    }
    census.variants = variants;
    if !names_refuse_nothing(&census, cancelled)? {
        return Ok(None);
    }
    let mut own = std::mem::take(&mut census.refusals);
    Ok(Some(dashboard_base::catalogue::own_refusals(
        &mut own,
        || census.content_warnings(),
    )))
}

/// Whether naming every variant the census's catalogue read refuses no text
/// and uses no lazy reader. Forward preparation names only variants read on
/// the branch, so its census's own refusals are then none, as the look-back's
/// are. Without content nothing is named.
fn names_refuse_nothing(census: &Census, cancelled: &AtomicBool) -> io::Result<bool> {
    let Some(catalogue) = &census.catalogue else {
        return Ok(true);
    };
    let texts: &dyn dashboard_base::products::equipment::Texts = &**catalogue;
    let mut own = dashboard_base::catalogue::OwnRefusals::default();
    dashboard_base::catalogue::own_refusals(&mut own, || -> io::Result<()> {
        for raw in census
            .variants
            .versions
            .values()
            .flatten()
            .filter_map(|version| version.raw.as_ref())
        {
            check_cancel(cancelled)?;
            dashboard_base::products::equipment::variant_names(raw, Some(texts));
        }
        Ok(())
    })?;
    Ok(own == dashboard_base::catalogue::OwnRefusals::default())
}

/// The fields of the variant catalogue's tables whose changes can change a
/// variant's naming operands: the countries' keys and tags (their balances
/// change every hour and name nothing), and every field of the rest.
fn naming_fields() -> io::Result<Vec<TableFields>> {
    VariantCatalogue::TABLES
        .iter()
        .map(|&table| {
            if table == TableId::Countries {
                TableFields::named(table, &["canonical_slot", "tag"])
            } else {
                Ok(TableFields::all(table))
            }
        })
        .collect()
}

/// One block's reads of the variant table onto `variants`, the block read
/// only at the hours its naming tables change. A read (the table's daily
/// slot) is observed when a naming table changed since the last read the
/// block observed, the block's first read always, on the frame of the block
/// read alone there; a read with no such change derives the operands the
/// last one observed did. Each read is observed as continuing the last
/// observed, so the catalogue keeps one version per change of a variant's
/// operands: forgetting between reads would add only versions without
/// operands, which name and refuse nothing.
fn block_variant_reads(
    replay: &Replay,
    index: &replay::container::BlockIndex,
    fields: &[TableFields],
    variants: &mut VariantCatalogue,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    let format = replay.format;
    let read = |metrics: &replay_frame::frame::Metrics, hour: i32| {
        slot_read(format, metrics, TableId::VariantDefinitions, hour)
    };
    let observe = |variants: &mut VariantCatalogue, frame: &DecodedFrame| {
        variants.advance(frame.hour.saturating_sub(1));
        variants.observe(format, frame);
    };
    // The block's last changed frame, which every quiet hour after it
    // holds for the watched tables; and whether a naming table changed
    // since the last read observed.
    let mut last: Option<DecodedFrame> = None;
    let mut stale = true;
    replay.read_block_changes(
        index,
        &fields,
        |change| {
            match change {
                BlockChange::Changed { frame, .. } => {
                    stale = true;
                    if read(&frame.metrics, frame.hour) {
                        observe(&mut *variants, frame);
                        stale = false;
                    }
                    last = Some(frame.clone());
                }
                BlockChange::Quiet(quiet) => {
                    if !stale {
                        return Ok(());
                    }
                    let Some(position) = (0..quiet.hours().len())
                        .find(|&at| read(&quiet.metrics()[at], quiet.hours()[at]))
                    else {
                        return Ok(());
                    };
                    let mut frame = last.clone().ok_or_else(|| {
                        io::Error::other("A quiet hour came before the block's first")
                    })?;
                    frame.hour = quiet.hours()[position];
                    frame.metrics = quiet.metrics()[position].clone();
                    observe(&mut *variants, &frame);
                    stale = false;
                }
            }
            Ok(())
        },
        || check_cancel(cancelled),
    )?;
    Ok(())
}

/// A catalogue of every variant read on `blocks`, each block's reads
/// ([`block_variant_reads`]) on a catalogue of its own on `workers` threads
/// within the shared block-read core budget
/// ([`dashboard_core::blocks::scan_blocks_by`]), joined in block order. The
/// same reads are observed (a block's first read always is), every version
/// holds the operands of a read observed, and each read's operands are those
/// of the version it adds or of the one before, which also holds them. A
/// block's first version of a variant whose operands the variant's last
/// joined version holds is dropped, as reading in order adds none there, so
/// the joined catalogue keeps about the in-order catalogue's versions (the
/// in-order one's alone, but for the versions without operands a read
/// forgetting earlier blocks' names adds), and their operands exactly. Naming refusals and variant warnings are of the operands read,
/// whatever their order and however often each is read
/// ([`VariantCatalogue::warnings`] notes each variant once per reason), so
/// they are the in-order catalogue's.
pub(super) fn variant_reads(
    replay: &Replay,
    blocks: &[usize],
    workers: usize,
    cancelled: &AtomicBool,
) -> io::Result<VariantCatalogue> {
    let fields = naming_fields()?;
    let consumer = replay.section_consumer(blocks.first().copied().unwrap_or(0));
    let indices = blocks.iter().map(|&block| &replay.index[block]).collect();
    dashboard_core::blocks::scan_blocks_by(
        indices,
        workers.saturating_sub(1),
        cancelled,
        &|index| {
            if let Some(position) = replay.block_position(index) {
                consumer.reading(position);
            }
            let mut variants = VariantCatalogue::default();
            block_variant_reads(replay, index, &fields, &mut variants, cancelled)?;
            Ok(variants)
        },
        |reads| {
            let mut variants = VariantCatalogue::default();
            for (index, read) in reads {
                if let Some(position) = replay.block_position(index) {
                    consumer.at(position);
                }
                for (id, versions) in read?.versions {
                    let held = variants.versions.entry(id).or_default();
                    let mut versions = versions.into_iter().peekable();
                    // Reading in order adds no version where a read's
                    // operands are the variant's last version's.
                    if let (Some(last), Some(first)) = (held.last(), versions.peek())
                        && same_operands(last, first)
                    {
                        versions.next();
                    }
                    held.extend(versions);
                }
            }
            Ok(variants)
        },
    )
}

/// Whether `next` holds the operands `last` holds, whatever read first read
/// them.
fn same_operands(
    last: &dashboard_base::products::equipment::VariantVersion,
    next: &dashboard_base::products::equipment::VariantVersion,
) -> bool {
    last.raw
        .as_ref()
        .zip(next.raw.as_ref())
        .is_some_and(|(last, next)| {
            let mut probe = next.clone();
            probe.first_hour = last.first_hour;
            probe == *last
        })
}

/// [`variant_reads`] block after block on one catalogue, in order.
#[cfg(test)]
pub(super) fn variant_reads_in_order(
    replay: &Replay,
    blocks: &[usize],
    cancelled: &AtomicBool,
) -> io::Result<VariantCatalogue> {
    let fields = naming_fields()?;
    let mut variants = VariantCatalogue::default();
    for &block in blocks {
        block_variant_reads(
            replay,
            &replay.index[block],
            &fields,
            &mut variants,
            cancelled,
        )?;
    }
    Ok(variants)
}

/// Threads the earlier blocks' naming tables are read on: one per core.
pub fn workers() -> usize {
    std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
}

impl Newest {
    /// Writes the page, with the bytes forward preparation writes for the
    /// window, what the query checks it by, and its diagnostics.
    pub fn write(&self, writer: &mut store::Writer) -> io::Result<()> {
        write_page(writer, self.branch, &self.reply, &self.position)?;
        series::write_reply(writer, &meta_section(self.branch), 0, &self.meta)
    }
}

fn write_page(
    writer: &mut store::Writer,
    branch: u64,
    reply: &LandBattlesReply,
    position: &Position,
) -> io::Result<()> {
    series::write_reply(writer, &page_section(branch), 0, reply)?;
    series::write_reply(writer, &position_section(branch), 0, position)
}

impl Proven {
    /// Writes the page, with the bytes forward preparation writes for the
    /// window, and what the query checks it by.
    pub fn write(&self, writer: &mut store::Writer) -> io::Result<()> {
        write_page(writer, self.branch, &self.reply, &self.position)
    }

    /// The window's diagnostics, from its look-back's census and the naming
    /// tables of the branch's earlier blocks, read on `workers` threads;
    /// None where forward preparation would refuse or fail where they read,
    /// or naming the variants read refuses text.
    pub fn diagnose(
        self,
        replay: &Replay,
        workers: usize,
        cancelled: &AtomicBool,
    ) -> io::Result<Option<Diagnosed>> {
        self.diagnose_reported(replay, workers, cancelled, &mut Report::default())
    }

    /// [`Self::diagnose`], counting its reads and outcome in the proof's report.
    pub fn diagnose_reported(
        self,
        replay: &Replay,
        workers: usize,
        cancelled: &AtomicBool,
        report: &mut Report,
    ) -> io::Result<Option<Diagnosed>> {
        let started = std::time::Instant::now();
        let named = diagnostics(replay, &self.earlier, self.census, workers, cancelled);
        report.naming = self.earlier.len();
        report.naming_took = started.elapsed();
        let content_warnings = match named {
            Ok(Some(warnings)) => warnings,
            Ok(None) => {
                report.outcome = Outcome::NamingRefuses;
                return Ok(None);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
            Err(error) => {
                report.outcome = Outcome::Unreadable(error.to_string());
                return Ok(None);
            }
        };
        Ok(Some(Diagnosed {
            branch: self.branch,
            meta: Meta {
                window: self.position.window,
                interval: self.position.interval,
                content_warnings,
            },
        }))
    }
}

impl Diagnosed {
    /// Writes the diagnostics beside the window's page.
    pub fn write(&self, writer: &mut store::Writer) -> io::Result<()> {
        series::write_reply(writer, &meta_section(self.branch), 0, &self.meta)
    }
}

/// Whether `window` at `interval` is the branch's current last window.
fn current(replay: &Replay, branch: u64, window: usize, interval: &BattleWindow) -> bool {
    let windows = windows(replay, branch);
    windows.len().checked_sub(1) == Some(window)
        && windows.last().is_some_and(|last| {
            last.first_hour == interval.first_hour && last.last_hour == interval.last_hour
        })
}

/// The newest page's position when it is the branch's current last window:
/// its own section, or the diagnostics written with it by earlier builds.
fn position(
    store: &store::Store,
    key: &store::SourceKey,
    replay: &Replay,
    branch: u64,
) -> io::Result<Option<Position>> {
    let position = match series::read_reply::<Position>(store, key, &position_section(branch), 0)? {
        Some(position) => Some(position),
        None => {
            series::read_reply::<Meta>(store, key, &meta_section(branch), 0)?.map(|meta| Position {
                window: meta.window,
                interval: meta.interval,
            })
        }
    };
    Ok(position.filter(|position| current(replay, branch, position.window, &position.interval)))
}

/// Whether the store holds the newest window of the branch's current last
/// block and its diagnostics.
pub fn published(
    store: &store::Store,
    key: &store::SourceKey,
    replay: &Replay,
    branch: u64,
) -> io::Result<bool> {
    Ok(
        series::read_reply::<Meta>(store, key, &meta_section(branch), 0)?
            .is_some_and(|meta| current(replay, branch, meta.window, &meta.interval)),
    )
}

/// A land battle query answered from the branch's newest window before
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
    if !matches!(product, "land-battles-manifest" | "land-battles") {
        return Ok(None);
    }
    let replay = source.replay_ref();
    let Some(position) = position(store, key, replay, branch)? else {
        return Ok(None);
    };
    if product == "land-battles-manifest" {
        let params: LandBattlesManifestParams = serde_json::from_value(params.clone())?;
        LandBattlesManifest::validate(&params).map_err(dashboard_core::invalid)?;
        // Published only where the look-back proved availability.
        return Ok(Some(serde_json::to_value(bounded(
            LandBattlesManifestReply {
                windows: windows(replay, branch),
                available: true,
                unavailable: None,
                content_warnings: Vec::new(),
            },
        )?)?));
    }
    let params: LandBattlesParams = serde_json::from_value(params.clone())?;
    LandBattles::validate(&params).map_err(dashboard_core::invalid)?;
    if params.window != position.window {
        return Ok(None);
    }
    let Some(mut reply) =
        series::read_reply::<LandBattlesReply>(store, key, &page_section(branch), 0)?
    else {
        return Ok(None);
    };
    // The page is published once proven; its diagnostics follow.
    let meta = series::read_reply::<Meta>(store, key, &meta_section(branch), 0)?.filter(|meta| {
        meta.window == position.window && current(replay, branch, meta.window, &meta.interval)
    });
    let complete = meta.is_some();
    reply.content_warnings = meta.map(|meta| meta.content_warnings).unwrap_or_default();
    let mut value = serde_json::to_value(bounded(reply)?)?;
    if !complete && let Some(fields) = value.as_object_mut() {
        fields.insert("warningsComplete".into(), Value::Bool(false));
    }
    Ok(Some(value))
}
