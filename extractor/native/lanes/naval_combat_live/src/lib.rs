pub mod convoy_terminal_attribution;
pub mod convoy_terminal_runtime;
pub mod convoy_route_source;
pub mod convoy_transport_attack;
#[cfg(all(windows, target_arch = "x86_64"))]
pub mod convoy_terminal_hook_windows;
pub mod domain49_50_naval_combat_live;
pub mod extractor;
pub mod member_epoch_runtime;

pub use convoy_terminal_attribution::{
    activation_blockers as convoy_terminal_activation_blockers,
    capture_candidate as capture_convoy_terminal_candidate,
    prepare_a729_hook as prepare_a729_convoy_terminal_hook, A729PreCollapseRegisters,
    ActivationBlocker as ConvoyTerminalActivationBlocker,
    CaptureError as ConvoyTerminalCaptureError,
    CarrierDecodeError as ConvoyTerminalCarrierDecodeError, CombatEpoch,
    ConvoyTerminalAttributionCarrier, EntityHandle as ConvoyTerminalEntityHandle,
    ExactCombatEpochResolver, ExactLiveRouteResolver, ExactMemory as ConvoyTerminalExactMemory,
    LiveRouteResolution, PreparedA729Hook as PreparedA729ConvoyTerminalHook, RouteEndpoint,
    RouteEndpointKind, RouteProducerKind, A729_CALL_BYTES, A729_CONTEXT_BYTES,
    A729_CONTEXT_RVA, A729_EXE_SHA256, A729_HOOK_RESUME_RVA, A729_HOOK_RVA,
    A729_TERMINAL_AGGREGATOR_RVA, ACTIVATION_COMPILED, CARRIER_FLAG_LIVE_ROUTE_EXACT,
    CARRIER_FLAG_LIVE_ROUTE_MANAGER_EXACT,
    CARRIER_LEN, CARRIER_MAGIC, CARRIER_VERSION, CFEX_MEMBER_BATTLE_SLOT_OFF,
    CFEX_MEMBER_CLIENT_HANDLE_OFF, CFEX_MEMBER_CONVOY_HANDLE_OFF,
    CFEX_MEMBER_CONVOY_INDEX_OFF, CFEX_MEMBER_OWNER_TAG_OFF, CFEX_MEMBER_STATE_OFF,
    CFEX_MEMBER_VTABLE_RVA, GAME_ENTITY_TYPE_TAG, H_CONVOY_CONTROL_PAIR_SHA256,
    MAX_COUNTRY_TAG_DB_ID, PrepareError as ConvoyTerminalPrepareError,
    DISCOVERY_ONLY_CONVOY_CUSTODY_PROMOTABLE, TEN_SHARD_AUTHORITY_MANIFEST_SHA256,
    TEN_SHARD_POOL_STATUS_RECEIPT_SHA256, TERMINAL_SUNK_STATE,
};

pub use convoy_terminal_runtime::{
    authenticate_exact_a729_route_read, clear_convoy_terminal_resolver_snapshot,
    current_convoy_terminal_publication_receipt, current_convoy_terminal_resolver_snapshot,
    publish_convoy_terminal_resolver_bindings, publish_convoy_terminal_resolver_snapshot,
    try_current_convoy_terminal_resolver_snapshot, AuthenticatedLiveRouteBinding,
    ResolverPublicationReceipt, ResolverSnapshot,
};

pub use convoy_route_source::{
    read_exact_a729_client_route, ExactA729RouteRead,
    RouteSourceError as ConvoyRouteSourceError, A729_CLEND_LEASE_EXCHANGE_VTABLE_RVA,
    A729_CNAVAL_UNIT_TRANSFER_VTABLE_RVA, A729_REFERENCE_REGISTRY_DEFAULT_RVA,
    A729_REFERENCE_REGISTRY_LARGE_RVA, A729_REFERENCE_REGISTRY_SMALL_TABLE_RVA,
    A729_REFERENCE_RESOLVER_RVA,
    CLEND_LEASE_EXCHANGE_DESTINATION_STATE_PTR_OFF,
    CLEND_LEASE_EXCHANGE_ORIGIN_STATE_PTR_OFF, CNAVAL_UNIT_TRANSFER_SOURCE_PROVINCE_OFF,
    CNAVAL_UNIT_TRANSFER_TARGET_PROVINCE_OFF, CSTATE_DATABASE_ID_OFF,
};

pub use convoy_transport_attack::{
    decode_convoy_transport_attack_batch, encode_convoy_transport_attack_batch,
    BatchDecodeError as ConvoyTransportAttackBatchError, ConvoyD70MutationKind,
    ConvoyD70MutationProof, ConvoyTransportAttackRow, ConvoyTransportAttackTelemetry,
    ConvoyTransportAttackWitness, DecodedConvoyTransportAttackBatch,
    BATCH_FLAG_COMPLETE_EXACT_EVENTS, BATCH_FLAG_HOOK_INSTALLED,
    BATCH_HEADER_LEN as CONVOY_TRANSPORT_ATTACK_BATCH_HEADER_LEN,
    BATCH_MAGIC as CONVOY_TRANSPORT_ATTACK_BATCH_MAGIC,
    BATCH_ROW_LEN as CONVOY_TRANSPORT_ATTACK_BATCH_ROW_LEN,
    BATCH_VERSION as CONVOY_TRANSPORT_ATTACK_BATCH_VERSION,
    DOMAIN_CONVOY_TRANSPORT_ATTACK, DOMAIN_CONVOY_TRANSPORT_ATTACK_NAME,
    MAX_BATCH_ROWS as MAX_CONVOY_TRANSPORT_ATTACK_BATCH_ROWS,
};

#[cfg(all(windows, target_arch = "x86_64"))]
pub use convoy_terminal_hook_windows::{
    convoy_terminal_hook_heartbeat, convoy_terminal_hook_installed,
    drain_convoy_terminal_events, drain_convoy_transport_attack_payload,
    install_convoy_terminal_hook, read_live_a729_client_route,
    seal_and_drain_convoy_transport_attack_payload, uninstall_convoy_terminal_hook,
    ConvoyTerminalDrain, ConvoyTerminalHookHeartbeat,
    ConvoyTerminalHookTelemetry, ConvoyTerminalInstallError, ConvoyTerminalValidatedEvent,
    ConvoyTerminalSealError, D70MutationKind, D70MutationProof,
};

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

pub use member_epoch_runtime::{
    clear_member_epoch_snapshot, current_member_epoch_publication_receipt,
    current_member_epoch_snapshot, publish_member_epoch_snapshot,
    try_current_member_epoch_snapshot, MemberEpochBinding, MemberEpochPublicationReceipt,
    MemberEpochResolution, MemberEpochSnapshot, MAX_ACTIVE_MEMBER_BINDINGS,
};
