//! Cross-crate byte-level round trip for the D11 sink-failure contract.
//!
//! The collector-side producer encodes these reports through
//! `compact_contract::sink_failure::encode_sink_failure` (the single-source
//! payload codec). This integration test proves that bytes produced by that
//! exact encoder decode through the recorder-host consumer path
//! (`transport_status::decode_sink_failure`, keyed by the control-transport
//! `MessageKind`), byte-for-byte, across the crate boundary — the guard that the
//! producer in the collector crate and the consumer in the recorder-host crate
//! can never drift into two silently divergent layouts.

use compact_contract::sink_failure::{
    encode_sink_failure, SinkErrorClass, SinkFailureKind, SinkFailureReport, SinkTerminality,
    SinkTopology,
};
use recorder_host::control_transport::MessageKind;
use recorder_host::transport_status::{decode_sink_failure, SinkFailureMessageKind};

fn topology(bits: u8) -> SinkTopology {
    SinkTopology::from_bits(bits).unwrap()
}

/// The setup-failure report a collector produces when the pipe sink fails to
/// open while the file sink opened.
fn producer_setup_report() -> SinkFailureReport {
    SinkFailureReport {
        kind: SinkFailureKind::Setup,
        requested: topology(SinkTopology::FILE | SinkTopology::PIPE),
        opened: topology(SinkTopology::FILE),
        failed: topology(SinkTopology::PIPE),
        terminality: SinkTerminality::AttemptTerminal,
        error_class: SinkErrorClass::AccessDenied,
        win32_error: Some(5),
        last_complete_sequence: None,
        failed_sequence: None,
        file_bytes_committed: 0,
        pipe_bytes_committed: 0,
    }
}

/// The terminal-failure report a collector produces on a mid-stream transport
/// failure after both sinks were open.
fn producer_terminal_report() -> SinkFailureReport {
    SinkFailureReport {
        kind: SinkFailureKind::Terminal,
        requested: topology(SinkTopology::FILE | SinkTopology::PIPE),
        opened: topology(SinkTopology::FILE | SinkTopology::PIPE),
        failed: topology(SinkTopology::PIPE),
        terminality: SinkTerminality::AttemptTerminal,
        error_class: SinkErrorClass::BrokenPipe,
        win32_error: Some(109),
        last_complete_sequence: Some(41),
        failed_sequence: Some(42),
        file_bytes_committed: 8_192,
        pipe_bytes_committed: 7_000,
    }
}

#[test]
fn collector_produced_bytes_decode_through_the_recorder_consumer() {
    for report in [producer_setup_report(), producer_terminal_report()] {
        // Producer side: the collector's exact encoder.
        let payload = encode_sink_failure(&report).unwrap();
        // Consumer side: the recorder framing carries the intrinsic branch as a
        // control MessageKind. Decoding under that kind reproduces the report.
        let message_kind = report.kind.message_kind();
        let decoded = decode_sink_failure(message_kind, &payload).unwrap();
        assert_eq!(decoded, report);
    }
}

#[test]
fn recorder_consumer_rejects_a_setup_payload_delivered_under_the_terminal_kind() {
    // A setup payload carries no stream progress; decoding it under the terminal
    // envelope kind must fail closed rather than fabricate a terminal report.
    let payload = encode_sink_failure(&producer_setup_report()).unwrap();
    assert!(decode_sink_failure(MessageKind::SinkTerminalFailure, &payload).is_err());
}

#[test]
fn recorder_consumer_rejects_a_non_sink_message_kind() {
    let payload = encode_sink_failure(&producer_terminal_report()).unwrap();
    assert!(decode_sink_failure(MessageKind::HeartbeatProof, &payload).is_err());
}
