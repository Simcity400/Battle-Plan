use compact_contract::sink_failure::{
    encode_sink_failure, SinkErrorClass, SinkFailureKind, SinkFailureReport, SinkTerminality,
    SinkTopology,
};
use recorder_host::control_transport::MessageKind;
use recorder_host::transport_status::{decode_sink_failure, SinkFailureMessageKind};

fn topology(bits: u8) -> SinkTopology {
    SinkTopology::from_bits(bits).unwrap()
}

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
fn recorder_consumer_rejects_a_setup_payload_delivered_under_the_terminal_kind() {
    let payload = encode_sink_failure(&producer_setup_report()).unwrap();
    assert!(decode_sink_failure(MessageKind::SinkTerminalFailure, &payload).is_err());
}
