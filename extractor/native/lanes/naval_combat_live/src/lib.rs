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
    CFEX_MEMBER_VTABLE_RVA, D90_STATIC_AUTHORITY_MANIFEST_SHA256, GAME_ENTITY_TYPE_TAG,
    MAX_COUNTRY_TAG_DB_ID, PrepareError as ConvoyTerminalPrepareError,
    DISCOVERY_ONLY_CONVOY_CUSTODY_PROMOTABLE, TERMINAL_SUNK_STATE,
};

pub use convoy_terminal_runtime::{
    authenticate_evidence_a729_route_read, authenticate_exact_a729_route_read,
    clear_convoy_terminal_resolver_snapshot,
    current_convoy_terminal_publication_receipt, current_convoy_terminal_resolver_snapshot,
    publish_convoy_terminal_resolver_bindings,
    publish_convoy_terminal_resolver_snapshot,
    publish_evidence_convoy_terminal_resolver_bindings,
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
    abort_partial_install_and_uninstall as abort_partial_convoy_terminal_install,
    capture_h_convoy_control_raw, convoy_terminal_evidence_activation_enabled,
    commit_convoy_terminal_events, commit_convoy_transport_attack_payload,
    convoy_terminal_hook_heartbeat, convoy_terminal_hook_installed,
    convoy_terminal_hook_lifecycle_pending, install_convoy_terminal_evidence_hook,
    install_convoy_terminal_hook, installed_hook_receipt, peek_convoy_terminal_events,
    peek_convoy_transport_attack_payload, publish_h_convoy_evidence_game_state,
    read_live_a729_client_route, rollback_convoy_terminal_events,
    rollback_convoy_transport_attack_payload,
    seal_and_drain_convoy_transport_attack_payload, take_h_convoy_positive_raw,
    take_released_hook_receipt, uninstall_convoy_terminal_hook, ConvoyTerminalDrain,
    ConvoyTerminalDrainReservation, ConvoyTerminalHookHeartbeat, ConvoyTerminalHookTelemetry,
    ConvoyTerminalInstallError, ConvoyTerminalPayloadReservation, ConvoyTerminalSealError,
    ConvoyTerminalValidatedEvent, D70MutationKind, D70MutationProof, HConvoyControlRawObservation,
    HConvoyD70Identity, HConvoyD70Row, HConvoyPositiveRawObservation, HConvoyRouteRawObservation,
    H_CONVOY_EVIDENCE_ACTIVATION_ENV,
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

/// EVERY HELD-FREEZE WINDOW IN THIS CRATE, READ.
///
/// This crate takes `inline_detour::freeze_process()` directly, which stops
/// every other thread of the game. Until the guard drops, an allocation or a
/// lock here is a HANG and not a slow path: the lock may be held by a thread
/// that can no longer run, and the collector declares no `#[global_allocator]`,
/// so Rust's allocator is `HeapAlloc` on the PROCESS DEFAULT HEAP -- the same
/// heap the game's own threads reach through ordinary Win32 calls.
///
/// `inline_detour` has enforced this on its own windows for a long time. This
/// crate did not, and the audit found real defects the first time it ran.
///
/// What production change would make this fail? Building an error string,
/// collecting an iterator, taking a mutex, or growing a vector between the
/// freeze and its drop -- at any call depth inside this crate, in any file
/// under `src/`, including one added after this test was written.
#[cfg(test)]
mod frozen_window_audit_tests {
    use freeze_window_audit::{audit, crate_sources, fingerprint};

    /// How many findings this crate still carries, and WHICH ones.
    ///
    /// THE 30 ALONE IS NOT A RATCHET: fixing one finding while adding
    /// another leaves it unchanged. The fingerprint is over the sorted findings,
    /// so any change to which ones exist fails here even when there are the same
    /// number of them. Both may only move when a human decides they should.
    const OPEN_FINDINGS: usize = 30;
    const OPEN_15221709046038960093u64: u64 = 15221709046038960093u64;

    #[test]
    fn nothing_in_a_held_freeze_window_allocates_or_blocks() {
        // THE DIRECTORY IS THE FILE LIST. A hand-written one cannot see a
        // module added later, and an unscanned module is an unscanned window.
        let sources = crate_sources(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src"
        )));
        assert!(
            sources.len() >= 2,
            "the scan found only {} source file(s); it is not reaching the crate",
            sources.len()
        );
        let findings = audit(&sources, &["suspend_all_other_threads()"]);
        let listed = findings
            .iter()
            .map(|finding| format!("  {finding}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            (findings.len(), fingerprint(&findings)),
            (OPEN_FINDINGS, OPEN_15221709046038960093u64),
            "the held-freeze windows of this crate changed. If you FIXED one, \
             lower `OPEN_FINDINGS` and update `OPEN_15221709046038960093u64`. If this grew, \
             the change put new work inside a window where the whole game is \
             stopped:\n{listed}"
        );
    }
}
