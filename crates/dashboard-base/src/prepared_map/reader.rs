//! One exact-hour map reader for viewer playback and acceptance measurements.
use super::pages::{
    BranchPages, Header, chain_point, check_step, read_page, section as page_section,
};
use super::{
    Checkpoint, Current, Position, Progress, decode_checkpoint, gap_reason, header, inflate,
    invalid, section, validate_selection,
};
use replay::{container::Parallel, container::Replay};
use replay_data::{BranchId, Hour, rows::TableId, rows::tables};
use replay_frame::frame::endpoint_step::EndpointStep;
use replay_frame::{frame::DecodedFrame, frame::Frame, frame::FrameDecoder, frame::TableView};
use std::{
    io,
    ops::{ControlFlow, Range},
    sync::Arc,
    time::Instant,
};
use store::{SourceKey, Store};

mod load;
use load::Load;

#[cfg(test)]
mod reuse_tests;

#[cfg(test)]
type SourceReadHook = Box<dyn FnMut(Hour, &io::Result<Vec<Frame>>) + Send + Sync>;

#[derive(Clone)]
struct Block {
    index: usize,
    through: Hour,
    frames: Arc<Vec<Frame>>,
}

struct Admitted {
    frames: Vec<(Block, Range<usize>)>,
    cursor: Position,
}

struct Attempt {
    decoder: FrameDecoder,
    reached: Hour,
    cursor: Position,
    tail: Option<Admitted>,
    pages: u64,
    bytes: u64,
    stride: u32,
    next: Option<u64>,
}

struct DecodedPage {
    step: EndpointStep,
    #[cfg(test)]
    live: LivePage,
}

#[cfg(test)]
#[derive(Default)]
struct PagePayloads {
    live: std::sync::atomic::AtomicUsize,
    maximum: std::sync::atomic::AtomicUsize,
}

#[cfg(test)]
struct LivePage(Arc<PagePayloads>);

#[cfg(test)]
impl LivePage {
    fn new(payloads: &Arc<PagePayloads>) -> Self {
        use std::sync::atomic::Ordering::SeqCst;
        let live = payloads.live.fetch_add(1, SeqCst) + 1;
        payloads.maximum.fetch_max(live, SeqCst);
        Self(Arc::clone(payloads))
    }
}

#[cfg(test)]
impl Drop for LivePage {
    fn drop(&mut self) {
        self.0
            .live
            .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

struct Prefetch {
    branch: BranchId,
    stride: u32,
    from: Hour,
    range: u64,
    read: Option<std::thread::JoinHandle<io::Result<Prefetched>>>,
}

struct Prefetched {
    id: store::RangeId,
    header: Header,
    payload: DecodedPage,
}

impl Prefetch {
    fn is(&self, branch: BranchId, stride: u32, from: Hour, range: u64) -> bool {
        (self.branch, self.stride, self.from, self.range) == (branch, stride, from, range)
    }

    /// The page once read; None when its read failed.
    fn finish(mut self) -> Option<Prefetched> {
        self.read.take()?.join().ok()?.ok()
    }
}

impl Drop for Prefetch {
    fn drop(&mut self) {
        if let Some(read) = self.read.take() {
            let _finished = read.join();
        }
    }
}

#[cfg(test)]
thread_local! {
    /// Called after each checked page is speculatively applied.
    static PAGE_READ: std::cell::RefCell<Option<Box<dyn FnMut()>>> =
        const { std::cell::RefCell::new(None) };
    /// Called after each chain-search header read.
    static PAGE_HEADER_READ: std::cell::RefCell<Option<Box<dyn FnMut()>>> =
        const { std::cell::RefCell::new(None) };
    /// Whether this thread is reading or applying pages.
    static PAGE_WORK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Page ranges this thread's readers opened, headers included.
    static PAGE_RANGES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// Pages this thread's readers applied from a read ahead.
    static PAGES_READ_AHEAD: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Run `hook` after each speculative page application, for fault tests.
#[cfg(test)]
pub(super) fn on_page_read(hook: Option<Box<dyn FnMut()>>) {
    PAGE_READ.with(|read| *read.borrow_mut() = hook);
}

#[cfg(test)]
pub(super) fn on_page_header_read(hook: Option<Box<dyn FnMut()>>) {
    PAGE_HEADER_READ.with(|read| *read.borrow_mut() = hook);
}

/// Whether this thread is reading or applying pages, for fault tests.
#[cfg(test)]
pub(super) fn in_page_work() -> bool {
    PAGE_WORK.with(std::cell::Cell::get)
}

/// Page ranges this thread's readers opened, headers included.
#[cfg(test)]
pub(super) fn page_ranges_read() -> u64 {
    PAGE_RANGES.with(std::cell::Cell::get)
}

/// Pages this thread's readers applied from a read ahead.
#[cfg(test)]
pub(super) fn pages_read_ahead() -> u64 {
    PAGES_READ_AHEAD.with(std::cell::Cell::get)
}

#[cfg(test)]
struct PageWork(bool);

#[cfg(test)]
impl PageWork {
    fn enter() -> Self {
        Self(PAGE_WORK.with(|work| work.replace(true)))
    }
}

#[cfg(test)]
impl Drop for PageWork {
    fn drop(&mut self) {
        PAGE_WORK.with(|work| work.set(self.0));
    }
}

fn caller_priority() -> impl Fn() + Copy + Send + Sync + 'static {
    #[cfg(windows)]
    let priority = {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, GetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
        };
        // SAFETY: the current-thread pseudo handle is valid and is only read.
        match unsafe { GetThreadPriority(GetCurrentThread()) } {
            i32::MAX => THREAD_PRIORITY_BELOW_NORMAL,
            priority => priority,
        }
    };
    move || {
        #[cfg(windows)]
        // SAFETY: only the calling worker's scheduling priority is changed.
        unsafe {
            use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority};
            SetThreadPriority(GetCurrentThread(), priority);
        }
    }
}

/// Restores a sparse native checkpoint, then reads the recording's selected
/// updates through the target. Displayed-hour presentation and clocks stay with
/// the caller. Only one parsed source block is retained between requests.
pub struct Reader {
    store: Arc<Store>,
    key: SourceKey,
    progress: Progress,
    replay: Arc<Replay>,
    selected: Vec<TableId>,
    decoder: FrameDecoder,
    cursor: Option<Position>,
    block: Option<Block>,
    requested: Option<(BranchId, Hour)>,
    workers: usize,
    pages: Option<Vec<(BranchId, BranchPages)>>,
    page_cursor: bool,
    page_workers: usize,
    prefetch: Option<Prefetch>,
    #[cfg(test)]
    page_payloads: Arc<PagePayloads>,
    #[cfg(test)]
    source_read_hook: Option<SourceReadHook>,
    /// Native frames applied by the most recent request.
    pub applied_frames: u64,
    pub applied_pages: u64,
    pub page_bytes: u64,
    pub page_fallbacks: u64,
    pub page_skip: Option<&'static str>,
    /// Checkpoint headers read by the most recent seek.
    pub checkpoint_reads: u64,
    /// Whole checkpoint range bytes read by the most recent request.
    pub checkpoint_bytes: u64,
    /// Selected source block loads, including extension after a prefix seek.
    pub source_reads: u64,
    /// Container bytes of loaded source blocks; an upper bound on selected I/O.
    pub source_block_bytes: u64,
    /// Source frames parsed, including those preceding the native checkpoint.
    pub source_decoded_frames: u64,
    /// Time inside the selected source reader, including table decoding.
    pub source_load_us: u64,
    /// Frames in the one retained source block, for acceptance reporting.
    pub retained_source_frames: usize,
}

impl Reader {
    pub fn new(
        store: impl Into<Arc<Store>>,
        key: SourceKey,
        progress: Progress,
        replay: Arc<Replay>,
        selected: &[TableId],
    ) -> io::Result<Self> {
        validate_selection(selected)?;
        if progress.stride == 0 {
            return Err(invalid("map checkpoint stride is zero"));
        }
        let mut selection: Vec<u16> = selected.iter().map(|table| *table as u16).collect();
        selection.sort_unstable();
        if selection != progress.selected {
            return Err(invalid(
                "map checkpoint table selection differs from progress",
            ));
        }
        Ok(Self {
            store: store.into(),
            key,
            progress,
            replay,
            selected: selected.to_vec(),
            decoder: FrameDecoder::for_tables(selected),
            cursor: None,
            block: None,
            requested: None,
            workers: std::thread::available_parallelism()
                .map_or(1, |count| count.get() / 2)
                .max(1),
            pages: None,
            page_cursor: false,
            page_workers: 1,
            prefetch: None,
            #[cfg(test)]
            page_payloads: Arc::default(),
            applied_pages: 0,
            page_bytes: 0,
            page_fallbacks: 0,
            page_skip: None,
            #[cfg(test)]
            source_read_hook: None,
            applied_frames: 0,
            checkpoint_reads: 0,
            checkpoint_bytes: 0,
            source_reads: 0,
            source_block_bytes: 0,
            source_decoded_frames: 0,
            source_load_us: 0,
            retained_source_frames: 0,
        })
    }

    #[cfg(test)]
    pub(super) fn retained_source(&self) -> Option<(usize, Hour)> {
        self.block
            .as_ref()
            .map(|block| (block.index, block.through))
    }

    // Pages move the cursor without loading source; a native request can reload the released block.
    pub fn release_passed_source(&mut self) -> Option<impl Send + 'static + use<>> {
        let passed = self.page_cursor
            && self
                .block
                .as_ref()
                .zip(self.cursor)
                .is_some_and(|(block, cursor)| block.index != cursor.block);
        if !passed {
            return None;
        }
        self.retained_source_frames = 0;
        self.block.take()
    }

    pub fn with_pages(mut self, pages: Vec<(BranchId, BranchPages)>) -> io::Result<Self> {
        self.bind_pages(pages)?;
        Ok(self)
    }

    pub fn with_committed_pages(self) -> Self {
        self.with_committed_pages_reporting().0
    }

    pub fn with_committed_pages_reporting(mut self) -> (Self, io::Result<()>) {
        let bound = super::pages::committed(&self.store, &self.key, &self.progress)
            .and_then(|pages| self.bind_pages(pages));
        (self, bound)
    }

    /// The committed chains bound, in branch order; none without pages.
    pub fn page_chains(&self) -> &[(BranchId, BranchPages)] {
        self.pages.as_deref().unwrap_or(&[])
    }

    pub fn held_hour(&self) -> Option<Hour> {
        self.decoder.current().map(|frame| frame.hour)
    }

    pub fn page_entry(&self, branch: BranchId, from: Hour, limit: Hour) -> Option<Hour> {
        let (_, pages) = self.page_chains().iter().find(|(at, _)| *at == branch)?;
        pages.chains.iter().rev().find_map(|chain| {
            let stride = i64::from(chain.stride);
            let entry = (i64::from(from) + stride - 1).div_euclid(stride) * stride;
            let fits = entry + stride <= i64::from(limit)
                && chain.first.is_some_and(|first| i64::from(first) <= entry)
                && chain
                    .last
                    .is_some_and(|last| entry + stride <= i64::from(last));
            fits.then(|| Hour::try_from(entry).ok()).flatten()
        })
    }

    pub fn set_page_workers(&mut self, workers: usize) {
        self.page_workers = workers.max(1);
    }

    fn bind_pages(&mut self, pages: Vec<(BranchId, BranchPages)>) -> io::Result<()> {
        if pages.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
            return Err(invalid("map page branches are not distinct and ascending"));
        }
        for (_, branch) in &pages {
            branch.check(&self.progress.selected)?;
        }
        self.pages = Some(pages);
        Ok(())
    }

    pub fn replay_handle(&self) -> &Arc<Replay> {
        &self.replay
    }

    pub fn range(&self, branch: BranchId) -> Option<(Hour, Hour)> {
        self.progress.ranges.get(&branch).copied()
    }

    pub fn covers(&self, branch: BranchId, hour: Hour) -> bool {
        self.progress
            .branches
            .get(&branch)
            .and_then(|p| p.last_hour)
            .is_some_and(|last| hour <= last)
    }

    pub fn walk_start(&self, branch: BranchId, hour: Hour) -> io::Result<Option<Hour>> {
        if let Some(frame) = self.decoder.current().filter(|frame| {
            frame.branch == branch
                && frame.hour < hour
                && (self.steps_to(branch, hour)
                    || i64::from(hour) - i64::from(frame.hour) < i64::from(self.progress.stride))
        }) {
            return Ok(Some(frame.hour));
        }
        Ok(self.find_before(branch, hour)?.0.map(|(_, hour)| hour))
    }

    fn find_before(&self, branch: BranchId, hour: Hour) -> io::Result<(Option<(u64, Hour)>, u64)> {
        let mut low = 0;
        let mut high = self
            .progress
            .branches
            .get(&branch)
            .map_or(0, |p| p.checkpoints);
        let mut before = None;
        let mut reads = 0;
        while low < high {
            let mid = low + (high - low) / 2;
            let (page_branch, page_hour) = header(self.page(branch, mid)?)?;
            reads += 1;
            if page_branch != branch {
                return Err(invalid("map checkpoint belongs to another branch"));
            }
            if page_hour <= hour {
                before = Some((mid, page_hour));
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        Ok((before, reads))
    }

    pub fn seek(&mut self, branch: BranchId, hour: Hour) -> io::Result<Option<Current>> {
        self.seek_with(branch, hour, |_| {})
    }

    /// `None` leaves an unprepared target to ordinary playback. Recorded holes
    /// within the accepted prefix keep their exact named gap reasons.
    pub fn seek_with(
        &mut self,
        branch: BranchId,
        hour: Hour,
        passed: impl FnMut(&DecodedFrame),
    ) -> io::Result<Option<Current>> {
        if self.seek_native(branch, hour, passed)? {
            self.current_at(branch, hour).map(Some)
        } else {
            Ok(None)
        }
    }

    fn seek_native(
        &mut self,
        branch: BranchId,
        hour: Hour,
        passed: impl FnMut(&DecodedFrame),
    ) -> io::Result<bool> {
        self.reset_metrics();
        self.prefetch = None;
        if !self.store.is_reusable(&self.key)? || !self.covers(branch, hour) {
            return Ok(false);
        }
        let (before, reads) = self.find_before(branch, hour)?;
        self.checkpoint_reads = reads;
        self.page_cursor = false;
        let Some((range, checkpoint_hour)) = before else {
            self.decoder = FrameDecoder::for_tables(&self.selected);
            self.cursor = None;
            self.requested = Some((branch, hour));
            self.gap(branch, hour)?;
            return Ok(true);
        };
        let file = self.page(branch, range)?;
        self.checkpoint_bytes = file.len();
        let checkpoint = decode_checkpoint(file, &self.selected)?;
        if checkpoint.frame.hour != checkpoint_hour {
            return Err(invalid("map checkpoint search differs from its header"));
        }
        self.validate_checkpoint(branch, &checkpoint)?;
        self.cursor = Some(checkpoint.position);
        self.decoder = FrameDecoder::resume_for_tables(&self.selected, checkpoint.frame)?;
        self.advance_to(branch, hour, passed, Load::RequestedPrefix)?;
        self.requested = Some((branch, hour));
        Ok(true)
    }

    /// Applies one hour in place for a step. A block load is retained so later
    /// steps never repeat source-prefix decoding within that block.
    pub fn advance(
        &mut self,
        branch: BranchId,
        hour: Hour,
        passed: impl FnMut(&DecodedFrame),
    ) -> io::Result<Option<Current>> {
        if self.advance_without_output(branch, hour, passed)? {
            self.current_at(branch, hour).map(Some)
        } else {
            Ok(None)
        }
    }

    /// Advances through a cancellation checkpoint without exporting an owned
    /// endpoint snapshot. Visitors still receive the earlier applied hours,
    /// and all source, campaign and gap validation is unchanged. `false` leaves
    /// a target outside prepared coverage to the ordinary replay reader.
    pub fn advance_without_output(
        &mut self,
        branch: BranchId,
        hour: Hour,
        passed: impl FnMut(&DecodedFrame),
    ) -> io::Result<bool> {
        self.reset_metrics();
        let ahead = self.prefetch.take().filter(|ahead| {
            ahead.branch == branch
                && ahead.from == hour
                && self
                    .decoder
                    .current()
                    .is_some_and(|frame| frame.branch == branch && frame.hour == hour)
        });
        if !self.store.is_reusable(&self.key)? || !self.covers(branch, hour) {
            return Ok(false);
        }
        if !self.continues(branch, hour) {
            return self.seek_native(branch, hour, passed);
        }
        let load = self.load(branch, hour);
        self.advance_to(branch, hour, passed, load)?;
        self.requested = Some((branch, hour));
        self.prefetch = ahead;
        Ok(true)
    }

    pub fn advance_paged(
        &mut self,
        branch: BranchId,
        hour: Hour,
        visit_from: Hour,
        passed: impl FnMut(&DecodedFrame),
    ) -> io::Result<Option<Current>> {
        if self.advance_paged_without_output(branch, hour, visit_from, passed)? {
            self.current_at(branch, hour).map(Some)
        } else {
            Ok(None)
        }
    }

    pub fn advance_paged_without_output(
        &mut self,
        branch: BranchId,
        hour: Hour,
        visit_from: Hour,
        mut passed: impl FnMut(&DecodedFrame),
    ) -> io::Result<bool> {
        let mut visit = |frame: &DecodedFrame| {
            if frame.hour >= visit_from {
                passed(frame);
            }
        };
        let limit = if visit_from >= hour {
            Some(hour)
        } else {
            visit_from.checked_sub(1)
        };
        let at = self.decoder.current().map(|frame| frame.hour);
        let Some((limit, at)) = limit
            .zip(at)
            .filter(|&(_, at)| self.pages.is_some() && at < hour && self.continues(branch, hour))
        else {
            return self.advance_without_output(branch, hour, &mut visit);
        };
        self.reset_metrics();
        let ahead = self.prefetch.take();
        if !self.store.is_reusable(&self.key)? || !self.covers(branch, hour) {
            return Ok(false);
        }
        self.gap(branch, hour)?;
        let retained = (self.block.clone(), self.retained_source_frames);
        let attempt = {
            #[cfg(test)]
            let _work = PageWork::enter();
            self.attempt_pages(branch, at, limit, ahead, &mut || true)
                .map(|attempt| match attempt {
                    ControlFlow::Continue(attempt) => attempt,
                    ControlFlow::Break(()) => unreachable!("uncancellable page request"),
                })
                .and_then(|attempt| match attempt {
                    Some(mut attempt) if attempt.reached != hour => {
                        let load = self.load(branch, hour);
                        let tail = self.admit(
                            branch,
                            hour,
                            Some(attempt.reached),
                            attempt.cursor,
                            true,
                            load,
                        )?;
                        attempt.tail = Some(tail);
                        Ok(Some(attempt))
                    }
                    attempt => Ok(attempt),
                })
        };
        let attempt = match attempt {
            Ok(Some(attempt)) => attempt,
            Ok(None) => return self.advance_without_output(branch, hour, &mut visit),
            Err(_) => {
                (self.block, self.retained_source_frames) = retained;
                let handled = self.advance_without_output(branch, hour, &mut visit);
                self.page_fallbacks = 1;
                return handled;
            }
        };
        if !self.store.is_reusable(&self.key)? {
            (self.block, self.retained_source_frames) = retained;
            return Ok(false);
        }
        self.decoder = attempt.decoder;
        match attempt.tail {
            Some(tail) => self.apply_admitted(hour, tail, &mut visit)?,
            None => {
                self.cursor = Some(attempt.cursor);
                self.page_cursor = true;
            }
        }
        self.requested = Some((branch, hour));
        self.applied_pages = attempt.pages;
        self.page_bytes = attempt.bytes;
        if attempt.reached == hour
            && let Some(range) = attempt.next
        {
            self.read_ahead(branch, hour, attempt.stride, range);
        }
        Ok(true)
    }

    pub fn advance_pages(
        &mut self,
        branch: BranchId,
        hour: Hour,
        visit_from: Hour,
        mut wanted: impl FnMut() -> bool,
    ) -> io::Result<ControlFlow<(), Option<Hour>>> {
        if !wanted() {
            return Ok(ControlFlow::Break(()));
        }
        let limit = if visit_from >= hour {
            Some(hour)
        } else {
            visit_from.checked_sub(1)
        };
        let at = self
            .decoder
            .current()
            .filter(|frame| frame.branch == branch)
            .map(|frame| frame.hour);
        self.page_skip = if self.pages.is_none() {
            Some("unbound")
        } else {
            match limit.zip(at) {
                None => Some("no-held-state"),
                Some((limit, at)) if at >= limit => Some("held-at-or-past-limit"),
                Some(_) => None,
            }
        };
        let Some((limit, at)) = limit.zip(at).filter(|_| self.page_skip.is_none()) else {
            return Ok(ControlFlow::Continue(None));
        };
        self.reset_metrics();
        let ahead = self.prefetch.take();
        if !self.store.is_reusable(&self.key)? || !self.covers(branch, hour) {
            self.page_skip = Some("store-not-reusable");
            return Ok(ControlFlow::Continue(None));
        }
        self.gap(branch, hour)?;
        let attempt = {
            #[cfg(test)]
            let _work = PageWork::enter();
            self.attempt_pages(branch, at, limit, ahead, &mut wanted)
        };
        if !wanted() || matches!(&attempt, Ok(ControlFlow::Break(()))) {
            return Ok(ControlFlow::Break(()));
        }
        let on_grid = self.page_chains().iter().any(|(_, pages)| {
            pages
                .chains
                .iter()
                .any(|chain| chain_point(at, chain.stride))
        });
        self.page_skip = match &attempt {
            Ok(ControlFlow::Continue(Some(_))) => None,
            Ok(ControlFlow::Continue(None)) if on_grid => Some("no-page-from-held"),
            Ok(ControlFlow::Continue(None)) => Some("held-off-every-grid"),
            Ok(ControlFlow::Break(())) => unreachable!("cancellation already returned"),
            Err(_) => Some("page-refused"),
        };
        match attempt {
            Ok(ControlFlow::Continue(Some(attempt))) => {
                self.decoder = attempt.decoder;
                self.cursor = Some(attempt.cursor);
                self.page_cursor = true;
                self.applied_pages = attempt.pages;
                self.page_bytes = attempt.bytes;
                if attempt.reached == hour
                    && let Some(range) = attempt.next
                {
                    self.read_ahead(branch, hour, attempt.stride, range);
                }
                Ok(ControlFlow::Continue(Some(attempt.reached)))
            }
            Ok(ControlFlow::Continue(None)) => Ok(ControlFlow::Continue(None)),
            Ok(ControlFlow::Break(())) => unreachable!("cancellation already returned"),
            Err(_) => {
                self.page_fallbacks = 1;
                Ok(ControlFlow::Continue(None))
            }
        }
    }

    fn continues(&self, branch: BranchId, hour: Hour) -> bool {
        self.decoder.current().is_some_and(|frame| {
            frame.branch == branch
                && frame.hour <= hour
                && (self.steps_to(branch, hour)
                    || i64::from(hour) - i64::from(frame.hour) < i64::from(self.progress.stride))
        })
    }

    fn attempt_pages(
        &self,
        branch: BranchId,
        at: Hour,
        limit: Hour,
        mut ahead: Option<Prefetch>,
        wanted: &mut impl FnMut() -> bool,
    ) -> io::Result<ControlFlow<(), Option<Attempt>>> {
        if !wanted() {
            return Ok(ControlFlow::Break(()));
        }
        let progress = self
            .progress
            .branches
            .get(&branch)
            .ok_or_else(|| invalid("map page branch is not accepted"))?;
        let (last_hour, last_block) = progress
            .last_hour
            .zip(progress.last_block)
            .ok_or_else(|| invalid("map page branch is not accepted"))?;
        let Some((_, pages)) = self.pages.as_ref().and_then(|pages| {
            pages
                .binary_search_by_key(&branch, |&(branch, _)| branch)
                .ok()
                .map(|at| &pages[at])
        }) else {
            return Ok(ControlFlow::Continue(None));
        };
        for chain in pages.chains.iter().rev().filter(|chain| {
            chain.ranges > 0
                && chain_point(at, chain.stride)
                && i64::from(at) + i64::from(chain.stride) <= i64::from(limit)
        }) {
            let (stride, ranges) = (chain.stride, chain.ranges);
            let first = match self.page_from(branch, stride, ranges, at, wanted)? {
                ControlFlow::Break(()) => return Ok(ControlFlow::Break(())),
                ControlFlow::Continue(None) => continue,
                ControlFlow::Continue(Some(first)) => first,
            };
            let mut decoder = self.decoder.clone();
            let on_worker = caller_priority();
            let (mut count, mut total_bytes) = (0, 0);
            let mut last = None;
            let (mut from, mut next) = (at, first);
            for range in first..ranges {
                if !wanted() {
                    return Ok(ControlFlow::Break(()));
                }
                let mut file = self.page_range(branch, stride, range)?;
                let bytes = file.len();
                let read = ahead
                    .take()
                    .filter(|ahead| ahead.is(branch, stride, from, range))
                    .and_then(Prefetch::finish)
                    .filter(|page| page.id == file.id());
                let header = match &read {
                    Some(page) => page.header,
                    None => Header::read(&mut file)?,
                };
                if header.to > limit || header.to > last_hour {
                    break;
                }
                let source = self.replay.index.get(header.block).is_some_and(|index| {
                    index.branch == branch
                        && header.block <= last_block
                        && index.first_hour <= header.to
                        && header.to <= index.last_hour
                });
                if header.branch != branch
                    || header.stride != stride
                    || header.from != from
                    || !source
                {
                    return Err(invalid("map page does not continue its chain"));
                }
                let payload = match read {
                    // Read from these bytes and checked as below.
                    Some(page) => {
                        #[cfg(test)]
                        PAGES_READ_AHEAD.with(|pages| pages.set(pages.get() + 1));
                        page.payload
                    }
                    None => {
                        let step = EndpointStep::decode(&inflate(file)?)?;
                        check_step(&header, &step, &self.selected)?;
                        DecodedPage {
                            step,
                            #[cfg(test)]
                            live: LivePage::new(&self.page_payloads),
                        }
                    }
                };
                if !wanted() {
                    return Ok(ControlFlow::Break(()));
                }
                let DecodedPage {
                    step,
                    #[cfg(test)]
                    live,
                } = payload;
                let reached =
                    decoder.advance_endpoint_parallel(step, self.page_workers, &on_worker)?;
                #[cfg(test)]
                drop(live);
                Self::validate_campaign(&self.replay.campaign_id, reached)?;
                count += 1;
                total_bytes += bytes;
                last = Some(header);
                #[cfg(test)]
                PAGE_READ.with(|hook| {
                    if let Some(hook) = hook.borrow_mut().as_mut() {
                        hook();
                    }
                });
                from = header.to;
                next = range + 1;
            }
            if !wanted() {
                return Ok(ControlFlow::Break(()));
            }
            let Some(last) = last else { continue };
            if !self.store.is_reusable(&self.key)? {
                return Err(invalid("map store was cleared during page read"));
            }
            return Ok(ControlFlow::Continue(Some(Attempt {
                decoder,
                reached: last.to,
                cursor: Position {
                    block: last.block,
                    next_frame: 0,
                },
                tail: None,
                pages: count,
                bytes: total_bytes,
                stride,
                next: (next < ranges).then_some(next),
            })));
        }
        Ok(ControlFlow::Continue(None))
    }

    fn read_ahead(&mut self, branch: BranchId, from: Hour, stride: u32, range: u64) {
        let Ok(selected) = engine_allocation::copy_vec(&self.selected) else {
            return;
        };
        let (store, key) = (Arc::clone(&self.store), self.key.clone());
        let priority = caller_priority();
        #[cfg(test)]
        let payloads = Arc::clone(&self.page_payloads);
        let read = std::thread::Builder::new()
            .name("map page read-ahead".into())
            .spawn(move || -> io::Result<Prefetched> {
                priority();
                let mut file = store
                    .read(&key, &page_section(branch, stride), range)?
                    .ok_or_else(|| invalid("map page is missing"))?;
                let id = file.id();
                let (header, step) = read_page(&mut file, &selected)?;
                Ok(Prefetched {
                    id,
                    header,
                    payload: DecodedPage {
                        step,
                        #[cfg(test)]
                        live: LivePage::new(&payloads),
                    },
                })
            });
        self.prefetch = read.ok().map(|read| Prefetch {
            branch,
            stride,
            from,
            range,
            read: Some(read),
        });
    }

    #[cfg(test)]
    pub(super) fn page_payload_counts(&self) -> (usize, usize) {
        use std::sync::atomic::Ordering::SeqCst;
        (
            self.page_payloads.live.load(SeqCst),
            self.page_payloads.maximum.load(SeqCst),
        )
    }

    /// The page being read ahead: its branch, stride, chain point and range.
    #[cfg(test)]
    pub(super) fn reading_ahead(&self) -> Option<(BranchId, u32, Hour, u64)> {
        self.prefetch
            .as_ref()
            .map(|ahead| (ahead.branch, ahead.stride, ahead.from, ahead.range))
    }

    /// The range of `stride`'s page that starts at `at`, if one does.
    fn page_from(
        &self,
        branch: BranchId,
        stride: u32,
        ranges: u64,
        at: Hour,
        wanted: &mut impl FnMut() -> bool,
    ) -> io::Result<ControlFlow<(), Option<u64>>> {
        let (mut low, mut high) = (0, ranges);
        let mut after = None;
        while low < high {
            if !wanted() {
                return Ok(ControlFlow::Break(()));
            }
            let mid = low + (high - low) / 2;
            let page = Header::read(self.page_range(branch, stride, mid)?)?;
            #[cfg(test)]
            PAGE_HEADER_READ.with(|hook| {
                if let Some(hook) = hook.borrow_mut().as_mut() {
                    hook();
                }
            });
            if page.branch != branch || page.stride != stride {
                return Err(invalid("map page belongs to another chain"));
            }
            if page.to > at {
                after = Some((mid, page.from));
                high = mid;
            } else {
                low = mid + 1;
            }
        }
        Ok(ControlFlow::Continue(
            after
                .filter(|&(_, from)| from == at)
                .map(|(range, _)| range),
        ))
    }

    fn page_range(&self, branch: BranchId, stride: u32, range: u64) -> io::Result<store::Range> {
        #[cfg(test)]
        PAGE_RANGES.with(|ranges| ranges.set(ranges.get() + 1));
        self.store
            .read(&self.key, &page_section(branch, stride), range)?
            .ok_or_else(|| invalid("map page is missing"))
    }

    #[cfg(test)]
    #[allow(clippy::type_complexity)]
    pub(super) fn continuation(
        &self,
    ) -> (
        Option<Position>,
        bool,
        Option<(BranchId, Hour)>,
        Option<(usize, Hour)>,
    ) {
        (
            self.cursor,
            self.page_cursor,
            self.requested,
            self.block
                .as_ref()
                .map(|block| (block.index, block.through)),
        )
    }

    fn current_at(&self, branch: BranchId, hour: Hour) -> io::Result<Current> {
        match self.decoder.current() {
            Some(frame) if frame.branch == branch && frame.hour == hour => {
                Ok(Current::Frame(Arc::new(frame.clone())))
            }
            _ => self.gap(branch, hour),
        }
    }

    fn reset_metrics(&mut self) {
        self.applied_frames = 0;
        self.applied_pages = 0;
        self.page_bytes = 0;
        self.page_fallbacks = 0;
        self.checkpoint_reads = 0;
        self.checkpoint_bytes = 0;
        self.source_reads = 0;
        self.source_block_bytes = 0;
        self.source_decoded_frames = 0;
        self.source_load_us = 0;
    }

    fn steps_to(&self, branch: BranchId, hour: Hour) -> bool {
        self.requested
            .is_some_and(|(b, h)| b == branch && h.checked_add(1) == Some(hour))
    }

    fn load(&self, branch: BranchId, hour: Hour) -> Load {
        if self.steps_to(branch, hour) {
            Load::AcceptedBlock
        } else if self.requested.is_some_and(|(b, h)| b == branch && h < hour) {
            Load::AcceptedBlockOrPrefix
        } else {
            Load::RequestedPrefix
        }
    }

    fn page(&self, branch: BranchId, range: u64) -> io::Result<store::Range> {
        self.store
            .read(&self.key, &section(branch), range)?
            .ok_or_else(|| invalid("map checkpoint page is missing"))
    }

    fn advance_to(
        &mut self,
        branch: BranchId,
        hour: Hour,
        passed: impl FnMut(&DecodedFrame),
        load: Load,
    ) -> io::Result<()> {
        if self
            .decoder
            .current()
            .is_some_and(|f| f.branch == branch && f.hour == hour)
        {
            return Ok(());
        }
        // Validate gap metadata before any visitor, even without an endpoint snapshot.
        self.gap(branch, hour)?;
        let Some(cursor) = self.cursor else {
            return Ok(());
        };
        let previous = self.decoder.current().map(|frame| frame.hour);
        let admitted = self.admit(branch, hour, previous, cursor, self.page_cursor, load)?;
        self.apply_admitted(hour, admitted, passed)
    }

    fn admit(
        &mut self,
        branch: BranchId,
        hour: Hour,
        mut previous: Option<Hour>,
        mut cursor: Position,
        page_end: bool,
        load: Load,
    ) -> io::Result<Admitted> {
        let progress = &self.progress.branches[&branch];
        let last_block = progress
            .last_block
            .ok_or_else(|| invalid("map accepted source cursor is missing"))?;
        let last_hour = progress.last_hour.unwrap();
        let mut page_end = page_end.then_some(cursor.block);
        let mut admitted = Vec::new();
        // Admit every source load before visitors, so a refusal cannot repeat their history.
        while cursor.block <= last_block {
            let index = self
                .replay
                .index
                .get(cursor.block)
                .ok_or_else(|| invalid("map source cursor exceeds recording"))?;
            if index.branch != branch {
                cursor.block += 1;
                cursor.next_frame = 0;
                continue;
            }
            if index.first_hour > hour {
                break;
            }
            // A checkpoint at the block's last hour already consumed its final frame.
            if previous == Some(index.last_hour) {
                cursor.block += 1;
                cursor.next_frame = 0;
                continue;
            }
            let accepted_end = index.last_hour.min(last_hour);
            let required = load.required(hour, accepted_end);
            if self
                .block
                .as_ref()
                .is_none_or(|block| block.index != cursor.block || block.through < required)
            {
                self.block = None;
                self.retained_source_frames = 0;
                let on_worker = || {
                    #[cfg(windows)]
                    unsafe {
                        use windows_sys::Win32::System::Threading::{
                            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
                        };
                        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
                    }
                };
                let parallel = Parallel {
                    workers: self.workers,
                    on_worker: &on_worker,
                };
                let started = Instant::now();
                let result = load.read(required, accepted_end, |through| {
                    let result = self.replay.read_block_selected_through(
                        index,
                        Some(&self.selected),
                        through,
                        (self.workers > 1).then_some(&parallel),
                    );
                    #[cfg(test)]
                    if let Some(hook) = self.source_read_hook.as_mut() {
                        hook(through, &result);
                    }
                    result
                });
                self.source_load_us +=
                    u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
                let (through, frames) = result?;
                self.source_reads += 1;
                self.source_block_bytes += index.length;
                self.source_decoded_frames += frames.len() as u64;
                self.retained_source_frames = frames.len();
                self.block = Some(Block {
                    index: cursor.block,
                    through,
                    frames: Arc::new(frames),
                });
            }
            let block = self.block.as_ref().unwrap();
            if page_end == Some(cursor.block) {
                page_end = None;
                cursor.next_frame = block
                    .frames
                    .partition_point(|frame| Some(frame.hour) <= previous);
                if cursor.next_frame == 0 {
                    return Err(invalid("map page end is not a source frame"));
                }
            }
            if cursor.next_frame > block.frames.len()
                || (cursor.next_frame > 0
                    && block
                        .frames
                        .get(cursor.next_frame - 1)
                        .map(|frame| frame.hour)
                        != previous)
            {
                return Err(invalid("map source cursor differs from decoded baseline"));
            }
            let start = cursor.next_frame;
            while let Some(frame) = block
                .frames
                .get(cursor.next_frame)
                .filter(|f| f.hour <= hour)
            {
                if frame.branch != branch || previous.is_some_and(|h| frame.hour <= h) {
                    return Err(invalid(
                        "map source frames do not advance within their branch",
                    ));
                }
                previous = Some(frame.hour);
                cursor.next_frame += 1;
            }
            if start != cursor.next_frame {
                engine_allocation::reserve(&mut admitted, 1)?;
                admitted.push((block.clone(), start..cursor.next_frame));
            }
            if cursor.next_frame < block.frames.len() || block.through < index.last_hour {
                break;
            }
            cursor.block += 1;
            cursor.next_frame = 0;
        }
        // Check Clear after all I/O and before touching an external visitor.
        if self.source_reads != 0 && !self.store.is_reusable(&self.key)? {
            return Err(invalid("map store was cleared during source read"));
        }
        Ok(Admitted {
            frames: admitted,
            cursor,
        })
    }

    fn apply_admitted(
        &mut self,
        hour: Hour,
        admitted: Admitted,
        mut passed: impl FnMut(&DecodedFrame),
    ) -> io::Result<()> {
        for (block, range) in admitted.frames {
            for offset in range {
                let decoded = self.decoder.advance(&block.frames[offset])?;
                Self::validate_campaign(&self.replay.campaign_id, decoded)?;
                self.applied_frames += 1;
                if decoded.hour == hour {
                    self.cursor = Some(Position {
                        block: block.index,
                        next_frame: offset + 1,
                    });
                    self.page_cursor = false;
                    return Ok(());
                }
                passed(decoded);
            }
        }
        self.cursor = Some(admitted.cursor);
        self.page_cursor = false;
        Ok(())
    }

    fn validate_checkpoint(&self, branch: BranchId, checkpoint: &Checkpoint) -> io::Result<()> {
        let progress = &self.progress.branches[&branch];
        if checkpoint.frame.branch != branch
            || checkpoint.position.next_frame == 0
            || progress
                .last_block
                .is_none_or(|block| checkpoint.position.block > block)
            || progress
                .last_hour
                .is_none_or(|hour| checkpoint.frame.hour > hour)
        {
            return Err(invalid("map checkpoint cursor is invalid"));
        }
        let source = self
            .replay
            .index
            .get(checkpoint.position.block)
            .ok_or_else(|| invalid("map checkpoint source cursor is invalid"))?;
        if source.branch != branch
            || checkpoint.frame.hour < source.first_hour
            || checkpoint.frame.hour > source.last_hour
        {
            return Err(invalid("map checkpoint hour differs from source metadata"));
        }
        Self::validate_campaign(&self.replay.campaign_id, &checkpoint.frame)
    }

    fn validate_campaign(campaign_id: &[u8], frame: &DecodedFrame) -> io::Result<()> {
        if let TableView::Complete(rows) = &*frame.tables[TableId::Campaign.index()]
            && rows
                .rows::<tables::Campaign>()
                .is_none_or(|rows| rows.len() != 1 || rows[0].id != campaign_id)
        {
            return Err(invalid(
                "frame campaign identity differs from replay header",
            ));
        }
        Ok(())
    }

    fn gap(&self, branch: BranchId, hour: Hour) -> io::Result<Current> {
        Ok(Current::Gap {
            branch,
            hour,
            reason: gap_reason(&self.store, &self.key, &self.progress, branch, hour)?,
        })
    }
}

#[cfg(all(test, windows))]
mod priority_tests {
    #[test]
    fn page_threads_run_at_the_asking_threads_priority() {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, GetThreadPriority, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
            THREAD_PRIORITY_NORMAL,
        };
        for priority in [THREAD_PRIORITY_BELOW_NORMAL, THREAD_PRIORITY_NORMAL] {
            let seen = std::thread::spawn(move || {
                // SAFETY: this test owns the current thread.
                assert_ne!(
                    unsafe { SetThreadPriority(GetCurrentThread(), priority) },
                    0
                );
                let on_worker = super::caller_priority();
                std::thread::spawn(move || {
                    on_worker();
                    // SAFETY: the current-thread pseudo handle is valid.
                    unsafe { GetThreadPriority(GetCurrentThread()) }
                })
                .join()
                .unwrap()
            })
            .join()
            .unwrap();
            assert_eq!(seen, priority);
        }
    }
}
