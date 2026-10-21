//! One serial source actor for all dashboard maps sharing a replay. Only its
//! thread seeks the playback; each map owns one coalescing, cancellable mailbox.
use super::*;

pub struct SharedDecoder {
    #[cfg(test)]
    pub(super) state: Arc<Mutex<DecoderState>>,
    actor: Actor,
}
pub(super) struct DecoderState {
    pub(super) playback: Playback,
    // Weak entries share frames already held by maps; no second payload cache.
    frames: BTreeMap<(BranchId, Hour), Weak<replay_frame::frame::DecodedFrame>>,
}
impl DecoderState {
    fn seed(&mut self, playback: Playback) {
        if let Current::Frame(frame) = playback.current() {
            self.frames
                .insert((frame.branch, frame.hour), Arc::downgrade(frame));
        }
        self.playback = playback;
    }
    fn frame(&mut self, branch: BranchId, hour: Hour) -> io::Result<Current> {
        if let Some(frame) = self.frames.get(&(branch, hour)).and_then(Weak::upgrade) {
            return Ok(Current::Frame(frame));
        }
        self.playback.seek(branch, hour)?;
        let current = self.playback.current().clone();
        self.frames.retain(|_, frame| frame.strong_count() > 0);
        if self.frames.len() >= FRAME_CACHE {
            self.frames.pop_first();
        }
        if let Current::Frame(frame) = &current {
            self.frames.insert((branch, hour), Arc::downgrade(frame));
        }
        Ok(current)
    }
}
impl SharedDecoder {
    pub(super) fn new(replay: Arc<Replay>) -> io::Result<Self> {
        let state = Arc::new(Mutex::new(DecoderState {
            playback: Playback::unloaded(replay)?,
            frames: BTreeMap::new(),
        }));
        let actor = Actor::start(state.clone())?;
        Ok(Self {
            #[cfg(test)]
            state,
            actor,
        })
    }
    pub fn seed(&self, mut playback: Playback) -> io::Result<()> {
        // Source loaders, never the coordinator/UI, bind the committed reader.
        if let Some(source) = playback.checkpoint_source() {
            match source.open() {
                Ok(Some(checkpoints)) => playback = playback.with_checkpoints(checkpoints),
                Ok(None) => {}
                Err(error) => eprintln!(
                    "Map checkpoint binding unavailable: {error}; using the replay reader"
                ),
            }
        }
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
                            let result = state
                                .lock()
                                .map_err(|_| stopped())
                                .and_then(|mut state| state.frame(branch, hour));
                            let sample = clock.finish(branch, hour);
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
        Ok(Self { events, clients })
    }
}
impl Drop for Actor {
    fn drop(&mut self) {
        let _ = self.events.send(Command::Shutdown);
    }
}

#[cfg(test)]
mod tests;
