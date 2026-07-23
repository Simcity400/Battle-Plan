//! Pointer-free compact transport for authenticated convoy terminal attacks.
//!
//! `CTA1` is the exact single-event carrier captured at the pre-collapse
//! terminal producer. `CTA2` is the domain-90 batch envelope consumed by the
//! collector. It preserves producer order and the exact same-hour D70 mutation
//! proof; it never serializes a process pointer or reconstructs a route from a
//! terminal monthly bucket.

use crate::convoy_terminal_attribution::{
    CarrierDecodeError, ConvoyTerminalAttributionCarrier, CARRIER_LEN,
};

pub const DOMAIN_CONVOY_TRANSPORT_ATTACK: u16 = 90;
pub const DOMAIN_CONVOY_TRANSPORT_ATTACK_NAME: &str = "convoy_transport_attack_live";

pub const BATCH_MAGIC: u32 = u32::from_le_bytes(*b"CTA2");
pub const BATCH_VERSION: u16 = 1;
pub const BATCH_HEADER_LEN: usize = 192;
pub const BATCH_ROW_LEN: usize = 120;
pub const BATCH_FLAG_COMPLETE_EXACT_EVENTS: u32 = 1 << 0;
pub const BATCH_FLAG_HOOK_INSTALLED: u32 = 1 << 1;
pub const BATCH_KNOWN_FLAGS: u32 =
    BATCH_FLAG_COMPLETE_EXACT_EVENTS | BATCH_FLAG_HOOK_INSTALLED;
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
    pub dropped_d70_pre: u64,
    pub dropped_d70_post: u64,
    pub dropped_d70_mutation: u64,
    pub dropped_queue_busy: u64,
    pub dropped_queue_full: u64,
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
        ]
        .into_iter()
        .any(|value| value != 0)
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
    pub telemetry: ConvoyTransportAttackTelemetry,
}

impl ConvoyTransportAttackWitness {
    pub fn exact_events_complete(&self) -> bool {
        self.hook_installed
            && self.queue_drain_complete
            && self.remaining_events == 0
            && self.active_callbacks == 0
            // A freshly installed observer can produce a complete measured-
            // empty drain before the first d49/d50 resolver publication. Any
            // callback in that interval increments a drop counter, so only a
            // genuinely zero-callback drain may omit resolver custody.
            && (self.telemetry.callbacks_seen == 0
                || self.resolver_publication_sequence != 0)
            && self.telemetry.callbacks_seen == self.telemetry.original_calls
            && self.telemetry.callbacks_seen == self.telemetry.events_emitted
            && self.producer_sequence_next
                == self.telemetry.events_emitted.checked_add(1).unwrap_or(0)
            && !self.telemetry.has_drop()
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

/// Encode one complete drain in callback producer order. The caller must not
/// sort rows: `producer_sequence` is the durable event ordinal.
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
        | u32::from(witness.hook_installed) * BATCH_FLAG_HOOK_INSTALLED;
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
        write_u64(&mut out, 32 + index * 8, value);
    }
    write_u32(&mut out, 152, witness.drained_rows);
    write_u32(&mut out, 156, witness.remaining_events);
    write_u32(&mut out, 160, witness.active_callbacks);
    write_u32(&mut out, 164, u32::from(witness.queue_drain_complete));
    write_u64(&mut out, 168, witness.resolver_publication_sequence);
    write_i32(&mut out, 176, witness.resolver_game_hour_raw);
    write_u32(&mut out, 180, witness.resolver_member_bindings);
    write_u32(&mut out, 184, witness.resolver_route_bindings);

    for (index, row) in rows.iter().enumerate() {
        validate_row(row)?;
        let base = BATCH_HEADER_LEN + index * BATCH_ROW_LEN;
        let carrier = row.carrier.encode();
        // CTA1 encode is infallible, so round-trip validation here prevents an
        // invalid in-memory value from entering a production payload.
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
    if read_u16(bytes, 10)? != 0
        || read_u32(bytes, 20)? != 0
        || read_u32(bytes, 188)? != 0
    {
        return Err(BatchDecodeError::Reserved);
    }
    let flags = read_u32(bytes, 16)?;
    if flags & !BATCH_KNOWN_FLAGS != 0
        || flags & BATCH_FLAG_COMPLETE_EXACT_EVENTS != 0
            && flags & BATCH_FLAG_HOOK_INSTALLED == 0
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
        .map(|index| read_u64(bytes, 32 + index * 8))
        .collect::<Result<Vec<_>, _>>()?;
    let witness = ConvoyTransportAttackWitness {
        hook_installed: flags & BATCH_FLAG_HOOK_INSTALLED != 0,
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
            dropped_d70_pre: telemetry_values[10],
            dropped_d70_post: telemetry_values[11],
            dropped_d70_mutation: telemetry_values[12],
            dropped_queue_busy: telemetry_values[13],
            dropped_queue_full: telemetry_values[14],
        },
    };

    let mut rows = Vec::with_capacity(row_count);
    for index in 0..row_count {
        let base = BATCH_HEADER_LEN + index * BATCH_ROW_LEN;
        if bytes[base + 97..base + 100].iter().any(|byte| *byte != 0) {
            return Err(BatchDecodeError::Reserved);
        }
        let row = ConvoyTransportAttackRow {
            carrier: ConvoyTerminalAttributionCarrier::decode(
                &bytes[base..base + CARRIER_LEN],
            )?,
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
    if witness.exact_events_complete()
        != (flags & BATCH_FLAG_COMPLETE_EXACT_EVENTS != 0)
    {
        return Err(BatchDecodeError::InvalidWitness("complete_flag"));
    }
    Ok(DecodedConvoyTransportAttackBatch {
        exact_events_complete: witness.exact_events_complete(),
        witness,
        rows,
    })
}

fn validate_producer_order(rows: &[ConvoyTransportAttackRow]) -> Result<(), BatchDecodeError> {
    if rows.windows(2).any(|pair| {
        pair[0].carrier.producer_sequence >= pair[1].carrier.producer_sequence
    }) {
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
                || proof.after_count != proof.before_count.checked_add(1).ok_or(
                    BatchDecodeError::InvalidMutationProof("increment_count_overflow"),
                )?
            {
                return Err(BatchDecodeError::InvalidMutationProof(
                    "increment_existing",
                ));
            }
        }
        ConvoyD70MutationKind::AppendFirstSeen => {
            if proof.after_rows != proof.before_rows.checked_add(1).ok_or(
                BatchDecodeError::InvalidMutationProof("append_row_count_overflow"),
            )?
                || proof.ordinal != proof.before_rows
                || proof.before_count != 0
                || proof.after_count != 1
            {
                return Err(BatchDecodeError::InvalidMutationProof(
                    "append_first_seen",
                ));
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
            remaining_events: 0,
            active_callbacks: 0,
            resolver_publication_sequence: 3,
            resolver_game_hour_raw: 61_000_000,
            resolver_member_bindings: 4,
            resolver_route_bindings: 2,
            telemetry: ConvoyTransportAttackTelemetry {
                callbacks_seen: emitted,
                original_calls: emitted,
                events_emitted: emitted,
                ..ConvoyTransportAttackTelemetry::default()
            },
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
            Err(BatchDecodeError::InvalidMutationProof(
                "increment_existing"
            ))
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
            Err(BatchDecodeError::Length(BATCH_HEADER_LEN + BATCH_ROW_LEN + 1))
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
    fn cta2_accepts_zero_callback_prepublication_drain_only() {
        let observed = ConvoyTransportAttackWitness {
            hook_installed: true,
            queue_drain_complete: true,
            producer_sequence_next: 1,
            ..ConvoyTransportAttackWitness::default()
        };
        let encoded = encode_convoy_transport_attack_batch(&[], observed).unwrap();
        assert!(decode_convoy_transport_attack_batch(&encoded)
            .unwrap()
            .exact_events_complete);

        let mut dropped = observed;
        dropped.telemetry.callbacks_seen = 1;
        dropped.telemetry.original_calls = 1;
        dropped.telemetry.dropped_resolver = 1;
        assert!(!decode_convoy_transport_attack_batch(
            &encode_convoy_transport_attack_batch(&[], dropped).unwrap(),
        )
        .unwrap()
        .exact_events_complete);
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
