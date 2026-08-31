use super::{
    CollectorControlClient, DeadlineBridge, DeadlineControlIo, DuplexIoError, MonotonicClock,
    MonotonicDeadline, MonotonicInstant,
};
use compact_contract::control_transport::{
    ControlChannel, EndpointRole, MessageKind, SessionCapability,
};
use compact_contract::hourly_service::{
    HourlyServiceAckKindV1, HourlyServiceAckV1, HourlyServiceBatchV1, HourlyServiceCensusV1,
    HourlyServiceSampleV1, HOURLY_SERVICE_MAX_BATCH_SAMPLES,
};
use std::cell::Cell;
use std::rc::Rc;

pub(super) fn require_deadline<C: MonotonicClock>(
    clock: &C,
    deadline: MonotonicDeadline,
) -> Result<(), String> {
    if deadline.is_expired(clock.now()) {
        Err("hourly_service_deadline".into())
    } else {
        Ok(())
    }
}

pub(super) trait ReportExchange {
    fn now(&self) -> MonotonicInstant;
    fn exchange(
        &mut self,
        kind: MessageKind,
        payload: &[u8],
        ack_kind: HourlyServiceAckKindV1,
        batch_id: u64,
        accepted_count: u16,
        deadline: MonotonicDeadline,
    ) -> Result<(), String>;
}

pub(super) fn send_report(
    exchange: &mut impl ReportExchange,
    samples: &[HourlyServiceSampleV1],
    census: &mut HourlyServiceCensusV1,
    deadline: MonotonicDeadline,
) -> Result<(), String> {
    let transport_start = exchange.now().0;
    census.last_batch_id = 0;
    census.pending_delivery =
        u64::try_from(samples.len()).map_err(|_| "hourly_service_count_overflow")?;
    for (index, chunk) in samples.chunks(HOURLY_SERVICE_MAX_BATCH_SAMPLES).enumerate() {
        if deadline.is_expired(exchange.now()) {
            return Err("hourly_service_deadline".into());
        }
        let batch_id = u64::try_from(index)
            .ok()
            .and_then(|n| n.checked_add(1))
            .ok_or("hourly_service_batch_overflow")?;
        let payload = HourlyServiceBatchV1 {
            batch_id,
            samples: chunk.to_vec(),
        }
        .encode()
        .map_err(|error| format!("hourly_service_encode:{error}"))?;
        exchange.exchange(
            MessageKind::HourlyServiceBatch,
            &payload,
            HourlyServiceAckKindV1::Batch,
            batch_id,
            chunk.len() as u16,
            deadline,
        )?;
        census.last_batch_id = batch_id;
        census.pending_delivery = census
            .pending_delivery
            .checked_sub(chunk.len() as u64)
            .ok_or("hourly_service_receipt_underflow")?;
    }
    let transport_ns = exchange
        .now()
        .0
        .checked_sub(transport_start)
        .and_then(|ns| u64::try_from(ns).ok())
        .ok_or("hourly_service_transport_clock")?;
    census.diagnostic_transport_ns = census
        .diagnostic_transport_ns
        .checked_add(transport_ns)
        .ok_or("hourly_service_transport_total_overflow")?;
    let payload = census
        .encode()
        .map_err(|error| format!("hourly_service_census_encode:{error}"))?;
    exchange.exchange(
        MessageKind::HourlyServiceCensus,
        &payload,
        HourlyServiceAckKindV1::Census,
        census.last_batch_id,
        1,
        deadline,
    )
}

impl<T: DeadlineControlIo, C: MonotonicClock> ReportExchange for CollectorControlClient<T, C> {
    fn now(&self) -> MonotonicInstant {
        self.clock.now()
    }
    fn exchange(
        &mut self,
        kind: MessageKind,
        payload: &[u8],
        ack_kind: HourlyServiceAckKindV1,
        batch_id: u64,
        accepted_count: u16,
        deadline: MonotonicDeadline,
    ) -> Result<(), String> {
        require_deadline(&self.clock, deadline)?;
        self.send_frame(kind, payload, deadline)
            .map_err(|error| format!("hourly_service_send:{error:?}"))?;
        require_deadline(&self.clock, deadline)?;
        self.receive_hourly_service_ack(ack_kind, batch_id, accepted_count, deadline)
    }
}

#[derive(Default)]
pub(super) enum DirectAuthority {
    #[default]
    Unbound,
    Bound(Option<[u8; 16]>),
    Used,
}

impl DirectAuthority {
    pub(super) fn authorize(&mut self, session_id: Option<[u8; 16]>) {
        *self = if matches!(self, Self::Unbound) {
            Self::Bound(session_id)
        } else {
            Self::Used
        };
    }

    pub(super) fn take(&mut self) -> Result<SessionCapability, String> {
        let session_id = match std::mem::replace(self, Self::Used) {
            Self::Bound(Some(session_id)) => session_id,
            Self::Bound(None) => return Err("hourly_service_direct_session_invalid".into()),
            Self::Unbound => return Err("hourly_service_direct_not_authorized".into()),
            Self::Used => return Err("hourly_service_direct_already_used".into()),
        };
        compact_contract::control_transport::claim_session_capability(session_id)
            .map_err(|error| format!("hourly_service_direct_session:{error}"))
    }
}

pub(super) struct BorrowedIo<'a, T: DeadlineControlIo>(pub(super) &'a mut T);
impl<T: DeadlineControlIo> DeadlineControlIo for BorrowedIo<'_, T> {
    fn send_all(&mut self, bytes: &[u8], deadline: MonotonicDeadline) -> Result<(), DuplexIoError> {
        self.0.send_all(bytes, deadline)
    }
    fn recv_exact(
        &mut self,
        bytes: &mut [u8],
        deadline: MonotonicDeadline,
    ) -> Result<(), DuplexIoError> {
        self.0.recv_exact(bytes, deadline)
    }
}

pub(super) struct DirectClient<T: DeadlineControlIo, C: MonotonicClock> {
    channel: ControlChannel<DeadlineBridge<T>>,
    clock: C,
    deadline: Rc<Cell<MonotonicDeadline>>,
    attempted: bool,
}

impl<T: DeadlineControlIo, C: MonotonicClock> DirectClient<T, C> {
    pub(super) fn new(io: T, clock: C, session: SessionCapability) -> Self {
        let deadline = Rc::new(Cell::new(MonotonicDeadline::from_now(
            &clock,
            std::time::Duration::ZERO,
        )));
        let bridge = DeadlineBridge {
            inner: io,
            deadline: Rc::clone(&deadline),
        };
        Self {
            channel: ControlChannel::new(bridge, EndpointRole::Collector, session),
            clock,
            deadline,
            attempted: false,
        }
    }

    pub(super) fn send_report(
        &mut self,
        samples: &[HourlyServiceSampleV1],
        census: &mut HourlyServiceCensusV1,
        deadline: MonotonicDeadline,
    ) -> Result<(), String> {
        if std::mem::replace(&mut self.attempted, true) {
            return Err("hourly_service_direct_already_used".into());
        }
        send_report(self, samples, census, deadline)
    }
}

impl<T: DeadlineControlIo, C: MonotonicClock> ReportExchange for DirectClient<T, C> {
    fn now(&self) -> MonotonicInstant {
        self.clock.now()
    }
    fn exchange(
        &mut self,
        kind: MessageKind,
        payload: &[u8],
        ack_kind: HourlyServiceAckKindV1,
        batch_id: u64,
        accepted_count: u16,
        deadline: MonotonicDeadline,
    ) -> Result<(), String> {
        require_deadline(&self.clock, deadline)?;
        if !matches!(
            kind,
            MessageKind::HourlyServiceBatch | MessageKind::HourlyServiceCensus
        ) {
            return Err("hourly_service_direct_send_kind".into());
        }
        self.deadline.set(deadline);
        self.channel
            .send(kind, payload)
            .map_err(|error| format!("hourly_service_direct_send:{error}"))?;
        require_deadline(&self.clock, deadline)?;
        let frame = self
            .channel
            .recv()
            .map_err(|error| format!("hourly_service_direct_receipt:{error}"))?;
        require_deadline(&self.clock, deadline)?;
        if frame.kind != MessageKind::HourlyServiceAck {
            return Err(format!(
                "hourly_service_direct_receipt_kind:{:?}",
                frame.kind
            ));
        }
        let ack = HourlyServiceAckV1::decode(&frame.payload)
            .map_err(|error| format!("hourly_service_direct_receipt_codec:{error}"))?;
        if ack.kind != ack_kind || ack.batch_id != batch_id || ack.accepted_count != accepted_count
        {
            return Err("hourly_service_direct_receipt_binding".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::control_transport::{decode_frame, encode_frame, SessionId};
    use compact_contract::hourly_service::{HourlyServiceOutcomeV1, HourlyServiceOwnerV1};
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    #[derive(Clone)]
    struct Clock(Rc<Cell<u128>>);
    impl MonotonicClock for Clock {
        fn now(&self) -> MonotonicInstant {
            MonotonicInstant(self.0.get())
        }
    }
    struct Io {
        inbound: VecDeque<u8>,
        writes: Rc<RefCell<Vec<Vec<u8>>>>,
        clock: Clock,
        advance_on_read: u128,
    }
    impl DeadlineControlIo for Io {
        fn send_all(&mut self, bytes: &[u8], _: MonotonicDeadline) -> Result<(), DuplexIoError> {
            self.writes.borrow_mut().push(bytes.to_vec());
            Ok(())
        }
        fn recv_exact(
            &mut self,
            bytes: &mut [u8],
            _: MonotonicDeadline,
        ) -> Result<(), DuplexIoError> {
            if self.inbound.len() < bytes.len() {
                return Err(DuplexIoError::PeerClosed);
            }
            for byte in bytes {
                *byte = self.inbound.pop_front().unwrap();
            }
            self.clock.0.set(self.clock.0.get() + self.advance_on_read);
            Ok(())
        }
    }
    fn id() -> [u8; 16] {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let mut id = [0xd6; 16];
        id[..8].copy_from_slice(&NEXT.fetch_add(1, Ordering::Relaxed).to_le_bytes());
        id
    }
    fn ack(
        session: [u8; 16],
        sequence: u64,
        kind: HourlyServiceAckKindV1,
        batch_id: u64,
        count: u16,
    ) -> Vec<u8> {
        encode_frame(
            SessionId::from_bytes(session).unwrap(),
            sequence,
            MessageKind::HourlyServiceAck,
            &HourlyServiceAckV1 {
                kind,
                batch_id,
                accepted_count: count,
            }
            .encode()
            .unwrap(),
        )
        .unwrap()
    }
    fn sample(ordinal: u64) -> HourlyServiceSampleV1 {
        HourlyServiceSampleV1 {
            owner: HourlyServiceOwnerV1 {
                logical_stream_id: 7,
                epoch: 3,
                game_hour_raw: 100 + ordinal as i32,
                observation_ordinal: ordinal,
            },
            outcome: HourlyServiceOutcomeV1::Refused,
            data_sequence: None,
            observed_service_ns: Some(16_000_001),
            stage_ns: [Some(16_000_001), None, None, None, None, None, None],
            observed_stages: 1,
            closed_stages: 0x7f,
            not_applicable_stages: 0x7e,
            edge_to_publication_ns: None,
            edge_to_completion_ns: None,
            fault_bits: 0,
        }
    }
    fn client(
        session: [u8; 16],
        incoming: Vec<u8>,
        advance: u128,
    ) -> (
        DirectClient<Io, Clock>,
        Rc<RefCell<Vec<Vec<u8>>>>,
        MonotonicDeadline,
    ) {
        let clock = Clock(Rc::new(Cell::new(0)));
        let deadline = MonotonicDeadline::from_now(&clock, Duration::from_secs(5));
        let writes = Rc::new(RefCell::new(Vec::new()));
        let io = Io {
            inbound: incoming.into(),
            writes: Rc::clone(&writes),
            clock: clock.clone(),
            advance_on_read: advance,
        };
        (
            DirectClient::new(
                io,
                clock,
                compact_contract::control_transport::claim_session_capability(session).unwrap(),
            ),
            writes,
            deadline,
        )
    }

    #[test]
    fn direct_report_keeps_original_owners_and_sends_closed_census_at_sequence_zero() {
        let session = id();
        let mut inbound = ack(session, 0, HourlyServiceAckKindV1::Batch, 1, 16);
        inbound.extend(ack(session, 1, HourlyServiceAckKindV1::Batch, 2, 1));
        inbound.extend(ack(session, 2, HourlyServiceAckKindV1::Census, 2, 1));
        let (mut client, writes, deadline) = client(session, inbound, 0);
        let samples = (0..17).map(sample).collect::<Vec<_>>();
        let mut census = HourlyServiceCensusV1 {
            offered_requests: 17,
            admitted_requests: 17,
            closed_requests: 17,
            queued_measurements: 17,
            complete_measurements: 17,
            refused_requests: 17,
            over_limit_measurements: 17,
            total_service_ns: 17 * 16_000_001,
            maximum_service_ns: Some(16_000_001),
            terminal_lifecycle_ns: Some(0),
            sealed: true,
            ..Default::default()
        };
        client.send_report(&samples, &mut census, deadline).unwrap();
        let frames = writes.borrow();
        assert_eq!(frames.len(), 3);
        let first = decode_frame(&frames[0], SessionId::from_bytes(session).unwrap(), 0).unwrap();
        let second = decode_frame(&frames[1], SessionId::from_bytes(session).unwrap(), 1).unwrap();
        let terminal =
            decode_frame(&frames[2], SessionId::from_bytes(session).unwrap(), 2).unwrap();
        assert_eq!(first.kind, MessageKind::HourlyServiceBatch);
        assert_eq!(
            HourlyServiceBatchV1::decode(&first.payload)
                .unwrap()
                .samples,
            samples[..16]
        );
        assert_eq!(
            HourlyServiceBatchV1::decode(&second.payload)
                .unwrap()
                .samples,
            samples[16..]
        );
        assert_eq!(terminal.kind, MessageKind::HourlyServiceCensus);
        let sent = HourlyServiceCensusV1::decode(&terminal.payload).unwrap();
        assert_eq!(sent, census);
        assert!(sent.sealed);
        assert_eq!(sent.last_batch_id, 2);
        assert_eq!(sent.pending_delivery, 0);
        assert!(client
            .send_report(&[], &mut census, deadline)
            .unwrap_err()
            .contains("already_used"));
        assert_eq!(writes.borrow().len(), 3);
    }

    #[test]
    fn direct_zero_sample_report_still_requires_the_terminal_census_ack() {
        let session = id();
        let (mut client, writes, deadline) = client(
            session,
            ack(session, 0, HourlyServiceAckKindV1::Census, 0, 1),
            0,
        );
        client
            .send_report(&[], &mut HourlyServiceCensusV1::default(), deadline)
            .unwrap();
        let frame = decode_frame(
            &writes.borrow()[0],
            SessionId::from_bytes(session).unwrap(),
            0,
        )
        .unwrap();
        assert_eq!(frame.kind, MessageKind::HourlyServiceCensus);
    }

    #[test]
    fn direct_report_rejects_wrong_session_sequence_ack_kind_and_count() {
        for variant in 0..4 {
            let session = id();
            let wrong = id();
            let incoming = ack(
                if variant == 0 { wrong } else { session },
                if variant == 1 { 1 } else { 0 },
                if variant == 2 {
                    HourlyServiceAckKindV1::Census
                } else {
                    HourlyServiceAckKindV1::Batch
                },
                1,
                if variant == 3 { 0 } else { 1 },
            );
            let (mut client, writes, deadline) = client(session, incoming, 0);
            let mut census = HourlyServiceCensusV1::default();
            assert!(client
                .send_report(&[sample(1)], &mut census, deadline)
                .is_err());
            assert_eq!(census.pending_delivery, 1);
            assert_eq!(census.last_batch_id, 0);
            assert_eq!(writes.borrow().len(), 1);
        }
    }

    #[test]
    fn direct_report_forbids_nonterminal_control_messages() {
        let session = id();
        let incoming = encode_frame(
            SessionId::from_bytes(session).unwrap(),
            0,
            MessageKind::HeartbeatChallenge,
            &[0; 16],
        )
        .unwrap();
        let (mut client, writes, deadline) = client(session, incoming, 0);
        assert!(client
            .send_report(
                &[sample(1)],
                &mut HourlyServiceCensusV1::default(),
                deadline
            )
            .unwrap_err()
            .contains("receipt_kind"));
        assert_eq!(writes.borrow().len(), 1);
    }

    #[test]
    fn direct_report_names_closed_peer_without_consuming_delivery() {
        let session = id();
        let (mut client, writes, deadline) = client(session, Vec::new(), 0);
        let mut census = HourlyServiceCensusV1::default();
        assert!(client
            .send_report(&[sample(1)], &mut census, deadline)
            .unwrap_err()
            .contains("receipt"));
        assert_eq!(census.pending_delivery, 1);
        assert_eq!(writes.borrow().len(), 1);
    }

    #[test]
    fn direct_report_checks_deadline_after_immediately_successful_reads() {
        let session = id();
        let incoming = ack(session, 0, HourlyServiceAckKindV1::Census, 0, 1);
        let (mut client, _, deadline) = client(session, incoming, 5_000_000_001);
        assert_eq!(
            client
                .send_report(&[], &mut HourlyServiceCensusV1::default(), deadline)
                .unwrap_err(),
            "hourly_service_deadline"
        );
    }

    #[test]
    fn ordinary_report_also_bounds_immediately_successful_receipts_without_changing_phase() {
        let session = id();
        let clock = Clock(Rc::new(Cell::new(0)));
        let deadline = MonotonicDeadline::from_now(&clock, Duration::from_secs(5));
        let writes = Rc::new(RefCell::new(Vec::new()));
        let io = Io {
            inbound: ack(session, 0, HourlyServiceAckKindV1::Census, 0, 1).into(),
            writes,
            clock: clock.clone(),
            advance_on_read: 5_000_000_001,
        };
        let permit = super::super::test_duplex_client_permit();
        let mut client = CollectorControlClient::new(
            &permit,
            io,
            clock,
            compact_contract::control_transport::claim_session_capability(session).unwrap(),
            super::super::CollectorIdentity::from_attach(1, 1, 7, [1; 32], [2; 16]),
            super::super::DuplexClientPolicy::DEFAULT,
        );
        client.perform_handshake(&permit, deadline).unwrap();
        assert_eq!(
            client
                .send_hourly_service_report(&[], &mut HourlyServiceCensusV1::default(), deadline)
                .unwrap_err(),
            "hourly_service_deadline"
        );
        assert_eq!(client.phase(), super::super::ClientPhase::Attached);
    }

    #[test]
    fn direct_report_does_not_write_after_an_expired_deadline() {
        let session = id();
        let (mut client, writes, _) = client(session, Vec::new(), 0);
        let expired = MonotonicDeadline::from_now(&client.clock, Duration::ZERO);
        assert_eq!(
            client
                .send_report(&[], &mut HourlyServiceCensusV1::default(), expired)
                .unwrap_err(),
            "hourly_service_deadline"
        );
        assert!(writes.borrow().is_empty());
    }

    #[test]
    fn direct_authority_is_unusable_before_admission_and_consumed_on_error() {
        let mut authority = DirectAuthority::default();
        assert!(authority.take().is_err());
        authority.authorize(Some(id()));
        assert!(authority.take().is_err());
        let mut invalid = DirectAuthority::default();
        invalid.authorize(None);
        assert!(invalid.take().is_err());
        assert!(invalid.take().is_err());
        let mut valid = DirectAuthority::default();
        let session = id();
        valid.authorize(Some(session));
        assert_eq!(*valid.take().unwrap().session_id().as_bytes(), session);
        assert!(valid.take().is_err());
    }
}
