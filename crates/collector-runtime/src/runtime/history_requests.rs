use super::Processor;
use collector_spool::spooling::HistoryReply;
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, Write},
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, MOVEFILE_REPLACE_EXISTING,
    MOVEFILE_WRITE_THROUGH, MoveFileExW,
};

const POLL: Duration = Duration::from_millis(100);

#[derive(Default)]
struct Retry {
    next: Option<Instant>,
    failures: u32,
}
impl Retry {
    fn ready(&self, now: Instant) -> bool {
        self.next.is_none_or(|next| now >= next)
    }
    fn failed(&mut self, now: Instant) {
        self.failures = self.failures.saturating_add(1);
        let delay = POLL.saturating_mul(1 << self.failures.saturating_sub(1).min(9));
        self.next = Some(now + delay.min(Duration::from_secs(30)));
    }
}

#[derive(Default)]
struct RequestState {
    reply: Option<HistoryReply>,
    retry: Retry,
}
#[derive(Default)]
struct HistoryRequests {
    states: BTreeMap<PathBuf, RequestState>,
    loans: BTreeMap<String, String>,
    stopped: Option<Arc<AtomicBool>>,
}

enum Command {
    Begin(String),
    Retake(String, Option<File>),
}
enum Response {
    Began(io::Result<(HistoryReply, Option<File>)>),
    Retaken(Option<File>),
}
#[derive(Default)]
struct Mailbox {
    command: Option<Command>,
    response: Option<Response>,
    fence: Option<u64>,
    refresh_failed: bool,
}
pub(super) struct SpoolHistory(Arc<Mutex<Mailbox>>);
impl SpoolHistory {
    pub(super) fn poll<'a>(
        &self,
        processor: &mut Processor,
        frames: &collector_workers::frame_fence::Frames,
        owned: impl IntoIterator<Item = &'a std::sync::atomic::AtomicU64>,
    ) {
        self.poll_fenced_with(Some((frames, owned)), |text| {
            // SAFETY: The spool supplies the same session's fence and slots for the history
            // request.
            unsafe { processor.try_history_request(text) }
        });
        self.restore_with(|campaign, file|
        // SAFETY: The originating spool has cleared this request through its capture and writer
        // fence.
        unsafe { processor.exchange_history_lease(campaign, file) });
    }

    pub(super) fn take_refresh_failure(&self) -> bool {
        self.0
            .try_lock()
            .is_ok_and(|mut mailbox| std::mem::take(&mut mailbox.refresh_failed))
    }

    #[cfg(test)]
    fn poll_with(
        &self,
        begin: impl FnMut(&str) -> io::Result<Option<(HistoryReply, Option<File>)>>,
    ) {
        self.poll_fenced_with(
            None::<(
                &collector_workers::frame_fence::Frames,
                [&std::sync::atomic::AtomicU64; 0],
            )>,
            begin,
        );
    }

    fn poll_fenced_with<'a>(
        &self,
        frames: Option<(
            &collector_workers::frame_fence::Frames,
            impl IntoIterator<Item = &'a std::sync::atomic::AtomicU64>,
        )>,
        mut begin: impl FnMut(&str) -> io::Result<Option<(HistoryReply, Option<File>)>>,
    ) {
        let Ok(mut mailbox) = self.0.try_lock() else {
            return;
        };
        let Mailbox { command, fence, .. } = &mut *mailbox;
        let Some(Command::Begin(text)) = command.as_ref() else {
            return;
        };
        if let Some((frames, owned)) = frames
            && matches!(text.split_whitespace().nth(1), Some("close" | "release"))
            &&
            // SAFETY: The originating session supplies its own frame slots and fence.
            unsafe { frames.pending(*fence.get_or_insert_with(|| frames.snapshot()), owned) }
        {
            return;
        }
        *fence = None;
        let result =
            begin(text).and_then(|reply| reply.ok_or_else(|| io::ErrorKind::WouldBlock.into()));
        mailbox.command = None;
        mailbox.response = Some(Response::Began(result));
    }

    fn restore_with(&self, restore: impl FnOnce(&str, Option<File>) -> Option<File>) {
        let Ok(mut mailbox) = self.0.try_lock() else {
            return;
        };
        if !matches!(mailbox.command, Some(Command::Retake(..))) {
            return;
        }
        let Some(Command::Retake(campaign, file)) = mailbox.command.take() else {
            unreachable!()
        };
        mailbox.response = Some(Response::Retaken(restore(&campaign, file)));
    }
}

struct Remote {
    mailbox: Arc<Mutex<Mailbox>>,
    stop: Arc<AtomicBool>,
    replay_directory: PathBuf,
}
impl Remote {
    fn exchange(&self, command: Command) -> io::Result<Response> {
        if self.stop.load(Ordering::Acquire) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        {
            let mut mailbox = self.mailbox.lock().unwrap();
            assert!(mailbox.command.is_none() && mailbox.response.is_none());
            mailbox.command = Some(command);
        }
        loop {
            if self.stop.load(Ordering::Acquire) {
                return Err(io::ErrorKind::Interrupted.into());
            }
            if let Some(response) = self.mailbox.lock().unwrap().response.take() {
                return Ok(response);
            }
            std::thread::park_timeout(Duration::from_millis(10));
        }
    }
    fn begin(&self, text: &str) -> io::Result<HistoryReply> {
        let Response::Began(result) = self.exchange(Command::Begin(text.to_owned()))? else {
            unreachable!()
        };
        let (reply, lease) = result?;
        drop(lease);
        Ok(reply)
    }
    fn retake(&self, campaign: &str) -> io::Result<()> {
        use replay_files::recording_files;
        let path = match recording_files::unsaved_recording(
            &self.replay_directory,
            campaign.as_bytes(),
        )? {
            Some(path) => Some(path),
            None => recording_files::recording_of(&self.replay_directory, campaign.as_bytes())?,
        };
        let lease = path
            .map(|path| collector_spool::history::lease(&path))
            .transpose()?;
        let Response::Retaken(previous) =
            self.exchange(Command::Retake(campaign.to_owned(), lease))?
        else {
            unreachable!()
        };
        drop(previous);
        Ok(())
    }
}

pub(super) struct HistoryWorker {
    stop: Arc<AtomicBool>,
    released: Arc<AtomicBool>,
    thread: Option<JoinHandle<io::Result<()>>>,
    thread_id: u32,
}
impl HistoryWorker {
    pub(super) fn start(
        session: PathBuf,
        replay_directory: PathBuf,
        refresh: Option<fn()>,
    ) -> io::Result<(Self, SpoolHistory)> {
        let mailbox = Arc::new(Mutex::new(Mailbox::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let released = Arc::new(AtomicBool::new(false));
        let id = Arc::new(AtomicU32::new(0));
        let worker_id = Arc::clone(&id);
        let worker_released = Arc::clone(&released);
        let remote = Remote {
            mailbox: Arc::clone(&mailbox),
            stop: Arc::clone(&stop),
            replay_directory,
        };
        let thread = std::thread::Builder::new()
            .name("hoi4-history".into())
            .spawn(move || {
                collector_workers::private_heap::off_hold_path();
                let _phase =
                    collector_memory::terminal::enter(collector_memory::terminal::Phase::History);
                // SAFETY: The owning worker adjusts only its own thread's priority.
                unsafe { collector_workers::priority::OwnPriority::new().follow(false) };
                worker_id.store(
                    unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() },
                    Ordering::Release,
                );
                let mut requests = HistoryRequests {
                    stopped: Some(Arc::clone(&remote.stop)),
                    ..HistoryRequests::default()
                };
                let mut refresh_requests = super::RefreshRequests::default();
                while !remote.stop.load(Ordering::Acquire) {
                    if let Err(error) = requests.poll_with(
                        &session,
                        &mut (),
                        Instant::now,
                        |_, text| remote.begin(text),
                        |_, campaign| remote.retake(campaign),
                    ) {
                        collector_memory::terminal::observe(&error);
                    }
                    if let Some(accept) = refresh {
                        let _status = collector_memory::terminal::enter(
                            collector_memory::terminal::Phase::Status,
                        );
                        if let Err(error) = refresh_requests.poll(&session, accept) {
                            collector_memory::terminal::observe(&error);
                            remote.mailbox.lock().unwrap().refresh_failed = true;
                        }
                    }
                    std::thread::park_timeout(POLL);
                }
                let (command, response) = {
                    let mut mailbox = remote.mailbox.lock().unwrap();
                    (mailbox.command.take(), mailbox.response.take())
                };
                drop((command, response));
                if worker_released.load(Ordering::Acquire) {
                    detached(&session)
                } else {
                    Ok(())
                }
            })?;
        while id.load(Ordering::Acquire) == 0 && !thread.is_finished() {
            std::thread::yield_now();
        }
        Ok((
            Self {
                stop,
                released,
                thread: Some(thread),
                thread_id: id.load(Ordering::Acquire),
            },
            SpoolHistory(mailbox),
        ))
    }
    pub(super) fn thread_id(&self) -> u32 {
        self.thread_id
    }
    pub(super) fn finish(&mut self) -> io::Result<()> {
        self.released.store(true, Ordering::Release);
        self.join()
    }
    fn join(&mut self) -> io::Result<()> {
        self.stop.store(true, Ordering::Release);
        match self.thread.take() {
            Some(thread) => {
                thread.thread().unpark();
                thread
                    .join()
                    .map_err(|_| io::Error::other("history worker panicked"))?
            }
            None => Ok(()),
        }
    }
}
impl Drop for HistoryWorker {
    fn drop(&mut self) {
        let _ = self.join();
    }
}

fn temporary_path(path: &Path) -> PathBuf {
    path.with_extension(format!(
        "{}.tmp",
        path.extension().unwrap_or_default().to_string_lossy()
    ))
}

fn publish(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = temporary_path(path);
    let mut file = File::create(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    let from: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn retry_marker(
    mut write: impl FnMut() -> io::Result<()>,
    mut pause: impl FnMut(Duration),
) -> io::Result<()> {
    for delay in [10, 20, 0] {
        match write() {
            Err(_) if delay != 0 => pause(Duration::from_millis(delay)),
            result => return result,
        }
    }
    unreachable!()
}
fn detached(session: &Path) -> io::Result<()> {
    let directory = session.join("history-requests");
    retry_marker(
        || {
            fs::create_dir_all(&directory)?;
            publish(&directory.join("detached"), b"history leases released")
        },
        std::thread::sleep,
    )
    .map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("{}: {error}", directory.join("detached").display()),
        )
    })
}

fn archive(session: &Path, path: &Path) -> io::Result<()> {
    let completed = session.join("history-completed");
    fs::create_dir_all(&completed)?;
    for extension in [
        "ack",
        "returned",
        "return",
        "waiting",
        "ack.tmp",
        "returned.tmp",
        "return.tmp",
        "waiting.tmp",
        "request.tmp",
        "request",
    ] {
        let source = path.with_extension(extension);
        match fs::rename(&source, completed.join(source.file_name().unwrap())) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            result => result?,
        }
    }
    Ok(())
}

fn completed(session: &Path, path: &Path, extension: &str) -> PathBuf {
    session
        .join("history-completed")
        .join(path.with_extension(extension).file_name().unwrap())
}

impl HistoryRequests {
    fn poll_with<P>(
        &mut self,
        session: &Path,
        processor: &mut P,
        mut now: impl FnMut() -> Instant,
        mut begin: impl FnMut(&mut P, &str) -> io::Result<HistoryReply>,
        mut retake: impl FnMut(&mut P, &str) -> io::Result<()>,
    ) -> io::Result<()> {
        let directory = session.join("history-requests");
        let entries = match fs::read_dir(&directory) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            entries => entries?,
        };
        let mut paths = Vec::new();
        let mut failure = None;
        for entry in entries {
            match entry {
                Ok(entry)
                    if entry
                        .path()
                        .extension()
                        .is_some_and(|extension| extension == "request") =>
                {
                    paths.push(entry.path())
                }
                Ok(entry) => {
                    let path = entry.path();
                    if let Some((token, _)) = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .and_then(|name| name.split_once('.'))
                    {
                        let request = directory.join(token).with_extension("request");
                        if completed(session, &request, "returned").exists()
                            || completed(session, &request, "request").exists()
                        {
                            paths.push(request);
                        }
                    }
                }
                Err(error) => {
                    failure.get_or_insert(error);
                }
            }
        }
        paths.sort();
        paths.dedup();
        for path in paths {
            if self
                .stopped
                .as_ref()
                .is_some_and(|stopped| stopped.load(Ordering::Acquire))
            {
                break;
            }
            let state = self.states.entry(path.clone()).or_default();
            if !state.retry.ready(now()) {
                continue;
            }
            let result = Self::process(
                session,
                &path,
                state,
                &mut self.loans,
                processor,
                &mut begin,
                &mut retake,
            );
            match result {
                Ok(true) => {
                    self.states.remove(&path);
                }
                Ok(false) => {
                    state.retry = Retry::default();
                }
                Err(error) => {
                    state.retry.failed(now());
                    let _ = publish(
                        &path.with_extension("waiting"),
                        error.to_string().as_bytes(),
                    );
                    failure.get_or_insert(error);
                }
            }
        }
        failure.map_or(Ok(()), Err)
    }

    fn process<P>(
        session: &Path,
        path: &Path,
        state: &mut RequestState,
        loans: &mut BTreeMap<String, String>,
        processor: &mut P,
        begin: &mut impl FnMut(&mut P, &str) -> io::Result<HistoryReply>,
        retake: &mut impl FnMut(&mut P, &str) -> io::Result<()>,
    ) -> io::Result<bool> {
        if path.with_extension("returned").exists()
            || completed(session, path, "returned").exists()
            || (completed(session, path, "request").exists()
                && completed(session, path, "ack").exists())
        {
            archive(session, path)?;
            return Ok(true);
        }
        let mut file = fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .open(path)?;
        let mut text = String::new();
        std::io::Read::read_to_string(&mut file, &mut text)?;
        drop(file);
        let mut words = text.split_whitespace();
        let (Some(token), Some(action), Some(campaign)) =
            (words.next(), words.next(), words.next())
        else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid history request",
            ));
        };
        if path.file_stem().is_none_or(|stem| stem != token) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "history token differs from journal",
            ));
        }
        let campaign = collector_spool::spooling::campaign_name(campaign.as_bytes())?;
        let loan = matches!(action, "lend" | "release");
        if path.with_extension("return").exists() {
            if loans.get(&campaign).is_some_and(|owner| owner == token) {
                retake(processor, &campaign)?;
                loans.remove(&campaign);
            }
            publish(
                &path.with_extension("returned"),
                format!("{token} ok").as_bytes(),
            )?;
            archive(session, path)?;
            return Ok(true);
        }
        if path.with_extension("ack").exists() || completed(session, path, "ack").exists() {
            if !loan {
                archive(session, path)?;
            }
            return Ok(!loan);
        }
        if loan && loans.contains_key(&campaign) && state.reply.is_none() {
            return Ok(false);
        }
        if state.reply.is_none() {
            let reply = begin(processor, &text)?;
            if loan && matches!(&reply, HistoryReply::Ready(ack) if ack == &format!("{token} ok")) {
                loans.insert(campaign.clone(), token.to_owned());
            }
            state.reply = Some(reply);
        }
        let ack = match state.reply.as_ref().unwrap() {
            HistoryReply::Ready(ack) => ack.clone(),
            HistoryReply::Closing { token, receipt } => match receipt.result() {
                None => return Ok(false),
                Some(Ok(())) => format!("{token} ok"),
                Some(Err(_)) => format!("{token} refused its segment could not be closed"),
            },
        };
        publish(&path.with_extension("ack"), ack.as_bytes())?;
        if !loan {
            archive(session, path)?;
        }
        Ok(!loan)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use collector_spool::spooling::HistoryReply;
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    const CAMPAIGN: &str = "21b7cc84-3028-4f65-8e0d-adf2f37ca1a6";
    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn root() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "hoi4-history-journal-{}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("history-requests")).unwrap();
        root
    }

    fn request(root: &Path, token: &str, action: &str) -> PathBuf {
        let path = root
            .join("history-requests")
            .join(format!("{token}.request"));
        publish(&path, format!("{token} {action} {CAMPAIGN}").as_bytes()).unwrap();
        path
    }

    fn archived(root: &Path, path: &Path, extension: &str) -> PathBuf {
        root.join("history-completed")
            .join(path.with_extension(extension).file_name().unwrap())
    }

    fn poll_processor(
        requests: &mut HistoryRequests,
        root: &Path,
        output: &Path,
        processor: &mut Processor,
        now: Instant,
    ) {
        requests
            .poll_with(
                root,
                processor,
                || now,
                |processor, text| {
                    let (reply, lease) =
                    // SAFETY: The spool supplies the same session's fence and slots for the
                    // history request.
                    unsafe { processor.try_history_request(text) }?.unwrap();
                    drop(lease);
                    Ok(reply)
                },
                |processor, campaign| {
                    let path = match replay_files::recording_files::unsaved_recording(
                        output,
                        campaign.as_bytes(),
                    )? {
                        Some(path) => Some(path),
                        None => replay_files::recording_files::recording_of(output, campaign.as_bytes())?,
                    };
                    let lease = path
                        .map(|path| collector_spool::history::lease(&path))
                        .transpose()?;
                    drop(
                        // SAFETY: The originating spool has cleared this request through its capture
                        // and writer fence.
                        unsafe { processor.exchange_history_lease(campaign, lease) },
                    );
                    Ok(())
                },
            )
            .unwrap_or_else(|error| assert!(error.to_string().contains("os error 32"), "{error}"));
    }

    struct Model {
        lent: usize,
        retaken: usize,
        refuse_retake: bool,
        elapsed: u64,
        clock: Instant,
    }
    impl Default for Model {
        fn default() -> Self {
            Self {
                lent: 0,
                retaken: 0,
                refuse_retake: false,
                elapsed: 0,
                clock: Instant::now(),
            }
        }
    }

    fn poll(requests: &mut HistoryRequests, root: &Path, model: &mut Model) -> io::Result<()> {
        model.elapsed += 1;
        let now = model.clock + Duration::from_secs(model.elapsed);
        requests.poll_with(
            root,
            model,
            || now,
            |model, text| {
                model.lent += 1;
                Ok(HistoryReply::Ready(format!(
                    "{} ok",
                    text.split_whitespace().next().unwrap()
                )))
            },
            |model, _| {
                model.retaken += 1;
                if model.refuse_retake {
                    Err(io::Error::from_raw_os_error(32))
                } else {
                    Ok(())
                }
            },
        )
    }

    #[test]
    fn cancelled_unprocessed_lend_is_returned_without_lending() {
        let root = root();
        let path = request(&root, "0001", "lend");
        publish(&path.with_extension("return"), b"0001").unwrap();
        let mut requests = HistoryRequests::default();
        let mut model = Model::default();
        poll(&mut requests, &root, &mut model).unwrap();
        assert_eq!((model.lent, model.retaken), (0, 0));
        assert!(!path.exists());
        assert!(archived(&root, &path, "request").exists());
        assert_eq!(
            fs::read_to_string(archived(&root, &path, "returned")).unwrap(),
            "0001 ok"
        );
        poll(&mut requests, &root, &mut model).unwrap();
        assert_eq!((model.lent, model.retaken), (0, 0));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lost_lend_ack_and_failed_retake_cannot_be_overwritten_by_the_next_loan() {
        let root = root();
        let old = request(&root, "0001", "lend");
        fs::create_dir(old.with_extension("ack.tmp")).unwrap();
        let mut requests = HistoryRequests::default();
        let mut model = Model::default();
        assert!(poll(&mut requests, &root, &mut model).is_err());
        assert_eq!(model.lent, 1);
        fs::remove_dir(old.with_extension("ack.tmp")).unwrap();
        poll(&mut requests, &root, &mut model).unwrap();
        assert_eq!(model.lent, 1, "an ack retry must not lend twice");
        publish(&old.with_extension("return"), b"0001").unwrap();
        let new = request(&root, "0002", "lend");
        model.refuse_retake = true;
        for _ in 0..3 {
            assert!(poll(&mut requests, &root, &mut model).is_err());
            assert_eq!(model.lent, 1);
            assert!(!archived(&root, &old, "returned").exists());
            assert!(!new.with_extension("ack").exists());
        }
        assert!(
            fs::read_to_string(old.with_extension("waiting"))
                .unwrap()
                .contains("os error 32")
        );
        model.refuse_retake = false;
        poll(&mut requests, &root, &mut model).unwrap();
        assert_eq!(model.lent, 2);
        assert!(archived(&root, &old, "returned").exists());
        assert!(new.with_extension("ack").exists());
        publish(&new.with_extension("return"), b"0002").unwrap();
        poll(&mut requests, &root, &mut model).unwrap();
        assert_eq!(model.retaken, 5);
        poll(&mut requests, &root, &mut model).unwrap();
        assert_eq!(model.retaken, 5);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_out_of_order_loan_cannot_release_or_retake_another_owners_history() {
        let root = root();
        let active = request(&root, "0002", "lend");
        let mut requests = HistoryRequests::default();
        let mut model = Model::default();
        poll(&mut requests, &root, &mut model).unwrap();
        assert_eq!(model.lent, 1);
        let late = request(&root, "0001", "lend");
        poll(&mut requests, &root, &mut model).unwrap();
        assert!(!late.with_extension("ack").exists());
        publish(&late.with_extension("return"), b"0001").unwrap();
        poll(&mut requests, &root, &mut model).unwrap();
        assert_eq!((model.lent, model.retaken), (1, 0));
        assert!(archived(&root, &late, "returned").exists());
        publish(&active.with_extension("return"), b"0002").unwrap();
        poll(&mut requests, &root, &mut model).unwrap();
        assert_eq!((model.lent, model.retaken), (1, 1));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_delayed_close_is_acknowledged_once_only_after_its_durable_receipt() {
        let root = root();
        let path = request(&root, "0001", "close");
        let receipt = collector_spool::spool_writer::CloseReceipt::pending();
        let mut requests = HistoryRequests::default();
        let mut began = 0;
        for finished in [false, false, true, true] {
            if finished {
                // SAFETY: This is the close result from its originating writer, or an isolated
                // receipt owned by the fixture.
                unsafe { receipt.complete(Ok(())) };
            }
            requests
                .poll_with(
                    &root,
                    &mut began,
                    Instant::now,
                    |began, _| {
                        *began += 1;
                        Ok(HistoryReply::Closing {
                            token: "0001".to_owned(),
                            receipt: receipt.clone(),
                        })
                    },
                    |_, _| panic!("close does not lend"),
                )
                .unwrap();
            assert_eq!(archived(&root, &path, "ack").exists(), finished);
        }
        assert_eq!(began, 1);
        assert!(!path.exists());
        assert!(archived(&root, &path, "request").exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn blocked_journal_worker_does_not_block_frame_recycling() {
        use std::sync::mpsc;
        let root = root();
        let path = request(&root, "0001", "lend");
        let mailbox = Arc::new(Mutex::new(Mailbox::default()));
        let spool = SpoolHistory(Arc::clone(&mailbox));
        let remote = Remote {
            mailbox: Arc::clone(&mailbox),
            stop: Arc::new(AtomicBool::new(false)),
            replay_directory: root.clone(),
        };
        let (blocked, entered) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        std::thread::scope(|scope| {
            let release = release;
            let journal_root = &root;
            let worker = scope.spawn(move || {
                HistoryRequests::default()
                    .poll_with(
                        journal_root,
                        &mut (),
                        Instant::now,
                        |_, text| {
                            let reply = remote.begin(text)?;
                            blocked.send(()).unwrap();
                            let _ = resume.recv();
                            Ok(reply)
                        },
                        |_, _| panic!("not returning"),
                    )
                    .unwrap();
            });
            let mut transitions = 0;
            while entered.try_recv().is_err() {
                spool.poll_with(|_| {
                    transitions += 1;
                    Ok(Some((HistoryReply::Ready("0001 ok".into()), None)))
                });
                std::thread::yield_now();
            }
            assert_eq!(transitions, 1);
            assert!(!path.with_extension("ack").exists());
            let shared =
            // SAFETY: The caller owns the session or isolated fixture state required by this
            // constructor.
            unsafe { collector_capture::sampling::Shared::new(0, 0, 1) };
            let held: Vec<_> = (0..collector_capture::sampling::BUFFERS)
                .map(|_|
                    // SAFETY: This consumer owns the queue removal for the named pool.
                    unsafe {  shared.frames() .pop() }.unwrap())
                .collect();
            assert_eq!(
                // SAFETY: This code uses the frame pool belonging to its own recording or fixture.
                unsafe { shared.frames() }.len(),
                0
            );
            for &index in &held {
                spool.poll_with(|_| panic!("worker is blocked outside the handoff"));
                // SAFETY: This is the last owner of the installed slot and all other uses have
                // ended.
                unsafe { shared.recycle_frame(index) };
                let mut wait = u64::MAX;
                assert_eq!(
                    // SAFETY: This is the admitted hour or isolated fixture taking exclusive ownership
                    // of an installed slot.
                    unsafe { shared.take_frame(&mut wait) },
                    Some(index)
                );
                assert_eq!(wait, 0);
            }
            for index in held {
                // SAFETY: This is the last owner of the installed slot and all other uses have
                // ended.
                unsafe { shared.recycle_frame(index) };
            }
            assert_eq!(
                // SAFETY: This code uses the frame pool belonging to its own recording or fixture.
                unsafe { shared.frames() }.len(),
                collector_capture::sampling::BUFFERS
            );
            release.send(()).unwrap();
            worker.join().unwrap();
        });
        assert!(path.with_extension("ack").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn contended_or_full_handoffs_defer_without_waiting_or_overwriting() {
        let mailbox = Arc::new(Mutex::new(Mailbox::default()));
        let spool = SpoolHistory(Arc::clone(&mailbox));
        let guard = mailbox.lock().unwrap();
        spool.poll_with(|_| panic!("contended handoff must be skipped"));
        spool.restore_with(|_, _| panic!("contended handoff must be skipped"));
        drop(guard);
        mailbox.lock().unwrap().command = Some(Command::Begin("0001 close campaign".into()));
        spool.poll_with(|_| Ok(None));
        assert!(mailbox.lock().unwrap().command.is_none());
        let response = mailbox.lock().unwrap().response.take().unwrap();
        assert!(
            matches!(response, Response::Began(Err(error)) if error.kind() == io::ErrorKind::WouldBlock)
        );
        mailbox.lock().unwrap().command = Some(Command::Begin("0001 close campaign".into()));
        spool.poll_with(|_| Ok(Some((HistoryReply::Ready("0001 ok".into()), None))));
        for _ in 0..4 {
            spool.poll_with(|_| panic!("full reply slot must not be overwritten"));
        }
        assert!(
            matches!(&mailbox.lock().unwrap().response, Some(Response::Began(Ok((HistoryReply::Ready(ack), None)))) if ack == "0001 ok")
        );
    }

    #[test]
    fn one_unreadable_token_does_not_block_another_campaign_and_is_backed_off() {
        let root = root();
        fs::create_dir(root.join("history-requests/0001.request")).unwrap();
        let other = root.join("history-requests/0002.request");
        publish(&other, b"0002 close 31b7cc84-3028-4f65-8e0d-adf2f37ca1a6").unwrap();
        let mut requests = HistoryRequests::default();
        let mut model = Model::default();
        let now = Instant::now();
        let mut begin = |model: &mut Model, text: &str| {
            model.lent += 1;
            Ok(HistoryReply::Ready(format!(
                "{} ok",
                text.split_whitespace().next().unwrap()
            )))
        };
        assert!(
            requests
                .poll_with(&root, &mut model, || now, &mut begin, |_, _| unreachable!())
                .is_err()
        );
        assert_eq!(model.lent, 1);
        assert!(archived(&root, &other, "ack").exists());
        assert!(
            requests
                .poll_with(&root, &mut model, || now, &mut begin, |_, _| unreachable!())
                .is_ok()
        );
        assert!(
            requests
                .poll_with(
                    &root,
                    &mut model,
                    || now + POLL,
                    &mut begin,
                    |_, _| unreachable!()
                )
                .is_err()
        );
        assert_eq!(model.lent, 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retake_retries_use_per_token_exponential_backoff() {
        let root = root();
        let path = request(&root, "0001", "lend");
        let mut requests = HistoryRequests::default();
        let mut attempts = 0;
        let start = Instant::now();
        requests
            .poll_with(
                &root,
                &mut attempts,
                || start,
                |_, _| Ok(HistoryReply::Ready("0001 ok".into())),
                |_, _| unreachable!(),
            )
            .unwrap();
        publish(&path.with_extension("return"), b"0001").unwrap();
        for (millis, expected) in [(0, 1), (99, 1), (100, 2), (299, 2), (300, 3), (699, 3)] {
            let _ = requests.poll_with(
                &root,
                &mut attempts,
                || start + Duration::from_millis(millis),
                |_, _| panic!("loan must not repeat"),
                |attempts, _| {
                    *attempts += 1;
                    Err(io::Error::from_raw_os_error(32))
                },
            );
            assert_eq!(attempts, expected);
        }
        requests
            .poll_with(
                &root,
                &mut attempts,
                || start + Duration::from_millis(700),
                |_, _| unreachable!(),
                |attempts, _| {
                    *attempts += 1;
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(attempts, 4);
        assert!(archived(&root, &path, "returned").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn completed_commands_leave_the_polled_directory_and_partial_archives_resume() {
        let root = root();
        let path = request(&root, "0001", "close");
        fs::create_dir_all(archived(&root, &path, "request")).unwrap();
        let mut requests = HistoryRequests::default();
        let mut model = Model::default();
        assert!(poll(&mut requests, &root, &mut model).is_err());
        assert!(archived(&root, &path, "ack").exists());
        fs::remove_dir(archived(&root, &path, "request")).unwrap();
        poll(&mut requests, &root, &mut model).unwrap();
        assert_eq!(
            model.lent, 1,
            "archival must not replay an acknowledged command"
        );
        for number in 2..=32 {
            request(&root, &format!("{number:04}"), "close");
            poll(&mut requests, &root, &mut model).unwrap();
        }
        assert_eq!(
            fs::read_dir(root.join("history-requests")).unwrap().count(),
            0
        );
        assert!(requests.states.is_empty());
        for extension in ["request", "ack"] {
            assert_eq!(
                fs::read_dir(root.join("history-completed"))
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .filter(|path| path.extension().is_some_and(|found| found == extension))
                    .count(),
                32
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn release_marker_retries_are_bounded_and_preserve_the_actual_error() {
        let mut attempts = 0;
        let mut delays = Vec::new();
        let error = retry_marker(
            || {
                attempts += 1;
                Err(io::Error::from_raw_os_error(5))
            },
            |delay| delays.push(delay),
        )
        .unwrap_err();
        assert_eq!(attempts, 3);
        assert_eq!(error.raw_os_error(), Some(5));
        assert_eq!(
            delays,
            [Duration::from_millis(10), Duration::from_millis(20)]
        );
        let mut attempts = 0;
        retry_marker(
            || {
                attempts += 1;
                if attempts < 3 {
                    Err(io::Error::from_raw_os_error(5))
                } else {
                    Ok(())
                }
            },
            |_| {},
        )
        .unwrap();
        assert_eq!(attempts, 3);
    }

    #[test]
    fn shutdown_cancels_a_fenced_close_before_releasing_history() {
        let root = root();
        request(&root, "0001", "close");
        let (mut worker, handoff) = HistoryWorker::start(root.clone(), root.clone(), None).unwrap();
        while handoff.0.lock().unwrap().command.is_none() {
            assert!(!worker.thread.as_ref().unwrap().is_finished());
            std::thread::yield_now();
        }
        let frames = collector_workers::frame_fence::Frames::default();
        let acquiring =
        // SAFETY: The admitted capture or its fixture owns this frame's observer sequencing.
        unsafe { frames.capture() };
        handoff.poll_fenced_with(Some((&frames, std::iter::empty())), |_| {
            panic!("capture has not drained")
        });
        assert!(handoff.0.lock().unwrap().fence.is_some());
        worker.finish().unwrap();
        assert!(root.join("history-requests/detached").exists());
        assert!(handoff.0.lock().unwrap().command.is_none());
        assert!(handoff.0.lock().unwrap().response.is_none());
        drop(acquiring);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn detach_releases_a_pending_retake_even_when_its_marker_cannot_be_written() {
        let root = root();
        let recording = root.join("history");
        fs::write(&recording, b"synthetic history").unwrap();
        let lease = collector_spool::history::lease(&recording).unwrap();
        let (mut worker, handoff) = HistoryWorker::start(root.clone(), root.clone(), None).unwrap();
        handoff.0.lock().unwrap().command = Some(Command::Retake(CAMPAIGN.to_owned(), Some(lease)));
        drop(handoff);
        assert!(
            fs::OpenOptions::new()
                .write(true)
                .share_mode(0)
                .open(&recording)
                .is_err()
        );
        fs::create_dir(temporary_path(&root.join("history-requests/detached"))).unwrap();
        let error = worker.finish().unwrap_err();
        assert!(error.to_string().contains("detached"));
        assert!(!root.join("history-requests/detached").exists());
        drop(
            fs::OpenOptions::new()
                .write(true)
                .share_mode(0)
                .open(&recording)
                .unwrap(),
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_worker_accepts_refresh_requests_and_reports_their_failures() {
        use super::super::REFRESH_REQUESTS;
        let root = root();
        let requests = root.join(REFRESH_REQUESTS);
        fs::write(&requests, "not a folder").unwrap();
        static ACCEPTED: AtomicU32 = AtomicU32::new(0);
        let (mut worker, handoff) = HistoryWorker::start(
            root.clone(),
            root.clone(),
            Some(|| _ = ACCEPTED.fetch_add(1, Ordering::SeqCst)),
        )
        .unwrap();
        let started = Instant::now();
        while !handoff.take_refresh_failure() {
            assert!(started.elapsed() < collector_test_support::TEST_HANG_GUARD);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(ACCEPTED.load(Ordering::SeqCst), 0);
        fs::remove_file(&requests).unwrap();
        fs::create_dir(&requests).unwrap();
        let request = requests.join("token-9.request");
        fs::write(requests.join("token-9.tmp"), "token-9").unwrap();
        fs::rename(requests.join("token-9.tmp"), &request).unwrap();
        let started = Instant::now();
        while request.exists() {
            assert!(started.elapsed() < collector_test_support::TEST_HANG_GUARD);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(ACCEPTED.load(Ordering::SeqCst), 1);
        assert_eq!(
            fs::read_to_string(requests.join("token-9.ack")).unwrap(),
            "token-9"
        );
        worker.finish().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_late_return_publication_after_archival_is_retired_without_repeating_the_loan() {
        let root = root();
        let path = request(&root, "0001", "lend");
        let mut requests = HistoryRequests::default();
        let mut model = Model::default();
        poll(&mut requests, &root, &mut model).unwrap();
        publish(&path.with_extension("return"), b"0001").unwrap();
        poll(&mut requests, &root, &mut model).unwrap();
        assert!(archived(&root, &path, "returned").exists());
        publish(&path.with_extension("return"), b"0001").unwrap();
        poll(&mut requests, &root, &mut model).unwrap();
        assert_eq!((model.lent, model.retaken), (1, 1));
        assert_eq!(
            fs::read_dir(root.join("history-requests")).unwrap().count(),
            0
        );
        fs::remove_dir_all(root).unwrap();
    }

    fn session(id: u64) -> replay_data::identity::SessionStart {
        use replay_data::identity::{
            CaptureMode, Identity, SampleBoundary, SessionStart, Verification,
        };
        SessionStart {
            id,
            process_id: 100,
            process_start_time: id * 1000,
            qpc_frequency: 10_000_000,
            sample_boundary: SampleBoundary::EndOfHour,
            capture_mode: CaptureMode::Full,
            identity: Identity {
                game_version: "fixture".into(),
                exe_hash: [1; 32],
                exe_size: 1,
                vanilla_hash: [2; 32],
                dlc: Vec::new(),
                mods: Vec::new(),
                coverage_policy: 1,
                language: "english".into(),
                recorder_version: "fixture".into(),
                collector_hash: [3; 32],
                verification: Verification::Unchecked,
            },
        }
    }

    #[test]
    fn close_waits_for_both_queued_a_frames_when_b_is_loaded_and_paused() {
        use collector_spool::history::PreparedCampaigns;
        use collector_spool::spooling::campaign_frame;
        let root = root();
        let live = root.join("sessions/1");
        let frames = collector_workers::frame_fence::Frames::default();
        let owned = [const { std::sync::atomic::AtomicU64::new(0) }; 3];
        let mailbox = Arc::new(Mutex::new(Mailbox::default()));
        let spool = SpoolHistory(Arc::clone(&mailbox));
        let mut processor =
            // SAFETY: The caller owns the session or isolated fixture state required by this
            // constructor.
            unsafe { Processor::new(&live, PreparedCampaigns::default(), session(1)) }.unwrap();
        for index in 0..2 {
            let capture =
            // SAFETY: The admitted capture or its fixture owns this frame's observer sequencing.
            unsafe { frames.capture() };
            // SAFETY: This frame or slot belongs to the current admitted capture or the sole spool
            // consuming it.
            unsafe { capture.frame(&owned[index]) };
        }
        // SAFETY: This frame or slot belongs to the current admitted capture or the sole spool
        // consuming it.
        unsafe { processor.frame(&campaign_frame(CAMPAIGN.as_bytes(), 1234, 10)) }.unwrap();
        // SAFETY: The spool or refusing capture is the last owner of this frame slot.
        unsafe { frames.recycle(&owned[0]) };
        mailbox.lock().unwrap().command =
            Some(Command::Begin(format!("0001 close {CAMPAIGN} 1234")));
        spool.poll(&mut processor, &frames, &owned);
        assert!(mailbox.lock().unwrap().response.is_none());
        // SAFETY: This frame or slot belongs to the current admitted capture or the sole spool
        // consuming it.
        unsafe { processor.frame(&campaign_frame(CAMPAIGN.as_bytes(), 1234, 11)) }.unwrap();
        // SAFETY: The spool or refusing capture is the last owner of this frame slot.
        unsafe { frames.recycle(&owned[1]) };
        spool.poll(&mut processor, &frames, &owned);
        let Some(Response::Began(Ok((HistoryReply::Closing { receipt, .. }, None)))) =
            mailbox.lock().unwrap().response.take()
        else {
            panic!("close must follow the last queued A frame")
        };
        while receipt.result().is_none() {
            std::thread::yield_now();
        }
        assert_eq!(receipt.result(), Some(Ok(())));
        mailbox.lock().unwrap().command = Some(Command::Begin(format!("0002 release {CAMPAIGN}")));
        spool.poll(&mut processor, &frames, &owned);
        assert!(matches!(mailbox.lock().unwrap().response.take(),
            Some(Response::Began(Ok((HistoryReply::Ready(ack), None)))) if ack == "0002 ok"));
        let settings = root.join("settings.json");
        let naming = recorder_replay::replay_io::Naming {
            localisation: None,
            settings: &settings,
            report: None,
        };
        // SAFETY: This spool's frame and command processing have finished before its processor is
        // closed.
        unsafe { processor.detach() }.unwrap();
        let ended = recorder_replay::replay_io::finalize_session_ahead(
            &live,
            &root.join("recordings"),
            &|| Ok(()),
            recorder_replay::replay_io::Live::default(),
            naming,
            &|_| recorder_replay::replay_io::Destination::Recording,
        )
        .unwrap();
        assert!(ended.errors.is_empty(), "{:?}", ended.errors);
        let replay = replay::campaign::Replay::open_path(&ended.replays[0]).unwrap();
        let hours: Vec<_> = replay
            .index
            .iter()
            .flat_map(|block| replay.read_block(block).unwrap())
            .map(|frame| frame.hour)
            .collect();
        assert_eq!(hours, [10, 11]);
        drop((replay, processor));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn release_waits_for_pending_frames_and_close_refuses_a_queued_reload() {
        use collector_spool::history::PreparedCampaigns;
        use collector_spool::spooling::campaign_frame;
        let root = root();
        let frames = collector_workers::frame_fence::Frames::default();
        let owned = [const { std::sync::atomic::AtomicU64::new(0) }; 3];
        let mailbox = Arc::new(Mutex::new(Mailbox::default()));
        let spool = SpoolHistory(Arc::clone(&mailbox));
        let mut processor =
            // SAFETY: The caller owns the session or isolated fixture state required by this
            // constructor.
            unsafe { Processor::new(&root, PreparedCampaigns::default(), session(1)) }.unwrap();
        let capture =
        // SAFETY: The admitted capture or its fixture owns this frame's observer sequencing.
        unsafe { frames.capture() };
        // SAFETY: This frame or slot belongs to the current admitted capture or the sole spool
        // consuming it.
        unsafe { capture.frame(&owned[0]) };
        mailbox.lock().unwrap().command = Some(Command::Begin(format!("0001 release {CAMPAIGN}")));
        spool.poll(&mut processor, &frames, &owned);
        assert!(mailbox.lock().unwrap().response.is_none());
        drop(capture);
        spool.poll(&mut processor, &frames, &owned);
        assert!(mailbox.lock().unwrap().response.is_none());
        // SAFETY: This frame or slot belongs to the current admitted capture or the sole spool
        // consuming it.
        unsafe { processor.frame(&campaign_frame(CAMPAIGN.as_bytes(), 1234, 10)) }.unwrap();
        // SAFETY: The spool or refusing capture is the last owner of this frame slot.
        unsafe { frames.recycle(&owned[0]) };
        spool.poll(&mut processor, &frames, &owned);
        assert!(matches!(mailbox.lock().unwrap().response.take(),
            Some(Response::Began(Ok((HistoryReply::Ready(ack), None)))) if ack == "0001 refused the campaign is recording"));
        let capture =
        // SAFETY: The admitted capture or its fixture owns this frame's observer sequencing.
        unsafe { frames.capture() };
        // SAFETY: This frame or slot belongs to the current admitted capture or the sole spool
        // consuming it.
        unsafe { capture.frame(&owned[1]) };
        drop(capture);
        mailbox.lock().unwrap().command =
            Some(Command::Begin(format!("0002 close {CAMPAIGN} 1234")));
        spool.poll(&mut processor, &frames, &owned);
        assert!(mailbox.lock().unwrap().response.is_none());
        // SAFETY: This frame or slot belongs to the current admitted capture or the sole spool
        // consuming it.
        unsafe { processor.frame(&campaign_frame(CAMPAIGN.as_bytes(), 5678, 20)) }.unwrap();
        // SAFETY: The spool or refusing capture is the last owner of this frame slot.
        unsafe { frames.recycle(&owned[1]) };
        let later_b =
        // SAFETY: The admitted capture or its fixture owns this frame's observer sequencing.
        unsafe { frames.capture() };
        // SAFETY: This frame or slot belongs to the current admitted capture or the sole spool
        // consuming it.
        unsafe { later_b.frame(&owned[2]) };
        spool.poll(&mut processor, &frames, &owned);
        assert!(matches!(mailbox.lock().unwrap().response.take(),
            Some(Response::Began(Ok((HistoryReply::Ready(ack), None)))) if ack == "0002 refused the campaign was loaded again"));
        drop(later_b);
        drop(processor);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lease_return_retries_and_detach_preserve_exact_history() {
        use collector_spool::history::PreparedCampaigns;
        use collector_spool::spooling::campaign_frame;
        use recorder_replay::replay_io;
        let root = root();
        let output = root.join("recordings");
        let settings = root.join("settings.json");
        let earlier = root.join("sessions/1");
        let live = root.join("sessions/2");
        let save = |folder: &Path| {
            let naming = replay_io::Naming {
                localisation: None,
                settings: &settings,
                report: None,
            };
            let ended = replay_io::finalize_session_ahead(
                folder,
                &output,
                &|| Ok(()),
                replay_io::Live::default(),
                naming,
                &|_| replay_io::Destination::Recording,
            )
            .unwrap();
            assert!(ended.errors.is_empty(), "{:?}", ended.errors);
            ended.replays[0].clone()
        };
        let mut processor =
            // SAFETY: The caller owns the session or isolated fixture state required by this
            // constructor.
            unsafe { Processor::new(&earlier, PreparedCampaigns::default(), session(1)) }.unwrap();
        for hour in 10..=11 {
            // SAFETY: This frame or slot belongs to the current admitted capture or the sole spool
            // consuming it.
            unsafe { processor.frame(&campaign_frame(CAMPAIGN.as_bytes(), 1234, hour)) }.unwrap();
        }
        // SAFETY: This spool's frame and command processing have finished before its processor is
        // closed.
        unsafe { processor.detach() }.unwrap();
        drop(processor);
        let recording = save(&earlier);
        let history = PreparedCampaigns::load(&output, &session(2), |_| Ok(())).unwrap();
        let mut processor =
        // SAFETY: The caller owns the session or isolated fixture state required by this
        // constructor.
        unsafe { Processor::new(&live, history, session(2)) }.unwrap();
        for hour in 20..=21 {
            // SAFETY: This frame or slot belongs to the current admitted capture or the sole spool
            // consuming it.
            unsafe { processor.frame(&campaign_frame(CAMPAIGN.as_bytes(), 5678, hour)) }.unwrap();
        }
        fs::create_dir_all(live.join("history-requests")).unwrap();
        let mut requests = HistoryRequests::default();
        let mut now = Instant::now();
        for token in ["0001", "0002"] {
            let path = request(&live, token, "lend");
            now += Duration::from_secs(1);
            poll_processor(&mut requests, &live, &output, &mut processor, now);
            let writer = fs::OpenOptions::new()
                .write(true)
                .share_mode(0)
                .open(&recording)
                .unwrap();
            assert!(replay_files::recording_files::recording_of(&output, CAMPAIGN.as_bytes()).is_err());
            publish(&path.with_extension("return"), token.as_bytes()).unwrap();
            now += Duration::from_secs(1);
            poll_processor(&mut requests, &live, &output, &mut processor, now);
            assert!(!archived(&live, &path, "returned").exists());
            assert!(!live.join("history-requests/detached").exists());
            drop(writer);
            if token == "0001" {
                now += Duration::from_secs(1);
                poll_processor(&mut requests, &live, &output, &mut processor, now);
                assert!(archived(&live, &path, "returned").exists());
                assert!(
                    fs::OpenOptions::new()
                        .write(true)
                        .share_mode(0)
                        .open(&recording)
                        .is_err()
                );
            }
        }
        // SAFETY: This spool's frame and command processing have finished before its processor is
        // closed.
        unsafe { processor.detach() }.unwrap();
        drop(processor);
        detached(&live).unwrap();
        assert!(live.join("history-requests/detached").exists());
        let recording = save(&live);
        let replay = replay::campaign::Replay::open(fs::read(recording).unwrap()).unwrap();
        let hours: Vec<_> = replay
            .index
            .iter()
            .flat_map(|block| replay.read_block(block).unwrap())
            .map(|frame| frame.hour)
            .collect();
        assert_eq!(hours, [10, 11, 20, 21]);
        fs::remove_dir_all(root).unwrap();
    }
}
