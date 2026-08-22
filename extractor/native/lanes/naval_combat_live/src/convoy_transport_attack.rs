use crate::convoy_terminal_attribution::{
    CarrierDecodeError, ConvoyTerminalAttributionCarrier, CARRIER_LEN,
};

pub const DOMAIN_CONVOY_TRANSPORT_ATTACK: u16 = 90;
pub const DOMAIN_CONVOY_TRANSPORT_ATTACK_NAME: &str = "convoy_transport_attack_live";

pub const BATCH_MAGIC: u32 = u32::from_le_bytes(*b"CTA2");
pub const BATCH_VERSION: u16 = 5;
pub const BATCH_HEADER_LEN: usize = 384;
const V2_TELEMETRY_WORDS_OFF: usize = 32;
const V3_TELEMETRY_WORDS_OFF: usize = 192;
const V3_TELEMETRY_WORDS: usize = 8;
// v5 appends the per-drain facts that exactness is now judged from. The v3/v4
// lifetime counters stay on the wire verbatim as diagnostics; they no longer
// decide anything (pw_gate_final: one early incident poisoned all 392 batches).
const V5_PER_DRAIN_OFF: usize = 256;
const V5_SUBSITE_OFF: usize = 280;
const V5_SUBSITE_WORDS: usize = 8;
const V5_RECEIPT_AHEAD_OFF: usize = 344;
const V5_LAG_BEYOND_OFF: usize = 352;
const V5_RESERVED_OFF: usize = 360;
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

impl ConvoyD70MutationKind {
    fn decode(value: u8) -> Result<Self, BatchDecodeError> {
        match value {
            1 => Ok(Self::IncrementExisting),
            2 => Ok(Self::AppendFirstSeen),
            _ => Err(BatchDecodeError::InvalidMutationKind(value)),
        }
    }
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
}

impl ConvoyTransportAttackTelemetry {
    pub fn has_drop(&self) -> bool {
        [
            self.dropped_no_original,
            self.dropped_teardown,
            self.dropped_nested,
            self.dropped_capture_busy,
            self.dropped_hour_or_epoch,
            self.dropped_resolver,
            self.dropped_candidate,
            self.dropped_d70_pre,
            self.dropped_d70_post,
            self.dropped_d70_mutation,
            self.dropped_queue_busy,
            self.dropped_queue_full,
            self.dropped_receipt_hour_ahead,
            self.dropped_receipt_lag_beyond,
        ]
        .into_iter()
        .any(|value| value != 0)
    }
}

/// Where a DROP_CANDIDATE came from. The aggregate `dropped_candidate` could
/// not say, so pw_gate_final's 46 candidate drops stayed unattributable and
/// every one of them was unfixable by construction. These are LIFETIME
/// counters like `dropped_candidate` and must sum to it exactly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConvoyTransportAttackCandidateSubsites {
    pub initial_gate: u64,
    pub producer_sequence_exhausted: u64,
    pub capture_error: u64,
    /// The A729 context bytes did not read or did not match.
    pub context_mismatch: u64,
    /// The context ADDRESS itself overflowed (previously a silent uncounted
    /// loss path).
    pub context_address_overflow: u64,
    pub member_bytes_unreadable: u64,
    pub route_reread_mismatch: u64,
    pub slot_busy: u64,
}

impl ConvoyTransportAttackCandidateSubsites {
    pub fn total(&self) -> u64 {
        self.initial_gate
            .wrapping_add(self.producer_sequence_exhausted)
            .wrapping_add(self.capture_error)
            .wrapping_add(self.context_mismatch)
            .wrapping_add(self.context_address_overflow)
            .wrapping_add(self.member_bytes_unreadable)
            .wrapping_add(self.route_reread_mismatch)
            .wrapping_add(self.slot_busy)
    }

    pub const FIELDS: [&'static str; 8] = [
        "convoy_transport_attack_dropped_candidate_initial_gate",
        "convoy_transport_attack_dropped_candidate_producer_sequence_exhausted",
        "convoy_transport_attack_dropped_candidate_capture_error",
        "convoy_transport_attack_dropped_candidate_context_mismatch",
        "convoy_transport_attack_dropped_candidate_context_address_overflow",
        "convoy_transport_attack_dropped_candidate_member_bytes_unreadable",
        "convoy_transport_attack_dropped_candidate_route_reread_mismatch",
        "convoy_transport_attack_dropped_candidate_slot_busy",
    ];

    pub fn as_array(&self) -> [u64; 8] {
        [
            self.initial_gate,
            self.producer_sequence_exhausted,
            self.capture_error,
            self.context_mismatch,
            self.context_address_overflow,
            self.member_bytes_unreadable,
            self.route_reread_mismatch,
            self.slot_busy,
        ]
    }
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
    pub telemetry: ConvoyTransportAttackTelemetry,
}

impl ConvoyTransportAttackWitness {
    /// Whether THIS batch proves its event set exhaustive. Every conjunct is
    /// a fact of this drain interval or a point-in-time reading taken under
    /// the drain's own correlation guard -- nothing session-cumulative
    /// decides it (pw_gate_final: 392/393 batches failed on lifetime scars
    /// from incidents their own intervals never contained).
    pub fn exact_events_complete(&self) -> bool {
        self.hook_installed
            && self.queue_drain_complete
            && self.remaining_events == 0
            && self.active_callbacks == 0
            && self.resolver_unbound_convoy_clients == 0
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

    for (index, row) in rows.iter().enumerate() {
        validate_row(row)?;
        let base = BATCH_HEADER_LEN + index * BATCH_ROW_LEN;
        let carrier = row.carrier.encode();
        ConvoyTerminalAttributionCarrier::decode(&carrier)?;
        out[base..base + CARRIER_LEN].copy_from_slice(&carrier);
        out[base + 96] = row.d70.kind as u8;
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
    if bytes[V5_RESERVED_OFF..BATCH_HEADER_LEN]
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
    let candidate_subsites = ConvoyTransportAttackCandidateSubsites {
        initial_gate: subsite_values[0],
        producer_sequence_exhausted: subsite_values[1],
        capture_error: subsite_values[2],
        context_mismatch: subsite_values[3],
        context_address_overflow: subsite_values[4],
        member_bytes_unreadable: subsite_values[5],
        route_reread_mismatch: subsite_values[6],
        slot_busy: subsite_values[7],
    };
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
        CombatEpoch, EntityHandle, LiveRouteResolution, RouteEndpoint, RouteEndpointKind,
        RouteProducerKind, TERMINAL_SUNK_STATE,
    };

    fn row(sequence: u64, kind: ConvoyD70MutationKind) -> ConvoyTransportAttackRow {
        ConvoyTransportAttackRow {
            carrier: ConvoyTerminalAttributionCarrier {
                producer_sequence: sequence,
                game_hour_raw: 61_000_000,
                combat: CombatEpoch {
                    combat_id: 21,
                    generation: 4,
                },
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
                member_state: TERMINAL_SUNK_STATE,
                battle_slot: 8,
                route: LiveRouteResolution {
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
                },
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
        assert!(!decoded.exact_events_complete);
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
            assert_eq!(
                decoded.exact_events_complete,
                unbound == 0,
                "a partial resolver must disqualify the exhaustive claim"
            );
        }
    }

    #[test]
    fn cta2_rejects_a_complete_flag_over_a_partial_resolver() {
        let rows = vec![row(7, ConvoyD70MutationKind::IncrementExisting)];
        let mut encoded = encode_convoy_transport_attack_batch(&rows, witness(&rows)).unwrap();
        assert!(
            decode_convoy_transport_attack_batch(&encoded)
                .unwrap()
                .exact_events_complete
        );
        encoded[188..192].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(
            decode_convoy_transport_attack_batch(&encoded),
            Err(BatchDecodeError::InvalidWitness("complete_flag"))
        );
    }

    #[test]
    fn cta2_a_clean_drain_recovers_after_an_ever_partial_publication() {
        // kills pw_gate_final M-A: one partial publication latched
        // RESOLVER_EVER_PARTIAL for the session and every later batch failed
        // exactness on a scar its own interval never contained (392/393
        // incomplete). The latch is now diagnostic only.
        let rows = vec![row(7, ConvoyD70MutationKind::IncrementExisting)];

        let mut partial = witness(&rows);
        partial.resolver_unbound_convoy_clients = 2;
        partial.resolver_ever_partial = true;
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
        scarred.candidate_subsites.route_reread_mismatch = 25;
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
    fn cta2_refuses_pre_v5_batches_by_version() {
        let rows = [row(7, ConvoyD70MutationKind::IncrementExisting)];
        let mut encoded = encode_convoy_transport_attack_batch(&rows, witness(&rows)).unwrap();
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
