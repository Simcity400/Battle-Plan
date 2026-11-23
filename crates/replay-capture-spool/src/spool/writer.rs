use super::*;
use replay_frame::frame::EncodedFrame;

fn encode_frame_record(frame: EncodedFrame) -> io::Result<Vec<u8>> {
    frame.into_spool_record()
}

pub struct SealedRecord {
    bytes: Vec<u8>,
    frame: Option<(BranchId, Hour, bool)>,
    gap: Option<(BranchId, Hour, Hour)>,
}

impl SealedRecord {
    pub fn context(record: Record) -> io::Result<Self> {
        if matches!(record, Record::Frame(_)) {
            return Err(invalid_static("seal frames through the encoded-frame API"));
        }
        Ok(Self {
            bytes: encode_record(&record)?,
            frame: None,
            gap: match record {
                Record::Gap(gap) => Some((gap.branch, gap.first_hour, gap.last_hour)),
                _ => None,
            },
        })
    }

    pub fn frame(frame: EncodedFrame) -> io::Result<Self> {
        let identity = (frame.branch(), frame.hour(), frame.keyframe());
        Ok(Self {
            bytes: frame.into_spool_record()?,
            frame: Some(identity),
            gap: None,
        })
    }

    pub fn gap_range(&self) -> Option<(BranchId, Hour, Hour)> {
        self.gap
    }

    pub fn storage_bytes(&self) -> usize {
        self.bytes.capacity()
    }

    fn accept(&self, state: &mut SegmentState) -> io::Result<()> {
        if let Some((branch, hour, keyframe)) = self.frame {
            state.accept_frame(branch, hour, keyframe)
        } else {
            let mut input = Decoder::new(&self.bytes[RECORD_HEADER_BYTES..]);
            state.accept(&Record::decode(&mut input)?)
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BackgroundSyncWork {
    pub requested: u64,
    pub completed: u64,
    pub total_ns: u64,
    pub max_ns: u64,
    pub pending: bool,
    pub threads: u64,
}

pub trait SegmentSink: Write {
    fn sync_all(&mut self) -> io::Result<()>;

    fn start_sync(&mut self) -> io::Result<()> {
        self.sync_all()
    }

    fn poll_sync(&mut self) -> io::Result<bool> {
        Ok(true)
    }

    fn background_sync_work(&self) -> Option<BackgroundSyncWork> {
        None
    }
}

impl SegmentSink for File {
    fn sync_all(&mut self) -> io::Result<()> {
        File::sync_all(self).map_err(allocation_error)
    }
}

pub struct FileSink {
    file: File,
    sync_file: File,
    worker: Option<DurabilityWorker>,
    pending: bool,
    work: BackgroundSyncWork,
    ordered_sync: bool,
}

struct DurabilityWorker {
    shared: Arc<DurabilityShared>,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[derive(Default)]
struct DurabilityShared {
    state: Mutex<DurabilityState>,
    changed: Condvar,
}

#[derive(Default)]
struct DurabilityState {
    requested: bool,
    finished: Option<(u64, io::Result<()>)>,
    stop: bool,
}

impl DurabilityShared {
    fn lock(&self) -> std::sync::MutexGuard<'_, DurabilityState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn serve(&self, file: File) {
        let mut state = self.lock();
        loop {
            if state.requested {
                state.requested = false;
                drop(state);
                let started = Instant::now();
                let result = file.sync_all();
                let elapsed = started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
                state = self.lock();
                state.finished = Some((elapsed, result));
                self.changed.notify_all();
            } else if state.stop {
                return;
            } else {
                state = self
                    .changed
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        }
    }
}

impl DurabilityWorker {
    fn start(file: File) -> io::Result<Self> {
        let shared = Arc::new(DurabilityShared::default());
        let worker = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("replay-durability".into())
            .spawn(move || worker.serve(file))?;
        Ok(Self {
            shared,
            thread: Some(thread),
        })
    }

    fn request(&self) {
        let mut state = self.shared.lock();
        state.requested = true;
        self.shared.changed.notify_all();
    }

    fn busy(&self) -> bool {
        self.shared.lock().finished.is_none()
            && self
                .thread
                .as_ref()
                .is_some_and(|thread| !thread.is_finished())
    }

    fn wait(&self) -> (u64, io::Result<()>) {
        let mut state = self.shared.lock();
        loop {
            if let Some(finished) = state.finished.take() {
                return finished;
            }
            if self
                .thread
                .as_ref()
                .is_none_or(|thread| thread.is_finished())
            {
                return (0, Err(io::Error::from(io::ErrorKind::Other)));
            }
            state = self
                .shared
                .changed
                .wait_timeout(state, Duration::from_millis(50))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

impl Drop for DurabilityWorker {
    fn drop(&mut self) {
        self.shared.lock().stop = true;
        self.shared.changed.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(windows)]
fn independent_sync_file(file: &File) -> io::Result<File> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use windows_sys::Win32::{
        Foundation::{GENERIC_WRITE, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, ReOpenFile},
    };
    let handle = unsafe {
        ReOpenFile(
            file.as_raw_handle(),
            GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            0,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        Err(allocation_error(io::Error::last_os_error()))
    } else {
        Ok(unsafe { File::from_raw_handle(handle) })
    }
}

#[cfg(not(windows))]
fn independent_sync_file(file: &File) -> io::Result<File> {
    file.try_clone()
}

impl FileSink {
    fn finish_pending(&mut self) -> io::Result<()> {
        if !std::mem::take(&mut self.pending) {
            return Ok(());
        }
        let worker = self.worker.as_ref().expect("a pending sync has a worker");
        let (elapsed, result) = worker.wait();
        self.work.completed += 1;
        self.work.total_ns = self.work.total_ns.saturating_add(elapsed);
        self.work.max_ns = self.work.max_ns.max(elapsed);
        result.map_err(allocation_error)
    }
}

impl Write for FileSink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.file.write(bytes).map_err(allocation_error)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush().map_err(allocation_error)
    }
}

impl SegmentSink for FileSink {
    fn sync_all(&mut self) -> io::Result<()> {
        self.finish_pending()?;
        self.file.sync_all().map_err(allocation_error)
    }

    fn start_sync(&mut self) -> io::Result<()> {
        if self.ordered_sync {
            let started = Instant::now();
            self.work.requested += 1;
            let result = self.file.sync_all().map_err(allocation_error);
            let elapsed = started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
            self.work.completed += 1;
            self.work.total_ns = self.work.total_ns.saturating_add(elapsed);
            self.work.max_ns = self.work.max_ns.max(elapsed);
            return result;
        }
        if !self.poll_sync()? {
            return Ok(());
        }
        let worker = match &mut self.worker {
            Some(worker) => worker,
            empty => {
                let file = self.sync_file.try_clone().map_err(allocation_error)?;
                let worker = DurabilityWorker::start(file).map_err(allocation_error)?;
                self.work.threads += 1;
                empty.insert(worker)
            }
        };
        worker.request();
        self.pending = true;
        self.work.requested += 1;
        Ok(())
    }

    fn poll_sync(&mut self) -> io::Result<bool> {
        if self.pending && self.worker.as_ref().is_some_and(DurabilityWorker::busy) {
            return Ok(false);
        }
        self.finish_pending()?;
        Ok(true)
    }

    fn background_sync_work(&self) -> Option<BackgroundSyncWork> {
        Some(BackgroundSyncWork {
            pending: self.pending,
            ..self.work
        })
    }
}

impl Drop for FileSink {
    fn drop(&mut self) {
        let _ = self.finish_pending();
        self.worker = None;
    }
}

pub trait SegmentBackend {
    type Sink: SegmentSink;
    fn create(&mut self, index: u32) -> io::Result<Self::Sink>;
    fn allocated_bytes(&self) -> usize {
        0
    }
}

pub struct FileBackend {
    directory: PathBuf,
    ordered_sync: bool,
}

impl FileBackend {
    pub fn new(directory: impl Into<PathBuf>) -> io::Result<Self> {
        let directory = directory.into();
        fs::create_dir_all(&directory).map_err(allocation_error)?;
        Ok(Self {
            directory,
            ordered_sync: false,
        })
    }

    pub fn ordered(directory: impl Into<PathBuf>) -> io::Result<Self> {
        let mut backend = Self::new(directory)?;
        backend.ordered_sync = true;
        Ok(backend)
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

impl SegmentBackend for FileBackend {
    type Sink = FileSink;
    fn allocated_bytes(&self) -> usize {
        self.directory.capacity()
    }

    fn create(&mut self, index: u32) -> io::Result<FileSink> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.directory.join(format!("spool.{index:04}.h4s")))
            .map_err(allocation_error)?;
        let sync_file = independent_sync_file(&file)?;
        Ok(FileSink {
            file,
            sync_file,
            worker: None,
            pending: false,
            work: BackgroundSyncWork::default(),
            ordered_sync: self.ordered_sync,
        })
    }
}

pub type SpoolWriteStartHook = fn(Option<(u32, u64)>);

#[cfg(test)]
#[derive(Default)]
struct SyncClock {
    fixed: Option<Arc<dyn Fn() -> Instant + Send + Sync>>,
}

#[cfg(not(test))]
#[derive(Default)]
struct SyncClock;

impl SyncClock {
    #[inline]
    fn now(&self) -> Instant {
        self.fixed().unwrap_or_else(Instant::now)
    }

    #[cfg(test)]
    fn fixed(&self) -> Option<Instant> {
        self.fixed.as_ref().map(|clock| clock())
    }

    #[cfg(not(test))]
    #[inline]
    fn fixed(&self) -> Option<Instant> {
        None
    }
}

pub struct SegmentedSpool<B: SegmentBackend = FileBackend> {
    backend: B,
    sink: Option<B::Sink>,
    state: SegmentState,
    next_segment: u32,
    clock: SyncClock,
    last_sync: Instant,
    last_error: Option<io::ErrorKind>,
    committed_segment: Option<u32>,
    committed_bytes: u64,
    current_segment: Option<u32>,
    current_bytes: u64,
    write_start_hook: Option<SpoolWriteStartHook>,
    commit_hook: Option<fn(u32, u64, bool)>,
    committed_frames: u64,
    closed_sync: BackgroundSyncWork,
}

impl<B: SegmentBackend> SegmentedSpool<B> {
    pub fn allocated_bytes(&self) -> usize {
        self.backend.allocated_bytes() + self.state.branches.capacity() * size_of::<u64>()
    }

    pub fn new(backend: B) -> Self {
        Self::with_next_segment_index(backend, 0)
    }

    pub fn with_next_segment_index(backend: B, next_segment: u32) -> Self {
        Self {
            backend,
            sink: None,
            state: SegmentState::default(),
            next_segment,
            clock: SyncClock::default(),
            last_sync: Instant::now(),
            last_error: None,
            committed_segment: None,
            committed_bytes: 0,
            current_segment: None,
            current_bytes: 0,
            write_start_hook: None,
            commit_hook: None,
            committed_frames: 0,
            closed_sync: BackgroundSyncWork::default(),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn next_segment_index(&self) -> u32 {
        self.next_segment
    }
    pub fn is_open(&self) -> bool {
        self.sink.is_some()
    }
    pub fn last_error(&self) -> Option<io::ErrorKind> {
        self.last_error
    }

    pub fn committed_watermark(&self) -> Option<(u32, u64)> {
        self.committed_segment
            .map(|segment| (segment, self.committed_bytes))
    }

    pub fn committed_frames(&self) -> u64 {
        self.committed_frames
    }

    pub fn start_sealed_segment(&mut self, records: Vec<SealedRecord>) -> io::Result<()> {
        self.start_sealed(records, true)
    }

    pub fn start_sealed_context(&mut self, records: Vec<SealedRecord>) -> io::Result<()> {
        self.start_sealed(records, false)
    }

    fn start_sealed(&mut self, records: Vec<SealedRecord>, frame: bool) -> io::Result<()> {
        if self.is_open()
            || records.is_empty()
            || frame
                && (records.len() < 2
                    || !records.last().is_some_and(|record| {
                        record.frame.is_some_and(|(_, _, keyframe)| keyframe)
                    }))
        {
            return Err(invalid_static(
                "segment start needs closed output and valid context",
            ));
        }
        let mut state = SegmentState::default();
        let context = records.len() - usize::from(frame);
        for record in &records[..context] {
            if record.frame.is_some() || matches!(record.bytes[RECORD_HEADER_BYTES], 4 | 7) {
                return Err(invalid_static("segment context contains frame or detach"));
            }
            record.accept(&mut state)?;
        }
        if frame {
            records.last().unwrap().accept(&mut state)?;
        }
        let index = self.next_segment;
        self.next_segment = index
            .checked_add(1)
            .ok_or_else(|| invalid_static("segment index exhausted"))?;
        let sink = match self.backend.create(index) {
            Ok(sink) => sink,
            Err(error) => return self.failed(error),
        };
        self.sink = Some(sink);
        self.current_segment = Some(index);
        self.current_bytes = 0;
        for record in records {
            if let Err(error) = self.write_flushed(&record.bytes) {
                return self.failed(error);
            }
        }
        if frame && let Err(error) = self.sync_current() {
            return self.failed(error);
        }
        self.state = state;
        self.last_error = None;
        Ok(())
    }

    pub fn append_sealed(&mut self, record: SealedRecord) -> io::Result<()> {
        if !self.is_open() {
            return Err(invalid_static(
                "no writable segment; restart with context and a keyframe",
            ));
        }
        let force_sync = record.bytes[RECORD_HEADER_BYTES] == 4
            || self.state.needs_keyframe && record.frame.is_some();
        record.accept(&mut self.state)?;
        self.append_bytes(record.bytes, force_sync)
    }

    pub fn set_commit_hook(&mut self, hook: fn(u32, u64, bool)) {
        self.commit_hook = Some(hook);
    }

    pub fn set_write_start_hook(&mut self, hook: SpoolWriteStartHook) {
        self.write_start_hook = Some(hook);
    }

    pub fn background_sync_work(&self) -> Option<BackgroundSyncWork> {
        self.sink.as_ref()?.background_sync_work()
    }

    pub fn background_sync_totals(&self) -> BackgroundSyncWork {
        let current = self.background_sync_work().unwrap_or_default();
        BackgroundSyncWork {
            requested: self.closed_sync.requested.saturating_add(current.requested),
            completed: self.closed_sync.completed.saturating_add(current.completed),
            total_ns: self.closed_sync.total_ns.saturating_add(current.total_ns),
            max_ns: self.closed_sync.max_ns.max(current.max_ns),
            threads: self.closed_sync.threads.saturating_add(current.threads),
            pending: current.pending,
        }
    }

    fn retire_sink(&mut self) {
        if let Some(sink) = self.sink.take() {
            let work = sink.background_sync_work().unwrap_or_default();
            self.closed_sync.requested = self.closed_sync.requested.saturating_add(work.requested);
            self.closed_sync.completed = self.closed_sync.completed.saturating_add(work.completed);
            self.closed_sync.total_ns = self.closed_sync.total_ns.saturating_add(work.total_ns);
            self.closed_sync.max_ns = self.closed_sync.max_ns.max(work.max_ns);
            self.closed_sync.threads = self.closed_sync.threads.saturating_add(work.threads);
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn start_segment(&mut self, context: &[Record], keyframe: &Frame) -> io::Result<()> {
        self.start_segment_with(
            context,
            keyframe.branch,
            keyframe.hour,
            keyframe.keyframe,
            || encode_frame_record(keyframe.encoded()?),
        )
    }

    pub fn start_encoded_segment(
        &mut self,
        context: &[Record],
        keyframe: EncodedFrame,
    ) -> io::Result<()> {
        self.start_segment_with(
            context,
            keyframe.branch(),
            keyframe.hour(),
            keyframe.keyframe(),
            || encode_frame_record(keyframe),
        )
    }

    fn start_segment_with(
        &mut self,
        context: &[Record],
        branch: u64,
        hour: i32,
        keyframe: bool,
        encode: impl FnOnce() -> io::Result<Vec<u8>>,
    ) -> io::Result<()> {
        if self.sink.is_some() {
            return Err(invalid_static(
                "close the current segment before starting another",
            ));
        }
        if context.is_empty() || !keyframe {
            return Err(invalid_static("segment start needs context and a keyframe"));
        }
        let mut state = SegmentState::default();
        let count = context
            .len()
            .checked_add(1)
            .ok_or_else(engine_allocation::oom)?;
        let mut encoded = engine_allocation::vec(count)?;
        for record in context {
            if matches!(
                record,
                Record::Frame(_)
                    | Record::Control(ControlRecord::Detach { .. } | ControlRecord::Partial { .. })
            ) {
                return Err(invalid_static("segment context contains frame or detach"));
            }
            state.accept(record)?;
            encoded.push(encode_record(record)?);
        }
        state.accept_frame(branch, hour, keyframe)?;
        encoded.push(encode()?);
        let index = self.next_segment;
        self.next_segment = index
            .checked_add(1)
            .ok_or_else(|| invalid_static("segment index exhausted"))?;
        let sink = match self.backend.create(index) {
            Ok(sink) => sink,
            Err(error) => return self.failed(error),
        };
        self.sink = Some(sink);
        self.current_segment = Some(index);
        self.current_bytes = 0;
        for bytes in encoded {
            if let Err(error) = self.write_flushed(&bytes) {
                return self.failed(error);
            }
        }
        if let Err(error) = self.sync_current() {
            return self.failed(error);
        }
        self.state = state;
        self.last_error = None;
        Ok(())
    }

    pub fn append(&mut self, record: &Record) -> io::Result<()> {
        if self.sink.is_none() {
            return Err(invalid_static(
                "no writable segment; restart with context and a keyframe",
            ));
        }
        let bytes = encode_record(record)?;
        self.state.accept(record)?;
        self.append_bytes(
            bytes,
            matches!(record, Record::Control(ControlRecord::Detach { .. })),
        )
    }

    pub fn append_encoded_frame(&mut self, frame: EncodedFrame) -> io::Result<()> {
        if self.sink.is_none() {
            return Err(invalid_static(
                "no writable segment; restart with context and a keyframe",
            ));
        }
        let (branch, hour, keyframe) = (frame.branch(), frame.hour(), frame.keyframe());
        let bytes = encode_frame_record(frame)?;
        self.state.accept_frame(branch, hour, keyframe)?;
        self.append_bytes(bytes, false)
    }

    fn append_bytes(&mut self, bytes: Vec<u8>, force_sync: bool) -> io::Result<()> {
        if let Some(sink) = &mut self.sink
            && let Err(error) = sink.poll_sync()
        {
            return self.failed(error);
        }
        if let Err(error) = self.write_flushed(&bytes) {
            return self.failed(error);
        }
        let synced = if force_sync {
            self.sync_current()
        } else {
            self.sync_if_due()
        };
        if let Err(error) = synced {
            return self.failed(error);
        }
        Ok(())
    }

    pub fn sync_if_due(&mut self) -> io::Result<()> {
        let result = (|| {
            if let Some(sink) = &mut self.sink
                && sink.poll_sync()?
                && self.clock.now().saturating_duration_since(self.last_sync) >= SYNC_INTERVAL
            {
                sink.flush()?;
                sink.start_sync()?;
                self.last_sync = self.clock.now();
            }
            Ok(())
        })();
        if let Err(error) = result {
            return self.failed(error);
        }
        Ok(())
    }

    pub fn finish_segment(&mut self) -> io::Result<()> {
        if self.sink.is_some()
            && let Err(error) = self.sync_current()
        {
            return self.failed(error);
        }
        self.retire_sink();
        self.state = SegmentState::default();
        Ok(())
    }

    fn write_flushed(&mut self, bytes: &[u8]) -> io::Result<()> {
        let preceding = self.committed_watermark();
        let sink = self
            .sink
            .as_mut()
            .ok_or_else(|| invalid_static("no writable segment"))?;
        if let Some(hook) = self.write_start_hook {
            hook(preceding);
        }
        if engine_allocation::capture_stopped() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        sink.write_all(bytes)?;
        sink.flush()?;
        if engine_allocation::capture_stopped() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        self.current_bytes = self.current_bytes.saturating_add(bytes.len() as u64);
        self.committed_segment = self.current_segment;
        self.committed_bytes = self.current_bytes;
        if bytes[RECORD_HEADER_BYTES] == 2 {
            self.committed_frames = self.committed_frames.saturating_add(1);
        }
        if let Some(hook) = self.commit_hook {
            hook(
                self.committed_segment.unwrap(),
                self.committed_bytes,
                bytes[RECORD_HEADER_BYTES] == 2,
            );
        }
        Ok(())
    }

    fn sync_current(&mut self) -> io::Result<()> {
        let sink = self
            .sink
            .as_mut()
            .ok_or_else(|| invalid_static("no writable segment"))?;
        sink.flush()?;
        sink.sync_all()?;
        self.last_sync = self.clock.now();
        Ok(())
    }

    fn failed<T>(&mut self, error: io::Error) -> io::Result<T> {
        self.retire_sink();
        self.state = SegmentState::default();
        self.last_error = Some(error.kind());
        Err(error)
    }
}


#[cfg(test)]
mod tests;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
