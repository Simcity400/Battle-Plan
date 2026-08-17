//! The timeline the owner asked for: "a smooth timeline that you can scrub like
//! a video" (owner ruling, 2026-08-18).
//!
//! WHY THE DRAWING THREAD CANNOT DO THIS WORK. Building one hour's world costs,
//! at the bottom of the stack, a checkpoint decode plus every delta between it
//! and the wanted hour; above that a `serde_json` parse of the whole world and
//! a typed frame build; and above THAT the F4 anomaly scan, which observes
//! every hour between where it had got to and the target. Measured on the real
//! captures with the shipped reader
//! (`extractor/native/runs/mc_replay_scrub/`), the cache layer alone costs
//! 6,623 ms for a seek 743 deltas deep, and even standing still — asking again
//! for the hour already reconstructed — costs 179 ms on capture t53, because
//! the world is spelled back out as 31 MB of JSON and hashed against its parity
//! digest every time. A display frame is 16 ms. There is no arrangement of
//! checkpoints, cursors or ladders that makes a cold seek fit in one, so the
//! drawing thread must stop waiting for one.
//!
//! WHAT THIS DOES INSTEAD. One worker owns the frame path. The window loop
//! posts the hour it wants and takes whatever the worker has finished; if that
//! is not the requested hour yet, it draws the hour it does have and says so.
//! That is what a video scrubber does, and it is why a drag feels continuous
//! even though no individual frame arrived in 16 ms.
//!
//! WHAT IT DELIBERATELY DOES NOT DO. It does not interpolate, average, or
//! present one hour's state under another hour's number. Every frame it hands
//! back is a real recorded hour, reconstructed in full and proved against the
//! digest the recorder published for it, and it is labelled with the hour it
//! actually is.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use replay_core::WorldFrame;

use crate::timeline::OwnerProvider;

/// Everything the render path needs for one hour, computed together on the
/// worker so the drawing thread never reaches back into the provider for a
/// piece it is missing.
///
/// This bundle exists because the pieces are NOT independently cheap. The
/// anomaly note in particular is taken under a lock the worker holds across its
/// whole scan (`RecordingTimeline::scan_anomalies_through`), so a UI thread
/// that asked for it separately would block on exactly the work this module
/// exists to move off that thread (Codex design review, finding 3).
#[derive(Clone)]
pub struct DisplayedHour {
    /// The hour this state IS. Never the hour that was asked for, unless they
    /// happen to be the same.
    pub hour: i64,
    pub frame: Option<Arc<WorldFrame>>,
    pub owners: Arc<BTreeMap<u32, String>>,
    pub controllers: Arc<BTreeMap<u32, String>>,
    pub label: String,
    pub anomaly_note: Option<String>,
}

/// How the hour being drawn relates to the hour that was asked for.
///
/// A viewer that only ever showed "the current frame" could not tell a settled
/// timeline from one that has silently stopped converging, which is the stale
/// frame Codex's review named (finding 6). These three are the whole space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScrubStatus {
    /// What is drawn is what was asked for.
    Settled,
    /// Still working towards `requested`; what is drawn is an earlier answer.
    Converging { requested: i64 },
    /// `requested` will never be drawn, and this is why. The recording refused
    /// it — a declared coverage gap, a damaged record, a parity mismatch — so
    /// the timeline is NOT still converging and must not be presented as if it
    /// were. What is drawn is the last hour that was good.
    Refused { requested: i64, reason: String },
}

/// The worker's answer to one request.
enum Answer {
    Ready(Box<DisplayedHour>),
    Refused { hour: i64, reason: String },
}

struct Wanted {
    hour: Option<i64>,
    /// The most recently finished answer, waiting to be collected.
    ///
    /// It lives INSIDE the request lock, not beside it. Checking the generation
    /// under one lock and publishing under another let the check pass, a new
    /// request land, and the stale answer be published after it, so a
    /// superseded `Refused` could replace a live `Converging`, log the wrong
    /// refusal, and make the next frame re-post the request it had just
    /// cancelled (Codex implementation review, finding 3). One lock makes
    /// "still the generation I was serving" and "publish" indivisible.
    finished: Option<(u64, Answer)>,
    /// Bumped on every new request. The worker carries the generation it is
    /// serving and discards its own result if the number moved while it worked,
    /// which is what makes "latest request wins" true of what is DRAWN and not
    /// merely of a queue.
    generation: u64,
    shutting_down: bool,
}

struct Shared {
    wanted: Mutex<Wanted>,
    wake: Condvar,
    /// Read WITHOUT the request lock, by the reconstruction interrupt, so a walk
    /// in progress can notice it has been superseded without contending for
    /// that lock on every delta record it applies.
    live_generation: AtomicU64,
    /// The thread the reconstructions run on, published by that thread. The
    /// stop switch speaks only for it; see [`ScrubEngine::start`].
    worker_thread: std::sync::OnceLock<std::thread::ThreadId>,
    /// How far the eager walk has got. See [`ScrubEngine::start`].
    scanned_through: AtomicI64,
    /// The hour the eager walk was refused at, if any. It is not retried; see
    /// the eager arm of [`run_worker`].
    eager_stalled_at: AtomicI64,
    stopped: AtomicBool,
}

/// The drawing thread's handle on the worker.
pub struct ScrubEngine {
    shared: Arc<Shared>,
    /// Held so `Drop` can uninstall the stop switch it installed.
    provider: Arc<dyn OwnerProvider>,
    worker: Option<std::thread::JoinHandle<()>>,
    /// The last good hour, owned by the drawing thread. This is what gets drawn
    /// whenever the worker has nothing newer, and it is why a scrub never shows
    /// an empty map.
    shown: DisplayedHour,
    status: ScrubStatus,
    /// The generation of the request `shown` answered, so a late arrival from
    /// an older request cannot overwrite a newer answer.
    shown_generation: u64,
}

/// Build one hour's bundle. Every call here is on the worker thread.
fn resolve(provider: &dyn OwnerProvider, hour: i64) -> Result<DisplayedHour, String> {
    // Order matters only for cost, not for correctness: `world_frame_at` is the
    // call that runs the F4 scan, so doing it first means the cheaper lookups
    // below find the provider's caches already warm for this hour.
    let frame = provider.world_frame_at(hour).map_err(|error| error.to_string())?;
    let owners = provider.owners_at(hour).map_err(|error| error.to_string())?;
    let controllers = provider
        .controllers_at(hour)
        .map_err(|error| error.to_string())?;
    let label = provider.hour_label(hour).map_err(|error| error.to_string())?;
    let anomaly_note = provider
        .anomaly_note(hour)
        .map_err(|error| error.to_string())?;
    Ok(DisplayedHour {
        hour,
        frame,
        owners,
        controllers,
        label,
        anomaly_note,
    })
}

impl ScrubEngine {
    /// Start the worker and produce the first hour.
    ///
    /// THE ONE PLACE THIS BLOCKS, AND WHY. There is nothing to draw until one
    /// hour exists, so opening waits for the first. Every later request is
    /// answered from what is already in hand.
    pub fn start(
        provider: Arc<dyn OwnerProvider>,
        first_hour: i64,
    ) -> Result<ScrubEngine, String> {
        let first = resolve(provider.as_ref(), first_hour)?;
        let shared = Arc::new(Shared {
            wanted: Mutex::new(Wanted {
                hour: None,
                finished: None,
                generation: 0,
                shutting_down: false,
            }),
            wake: Condvar::new(),
            live_generation: AtomicU64::new(0),
            worker_thread: std::sync::OnceLock::new(),
            scanned_through: AtomicI64::new(first_hour),
            eager_stalled_at: AtomicI64::new(i64::MIN),
            stopped: AtomicBool::new(false),
        });
        // THE STOP SWITCH THE FRAME PATH CONSULTS WHILE IT WORKS.
        //
        // Superseding a request in the queue is not enough: a cold backward
        // seek inside a 744-hour window applies up to 743 delta records and
        // takes seconds (6,623 ms measured on capture t55), and until this
        // existed the newest drag position waited behind a walk towards a
        // position the user had already dragged past. The switch says "the
        // generation I started is still the generation being asked for"; the
        // cache reader reads it at every delta-record boundary and, when it
        // says stop, KEEPS the position it had reached — so the abandoned
        // distance is resumed rather than repeated.
        //
        // `serving` is the generation the worker is currently on. It is written
        // by the worker before it starts an hour and read by the reconstruction
        // it started, so the switch always compares the live request against
        // the work in flight and never against a request that has not begun.
        // ...AND IT ONLY EVER SPEAKS FOR THE WORKER'S OWN WORK. The switch is
        // installed on a provider the WINDOW thread also holds, and the window
        // thread does reconstruct at open time (`pov_picker` reads the first
        // frame's country roster). Without the thread check below, a request
        // arriving at that moment would abandon the WINDOW's reconstruction and
        // fail the launch over a seek nobody asked that thread to make. Today
        // that call happens to be served from the frame cache, so the bug is
        // latent rather than live — which is exactly the kind of thing that
        // stops being latent when somebody reorders two lines.
        //
        // Until the worker's identity is published (a window of a few
        // instructions during `spawn`) the switch says keep going, which is the
        // safe answer: an uninterruptible reconstruction is merely the old
        // behaviour.
        let serving = Arc::new(AtomicU64::new(0));
        let switch_shared = Arc::clone(&shared);
        let switch_serving = Arc::clone(&serving);
        provider.set_scrub_interrupt(Some(Arc::new(move || {
            let Some(worker) = switch_shared.worker_thread.get() else {
                return true;
            };
            if std::thread::current().id() != *worker {
                return true;
            }
            switch_shared.live_generation.load(Ordering::SeqCst)
                == switch_serving.load(Ordering::SeqCst)
                && !switch_shared.stopped.load(Ordering::SeqCst)
        })));
        let worker_shared = Arc::clone(&shared);
        let worker_provider = Arc::clone(&provider);
        let worker_serving = Arc::clone(&serving);
        let worker = std::thread::Builder::new()
            .name("replay-scrub".to_string())
            .spawn(move || {
                // Published by the worker itself, so the id is unarguably the
                // thread that runs the reconstructions.
                let _ = worker_shared.worker_thread.set(std::thread::current().id());
                run_worker(worker_shared, worker_provider, worker_serving)
            })
            .map_err(|error| {
                // The switch is installed above, before the spawn, because the
                // worker must never run an unguarded reconstruction. If the
                // spawn fails there is no `ScrubEngine` and therefore no `Drop`
                // to clear it, so a retained provider would keep the closure
                // and its `Arc<Shared>` for ever (Codex round 4, finding 1).
                provider.set_scrub_interrupt(None);
                format!("start the replay scrub worker: {error}")
            })?;
        Ok(ScrubEngine {
            shared,
            provider,
            worker: Some(worker),
            shown: first,
            status: ScrubStatus::Settled,
            shown_generation: 0,
        })
    }

    /// Post the hour the transport wants and return what to draw NOW.
    ///
    /// Never blocks, never fails, and always returns a real recorded hour.
    /// Returned by value, not by reference. The window loop that consumes this
    /// holds the engine for the whole frame and reaches for a dozen other
    /// things in between; a borrow of the engine would make every one of them
    /// a borrow-checker argument. Everything inside is an `Arc` or a short
    /// string, so the clone is cheap.
    pub fn show(&mut self, requested: i64) -> (DisplayedHour, ScrubStatus) {
        if requested != self.shown.hour || !matches!(self.status, ScrubStatus::Settled) {
            self.post(requested);
        }
        self.collect(requested);
        (self.shown.clone(), self.status.clone())
    }

    /// The last good hour, without posting a request. Event handlers that need
    /// the current frame use this: a key press is not a seek.
    pub fn shown(&self) -> &DisplayedHour {
        &self.shown
    }

    fn post(&mut self, requested: i64) {
        // A request already in flight for this very hour is not re-posted:
        // bumping the generation would cancel the work that is about to answer
        // it, and a drag that sat still would never converge.
        if let ScrubStatus::Converging { requested: in_flight } = &self.status {
            if *in_flight == requested {
                return;
            }
        }
        if let ScrubStatus::Refused { requested: refused, .. } = &self.status {
            if *refused == requested {
                return;
            }
        }
        let Ok(mut wanted) = self.shared.wanted.lock() else {
            return;
        };
        wanted.hour = Some(requested);
        wanted.generation += 1;
        self.shared
            .live_generation
            .store(wanted.generation, Ordering::SeqCst);
        self.status = ScrubStatus::Converging { requested };
        self.shared.wake.notify_all();
    }

    fn collect(&mut self, requested: i64) {
        let Ok(mut wanted) = self.shared.wanted.lock() else {
            return;
        };
        let current_generation = wanted.generation;
        let Some((generation, answer)) = wanted.finished.take() else {
            drop(wanted);
            self.notice_a_dead_worker(requested);
            return;
        };
        drop(wanted);
        // An answer can be for a request that has ALREADY been superseded: it
        // was published, and then `post` ran before this collect. What may be
        // done with it depends entirely on what it says.
        //
        // A READY answer is always welcome whatever its age -- it is a real
        // recorded hour, fully reconstructed and proved against its digest, and
        // showing it is strictly better than showing an older one while the
        // newer request works. It just must not be called Settled.
        //
        // A REFUSED answer is a claim about a specific requested hour, and a
        // superseded one is a claim about a question nobody is asking any more.
        // Adopting it made the timeline report the wrong refusal and re-post
        // the request it had just cancelled (Codex round 3, finding 4).
        let superseded = generation != current_generation;
        if superseded && matches!(answer, Answer::Refused { .. }) {
            return;
        }
        // An answer from an older request than the one already drawn is stale
        // and is dropped rather than shown.
        if generation < self.shown_generation {
            return;
        }
        self.shown_generation = generation;
        match answer {
            Answer::Ready(hour) => {
                let settled = hour.hour == requested && !superseded;
                self.shown = *hour;
                self.status = if settled {
                    ScrubStatus::Settled
                } else {
                    ScrubStatus::Converging { requested }
                };
            }
            Answer::Refused { hour, reason } => {
                // The last good frame stays on screen; what changes is that the
                // timeline stops claiming to be on its way there.
                self.status = ScrubStatus::Refused {
                    requested: hour,
                    reason,
                };
            }
        }
    }

    /// A worker that has ended without answering must not leave the timeline
    /// saying "seeking" for ever.
    ///
    /// The refusal protocol covers an hour the RECORDING refuses. This covers
    /// the other way a request can never be answered: the thread that would
    /// have answered it is gone — it panicked inside the provider, or a lock it
    /// needed was poisoned and it returned. Without this the viewer shows the
    /// last good frame under a permanent "seeking to hour N", which is exactly
    /// the silent stale frame the status enum exists to make impossible.
    fn notice_a_dead_worker(&mut self, requested: i64) {
        if !matches!(self.status, ScrubStatus::Converging { .. }) {
            return;
        }
        let dead = self
            .worker
            .as_ref()
            .is_none_or(std::thread::JoinHandle::is_finished);
        if dead {
            self.status = ScrubStatus::Refused {
                requested,
                reason: "the replay scrub worker stopped; no further hour can be prepared"
                    .to_string(),
            };
        }
    }

    /// How far the eager walk has authenticated, for a progress indicator. This
    /// is the frontier the F4 scan has observed in order, not a promise about
    /// any particular hour.
    pub fn scanned_through(&self) -> i64 {
        self.shared.scanned_through.load(Ordering::Relaxed)
    }
}

impl Drop for ScrubEngine {
    fn drop(&mut self) {
        if let Ok(mut wanted) = self.shared.wanted.lock() {
            wanted.shutting_down = true;
        }
        self.shared.stopped.store(true, Ordering::SeqCst);
        self.shared.wake.notify_all();
        // THE SWITCH IS LEFT ARMED UNTIL THE JOIN RETURNS. It reads `stopped`,
        // which was just set, so a walk in flight abandons at its next delta
        // record instead of running to completion -- that is what keeps closing
        // the window from waiting seconds on a deep cold seek. Clearing it here
        // would disarm cancellation at exactly the moment shutdown needs it.
        if let Some(worker) = self.worker.take() {
            // Joining rather than detaching: the worker holds an `Arc` on the
            // provider, and a detached one would keep a recording's file
            // handles alive past the window that opened them.
            //
            // WHAT THIS CAN WAIT FOR, STATED. The worker checks `stopped` only
            // between hours, so closing the window while a cold deep seek is in
            // flight waits for that one hour — seconds on a wide capture. It
            // cannot wait for two, and it cannot wait for the eager walk, which
            // advances one hour at a time. Cutting it shorter needs the stop
            // switch to reach INSIDE a reconstruction, which the cache reader
            // already supports (`reconstruct_json_interruptible`) and the
            // provider chain does not yet carry: residual R1 in
            // `runs/mc_replay_scrub/SCRUB_FINDINGS.txt`.
            let _ = worker.join();
        }
        // Now that nothing is running, break the switch's hold: the closure
        // owns an `Arc<Shared>`, and a provider outliving this engine would
        // keep it alive for ever. Clearing it also leaves any later, non-scrub
        // caller of the same provider uninterruptible again, which is the
        // default every other caller expects.
        self.provider.set_scrub_interrupt(None);
    }
}

/// One request at a time, requests before eager work, and never more than one
/// hour of eager work between two requests.
///
/// THE QUANTUM IS THE POINT. The F4 anomaly scan has to observe every hour in
/// order before any later hour can be shown, so somebody has to walk the
/// recording; doing it eagerly means the walk is finished before the user drags
/// somewhere far away, instead of landing on them at the moment they do. But a
/// tight eager loop would hold the provider continuously and starve the very
/// requests it is meant to serve (Codex design review, finding 4), so it
/// advances by exactly ONE hour and then looks again for a request.
fn run_worker(shared: Arc<Shared>, provider: Arc<dyn OwnerProvider>, serving: Arc<AtomicU64>) {
    let hours: Vec<i64> = provider.hours().to_vec();
    loop {
        let (hour, generation, eager) = {
            let Ok(mut wanted) = shared.wanted.lock() else {
                return;
            };
            loop {
                if wanted.shutting_down {
                    return;
                }
                if let Some(hour) = wanted.hour.take() {
                    break (hour, wanted.generation, false);
                }
                // Nothing wanted: take one step of the eager walk if there is
                // one left, otherwise sleep until there is something to do.
                let frontier = shared.scanned_through.load(Ordering::Relaxed);
                let stalled = shared.eager_stalled_at.load(Ordering::Relaxed);
                // Binary search, not a scan: this runs every time the worker
                // goes idle, which on a responsive timeline is once per drawn
                // frame, and the hour spine of a full campaign is six figures
                // long. The spine is ascending -- `WorldTimeline::open` refuses
                // one that is not -- so `partition_point` is exact.
                let next = hours
                    .get(hours.partition_point(|hour| *hour <= frontier))
                    .copied();
                match next {
                    // The eager walk has already been refused this hour once.
                    // Retrying it on every idle turn would re-pay a failing
                    // resolve for every request the user makes, for ever, and
                    // buy nothing: the scan is ordered and cannot step over it.
                    // A request for that hour still refuses on its own account,
                    // where somebody is waiting to be told.
                    Some(next) if next != stalled => break (next, wanted.generation, true),
                    _ => {}
                }
                let Ok(next) = shared.wake.wait(wanted) else {
                    return;
                };
                wanted = next;
            }
        };

        // Publish which generation is in flight BEFORE starting, so the stop
        // switch compares against this hour and not the last.
        //
        // ALWAYS the generation captured UNDER THE LOCK when this hour was
        // chosen -- never a fresh read of the live one. Re-reading it here let
        // an eager step ADOPT a request that had arrived in between: the eager
        // hour was picked, the window posted a request, the eager step then
        // loaded the new generation, the switch saw them agree, and a
        // hundreds-of-deltas eager reconstruction ran to completion while the
        // request waited (Codex implementation review round 3, finding 1).
        // Captured at selection, any later post makes the two disagree and the
        // eager walk gives way at its next record, which is the whole point of
        // the eager walk being one hour at a time.
        serving.store(generation, Ordering::SeqCst);
        let answer = resolve(provider.as_ref(), hour);
        if shared.stopped.load(Ordering::SeqCst) {
            return;
        }
        if eager {
            // The eager walk publishes nothing: it exists to move the scan's
            // frontier, and a frontier hour is not an hour anybody asked to
            // see. It stops advancing at the first hour the recording refuses,
            // because the scan is ordered and cannot step over one.
            match answer {
                Ok(_) => shared.scanned_through.store(hour, Ordering::Relaxed),
                // A failure here means one of two very different things, and
                // only one of them is about the recording. If the generation
                // moved while this step ran, the failure IS the stop switch
                // firing -- the walk was told to give way to a request -- and
                // recording nothing is right: the hour is untried, not refused.
                // Marking it stalled would have parked the eager scan at the
                // first hour a user happened to interrupt (Codex round 3,
                // finding 3).
                Err(_) => {
                    if shared.live_generation.load(Ordering::SeqCst) == generation {
                        shared.eager_stalled_at.store(hour, Ordering::Relaxed);
                    }
                }
            }
            continue;
        }

        if answer.is_ok() {
            shared.scanned_through.fetch_max(hour, Ordering::Relaxed);
        }
        // THE GENERATION CHECK AND THE PUBLICATION ARE ONE CRITICAL SECTION.
        // A request whose generation moved while it was being served is stale;
        // its work is not wasted (the provider's caches and the cache reader's
        // cursor keep it) but it is not what anyone is waiting for now, and it
        // must not be able to land AFTER the request that replaced it.
        //
        // A stale result is dropped whether it succeeded or failed. That is
        // what makes the reconstruction interrupt safe without a typed
        // "cancelled" error: an interrupted walk fails, and it can only have
        // been interrupted because the generation moved, so its failure is
        // discarded here rather than reported as a refusal of an hour the
        // recording is perfectly able to serve.
        let Ok(mut wanted) = shared.wanted.lock() else {
            return;
        };
        if wanted.generation != generation {
            continue;
        }
        wanted.finished = Some((
            generation,
            match answer {
                Ok(displayed) => Answer::Ready(Box::new(displayed)),
                Err(reason) => Answer::Refused { hour, reason },
            },
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use std::time::{Duration, Instant};

    /// A provider whose every hour takes a stated time to resolve, so a test
    /// can ask what the drawing thread was doing while it did.
    struct SlowProvider {
        hours: Vec<i64>,
        per_hour: Duration,
        resolved: AtomicUsize,
        /// Counted only when an hour is actually REFUSED. `resolved` is already
        /// past any small threshold before `ScrubEngine::start` returns (the
        /// first hour costs two counted calls), so a test that waits on it is
        /// really only waiting on its own sleep (Codex round 5, finding 2).
        refusals: AtomicUsize,
        refuse: Option<i64>,
        /// Kills the worker outright, standing in for a panic anywhere inside
        /// the provider stack.
        panic_at: Option<i64>,
    }

    impl SlowProvider {
        fn new(count: i64, per_hour: Duration, refuse: Option<i64>) -> Arc<SlowProvider> {
            Arc::new(SlowProvider {
                hours: (0..count).collect(),
                per_hour,
                resolved: AtomicUsize::new(0),
                refusals: AtomicUsize::new(0),
                refuse,
                panic_at: None,
            })
        }

        fn panicking_at(count: i64, hour: i64) -> Arc<SlowProvider> {
            Arc::new(SlowProvider {
                hours: (0..count).collect(),
                per_hour: Duration::from_millis(1),
                resolved: AtomicUsize::new(0),
                refusals: AtomicUsize::new(0),
                refuse: None,
                panic_at: Some(hour),
            })
        }
    }

    impl OwnerProvider for SlowProvider {
        fn hours(&self) -> &[i64] {
            &self.hours
        }
        fn owners_at(&self, hour: i64) -> crate::Result<Arc<BTreeMap<u32, String>>> {
            if self.panic_at == Some(hour) {
                panic!("provider stack died at hour {hour}");
            }
            // Counted BEFORE the refusal, so it counts ATTEMPTS. Counting only
            // the ones that succeed made a retry loop over a refused hour
            // invisible to the test that exists to catch it (Codex round 3,
            // finding 7).
            self.resolved.fetch_add(1, Ordering::SeqCst);
            if self.refuse == Some(hour) {
                self.refusals.fetch_add(1, Ordering::SeqCst);
                return Err(format!("hour {hour} is inside a declared coverage gap").into());
            }
            std::thread::sleep(self.per_hour);
            Ok(Arc::new(BTreeMap::from([(1u32, format!("H{hour}"))])))
        }
        fn controllers_at(&self, hour: i64) -> crate::Result<Arc<BTreeMap<u32, String>>> {
            self.owners_at(hour)
        }
        fn hour_label(&self, hour: i64) -> crate::Result<String> {
            Ok(format!("hour {hour}"))
        }
    }

    /// EVERY LINK OF THE CHAIN THAT CARRIES THE STOP SWITCH TO A READER.
    ///
    /// `InterruptibleProvider` below stands in for the reader, so it proves the
    /// engine INSTALLS a switch and that a reader honouring one is obeyed — but
    /// it is handed the switch directly, so deleting any of the real
    /// pass-throughs in between left the whole suite green (Codex round 4,
    /// finding 2). Those pass-throughs are one-line delegations with no
    /// behaviour of their own to observe from a test, so they are checked the
    /// way this repository already checks the product source wrapper
    /// (`recording_open::tests::the_product_source_wrapper_forwards_every_replay_source_accessor`):
    /// by requiring the delegation to be present in the source.
    ///
    /// kills: deleting the body of any one of the four delegations. The chain
    /// is only as good as its weakest link, and a silently-defaulted link means
    /// a scrub that can never be interrupted on the route players take.
    #[test]
    fn every_link_that_carries_the_stop_switch_to_a_reader_delegates() {
        // (file, what it must delegate to)
        let links: [(&str, &str, &str); 4] = [
            (
                include_str!("timeline.rs"),
                "OwnerProvider for RecordingTimeline",
                "self.timeline.set_scrub_interrupt(",
            ),
            (
                include_str!("../../replay_core/src/world.rs"),
                "WorldTimeline",
                "self.source.set_scrub_interrupt(",
            ),
            (
                include_str!("../../replay_core/src/source.rs"),
                "ReplaySource for IndexedReplaySource",
                "self.cache.set_scrub_interrupt(",
            ),
            (
                include_str!("recording_open.rs"),
                "ReplaySource for ProductReplaySource",
                "self.inner.set_scrub_interrupt(",
            ),
        ];
        for (source, who, delegation) in links {
            let source = source.replace("
", "
");
            assert!(
                source.contains("fn set_scrub_interrupt("),
                "{who} must implement set_scrub_interrupt"
            );
            assert!(
                source.contains(delegation),
                "{who} must delegate the stop switch onward with {delegation}; without it the                  switch stops here and a scrub can never be interrupted below this point"
            );
        }
    }

    /// A provider that actually honours an installed stop switch, so the
    /// production installation and propagation path is under test.
    ///
    /// Every other double here leaves `set_scrub_interrupt` as the trait's
    /// default no-op, which means deleting the whole interruption path --
    /// `ScrubEngine`'s closure, `OwnerProvider::set_scrub_interrupt`,
    /// `WorldTimeline`'s and `ReplaySource`'s pass-throughs, and the cache
    /// reader's boundary check -- left the suite green (Codex round 3, finding
    /// 6). This one stands in for the cache reader: it consults the switch
    /// while "reconstructing" and gives up when told to.
    struct InterruptibleProvider {
        hours: Vec<i64>,
        switch: Mutex<Option<Arc<dyn Fn() -> bool + Send + Sync>>>,
        /// Set the first time a resolve was abandoned because the switch said
        /// so.
        abandoned: AtomicBool,
        /// WHICH hours were abandoned. `abandoned` alone cannot tell an
        /// interrupted EAGER step from an interrupted requested one, and the
        /// eager rule is what one of these tests is about (Codex round 5,
        /// finding 3).
        abandoned_hours: Mutex<Vec<i64>>,
        /// How long one hour pretends to take, in switch consultations.
        steps: usize,
    }

    impl OwnerProvider for InterruptibleProvider {
        fn hours(&self) -> &[i64] {
            &self.hours
        }
        fn set_scrub_interrupt(&self, interrupt: Option<Arc<dyn Fn() -> bool + Send + Sync>>) {
            if let Ok(mut held) = self.switch.lock() {
                *held = interrupt;
            }
        }
        fn owners_at(&self, hour: i64) -> crate::Result<Arc<BTreeMap<u32, String>>> {
            let switch = self.switch.lock().ok().and_then(|held| held.clone());
            for _ in 0..self.steps {
                std::thread::sleep(Duration::from_millis(1));
                if let Some(switch) = &switch {
                    if !switch() {
                        self.abandoned.store(true, Ordering::SeqCst);
                        if let Ok(mut seen) = self.abandoned_hours.lock() {
                            seen.push(hour);
                        }
                        return Err(format!(
                            "the reconstruction of hour {hour} was abandoned because the                              caller's stop switch asked it to stop"
                        )
                        .into());
                    }
                }
            }
            Ok(Arc::new(BTreeMap::from([(1u32, format!("H{hour}"))])))
        }
        fn controllers_at(&self, hour: i64) -> crate::Result<Arc<BTreeMap<u32, String>>> {
            self.owners_at(hour)
        }
        fn hour_label(&self, hour: i64) -> crate::Result<String> {
            Ok(format!("hour {hour}"))
        }
    }

    /// A seek the user has dragged past must actually STOP, not merely be
    /// discarded when it eventually finishes.
    ///
    /// This is the difference between a responsive window and a video-like
    /// timeline: a cold backward seek inside a 744-hour window walks up to 743
    /// delta records and takes seconds (6,623 ms measured on capture t55), and
    /// until the switch reached it, the newest drag position waited behind a
    /// walk towards a position already dragged past.
    ///
    /// kills: deleting the `set_scrub_interrupt` call in `ScrubEngine::start`,
    /// or any link of the chain that carries it to a reader -- the provider's
    /// switch is then never installed, `abandoned` is never set, and this fails.
    /// It also fails if the switch is installed but never says stop.
    #[test]
    fn a_seek_the_user_dragged_past_is_told_to_stop() {
        let provider = Arc::new(InterruptibleProvider {
            hours: (0..200).collect(),
            switch: Mutex::new(None),
            abandoned: AtomicBool::new(false),
            abandoned_hours: Mutex::new(Vec::new()),
            steps: 400,
        });
        let watcher = Arc::clone(&provider);
        let mut engine = ScrubEngine::start(provider, 0).unwrap();

        // Drag: each new position must be able to cut short the one before.
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut hour = 1;
        while !watcher.abandoned.load(Ordering::SeqCst) {
            engine.show(hour);
            hour = if hour >= 190 { 1 } else { hour + 1 };
            assert!(
                Instant::now() < deadline,
                "no reconstruction was ever told to stop; the stop switch is not reaching the                  provider"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Dropping the engine must give the provider its switch back.
    ///
    /// The closure owns an `Arc<Shared>`; a provider that outlives the engine
    /// while still holding it keeps the whole engine's state alive, and any
    /// later non-scrub caller of that provider would be answering to a stop
    /// switch nobody owns (Codex round 4, findings 1 and 2).
    ///
    /// kills: deleting the `set_scrub_interrupt(None)` after the join in
    /// `Drop`. It does NOT kill deleting the one on the failed-spawn path --
    /// that branch needs `std::thread::Builder::spawn` to fail, which a test
    /// cannot ask for reliably (Codex round 5, finding 1). That line is gated
    /// by `the_failed_spawn_path_gives_the_switch_back_too` instead, which is
    /// weaker on purpose and says so.
    #[test]
    fn dropping_the_engine_gives_the_provider_its_switch_back() {
        let provider = Arc::new(InterruptibleProvider {
            hours: (0..20).collect(),
            switch: Mutex::new(None),
            abandoned: AtomicBool::new(false),
            abandoned_hours: Mutex::new(Vec::new()),
            steps: 1,
        });
        let watcher = Arc::clone(&provider);
        assert!(
            watcher.switch.lock().unwrap().is_none(),
            "a provider starts with no switch"
        );
        {
            let mut engine = ScrubEngine::start(provider, 0).unwrap();
            engine.show(1);
            assert!(
                watcher.switch.lock().unwrap().is_some(),
                "a running engine must have installed its switch"
            );
        }
        assert!(
            watcher.switch.lock().unwrap().is_none(),
            "dropping the engine must uninstall the switch it installed"
        );
    }

    /// The other place the switch must be handed back: a worker that never
    /// started.
    ///
    /// The switch is installed BEFORE the spawn, because the worker must never
    /// run an unguarded reconstruction. A spawn that fails therefore returns no
    /// `ScrubEngine`, so no `Drop` runs, and a retained provider would keep the
    /// closure and its `Arc<Shared>` for ever.
    ///
    /// THIS IS A WEAKER TEST THAN THE ONE ABOVE AND THAT IS DELIBERATE. Making
    /// a thread spawn fail on demand is not something a test can ask for
    /// portably, so this checks the source rather than the behaviour -- the
    /// same idiom `every_link_that_carries_the_stop_switch_to_a_reader_delegates`
    /// uses, and for the same reason: a one-line cleanup on an unreachable-in-
    /// test branch has no observable behaviour of its own.
    ///
    /// kills: deleting the `provider.set_scrub_interrupt(None)` from the
    /// `map_err` on the spawn.
    #[test]
    fn the_failed_spawn_path_gives_the_switch_back_too() {
        let source = include_str!("scrub.rs").replace("
", "
");
        let start = source
            .find("let worker = std::thread::Builder::new()")
            .expect("the spawn must exist; this test's anchor moved");
        let spawn_block = &source[start..];
        let end = spawn_block
            .find("Ok(ScrubEngine {")
            .expect("the spawn block must end before the constructor");
        let spawn_block = &spawn_block[..end];
        assert!(
            spawn_block.contains("provider.set_scrub_interrupt(None)"),
            "a spawn that fails leaves no ScrubEngine and therefore no Drop, so it must hand              the switch back itself or a retained provider keeps the closure for ever"
        );
    }

    /// A refusal the user has already dragged past must not be shown.
    ///
    /// The worker publishes only for the generation it was serving, but the
    /// window can post a NEW request between that publication and the collect
    /// that would read it. Adopting the stale refusal then reported a refusal
    /// of an hour nobody is asking about any more, and made the next frame
    /// re-post the request it had just cancelled (Codex round 3, finding 4).
    ///
    /// kills: deleting the `superseded && matches!(answer, Answer::Refused..)`
    /// filter in `collect` — the status below becomes `Refused { requested: A }`
    /// and the assertion fires.
    #[test]
    fn a_refusal_the_timeline_has_already_moved_past_is_not_shown() {
        const REFUSED: i64 = 7;
        let provider = SlowProvider::new(60, Duration::from_millis(1), Some(REFUSED));
        let counter = Arc::clone(&provider);
        let mut engine = ScrubEngine::start(provider, 0).unwrap();

        // Ask for the hour that refuses, and DO NOT collect the answer: wait
        // until the provider itself says it has been asked, then give the
        // worker a moment to publish the refusal into the slot.
        engine.show(REFUSED);
        // Wait for the REFUSAL, which is the event this test needs to have
        // happened -- not for a resolve counter that the engine's own first
        // hour already advanced past.
        let deadline = Instant::now() + Duration::from_secs(10);
        while counter.refusals.load(Ordering::SeqCst) == 0 {
            assert!(Instant::now() < deadline, "the refusing hour was never attempted");
            std::thread::yield_now();
        }
        // The refusal has happened; give the worker its moment to publish it
        // into the slot, which is a lock acquisition away.
        std::thread::sleep(Duration::from_millis(100));

        // Now move on. This posts a new generation and only then collects, so
        // the refusal in the slot belongs to a superseded request.
        let (_, status) = engine.show(30);
        assert_ne!(
            status,
            ScrubStatus::Refused {
                requested: REFUSED,
                reason: "hour 7 is inside a declared coverage gap".to_string()
            },
            "a refusal for an hour the timeline has moved past must not be adopted"
        );
        assert!(
            !matches!(status, ScrubStatus::Refused { .. }),
            "nothing about hour 30 was refused, but the status says {status:?}"
        );
        // ...and the timeline still arrives where it was actually asked to go.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let (shown, status) = engine.show(30);
            if status == ScrubStatus::Settled {
                assert_eq!(shown.hour, 30);
                break;
            }
            assert!(Instant::now() < deadline, "never converged past the stale refusal");
            std::thread::yield_now();
        }
    }

    /// An eager step that was INTERRUPTED is an untried hour, not a refused one.
    ///
    /// The eager walk gives way to real requests, so it is abandoned routinely.
    /// Recording those abandonments as refusals parked the ordered scan at the
    /// first hour a user happened to drag past, and it never advanced again
    /// (Codex round 3, finding 3).
    ///
    /// kills: removing the `live_generation == generation` guard around the
    /// `eager_stalled_at` store — the frontier below stops moving and the test
    /// times out.
    #[test]
    fn an_interrupted_eager_step_is_not_a_refusal() {
        // Every hour takes many switch consultations, so requests reliably
        // interrupt the eager walk rather than racing past it.
        let provider = Arc::new(InterruptibleProvider {
            hours: (0..200).collect(),
            switch: Mutex::new(None),
            abandoned: AtomicBool::new(false),
            abandoned_hours: Mutex::new(Vec::new()),
            steps: 60,
        });
        let watcher = Arc::clone(&provider);
        let mut engine = ScrubEngine::start(provider, 0).unwrap();

        // INTERRUPT THE EAGER WALK SPECIFICALLY, which needs the worker to be
        // IDLE first. A continuous drag never lets it idle -- there is always a
        // request pending -- so every abandonment is of requested work and the
        // eager rule goes untested (which is how this test read before Codex
        // round 5, finding 3, and what it now proves by failing without this).
        //
        // So: settle each request, let the worker go idle and begin an eager
        // hour, then post the next request while that hour is still running.
        // An eager hour here costs about 60 switch consultations at a
        // millisecond each, so a 20 ms pause lands reliably inside one.
        //
        // Requests step by TWO. Serving an hour also advances the eager
        // frontier (`fetch_max` on a served hour), so the eager walk always
        // picks frontier+1 -- and if requests stepped by one, that eager hour
        // would be the very hour requested next and an abandoned hour could not
        // be told apart from requested work. Stepping by two makes every eager
        // hour ODD and every requested hour EVEN, so an abandoned odd hour is
        // unambiguously an interrupted eager step.
        let requested: Vec<i64> = (0..12).map(|step| 150 + step * 2).collect();
        for hour in &requested {
            let deadline = Instant::now() + Duration::from_secs(30);
            while engine.show(*hour).1 != ScrubStatus::Settled {
                assert!(Instant::now() < deadline, "never settled on hour {hour}");
                std::thread::sleep(Duration::from_millis(2));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let abandoned_eager = watcher
            .abandoned_hours
            .lock()
            .map(|seen| seen.iter().any(|hour| !requested.contains(hour)))
            .unwrap_or(false);
        assert!(
            abandoned_eager,
            "no EAGER step was ever interrupted, so this test proves nothing about the eager              rule; abandoned hours were {:?}",
            watcher.abandoned_hours.lock().map(|seen| seen.clone())
        );

        // Now leave it alone somewhere the eager walk has ALREADY passed, so
        // the frontier can only advance if the eager walk itself is still
        // running. If an interrupted eager step had been recorded as a refusal,
        // the walk is parked at the first hour that was interrupted and this
        // never finishes.
        let parked_at = engine.scanned_through();
        engine.show(1);
        let deadline = Instant::now() + Duration::from_secs(60);
        while engine.scanned_through() <= parked_at {
            engine.show(1);
            assert!(
                Instant::now() < deadline,
                "the eager walk never advanced past {parked_at}; an interrupted step was                  treated as a refusal and parked the ordered scan"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// THE OWNER'S RULING, AS A TEST. Dragging the timeline must not stop the
    /// drawing thread, and what it draws must be a real hour throughout.
    ///
    /// The provider here takes 50 ms an hour — a fifth of what capture t53
    /// costs per forward step, and a hundredth of a deep cold seek — and the
    /// test drags across 40 hours. If `show` were synchronous that is 2
    /// seconds of frozen window; the assertion is that 40 drag positions cost
    /// less than one hour's resolve.
    ///
    /// kills: making `show` wait for its own request. It also kills a `show`
    /// that returns no frame while converging, because every position asserts
    /// a real hour is in hand.
    #[test]
    fn dragging_the_timeline_never_waits_for_the_hour_it_asked_for() {
        let provider = SlowProvider::new(200, Duration::from_millis(50), None);
        let mut engine = ScrubEngine::start(provider, 0).unwrap();

        let start = Instant::now();
        for hour in 1..=40 {
            let (shown, status) = engine.show(hour);
            assert!(
                shown.hour >= 0 && shown.hour < 200,
                "a scrub must always have a real recorded hour to draw"
            );
            if shown.hour != hour {
                assert_eq!(status, ScrubStatus::Converging { requested: hour });
            }
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_millis(50),
            "40 drag positions took {elapsed:?}; the drawing thread is waiting on the worker"
        );
    }

    /// A timeline left alone must arrive, and say that it has.
    ///
    /// kills: dropping the request instead of posting it, and any convergence
    /// rule that leaves `Converging` set after the wanted hour is in hand.
    #[test]
    fn a_settled_request_converges_to_the_hour_that_was_asked_for() {
        let provider = SlowProvider::new(50, Duration::from_millis(5), None);
        let mut engine = ScrubEngine::start(provider, 0).unwrap();

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let (shown, status) = engine.show(30);
            if status == ScrubStatus::Settled {
                assert_eq!(shown.hour, 30);
                assert_eq!(shown.label, "hour 30");
                break;
            }
            assert!(Instant::now() < deadline, "the scrub never converged on hour 30");
            std::thread::yield_now();
        }
    }

    /// A refused hour must be NAMED, and the timeline must stop pretending it
    /// is on its way there.
    ///
    /// This is Codex design review finding 6: without it, a worker that stops
    /// advancing at a bad hour is indistinguishable from a slow one, and the
    /// viewer shows a stale frame for ever with no reason given.
    ///
    /// kills: turning a resolve failure into a silent `continue` on the worker,
    /// or into a `Converging` status on the drawing thread.
    #[test]
    fn a_refused_hour_is_named_and_stops_the_timeline_claiming_to_converge() {
        let provider = SlowProvider::new(50, Duration::from_millis(1), Some(20));
        let mut engine = ScrubEngine::start(provider, 0).unwrap();

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let (shown, status) = engine.show(20);
            if let ScrubStatus::Refused { requested, reason } = &status {
                assert_eq!(*requested, 20);
                assert!(
                    reason.contains("declared coverage gap"),
                    "the refusal must carry the recording's own reason, got {reason:?}"
                );
                // The last good hour is still on screen, and it is a real one.
                assert_ne!(shown.hour, 20);
                break;
            }
            assert!(
                Instant::now() < deadline,
                "a refused hour was never reported; status stayed {status:?}"
            );
            std::thread::yield_now();
        }
    }

    /// The OTHER way a request can never be answered: the thread that would
    /// answer it is gone.
    ///
    /// A recording that refuses an hour reports a reason. A worker that dies
    /// reports nothing at all, and without this the viewer would sit under a
    /// permanent "seeking to hour N" over a frame that will never change —
    /// the silent stale frame `ScrubStatus` exists to make impossible.
    ///
    /// kills: deleting `notice_a_dead_worker`, or calling it only when an
    /// answer WAS collected (there is never an answer in this case).
    #[test]
    fn a_dead_worker_is_reported_instead_of_seeking_for_ever() {
        // The worker dies ON the hour being requested, which is the case that
        // has somebody waiting. (A worker that dies on the eager walk while the
        // timeline is settled is deliberately NOT reported: nothing is pending,
        // the frame on screen is the requested one, and the next request
        // surfaces the death on its own account.)
        let provider = SlowProvider::panicking_at(50, 30);
        let mut engine = ScrubEngine::start(provider, 0).unwrap();

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let (shown, status) = engine.show(30);
            if let ScrubStatus::Refused { requested, reason } = &status {
                assert_eq!(*requested, 30);
                assert!(
                    reason.contains("scrub worker stopped"),
                    "a dead worker must be named as such, got {reason:?}"
                );
                assert_eq!(shown.hour, 0, "the last good hour stays on screen");
                assert_ne!(shown.hour, 30);
                break;
            }
            assert!(
                Instant::now() < deadline,
                "a dead worker was never reported; status stayed {status:?}"
            );
            std::thread::yield_now();
        }
    }

    /// The eager walk must not re-pay a failing resolve on every idle turn.
    ///
    /// kills: removing `eager_stalled_at`. Without it the walk picks the same
    /// refused hour every time it goes idle, so every request the user makes
    /// costs one extra failed resolve for the rest of the session.
    #[test]
    fn the_eager_walk_does_not_retry_the_hour_it_was_refused() {
        // Hour 1 is refused, so the eager walk stalls immediately at it.
        let provider = SlowProvider::new(50, Duration::from_millis(1), Some(1));
        let counter = Arc::clone(&provider);
        let mut engine = ScrubEngine::start(provider, 0).unwrap();

        // Let it discover the stall.
        let deadline = Instant::now() + Duration::from_secs(10);
        while engine.show(0).1 != ScrubStatus::Settled {
            assert!(Instant::now() < deadline, "engine never settled on hour 0");
            std::thread::yield_now();
        }
        std::thread::sleep(Duration::from_millis(50));
        let after_stall = counter.resolved.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(200));
        let later = counter.resolved.load(Ordering::SeqCst);
        assert_eq!(
            after_stall, later,
            "the eager walk kept retrying the hour it was refused"
        );
    }

    /// The eager walk exists to have the ordered F4 scan already done by the
    /// time somebody drags somewhere far away. Left alone, it must advance.
    ///
    /// kills: removing the eager arm from the worker loop — the frontier never
    /// moves and the first far jump pays for the whole walk.
    #[test]
    fn an_idle_timeline_walks_ahead_on_its_own() {
        let provider = SlowProvider::new(100, Duration::from_millis(1), None);
        let engine = ScrubEngine::start(provider, 0).unwrap();

        let deadline = Instant::now() + Duration::from_secs(10);
        while engine.scanned_through() < 20 {
            assert!(
                Instant::now() < deadline,
                "the eager walk did not advance; frontier stuck at {}",
                engine.scanned_through()
            );
            std::thread::yield_now();
        }
    }

    /// Only one hour is resolved at a time and only one bundle is retained by
    /// the drawing thread, so the engine's own memory does not grow with how
    /// hard the timeline is scrubbed.
    ///
    /// kills: queueing requests instead of overwriting the wanted hour — a
    /// 200-position drag would then resolve 200 hours instead of a handful.
    #[test]
    fn a_long_drag_resolves_far_fewer_hours_than_it_requests() {
        let provider = SlowProvider::new(400, Duration::from_millis(5), None);
        let counter = Arc::clone(&provider);
        let mut engine = ScrubEngine::start(provider, 0).unwrap();

        for hour in 1..=300 {
            engine.show(hour);
        }
        // WAIT FOR THE TIMELINE TO ARRIVE BEFORE COUNTING. Reading the counter
        // straight after the drag would pass even against an implementation
        // that queued all 300, because a slow worker has not drained the queue
        // yet -- the test would be measuring the worker's speed, not the
        // request policy (Codex implementation review, finding 4). Settling
        // first means every hour the implementation ever intended to resolve
        // has been resolved by the time the count is taken.
        let deadline = Instant::now() + Duration::from_secs(30);
        while engine.show(300).1 != ScrubStatus::Settled {
            assert!(Instant::now() < deadline, "the drag never converged on hour 300");
            std::thread::yield_now();
        }
        // ...then let any straggler finish, so a queue would have time to show
        // itself rather than being cut off by the assertion below.
        std::thread::sleep(Duration::from_millis(200));
        // Each resolve makes two counted calls (owners + controllers).
        let resolved = counter.resolved.load(Ordering::SeqCst) / 2;
        assert!(
            resolved < 60,
            "a 300-position drag resolved {resolved} hours; requests are being queued rather \
             than superseded"
        );
    }
}
