use super::*;
use crate::spool_writer::Writer;
use replay::spool::{SegmentSink, recover_prefix};
use std::{
    collections::BTreeMap,
    io::Write,
    sync::{
        Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

const ID: &[u8; 36] = b"21b7cc84-3028-4f65-8e0d-adf2f37ca1a6";
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
    fn entered(&self) {
        let mut state = self.state.lock().unwrap();
        while !state.0 {
            state = self.changed.wait(state).unwrap();
        }
    }
    fn release(&self) {
        self.state.lock().unwrap().1 = true;
        self.changed.notify_all();
    }
}
struct Release(Arc<Gate>);
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
    sync_gate: Option<(Arc<Gate>, bool)>,
    create_gate: Option<(u32, Arc<Gate>, bool)>,
    closed: usize,
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
                replay_data::codec::Decoder::new(&bytes[replay::spool::RECORD_HEADER_BYTES..]);
            let hour = match Record::decode(&mut input).unwrap() {
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
    Writer::start_with(10_000_000, move |campaign| {
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
    let session = super::tests::session();
    let processor = Processor {
        root: PathBuf::from("captures/spool-writer/synthetic").join(format!(
            "{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )),
        history: PreparedCampaigns::default(),
        session,
        campaigns: Campaigns::new(),
        stats: Statistics::default(),
        unknown_pending: 0,
        suppress_origin: false,
        retry_full: false,
        unpublished_families: 0,
        helper: None,
        second_helper: None,
        backlogged: false,
        comparison: Box::default(),
        writer: writer(&store),
        queued: std::collections::VecDeque::new(),
        flight: None,
        writer_error: None,
    };
    (processor, store)
}
fn admit(processor: &mut Processor, campaign: &[u8; 36], load: i64, hour: i32) {
    let prepared = processor
        .prepare_frame(&campaign_frame(campaign, load, hour))
        .unwrap();
    processor.write_prepared(prepared).unwrap();
}
fn replay(store: &Arc<Mutex<Store>>) -> replay::container::Replay {
    let segments = store
        .lock()
        .unwrap()
        .files
        .iter()
        .filter(|((campaign, _), _)| campaign == ID)
        .map(|((_, index), bytes)| (*index, recover_prefix(bytes)))
        .collect::<Vec<_>>();
    replay::container::Replay::from_recovered(ID.to_vec(), &segments).unwrap()
}
fn block(store: &Arc<Mutex<Store>>, hour: i32, fail: bool) -> Release {
    let gate = Arc::new(Gate::default());
    store.lock().unwrap().gate = Some((hour, Arc::clone(&gate), fail));
    Release(gate)
}

#[test]
fn slow_sink_serves_owned_records_until_the_byte_budget_including_inflight_is_full() {
    let (mut processor, store) = processor();
    processor.frame(&campaign_frame(ID, 7, 10)).unwrap();
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
    let status = serde_json::to_value(processor.writer.diagnostics().queue_status()).unwrap();
    assert!(status["backpressure"]["calls"].as_u64().unwrap() >= 1);
    assert!(status["peak_bytes"].as_u64().unwrap() <= status["budget_bytes"].as_u64().unwrap());
    processor.detach().unwrap();
}

#[test]
fn failed_write_gaps_all_queued_hours_once_and_restarts_with_a_keyframe() {
    let (mut processor, store) = processor();
    processor.frame(&campaign_frame(ID, 7, 10)).unwrap();
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
    assert!(processor.flush().is_err());
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
    processor.detach().unwrap();
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

fn forced_frame(hour: i32, seq: u64) -> Captured {
    let mut source = campaign_frame(ID, 7, hour);
    super::tests::prepare_order_counter_sample(&mut source, hour, seq);
    source.clock.as_mut().unwrap().play_session_started_at = 7;
    source
}

#[test]
fn later_failures_keep_their_witness_when_older_writes_complete_at_poll_or_detach() {
    for preparation in [false, true] {
        for forced in [false, true] {
            for detach in [false, true] {
                let (mut processor, store) = processor();
                processor.frame(&forced_frame(8, 5)).unwrap();
                let release = block(&store, 9, false);
                admit(&mut processor, ID, 7, 9);
                release.0.entered();
                let prepared = processor.prepare_frame(&forced_frame(10, 7)).unwrap();
                processor.write_prepared(prepared).unwrap();
                assert_eq!(processor.queued.len(), 2);
                assert_eq!(processor.recorded_hours(), 1);
                let source = if forced {
                    forced_frame(11, 8)
                } else {
                    campaign_frame(ID, 7, 11)
                };
                let mut prepared = processor.prepare_frame(&source).unwrap();
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
                        processor.write_prepared(prepared).unwrap_err()
                    };
                    assert_eq!(error.to_string(), detail);
                    let after_failure = processor.stats.order_counters.json();
                    if detach {
                        processor.detach().unwrap();
                    } else {
                        processor.writer.drain().unwrap();
                        processor.poll_writer().unwrap();
                    }
                    (processor, after_failure)
                });
                drain();
                release.0.release();
                let (mut processor, after_failure) = failing.join().unwrap();
                let expected = format!("spool validation failed: {detail}");
                for witness in [after_failure, processor.stats.order_counters.json()] {
                    let sample = &witness["sample"];
                    assert_eq!(
                        sample["clock"]["campaign"],
                        std::str::from_utf8(ID).unwrap()
                    );
                    assert_eq!(sample["clock"]["hour"], if forced { 11 } else { 10 });
                    assert_eq!(sample["sequence"], if forced { 8 } else { 7 });
                    assert_eq!(sample["branch"], branch);
                    assert_eq!(sample["countries"][0]["counter"], 3806);
                    assert_eq!(sample["sample_valid"], true);
                    assert_eq!(sample["frame_write_succeeded"], !forced);
                    if forced {
                        assert_eq!(sample["frame_write_error"], expected);
                        assert!(sample["subsequent_io_error"].is_null());
                    } else {
                        assert!(sample["frame_write_error"].is_null());
                        assert_eq!(sample["subsequent_io_error"], expected);
                    }
                    assert_eq!(sample["usable_with_recorded_frame"], false);
                }
                assert_eq!(processor.recorded_hours(), 3);
                assert!(processor.queued.is_empty());
                assert_eq!(processor.json()["write_errors"], 1);
                assert_eq!(processor.json()["last_error"], expected);
                processor.detach().unwrap();
                let replay = replay(&store);
                assert_eq!(replay.recorded_hours(branch).unwrap(), [8, 9, 10]);
                assert_eq!(replay.gaps.len(), 1);
                assert_eq!(replay.gaps[0].branch, branch);
                assert_eq!(replay.gaps[0].first_hour, 11);
                assert_eq!(replay.gaps[0].last_hour, 11);
            }
        }
    }
}

#[test]
fn postcommit_sync_failure_does_not_gap_the_committed_frame() {
    let (mut processor, store) = processor();
    store.lock().unwrap().fail_sync = true;
    assert!(processor.frame(&campaign_frame(ID, 7, 10)).is_err());
    assert_eq!(processor.recorded_hours(), 1);
    assert!(processor.campaigns.values().all(|campaign| {
        campaign
            .pending
            .iter()
            .all(|pending| pending.gaps.is_empty())
    }));
    processor.frame(&campaign_frame(ID, 7, 11)).unwrap();
    assert_eq!(processor.stats.errors, 1);
    processor.detach().unwrap();
    let replay = replay(&store);
    assert!(replay.gaps.is_empty());
    assert_eq!(
        replay.recorded_hours(replay.branches[0].id).unwrap(),
        [10, 11]
    );
}

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
        processor.frame(&campaign_frame(ID, 7, 10)).unwrap();
        let release = block(&store, 11, false);
        admit(&mut processor, ID, 7, 11);
        release.0.entered();
        let campaign = if campaign_switch { OTHER } else { ID };
        // The unchanged load and discontinuous hour exercise a live branch switch.
        let load = if campaign_switch { 8 } else { 7 };
        admit(&mut processor, campaign, load, 20);
        admit(&mut processor, campaign, load, 21);
        assert_eq!(processor.queued.len(), 3);
        assert_eq!(processor.recorded_hours(), 1);
        assert_eq!(store.lock().unwrap().files.len(), 1);
        release.0.release();
        processor.drain_writer().unwrap();
        assert_eq!(processor.recorded_hours(), 4);
        processor.detach().unwrap();
    }
}

#[test]
fn checkpoint_close_and_start_service_frames_and_acknowledge_only_real_handle_close() {
    let (mut processor, store) = processor();
    processor.frame(&campaign_frame(ID, 7, 10)).unwrap();
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
    // Both a real pending close and the following initial keyframe are queued.
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
    processor.detach().unwrap();
}

#[test]
fn a_queued_close_or_start_failure_gaps_exactly_the_cancelled_suffix() {
    for start_failure in [false, true] {
        let (mut processor, store) = processor();
        processor.frame(&campaign_frame(ID, 7, 10)).unwrap();
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
        assert!(processor.flush().is_err());
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
        processor.detach().unwrap();
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
    processor.frame(&campaign_frame(ID, 7, 10)).unwrap();
    let loss = LostObservation {
        campaign_id: campaign_frame(ID, 7, 11).clock.unwrap().campaign_id,
        first_hour: 11,
        last_hour: 12,
        count: 2,
        load_signal: Some(7),
        ..Default::default()
    };
    processor.lost(&loss).unwrap();
    let closing = Release(Arc::new(Gate::default()));
    store.lock().unwrap().sync_gate = Some((Arc::clone(&closing.0), true));
    let receipt = close(&mut processor);
    closing.0.entered();
    admit(&mut processor, ID, 7, 13);
    admit(&mut processor, ID, 7, 14);
    closing.0.release();
    processor.drain_writer().unwrap();
    assert!(processor.flush().is_err());
    assert_eq!(receipt.result(), Some(Err(io::ErrorKind::Other)));
    admit(&mut processor, ID, 7, 15);
    processor.drain_writer().unwrap();
    processor.detach().unwrap();
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
    processor.frame(&campaign_frame(ID, 7, 10)).unwrap();
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
    assert!(processor.flush().is_err());
    assert_eq!(processor.recorded_hours(), 2);
    assert_eq!(processor.writer.test_uncommitted_hours(), [12, 13, 14]);
    admit(&mut processor, ID, 7, 15);
    processor.drain_writer().unwrap();
    processor.detach().unwrap();
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
        processor.frame(&campaign_frame(ID, 7, 10)).unwrap();
        let release = block(&store, 11, true);
        admit(&mut processor, ID, 7, 11);
        release.0.entered();
        admit(&mut processor, ID, 7, 12);
        let current = if campaign_switch { OTHER } else { ID };
        admit(&mut processor, current, 8, 20);
        admit(&mut processor, current, 8, 21);
        release.0.release();
        processor.drain_writer().unwrap();
        assert!(processor.flush().is_err());
        assert_eq!(processor.recorded_hours(), 1);
        assert_eq!(processor.writer.test_uncommitted_hours(), [11, 12, 20, 21]);
        admit(&mut processor, current, 8, 22);
        processor.drain_writer().unwrap();
        assert_eq!(processor.recorded_hours(), 2);
        assert!(processor.writer.test_uncommitted_hours().is_empty());
        processor.detach().unwrap();
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
    processor.frame(&campaign_frame(ID, 7, 10)).unwrap();
    let release = block(&store, 11, false);
    admit(&mut processor, ID, 7, 11);
    release.0.entered();
    let drain = processor.writer.test_drain_probe();
    let stopped = std::thread::spawn(move || {
        processor.detach().unwrap();
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
        processor.frame(&campaign_frame(ID, 7, 10)).unwrap();
        let release = block(&store, 11, true);
        admit(&mut processor, ID, 7, 11);
        release.0.entered();
        admit(&mut processor, ID, 7, 12);
        release.0.release();
        processor.drain_writer().unwrap();
        assert_eq!(processor.writer.test_uncommitted_hours(), [11, 12]);
        assert!(processor.flush().is_err());
        match boundary {
            0 => admit(&mut processor, OTHER, 8, 20),
            1 => admit(&mut processor, ID, 8, 20),
            2 => processor.detach().unwrap(),
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
        processor.detach().unwrap();
    }
}

#[test]
fn conversion_failure_after_a_load_cannot_hide_an_earlier_failed_campaign_range() {
    for detach in [false, true] {
        let (mut processor, store) = processor();
        processor.frame(&campaign_frame(ID, 7, 10)).unwrap();
        let release = block(&store, 11, true);
        admit(&mut processor, ID, 7, 11);
        release.0.entered();
        admit(&mut processor, ID, 7, 12);
        release.0.release();
        processor.drain_writer().unwrap();
        assert!(processor.flush().is_err());
        for (load, hour) in [(8, 20), (9, 30)] {
            let captured = campaign_frame(ID, load, hour);
            let prepared = processor.prepare_frame(&captured).unwrap();
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
            processor.detach().unwrap();
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
        processor.detach().unwrap();
    }
}

#[test]
fn a_write_failure_after_preparing_the_next_branch_restores_context_in_observation_order() {
    let (mut processor, store) = processor();
    processor.frame(&campaign_frame(ID, 7, 10)).unwrap();
    let release = block(&store, 11, true);
    admit(&mut processor, ID, 7, 11);
    release.0.entered();
    let prepared = processor.prepare_frame(&campaign_frame(ID, 8, 20)).unwrap();
    let next = prepared.source.branch;
    let id = std::str::from_utf8(ID).unwrap();
    assert_eq!(
        processor.campaigns.get_mut(id).unwrap().pending[0].start.id,
        next
    );
    release.0.release();
    processor.writer.drain().unwrap();
    assert!(processor.write_prepared(prepared).is_err());
    assert!(processor.flush().is_err());
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
    processor.detach().unwrap();
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

fn large_receipt(processor: &mut Processor, hour: i32, bytes: usize) -> PreparedFrame {
    let mut prepared = processor
        .prepare_frame(&campaign_frame(ID, 7, hour))
        .unwrap();
    prepared.event_feed_publication = Some(crate::event_feed_history::Prepared::for_test(
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
    let counter = Arc::new(AtomicU64::new(BASELINE));
    let (mut processor, store) = processor();
    processor.account_writer(Arc::clone(&counter));
    processor.frame(&campaign_frame(ID, 7, 10)).unwrap();
    let release = block(&store, 11, false);
    let prepared = large_receipt(&mut processor, 11, PAYLOAD);
    processor.write_prepared(prepared).unwrap();
    release.0.entered();
    assert!(processor.writer.test_storage().0 >= PAYLOAD);
    let check = processor.writer.test_storage_probe(BASELINE);
    let producer = std::thread::spawn(move || {
        for hour in 12..=20 {
            let prepared = large_receipt(&mut processor, hour, PAYLOAD);
            processor.write_prepared(prepared).unwrap();
        }
        processor
    });
    let diagnostics = check();
    let status = serde_json::to_value(diagnostics.queue_status()).unwrap();
    assert!(status["deferred_receipt_bytes"].as_u64().unwrap() >= (3 * PAYLOAD) as u64);
    assert!(
        status["owned_bytes"].as_u64().unwrap()
            >= status["deferred_receipt_bytes"].as_u64().unwrap()
    );
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
    processor.detach().unwrap();
    drop(processor);
    assert_eq!(counter.load(Ordering::Relaxed), BASELINE);
}

#[test]
fn a_real_write_keeps_its_deferred_storage_charged_until_the_spool_applies_the_receipt() {
    for fail in [false, true] {
        let counter = Arc::new(AtomicU64::new(0));
        let (mut processor, store) = processor();
        processor.account_writer(Arc::clone(&counter));
        processor.frame(&campaign_frame(ID, 7, 10)).unwrap();
        let release = block(&store, 11, fail);
        let prepared = large_receipt(&mut processor, 11, 1024 * 1024);
        processor.write_prepared(prepared).unwrap();
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
            processor.detach().unwrap();
        }
        drop(processor);
        assert_eq!(counter.load(Ordering::Relaxed), 0);
    }
}

#[test]
fn a_queued_close_keeps_its_pending_context_charged_until_its_receipt_is_applied() {
    for fail in [false, true] {
        let counter = Arc::new(AtomicU64::new(0));
        let (mut processor, store) = processor();
        processor.account_writer(Arc::clone(&counter));
        processor.frame(&campaign_frame(ID, 7, 10)).unwrap();
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
