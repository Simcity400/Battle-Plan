use super::*;
use crate::control_transport::{
    claim_session_capability, decode_frame, encode_frame, ControlCapabilities, ControlPeerIdentity,
    MessageKind, SessionId, CONTROL_HEADER_LEN, CONTROL_MAX_PAYLOAD_LEN,
};
use crate::hourly_service::{HourlyServiceBinding, RECEIPT_FILE_NAME};
use compact_contract::hourly_service::{
    HourlyServiceAckV1, HourlyServiceBatchV1, HourlyServiceCensusV1, HourlyServiceOutcomeV1,
    HourlyServiceOwnerV1, HourlyServiceSampleV1,
};
use serde_json::Value;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

#[derive(Default)]
struct MemoryIo {
    incoming: VecDeque<u8>,
    outgoing: Vec<Vec<u8>>,
    reads: Vec<(usize, Instant)>,
    writes: Vec<Instant>,
    received: usize,
    complete_after: Option<(usize, ControlCancellation)>,
    fail_write: Option<usize>,
    fail_read: Option<(usize, usize, io::ErrorKind)>,
}

impl DeadlineControlIo for MemoryIo {
    fn send_all_until(
        &mut self,
        bytes: &[u8],
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> io::Result<()> {
        if cancellation.is_cancelled() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if deadline.is_expired() {
            return Err(io::ErrorKind::TimedOut.into());
        }
        self.writes.push(deadline.instant());
        if self.fail_write == Some(self.writes.len()) {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        self.outgoing.push(bytes.to_vec());
        Ok(())
    }

    fn recv_exact_until(
        &mut self,
        bytes: &mut [u8],
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> io::Result<()> {
        if cancellation.is_cancelled() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if deadline.is_expired() {
            return Err(io::ErrorKind::TimedOut.into());
        }
        self.reads.push((bytes.len(), deadline.instant()));
        for (offset, byte) in bytes.iter_mut().enumerate() {
            if let Some((call, after, kind)) = self.fail_read {
                if call == self.reads.len() && offset == after {
                    return Err(kind.into());
                }
            }
            *byte = self
                .incoming
                .pop_front()
                .ok_or(io::ErrorKind::UnexpectedEof)?;
            self.received += 1;
        }
        if let Some((after, completion)) = &self.complete_after {
            if self.received >= *after {
                completion.cancel();
            }
        }
        Ok(())
    }
}

struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Harness {
    session: SessionId,
    admission: Option<ValidatedSessionCapability>,
    receiver: Option<HourlyServiceReceiver>,
    binding: HourlyServiceBinding,
    directory: Directory,
}

impl Harness {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let mut bytes = [0xda; 16];
        bytes[8..].copy_from_slice(&id.to_le_bytes());
        let session = SessionId::from_bytes(bytes).unwrap();
        let capability = claim_session_capability(bytes).unwrap();
        let peer = ControlPeerIdentity {
            pid: 41,
            creation_filetime: 99,
        };
        let caps = ControlCapabilities::from_bits(ControlCapabilities::HOURLY_SERVICE).unwrap();
        let admission =
            ValidatedSessionCapability::admit(capability, peer, peer, caps, caps).unwrap();
        let path =
            std::env::temp_dir().join(format!("hoi4-direct-hourly-{}-{id}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        let binding =
            HourlyServiceBinding::new(bytes, peer.pid, peer.creation_filetime, [0x31; 32]);
        let receiver = HourlyServiceReceiver::new(&path.join(RECEIPT_FILE_NAME), binding.clone());
        Self {
            session,
            admission: Some(admission),
            receiver: Some(receiver),
            binding,
            directory: Directory(path),
        }
    }

    fn wire(&self, sequence: u64, kind: MessageKind, payload: &[u8]) -> Vec<u8> {
        encode_frame(self.session, sequence, kind, payload).unwrap()
    }

    fn report(&self) -> Vec<u8> {
        let mut wire = self.wire(
            0,
            MessageKind::HourlyServiceBatch,
            &batch(1).encode().unwrap(),
        );
        wire.extend(self.wire(
            1,
            MessageKind::HourlyServiceCensus,
            &census(1).encode().unwrap(),
        ));
        wire
    }

    fn run(&mut self, io: &mut MemoryIo, completion: &ControlCancellation) -> Result<(), String> {
        self.run_until(
            io,
            completion,
            ControlDeadline::after(Duration::from_secs(30)).unwrap(),
        )
    }

    fn run_until(
        &mut self,
        io: &mut MemoryIo,
        completion: &ControlCancellation,
        deadline: ControlDeadline,
    ) -> Result<(), String> {
        receive_direct_hourly_service(
            io,
            self.admission.take().unwrap(),
            self.receiver.as_mut().unwrap(),
            deadline,
            completion,
        )
    }

    fn rows(&mut self) -> Vec<Value> {
        drop(self.receiver.take());
        std::fs::read_to_string(self.directory.0.join(RECEIPT_FILE_NAME))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

fn sample() -> HourlyServiceSampleV1 {
    HourlyServiceSampleV1 {
        owner: HourlyServiceOwnerV1 {
            logical_stream_id: 0,
            epoch: 0,
            game_hour_raw: 100,
            observation_ordinal: 1,
        },
        outcome: HourlyServiceOutcomeV1::Complete,
        data_sequence: Some((1, 3)),
        observed_service_ns: Some(1),
        stage_ns: [Some(1), Some(0), Some(0), Some(0), Some(0), Some(0), None],
        observed_stages: 0x3f,
        closed_stages: 0x7f,
        not_applicable_stages: 0x40,
        edge_to_publication_ns: Some(1),
        edge_to_completion_ns: Some(1),
        fault_bits: 0,
    }
}

fn batch(id: u64) -> HourlyServiceBatchV1 {
    HourlyServiceBatchV1 {
        batch_id: id,
        samples: vec![sample()],
    }
}

fn census(last_batch_id: u64) -> HourlyServiceCensusV1 {
    HourlyServiceCensusV1 {
        last_batch_id,
        offered_requests: 1,
        admitted_requests: 1,
        closed_requests: 1,
        queued_measurements: 1,
        complete_measurements: 1,
        total_service_ns: 1,
        maximum_service_ns: Some(1),
        sealed: true,
        ..Default::default()
    }
}

fn assert_fault(rows: &[Value]) {
    assert_eq!(rows.last().unwrap()["kind"], "receipt_complete");
    assert_eq!(rows.last().unwrap()["fault"], true);
    assert!(rows.iter().any(|row| row["kind"] == "diagnostic_fault"));
}

#[test]
fn direct_hourly_completion_guard_cancels_on_ok_error_and_panic() {
    for fails in [false, true] {
        let token = ControlCancellation::new();
        let result = (|| -> Result<(), ()> {
            let _guard = DataThreadCompletionGuard::new(token.clone());
            if fails {
                Err(())
            } else {
                Ok(())
            }
        })();
        assert_eq!(result.is_err(), fails);
        assert!(token.is_cancelled());
    }
    let token = ControlCancellation::new();
    let completion = token.clone();
    assert!(std::panic::catch_unwind(move || {
        let _guard = DataThreadCompletionGuard::new(completion);
        panic!("DATA worker fixture");
    })
    .is_err());
    assert!(token.is_cancelled());
}

#[test]
fn direct_hourly_early_data_failure_is_named_without_reading() {
    let mut harness = Harness::new();
    let token = ControlCancellation::new();
    drop(DataThreadCompletionGuard::new(token.clone()));
    let mut io = MemoryIo::default();
    assert!(harness
        .run(&mut io, &token)
        .unwrap_err()
        .contains("missing_before_census"));
    assert!(io.reads.is_empty());
    assert_fault(&harness.rows());
}

#[test]
fn direct_hourly_census_returns_and_acks_before_data_eof() {
    let mut harness = Harness::new();
    let token = ControlCancellation::new();
    let guard = DataThreadCompletionGuard::new(token.clone());
    let mut io = MemoryIo {
        incoming: harness.report().into(),
        ..Default::default()
    };
    harness.run(&mut io, &token).unwrap();
    assert!(!token.is_cancelled());
    assert_eq!(io.outgoing.len(), 2);
    for (sequence, (kind, count)) in [
        (HourlyServiceAckKindV1::Batch, 1),
        (HourlyServiceAckKindV1::Census, 1),
    ]
    .into_iter()
    .enumerate()
    {
        let frame = decode_frame(&io.outgoing[sequence], harness.session, sequence as u64).unwrap();
        assert_eq!(frame.kind, MessageKind::HourlyServiceAck);
        assert_eq!(
            HourlyServiceAckV1::decode(&frame.payload).unwrap(),
            HourlyServiceAckV1 {
                kind,
                batch_id: 1,
                accepted_count: count
            }
        );
    }
    drop(guard);
    let rows = harness.rows();
    assert_eq!(rows.last().unwrap()["census_received"], true);
    assert_eq!(rows.last().unwrap()["fault"], false);
}

#[test]
fn direct_hourly_completed_census_survives_data_completion_before_ack() {
    let mut harness = Harness::new();
    let token = ControlCancellation::new();
    let wire = harness.report();
    let mut io = MemoryIo {
        complete_after: Some((wire.len(), token.clone())),
        incoming: wire.into(),
        ..Default::default()
    };
    harness.run(&mut io, &token).unwrap();
    assert!(token.is_cancelled());
    assert_eq!(io.outgoing.len(), 2);
    assert_eq!(harness.rows().last().unwrap()["fault"], false);
}

#[test]
fn direct_hourly_data_completion_before_census_body_is_missing() {
    let mut harness = Harness::new();
    let token = ControlCancellation::new();
    let wire = harness.wire(
        0,
        MessageKind::HourlyServiceCensus,
        &census(0).encode().unwrap(),
    );
    let mut io = MemoryIo {
        incoming: wire.into(),
        complete_after: Some((CONTROL_HEADER_LEN, token.clone())),
        ..Default::default()
    };
    assert!(harness
        .run(&mut io, &token)
        .unwrap_err()
        .contains("missing_before_census"));
    assert!(io.outgoing.is_empty());
    assert_fault(&harness.rows());
}

#[test]
fn direct_hourly_partial_header_and_body_never_acknowledge() {
    for retained in [1, CONTROL_HEADER_LEN - 1, CONTROL_HEADER_LEN + 7] {
        let mut harness = Harness::new();
        let token = ControlCancellation::new();
        let mut wire = harness.report();
        wire.truncate(retained);
        let mut io = MemoryIo {
            incoming: wire.into(),
            ..Default::default()
        };
        assert!(harness.run(&mut io, &token).is_err());
        assert!(io.outgoing.is_empty());
        assert_fault(&harness.rows());
    }
}

#[test]
fn direct_hourly_partial_body_timeout_does_not_restart_deadline() {
    let mut harness = Harness::new();
    let token = ControlCancellation::new();
    let mut io = MemoryIo {
        incoming: harness.report().into(),
        fail_read: Some((3, 7, io::ErrorKind::TimedOut)),
        ..Default::default()
    };
    assert!(harness.run(&mut io, &token).is_err());
    assert_eq!(io.received, CONTROL_HEADER_LEN + 7);
    assert_eq!(io.reads[1].1, io.reads[2].1);
    assert!(io.outgoing.is_empty());
    assert_fault(&harness.rows());
}

#[test]
fn direct_hourly_wrong_session_and_sequence_are_rejected() {
    for wrong_session in [false, true] {
        let mut harness = Harness::new();
        let token = ControlCancellation::new();
        let session = if wrong_session {
            SessionId::from_bytes([0x19; 16]).unwrap()
        } else {
            harness.session
        };
        let sequence = if wrong_session { 0 } else { 1 };
        let wire = encode_frame(
            session,
            sequence,
            MessageKind::HourlyServiceBatch,
            &batch(1).encode().unwrap(),
        )
        .unwrap();
        let mut io = MemoryIo {
            incoming: wire.into(),
            ..Default::default()
        };
        assert!(harness.run(&mut io, &token).is_err());
        assert!(io.outgoing.is_empty());
        assert_fault(&harness.rows());
    }
}

#[test]
fn direct_hourly_duplicate_control_sequence_rejects_census() {
    let mut harness = Harness::new();
    let token = ControlCancellation::new();
    let mut wire = harness.wire(
        0,
        MessageKind::HourlyServiceBatch,
        &batch(1).encode().unwrap(),
    );
    wire.extend(harness.wire(
        0,
        MessageKind::HourlyServiceCensus,
        &census(1).encode().unwrap(),
    ));
    let mut io = MemoryIo {
        incoming: wire.into(),
        ..Default::default()
    };
    assert!(harness.run(&mut io, &token).is_err());
    assert_eq!(io.outgoing.len(), 1);
    assert_fault(&harness.rows());
}

#[test]
fn direct_hourly_wrong_kind_and_invalid_batch_count_are_rejected() {
    for kind in [MessageKind::CollectorHello, MessageKind::HeartbeatProof] {
        let mut harness = Harness::new();
        let token = ControlCancellation::new();
        let mut io = MemoryIo {
            incoming: harness.wire(0, kind, &[]).into(),
            ..Default::default()
        };
        assert!(harness.run(&mut io, &token).is_err());
        assert!(io.outgoing.is_empty());
        assert_fault(&harness.rows());
    }
    for count in [0u16, 17] {
        let mut harness = Harness::new();
        let token = ControlCancellation::new();
        let mut payload = batch(1).encode().unwrap();
        payload[16..18].copy_from_slice(&count.to_le_bytes());
        let mut io = MemoryIo {
            incoming: harness
                .wire(0, MessageKind::HourlyServiceBatch, &payload)
                .into(),
            ..Default::default()
        };
        assert!(harness.run(&mut io, &token).is_err());
        assert!(io.outgoing.is_empty());
        assert_fault(&harness.rows());
    }
}

#[test]
fn direct_hourly_rejected_census_does_not_ignore_data_completion() {
    for race in [false, true] {
        let mut harness = Harness::new();
        let token = ControlCancellation::new();
        let wire = harness.wire(
            0,
            MessageKind::HourlyServiceCensus,
            &census(2).encode().unwrap(),
        );
        let complete_after = race.then(|| (wire.len(), token.clone()));
        let mut io = MemoryIo {
            incoming: wire.into(),
            complete_after,
            ..Default::default()
        };
        assert!(harness.run(&mut io, &token).is_err());
        if race {
            assert!(io.outgoing.is_empty());
        } else {
            let frame = decode_frame(&io.outgoing[0], harness.session, 0).unwrap();
            assert_eq!(
                HourlyServiceAckV1::decode(&frame.payload)
                    .unwrap()
                    .accepted_count,
                0
            );
        }
        assert_fault(&harness.rows());
    }
}

#[test]
fn direct_hourly_report_uses_one_absolute_deadline_after_first_byte() {
    let mut harness = Harness::new();
    let token = ControlCancellation::new();
    let mut wire = harness.wire(
        0,
        MessageKind::HourlyServiceBatch,
        &batch(1).encode().unwrap(),
    );
    wire.extend(harness.wire(
        1,
        MessageKind::HourlyServiceBatch,
        &batch(2).encode().unwrap(),
    ));
    wire.extend(harness.wire(
        2,
        MessageKind::HourlyServiceCensus,
        &census(2).encode().unwrap(),
    ));
    let mut io = MemoryIo {
        incoming: wire.into(),
        ..Default::default()
    };
    let started = Instant::now();
    let capture = ControlDeadline::after(Duration::from_secs(30)).unwrap();
    harness.run_until(&mut io, &token, capture).unwrap();
    assert_eq!(io.reads[0], (1, capture.instant()));
    let report = io.reads[1].1;
    assert!(report >= started + DIRECT_HOURLY_REPORT_BUDGET);
    assert!(report <= Instant::now() + DIRECT_HOURLY_REPORT_BUDGET);
    assert!(report < capture.instant());
    assert!(io.reads[1..]
        .iter()
        .all(|(_, deadline)| *deadline == report));
    assert!(io.writes.iter().all(|deadline| *deadline == report));
}

#[test]
fn direct_hourly_first_byte_checks_expiry_and_immediate_completion() {
    let mut harness = Harness::new();
    let token = ControlCancellation::new();
    let mut io = MemoryIo::default();
    assert!(harness
        .run_until(&mut io, &token, ControlDeadline::at(Instant::now()))
        .is_err());
    assert!(io.reads.is_empty());
    assert_fault(&harness.rows());

    let mut harness = Harness::new();
    let token = ControlCancellation::new();
    let mut io = MemoryIo {
        incoming: harness.report().into(),
        complete_after: Some((1, token.clone())),
        ..Default::default()
    };
    assert!(harness.run(&mut io, &token).is_err());
    assert_eq!(io.reads.len(), 1);
    assert_fault(&harness.rows());
}

#[test]
fn direct_hourly_oversized_frame_is_rejected_before_body_read() {
    let mut harness = Harness::new();
    let token = ControlCancellation::new();
    let mut wire = harness.wire(0, MessageKind::HourlyServiceBatch, &[]);
    wire[12..16].copy_from_slice(&((CONTROL_MAX_PAYLOAD_LEN as u32) + 1).to_le_bytes());
    let mut io = MemoryIo {
        incoming: wire.into(),
        ..Default::default()
    };
    assert!(harness.run(&mut io, &token).is_err());
    assert_eq!(io.received, CONTROL_HEADER_LEN);
    assert_eq!(io.reads.len(), 2);
    assert!(io.outgoing.is_empty());
    assert_fault(&harness.rows());
}

#[test]
fn direct_hourly_ack_failure_keeps_received_census_but_faults_receipt() {
    let mut harness = Harness::new();
    let token = ControlCancellation::new();
    let mut io = MemoryIo {
        incoming: harness.report().into(),
        fail_write: Some(2),
        ..Default::default()
    };
    assert!(harness
        .run(&mut io, &token)
        .unwrap_err()
        .contains("ack_failed"));
    let rows = harness.rows();
    assert_eq!(rows.last().unwrap()["census_received"], true);
    assert_fault(&rows);
}

#[test]
fn direct_hourly_premature_census_cannot_certify_missing_data() {
    let mut harness = Harness::new();
    let token = ControlCancellation::new();
    let mut io = MemoryIo {
        incoming: harness.report().into(),
        ..Default::default()
    };
    harness.run(&mut io, &token).unwrap();
    harness.rows();
    let raw = harness.directory.0.join("stream.bin");
    std::fs::write(&raw, []).unwrap();
    let report = crate::hourly_service::report_for_capture(
        &harness.directory.0.join(RECEIPT_FILE_NAME),
        &harness.binding,
        &[(0, raw)],
        true,
    );
    assert_eq!(report["timing_population_complete"], false);
    assert_eq!(report["passes_16ms"], false);
}

#[test]
fn direct_hourly_report_deadline_never_extends_capture_deadline() {
    let mut harness = Harness::new();
    let token = ControlCancellation::new();
    let mut io = MemoryIo {
        incoming: harness.report().into(),
        ..Default::default()
    };
    let capture = ControlDeadline::after(Duration::from_millis(500)).unwrap();
    harness.run_until(&mut io, &token, capture).unwrap();
    assert!(io.reads.len() > 1);
    assert_eq!(io.outgoing.len(), 2);
    assert!(io
        .reads
        .iter()
        .all(|(_, deadline)| *deadline == capture.instant()));
    assert!(io
        .writes
        .iter()
        .all(|deadline| *deadline == capture.instant()));
}
