//! One serial source actor for all dashboard maps sharing a replay. Only its
//! thread seeks the playback; each map owns one coalescing, cancellable mailbox.
use super::*;

pub struct SharedDecoder {
    actor: Actor,
}
pub(super) struct DecoderState {
    pub(super) playback: Playback,
    // Weak entries share frames already held by maps; no second payload cache.
    frames: BTreeMap<(BranchId, Hour), Weak<replay_frame::frame::DecodedFrame>>,
    workers: usize,
    dropper: Option<Arc<Dropper>>,
}
impl DecoderState {
    /// The campaign lane's read setup: its change-run workers and block read-ahead.
    fn reading(playback: Playback, workers: usize, dropper: Option<&Arc<Dropper>>) -> Playback {
        let playback = playback.with_map_workers(workers).with_display_changes();
        match dropper {
            Some(dropper) => {
                let reader = BlockReader::new(playback.replay_handle(), Arc::clone(dropper));
                playback.with_reader(reader)
            }
            None => playback,
        }
    }
    fn seed(&mut self, playback: Playback) {
        self.frames.clear();
        if let Current::Frame(frame) = playback.current() {
            self.frames
                .insert((frame.branch, frame.hour), Arc::downgrade(frame));
        }
        self.playback = Self::reading(playback, self.workers, self.dropper.as_ref());
    }
    #[cfg(test)]
    fn frame(&mut self, branch: BranchId, hour: Hour) -> io::Result<Current> {
        self.frame_cancellable(branch, hour, &|| false)
    }
    fn frame_cancellable(
        &mut self,
        branch: BranchId,
        hour: Hour,
        cancelled: &dyn Fn() -> bool,
    ) -> io::Result<Current> {
        if cancelled() {
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        if let Some(frame) = self.frames.get(&(branch, hour)).and_then(Weak::upgrade) {
            return Ok(Current::Frame(frame));
        }
        self.playback
            .seek_cancellable(branch, hour, |_| {}, cancelled)?;
        let current = self.playback.current().clone();
        self.frames.retain(|_, frame| frame.strong_count() > 0);
        if let Current::Frame(frame) = &current {
            self.frames.insert((branch, hour), Arc::downgrade(frame));
        }
        Ok(current)
    }
}
impl SharedDecoder {
    #[cfg(test)]
    pub(super) fn new(replay: Arc<Replay>) -> io::Result<Self> {
        Self::serving(replay, 1, None)
    }
    pub(super) fn serving(
        replay: Arc<Replay>,
        workers: usize,
        dropper: Option<Arc<Dropper>>,
    ) -> io::Result<Self> {
        let state = Arc::new(Mutex::new(DecoderState {
            playback: DecoderState::reading(Playback::unloaded(replay)?, workers, dropper.as_ref()),
            frames: BTreeMap::new(),
            workers,
            dropper,
        }));
        let actor = Actor::start(state)?;
        Ok(Self { actor })
    }
    pub fn seed(&self, playback: Playback) -> io::Result<()> {
        let (send, result) = mpsc::sync_channel(1);
        self.actor
            .events
            .send(Command::Seed(Box::new(playback), send))
            .map_err(|_| stopped())?;
        result.recv().map_err(|_| stopped())?
    }
    pub(super) fn client(&self) -> Arc<SourceMailbox> {
        self.actor.client()
    }
}

enum Command {
    ReadReady,
    Seed(Box<Playback>, mpsc::SyncSender<io::Result<()>>),
    Shutdown,
}
#[derive(Default)]
struct Clients {
    dead: bool,
    mailboxes: Vec<Weak<SourceMailbox>>,
    ready: VecDeque<Weak<SourceMailbox>>,
    queued: bool,
}
struct Actor {
    events: Sender<Command>,
    clients: Arc<Mutex<Clients>>,
    closing: Arc<AtomicBool>,
}
struct Exit(Arc<Mutex<Clients>>);
impl Drop for Exit {
    fn drop(&mut self) {
        let clients = {
            let mut clients = self.0.lock().unwrap_or_else(|p| p.into_inner());
            clients.dead = true;
            clients
                .mailboxes
                .iter()
                .filter_map(Weak::upgrade)
                .collect::<Vec<_>>()
        };
        // Callbacks run outside the registry and source locks, including panic.
        for client in clients {
            client.stop();
        }
    }
}
impl Actor {
    fn client(&self) -> Arc<SourceMailbox> {
        let registry = self.clients.clone();
        let events = self.events.clone();
        let mailbox = SourceMailbox::new(move |mailbox| {
            let mut clients = registry.lock().unwrap_or_else(|p| p.into_inner());
            if clients.dead {
                return false;
            }
            // The queue owns a single wake, never a client. Prune dead weak
            // entries on registration/wake so cancelled, dropped maps cannot
            // accumulate behind a source read currently in progress.
            clients.ready.retain(|client| client.strong_count() > 0);
            clients.ready.push_back(Arc::downgrade(&mailbox));
            if !clients.queued {
                clients.queued = true;
                return events.send(Command::ReadReady).is_ok();
            }
            true
        });
        let dead = {
            let mut clients = self.clients.lock().unwrap_or_else(|p| p.into_inner());
            clients.mailboxes.retain(|client| client.strong_count() > 0);
            clients.ready.retain(|client| client.strong_count() > 0);
            clients.mailboxes.push(Arc::downgrade(&mailbox));
            clients.dead
        };
        if dead {
            mailbox.stop();
        }
        mailbox
    }

    fn take_ready(
        clients: &Mutex<Clients>,
        events: &Sender<Command>,
    ) -> Option<Arc<SourceMailbox>> {
        let mut clients = clients.lock().unwrap_or_else(|p| p.into_inner());
        clients.queued = false;
        let mailbox = loop {
            let weak = clients.ready.pop_front()?;
            if let Some(mailbox) = weak.upgrade() {
                break mailbox;
            }
        };
        if !clients.ready.is_empty() {
            clients.queued = events.send(Command::ReadReady).is_ok();
        }
        Some(mailbox)
    }

    fn start(state: Arc<Mutex<DecoderState>>) -> io::Result<Self> {
        let (events, inbox) = mpsc::channel();
        let clients = Arc::new(Mutex::new(Clients::default()));
        let registry = clients.clone();
        let events_for_thread = events.clone();
        let closing = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&closing);
        thread::Builder::new()
            .name("map source decoder".into())
            .spawn(move || {
                let _exit = Exit(registry.clone());
                background_priority();
                // recv owns the idle source thread, never a map coordinator.
                while let Ok(command) = inbox.recv() {
                    match command {
                        Command::Shutdown => return,
                        Command::Seed(playback, done) => {
                            let result = state
                                .lock()
                                .map_err(|_| stopped())
                                .map(|mut state| state.seed(*playback));
                            let _ = done.send(result);
                        }
                        Command::ReadReady => {
                            let Some(mailbox) = Self::take_ready(&registry, &events_for_thread)
                            else {
                                continue;
                            };
                            let Some(read) = mailbox.take() else { continue };
                            if read.cancelled() {
                                continue;
                            }
                            let (branch, hour) = read.key;
                            let clock = trace::DecodeClock::start(
                                viewer_foundation::profile_writer::profiling(),
                            );
                            let result =
                                state.lock().map_err(|_| stopped()).and_then(|mut state| {
                                    state.frame_cancellable(branch, hour, &|| {
                                        read.cancelled() || stopping.load(Ordering::Acquire)
                                    })
                                });
                            let sample = clock.finish(branch, hour);
                            if result
                                .as_ref()
                                .is_err_and(|error| error.kind() == io::ErrorKind::Interrupted)
                            {
                                drop(read);
                                continue;
                            }
                            read.ready(
                                result.unwrap_or_else(|error| Current::Gap {
                                    branch,
                                    hour,
                                    reason: error.to_string(),
                                }),
                                sample,
                            );
                        }
                    }
                }
            })?;
        Ok(Self {
            events,
            clients,
            closing,
        })
    }
}
impl Drop for Actor {
    fn drop(&mut self) {
        self.closing.store(true, Ordering::Release);
        let _ = self.events.send(Command::Shutdown);
    }
}

#[cfg(test)]
mod tests;
