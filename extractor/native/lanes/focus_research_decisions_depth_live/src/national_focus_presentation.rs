//! A31/XUI10 exact national-focus presentation boundary.
//!
//! The portable NFP1 schema, validation, receipt binding, and inert production
//! gate are owned by `compact_contract`. The a729 owner/item observer lives in
//! this lane's `a729_focus_item_producer` module. This re-export preserves the
//! lane-facing API without maintaining a second, divergent carrier grammar.

pub use compact_contract::national_focus_presentation::{
    collect_and_encode, collect_and_validate, decode_payload, encode_payload, encode_unavailable,
    validate_frame, validate_semantic_snapshot, validate_snapshot, AuthorityState,
    CarrierError as ProducerError,
    ExactNationalFocusPresentationSource, FocusPresentationItem, Frame as CompactFrame,
    RectI32, SelectedPresentBinding, Snapshot as NationalFocusPresentationFrame,
    AUTHORED_SURFACE_IDENTITY, DOMAIN_ID, DOMAIN_NAME, EFFECTIVE_INTERFACE_WINNER,
    EXACT20_FUNCTION_MANIFEST_SHA256, EXACT20_JOB_RECEIPT_SHA256, EXECUTABLE_SHA256,
    LIVE_FIXTURE_ACCEPTED, MAGIC, MAX_UI_PASS_OPERATIONS, MAX_VISIBLE_ITEMS,
    PRODUCTION_SCHEDULING_ENABLED, RUNTIME_ROOT_IDENTITY, SCHEMA_VERSION,
};
