use super::*;
use crate::spool_writer::Writer;
use replay_capture_spool::spool::SegmentSink;
#[cfg(test)]
use replay_capture_spool::spool::recover_prefix;
#[cfg(test)]
use replay_frame::branches::Parentage;
use std::{
    collections::BTreeMap,
    io::Write,
    sync::{
        Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

const ID: &[u8; 36] = b"21b7cc84-3028-4f65-8e0d-adf2f37ca1a6";
#[cfg(test)]
const OTHER: &[u8; 36] = b"31b7cc84-3028-4f65-8e0d-adf2f37ca1a6";

#[derive(Default)]
struct Gate {
    state: Mutex<(bool, bool)>,
    changed: Condvar,
}
impl Gate {
    fn block(&self) {
        let mut state = self.state.lock().unwrap();
        state.0 = true;
        self.changed.notify_all();
        while !state.1 {
            state = self.changed.wait(state).unwrap();
        }
    }
    #[cfg(test)]
    fn entered(&self) {
        let mut state = self.state.lock().unwrap();
        while !state.0 {
            state = self.changed.wait(state).unwrap();
        }
    }
    #[cfg(test)]
    fn release(&self) {
        self.state.lock().unwrap().1 = true;
        self.changed.notify_all();
    }
}
#[cfg(test)]
struct Release(Arc<Gate>);
#[cfg(test)]
impl Drop for Release {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[derive(Default)]
struct Store {
    files: BTreeMap<([u8; 36], u32), Vec<u8>>,
    gate: Option<(i32, Arc<Gate>, bool)>,
    fail_sync: bool,
    fail_commands: bool,
    sync_gate: Option<(Arc<Gate>, bool)>,
    create_gate: Option<(u32, Arc<Gate>, bool)>,
    closed: usize,
    refuse_poll: Option<std::sync::mpsc::Sender<()>>,
}
struct Backend {
    store: Arc<Mutex<Store>>,
    campaign: [u8; 36],
}
struct Sink {
    store: Arc<Mutex<Store>>,
    key: ([u8; 36], u32),
}
impl SegmentBackend for Backend {
    type Sink = Sink;
    fn create(&mut self, index: u32) -> io::Result<Sink> {
        let gate = {
            let mut store = self.store.lock().unwrap();
            if store
                .create_gate
                .as_ref()
                .is_some_and(|(blocked, _, _)| *blocked == index)
            {
                store.create_gate.take()
            } else {
                None
            }
        };
        if let Some((_, gate, fail)) = gate {
            gate.block();
            if fail {
                return Err(io::Error::other("injected segment start failure"));
            }
        }
        let key = (self.campaign, index);
        assert!(
            self.store
                .lock()
                .unwrap()
                .files
                .insert(key, Vec::new())
                .is_none()
        );
        Ok(Sink {
            store: Arc::clone(&self.store),
            key,
        })
    }
}
impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let gate = {
            let mut store = self.store.lock().unwrap();
            let mut input =
                replay_data::codec::Decoder::new(&bytes[replay_capture_spool::spool::RECORD_HEADER_BYTES..]);
            let hour = match Record::decode(&mut input).unwrap() {
                Record::Control(ControlRecord::PlayerCommands { .. })
                    if std::mem::take(&mut store.fail_commands) =>
                {
                    return Err(io::Error::other("injected command write failure"));
                }
                Record::Frame(frame) => Some(frame.hour),
                _ => None,
            };
            if store
                .gate
                .as_ref()
                .is_some_and(|(blocked, _, _)| Some(*blocked) == hour)
            {
                store.gate.take()
            } else {
                None
            }
        };
        if let Some((_, gate, fail)) = gate {
            gate.block();
            if fail {
                return Err(io::Error::other("injected ordered write failure"));
            }
        }
        self.store
            .lock()
            .unwrap()
            .files
            .get_mut(&self.key)
            .unwrap()
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl SegmentSink for Sink {
    fn start_sync(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn poll_sync(&mut self) -> io::Result<bool> {
        let refuse = self.store.lock().unwrap().refuse_poll.take();
        let Some(refused) = refuse else {
            return Ok(true);
        };
        let error = engine_allocation::vec::<u64>(usize::MAX).unwrap_err();
        refused.send(()).unwrap();
        Err(error)
    }
    fn sync_all(&mut self) -> io::Result<()> {
        let gate = self.store.lock().unwrap().sync_gate.take();
        if let Some((gate, fail)) = gate {
            gate.block();
            if fail {
                return Err(io::Error::other("injected segment close failure"));
            }
        }
        if std::mem::take(&mut self.store.lock().unwrap().fail_sync) {
            Err(io::Error::other("injected sync failure"))
        } else {
            Ok(())
        }
    }
}
impl Drop for Sink {
    fn drop(&mut self) {
        self.store.lock().unwrap().closed += 1;
    }
}

fn writer(store: &Arc<Mutex<Store>>) -> Writer {
    let worker = Arc::clone(store);
    Writer::start_with(move |campaign| {
        Ok(Backend {
            store: Arc::clone(&worker),
            campaign: *campaign,
        })
    })
    .unwrap()
}
fn processor() -> (Processor, Arc<Mutex<Store>>) {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let store = Arc::new(Mutex::new(Store::default()));
    let session = session();
    let processor = Processor {
        root: std::env::temp_dir()
            .join("hoi4-spool-writer-synthetic")
            .join(format!(
                "{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )),
        history: PreparedCampaigns::default(),
        session,
        session_verified: false,
        campaigns: Campaigns::new(),
        stats: Statistics::default(),
        unknown_pending: 0,
        suppress_origin: false,
        retry_full: false,
        unpublished_families: 0,
        helpers: Vec::new(),
        backlogged: false,
        comparison: Box::default(),
        writer: writer(&store),
        journal_thread: crate::columnar::JournalThread::start().unwrap(),
        queued: std::collections::VecDeque::new(),
        flight: None,
        writer_error: None,
        command_drain: Default::default(),
        command_pending: Vec::new(),
        command_contexts: Default::default(),
        command_anchors: Default::default(),
        command_ordinals: Default::default(),
        command_definitions: Default::default(),
        command_last_flush: std::time::Instant::now(),
        command_ready: Default::default(),
        command_inflight: Default::default(),
    };
    (processor, store)
}
#[cfg(test)]
fn admit(processor: &mut Processor, campaign: &[u8; 36], load: i64, hour: i32) {
    let prepared =
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { processor
        .prepare_frame(&campaign_frame(campaign, load, hour)) }
        .unwrap();
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { processor.write_prepared(prepared) }.unwrap();
}
#[cfg(test)]
fn replay(store: &Arc<Mutex<Store>>) -> replay::campaign::Replay {
    let segments = store
        .lock()
        .unwrap()
        .files
        .iter()
        .filter(|((campaign, _), _)| campaign == ID)
        .map(|((_, index), bytes)| (*index, recover_prefix(bytes)))
        .collect::<Vec<_>>();
    replay::campaign::Replay::from_recovered(recorder_derive::writers(), ID.to_vec(), &segments).unwrap()
}
/// # Safety
/// Run only in an isolated fixture process with no live attachment or competing allocation-failure
/// fixture. This starts a writer and injects a process-wide allocation failure.
pub unsafe fn idle_sync_refused() {
    let (mut processor, store) = processor();
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { processor.frame(&campaign_frame(ID, 7, 10)) }.unwrap();
    processor.writer.drain().unwrap();
    let (refused, polled) = std::sync::mpsc::channel();
    store.lock().unwrap().refuse_poll = Some(refused);
    polled.recv().unwrap();
}

#[cfg(test)]
fn block(store: &Arc<Mutex<Store>>, hour: i32, fail: bool) -> Release {
    let gate = Arc::new(Gate::default());
    store.lock().unwrap().gate = Some((hour, Arc::clone(&gate), fail));
    Release(gate)
}

#[test]
fn slow_sink_serves_owned_records_until_the_byte_budget_including_inflight_is_full() {
    let (mut processor, store) = processor();
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { processor.frame(&campaign_frame(ID, 7, 10)) }.unwrap();
    let release = block(&store, 11, false);
    admit(&mut processor, ID, 7, 11);
    release.0.entered();
    let bytes = processor.writer.test_bytes();
    assert!(bytes > 0);
    let budget = bytes * 4;
    processor.writer.test_budget(budget);
    assert_eq!(processor.recorded_hours(), 1);
    let blocked = processor.writer.test_backpressure_probe();
    let producer = std::thread::spawn(move || {
        for hour in 12..=32 {
            admit(&mut processor, ID, 7, hour);
        }
        processor
    });
    blocked();
    release.0.release();
    let mut processor = producer.join().unwrap();
    processor.drain_writer().unwrap();
    assert_eq!(processor.recorded_hours(), 23);
    assert_eq!(processor.writer.test_bytes(), 0);
    // SAFETY: This spool's frame and command processing have finished before its processor is
    // closed.
    unsafe { processor.detach() }.unwrap();
}

#[test]
fn failed_write_gaps_all_queued_hours_once_and_restarts_with_a_keyframe() {
    let (mut processor, store) = processor();
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { processor.frame(&campaign_frame(ID, 7, 10)) }.unwrap();
    let release = block(&store, 11, true);
    admit(&mut processor, ID, 7, 11);
    release.0.entered();
    admit(&mut processor, ID, 7, 12);
    admit(&mut processor, ID, 7, 13);
    let campaign = processor
        .campaigns
        .get_mut(std::str::from_utf8(ID).unwrap())
        .unwrap();
    let branch = campaign.tracker.current_branch().unwrap();
    assert_eq!(campaign.tracker.recorded_hours(branch).unwrap(), [10]);
    assert_eq!(campaign.encoder.committed::<tables::Clock>().unwrap().0, 13);
    release.0.release();
    processor.drain_writer().unwrap();
    assert!(
        // SAFETY: The originating spool flushes its own writer before releasing its observer state.
        unsafe { processor.flush() }.is_err()
    );
    let campaign = processor
        .campaigns
        .get_mut(std::str::from_utf8(ID).unwrap())
        .unwrap();
    assert!(!campaign.spool.is_open());
    assert_eq!(campaign.pending.last().unwrap().gaps.len(), 1);
    assert_eq!(
        (
            campaign.pending.last().unwrap().gaps[0].first_hour,
            campaign.pending.last().unwrap().gaps[0].last_hour
        ),
        (11, 13)
    );
    assert_eq!(processor.writer.test_uncommitted_hours(), [11, 12, 13]);
    let restart = block(&store, 14, false);
    admit(&mut processor, ID, 7, 14);
    restart.0.entered();
    assert_eq!(processor.writer.test_uncommitted_hours(), [14]);
    let written_gaps = store.lock().unwrap().files.values()
        .flat_map(|bytes| recover_prefix(bytes).records)
        .filter(|record| matches!(record, Record::Gap(gap) if gap.first_hour == 11 && gap.last_hour == 13))
        .count();
    assert_eq!(written_gaps, 1);
    restart.0.release();
    processor.drain_writer().unwrap();
    // SAFETY: This spool's frame and command processing have finished before its processor is
    // closed.
    unsafe { processor.detach() }.unwrap();
    let replay = replay(&store);
    assert_eq!(replay.recorded_hours(branch).unwrap(), [10, 14]);
    assert_eq!(
        replay.gaps,
        [Gap {
            branch,
            first_hour: 11,
            last_hour: 13,
            reason: "spool write failed".into()
        }]
    );
    assert_eq!(processor.recorded_hours(), 2);
    let records = store
        .lock()
        .unwrap()
        .files
        .values()
        .flat_map(|bytes| recover_prefix(bytes).records)
        .collect::<Vec<_>>();
    assert!(records.iter().any(
        |record| matches!(record, Record::Frame(frame) if frame.hour == 14 && frame.keyframe)
    ));
}

#[test]
fn later_failures_are_recorded_when_older_writes_complete_at_poll_or_detach() {
    for preparation in [false, true] {
        for detach in [false, true] {
            let (mut processor, store) = processor();
            // SAFETY: This fixture owns its frame and worker state and runs without a live
            // attachment.
            unsafe { processor.frame(&campaign_frame(ID, 7, 8)) }.unwrap();
            let release = block(&store, 9, false);
            admit(&mut processor, ID, 7, 9);
            release.0.entered();
            let prepared =
            // SAFETY: This fixture owns its frame and worker state and runs without a live
            // attachment.
            unsafe { processor.prepare_frame(&campaign_frame(ID, 7, 10)) }.unwrap();
            // SAFETY: This fixture owns its frame and worker state and runs without a live
            // attachment.
            unsafe { processor.write_prepared(prepared) }.unwrap();
            assert_eq!(processor.queued.len(), 2);
            assert_eq!(processor.recorded_hours(), 1);
            let source = campaign_frame(ID, 7, 11);
            let mut prepared =
            // SAFETY: This fixture owns its frame and worker state and runs without a live
            // attachment.
            unsafe { processor.prepare_frame(&source) }.unwrap();
            let branch = prepared.source.branch;
            prepared.source.tables[TableId::Clock.index()] =
                TableView::Complete(TableRows::Clock(vec![ClockRow {
                    hour: 12,
                    ..Default::default()
                }]))
                .into();
            let detail = "Clock: clock row hour differs from frame hour";
            let drain = processor.writer.test_drain_probe();
            let failing = std::thread::spawn(move || {
                let _details = replay_data::codec::error_details_scope();
                let error = if preparation {
                    drop(prepared);
                    processor
                        .finish_preparation(&source, Some(branch), Err(invalid(detail)))
                        .err()
                        .unwrap()
                } else {
                    // SAFETY: This fixture owns its frame and worker state and runs without a live
                    // attachment.
                    unsafe { processor.write_prepared(prepared) }.unwrap_err()
                };
                assert_eq!(error.to_string(), detail);
                if detach {
                    // SAFETY: This spool's frame and command processing have finished before its
                    // processor is closed.
                    unsafe { processor.detach() }.unwrap();
                } else {
                    processor.writer.drain().unwrap();
                    processor.poll_writer().unwrap();
                }
                processor
            });
            drain();
            release.0.release();
            let mut processor = failing.join().unwrap();
            let expected = format!("spool validation failed: {detail}");
            assert_eq!(processor.recorded_hours(), 3);
            assert!(processor.queued.is_empty());
            assert_eq!(processor.json()["write_errors"], 1);
            assert_eq!(processor.json()["last_error"], expected);
            // SAFETY: This spool's frame and command processing have finished before its processor
            // is closed.
            unsafe { processor.detach() }.unwrap();
            let replay = replay(&store);
            assert_eq!(replay.recorded_hours(branch).unwrap(), [8, 9, 10]);
            assert_eq!(replay.gaps.len(), 1);
            assert_eq!(replay.gaps[0].branch, branch);
            assert_eq!(replay.gaps[0].first_hour, 11);
            assert_eq!(replay.gaps[0].last_hour, 11);
        }
    }
}

#[test]
fn postcommit_sync_failure_does_not_gap_the_committed_frame() {
    let (mut processor, store) = processor();
    store.lock().unwrap().fail_sync = true;
    assert!(
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { processor.frame(&campaign_frame(ID, 7, 10)) }.is_err()
    );
    assert_eq!(processor.recorded_hours(), 1);
    assert!(processor.campaigns.values().all(|campaign| {
        campaign
            .pending
            .iter()
            .all(|pending| pending.gaps.is_empty())
    }));
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { processor.frame(&campaign_frame(ID, 7, 11)) }.unwrap();
    assert_eq!(processor.stats.errors, 1);
    // SAFETY: This spool's frame and command processing have finished before its processor is
    // closed.
    unsafe { processor.detach() }.unwrap();
    let replay = replay(&store);
    assert!(replay.gaps.is_empty());
    assert_eq!(
        replay.recorded_hours(replay.branches[0].id).unwrap(),
        [10, 11]
    );
}

#[cfg(test)]
fn close(processor: &mut Processor) -> crate::spool_writer::CloseReceipt {
    match processor
        .begin_history_request(&format!("1 close {} 7", std::str::from_utf8(ID).unwrap()))
    {
        HistoryReply::Closing { receipt, .. } => receipt,
        HistoryReply::Ready(reply) => panic!("unexpected immediate close response: {reply}"),
    }
}

#[test]
fn campaign_and_live_branch_transitions_service_frames_without_draining() {
    for campaign_switch in [false, true] {
        let (mut processor, store) = processor();
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { processor.frame(&campaign_frame(ID, 7, 10)) }.unwrap();
        let release = block(&store, 11, false);
        admit(&mut processor, ID, 7, 11);
        release.0.entered();
        let campaign = if campaign_switch { OTHER } else { ID };
        let load = if campaign_switch { 8 } else { 7 };
        admit(&mut processor, campaign, load, 20);
        admit(&mut processor, campaign, load, 21);
        assert_eq!(processor.queued.len(), 3);
        assert_eq!(processor.recorded_hours(), 1);
        assert_eq!(store.lock().unwrap().files.len(), 1);
        release.0.release();
        processor.drain_writer().unwrap();
        assert_eq!(processor.recorded_hours(), 4);
        // SAFETY: This spool's frame and command processing have finished before its processor is
        // closed.
        unsafe { processor.detach() }.unwrap();
    }
}

#[cfg(test)]
fn branch_ids(processor: &mut Processor) -> Vec<BranchId> {
    let id = std::str::from_utf8(ID).unwrap();
    let campaign = processor.campaigns.get_mut(id).unwrap();
    campaign.tracker.branches().map(|start| start.id).collect()
}

#[cfg(test)]
fn parent(processor: &mut Processor, branch: BranchId) -> Parentage {
    let id = std::str::from_utf8(ID).unwrap();
    let campaign = processor.campaigns.get_mut(id).unwrap();
    campaign
        .tracker
        .branch_start(branch)
        .unwrap()
        .parent
        .clone()
}

#[cfg(test)]
fn written_parent(written: &replay::campaign::Replay, branch: BranchId) -> Parentage {
    let start = written.branches.iter().find(|start| start.id == branch);
    start.unwrap().parent.clone()
}

#[test]
fn a_load_before_its_hours_write_settles_takes_ancestry_from_that_writes_outcome() {
    for fail in [false, true] {
        let (mut processor, store) = processor();
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { processor.frame(&campaign_frame(ID, 7, 10)) }.unwrap();
        let release = block(&store, 11, fail);
        admit(&mut processor, ID, 7, 11);
        release.0.entered();
        admit(&mut processor, ID, 8, 11);
        let branches = branch_ids(&mut processor);
        let &[first, loaded] = branches.as_slice() else {
            panic!("expected the original and the loaded branch");
        };
        let original = Parentage::Probable {
            branch: first,
            hour: 11,
        };
        assert_eq!(processor.queued.len(), 2);
        assert_eq!(parent(&mut processor, loaded), original);
        release.0.release();
        processor.drain_writer().unwrap();
        if fail {
            assert!(
                // SAFETY: The originating spool flushes its own writer before releasing its observer
                // state.
                unsafe { processor.flush() }.is_err()
            );
            assert_eq!(parent(&mut processor, loaded), Parentage::Unknown);
            admit(&mut processor, ID, 8, 12);
            processor.drain_writer().unwrap();
        }
        // SAFETY: This spool's frame and command processing have finished before its processor is
        // closed.
        unsafe { processor.detach() }.unwrap();
        let replay = replay(&store);
        assert_eq!(
            written_parent(&replay, loaded),
            if fail { Parentage::Unknown } else { original }
        );
        assert_eq!(
            replay.recorded_hours(first).unwrap(),
            if fail { vec![10] } else { vec![10, 11] }
        );
        assert_eq!(
            replay.recorded_hours(loaded).unwrap(),
            if fail { vec![12] } else { vec![11] }
        );
    }
}

#[test]
fn a_load_while_another_branch_queues_its_hour_stays_ambiguous_unless_that_write_fails() {
    for fail in [false, true] {
        let (mut processor, store) = processor();
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { processor.frame(&campaign_frame(ID, 7, 10)) }.unwrap();
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { processor.frame(&campaign_frame(ID, 7, 11)) }.unwrap();
        let release = block(&store, 11, fail);
        admit(&mut processor, ID, 8, 11);
        release.0.entered();
        admit(&mut processor, ID, 9, 11);
        let branches = branch_ids(&mut processor);
        let &[first, second, third] = branches.as_slice() else {
            panic!("expected three branches");
        };
        let original = Parentage::Probable {
            branch: first,
            hour: 11,
        };
        assert_eq!(processor.queued.len(), 2);
        assert_eq!(parent(&mut processor, second), original);
        assert_eq!(parent(&mut processor, third), Parentage::Unknown);
        release.0.release();
        processor.drain_writer().unwrap();
        if fail {
            assert!(
                // SAFETY: The originating spool flushes its own writer before releasing its observer
                // state.
                unsafe { processor.flush() }.is_err()
            );
            assert_eq!(parent(&mut processor, third), original);
            admit(&mut processor, ID, 9, 12);
            processor.drain_writer().unwrap();
        }
        // SAFETY: This spool's frame and command processing have finished before its processor is
        // closed.
        unsafe { processor.detach() }.unwrap();
        let replay = replay(&store);
        assert_eq!(written_parent(&replay, second), original);
        assert_eq!(
            written_parent(&replay, third),
            if fail { original } else { Parentage::Unknown }
        );
        assert_eq!(replay.recorded_hours(first).unwrap(), [10, 11]);
        assert_eq!(
            replay.recorded_hours(second).unwrap(),
            if fail { vec![] } else { vec![11] }
        );
        assert_eq!(
            replay.recorded_hours(third).unwrap(),
            if fail { vec![12] } else { vec![11] }
        );
    }
}

#[test]
fn checkpoint_close_and_start_service_frames_and_acknowledge_only_real_handle_close() {
    let (mut processor, store) = processor();
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { processor.frame(&campaign_frame(ID, 7, 10)) }.unwrap();
    let first = block(&store, 11, false);
    admit(&mut processor, ID, 7, 11);
    first.0.entered();
    let closing = Release(Arc::new(Gate::default()));
    store.lock().unwrap().sync_gate = Some((Arc::clone(&closing.0), false));
    let receipt = close(&mut processor);
    admit(&mut processor, ID, 7, 12);
    admit(&mut processor, ID, 7, 13);
    assert_eq!(receipt.result(), None);
    first.0.release();
    closing.0.entered();
    admit(&mut processor, ID, 7, 14);
    assert_eq!(receipt.result(), None);
    assert_eq!(store.lock().unwrap().closed, 0);
    assert_eq!(store.lock().unwrap().files.len(), 1);
    closing.0.release();
    processor.drain_writer().unwrap();
    assert_eq!(receipt.result(), Some(Ok(())));
    assert_eq!(processor.recorded_hours(), 5);
    assert_eq!(store.lock().unwrap().closed, 1);
    let store = store.lock().unwrap();
    let second = recover_prefix(&store.files[&(*ID, 1)]);
    assert!(second.damage.is_none());
    assert!(second.records.iter().any(|record| matches!(record,
        Record::Frame(frame) if frame.hour == 12 && frame.keyframe)));
    drop(store);
    // SAFETY: This spool's frame and command processing have finished before its processor is
    // closed.
    unsafe { processor.detach() }.unwrap();
}

#[test]
fn a_queued_close_or_start_failure_gaps_exactly_the_cancelled_suffix() {
    for start_failure in [false, true] {
        let (mut processor, store) = processor();
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { processor.frame(&campaign_frame(ID, 7, 10)) }.unwrap();
        let first = block(&store, 11, false);
        admit(&mut processor, ID, 7, 11);
        first.0.entered();
        let transition = Release(Arc::new(Gate::default()));
        if start_failure {
            store.lock().unwrap().create_gate = Some((1, Arc::clone(&transition.0), true));
        } else {
            store.lock().unwrap().sync_gate = Some((Arc::clone(&transition.0), true));
        }
        let receipt = close(&mut processor);
        admit(&mut processor, ID, 7, 12);
        admit(&mut processor, ID, 7, 13);
        first.0.release();
        transition.0.entered();
        admit(&mut processor, ID, 7, 14);
        assert_eq!(
            receipt.result(),
            if start_failure { Some(Ok(())) } else { None }
        );
        transition.0.release();
        processor.drain_writer().unwrap();
        assert!(
            // SAFETY: The originating spool flushes its own writer before releasing its observer
            // state.
            unsafe { processor.flush() }.is_err()
        );
        assert_eq!(
            receipt.result(),
            if start_failure {
                Some(Ok(()))
            } else {
                Some(Err(io::ErrorKind::Other))
            }
        );
        assert_eq!(processor.recorded_hours(), 2);
        assert_eq!(processor.writer.test_uncommitted_hours(), [12, 13, 14]);
        admit(&mut processor, ID, 7, 15);
        processor.drain_writer().unwrap();
        // SAFETY: This spool's frame and command processing have finished before its processor is
        // closed.
        unsafe { processor.detach() }.unwrap();
        let replay = replay(&store);
        assert_eq!(
            replay
                .gaps
                .iter()
                .map(|gap| (gap.first_hour, gap.last_hour))
                .collect::<Vec<_>>(),
            [(12, 14)]
        );
        assert_eq!(
            replay.recorded_hours(replay.branches[0].id).unwrap(),
            [10, 11, 15]
        );
    }
}

#[test]
fn a_failed_close_retries_its_flushed_gap_without_duplicating_recorded_loss() {
    let (mut processor, store) = processor();
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { processor.frame(&campaign_frame(ID, 7, 10)) }.unwrap();
    let loss = LostObservation {
        campaign_id: campaign_frame(ID, 7, 11).clock.unwrap().campaign_id,
        first_hour: 11,
        last_hour: 12,
        count: 2,
        load_signal: Some(7),
        ..Default::default()
    };
    // SAFETY: The originating spool is submitting a loss from its own capture or isolated fixture.
    unsafe { processor.lost(&loss) }.unwrap();
    let closing = Release(Arc::new(Gate::default()));
    store.lock().unwrap().sync_gate = Some((Arc::clone(&closing.0), true));
    let receipt = close(&mut processor);
    closing.0.entered();
    admit(&mut processor, ID, 7, 13);
    admit(&mut processor, ID, 7, 14);
    closing.0.release();
    processor.drain_writer().unwrap();
    assert!(
        // SAFETY: The originating spool flushes its own writer before releasing its observer state.
        unsafe { processor.flush() }.is_err()
    );
    assert_eq!(receipt.result(), Some(Err(io::ErrorKind::Other)));
    admit(&mut processor, ID, 7, 15);
    processor.drain_writer().unwrap();
    // SAFETY: This spool's frame and command processing have finished before its processor is
    // closed.
    unsafe { processor.detach() }.unwrap();
    let replay = replay(&store);
    assert_eq!(
        replay.recorded_hours(replay.branches[0].id).unwrap(),
        [10, 15]
    );
    assert_eq!(
        replay
            .gaps
            .iter()
            .map(|gap| (gap.first_hour, gap.last_hour))
            .collect::<Vec<_>>(),
        [(11, 12), (13, 14)]
    );
}

#[test]
fn a_failed_start_sync_keeps_its_flushed_keyframe_and_gaps_only_queued_successors() {
    let (mut processor, store) = processor();
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { processor.frame(&campaign_frame(ID, 7, 10)) }.unwrap();
    let starting = Release(Arc::new(Gate::default()));
    store.lock().unwrap().create_gate = Some((1, Arc::clone(&starting.0), false));
    let receipt = close(&mut processor);
    admit(&mut processor, ID, 7, 11);
    starting.0.entered();
    assert_eq!(receipt.result(), Some(Ok(())));
    let syncing = Release(Arc::new(Gate::default()));
    store.lock().unwrap().sync_gate = Some((Arc::clone(&syncing.0), true));
    admit(&mut processor, ID, 7, 12);
    admit(&mut processor, ID, 7, 13);
    starting.0.release();
    syncing.0.entered();
    admit(&mut processor, ID, 7, 14);
    syncing.0.release();
    processor.drain_writer().unwrap();
    assert!(
        // SAFETY: The originating spool flushes its own writer before releasing its observer state.
        unsafe { processor.flush() }.is_err()
    );
    assert_eq!(processor.recorded_hours(), 2);
    assert_eq!(processor.writer.test_uncommitted_hours(), [12, 13, 14]);
    admit(&mut processor, ID, 7, 15);
    processor.drain_writer().unwrap();
    // SAFETY: This spool's frame and command processing have finished before its processor is
    // closed.
    unsafe { processor.detach() }.unwrap();
    let replay = replay(&store);
    assert_eq!(
        replay
            .gaps
            .iter()
            .map(|gap| (gap.first_hour, gap.last_hour))
            .collect::<Vec<_>>(),
        [(12, 14)]
    );
    assert_eq!(
        replay.recorded_hours(replay.branches[0].id).unwrap(),
        [10, 11, 15]
    );
}

#[test]
fn failure_cancels_and_gaps_each_live_transition_context_without_stranding_the_next_campaign() {
    for campaign_switch in [false, true] {
        let (mut processor, store) = processor();
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { processor.frame(&campaign_frame(ID, 7, 10)) }.unwrap();
        let release = block(&store, 11, true);
        admit(&mut processor, ID, 7, 11);
        release.0.entered();
        admit(&mut processor, ID, 7, 12);
        let current = if campaign_switch { OTHER } else { ID };
        admit(&mut processor, current, 8, 20);
        admit(&mut processor, current, 8, 21);
        release.0.release();
        processor.drain_writer().unwrap();
        assert!(
            // SAFETY: The originating spool flushes its own writer before releasing its observer
            // state.
            unsafe { processor.flush() }.is_err()
        );
        assert_eq!(processor.recorded_hours(), 1);
        assert_eq!(processor.writer.test_uncommitted_hours(), [11, 12, 20, 21]);
        admit(&mut processor, current, 8, 22);
        processor.drain_writer().unwrap();
        assert_eq!(processor.recorded_hours(), 2);
        assert!(processor.writer.test_uncommitted_hours().is_empty());
        // SAFETY: This spool's frame and command processing have finished before its processor is
        // closed.
        unsafe { processor.detach() }.unwrap();
        let files = &store.lock().unwrap().files;
        let gaps = files
            .values()
            .flat_map(|bytes| recover_prefix(bytes).records)
            .filter_map(|record| match record {
                Record::Gap(gap) => Some((gap.first_hour, gap.last_hour)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(gaps, [(11, 12), (20, 21)]);
    }
}

#[test]
fn detach_waits_only_after_production_has_stopped() {
    let (mut processor, store) = processor();
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { processor.frame(&campaign_frame(ID, 7, 10)) }.unwrap();
    let release = block(&store, 11, false);
    admit(&mut processor, ID, 7, 11);
    release.0.entered();
    let drain = processor.writer.test_drain_probe();
    let stopped = std::thread::spawn(move || {
        // SAFETY: This spool's frame and command processing have finished before its processor is
        // closed.
        unsafe { processor.detach() }.unwrap();
        processor
    });
    drain();
    assert_eq!(store.lock().unwrap().closed, 0);
    release.0.release();
    let processor = stopped.join().unwrap();
    assert_eq!(processor.recorded_hours(), 2);
    assert_eq!(store.lock().unwrap().closed, 1);
}

#[test]
fn a_failed_range_stays_witnessed_until_written_before_campaign_branch_or_detach() {
    for boundary in 0..3 {
        let (mut processor, store) = processor();
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { processor.frame(&campaign_frame(ID, 7, 10)) }.unwrap();
        let release = block(&store, 11, true);
        admit(&mut processor, ID, 7, 11);
        release.0.entered();
        admit(&mut processor, ID, 7, 12);
        release.0.release();
        processor.drain_writer().unwrap();
        assert_eq!(processor.writer.test_uncommitted_hours(), [11, 12]);
        assert!(
            // SAFETY: The originating spool flushes its own writer before releasing its observer
            // state.
            unsafe { processor.flush() }.is_err()
        );
        match boundary {
            0 => admit(&mut processor, OTHER, 8, 20),
            1 => admit(&mut processor, ID, 8, 20),
            2 =>
            // SAFETY: This spool's frame and command processing have finished before its processor
            // is closed.
            {
                unsafe { processor.detach() }.unwrap()
            }
            _ => unreachable!(),
        }
        processor.drain_writer().unwrap();
        assert!(processor.writer.test_uncommitted_hours().is_empty());
        let records = store
            .lock()
            .unwrap()
            .files
            .iter()
            .filter(|((campaign, _), _)| campaign == ID)
            .flat_map(|(_, bytes)| {
                let recovered = recover_prefix(bytes);
                assert!(recovered.damage.is_none());
                recovered.records
            })
            .collect::<Vec<_>>();
        let gaps = records
            .iter()
            .filter_map(|record| match record {
                Record::Gap(gap) => Some((gap.first_hour, gap.last_hour)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(gaps, [(11, 12)]);
        // SAFETY: This spool's frame and command processing have finished before its processor is
        // closed.
        unsafe { processor.detach() }.unwrap();
    }
}

#[test]
fn conversion_failure_after_a_load_cannot_hide_an_earlier_failed_campaign_range() {
    for detach in [false, true] {
        let (mut processor, store) = processor();
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { processor.frame(&campaign_frame(ID, 7, 10)) }.unwrap();
        let release = block(&store, 11, true);
        admit(&mut processor, ID, 7, 11);
        release.0.entered();
        admit(&mut processor, ID, 7, 12);
        release.0.release();
        processor.drain_writer().unwrap();
        assert!(
            // SAFETY: The originating spool flushes its own writer before releasing its observer
            // state.
            unsafe { processor.flush() }.is_err()
        );
        for (load, hour) in [(8, 20), (9, 30)] {
            let captured = campaign_frame(ID, load, hour);
            let prepared =
            // SAFETY: This fixture owns its frame and worker state and runs without a live
            // attachment.
            unsafe { processor.prepare_frame(&captured) }.unwrap();
            let branch = prepared.source.branch;
            drop(prepared);
            assert!(
                processor
                    .finish_preparation(
                        &captured,
                        Some(branch),
                        Err(invalid(
                            "injected conversion error after observing the branch"
                        )),
                    )
                    .is_err()
            );
        }
        if detach {
            // SAFETY: This spool's frame and command processing have finished before its processor
            // is closed.
            unsafe { processor.detach() }.unwrap();
        } else {
            admit(&mut processor, OTHER, 10, 40);
            processor.drain_writer().unwrap();
            assert_eq!(processor.recorded_hours(), 2);
        }
        assert!(processor.writer.test_uncommitted_hours().is_empty());
        let replay = replay(&store);
        assert_eq!(
            replay
                .gaps
                .iter()
                .map(|gap| (gap.first_hour, gap.last_hour, gap.reason.as_str()))
                .collect::<Vec<_>>(),
            [
                (11, 12, "spool write failed"),
                (20, 20, "spool frame preparation failed"),
                (30, 30, "spool frame preparation failed")
            ],
        );
        // SAFETY: This spool's frame and command processing have finished before its processor is
        // closed.
        unsafe { processor.detach() }.unwrap();
    }
}

#[test]
fn a_write_failure_after_preparing_the_next_branch_restores_context_in_observation_order() {
    let (mut processor, store) = processor();
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { processor.frame(&campaign_frame(ID, 7, 10)) }.unwrap();
    let release = block(&store, 11, true);
    admit(&mut processor, ID, 7, 11);
    release.0.entered();
    let prepared =
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { processor.prepare_frame(&campaign_frame(ID, 8, 20)) }.unwrap();
    let next = prepared.source.branch;
    let id = std::str::from_utf8(ID).unwrap();
    assert_eq!(
        processor.campaigns.get_mut(id).unwrap().pending[0].start.id,
        next
    );
    release.0.release();
    processor.writer.drain().unwrap();
    assert!(
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { processor.write_prepared(prepared) }.is_err()
    );
    assert!(
        // SAFETY: The originating spool flushes its own writer before releasing its observer state.
        unsafe { processor.flush() }.is_err()
    );
    let campaign = processor.campaigns.get_mut(id).unwrap();
    let previous = campaign.tracker.branches().next().unwrap().id;
    assert_eq!(
        campaign
            .pending
            .iter()
            .map(|pending| pending.start.id)
            .collect::<Vec<_>>(),
        [previous, next]
    );
    admit(&mut processor, ID, 8, 21);
    processor.drain_writer().unwrap();
    // SAFETY: This spool's frame and command processing have finished before its processor is
    // closed.
    unsafe { processor.detach() }.unwrap();
    let replay = replay(&store);
    assert_eq!(replay.recorded_hours(previous).unwrap(), [10]);
    assert_eq!(replay.recorded_hours(next).unwrap(), [21]);
    assert_eq!(
        replay
            .gaps
            .iter()
            .map(|gap| (gap.branch, gap.first_hour, gap.last_hour))
            .collect::<Vec<_>>(),
        [(previous, 11, 11), (next, 20, 20)]
    );
    assert!(processor.writer.test_uncommitted_hours().is_empty());
}

#[cfg(test)]
fn large_receipt(processor: &mut Processor, hour: i32, bytes: usize) -> PreparedFrame {
    let mut prepared =
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { processor
        .prepare_frame(&campaign_frame(ID, 7, hour)) }
        .unwrap();
    prepared.event_feed_publication =
        Some(collector_observers::event_feed_history::Prepared::for_test(
            replay_data::rows::EventFeedHistoryRow {
                read_hour: hour,
                failure: Some(vec![b'x'; bytes]),
                ..Default::default()
            },
        ));
    prepared
}

#[test]
fn deferred_observer_payloads_share_the_byte_budget_and_pool_charge_until_applied() {
    const BASELINE: u64 = 1234;
    const PAYLOAD: usize = 8 * 1024 * 1024;
    let counter = engine_allocation::shared(AtomicU64::new(BASELINE)).unwrap();
    let (mut processor, store) = processor();
    processor.account_writer(counter.clone());
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { processor.frame(&campaign_frame(ID, 7, 10)) }.unwrap();
    let release = block(&store, 11, false);
    let prepared = large_receipt(&mut processor, 11, PAYLOAD);
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { processor.write_prepared(prepared) }.unwrap();
    release.0.entered();
    assert!(processor.writer.test_storage().0 >= PAYLOAD);
    let check = processor.writer.test_storage_probe(BASELINE);
    let producer = std::thread::spawn(move || {
        for hour in 12..=20 {
            let prepared = large_receipt(&mut processor, hour, PAYLOAD);
            // SAFETY: This fixture owns its frame and worker state and runs without a live
            // attachment.
            unsafe { processor.write_prepared(prepared) }.unwrap();
        }
        processor
    });
    let (deferred, owned) = check();
    assert!(deferred >= 3 * PAYLOAD);
    assert!(owned >= deferred);
    release.0.release();
    let mut processor = producer.join().unwrap();
    processor.drain_writer().unwrap();
    assert_eq!(processor.recorded_hours(), 11);
    assert_eq!(processor.writer.test_bytes(), 0);
    assert_eq!(processor.writer.test_storage().0, 0);
    assert_eq!(
        counter.load(Ordering::Relaxed),
        BASELINE + processor.writer.test_storage().1 as u64
    );
    // SAFETY: This spool's frame and command processing have finished before its processor is
    // closed.
    unsafe { processor.detach() }.unwrap();
    drop(processor);
    assert_eq!(counter.load(Ordering::Relaxed), BASELINE);
}

#[test]
fn a_real_write_keeps_its_deferred_storage_charged_until_the_spool_applies_the_receipt() {
    for fail in [false, true] {
        let counter = engine_allocation::shared(AtomicU64::new(0)).unwrap();
        let (mut processor, store) = processor();
        processor.account_writer(counter.clone());
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { processor.frame(&campaign_frame(ID, 7, 10)) }.unwrap();
        let release = block(&store, 11, fail);
        let prepared = large_receipt(&mut processor, 11, 1024 * 1024);
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { processor.write_prepared(prepared) }.unwrap();
        release.0.entered();
        let receipt = processor.writer.test_storage().0;
        assert!(receipt >= 1024 * 1024);
        release.0.release();
        processor.writer.drain().unwrap();
        assert_eq!(processor.recorded_hours(), 1);
        assert!(processor.writer.test_bytes() > receipt);
        assert_eq!(
            counter.load(Ordering::Relaxed),
            processor.writer.test_storage().1 as u64
        );
        processor.poll_writer().unwrap();
        assert_eq!(processor.recorded_hours(), if fail { 1 } else { 2 });
        assert_eq!(processor.writer.test_bytes() != 0, fail);
        assert_eq!(processor.writer.test_storage().0, 0);
        if !fail {
            // SAFETY: This spool's frame and command processing have finished before its processor
            // is closed.
            unsafe { processor.detach() }.unwrap();
        }
        drop(processor);
        assert_eq!(counter.load(Ordering::Relaxed), 0);
    }
}

#[test]
fn a_queued_close_keeps_its_pending_context_charged_until_its_receipt_is_applied() {
    for fail in [false, true] {
        let counter = engine_allocation::shared(AtomicU64::new(0)).unwrap();
        let (mut processor, store) = processor();
        processor.account_writer(counter.clone());
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { processor.frame(&campaign_frame(ID, 7, 10)) }.unwrap();
        let id = std::str::from_utf8(ID).unwrap();
        let campaign = processor.campaigns.get_mut(id).unwrap();
        let branch = campaign.tracker.current_branch().unwrap();
        const PAYLOAD: usize = 1024 * 1024;
        campaign.gap(branch, 11, 11, "x".repeat(PAYLOAD)).unwrap();
        let gate = Arc::new(Gate::default());
        let release = Release(Arc::clone(&gate));
        store.lock().unwrap().sync_gate = Some((gate, fail));
        let receipt = processor.close_campaign(id, Some(7)).unwrap();
        release.0.entered();
        assert!(receipt.result().is_none());
        assert!(processor.writer.test_storage().0 >= PAYLOAD);
        release.0.release();
        processor.writer.drain().unwrap();
        assert_eq!(receipt.result().unwrap().is_err(), fail);
        assert!(processor.writer.test_storage().0 >= PAYLOAD);
        assert_eq!(
            counter.load(Ordering::Relaxed),
            processor.writer.test_storage().1 as u64
        );
        processor.poll_writer().unwrap();
        assert_eq!(processor.writer.test_storage().0, 0);
        assert_eq!(processor.writer.test_bytes(), 0);
        assert_eq!(
            processor.campaigns.get_mut(id).unwrap().pending.is_empty(),
            !fail
        );
        drop(receipt);
        drop(processor);
        assert_eq!(counter.load(Ordering::Relaxed), 0);
    }
}

#[test]
fn command_packets_retry_without_losing_first_catalog_or_later_packets() {
    let (mut processor, store) = processor();
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { processor.frame(&campaign_frame(ID, 7, 8)) }.unwrap();
    processor.drain_writer().unwrap();
    let id = std::str::from_utf8(ID).unwrap().to_owned();
    let branch = processor
        .campaigns
        .get_mut(&id)
        .unwrap()
        .tracker
        .current_branch()
        .unwrap();
    for ordinal in 0..3 {
        let mut batch = replay_recording::command_packet::Batch::default();
        if ordinal == 0 {
            batch.kinds.push(replay_recording::command_packet::Kind {
                id: 0,
                token: 0,
                name: "synthetic".into(),
                category: replay_recording::command_packet::Category::Other,
            });
        }
        let record = Record::Control(ControlRecord::PlayerCommands {
            session: processor.session.id,
            packet: replay_recording::command_packet::Packet::pack(
                branch,
                1,
                100,
                ordinal,
                ordinal * 10,
                (ordinal + 1) * 10,
                0,
                &batch,
            )
            .unwrap(),
        });
        processor.command_ready.insert(
            (100, ordinal),
            CommandWrite {
                id: id.clone(),
                world: 100,
                ordinal,
                record,
                attempted: false,
                storage: None,
            },
        );
    }
    store.lock().unwrap().fail_commands = true;
    let _ = processor.submit_commands();
    processor.drain_writer().unwrap();
    assert_eq!(processor.command_ready.len(), 3);
    assert!(processor.command_inflight.is_empty());
    processor.submit_commands().unwrap();
    processor.drain_writer().unwrap();
    assert!(processor.command_ready.is_empty());
    let replay = replay(&store);
    let packets: Vec<_> = replay
        .controls
        .iter()
        .filter_map(|control| match control {
            ControlRecord::PlayerCommands { packet, .. } => Some(packet),
            _ => None,
        })
        .collect();
    assert_eq!(packets.len(), 3);
    assert_eq!(packets[0].unpack().unwrap().kinds.len(), 1);
    assert_eq!(
        packets
            .iter()
            .map(|packet| packet.ordinal)
            .collect::<Vec<_>>(),
        [0, 1, 2]
    );
}

#[test]
fn command_declarations_wait_for_parent_hours_and_remain_immutable_across_retry() {
    let (mut processor, store) = processor();
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { processor.frame(&campaign_frame(ID, 7, 10)) }.unwrap();
    let release = block(&store, 11, true);
    admit(&mut processor, ID, 7, 11);
    release.0.entered();
    admit(&mut processor, ID, 8, 11);
    let id = std::str::from_utf8(ID).unwrap().to_owned();
    let campaign = processor.campaigns.get_mut(&id).unwrap();
    let branch = campaign.tracker.current_branch().unwrap();
    let start = campaign.tracker.branch_start(branch).unwrap().clone();
    assert!(matches!(start.parent, Parentage::Probable { hour: 11, .. }));
    let mut packet = replay_recording::command_packet::Packet::pack(
        branch,
        1,
        100,
        0,
        100,
        200,
        0,
        &replay_recording::command_packet::Batch::default(),
    )
    .unwrap();
    packet.phase = Some(replay_recording::command_packet::Phase {
        start,
        campaign: *ID,
        qpc: 100,
        hour: 11,
        load_signal: 8,
    });
    processor.command_ready.insert(
        (100, 0),
        CommandWrite {
            id: id.clone(),
            world: 100,
            ordinal: 0,
            record: Record::Control(ControlRecord::PlayerCommands {
                session: processor.session.id,
                packet,
            }),
            attempted: false,
            storage: None,
        },
    );
    processor.submit_commands().unwrap();
    assert!(!processor.command_ready[&(100, 0)].attempted);
    assert!(processor.command_inflight.is_empty());
    release.0.release();
    processor.drain_writer().unwrap();
    assert_eq!(parent(&mut processor, branch), Parentage::Unknown);
    let starting = Release(Arc::new(Gate::default()));
    {
        let mut store = store.lock().unwrap();
        let next = store
            .files
            .keys()
            .filter(|(campaign, _)| campaign == ID)
            .count() as u32;
        store.fail_commands = true;
        store.create_gate = Some((next, Arc::clone(&starting.0), false));
    }
    processor.submit_commands().unwrap();
    starting.0.release();
    processor.drain_writer().unwrap();
    assert!(store.lock().unwrap().create_gate.is_none());
    let failed = &processor.command_ready[&(100, 0)];
    assert!(failed.attempted);
    let record = failed.record.clone();
    let Record::Control(ControlRecord::PlayerCommands { packet, .. }) = &record else {
        unreachable!()
    };
    assert_eq!(
        packet.phase.as_ref().unwrap().start.parent,
        Parentage::Unknown
    );
    assert_eq!(processor.writer_error.take(), Some(io::ErrorKind::Other));
    processor.submit_commands().unwrap();
    processor.drain_writer().unwrap();
    admit(&mut processor, ID, 8, 12);
    processor.drain_writer().unwrap();
    // SAFETY: This spool's frame and command processing have finished before its processor is
    // closed.
    unsafe { processor.detach() }.unwrap();
    let rows = replay(&store);
    assert_eq!(written_parent(&rows, branch), Parentage::Unknown);
    let Record::Control(control) = record else {
        unreachable!()
    };
    assert!(rows.controls.contains(&control));
}
