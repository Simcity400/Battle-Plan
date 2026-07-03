pub mod domain49_50_naval_combat_live;
pub mod extractor;

pub use domain49_50_naval_combat_live::{
    compact_record, decode_lifecycle_data_payload, decode_lifecycle_rows, decode_member_rows,
    decode_members_data_payload, decode_stream, encode_lifecycle_payload, encode_member_payload,
    gate_stream, result_json, CombatSide, GateResult, GateSummary, LifecycleRow, MemberRow,
    PayloadDecodeSummary, COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_MAGIC,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_VERSION, DOMAIN_NAVAL_COMBAT_LIFECYCLE,
    DOMAIN_NAVAL_COMBAT_LIFECYCLE_NAME, DOMAIN_NAVAL_COMBAT_MEMBERS,
    DOMAIN_NAVAL_COMBAT_MEMBERS_NAME, LIFECYCLE_ROW_LEN, MEMBER_ROW_LEN, SCHEMA_VERSION,
    STATUS_DELTA_DATA, STATUS_LOCATION_CANDIDATE_UNVALIDATED, STATUS_PARTIAL_DATA,
    STATUS_SOURCE_UNAVAILABLE,
};

pub use extractor::{
    build_capture_frame, build_transition_capture_frame, validate_member_cship_joins, CaptureFrame,
    CshipRoute, CshipSnapshotRow, ExtractorOffsets, MemberBridgeRow, NavalCombatBodyBridgeRow,
    ValidatedCombatSnapshot, CURRENT_870EBF_BUILD_SHA256, CURRENT_870EBF_CSHIP_ROUTE,
    CURRENT_870EBF_OFFSETS,
};
