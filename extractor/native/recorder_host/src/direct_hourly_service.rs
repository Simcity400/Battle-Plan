use crate::control_transport::{
    ControlCancellation, ControlDeadline, DeadlineControlChannel, DeadlineControlIo, EndpointRole,
    ValidatedSessionCapability,
};
use crate::hourly_service::HourlyServiceReceiver;
use compact_contract::hourly_service::HourlyServiceAckKindV1;
use std::io;
use std::time::Duration;

pub const DIRECT_HOURLY_REPORT_BUDGET: Duration = Duration::from_secs(5);

pub struct DataThreadCompletionGuard(ControlCancellation);

impl DataThreadCompletionGuard {
    pub fn new(completion: ControlCancellation) -> Self {
        Self(completion)
    }
}

impl Drop for DataThreadCompletionGuard {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

struct FirstByteIo<'a, I> {
    inner: &'a mut I,
    first: Option<u8>,
    data_completed: &'a ControlCancellation,
}

impl<I: DeadlineControlIo> DeadlineControlIo for FirstByteIo<'_, I> {
    fn send_all_until(
        &mut self,
        bytes: &[u8],
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> io::Result<()> {
        self.inner.send_all_until(bytes, deadline, cancellation)
    }

    fn recv_exact_until(
        &mut self,
        mut bytes: &mut [u8],
        deadline: ControlDeadline,
        _cancellation: &ControlCancellation,
    ) -> io::Result<()> {
        if !bytes.is_empty() {
            if let Some(first) = self.first.take() {
                bytes[0] = first;
                bytes = &mut bytes[1..];
            }
        }
        if bytes.is_empty() {
            return Ok(());
        }
        self.inner
            .recv_exact_until(bytes, deadline, self.data_completed)
    }
}

pub fn receive_direct_hourly_service<I: DeadlineControlIo>(
    io: &mut I,
    admission: ValidatedSessionCapability,
    receiver: &mut HourlyServiceReceiver,
    capture_deadline: ControlDeadline,
    data_completed: &ControlCancellation,
) -> Result<(), String> {
    let result = receive_report(io, admission, receiver, capture_deadline, data_completed);
    if let Err(error) = &result {
        receiver.transport_failed(error);
    }
    result
}

fn receive_report<I: DeadlineControlIo>(
    io: &mut I,
    admission: ValidatedSessionCapability,
    receiver: &mut HourlyServiceReceiver,
    capture_deadline: ControlDeadline,
    data_completed: &ControlCancellation,
) -> Result<(), String> {
    if data_completed.is_cancelled() {
        return Err("direct_hourly_service_missing_before_census: DATA already completed".into());
    }
    if capture_deadline.is_expired() {
        return Err("direct_hourly_service_missing_before_census: capture deadline expired".into());
    }
    let mut first = [0u8; 1];
    io.recv_exact_until(&mut first, capture_deadline, data_completed)
        .map_err(|error| {
            format!("direct_hourly_service_missing_before_census: first byte: {error}")
        })?;
    if data_completed.is_cancelled() || capture_deadline.is_expired() {
        return Err(
            "direct_hourly_service_missing_before_census: first byte exceeded capture custody"
                .into(),
        );
    }
    let report_deadline = ControlDeadline::after(DIRECT_HOURLY_REPORT_BUDGET)
        .map_err(|error| format!("direct_hourly_service_deadline: {error}"))?;
    let report_deadline =
        ControlDeadline::at(report_deadline.instant().min(capture_deadline.instant()));
    let borrowed = FirstByteIo {
        inner: io,
        first: Some(first[0]),
        data_completed,
    };
    let mut channel = DeadlineControlChannel::new(borrowed, EndpointRole::Recorder, admission);
    // The underlying read remains cancellable; completed frame custody is not revoked afterward.
    let received_frame = ControlCancellation::new();
    loop {
        let frame = channel
            .recv_until(report_deadline, &received_frame)
            .map_err(|error| format!("direct_hourly_service_missing_before_census: {error}"))?;
        let ack = receiver
            .receive(&frame)
            .ok_or_else(|| "direct_hourly_service_invalid_or_unexpected_payload".to_string())?;
        let accepted_census = ack.kind == HourlyServiceAckKindV1::Census && ack.accepted_count == 1;
        let bytes = ack
            .encode()
            .map_err(|error| format!("direct_hourly_service_ack_encoding: {error}"))?;
        let ack_cancellation = if accepted_census {
            &received_frame
        } else {
            data_completed
        };
        if let Err(error) = channel.send_until(
            crate::control_transport::MessageKind::HourlyServiceAck,
            &bytes,
            report_deadline,
            ack_cancellation,
        ) {
            let detail = format!("direct_hourly_service_ack_failed: {error}");
            receiver.acknowledge_send_failed(&detail);
            return Err(detail);
        }
        if ack.accepted_count == 0 {
            return Err("direct_hourly_service_payload_rejected".into());
        }
        if accepted_census {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests;
