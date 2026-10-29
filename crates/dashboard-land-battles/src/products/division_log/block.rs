//! The division log prepared block by block, on its own. Divisions move every
//! hour of any campaign, and the presentation tables the observer joins
//! (division names, templates, icons) are read again every hour, so no hour
//! is quiet for the observer: a pre-pass worker reads each block alone with
//! the observer's tables, every hour, and runs the observer on it from the
//! block's first hour with no state of earlier blocks. That is where the
//! hourly work runs: each hour's census, its new divisions, its witnessed
//! removals and the presentation they carry.
//!
//! After the block's first hour the extract and a scan carried from earlier
//! blocks hold the same census of live divisions, removal witnesses, country
//! tags and presentation operands, all read again at that hour, when the
//! divisions, the witnesses and the country tables were read completely and
//! the presentation tables changed there (as at every block's first hour).
//! They differ in three things only:
//! - The divisions seen earlier on the branch. A division absent from the last
//!   census is logged only the first time the branch sees it, and only the
//!   carried scan knows the earlier blocks' divisions, so the extract logs
//!   every one as a candidate and the fold keeps the candidates the carried
//!   scan never saw, adding them to its record as the hourly scan does.
//! - The template read hour. A presentation is dated by the last read of the
//!   template table; before the block's first such read only the carried scan
//!   knows it, so the extract dates those presentations with a placeholder
//!   ([`INHERITED`]) that the fold replaces with the carried hour.
//! - Their accumulations: events, warnings, the record of seen divisions and
//!   whether losses were ever readable, which the fold extends.
//!
//! The family's own thread folds the blocks in order: it observes each
//! block's first hour on the carried scan, as hourly preparation does, then,
//! when the carried scan continues that hour as the extract did, takes the
//! extract's census, witnesses, tags, presentation and dates and its events,
//! warnings and candidates as above. No hour but the first is observed there.
//! Otherwise the rest of the block is observed hour by hour.
//!
//! Hourly preparation publishes each block's window, the manifest and the
//! totals series at the block's end. The family renders the window and the
//! manifest at the block's end, from the scan then, and writes them with its
//! next commit; the totals take the block's events then, in hour order, and
//! flush once per block, as the hourly scan feeds and flushes them. A block
//! whose publication fails commits at once, so the branch is refused at the
//! hour hourly preparation refuses it.

use super::*;
use dashboard_base::prepared::{
    frame_changes::{FrameChangeTracker, FrameChanges},
    prepass::{Extract, Family, Folded, Held as Holds, HeldBytes},
    series,
};
use replay::container::BlockIndex;
use replay_data::format::FormatVersion;
use std::{any::Any, ops::Range};

/// The family the division log is prepared as: its refusal, progress and
/// coverage name is the hourly walk's.
pub static FAMILY: Family = Family {
    name: "divisions",
    extract: "divisions.extract",
    sequential: "divisions.sequential",
    fallback: "divisions.fallback",
    tables: &TABLES,
    // No hour is quiet for the observer: the divisions it reads move, and
    // their names are read again, every hour.
    fields: None,
    start,
};

/// Each hour observed on the carried scan, counted in tests: every folded
/// block's first hour, and every hour of a block observed hour by hour.
pub const OBSERVED: &str = "divisions.observed";

/// The template read hour the carried scan holds at a block's first hour,
/// which the extract of the block alone does not know: the extract dates its
/// presentations with it until the block's first template read, and the fold
/// replaces it with the carried hour.
pub(super) const INHERITED: i32 = i32::MIN;

fn start(format: FormatVersion) -> Box<dyn Extract> {
    Box::new(BlockExtract {
        scan: Box::new(Scan::new(format)),
        first: None,
        last: None,
    })
}

/// The block's first hour, which the fold observes on the carried scan, and
/// what the extract's scan held right after it.
struct First {
    frame: DecodedFrame,
    changes: FrameChanges,
    live: Vec<u64>,
    deletions: HashMap<u64, (u32, u32, u8, Option<u8>)>,
    previous_census: Option<i32>,
    census: bool,
    available: bool,
    /// The country tables were read completely.
    labelled: bool,
    /// The presentation tables changed, so the census was built again.
    presented: bool,
    /// Events the extract logged at the first hour.
    events: usize,
}

struct BlockFacts {
    first: First,
    /// The last hour the extract observed after the first.
    last: Option<i32>,
    /// The extract's scan at the block's end.
    scan: Box<Scan>,
}

struct BlockExtract {
    scan: Box<Scan>,
    first: Option<First>,
    last: Option<i32>,
}

/// An event row: every string it names.
impl Holds for DivisionLogEntry {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .heap(&self.id)
            .heap(&self.division)
            .heap(&self.status)
            .heap(&self.tag)
            .heap(&self.name)
            .heap(&self.name_missing)
            .heap(&self.icon)
            .heap(&self.icon_missing)
            .heap(&self.template)
            .heap(&self.template_name)
            .heap(&self.template_missing)
            .heap(&self.killer_tag)
            .heap(&self.killer_division);
    }
}

impl Holds for ArmyName {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.heap(&self.name).heap(&self.naming);
    }
}

impl Holds for TemplateName {
    fn held(&self, bytes: &mut HeldBytes) {
        match &self.0 {
            Ok(TemplateNaming::Text(text)) => bytes.heap(text),
            Ok(TemplateNaming::Localised { tag, key }) => bytes.heap(tag).heap(key),
            Err(why) => bytes.heap(why),
        };
    }
}

impl Holds for Presentation {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .heap(&self.status)
            .heap(&self.template)
            .heap(&self.icon);
    }
}

impl Holds for Event {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.heap(&self.row).heap(&self.presentation);
    }
}

impl Holds for Census {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.heap(&self.rows);
    }
}

impl Holds for Scan {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .heap(&self.events)
            .heap(&self.seen)
            .heap(&self.live)
            .heap(&self.deletions)
            .heap(&self.links)
            .heap(&self.presentation)
            .heap(&self.warnings)
            .heap(&self.counts)
            .heap(&self.candidates);
    }
}

impl Extract for BlockExtract {
    /// What the block's facts hold: its first frame and what the scan held
    /// after it, and the scan it ends with (its events and their strings,
    /// live divisions, deletions, tags, presentation census, warnings and
    /// counts).
    fn bytes(&self) -> u64 {
        let mut held = HeldBytes::default();
        if let Some(first) = &self.first {
            held.value(&first.frame)
                .value(&first.changes)
                .heap(&first.live)
                .heap(&first.deletions);
        }
        held.heap(&self.scan).total()
    }

    fn observe(
        &mut self,
        _: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        let scan = &mut self.scan;
        if self.first.is_none() {
            scan.observe_with_changes(frame, Some(changes));
            let hour = frame.hour;
            if hour == INHERITED {
                return Err(io::Error::other(
                    "A division block starts at the template placeholder hour",
                ));
            }
            // Before the block's first template read, presentations carry
            // the carried scan's template read hour.
            if !changes.slot_read[TableId::DivisionTemplates.index()] {
                scan.template_hour = Some(INHERITED);
                if let Some(census) = &mut scan.presentation {
                    census.template_hour = Some(INHERITED);
                    for row in &mut census.rows {
                        row.template_hour = Some(INHERITED);
                    }
                }
            }
            // The observer builds its census again when one of these changed.
            let presented = [
                TableId::Divisions,
                TableId::DivisionStatus,
                TableId::DivisionTemplates,
                TableId::DivisionTemplateIcons,
                TableId::Countries,
                TableId::CountryLinks,
            ]
            .iter()
            .any(|table| changes.rows_changed[table.index()]);
            self.first = Some(First {
                frame: frame.clone(),
                changes: FrameChanges {
                    rows_changed: changes.rows_changed,
                    table_read: changes.table_read,
                    slot_read: changes.slot_read,
                    updates: None,
                },
                live: scan.live.clone(),
                deletions: scan.deletions.clone(),
                previous_census: scan.previous_census,
                census: scan.presentation.is_some(),
                available: scan.available,
                labelled: !scan
                    .warnings
                    .contains_key(&(hour, IDENTITY_UNAVAILABLE.to_owned())),
                presented,
                events: scan.events.len(),
            });
            scan.candidates = Some(Vec::new());
            return Ok(());
        }
        scan.observe_with_changes(frame, Some(changes));
        if scan.irregular {
            return Err(io::Error::other(
                "A division block's first census comes after its first hour",
            ));
        }
        self.last = Some(frame.hour);
        Ok(())
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        let BlockExtract { scan, first, last } = *self;
        let first = first.ok_or_else(|| io::Error::other("Division block has no hours"))?;
        Ok(Box::new(BlockFacts { first, last, scan }))
    }
}

/// The observation of one hour's outcome: `Ok(true)` while the family goes
/// on, `Ok(false)` once it refused the branch.
pub type Observed<'a> = dyn FnMut(i32, io::Result<()>) -> io::Result<bool> + 'a;

/// Whether the carried scan, having observed the block's first hour,
/// continues it as the extract's scan did: both read the census, the
/// witnesses and the country tables completely there and built the
/// presentation census again, so they hold the same census, witnesses,
/// tags and presentation operands.
fn follows(scan: &Scan, first: &First) -> bool {
    first.available
        && first.labelled
        && first.presented
        && first.census
        && scan.available
        && scan.previous_hour == Some(first.frame.hour)
        && scan.previous_census == first.previous_census
        && scan.presentation.is_some()
        && scan.live == first.live
        && scan.deletions == first.deletions
}

/// Take the extract's hours after the first into the carried scan.
fn carry(scan: &mut Scan, own: Scan, first: &First) {
    let template = scan.template_hour;
    let date = |hour: &mut Option<i32>| {
        if *hour == Some(INHERITED) {
            *hour = template;
        }
    };
    let Scan {
        events,
        candidates,
        live,
        deletions,
        links,
        previous_hour,
        previous_census,
        presentation,
        template_hour,
        available,
        losses_available,
        warnings,
        ..
    } = own;
    let mut candidates = candidates.unwrap_or_default().into_iter().peekable();
    for (at, mut event) in events.into_iter().enumerate().skip(first.events) {
        // A candidate the carried scan saw earlier on the branch is no
        // creation.
        if let Some((_, id)) = candidates.next_if(|(index, _)| *index == at)
            && !scan.seen.insert(id)
        {
            continue;
        }
        date(&mut event.row.template_hour);
        if let Some(presentation) = &mut event.presentation {
            date(&mut presentation.template_hour);
        }
        scan.events.push(event);
    }
    for ((hour, reason), count) in warnings {
        if hour > first.frame.hour {
            *scan.warnings.entry((hour, reason)).or_default() += count;
        }
    }
    scan.live = live;
    scan.deletions = deletions;
    scan.links = links;
    scan.previous_hour = previous_hour;
    scan.previous_census = previous_census;
    scan.presentation = presentation.map(|mut census| {
        date(&mut census.template_hour);
        for row in &mut census.rows {
            date(&mut row.template_hour);
        }
        census
    });
    scan.template_hour = template_hour;
    date(&mut scan.template_hour);
    scan.available = available;
    scan.losses_available |= losses_available;
}

/// Fold one block's facts into the carried scan. Returns how it folded and
/// the last hour it reached.
pub fn fold(
    scan: &mut Scan,
    facts: Box<dyn Any + Send>,
    replay: &Replay,
    index: &BlockIndex,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<(Folded, Option<i32>)> {
    let _cpu = ::replay_frame::cpu::scope("fold driver", None);
    let facts = {
        let _cpu = ::replay_frame::cpu::scope("fold facts", None);
        facts
            .downcast::<BlockFacts>()
            .map_err(|_| io::Error::other("Pre-pass facts of another family"))?
    };
    let BlockFacts {
        first,
        last,
        scan: own,
    } = *facts;
    check()?;
    let hour = first.frame.hour;
    dashboard_base::prepared::prepass::note(OBSERVED);
    {
        let _cpu = ::replay_frame::cpu::scope("fold observe", None);
        scan.observe_with_changes(&first.frame, Some(&first.changes));
    }
    if !observed(hour, Ok(()))? {
        return Ok((Folded::Fast, Some(hour)));
    }
    let Some(last) = last else {
        return Ok((Folded::Fast, Some(hour)));
    };
    if !follows(scan, &first) {
        let reached = observe_block(scan, replay, index, Some(hour), observed, check)?;
        return Ok((Folded::Fallback, reached.or(Some(hour))));
    }
    check()?;
    {
        let _cpu = ::replay_frame::cpu::scope("fold observe", None);
        carry(scan, *own, &first);
    }
    observed(last, Ok(()))?;
    Ok((Folded::Fast, Some(last)))
}

/// Observe the block hour by hour after `after` (every hour without it), with
/// the change facts of the block alone, as hourly preparation does. Returns
/// the last hour observed.
pub fn observe_block(
    scan: &mut Scan,
    replay: &Replay,
    index: &BlockIndex,
    after: Option<i32>,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<Option<i32>> {
    let _cpu = ::replay_frame::cpu::scope("fold driver", None);
    let mut tracker = FrameChangeTracker::new();
    let mut reached = None;
    let mut stopped = false;
    let mut failure = None;
    let read = replay.read_block_alone(
        index,
        &TABLES,
        |raw, frame| {
            let _cpu = ::replay_frame::cpu::scope("fold driver", None);
            let changes = tracker.observe_updates(frame, Arc::new(raw), replay.format);
            if stopped || after.is_some_and(|after| frame.hour <= after) {
                return Ok(());
            }
            reached = Some(frame.hour);
            dashboard_base::prepared::prepass::note(OBSERVED);
            {
                let _cpu = ::replay_frame::cpu::scope("fold observe", None);
                scan.observe_with_changes(frame, Some(&changes));
            }
            match observed(frame.hour, Ok(())) {
                Ok(live) => {
                    stopped = !live;
                    Ok(())
                }
                Err(error) => {
                    failure = Some(error);
                    Err(io::Error::other("Division observation stopped"))
                }
            }
        },
        check,
    );
    if let Some(error) = failure {
        return Err(error);
    }
    read?;
    Ok(reached)
}

/// A reply's page as `series::write_reply` writes it.
fn page<T: Serialize>(reply: io::Result<T>) -> io::Result<Vec<u8>> {
    series::compress(serde_json::to_vec(&reply?)?.as_slice(), 3)
}

/// One block's publication, rendered at its end and written at the next
/// commit: its window and the manifest as hourly preparation writes them at
/// the block's end, then its totals.
struct Held {
    /// Each page's section, range and compressed reply.
    pages: Vec<(String, u64, Vec<u8>)>,
    /// The first and last hours observed in the block.
    hours: (i32, i32),
    /// The block's events in the scan.
    events: Range<usize>,
    /// Why a page could not be rendered: hourly preparation writes the pages
    /// before it and refuses the branch at the block's end.
    error: Option<io::Error>,
}

/// One branch's division log prepared on its own: the scan hourly
/// preparation keeps in its branch state, and the publication of the blocks
/// since its last commit.
pub struct OwnBranch {
    scan: Box<Scan>,
    held: Vec<Held>,
}

impl OwnBranch {
    /// The branch's scan as last saved (`saved`, the scan's JSON), or new.
    pub fn load(saved: Option<impl io::Read>, format: FormatVersion) -> io::Result<Self> {
        let scan = match saved {
            Some(saved) => serde_json::from_reader(saved)?,
            None => Scan::new(format),
        };
        Ok(Self {
            scan: Box::new(scan),
            held: Vec::new(),
        })
    }

    /// Resume the scan from the frame hourly preparation's stream holds where
    /// it loads the branch.
    pub fn resume(&mut self, frame: &DecodedFrame) {
        self.scan.resume(frame);
    }

    /// Fold a block's pre-pass facts, then hold its publication.
    #[allow(clippy::too_many_arguments)]
    pub fn fold_facts(
        &mut self,
        facts: Box<dyn Any + Send>,
        source: &Source,
        context: &LoadContext,
        replay: &Replay,
        index: &BlockIndex,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        self.scan.configure(replay.format);
        let events = self.scan.events.len();
        let mut hours = None;
        let mut live = true;
        let folded = {
            let mut watch = |hour: i32, result: io::Result<()>| -> io::Result<bool> {
                hours = Some((hours.map_or(hour, |(first, _)| first), hour));
                live = observed(hour, result)?;
                Ok(live)
            };
            fold(&mut self.scan, facts, replay, index, &mut watch, check)?
        };
        if live && let Some(hours) = hours {
            self.hold(source, context, replay, index, hours, events)?;
        }
        Ok(folded)
    }

    /// Observe a block hour by hour, then hold its publication.
    #[allow(clippy::too_many_arguments)]
    pub fn observe_hours(
        &mut self,
        source: &Source,
        context: &LoadContext,
        replay: &Replay,
        index: &BlockIndex,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        self.scan.configure(replay.format);
        let events = self.scan.events.len();
        let mut hours = None;
        let mut live = true;
        let reached = {
            let mut watch = |hour: i32, result: io::Result<()>| -> io::Result<bool> {
                hours = Some((hours.map_or(hour, |(first, _)| first), hour));
                live = observed(hour, result)?;
                Ok(live)
            };
            observe_block(&mut self.scan, replay, index, None, &mut watch, check)?
        };
        if live && let Some(hours) = hours {
            self.hold(source, context, replay, index, hours, events)?;
        }
        Ok(reached)
    }

    /// Render the block's window and the manifest from the scan at the
    /// block's end, as hourly preparation's publication of the block does.
    fn hold(
        &mut self,
        source: &Source,
        context: &LoadContext,
        replay: &Replay,
        index: &BlockIndex,
        hours: (i32, i32),
        events: usize,
    ) -> io::Result<()> {
        let branch = index.branch;
        let ordinal = replay
            .index
            .iter()
            .filter(|block| block.branch == branch)
            .position(|block| block.first_hour == index.first_hour)
            .ok_or_else(|| io::Error::other("A division block is not in its branch"))?;
        // Rendering checks no cancellation: the family checks between hours.
        let running = AtomicBool::new(false);
        let mut held = Held {
            pages: Vec::new(),
            hours,
            events: events..self.scan.events.len(),
            error: None,
        };
        let id = branch.to_string();
        let window = DivisionLogParams {
            branch_id: id.clone(),
            window: ordinal,
        };
        match page(self.scan.reply(source, &window, context, &running)) {
            Ok(bytes) => held
                .pages
                .push((format!("division-log-{branch}"), ordinal as u64, bytes)),
            Err(error) => held.error = Some(error),
        }
        if held.error.is_none() {
            let params = BranchParams { branch_id: id };
            match page(self.scan.manifest(source, &params, context, &running)) {
                Ok(bytes) => held
                    .pages
                    .push((format!("division-log-manifest-{branch}"), 0, bytes)),
                Err(error) => held.error = Some(error),
            }
        }
        self.held.push(held);
        Ok(())
    }

    /// Bytes held for the next commit; once a block's publication failed,
    /// more than any batch holds, so the family commits right after it.
    pub fn pending_bytes(&self) -> usize {
        if self.held.iter().any(|held| held.error.is_some()) {
            return usize::MAX;
        }
        self.held
            .iter()
            .flat_map(|held| &held.pages)
            .map(|(_, _, page)| page.len())
            .sum()
    }

    /// Write the blocks' publication held since the last commit, in block
    /// order, as each block's end wrote it hourly: its window and the
    /// manifest, then its totals, fed with the block's events in hour order
    /// and flushed. A page that could not be rendered fails after the pages
    /// before it.
    pub fn publish_blocks(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        for held in std::mem::take(&mut self.held) {
            for (section, range, page) in &held.pages {
                writer.write_at(section, *range, page.as_slice())?;
                #[cfg(any(test, feature = "test-support"))]
                dashboard_base::prepared::pages::framing::note_compressed(page);
            }
            if let Some(error) = held.error {
                return Err(error);
            }
            let scan = &mut *self.scan;
            let (first, last) = held.hours;
            let events = &scan.events[held.events];
            scan.counts.observe(first, &[])?;
            let mut at = 0;
            while at < events.len() {
                let hour = events[at].row.read_hour;
                let end = at + events[at..].partition_point(|event| event.row.read_hour == hour);
                scan.counts.observe(hour, &events[at..end])?;
                at = end;
            }
            scan.counts.observe(last, &[])?;
            scan.counts.publish(writer, branch, cancelled)?;
        }
        Ok(())
    }

    /// The scan hourly preparation keeps in its branch state, as JSON.
    pub fn state_json(&self) -> io::Result<Vec<u8>> {
        Ok(serde_json::to_vec(&*self.scan)?)
    }

    pub fn state_census(
        &self,
        line: dashboard_base::prepared::state_census::Line,
    ) -> dashboard_base::prepared::state_census::Line {
        self.scan.state_census(line)
    }
}

#[cfg(test)]
#[path = "block/tests.rs"]
mod tests;
