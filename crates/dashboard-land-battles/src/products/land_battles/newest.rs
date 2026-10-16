//! A branch's newest block window prepared first, from its last block and the
//! shortest look-back proven to hold every fact its page carries, so the
//! window answers before forward preparation reaches it.
//!
//! A look-back observes the branch's blocks from one of them through the last,
//! each read alone, on a fresh census configured as forward preparation's, and
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
//!   A battle on the page already open at its first hour moves the look-back
//!   to before that battle's native duration counter began.
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
//! preparation's naming refuses nothing either, and otherwise nothing is
//! published.
//!
//! A look-back not proven moves back to the block its failure names, and
//! after its first retry at least doubles its length. A look-back that would
//! start at the branch's first block would be forward preparation's walk of
//! the branch: none is tried, and the window is left to forward preparation.
//! A census refusal or a block that cannot be read publishes nothing: forward
//! preparation refuses the branch or reports the error itself. The page is
//! saved under names of its own, with the bytes forward preparation writes
//! for the window.
use super::*;
use dashboard_base::{prepared::series, products::equipment::VariantCatalogue};
use replay::container::{BlockChange, TableFields};
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

/// What is saved beside a newest window's page.
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

/// A branch's newest window, proven equal to forward preparation's.
pub struct Newest {
    branch: u64,
    reply: LandBattlesReply,
    meta: Meta,
    /// Blocks read over every look-back tried, the last block's included.
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

fn meta_section(branch: u64) -> String {
    format!("land-battle-newest-diagnostics-{branch}")
}

/// A look-back's outcome.
enum Attempt {
    /// Its page is forward preparation's.
    Proven(Page),
    /// The look-back must start at or before this hour.
    Before(i32),
}

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
    let replay = source.replay_ref();
    let blocks: Vec<usize> = replay
        .index
        .iter()
        .enumerate()
        .filter(|(_, index)| index.branch == branch)
        .map(|(at, _)| at)
        .collect();
    let Some(last) = blocks.len().checked_sub(1) else {
        return Ok(None);
    };
    let windows = windows(replay, branch);
    let window = &windows[last];
    // Whether the branch's block `at` follows a recorded gap: forward
    // preparation observes no hour just before its first.
    let after_gap = |at: usize| {
        at > 0
            && replay.index[blocks[at - 1]]
                .last_hour
                .checked_add(1)
                .is_some_and(|next| next < replay.index[blocks[at]].first_hour)
    };
    // The window's first hour closes the battles of the hour before it, which
    // only a look-back from an earlier block observes.
    let mut from = if last == 0 || after_gap(last) {
        last
    } else {
        last - 1
    };
    // From the branch's first block the look-back is forward preparation's
    // walk of the branch: the window is left to it.
    if from == 0 {
        return Ok(None);
    }
    #[cfg(any(test, feature = "test-support"))]
    let mut read = 0;
    let mut retries = 0;
    loop {
        check_cancel(cancelled)?;
        let attempt = look_back(
            source,
            context,
            &blocks[from..],
            from == 0,
            after_gap(from),
            last,
            window,
            cancelled,
        );
        #[cfg(any(test, feature = "test-support"))]
        {
            read += blocks.len() - from;
        }
        let before = match attempt {
            Ok(Attempt::Proven(page)) => {
                if !page.census.available {
                    return Ok(None);
                }
                let content_warnings =
                    match diagnostics(replay, &blocks[..from], page.census, cancelled) {
                        Ok(Some(warnings)) => warnings,
                        Ok(None) => return Ok(None),
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                            return Err(error);
                        }
                        Err(_) => return Ok(None),
                    };
                return Ok(Some(Newest {
                    branch,
                    reply: page.reply,
                    meta: Meta {
                        window: last,
                        interval: window.clone(),
                        content_warnings,
                    },
                    #[cfg(any(test, feature = "test-support"))]
                    blocks_read: read,
                    #[cfg(any(test, feature = "test-support"))]
                    naming_blocks: from,
                }));
            }
            Ok(Attempt::Before(hour)) => hour,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
            Err(_) => return Ok(None),
        };
        // The block holding that hour, or the first after the gap holding it.
        let hinted = blocks.partition_point(|&at| replay.index[at].last_hour < before);
        let doubled = blocks.len().saturating_sub(2 * (blocks.len() - from));
        from = if retries == 0 {
            hinted
        } else {
            hinted.min(doubled)
        }
        .min(from - 1);
        if from == 0 {
            return Ok(None);
        }
        retries += 1;
    }
}

/// One look-back over `blocks` (the branch's, from one through its last);
/// `first` when they start at the branch's first block, `after_gap` when the
/// first follows a recorded gap.
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
    for &block in blocks {
        replay.read_block_alone(
            &replay.index[block],
            &CENSUS_TABLES,
            |_, frame| census.observe(frame),
            || check_cancel(cancelled),
        )?;
        // As forward preparation's publication after each block.
        census.materialize()?;
        census.dirty.clear();
    }
    let reply = census.newest_page(ordinal, window)?;
    let check = census.newest.take().unwrap_or_default();
    // The earliest hour any unproven fact needs the look-back to start by.
    let mut before: Option<i32> = None;
    let mut back = |hour: i32| before = Some(before.map_or(hour, |earlier| earlier.min(hour)));
    if !first {
        if let Some(from) = check.results_from.filter(|from| *from < start) {
            back(from);
        }
        if !after_gap {
            if start >= window.first_hour || check.unnamed {
                back(start.saturating_sub(1));
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
                back(start.saturating_sub(counter.max(0)).saturating_sub(1));
            }
        }
    }
    if let Some(hour) = before {
        return Ok(Attempt::Before(hour));
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
    cancelled: &AtomicBool,
) -> io::Result<Option<Vec<String>>> {
    // Every variant read on the branch: its refusals do not depend on the
    // order of the reads, nor on the forgetting between them.
    let mut variants = variant_reads(replay, earlier, workers(), cancelled)?;
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
    let read = |metrics: &replay::frame::Metrics, hour: i32| {
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
    let indices = blocks.iter().map(|&block| &replay.index[block]).collect();
    dashboard_core::blocks::scan_blocks_by(
        indices,
        workers.saturating_sub(1),
        cancelled,
        &|index| {
            let mut variants = VariantCatalogue::default();
            block_variant_reads(replay, index, &fields, &mut variants, cancelled)?;
            Ok(variants)
        },
        |reads| {
            let mut variants = VariantCatalogue::default();
            for (_, read) in reads {
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
fn workers() -> usize {
    std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
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
    let Some(meta) = series::read_reply::<Meta>(store, key, &meta_section(branch), 0)? else {
        return Ok(None);
    };
    if !current(replay, branch, &meta) {
        return Ok(None);
    }
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
    if params.window != meta.window {
        return Ok(None);
    }
    let Some(mut reply) =
        series::read_reply::<LandBattlesReply>(store, key, &page_section(branch), 0)?
    else {
        return Ok(None);
    };
    reply.content_warnings = meta.content_warnings;
    Ok(Some(serde_json::to_value(bounded(reply)?)?))
}
