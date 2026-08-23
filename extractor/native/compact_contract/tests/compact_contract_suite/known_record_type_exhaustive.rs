use compact_contract::KnownRecordType;

const fn consumer_action(record_type: KnownRecordType) -> &'static str {
    match record_type {
        KnownRecordType::DomainPerf => "domain outcome",
        KnownRecordType::FrameEnd => "frame boundary",
        KnownRecordType::DomainData => "world data",
        KnownRecordType::DomainDataRepeat => "world data",
        KnownRecordType::StreamHeader => "identity",
        KnownRecordType::DomainGap => "domain outcome",
        KnownRecordType::FrameRefused => "frame boundary",
        KnownRecordType::FrameDegraded => "frame annotation",
        KnownRecordType::CaptureTerminated => "termination",
        KnownRecordType::ClockUnreadableInterval => "clock annotation",
        KnownRecordType::UntrustedClockRecovery => "clock annotation",
        KnownRecordType::FreezeCensus => "no coverage",
        KnownRecordType::ContentIdentityCensus => "session identity",
        KnownRecordType::StateTopologyCensus => "session identity",
    }
}

#[test]
fn public_record_type_vocabulary_is_exhaustively_matchable() {
    assert_eq!(
        consumer_action(KnownRecordType::FreezeCensus),
        "no coverage"
    );
    assert_eq!(
        consumer_action(KnownRecordType::ContentIdentityCensus),
        "session identity"
    );
}
