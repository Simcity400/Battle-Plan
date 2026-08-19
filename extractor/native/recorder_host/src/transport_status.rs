use crate::control_transport::MessageKind;

pub use compact_contract::sink_failure::{
    encode_sink_failure, SinkErrorClass, SinkFailureKind, SinkFailureReport, SinkStatusError,
    SinkTerminality, SinkTopology, SINK_FAILURE_PAYLOAD_LEN,
};

pub trait SinkFailureMessageKind: Sized {
    fn message_kind(self) -> MessageKind;
    fn from_message_kind(kind: MessageKind) -> Result<Self, SinkStatusError>;
}

impl SinkFailureMessageKind for SinkFailureKind {
    fn message_kind(self) -> MessageKind {
        match self {
            Self::Setup => MessageKind::SinkSetupFailure,
            Self::Terminal => MessageKind::SinkTerminalFailure,
        }
    }

    fn from_message_kind(kind: MessageKind) -> Result<Self, SinkStatusError> {
        match kind {
            MessageKind::SinkSetupFailure => Ok(Self::Setup),
            MessageKind::SinkTerminalFailure => Ok(Self::Terminal),
            other => Err(SinkStatusError::WrongMessageKind(other as u16)),
        }
    }
}

pub fn decode_sink_failure(
    kind: MessageKind,
    payload: &[u8],
) -> Result<SinkFailureReport, SinkStatusError> {
    let expected = SinkFailureKind::from_message_kind(kind)?;
    compact_contract::sink_failure::decode_sink_failure(expected, payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn topology(bits: u8) -> SinkTopology {
        SinkTopology::from_bits(bits).unwrap()
    }

    fn setup_failure() -> SinkFailureReport {
        SinkFailureReport {
            kind: SinkFailureKind::Setup,
            requested: topology(SinkTopology::FILE | SinkTopology::PIPE),
            opened: topology(SinkTopology::FILE),
            failed: topology(SinkTopology::PIPE),
            terminality: SinkTerminality::AttemptTerminal,
            error_class: SinkErrorClass::Win32,
            win32_error: Some(5),
            last_complete_sequence: None,
            failed_sequence: None,
            file_bytes_committed: 0,
            pipe_bytes_committed: 0,
        }
    }

    fn terminal_failure() -> SinkFailureReport {
        SinkFailureReport {
            kind: SinkFailureKind::Terminal,
            requested: topology(SinkTopology::FILE | SinkTopology::PIPE),
            opened: topology(SinkTopology::FILE | SinkTopology::PIPE),
            failed: topology(SinkTopology::PIPE),
            terminality: SinkTerminality::AttemptTerminal,
            error_class: SinkErrorClass::BrokenPipe,
            win32_error: Some(109),
            last_complete_sequence: Some(8),
            failed_sequence: Some(9),
            file_bytes_committed: 4_096,
            pipe_bytes_committed: 3_100,
        }
    }

    #[test]
    fn message_kind_bridge_round_trips_through_the_payload_codec() {
        for report in [setup_failure(), terminal_failure()] {
            let payload = encode_sink_failure(&report).unwrap();
            assert_eq!(
                decode_sink_failure(report.kind.message_kind(), &payload).unwrap(),
                report
            );
        }
    }

    #[test]
    fn status_is_closed_to_its_two_collector_message_kinds() {
        let payload = encode_sink_failure(&setup_failure()).unwrap();
        assert!(matches!(
            decode_sink_failure(MessageKind::HeartbeatProof, &payload),
            Err(SinkStatusError::WrongMessageKind(kind))
                if kind == MessageKind::HeartbeatProof as u16
        ));
    }
}
