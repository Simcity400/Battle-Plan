//! Playback's reader of a replay's blocks, which reads the next block ahead.
//!
//! A block holds up to `replay_frame::KEYFRAME_INTERVAL` hours (720), and reading one
//! decompresses and parses hundreds of megabytes: 0.2–2.2 s on the 2,302-hour
//! recording. Playback that read a block only when it reached it stalled at
//! every block boundary. Here playback asks for the next block ahead of time
//! ([`BlockReader::prefetch`]), so it is parsed by the time the decoder gets
//! there.
//!
//! Memory: the reader keeps at most two parsed blocks, the one playback decodes
//! (which playback holds anyway) and the next one, plus one read-ahead in
//! flight: a scrub that asks for another read-ahead while one is in flight
//! starts none, and playback reads that block itself when it gets there. A
//! read-ahead that is no longer the next block when it finishes is discarded,
//! so a seek never leaves a stale block behind. Passed blocks are freed on the
//! pipeline's dropper, not on the decoding thread.

use job_priority::SpawnCarried as _;
use std::sync::MutexGuard;
use std::{
    collections::BTreeSet,
    sync::{Arc, Condvar, Mutex},
    thread,
};

use replay::{container::Parallel, container::Replay};
use replay_frame::frame::Frame;

use crate::retirement::Dropper;

/// A parsed block, or why it could not be read.
pub type Block = Result<Arc<Vec<Frame>>, String>;

pub struct BlockReader {
    replay: Arc<Replay>,
    dropper: Arc<Dropper>,
    state: Mutex<State>,
    changed: Condvar,
    /// Blocks actually read from the replay.
    #[cfg(any(test, feature = "test-support"))]
    pub reads: std::sync::atomic::AtomicUsize,
}

#[derive(Default)]
struct State {
    /// The block playback decodes.
    current: Option<(usize, Block)>,
    /// The block after it, read ahead.
    next: Option<(usize, Block)>,
    /// The block the read-ahead is for; a read-ahead of any other is discarded.
    wanted_next: Option<usize>,
    /// Blocks being read.
    reading: BTreeSet<usize>,
}

impl BlockReader {
    pub fn new(replay: Arc<Replay>, dropper: Arc<Dropper>) -> Arc<Self> {
        Arc::new(Self {
            replay,
            dropper,
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            #[cfg(any(test, feature = "test-support"))]
            reads: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    pub fn replay(&self) -> &Arc<Replay> {
        &self.replay
    }

    /// Frees `value` on the pipeline's dropper thread.
    pub fn drop_later<T: Send + 'static>(&self, value: T) {
        self.dropper.drop_later(value);
    }

    /// Playback's block: the current or read-ahead one, one being read
    /// (waited for), or read now on this thread. It becomes the current block.
    pub fn block(&self, index: usize) -> Block {
        let mut state = self.lock();
        loop {
            if let Some((at, block)) = &state.current
                && *at == index
            {
                return block.clone();
            }
            if state.next.as_ref().is_some_and(|(at, _)| *at == index) {
                let next = state.next.take().expect("matched above");
                let block = next.1.clone();
                let passed = state.current.replace(next);
                state.wanted_next = None;
                drop(state);
                self.drop_later(passed);
                return block;
            }
            if !state.reading.contains(&index) {
                break;
            }
            // A read-ahead of this block is in flight: take it when it lands.
            state.wanted_next = Some(index);
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        state.reading.insert(index);
        // A read-ahead still in flight is for the block before this seek.
        state.wanted_next = None;
        drop(state);
        let guard = Reading {
            reader: self,
            index,
        };
        let block = self.read(index, None);
        std::mem::forget(guard);
        let mut state = self.lock();
        state.reading.remove(&index);
        let mut passed = state.current.replace((index, block.clone()));
        let stale = state
            .next
            .take_if(|(at, _)| *at != index + 1)
            .map(|(_, block)| block);
        // A step back keeps the block it left as the one after.
        if state.next.is_none() && passed.as_ref().is_some_and(|(at, _)| *at == index + 1) {
            state.next = passed.take();
        }
        drop(state);
        self.changed.notify_all();
        self.drop_later((passed, stale));
        block
    }

    /// Starts reading `index` ahead of playback on a background thread, unless
    /// it is kept or any read is in flight. Any other read-ahead is now stale.
    pub fn prefetch(self: &Arc<Self>, index: usize) {
        if index >= self.replay.index.len() {
            return;
        }
        {
            let mut state = self.lock();
            state.wanted_next = Some(index);
            if state.next.as_ref().is_some_and(|(at, _)| *at == index)
                || state.current.as_ref().is_some_and(|(at, _)| *at == index)
                || !state.reading.is_empty()
            {
                return;
            }
            state.reading.insert(index);
        }
        let reader = Arc::clone(self);
        let spawned = thread::Builder::new()
            .name("block read-ahead".into())
            .spawn_carried(move || {
                // Ahead of need: never before the window's or the builders' threads.
                crate::worker_priority::background_priority();
                let guard = Reading {
                    reader: &reader,
                    index,
                };
                let workers = thread::available_parallelism()
                    .map_or(1, |count| count.get() / 2)
                    .max(1);
                let on_worker = crate::worker_priority::background_priority;
                let block = reader.read(
                    index,
                    Some(&Parallel {
                        workers,
                        on_worker: &on_worker,
                    }),
                );
                std::mem::forget(guard);
                reader.finish_ahead(index, block);
            });
        if spawned.is_err() {
            // Playback reads it itself when it gets there.
            self.lock().reading.remove(&index);
            self.changed.notify_all();
        }
    }

    fn read(&self, index: usize, parallel: Option<&Parallel<'_>>) -> Block {
        #[cfg(any(test, feature = "test-support"))]
        self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let Some(block) = self.replay.index.get(index) else {
            return Err(format!("block {index} is not in the replay"));
        };
        let read = match parallel {
            Some(parallel) => self.replay.read_block_parallel(block, parallel),
            None => self.replay.read_block(block),
        };
        read.map(Arc::new)
            .map_err(|error| format!("unreadable block: {error}"))
    }

    /// Keeps a read-ahead when it is still the next block, else frees it.
    fn finish_ahead(&self, index: usize, block: Block) {
        let mut state = self.lock();
        state.reading.remove(&index);
        let stale = if state.wanted_next == Some(index) {
            state.next.replace((index, block)).map(|(_, block)| block)
        } else {
            Some(block)
        };
        drop(state);
        self.changed.notify_all();
        self.drop_later(stale);
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Clears a read's mark if its reader unwinds, so no waiter waits for ever.
struct Reading<'a> {
    reader: &'a BlockReader,
    index: usize,
}

impl Drop for Reading<'_> {
    fn drop(&mut self) {
        self.reader.lock().reading.remove(&self.index);
        self.reader.changed.notify_all();
    }
}

#[cfg(any(test, feature = "test-support"))]
impl BlockReader {
    /// Waits until no read is in flight.
    pub fn settle(&self) {
        let mut state = self.lock();
        while !state.reading.is_empty() {
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    /// The blocks kept: the current one and the read-ahead.
    pub fn kept(&self) -> (Option<usize>, Option<usize>) {
        let state = self.lock();
        (
            state.current.as_ref().map(|(at, _)| *at),
            state.next.as_ref().map(|(at, _)| *at),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::spool::Record;
    use replay_data::{
        identity::{CaptureMode, Identity, Roster, SampleBoundary, SessionStart, Verification},
        rows::{CampaignRow, ClockRow, CountryRow, TableRows},
    };
    use replay_frame::{
        branches::{BranchReason, BranchStart, Parentage},
        frame::{DecodedFrame, FrameEncoder, Metrics, TableView},
    };

    fn replay() -> Arc<Replay> {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation");
        let mut records = vec![
            Record::SessionStart(SessionStart {
                id: 1,
                process_id: 1,
                process_start_time: 1,
                qpc_frequency: 10_000_000,
                sample_boundary: SampleBoundary::EndOfHour,
                capture_mode: CaptureMode::Full,
                identity: Identity {
                    game_version: "synthetic".into(),
                    exe_hash: [1; 32],
                    exe_size: 1,
                    vanilla_hash: [2; 32],
                    dlc: Vec::new(),
                    mods: Vec::new(),
                    coverage_policy: 1,
                    language: "l_english".into(),
                    recorder_version: "test".into(),
                    collector_hash: [3; 32],
                    verification: Verification::Verified {
                        launcher: Roster::default(),
                        memory: Roster::default(),
                    },
                },
            }),
            Record::BranchStart(BranchStart {
                id: 1,
                session: 1,
                loaded_hour: None,
                first_hour: 100,
                parent: Parentage::Unknown,
                reason: BranchReason::SessionStart,
            }),
        ];
        let mut encoder = FrameEncoder::new();
        for hour in [100, 101, 820, 821, 1540, 1541] {
            let mut frame = DecodedFrame {
                branch: 1,
                hour,
                metrics: Metrics::default(),
                tables: std::array::from_fn(|_| {
                    TableView::Unavailable {
                        reason: "outside block fixture".into(),
                    }
                    .into()
                }),
            };
            for rows in [
                TableRows::Campaign(vec![CampaignRow {
                    id: b"blocks".to_vec(),
                }]),
                TableRows::Clock(vec![ClockRow {
                    hour,
                    ..Default::default()
                }]),
                TableRows::Countries(vec![CountryRow {
                    id: 1,
                    canonical_slot: 1,
                    political_power_raw: Some(i64::from(hour)),
                    ..Default::default()
                }]),
            ] {
                let index = rows.table().index();
                frame.tables[index] = TableView::Complete(rows).into();
            }
            records.push(Record::Frame(encoder.encode(frame, false).unwrap()));
        }
        Arc::new(Replay::from_records(b"blocks".to_vec(), &[records]).unwrap())
    }

    #[test]
    fn complete_reader_matches_recorded_payloads() {
        let replay = replay();
        let reader = BlockReader::new(Arc::clone(&replay), Arc::new(Dropper::new().unwrap()));
        for (index, block) in replay.index.iter().enumerate() {
            assert_eq!(
                *reader.block(index).unwrap(),
                replay.read_block(block).unwrap()
            );
        }
    }
}
