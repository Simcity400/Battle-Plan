use crate::convoy_terminal_attribution::{
    CarrierDecodeError, ConvoyTerminalAttributionCarrier, CARRIER_LEN,
};

pub const DOMAIN_CONVOY_TRANSPORT_ATTACK: u16 = 90;
pub const DOMAIN_CONVOY_TRANSPORT_ATTACK_NAME: &str = "convoy_transport_attack_live";

pub const BATCH_MAGIC: u32 = u32::from_le_bytes(*b"CTA2");
/// Header 512 bytes; reserved tail `V7_RESERVED_OFF`..512; v9 and below refused
/// by name.
///
/// Three things changed since v5, and the first changes what a reader may
/// conclude from a batch:
///
/// * `exact_events_complete` no longer conjoins
///   `resolver_unbound_convoy_clients == 0` (see the method's own comment). A
///   v5 batch and a v7 batch with identical bytes can disagree about the
///   complete flag, which is exactly the case a version exists to separate.
/// * the header carries the `CaptureError` leaves of the `capture_error`
///   candidate subsite, so a refused sink event names the proof that refused
///   it instead of the place it was refused at. They start at 344: lane RX2
///   deleted `InvalidMemberState`, whose predicate could never pass at this
///   callsite, rather than reserve a leaf that would always read zero. The
///   candidate subsites above them went from eight words to six in the same
///   wave, for the same reason, which is why the whole tail moved.
/// * the header carries `dropped_pre_hour_address_overflow`, the leaf for the
///   hook's last uncounted exit, immediately after them.
///
/// The middle version number is spent rather than reused. This lane's first
/// pass declared a v6 whose reserved tail ran 488..512, and the second pass put
/// the overflow leaf inside it. Nothing has ever produced or consumed either
/// (both land unreleased, and old captures are disposable pre-production), but
/// one version number naming two layouts is a trap for the next reader, and a
/// `u16` is not scarce. So the layout above is v7 and 6 is refused like 5.
///
/// v8 keeps that header byte for byte and changes the ROW: the carrier inside
/// it went to CTA1 v2, where the route is an outcome rather than a
/// precondition and the member state field changed sense. A v7 reader handed a
/// v8 row would apply both invariants backwards -- it would expect endpoints
/// where a refusal cause now sits, and reject every member state a v8 producer
/// writes -- so the batch version moves with the row it carries even though
/// the header did not change.
/// v9 keeps the header byte for byte and moves with the ROW again: the carrier
/// went to CTA1 v3, where a refused row's bytes 76..80 carry the unmodelled
/// producer's vtable RVA. A v8 reader requires that window to be zero and
/// would reject every v9 refusal that names a class.
///
/// v10 changes BOTH. The carrier went to CTA1 v4, where the combat-epoch join
/// is an outcome rather than a precondition; and with the join no longer able
/// to destroy an event, three `CaptureError` leaves lost the only paths that
/// could ever have made them nonzero -- `invalid_combat_epoch`, `combat_join`
/// and `combat_slot_mismatch` -- so they are deleted rather than left reading
/// zero forever, which is the rule v8 set for itself. The `CaptureError`
/// window is twelve words instead of fifteen and everything after it moves.
pub const BATCH_VERSION: u16 = 10;
pub const BATCH_HEADER_LEN: usize = 512;
const V2_TELEMETRY_WORDS_OFF: usize = 32;
const V3_TELEMETRY_WORDS_OFF: usize = 192;
const V3_TELEMETRY_WORDS: usize = 8;
// v5 appends the per-drain facts that exactness is now judged from. The v3/v4
// lifetime counters stay on the wire verbatim as diagnostics; they no longer
// decide anything (pw_gate_final: one early incident poisoned all 392 batches).
const V5_PER_DRAIN_OFF: usize = 256;
const V5_SUBSITE_OFF: usize = 280;
// DERIVED from the window above it, not written: a subsite slot added or
// removed moves everything after it, and an offset that did not move would
// have two meanings for one byte. The version pin below is what stops that
// move from happening quietly.
const V5_RECEIPT_AHEAD_OFF: usize = V5_SUBSITE_OFF + V5_SUBSITE_WORDS * 8;
const V5_LAG_BEYOND_OFF: usize = V5_RECEIPT_AHEAD_OFF + 8;
// v7 appends the CaptureError leaves. rc_vanilla spent all 56 of its sink
// events on `capture_error` and the variant had already been thrown away by
// `Err(_)` at the call site, so the capture could not say which of the
// proofs refused.
const V7_CAPTURE_ERROR_OFF: usize = V5_LAG_BEYOND_OFF + 8;
// The last silent exit out of the hook. `prepare_observation` computed the
// engine-hour address with `checked_add(..)?`, so an overflow returned from the
// hook after `CALLBACKS_SEEN` had already counted the entry and before any drop
// counter moved -- one event lost at no named site, the same shape as the
// context-address overflow QY closed one line further down the same function
// (Codex review 01a0310c-cd88-7f51-b09b-7ca49723b01a, BLOCKING).
const V7_PRE_HOUR_ADDRESS_OVERFLOW_OFF: usize = V7_CAPTURE_ERROR_OFF + V7_CAPTURE_ERROR_WORDS * 8;
const V7_RESERVED_OFF: usize = V7_PRE_HOUR_ADDRESS_OVERFLOW_OFF + 8;

/// v10's HEADER LAYOUT, pinned.
///
/// The two subsite windows are generated from their taxonomies now, and the
/// offsets after them are generated from the windows -- so adding a cause
/// relayouts the header correctly and silently. Correctly is not enough: a v10
/// batch would then mean something a v10 reader does not expect, which is what
/// a version number is for. These four numbers are v10's, and moving one is a
/// new layout that needs a new `BATCH_VERSION` and a decoder that refuses the
/// old one -- there is no backwards compatibility pre-production, so the edit
/// is to this pin and to the version together.
const _: () = assert!(
    BATCH_VERSION == 10
        && V5_SUBSITE_WORDS == 6
        && V7_CAPTURE_ERROR_WORDS == 12
        && V7_RESERVED_OFF == 448,
    "the CTA2 header layout moved; that is a new BATCH_VERSION, not an edit to v10",
);

/// v10's SUBSITE WINDOWS, slot by slot. NOTHING READS THESE.
///
/// The counts above stop a window changing width; these stop a slot changing
/// meaning at the same width. Renaming a cause, or moving one to another
/// slot, leaves an existing v10 capture's bytes decoded under the new
/// meanings -- the numbers still add up and the fold and the attach log
/// publish them under the wrong causes (Codex round 2, finding 2). So v10's
/// wire is written down here once, and a taxonomy edit that changes it is a
/// compile error naming the version bump.
const V10_CANDIDATE_SLOTS: [&str; V5_SUBSITE_WORDS] = [
    "initial_gate",
    "producer_sequence_exhausted",
    "capture_error",
    "context_mismatch",
    "context_address_overflow",
    "member_bytes_unreadable",
];

const V10_CAPTURE_ERROR_SLOTS: [&str; V7_CAPTURE_ERROR_WORDS] = [
    "null_call_register",
    "call_abi_mismatch",
    "address_overflow",
    "member_unreadable",
    "member_vtable_mismatch",
    "invalid_handle",
    "invalid_owner_tag",
    "invalid_killer_tag",
    "invalid_producer_sequence",
    "invalid_endpoint",
    "route_join",
    "route_key_mismatch",
];

const _: () = {
    let mut slot = 0;
    while slot < V5_SUBSITE_WORDS {
        assert!(
            compact_contract::diagnostic_channel::const_str_is_prefixed(
                ConvoyTransportAttackCandidateSubsites::FIELDS[slot],
                "convoy_transport_attack_dropped_candidate_",
                V10_CANDIDATE_SLOTS[slot],
            ),
            "a v10 candidate slot changed meaning; that is a new BATCH_VERSION",
        );
        slot += 1;
    }
    let mut slot = 0;
    while slot < V7_CAPTURE_ERROR_WORDS {
        assert!(
            compact_contract::diagnostic_channel::const_str_is_prefixed(
                ConvoyTransportAttackCaptureErrorSubsites::FIELDS[slot],
                "convoy_transport_attack_dropped_capture_error_",
                V10_CAPTURE_ERROR_SLOTS[slot],
            ),
            "a v10 capture-error slot changed meaning; that is a new BATCH_VERSION",
        );
        slot += 1;
    }
};
/// The reserved tail cannot be negative-length: a window that outgrew the
/// header would otherwise be written past it.
const _: () = assert!(V7_RESERVED_OFF <= BATCH_HEADER_LEN);
pub const BATCH_ROW_LEN: usize = 120;
pub const BATCH_FLAG_COMPLETE_EXACT_EVENTS: u32 = 1 << 0;
pub const BATCH_FLAG_HOOK_INSTALLED: u32 = 1 << 1;
pub const BATCH_FLAG_RESOLVER_EVER_PARTIAL: u32 = 1 << 2;
pub const BATCH_KNOWN_FLAGS: u32 =
    BATCH_FLAG_COMPLETE_EXACT_EVENTS | BATCH_FLAG_HOOK_INSTALLED | BATCH_FLAG_RESOLVER_EVER_PARTIAL;
pub const MAX_BATCH_ROWS: usize = 4_096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ConvoyD70MutationKind {
    IncrementExisting = 1,
    AppendFirstSeen = 2,
}

compact_contract::wire_coded_codes! {
    ConvoyD70MutationKind: u8,
    code = pub code,
    Ok = decode(value) -> Result<Self, BatchDecodeError> { Err(BatchDecodeError::InvalidMutationKind(value)) };

    IncrementExisting = 1,
    AppendFirstSeen = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConvoyD70MutationProof {
    pub kind: ConvoyD70MutationKind,
    pub ordinal: u32,
    pub before_rows: u32,
    pub after_rows: u32,
    pub before_count: u32,
    pub after_count: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConvoyTransportAttackRow {
    pub carrier: ConvoyTerminalAttributionCarrier,
    pub d70: ConvoyD70MutationProof,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConvoyTransportAttackTelemetry {
    pub callbacks_seen: u64,
    pub original_calls: u64,
    pub events_emitted: u64,
    pub dropped_no_original: u64,
    pub dropped_teardown: u64,
    pub dropped_nested: u64,
    pub dropped_capture_busy: u64,
    pub dropped_hour_or_epoch: u64,
    pub dropped_resolver: u64,
    pub dropped_candidate: u64,
    pub dropped_pre_hour_unreadable: u64,
    pub dropped_receipt_hour_mismatch: u64,
    pub dropped_post_hour_unreadable: u64,
    pub dropped_post_hour_changed: u64,
    pub dropped_resolver_snapshot_absent: u64,
    pub dropped_resolver_unpublished: u64,
    pub receipt_hour_lag_max: u64,
    pub receipt_hour_lead_max: u64,
    pub dropped_d70_pre: u64,
    pub dropped_d70_post: u64,
    pub dropped_d70_mutation: u64,
    pub dropped_queue_busy: u64,
    pub dropped_queue_full: u64,
    /// Events refused because the resolver receipt was NEWER than the event's
    /// own engine hour (clock skew / torn hour read).
    pub dropped_receipt_hour_ahead: u64,
    /// Events refused because the receipt lagged the event by more than one
    /// observed inter-publication interval (combat-epoch join too stale to
    /// attribute). The bound is the collector's own cadence, never a constant.
    pub dropped_receipt_lag_beyond: u64,
    /// Events refused because `game_state + GS_CURRENT_HOUR_OFF` overflowed, so
    /// the engine hour could not even be addressed. A leaf OF
    /// `dropped_hour_or_epoch`, like the two above it; before it existed this
    /// path returned from the hook with nothing counted at all.
    pub dropped_pre_hour_address_overflow: u64,
}

crate::convoy_wire_taxonomy::convoy_named_counters! {
    ConvoyTransportAttackTelemetry,
    prefix = "convoy_transport_attack_",
    all = named_drops;

    /// The aggregates. `dropped_events` is their sum; a leaf is incremented
    /// BESIDE its aggregate, so summing leaves too would count one refusal
    /// twice.
    summed_drops {
        dropped_no_original,
        dropped_teardown,
        dropped_nested,
        dropped_capture_busy,
        dropped_hour_or_epoch,
        dropped_resolver,
        dropped_candidate,
        dropped_d70_pre,
        dropped_d70_post,
        dropped_d70_mutation,
        dropped_queue_busy,
        dropped_queue_full,
    }

    /// Diagnostic leaves OF the aggregates above.
    leaf_drops {
        dropped_pre_hour_unreadable,
        dropped_pre_hour_address_overflow,
        dropped_receipt_hour_mismatch,
        dropped_receipt_hour_ahead,
        dropped_receipt_lag_beyond,
        dropped_post_hour_unreadable,
        dropped_post_hour_changed,
        dropped_resolver_snapshot_absent,
        dropped_resolver_unpublished,
    }
}

crate::convoy_wire_taxonomy::convoy_named_counters! {
    ConvoyTransportAttackTelemetry,
    prefix = "convoy_transport_attack_";

    named_gauges {
        callbacks_seen,
        original_calls,
        events_emitted,
        receipt_hour_lag_max,
        receipt_hour_lead_max,
    }
}

impl ConvoyTransportAttackTelemetry {
    /// SESSION-LIFETIME totals, which is what the drain carries by
    /// construction (`candidate_subsites` is validated against lifetime
    /// `dropped_candidate`); the per-drain number is `drops_this_drain`. Any
    /// caller printing these must say `lifetime_` or it will read as this
    /// interval's cause.
    pub fn has_drop(&self) -> bool {
        self.named_drops().into_iter().any(|(_, value)| value != 0)
    }
}

crate::convoy_wire_taxonomy::convoy_subsite_table! {
    /// Where a DROP_CANDIDATE came from. The aggregate `dropped_candidate`
    /// could not say, so pw_gate_final's 46 candidate drops stayed
    /// unattributable and every one of them was unfixable by construction.
    /// These are LIFETIME counters like `dropped_candidate` and must sum to it
    /// exactly.
    ///
    /// v8 dropped two slots, because neither could be a lost event any more: a
    /// live route read that no longer matches the published binding is a NAMED
    /// route refusal on a recorded sink, and the evidence fixture's raw
    /// observation slot is filled after the enqueue and so cannot cost one.
    /// Reserving them would have left two leaves permanently reading zero.
    ConvoyTransportAttackCandidateSubsites,
    unit_prefix = "convoy_transport_attack_dropped_candidate_",
    count = V5_SUBSITE_WORDS;

    initial_gate = 0,
    producer_sequence_exhausted = 1,
    capture_error = 2,
    /// The A729 context bytes did not read or did not match.
    context_mismatch = 3,
    /// The context ADDRESS itself overflowed (previously a silent uncounted
    /// loss path).
    context_address_overflow = 4,
    member_bytes_unreadable = 5,
}

crate::convoy_wire_taxonomy::convoy_capture_error_subsites! {
    /// Which `CaptureError` a `capture_error` candidate drop came from. TWELVE
    /// of them in v10. Each deletion went with the predicate that was the only
    /// thing producing it: `InvalidMemberState` in v8, and the three combat
    /// leaves in v10, where the combat-epoch join stopped being able to refuse
    /// an event at all.
    ///
    /// `prepare_observation` matched `capture_candidate(..)` with `Err(_)` and
    /// threw the variant away, so `capture_error` named the CALL, not the
    /// cause. In captures/rc_vanilla every one of the window's 56 sink events
    /// died there -- 56 of 56 hook entries, `dropped_candidate=56`,
    /// `dropped_candidate_capture_error=56`, every other leaf 0 -- and which
    /// proof inside `capture_candidate` refused was unrecoverable from the
    /// capture. These are LIFETIME counters like the candidate subsites they
    /// refine, and their sum must equal `candidate_subsites.capture_error`
    /// exactly (validated at decode).
    crate::convoy_terminal_attribution::CaptureError
        => ConvoyTransportAttackCaptureErrorSubsites,
    unit_prefix = "convoy_transport_attack_dropped_capture_error_",
    count = V7_CAPTURE_ERROR_WORDS;

    NullCallRegister = 0 => null_call_register,
    CallAbiMismatch = 1 => call_abi_mismatch,
    AddressOverflow = 2 => address_overflow,
    MemberUnreadable = 3 => member_unreadable,
    MemberVtableMismatch = 4 => member_vtable_mismatch,
    InvalidHandle = 5 => invalid_handle,
    InvalidOwnerTag = 6 => invalid_owner_tag,
    InvalidKillerTag = 7 => invalid_killer_tag,
    InvalidProducerSequence = 8 => invalid_producer_sequence,
    InvalidEndpoint = 9 => invalid_endpoint,
    RouteJoin = 10 => route_join,
    RouteKeyMismatch = 11 => route_key_mismatch,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConvoyTransportAttackWitness {
    pub hook_installed: bool,
    pub queue_drain_complete: bool,
    pub producer_sequence_next: u64,
    pub drained_rows: u32,
    pub remaining_events: u32,
    pub active_callbacks: u32,
    pub resolver_publication_sequence: u64,
    pub resolver_game_hour_raw: i32,
    pub resolver_member_bindings: u32,
    pub resolver_route_bindings: u32,
    pub resolver_unbound_convoy_clients: u32,
    /// Diagnostic only since v5. A past partial publication no longer
    /// disqualifies later batches: any events it lost were dropped under the
    /// per-drain drop accounting of their own intervals, so the taint is
    /// already carried where it happened. Keeping it on the wire preserves
    /// the session-level view; removing it from exactness is what lets a
    /// clean drain prove its own batch (pw_gate_final M-A).
    pub resolver_ever_partial: bool,
    /// Hook entries observed during THIS drain interval (delta against the
    /// last committed drain, not the session).
    pub events_seen_this_drain: u64,
    /// Events enqueued during THIS drain interval.
    pub events_emitted_this_drain: u64,
    /// Events refused at any named drop site during THIS drain interval.
    /// By construction every hook entry ends at exactly one drop site or in
    /// an enqueue, so seen == emitted + drops must hold and is validated.
    pub drops_this_drain: u64,
    pub candidate_subsites: ConvoyTransportAttackCandidateSubsites,
    pub capture_error_subsites: ConvoyTransportAttackCaptureErrorSubsites,
    pub telemetry: ConvoyTransportAttackTelemetry,
}

impl ConvoyTransportAttackWitness {
    /// Whether THIS batch proves its event set exhaustive. Every conjunct is
    /// a fact of this drain interval or a point-in-time reading taken under
    /// the drain's own correlation guard -- nothing session-cumulative
    /// decides it (pw_gate_final: 392/393 batches failed on lifetime scars
    /// from incidents their own intervals never contained).
    /// `resolver_unbound_convoy_clients` is deliberately NOT a conjunct.
    ///
    /// It is the last publication-level fact that was still deciding a
    /// per-drain claim -- the same defect class as the v3/v4 lifetime scars,
    /// one level up -- and in captures/rc_vanilla it alone cost 319 of 323
    /// frames their exactness while 267+ of those frames saw no sink traffic
    /// at all (window totals: `events_seen_this_drain=56`,
    /// `drops_this_drain=56`, `remaining_events=0`, `active_callbacks=0`,
    /// `hook_installed_batches=323`, `queue_drain_complete_batches=323`, so
    /// no other conjunct could have failed 319 times).
    ///
    /// Dropping it cannot hide a lost event. `convoy_terminal_hook`
    /// increments `CALLBACKS_SEEN` on ENTRY, before the resolver is consulted
    /// at all, and every path out of the hook ends at exactly one named drop
    /// counter or in an enqueue. An event whose convoy client has no route
    /// binding no longer ends at a drop at ALL: since fix C the refusal is an
    /// OUTCOME, so the sink is recorded as a row whose route is `Refused` with
    /// the sweep's own cause, and the drain stays exact instead of losing the
    /// sink to hide a route it never had. A drop that does happen, for this or
    /// any other reason, lands on a named `capture_error` subsite inside
    /// `drops_this_drain`, which IS a conjunct -- so nothing hides behind an
    /// unbound client either way. What `resolver_unbound_convoy_clients`
    /// expresses is coverage -- "the route side of these clients is unknown"
    /// -- which is a real gap and stays named on the wire
    /// (`resolver_unbound_convoy_clients`, its own manifest `problems` row,
    /// and the `h_convoy_route_refused_*` leaves that say why), but it is not
    /// the claim this predicate makes about THIS interval's event set.
    pub fn exact_events_complete(&self) -> bool {
        self.hook_installed
            && self.queue_drain_complete
            && self.remaining_events == 0
            && self.active_callbacks == 0
            && self.events_seen_this_drain == self.events_emitted_this_drain
            && self.drops_this_drain == 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedConvoyTransportAttackBatch {
    pub exact_events_complete: bool,
    pub witness: ConvoyTransportAttackWitness,
    pub rows: Vec<ConvoyTransportAttackRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BatchDecodeError {
    Length(usize),
    Magic,
    VersionOrLayout,
    Flags,
    Reserved,
    RowCount(usize),
    SizeOverflow,
    Carrier(CarrierDecodeError),
    InvalidMutationKind(u8),
    InvalidMutationProof(&'static str),
    ProducerOrder,
    MutationChain,
    InvalidWitness(&'static str),
}

impl From<CarrierDecodeError> for BatchDecodeError {
    fn from(value: CarrierDecodeError) -> Self {
        Self::Carrier(value)
    }
}

pub fn encode_convoy_transport_attack_batch(
    rows: &[ConvoyTransportAttackRow],
    witness: ConvoyTransportAttackWitness,
) -> Result<Vec<u8>, BatchDecodeError> {
    if rows.len() > MAX_BATCH_ROWS {
        return Err(BatchDecodeError::RowCount(rows.len()));
    }
    validate_producer_order(rows)?;
    validate_mutation_chain(rows)?;
    validate_witness(&witness, rows.len())?;
    let payload_len = BATCH_HEADER_LEN
        .checked_add(
            rows.len()
                .checked_mul(BATCH_ROW_LEN)
                .ok_or(BatchDecodeError::SizeOverflow)?,
        )
        .ok_or(BatchDecodeError::SizeOverflow)?;
    let mut out = vec![0u8; payload_len];
    write_u32(&mut out, 0, BATCH_MAGIC);
    write_u16(&mut out, 4, BATCH_VERSION);
    write_u16(&mut out, 6, BATCH_HEADER_LEN as u16);
    write_u16(&mut out, 8, BATCH_ROW_LEN as u16);
    write_u32(&mut out, 12, rows.len() as u32);
    let flags = u32::from(witness.exact_events_complete()) * BATCH_FLAG_COMPLETE_EXACT_EVENTS
        | u32::from(witness.hook_installed) * BATCH_FLAG_HOOK_INSTALLED
        | u32::from(witness.resolver_ever_partial) * BATCH_FLAG_RESOLVER_EVER_PARTIAL;
    write_u32(&mut out, 16, flags);
    write_u64(&mut out, 24, witness.producer_sequence_next);
    let telemetry_values = [
        witness.telemetry.callbacks_seen,
        witness.telemetry.original_calls,
        witness.telemetry.events_emitted,
        witness.telemetry.dropped_no_original,
        witness.telemetry.dropped_teardown,
        witness.telemetry.dropped_nested,
        witness.telemetry.dropped_capture_busy,
        witness.telemetry.dropped_hour_or_epoch,
        witness.telemetry.dropped_resolver,
        witness.telemetry.dropped_candidate,
        witness.telemetry.dropped_d70_pre,
        witness.telemetry.dropped_d70_post,
        witness.telemetry.dropped_d70_mutation,
        witness.telemetry.dropped_queue_busy,
        witness.telemetry.dropped_queue_full,
    ];
    for (index, value) in telemetry_values.into_iter().enumerate() {
        write_u64(&mut out, V2_TELEMETRY_WORDS_OFF + index * 8, value);
    }
    write_u32(&mut out, 152, witness.drained_rows);
    write_u32(&mut out, 156, witness.remaining_events);
    write_u32(&mut out, 160, witness.active_callbacks);
    write_u32(&mut out, 164, u32::from(witness.queue_drain_complete));
    write_u64(&mut out, 168, witness.resolver_publication_sequence);
    write_i32(&mut out, 176, witness.resolver_game_hour_raw);
    write_u32(&mut out, 180, witness.resolver_member_bindings);
    write_u32(&mut out, 184, witness.resolver_route_bindings);
    write_u32(&mut out, 188, witness.resolver_unbound_convoy_clients);
    for (index, value) in [
        witness.telemetry.dropped_pre_hour_unreadable,
        witness.telemetry.dropped_receipt_hour_mismatch,
        witness.telemetry.dropped_post_hour_unreadable,
        witness.telemetry.dropped_post_hour_changed,
        witness.telemetry.dropped_resolver_snapshot_absent,
        witness.telemetry.dropped_resolver_unpublished,
        witness.telemetry.receipt_hour_lag_max,
        witness.telemetry.receipt_hour_lead_max,
    ]
    .into_iter()
    .enumerate()
    {
        write_u64(&mut out, V3_TELEMETRY_WORDS_OFF + index * 8, value);
    }
    write_u64(&mut out, V5_PER_DRAIN_OFF, witness.events_seen_this_drain);
    write_u64(
        &mut out,
        V5_PER_DRAIN_OFF + 8,
        witness.events_emitted_this_drain,
    );
    write_u64(&mut out, V5_PER_DRAIN_OFF + 16, witness.drops_this_drain);
    for (index, value) in witness
        .candidate_subsites
        .as_array()
        .into_iter()
        .enumerate()
    {
        write_u64(&mut out, V5_SUBSITE_OFF + index * 8, value);
    }
    write_u64(
        &mut out,
        V5_RECEIPT_AHEAD_OFF,
        witness.telemetry.dropped_receipt_hour_ahead,
    );
    write_u64(
        &mut out,
        V5_LAG_BEYOND_OFF,
        witness.telemetry.dropped_receipt_lag_beyond,
    );
    write_u64(
        &mut out,
        V7_PRE_HOUR_ADDRESS_OVERFLOW_OFF,
        witness.telemetry.dropped_pre_hour_address_overflow,
    );
    for (index, value) in witness
        .capture_error_subsites
        .as_array()
        .into_iter()
        .enumerate()
    {
        write_u64(&mut out, V7_CAPTURE_ERROR_OFF + index * 8, value);
    }

    for (index, row) in rows.iter().enumerate() {
        validate_row(row)?;
        let base = BATCH_HEADER_LEN + index * BATCH_ROW_LEN;
        let carrier = row.carrier.encode();
        ConvoyTerminalAttributionCarrier::decode(&carrier)?;
        out[base..base + CARRIER_LEN].copy_from_slice(&carrier);
        out[base + 96] = row.d70.kind.code();
        write_u32(&mut out, base + 100, row.d70.ordinal);
        write_u32(&mut out, base + 104, row.d70.before_rows);
        write_u32(&mut out, base + 108, row.d70.after_rows);
        write_u32(&mut out, base + 112, row.d70.before_count);
        write_u32(&mut out, base + 116, row.d70.after_count);
    }
    Ok(out)
}

pub fn decode_convoy_transport_attack_batch(
    bytes: &[u8],
) -> Result<DecodedConvoyTransportAttackBatch, BatchDecodeError> {
    if bytes.len() < BATCH_HEADER_LEN {
        return Err(BatchDecodeError::Length(bytes.len()));
    }
    if read_u32(bytes, 0)? != BATCH_MAGIC {
        return Err(BatchDecodeError::Magic);
    }
    if read_u16(bytes, 4)? != BATCH_VERSION
        || read_u16(bytes, 6)? as usize != BATCH_HEADER_LEN
        || read_u16(bytes, 8)? as usize != BATCH_ROW_LEN
    {
        return Err(BatchDecodeError::VersionOrLayout);
    }
    if read_u16(bytes, 10)? != 0 || read_u32(bytes, 20)? != 0 {
        return Err(BatchDecodeError::Reserved);
    }
    if bytes[V7_RESERVED_OFF..BATCH_HEADER_LEN]
        .iter()
        .any(|byte| *byte != 0)
    {
        return Err(BatchDecodeError::Reserved);
    }
    let flags = read_u32(bytes, 16)?;
    if flags & !BATCH_KNOWN_FLAGS != 0
        || flags & BATCH_FLAG_COMPLETE_EXACT_EVENTS != 0 && flags & BATCH_FLAG_HOOK_INSTALLED == 0
    {
        return Err(BatchDecodeError::Flags);
    }
    let row_count = read_u32(bytes, 12)? as usize;
    if row_count > MAX_BATCH_ROWS {
        return Err(BatchDecodeError::RowCount(row_count));
    }
    let expected = BATCH_HEADER_LEN
        .checked_add(
            row_count
                .checked_mul(BATCH_ROW_LEN)
                .ok_or(BatchDecodeError::SizeOverflow)?,
        )
        .ok_or(BatchDecodeError::SizeOverflow)?;
    if bytes.len() != expected {
        return Err(BatchDecodeError::Length(bytes.len()));
    }

    let telemetry_values = (0..15)
        .map(|index| read_u64(bytes, V2_TELEMETRY_WORDS_OFF + index * 8))
        .collect::<Result<Vec<_>, _>>()?;
    let v3 = (0..V3_TELEMETRY_WORDS)
        .map(|index| read_u64(bytes, V3_TELEMETRY_WORDS_OFF + index * 8))
        .collect::<Result<Vec<_>, _>>()?;
    let read_u64_at = |offset: usize| read_u64(bytes, offset);
    let mut subsite_values = [0u64; V5_SUBSITE_WORDS];
    for (index, value) in subsite_values.iter_mut().enumerate() {
        *value = read_u64_at(V5_SUBSITE_OFF + index * 8)?;
    }
    let candidate_subsites = ConvoyTransportAttackCandidateSubsites::from_array(subsite_values);
    let mut capture_error_values = [0u64; V7_CAPTURE_ERROR_WORDS];
    for (index, value) in capture_error_values.iter_mut().enumerate() {
        *value = read_u64_at(V7_CAPTURE_ERROR_OFF + index * 8)?;
    }
    let capture_error_subsites =
        ConvoyTransportAttackCaptureErrorSubsites::from_array(capture_error_values);
    let witness = ConvoyTransportAttackWitness {
        hook_installed: flags & BATCH_FLAG_HOOK_INSTALLED != 0,
        resolver_ever_partial: flags & BATCH_FLAG_RESOLVER_EVER_PARTIAL != 0,
        queue_drain_complete: match read_u32(bytes, 164)? {
            0 => false,
            1 => true,
            _ => return Err(BatchDecodeError::Reserved),
        },
        producer_sequence_next: read_u64(bytes, 24)?,
        drained_rows: read_u32(bytes, 152)?,
        remaining_events: read_u32(bytes, 156)?,
        active_callbacks: read_u32(bytes, 160)?,
        resolver_publication_sequence: read_u64(bytes, 168)?,
        resolver_game_hour_raw: read_i32(bytes, 176)?,
        resolver_member_bindings: read_u32(bytes, 180)?,
        resolver_route_bindings: read_u32(bytes, 184)?,
        resolver_unbound_convoy_clients: read_u32(bytes, 188)?,
        events_seen_this_drain: read_u64_at(V5_PER_DRAIN_OFF)?,
        events_emitted_this_drain: read_u64_at(V5_PER_DRAIN_OFF + 8)?,
        drops_this_drain: read_u64_at(V5_PER_DRAIN_OFF + 16)?,
        candidate_subsites,
        capture_error_subsites,
        telemetry: ConvoyTransportAttackTelemetry {
            callbacks_seen: telemetry_values[0],
            original_calls: telemetry_values[1],
            events_emitted: telemetry_values[2],
            dropped_no_original: telemetry_values[3],
            dropped_teardown: telemetry_values[4],
            dropped_nested: telemetry_values[5],
            dropped_capture_busy: telemetry_values[6],
            dropped_hour_or_epoch: telemetry_values[7],
            dropped_resolver: telemetry_values[8],
            dropped_candidate: telemetry_values[9],
            dropped_pre_hour_unreadable: v3[0],
            dropped_receipt_hour_mismatch: v3[1],
            dropped_post_hour_unreadable: v3[2],
            dropped_post_hour_changed: v3[3],
            dropped_resolver_snapshot_absent: v3[4],
            dropped_resolver_unpublished: v3[5],
            receipt_hour_lag_max: v3[6],
            receipt_hour_lead_max: v3[7],
            dropped_d70_pre: telemetry_values[10],
            dropped_d70_post: telemetry_values[11],
            dropped_d70_mutation: telemetry_values[12],
            dropped_queue_busy: telemetry_values[13],
            dropped_queue_full: telemetry_values[14],
            dropped_receipt_hour_ahead: read_u64_at(V5_RECEIPT_AHEAD_OFF)?,
            dropped_receipt_lag_beyond: read_u64_at(V5_LAG_BEYOND_OFF)?,
            dropped_pre_hour_address_overflow: read_u64_at(
                V7_PRE_HOUR_ADDRESS_OVERFLOW_OFF,
            )?,
        },
    };

    let mut rows = Vec::with_capacity(row_count);
    for index in 0..row_count {
        let base = BATCH_HEADER_LEN + index * BATCH_ROW_LEN;
        if bytes[base + 97..base + 100].iter().any(|byte| *byte != 0) {
            return Err(BatchDecodeError::Reserved);
        }
        let row = ConvoyTransportAttackRow {
            carrier: ConvoyTerminalAttributionCarrier::decode(&bytes[base..base + CARRIER_LEN])?,
            d70: ConvoyD70MutationProof {
                kind: ConvoyD70MutationKind::decode(bytes[base + 96])?,
                ordinal: read_u32(bytes, base + 100)?,
                before_rows: read_u32(bytes, base + 104)?,
                after_rows: read_u32(bytes, base + 108)?,
                before_count: read_u32(bytes, base + 112)?,
                after_count: read_u32(bytes, base + 116)?,
            },
        };
        validate_row(&row)?;
        rows.push(row);
    }
    validate_producer_order(&rows)?;
    validate_mutation_chain(&rows)?;
    validate_witness(&witness, rows.len())?;
    if witness.exact_events_complete() != (flags & BATCH_FLAG_COMPLETE_EXACT_EVENTS != 0) {
        return Err(BatchDecodeError::InvalidWitness("complete_flag"));
    }
    Ok(DecodedConvoyTransportAttackBatch {
        exact_events_complete: witness.exact_events_complete(),
        witness,
        rows,
    })
}

fn validate_producer_order(rows: &[ConvoyTransportAttackRow]) -> Result<(), BatchDecodeError> {
    if rows
        .windows(2)
        .any(|pair| pair[0].carrier.producer_sequence >= pair[1].carrier.producer_sequence)
    {
        return Err(BatchDecodeError::ProducerOrder);
    }
    Ok(())
}

fn validate_mutation_chain(rows: &[ConvoyTransportAttackRow]) -> Result<(), BatchDecodeError> {
    for pair in rows.windows(2) {
        let previous = pair[0];
        let next = pair[1];
        if next.d70.before_rows != previous.d70.after_rows {
            return Err(BatchDecodeError::MutationChain);
        }
        if next.carrier.game_hour_raw == previous.carrier.game_hour_raw
            && next.carrier.owner_tag_db_id == previous.carrier.owner_tag_db_id
            && next.carrier.killer_tag_db_id == previous.carrier.killer_tag_db_id
            && next.d70.ordinal == previous.d70.ordinal
            && next.d70.before_count != previous.d70.after_count
        {
            return Err(BatchDecodeError::MutationChain);
        }
    }
    Ok(())
}

fn validate_witness(
    witness: &ConvoyTransportAttackWitness,
    row_count: usize,
) -> Result<(), BatchDecodeError> {
    if witness.drained_rows as usize != row_count {
        return Err(BatchDecodeError::InvalidWitness("drained_rows"));
    }
    if !witness.hook_installed {
        if row_count != 0 || *witness != ConvoyTransportAttackWitness::default() {
            return Err(BatchDecodeError::InvalidWitness("uninstalled"));
        }
        return Ok(());
    }
    if witness.producer_sequence_next == 0
        || witness.telemetry.events_emitted >= witness.producer_sequence_next
        || witness.telemetry.original_calls > witness.telemetry.callbacks_seen
        || witness.telemetry.events_emitted > witness.telemetry.original_calls
    {
        return Err(BatchDecodeError::InvalidWitness("counters"));
    }
    if witness.events_seen_this_drain
        != witness
            .events_emitted_this_drain
            .wrapping_add(witness.drops_this_drain)
    {
        // Every hook entry ends at exactly one named drop site or in an
        // enqueue; a batch that violates the identity miscounts its own loss.
        return Err(BatchDecodeError::InvalidWitness("drain_deltas"));
    }
    if witness.candidate_subsites.total() != witness.telemetry.dropped_candidate {
        return Err(BatchDecodeError::InvalidWitness("candidate_subsites"));
    }
    // Every `capture_error` drop came from exactly one `CaptureError` variant,
    // so the leaves partition the aggregate they refine. A batch whose leaves
    // do not add up is claiming a cause it cannot name -- which is the state
    // rc_vanilla was in for all 56 of its lost events.
    if witness.capture_error_subsites.total() != witness.candidate_subsites.capture_error {
        return Err(BatchDecodeError::InvalidWitness("capture_error_subsites"));
    }
    Ok(())
}

fn validate_row(row: &ConvoyTransportAttackRow) -> Result<(), BatchDecodeError> {
    let proof = row.d70;
    match proof.kind {
        ConvoyD70MutationKind::IncrementExisting => {
            if proof.before_rows == 0
                || proof.after_rows != proof.before_rows
                || proof.ordinal >= proof.before_rows
                || proof.before_count == 0
                || proof.after_count
                    != proof.before_count.checked_add(1).ok_or(
                        BatchDecodeError::InvalidMutationProof("increment_count_overflow"),
                    )?
            {
                return Err(BatchDecodeError::InvalidMutationProof("increment_existing"));
            }
        }
        ConvoyD70MutationKind::AppendFirstSeen => {
            if proof.after_rows
                != proof.before_rows.checked_add(1).ok_or(
                    BatchDecodeError::InvalidMutationProof("append_row_count_overflow"),
                )?
                || proof.ordinal != proof.before_rows
                || proof.before_count != 0
                || proof.after_count != 1
            {
                return Err(BatchDecodeError::InvalidMutationProof("append_first_seen"));
            }
        }
    }
    Ok(())
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, BatchDecodeError> {
    let value = bytes
        .get(offset..offset + 2)
        .ok_or(BatchDecodeError::Length(bytes.len()))?;
    Ok(u16::from_le_bytes(value.try_into().unwrap()))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, BatchDecodeError> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or(BatchDecodeError::Length(bytes.len()))?;
    Ok(u32::from_le_bytes(value.try_into().unwrap()))
}

fn read_i32(bytes: &[u8], offset: usize) -> Result<i32, BatchDecodeError> {
    Ok(i32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or(BatchDecodeError::Length(bytes.len()))?
            .try_into()
            .unwrap(),
    ))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, BatchDecodeError> {
    Ok(u64::from_le_bytes(
        bytes
            .get(offset..offset + 8)
            .ok_or(BatchDecodeError::Length(bytes.len()))?
            .try_into()
            .unwrap(),
    ))
}

fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_i32(bytes: &mut [u8], offset: usize, value: i32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convoy_terminal_attribution::{
        CombatEpoch, ConvoyCombatRefusalCause, ConvoyTerminalCombat, ConvoyTerminalRoute,
        EntityHandle, LiveRouteResolution, RouteEndpoint,
        RouteEndpointKind,
        RouteProducerKind, TERMINAL_SUNK_STATE,
    };

    fn row(sequence: u64, kind: ConvoyD70MutationKind) -> ConvoyTransportAttackRow {
        ConvoyTransportAttackRow {
            carrier: ConvoyTerminalAttributionCarrier {
                producer_sequence: sequence,
                game_hour_raw: 61_000_000,
                combat: ConvoyTerminalCombat::Bound(CombatEpoch {
                    combat_id: 21,
                    generation: 4,
                }),
                convoy: EntityHandle {
                    id: 158_065,
                    type_tag: 4_713,
                },
                owner_tag_db_id: 2,
                killer_tag_db_id: 51,
                client: EntityHandle {
                    id: 106_381,
                    type_tag: 4_713,
                },
                convoy_index: 8,
                // A PRE-terminal state: the hook reads this field before the
                // routine it stands inside writes the terminal value into it.
                member_state_at_entry: TERMINAL_SUNK_STATE - 1,
                battle_slot: 8,
                route: ConvoyTerminalRoute::Bound(LiveRouteResolution {
                    route_key: EntityHandle {
                        id: 106_381,
                        type_tag: 4_713,
                    },
                    producer_kind: RouteProducerKind::LendLease,
                    source: RouteEndpoint {
                        kind: RouteEndpointKind::State,
                        id: 282,
                    },
                    destination: RouteEndpoint {
                        kind: RouteEndpointKind::State,
                        id: 64,
                    },
                }),
            },
            d70: match kind {
                ConvoyD70MutationKind::IncrementExisting => ConvoyD70MutationProof {
                    kind,
                    ordinal: 440,
                    before_rows: 441,
                    after_rows: 441,
                    before_count: 2,
                    after_count: 3,
                },
                ConvoyD70MutationKind::AppendFirstSeen => ConvoyD70MutationProof {
                    kind,
                    ordinal: 441,
                    before_rows: 441,
                    after_rows: 442,
                    before_count: 0,
                    after_count: 1,
                },
            },
        }
    }

    fn witness(rows: &[ConvoyTransportAttackRow]) -> ConvoyTransportAttackWitness {
        let emitted = rows
            .last()
            .map(|row| row.carrier.producer_sequence)
            .unwrap_or(0);
        ConvoyTransportAttackWitness {
            hook_installed: true,
            queue_drain_complete: true,
            producer_sequence_next: emitted + 1,
            drained_rows: rows.len() as u32,
            events_seen_this_drain: u64::from(rows.len() as u32),
            events_emitted_this_drain: u64::from(rows.len() as u32),
            resolver_publication_sequence: 3,
            resolver_game_hour_raw: 61_000_000,
            resolver_member_bindings: 4,
            resolver_route_bindings: 2,
            ..ConvoyTransportAttackWitness::default()
        }
    }

    #[test]
    fn cta2_round_trips_exact_event_order() {
        let expected = vec![
            row(7, ConvoyD70MutationKind::IncrementExisting),
            row(8, ConvoyD70MutationKind::AppendFirstSeen),
        ];
        let encoded = encode_convoy_transport_attack_batch(&expected, witness(&expected)).unwrap();
        assert_eq!(encoded.len(), BATCH_HEADER_LEN + 2 * BATCH_ROW_LEN);
        let decoded = decode_convoy_transport_attack_batch(&encoded).unwrap();
        assert!(decoded.exact_events_complete);
        assert_eq!(decoded.rows, expected);
    }

    #[test]
    fn cta2_v3_telemetry_words_round_trip_without_disturbing_v2() {
        let rows = vec![row(7, ConvoyD70MutationKind::IncrementExisting)];
        let mut witness = witness(&rows);
        witness.telemetry.dropped_pre_hour_unreadable = 101;
        witness.telemetry.dropped_receipt_hour_mismatch = 102;
        witness.telemetry.dropped_post_hour_unreadable = 103;
        witness.telemetry.dropped_post_hour_changed = 104;
        witness.telemetry.dropped_resolver_snapshot_absent = 105;
        witness.telemetry.dropped_resolver_unpublished = 106;
        witness.telemetry.receipt_hour_lag_max = 107;
        witness.telemetry.receipt_hour_lead_max = 108;
        witness.telemetry.dropped_hour_or_epoch = 101 + 102 + 103 + 104;
        witness.telemetry.dropped_resolver = 105 + 106;

        let encoded = encode_convoy_transport_attack_batch(&rows, witness).unwrap();
        assert_eq!(
            encoded.len(),
            BATCH_HEADER_LEN + BATCH_ROW_LEN,
            "the header grew to hold the v3 words and the row still follows it"
        );
        let decoded = decode_convoy_transport_attack_batch(&encoded).unwrap();
        let telemetry = decoded.witness.telemetry;

        assert_eq!(telemetry.dropped_pre_hour_unreadable, 101);
        assert_eq!(telemetry.dropped_receipt_hour_mismatch, 102);
        assert_eq!(telemetry.dropped_post_hour_unreadable, 103);
        assert_eq!(telemetry.dropped_post_hour_changed, 104);
        assert_eq!(telemetry.dropped_resolver_snapshot_absent, 105);
        assert_eq!(telemetry.dropped_resolver_unpublished, 106);
        assert_eq!(telemetry.receipt_hour_lag_max, 107);
        assert_eq!(telemetry.receipt_hour_lead_max, 108);
        assert_eq!(telemetry.dropped_hour_or_epoch, 410);
        assert_eq!(telemetry.dropped_resolver, 211);

        assert_eq!(decoded.witness.resolver_publication_sequence, 3);
        assert_eq!(decoded.witness.resolver_game_hour_raw, 61_000_000);
        assert_eq!(decoded.witness.resolver_member_bindings, 4);
        assert_eq!(decoded.witness.resolver_route_bindings, 2);
        assert_eq!(decoded.witness.resolver_unbound_convoy_clients, 0);
        assert_eq!(decoded.witness.drained_rows, 1);
        assert_eq!(decoded.rows, rows);
    }

    /// kills rc_vanilla d90 (319 of 323 frames degraded): an unbound convoy
    /// client is a COVERAGE gap, and a drain that saw no event and lost none
    /// is exhaustive whether or not the resolver could have bound one.
    ///
    /// What production change makes this fail: putting
    /// `resolver_unbound_convoy_clients == 0` back into
    /// `exact_events_complete`.
    #[test]
    fn cta2_a_quiet_drain_is_exhaustive_even_while_clients_are_unbound() {
        let mut quiet = witness(&[]);
        quiet.resolver_unbound_convoy_clients = 22;
        let encoded = encode_convoy_transport_attack_batch(&[], quiet).unwrap();
        let decoded = decode_convoy_transport_attack_batch(&encoded).unwrap();
        assert_eq!(
            decoded.witness.resolver_unbound_convoy_clients, 22,
            "the coverage gap stays on the wire and stays named"
        );
        assert!(
            decoded.exact_events_complete,
            "nothing entered the hook this interval, so the empty event set IS all of them"
        );
    }

    /// The other half of the same law: a drop never HIDES, because every drop
    /// out of `capture_candidate` is counted on a named subsite and the count
    /// is a conjunct.
    ///
    /// This used to be the unbound-client case. Since fix C an unbound client
    /// produces a recorded row with a named refusal cause rather than a drop,
    /// so what this fixture drives is the generic counted-drop path;
    /// `RouteJoin` is one surviving example of a proof that can still refuse a
    /// candidate, and the subsite is what says which.
    ///
    /// What production change makes this fail: taking `drops_this_drain` out
    /// of `exact_events_complete`, or dropping a candidate without counting a
    /// subsite.
    #[test]
    fn cta2_a_counted_candidate_drop_still_disqualifies_the_batch() {
        let mut lost = witness(&[]);
        lost.events_seen_this_drain = 1;
        lost.events_emitted_this_drain = 0;
        lost.drops_this_drain = 1;
        lost.telemetry.callbacks_seen = 1;
        lost.telemetry.original_calls = 1;
        lost.telemetry.dropped_candidate = 1;
        lost.candidate_subsites.capture_error = 1;
        lost.capture_error_subsites.route_join = 1;
        let encoded = encode_convoy_transport_attack_batch(&[], lost).unwrap();
        let decoded = decode_convoy_transport_attack_batch(&encoded).unwrap();
        assert!(
            !decoded.exact_events_complete,
            "a counted drop is what refuses the batch, and it is still counted"
        );
        assert_eq!(
            decoded.witness.capture_error_subsites.route_join, 1,
            "and the wire names WHICH proof refused it"
        );
    }

    /// The witness ARITHMETIC this lane's recount rests on: a drain whose every
    /// event had an unjoinable combat is EXACT, because those events are rows
    /// now and a row is not a drop.
    ///
    /// In `captures/rc_vanilla7`, 12 of the window's 232 batches decoded zero
    /// rows and dropped every event they saw, and 3 more emitted rows and
    /// dropped one each -- all 17 losses on the `combat_join` leaf
    /// (`runs/tp_d90_unbound/FIX.md` section 7). Under this rule those 15
    /// batches emit what they saw and none of them lands on a drop site.
    ///
    /// What production change makes this fail: an encoder or decoder that
    /// refuses a refused-combat row, or an `exact_events_complete` that
    /// consults the combat outcome.
    ///
    /// What it does NOT prove, because `prepare_observation` reads
    /// `ProcessMemory` and no fixture can reach it: that production takes this
    /// path rather than counting a drop. Three other things do. The refusal is
    /// an `Ok` carrier
    /// (`a_member_absent_from_the_epoch_snapshot_is_recorded_with_its_gap_named`),
    /// the capture-error counters are incremented only inside the `Err(error)`
    /// arm and nowhere else
    /// (`the_capture_error_leaf_names_which_proof_refused`), and a combat drop
    /// can no longer be EXPRESSED: the three `CaptureError` variants were
    /// deleted, and `subsite_index`'s exhaustive match refuses to compile if one
    /// is added back without a wire slot of its own (Codex round 4, finding 3;
    /// round 5 corrected what that coverage test does and does not catch).
    #[test]
    fn cta2_a_drain_whose_combats_were_all_refused_is_still_exhaustive() {
        // The two mutation kinds, in the order the chain validator accepts:
        // two `IncrementExisting` rows on one ordinal would need the second to
        // start where the first ended, and an unchained pair is refused by the
        // encoder before this test reaches its assertions (Codex round 3,
        // finding 2).
        let rows: Vec<_> = [
            (7u64, ConvoyD70MutationKind::IncrementExisting),
            (8u64, ConvoyD70MutationKind::AppendFirstSeen),
        ]
        .into_iter()
        .map(|(sequence, kind)| {
            let mut row = row(sequence, kind);
            row.carrier.combat = ConvoyTerminalCombat::Refused(
                ConvoyCombatRefusalCause::MemberAbsentFromEpochSnapshot,
            );
            row
        })
        .collect();
        let mut witness = witness(&rows);
        witness.events_seen_this_drain = 2;
        witness.events_emitted_this_drain = 2;
        let encoded = encode_convoy_transport_attack_batch(&rows, witness).unwrap();
        let decoded = decode_convoy_transport_attack_batch(&encoded).unwrap();
        assert_eq!(decoded.rows, rows);
        assert_eq!(decoded.witness.drops_this_drain, 0);
        assert!(
            decoded.exact_events_complete,
            "the events are on the wire with their combat gap named, so the drain lost nothing"
        );
        for row in &decoded.rows {
            assert_eq!(row.carrier.combat.epoch(), None);
            assert_eq!(
                row.carrier.combat.refusal_cause(),
                Some(ConvoyCombatRefusalCause::MemberAbsentFromEpochSnapshot)
            );
        }
    }

    #[test]
    fn cta2_publishes_rows_captured_under_a_partial_resolver() {
        let expected = vec![
            row(7, ConvoyD70MutationKind::IncrementExisting),
            row(8, ConvoyD70MutationKind::AppendFirstSeen),
        ];
        let mut witness = witness(&expected);
        witness.resolver_unbound_convoy_clients = 1;
        let encoded = encode_convoy_transport_attack_batch(&expected, witness).unwrap();
        let decoded = decode_convoy_transport_attack_batch(&encoded).unwrap();
        assert_eq!(decoded.rows, expected);
        assert!(
            decoded.exact_events_complete,
            "the rows that DID bind are exhaustive for their own interval; the unbound client is a named coverage gap beside them, not a loss"
        );
    }

    #[test]
    fn cta2_carries_the_exact_unbound_convoy_client_count() {
        let rows = vec![row(7, ConvoyD70MutationKind::IncrementExisting)];
        for unbound in [0u32, 1, 9_865, u32::MAX] {
            let mut witness = witness(&rows);
            witness.resolver_unbound_convoy_clients = unbound;
            let encoded = encode_convoy_transport_attack_batch(&rows, witness).unwrap();
            let decoded = decode_convoy_transport_attack_batch(&encoded).unwrap();
            assert_eq!(
                decoded.witness.resolver_unbound_convoy_clients, unbound,
                "unbound count must round-trip exactly"
            );
            assert!(
                decoded.exact_events_complete,
                "the coverage gap is carried, not conflated with exhaustiveness"
            );
        }
    }

    /// The complete flag is not a free-standing byte: it must be exactly what
    /// the witness derives. Tampering with a conjunct without moving the flag
    /// is refused.
    #[test]
    fn cta2_rejects_a_complete_flag_that_the_witness_does_not_derive() {
        let rows = vec![row(7, ConvoyD70MutationKind::IncrementExisting)];
        let mut encoded = encode_convoy_transport_attack_batch(&rows, witness(&rows)).unwrap();
        assert!(
            decode_convoy_transport_attack_batch(&encoded)
                .unwrap()
                .exact_events_complete
        );
        // `active_callbacks` (offset 160) IS a conjunct; raising it without
        // clearing the flag is the forgery this check exists for.
        encoded[160..164].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(
            decode_convoy_transport_attack_batch(&encoded),
            Err(BatchDecodeError::InvalidWitness("complete_flag"))
        );
    }

    /// The `capture_error` leaves must partition the aggregate they refine, or
    /// a batch could claim a cause it cannot name -- which is the state
    /// rc_vanilla was in for all 56 of its lost events.
    #[test]
    fn cta2_rejects_capture_error_leaves_that_do_not_sum_to_their_aggregate() {
        let mut bad = witness(&[]);
        bad.telemetry.dropped_candidate = 3;
        bad.candidate_subsites.capture_error = 3;
        bad.events_seen_this_drain = 3;
        bad.drops_this_drain = 3;
        bad.telemetry.callbacks_seen = 3;
        bad.telemetry.original_calls = 3;
        bad.capture_error_subsites.route_join = 2;
        assert_eq!(
            encode_convoy_transport_attack_batch(&[], bad),
            Err(BatchDecodeError::InvalidWitness("capture_error_subsites"))
        );
        bad.capture_error_subsites.route_key_mismatch = 1;
        let encoded = encode_convoy_transport_attack_batch(&[], bad).unwrap();
        let decoded = decode_convoy_transport_attack_batch(&encoded).unwrap();
        assert_eq!(decoded.witness.capture_error_subsites.route_join, 2);
        assert_eq!(
            decoded.witness.capture_error_subsites.route_key_mismatch,
            1
        );
    }

    /// Law: every drop counter is in `named_drops`, so a message built from it
    /// cannot omit the cause that took the events.
    ///
    /// The literal below has no `..Default::default()` on purpose: a new
    /// `dropped_*` field fails to compile here until it is added, which is the
    /// only thing that keeps the taxonomy from silently going stale.
    #[test]
    fn every_drop_counter_is_named() {
        let telemetry = ConvoyTransportAttackTelemetry {
            callbacks_seen: 900,
            original_calls: 901,
            events_emitted: 902,
            receipt_hour_lag_max: 903,
            receipt_hour_lead_max: 904,
            dropped_no_original: 1,
            dropped_teardown: 2,
            dropped_nested: 3,
            dropped_capture_busy: 4,
            dropped_hour_or_epoch: 5,
            dropped_resolver: 6,
            dropped_candidate: 7,
            dropped_pre_hour_unreadable: 8,
            dropped_pre_hour_address_overflow: 9,
            dropped_receipt_hour_mismatch: 10,
            dropped_receipt_hour_ahead: 11,
            dropped_receipt_lag_beyond: 12,
            dropped_post_hour_unreadable: 13,
            dropped_post_hour_changed: 14,
            dropped_resolver_snapshot_absent: 15,
            dropped_resolver_unpublished: 16,
            dropped_d70_pre: 17,
            dropped_d70_post: 18,
            dropped_d70_mutation: 19,
            dropped_queue_busy: 20,
            dropped_queue_full: 21,
        };
        let named = telemetry.named_drops();
        let mut values: Vec<u64> = named.iter().map(|(_, value)| *value).collect();
        values.sort_unstable();
        assert_eq!(
            values,
            (1..=21).collect::<Vec<u64>>(),
            "every drop field must appear exactly once, and no non-drop field may"
        );
        let mut names: Vec<&str> = named.iter().map(|(name, _)| *name).collect();
        names.sort_unstable();
        let unique = names.len();
        names.dedup();
        assert_eq!(names.len(), unique, "two drop counters share a unit name");
        assert!(
            named
                .iter()
                .all(|(name, _)| name.starts_with("convoy_transport_attack_dropped_")),
            "the names must be the ones the manifest publishes, or a message \
             built from them cannot be joined to a capture"
        );
        assert!(
            ConvoyTransportAttackTelemetry::default()
                .named_drops()
                .iter()
                .all(|(_, value)| *value == 0),
            "a clean session names nothing"
        );
    }

    /// A slot's index addresses the same word in all three directions, so a
    /// counter incremented under `subsite_index` is read back under its own
    /// name.
    ///
    /// The coverage this replaces is compile-time now -- see
    /// `convoy_wire_taxonomy` for what each guard refuses and
    /// `runs/tv_capture_error_taxonomy/compile_guards.txt` for the compiler
    /// saying so. What is left for a test is the arithmetic.
    ///
    /// It does NOT kill a declaration-ordered rewrite of the generated code:
    /// `convoy_subsite_table!` asserts that a subsite slot's declared index IS
    /// its position, so the two orders are the same order and no fixture can
    /// separate them (Codex rounds 1 and 2, finding 7 then 6). What it kills
    /// is the two directions disagreeing at all.
    ///
    /// MUTATION: give `from_array` one wrong fixed slot -- `$field:
    /// values[0]` for every field, or a single transposed pair. The distinct
    /// per-slot values below then do not survive the round trip; a fixture of
    /// ones would.
    #[test]
    fn every_subsite_slot_round_trips_through_its_own_declared_index() {
        let mut capture_error = [0u64; V7_CAPTURE_ERROR_WORDS];
        for (slot, value) in capture_error.iter_mut().enumerate() {
            *value = slot as u64 + 1;
        }
        assert_eq!(
            ConvoyTransportAttackCaptureErrorSubsites::from_array(capture_error).as_array(),
            capture_error,
            "the capture-error window decodes under a different order than it encodes"
        );

        let mut candidate = [0u64; V5_SUBSITE_WORDS];
        for (slot, value) in candidate.iter_mut().enumerate() {
            *value = slot as u64 + 1;
        }
        assert_eq!(
            ConvoyTransportAttackCandidateSubsites::from_array(candidate).as_array(),
            candidate,
            "the candidate window decodes under a different order than it encodes"
        );

        for (index, name) in ConvoyTransportAttackCaptureErrorSubsites::FIELDS
            .iter()
            .enumerate()
        {
            let mut values = [0u64; V7_CAPTURE_ERROR_WORDS];
            values[index] = 1;
            let subsites = ConvoyTransportAttackCaptureErrorSubsites::from_array(values);
            assert_eq!(subsites.total(), 1, "{name} must be summed exactly once");
        }
        for (index, name) in ConvoyTransportAttackCandidateSubsites::FIELDS
            .iter()
            .enumerate()
        {
            let mut values = [0u64; V5_SUBSITE_WORDS];
            values[index] = 1;
            let subsites = ConvoyTransportAttackCandidateSubsites::from_array(values);
            assert_eq!(subsites.total(), 1, "{name} must be summed exactly once");
        }
    }

    #[test]
    fn cta2_a_clean_drain_recovers_after_an_ever_partial_publication() {
        // kills pw_gate_final M-A: one partial publication latched
        // RESOLVER_EVER_PARTIAL for the session and every later batch failed
        // exactness on a scar its own interval never contained (392/393
        // incomplete). The latch is now diagnostic only.
        let rows = vec![row(7, ConvoyD70MutationKind::IncrementExisting)];

        // The first interval is inexact because it LOST an event -- since v7
        // that is the only thing that can make an interval inexact, and it is
        // what "partial" has to mean here for the test to say anything. The
        // unbound count and the latch ride along as the diagnostics they are.
        let mut partial = witness(&rows);
        partial.resolver_unbound_convoy_clients = 2;
        partial.resolver_ever_partial = true;
        partial.events_seen_this_drain += 1;
        partial.drops_this_drain += 1;
        partial.telemetry.callbacks_seen += 1;
        partial.telemetry.original_calls += 1;
        partial.telemetry.dropped_candidate += 1;
        partial.candidate_subsites.capture_error += 1;
        partial.capture_error_subsites.route_join += 1;
        let encoded = encode_convoy_transport_attack_batch(&rows, partial).unwrap();
        assert!(
            !decode_convoy_transport_attack_batch(&encoded)
                .unwrap()
                .exact_events_complete,
            "the partial interval itself is still inexact"
        );

        let mut later = witness(&[]);
        later.producer_sequence_next = rows[0].carrier.producer_sequence;
        later.resolver_ever_partial = true;
        let encoded = encode_convoy_transport_attack_batch(&[], later).unwrap();
        let decoded = decode_convoy_transport_attack_batch(&encoded).unwrap();
        assert!(
            decoded.witness.resolver_ever_partial,
            "latch must round-trip as the session-level diagnostic it is"
        );
        assert!(
            decoded.exact_events_complete,
            "a drain whose own interval was fully bound and lossless proves its own batch"
        );
    }

    #[test]
    fn cta2_lifetime_scars_do_not_poison_a_clean_drain() {
        // kills pw_gate_final M-B: cumulative telemetry embedded in every
        // batch meant one dropped event anywhere made every later batch fail
        // callbacks_seen==events_emitted and has_drop forever. Exactness now
        // reads only this drain's deltas.
        let mut scarred = witness(&[]);
        scarred.telemetry.callbacks_seen = 71;
        scarred.telemetry.original_calls = 71;
        scarred.telemetry.events_emitted = 0;
        scarred.telemetry.dropped_candidate = 46;
        scarred.telemetry.dropped_receipt_hour_mismatch = 25;
        scarred.candidate_subsites.initial_gate = 21;
        scarred.candidate_subsites.context_mismatch = 25;
        let encoded = encode_convoy_transport_attack_batch(&[], scarred).unwrap();
        let decoded = decode_convoy_transport_attack_batch(&encoded).unwrap();
        assert_eq!(decoded.witness.telemetry.dropped_candidate, 46);
        assert!(decoded.exact_events_complete);
    }

    #[test]
    fn cta2_a_per_drain_drop_disqualifies_exactness() {
        // kills the residual-loss hole the lifetime version had: drops are
        // judged per interval, so a drop in THIS interval must make THIS
        // batch inexact even though nothing was ever dropped before.
        let mut dropped = witness(&[]);
        dropped.events_seen_this_drain = 1;
        dropped.drops_this_drain = 1;
        dropped.telemetry.dropped_candidate = 1;
        dropped.candidate_subsites.capture_error = 1;
        // The leaves partition the aggregate, so naming the aggregate without
        // naming which proof refused does not encode at all.
        dropped.capture_error_subsites.route_join = 1;
        let encoded = encode_convoy_transport_attack_batch(&[], dropped).unwrap();
        assert!(
            !decode_convoy_transport_attack_batch(&encoded)
                .unwrap()
                .exact_events_complete
        );
    }

    #[test]
    fn cta2_rejects_drain_delta_identity_violations() {
        // seen != emitted + drops miscounts the interval's own loss; encode
        // and decode both refuse it.
        let mut bad = ConvoyTransportAttackWitness {
            hook_installed: true,
            queue_drain_complete: true,
            producer_sequence_next: 1,
            ..ConvoyTransportAttackWitness::default()
        };
        bad.events_seen_this_drain = 1;
        assert_eq!(
            encode_convoy_transport_attack_batch(&[], bad),
            Err(BatchDecodeError::InvalidWitness("drain_deltas"))
        );
    }

    #[test]
    fn cta2_rejects_subsite_totals_that_disagree_with_the_candidate_counter() {
        let mut bad = witness(&[]);
        bad.telemetry.dropped_candidate = 2;
        bad.candidate_subsites.initial_gate = 1;
        assert_eq!(
            encode_convoy_transport_attack_batch(&[], bad),
            Err(BatchDecodeError::InvalidWitness("candidate_subsites"))
        );
    }

    #[test]
    fn cta2_refuses_earlier_batch_versions_by_version() {
        let rows = [row(7, ConvoyD70MutationKind::IncrementExisting)];
        let mut encoded = encode_convoy_transport_attack_batch(&rows, witness(&rows)).unwrap();
        // Every earlier version disagrees with the current one about something
        // a reader
        // acts on. v7 shares this header byte for byte but its ROWS carry a
        // carrier whose route is a precondition and whose member state has the
        // opposite sense, so a v7 batch must be refused rather than
        // reinterpreted; v5 disagrees about the complete flag as well; v6 is
        // refused because lane RX spent that number on a superseded header
        // layout rather than reuse it.
        // v9 shares nothing a reader may act on: its rows carry a CTA1 v3
        // carrier whose combat epoch is a precondition, and three of its
        // fifteen capture-error leaves no longer exist, so the whole header
        // tail sits at different offsets.
        write_u16(&mut encoded, 4, 9);
        assert_eq!(
            decode_convoy_transport_attack_batch(&encoded),
            Err(BatchDecodeError::VersionOrLayout)
        );
        write_u16(&mut encoded, 4, 8);
        assert_eq!(
            decode_convoy_transport_attack_batch(&encoded),
            Err(BatchDecodeError::VersionOrLayout)
        );
        write_u16(&mut encoded, 4, 7);
        assert_eq!(
            decode_convoy_transport_attack_batch(&encoded),
            Err(BatchDecodeError::VersionOrLayout)
        );
        write_u16(&mut encoded, 4, 6);
        assert_eq!(
            decode_convoy_transport_attack_batch(&encoded),
            Err(BatchDecodeError::VersionOrLayout)
        );
        write_u16(&mut encoded, 4, 5);
        assert_eq!(
            decode_convoy_transport_attack_batch(&encoded),
            Err(BatchDecodeError::VersionOrLayout)
        );
        write_u16(&mut encoded, 4, 4);
        assert_eq!(
            decode_convoy_transport_attack_batch(&encoded),
            Err(BatchDecodeError::VersionOrLayout)
        );
    }

    #[test]
    fn cta2_receipt_hour_ahead_round_trips_and_counts_as_a_drop() {
        let mut ahead = witness(&[]);
        ahead.events_seen_this_drain = 1;
        ahead.drops_this_drain = 1;
        ahead.telemetry.dropped_receipt_hour_ahead = 1;
        ahead.telemetry.dropped_hour_or_epoch = 1;
        let encoded = encode_convoy_transport_attack_batch(&[], ahead).unwrap();
        let decoded = decode_convoy_transport_attack_batch(&encoded).unwrap();
        assert_eq!(decoded.witness.telemetry.dropped_receipt_hour_ahead, 1);
        assert!(!decoded.exact_events_complete);
        assert!(decoded.witness.telemetry.has_drop());
    }

    #[test]
    fn cta2_rejects_reordered_or_duplicate_producer_sequences() {
        let rows = vec![
            row(8, ConvoyD70MutationKind::IncrementExisting),
            row(8, ConvoyD70MutationKind::AppendFirstSeen),
        ];
        assert_eq!(
            encode_convoy_transport_attack_batch(&rows, witness(&rows)),
            Err(BatchDecodeError::ProducerOrder)
        );
    }

    #[test]
    fn cta2_rejects_forged_d70_mutation() {
        let expected = row(7, ConvoyD70MutationKind::IncrementExisting);
        let rows = [expected];
        let mut encoded = encode_convoy_transport_attack_batch(&rows, witness(&rows)).unwrap();
        write_u32(&mut encoded, BATCH_HEADER_LEN + 116, 9);
        assert_eq!(
            decode_convoy_transport_attack_batch(&encoded),
            Err(BatchDecodeError::InvalidMutationProof("increment_existing"))
        );
    }

    #[test]
    fn cta2_rejects_reserved_bytes_and_trailing_data() {
        let expected = row(7, ConvoyD70MutationKind::AppendFirstSeen);
        let rows = [expected];
        let mut encoded = encode_convoy_transport_attack_batch(&rows, witness(&rows)).unwrap();
        encoded[BATCH_HEADER_LEN + 97] = 1;
        assert_eq!(
            decode_convoy_transport_attack_batch(&encoded),
            Err(BatchDecodeError::Reserved)
        );
        // The HEADER tail, which the row case above cannot reach. Both ends of
        // the v7 window, so moving the boundary in either direction fails here.
        for offset in [V7_RESERVED_OFF, BATCH_HEADER_LEN - 1] {
            let mut encoded = encode_convoy_transport_attack_batch(&rows, witness(&rows)).unwrap();
            encoded[offset] = 1;
            assert_eq!(
                decode_convoy_transport_attack_batch(&encoded),
                Err(BatchDecodeError::Reserved),
                "header byte {offset} is reserved"
            );
        }
        // And the word immediately below it is a FIELD, not reserved: the
        // eight bytes at `V7_PRE_HOUR_ADDRESS_OVERFLOW_OFF` are
        // `dropped_pre_hour_address_overflow`. A decoder that
        // reserved it would refuse that word with `Reserved` instead of reading
        // it -- whatever it then makes of the accounting is a different verdict.
        let mut encoded = encode_convoy_transport_attack_batch(&rows, witness(&rows)).unwrap();
        encoded[V7_PRE_HOUR_ADDRESS_OVERFLOW_OFF] = 1;
        assert_ne!(
            decode_convoy_transport_attack_batch(&encoded),
            Err(BatchDecodeError::Reserved)
        );
        let mut encoded = encode_convoy_transport_attack_batch(&rows, witness(&rows)).unwrap();
        encoded.push(0);
        assert_eq!(
            decode_convoy_transport_attack_batch(&encoded),
            Err(BatchDecodeError::Length(
                BATCH_HEADER_LEN + BATCH_ROW_LEN + 1
            ))
        );
    }

    #[test]
    fn cta2_marks_any_drop_or_queue_residue_incomplete() {
        let rows = [row(7, ConvoyD70MutationKind::IncrementExisting)];
        let mut observed = witness(&rows);
        observed.telemetry.dropped_queue_full = 1;
        observed.telemetry.callbacks_seen += 1;
        observed.telemetry.original_calls += 1;
        observed.producer_sequence_next += 1;
        observed.remaining_events = 1;
        let encoded = encode_convoy_transport_attack_batch(&rows, observed).unwrap();
        let decoded = decode_convoy_transport_attack_batch(&encoded).unwrap();
        assert!(!decoded.exact_events_complete);
    }

    #[test]
    fn cta2_accepts_counter_anchored_empty_before_and_after_resolver_publication() {
        let prepublication = ConvoyTransportAttackWitness {
            hook_installed: true,
            queue_drain_complete: true,
            producer_sequence_next: 1,
            ..ConvoyTransportAttackWitness::default()
        };
        let encoded = encode_convoy_transport_attack_batch(&[], prepublication).unwrap();
        assert!(
            decode_convoy_transport_attack_batch(&encoded)
                .unwrap()
                .exact_events_complete
        );

        let postpublication = ConvoyTransportAttackWitness {
            resolver_publication_sequence: 7,
            resolver_game_hour_raw: 61_000_000,
            ..prepublication
        };
        let encoded = encode_convoy_transport_attack_batch(&[], postpublication).unwrap();
        assert!(
            decode_convoy_transport_attack_batch(&encoded)
                .unwrap()
                .exact_events_complete
        );

        // A drop in THIS interval disqualifies this batch. The lifetime
        // counter rides along as the diagnostic it is; the exactness verdict
        // comes from the per-drain deltas.
        let mut dropped = prepublication;
        dropped.events_seen_this_drain = 1;
        dropped.drops_this_drain = 1;
        dropped.telemetry.callbacks_seen = 1;
        dropped.telemetry.original_calls = 1;
        dropped.telemetry.dropped_resolver = 1;
        assert!(
            !decode_convoy_transport_attack_batch(
                &encode_convoy_transport_attack_batch(&[], dropped).unwrap(),
            )
            .unwrap()
            .exact_events_complete
        );
    }

    #[test]
    fn cta2_rejects_broken_d70_chain() {
        let mut rows = vec![
            row(7, ConvoyD70MutationKind::IncrementExisting),
            row(8, ConvoyD70MutationKind::AppendFirstSeen),
        ];
        rows[1].d70.before_rows = 440;
        assert_eq!(
            encode_convoy_transport_attack_batch(&rows, witness(&rows)),
            Err(BatchDecodeError::MutationChain)
        );
    }
}
