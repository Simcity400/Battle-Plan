//! d21 `land_combats_live`: the full active land-combat snapshot.
//!
//! This boundary is derived from the shipping fold in
//! `world_fold.rs:20318`-`:20513` and the producer in
//! `collector/src/attach_log.rs:72387`-`:72454`.
// @world-fold-authority domain=21 functions=fold_land_combats lines=20318-20513 sha256=fbae0ff6117a6139916aaae661257bcd41ed720250be24e7eae50839dfe2a08e
//!
//! The shipping order is significant: the fold validates the 32-byte header,
//! reserved word, and exact payload length before consulting `status_flags`.
//! A well-formed failure-status record is then excluded without walking its
//! rows. Callers therefore run [`split_payload`] on every record, classify the
//! status through `compact_contract`, and call [`decode_success_payload`] only
//! when the status is not a failure.
//!
//! The fold retains combat identity, optional location, and the two optional
//! participant lists. It validates `day` and `duration` packing but drops both
//! words. They are consequently boundary invariants rather than portable state
//! fields. This is a shipping-fold fidelity gap, not permission for a mapper to
//! reinterpret either word.

use std::collections::{BTreeMap, BTreeSet};

use crate::body::{key, Key};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 21;
pub const COMBATS: u32 = 1;
pub const PARTICIPANTS: u32 = 2;

pub const COMBATS_INDEX: usize = 0;
pub const PARTICIPANTS_INDEX: usize = 1;
pub const COLLECTION_COUNT: usize = 2;

pub mod field {
    pub const COMBAT_ID: u32 = 1;
    pub const GENERATION: u32 = 2;
    pub const LOCATION: u32 = 3;
    pub const ATTACKERS_KNOWN: u32 = 4;
    pub const DEFENDERS_KNOWN: u32 = 5;

    pub const SIDE: u32 = 6;
    pub const DIVISION_ID: u32 = 7;
    pub const PARTICIPANT_ORDINAL: u32 = 8;
}

pub const HEADER_LEN: usize = 32;
pub const COMBAT_ROW_LEN: usize = 40;
pub const PARTICIPANT_ROW_LEN: usize = 24;

pub const FLAG_HAS_LOCATION: u32 = 1 << 0;
pub const FLAG_HAS_DAY: u32 = 1 << 1;
pub const FLAG_HAS_DURATION: u32 = 1 << 2;
pub const FLAG_ATTACKERS_EXACT: u32 = 1 << 3;
pub const FLAG_DEFENDERS_EXACT: u32 = 1 << 4;
pub const KNOWN_COMBAT_FLAGS: u32 = FLAG_HAS_LOCATION
    | FLAG_HAS_DAY
    | FLAG_HAS_DURATION
    | FLAG_ATTACKERS_EXACT
    | FLAG_DEFENDERS_EXACT;

pub const SIDE_ATTACKER: u32 = 1;
pub const SIDE_DEFENDER: u32 = 2;

const NONCANONICAL_SCHEMA: &str =
    "land combats schema does not match the canonical d21 declaration";
const NONCANONICAL_COMBATS_COLLECTION: &str =
    "land combats combat collection does not match the canonical d21 declaration";
const NONCANONICAL_PARTICIPANTS_COLLECTION: &str =
    "land combats participant collection does not match the canonical d21 declaration";

pub fn schema() -> Schema {
    use field as f;

    let combats = Collection {
        collection_id: COMBATS,
        shape: Shape::KeyedSet,
        fields: vec![
            Field::new(f::COMBAT_ID, Kind::Unsigned).key(0),
            Field::new(f::GENERATION, Kind::Unsigned).key(1),
            // A combat always has a location in the current exact-a729
            // collector. The shipping grammar nevertheless admits a clear gate;
            // that means applicable state was not captured, not "no location".
            Field::new(f::LOCATION, Kind::ContentId).unknown_capable(),
            // The participant rows alone cannot distinguish a measured empty
            // side from a side whose membership was not captured. Keep the two
            // exactness witnesses the fold represents as Option<Vec<_>>.
            Field::new(f::ATTACKERS_KNOWN, Kind::Boolean),
            Field::new(f::DEFENDERS_KNOWN, Kind::Boolean),
        ],
    };
    let participants = Collection {
        collection_id: PARTICIPANTS,
        // The fold rejects a repeated `(combat, generation, side, division)`
        // identity, so this is a keyed membership table. Its Vec order is local
        // to each `(combat, generation, side)` parent, not one meaningful global
        // list: carry that local ordinal as a value rather than inventing
        // continuity between different combats' physical row runs.
        shape: Shape::KeyedSet,
        fields: vec![
            Field::new(f::COMBAT_ID, Kind::Unsigned).key(0),
            Field::new(f::GENERATION, Kind::Unsigned).key(1),
            Field::new(f::SIDE, Kind::Unsigned).key(2),
            Field::new(f::DIVISION_ID, Kind::Unsigned).key(3),
            Field::new(f::PARTICIPANT_ORDINAL, Kind::Unsigned),
        ],
    };

    Schema {
        domain: DOMAIN,
        cadence_log2: 0,
        collections: vec![combats, participants],
        event_part: None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub elapsed_wall_ms: u64,
    pub combat_count: u32,
    pub changed_rows: u32,
    pub status_flags: u32,
    pub participant_count: u32,
    pub ended_count: u32,
}

/// Apply the structural checks the shipping fold performs before status
/// classification and split the two fixed-width row regions.
pub fn split_payload(
    payload: &[u8],
) -> core::result::Result<(PayloadHeader, &[u8], &[u8]), String> {
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "land combats payload {} shorter than the {HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    let header = PayloadHeader {
        elapsed_wall_ms: u64_at(payload, 0),
        combat_count: u32_at(payload, 8),
        changed_rows: u32_at(payload, 12),
        status_flags: u32_at(payload, 16),
        participant_count: u32_at(payload, 20),
        ended_count: u32_at(payload, 24),
    };
    if u32_at(payload, 28) != 0 {
        return Err("land combats header reserved field is nonzero".into());
    }

    let combats_len = (header.combat_count as usize)
        .checked_mul(COMBAT_ROW_LEN)
        .ok_or_else(|| {
            format!(
                "land combats combat_count {} overflows",
                header.combat_count
            )
        })?;
    let participants_len = (header.participant_count as usize)
        .checked_mul(PARTICIPANT_ROW_LEN)
        .ok_or_else(|| {
            format!(
                "land combats participant_count {} overflows",
                header.participant_count
            )
        })?;
    let expected = HEADER_LEN
        .checked_add(combats_len)
        .and_then(|len| len.checked_add(participants_len))
        .ok_or_else(|| "land combats payload length overflows".to_string())?;
    if payload.len() != expected {
        return Err(format!("land combats len {} != {expected}", payload.len()));
    }
    let combats_end = HEADER_LEN + combats_len;
    Ok((
        header,
        &payload[HEADER_LEN..combats_end],
        &payload[combats_end..],
    ))
}

#[derive(Clone, Debug, PartialEq)]
pub struct DecodedPayload {
    pub header: PayloadHeader,
    pub combats: Vec<(Key, Record)>,
    pub participants: Vec<(Key, Record)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CombatRow {
    combat_id: u32,
    generation: u64,
    flags: u32,
    location: u32,
    attacker_count: usize,
    defender_count: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ParticipantRow {
    combat_id: u32,
    generation: u64,
    side: u32,
    division_id: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ParticipantCounts {
    attackers_exact: bool,
    defenders_exact: bool,
    expected_attackers: usize,
    expected_defenders: usize,
    actual_attackers: usize,
    actual_defenders: usize,
}

/// Decode a structurally valid, non-failure d21 payload into portable rows.
///
/// This deliberately does not interpret `status_flags`: status authority lives
/// in `compact_contract`. Calling it for a failure-status record would tighten
/// the shipping grammar, which does not validate row semantics on that path.
pub fn decode_success_payload(
    supplied_schema: &Schema,
    payload: &[u8],
    frame_tick: u64,
) -> core::result::Result<DecodedPayload, String> {
    let (header, combat_bytes, participant_bytes) = split_payload(payload)?;
    require_canonical_schema(supplied_schema)?;
    let combats_collection = collection(supplied_schema, COMBATS)?;
    let participants_collection = collection(supplied_schema, PARTICIPANTS)?;

    let mut seen_combat_ids = BTreeSet::new();
    let mut counts = BTreeMap::new();
    let mut combats = Vec::with_capacity(header.combat_count as usize);
    let mut declared_participants = 0usize;
    for (row_index, row) in combat_bytes.chunks_exact(COMBAT_ROW_LEN).enumerate() {
        let parsed = parse_combat_row(row, row_index)?;
        if !seen_combat_ids.insert(parsed.combat_id) {
            return Err(format!(
                "land combat row {row_index} duplicates combat_id {}",
                parsed.combat_id
            ));
        }
        declared_participants = declared_participants
            .checked_add(parsed.attacker_count)
            .and_then(|count| count.checked_add(parsed.defender_count))
            .ok_or_else(|| "land combat declared participant count overflows".to_string())?;
        counts.insert(
            (parsed.combat_id, parsed.generation),
            ParticipantCounts {
                attackers_exact: parsed.flags & FLAG_ATTACKERS_EXACT != 0,
                defenders_exact: parsed.flags & FLAG_DEFENDERS_EXACT != 0,
                expected_attackers: parsed.attacker_count,
                expected_defenders: parsed.defender_count,
                actual_attackers: 0,
                actual_defenders: 0,
            },
        );
        combats.push((
            combat_key(parsed.combat_id, parsed.generation),
            combat_record(combats_collection, parsed, frame_tick)?,
        ));
    }
    if declared_participants != header.participant_count as usize {
        return Err(format!(
            "land combat participant_count {} != declared {declared_participants}",
            header.participant_count
        ));
    }

    let mut seen_participants = BTreeSet::new();
    let mut next_participant_ordinal = BTreeMap::new();
    let mut participants = Vec::with_capacity(header.participant_count as usize);
    for (row_index, row) in participant_bytes
        .chunks_exact(PARTICIPANT_ROW_LEN)
        .enumerate()
    {
        let parsed = parse_participant_row(row, row_index)?;
        let Some(combat_counts) = counts.get_mut(&(parsed.combat_id, parsed.generation)) else {
            return Err(format!(
                "land combat participant row {row_index} references unknown combat {}:{}",
                parsed.combat_id, parsed.generation
            ));
        };
        if !seen_participants.insert((
            parsed.combat_id,
            parsed.generation,
            parsed.side,
            parsed.division_id,
        )) {
            return Err(format!(
                "land combat participant row {row_index} duplicates participant"
            ));
        }
        match parsed.side {
            SIDE_ATTACKER if combat_counts.attackers_exact => {
                combat_counts.actual_attackers += 1;
            }
            SIDE_ATTACKER => {
                return Err(format!(
                    "land combat participant row {row_index} has attacker without exact flag"
                ));
            }
            SIDE_DEFENDER if combat_counts.defenders_exact => {
                combat_counts.actual_defenders += 1;
            }
            SIDE_DEFENDER => {
                return Err(format!(
                    "land combat participant row {row_index} has defender without exact flag"
                ));
            }
            _ => {
                return Err(format!(
                    "land combat participant row {row_index} has invalid side {}",
                    parsed.side
                ));
            }
        }
        let ordinal = next_participant_ordinal
            .entry((parsed.combat_id, parsed.generation, parsed.side))
            .or_insert(0u32);
        participants.push((
            participant_key(
                parsed.combat_id,
                parsed.generation,
                parsed.side,
                parsed.division_id,
            ),
            participant_record(participants_collection, *ordinal, frame_tick)?,
        ));
        *ordinal = (*ordinal)
            .checked_add(1)
            .ok_or_else(|| "land combat participant ordinal overflows".to_string())?;
    }

    for ((combat_id, _generation), count) in counts {
        if count.actual_attackers != count.expected_attackers {
            return Err(format!(
                "land combat {combat_id} attacker count mismatch"
            ));
        }
        if count.actual_defenders != count.expected_defenders {
            return Err(format!(
                "land combat {combat_id} defender count mismatch"
            ));
        }
    }

    Ok(DecodedPayload {
        header,
        combats,
        participants,
    })
}

pub fn key_from_combat_row(row: &[u8]) -> core::result::Result<Key, String> {
    let parsed = parse_combat_row(row, 0)?;
    Ok(combat_key(parsed.combat_id, parsed.generation))
}

pub fn record_from_combat_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    require_canonical_collection(
        collection,
        COMBATS_INDEX,
        NONCANONICAL_COMBATS_COLLECTION,
    )?;
    combat_record(collection, parse_combat_row(row, 0)?, frame_tick)
}

pub fn key_from_participant_row(row: &[u8]) -> core::result::Result<Key, String> {
    let parsed = parse_participant_row(row, 0)?;
    Ok(participant_key(
        parsed.combat_id,
        parsed.generation,
        parsed.side,
        parsed.division_id,
    ))
}

pub fn record_from_participant_row(
    collection: &Collection,
    row: &[u8],
    ordinal: u32,
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    require_canonical_collection(
        collection,
        PARTICIPANTS_INDEX,
        NONCANONICAL_PARTICIPANTS_COLLECTION,
    )?;
    parse_participant_row(row, 0)?;
    participant_record(collection, ordinal, frame_tick)
}

fn parse_combat_row(
    row: &[u8],
    row_index: usize,
) -> core::result::Result<CombatRow, String> {
    check_row_len(row, COMBAT_ROW_LEN, "combat")?;
    let combat_id = u32_at(row, 0);
    let generation = u64_at(row, 4);
    let flags = u32_at(row, 12);
    let location = u32_at(row, 16);
    let day = u32_at(row, 20);
    let duration = u32_at(row, 24);
    let attacker_count = u32_at(row, 28) as usize;
    let defender_count = u32_at(row, 32) as usize;

    if combat_id == 0 {
        return Err(format!("land combat row {row_index} has zero combat_id"));
    }
    if flags & !KNOWN_COMBAT_FLAGS != 0 {
        return Err(format!(
            "land combat row {row_index} has unknown flags {flags}"
        ));
    }
    if flags & FLAG_HAS_LOCATION != 0 && location == 0 {
        return Err(format!(
            "land combat row {row_index} has invalid location {location}"
        ));
    }
    if flags & FLAG_HAS_LOCATION == 0 && location != 0 {
        return Err(format!(
            "land combat row {row_index} has location without flag"
        ));
    }
    if flags & FLAG_HAS_DAY == 0 && day != 0 {
        return Err(format!("land combat row {row_index} has day without flag"));
    }
    if flags & FLAG_HAS_DURATION == 0 && duration != 0 {
        return Err(format!(
            "land combat row {row_index} has duration without flag"
        ));
    }
    if flags & FLAG_ATTACKERS_EXACT == 0 && attacker_count != 0 {
        return Err(format!(
            "land combat row {row_index} has attackers without exact flag"
        ));
    }
    if flags & FLAG_DEFENDERS_EXACT == 0 && defender_count != 0 {
        return Err(format!(
            "land combat row {row_index} has defenders without exact flag"
        ));
    }
    if u32_at(row, 36) != 0 {
        return Err(format!(
            "land combat row {row_index} reserved field is nonzero"
        ));
    }
    Ok(CombatRow {
        combat_id,
        generation,
        flags,
        location,
        attacker_count,
        defender_count,
    })
}

fn parse_participant_row(
    row: &[u8],
    row_index: usize,
) -> core::result::Result<ParticipantRow, String> {
    check_row_len(row, PARTICIPANT_ROW_LEN, "participant")?;
    let parsed = ParticipantRow {
        combat_id: u32_at(row, 0),
        generation: u64_at(row, 4),
        side: u32_at(row, 12),
        division_id: u32_at(row, 16),
    };
    if parsed.division_id == 0 {
        return Err(format!(
            "land combat participant row {row_index} has zero division_id"
        ));
    }
    if !matches!(parsed.side, SIDE_ATTACKER | SIDE_DEFENDER) {
        return Err(format!(
            "land combat participant row {row_index} has invalid side {}",
            parsed.side
        ));
    }
    if u32_at(row, 20) != 0 {
        return Err(format!(
            "land combat participant row {row_index} reserved field is nonzero"
        ));
    }
    Ok(parsed)
}

fn combat_record(
    collection: &Collection,
    row: CombatRow,
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    require_canonical_collection(
        collection,
        COMBATS_INDEX,
        NONCANONICAL_COMBATS_COLLECTION,
    )?;
    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        values.push(match f.field_id {
            field::LOCATION => {
                if row.flags & FLAG_HAS_LOCATION != 0 {
                    Value::ContentId(row.location as u64)
                } else {
                    Value::Unknown
                }
            }
            field::ATTACKERS_KNOWN => Value::Bool(row.flags & FLAG_ATTACKERS_EXACT != 0),
            field::DEFENDERS_KNOWN => Value::Bool(row.flags & FLAG_DEFENDERS_EXACT != 0),
            other => {
                return Err(format!(
                    "land combats has no combat mapping for field id {other}"
                ))
            }
        });
    }
    Ok(Record::new(values))
}

fn participant_record(
    collection: &Collection,
    ordinal: u32,
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    require_canonical_collection(
        collection,
        PARTICIPANTS_INDEX,
        NONCANONICAL_PARTICIPANTS_COLLECTION,
    )?;
    let mut values = Vec::with_capacity(collection.fields.len());
    for field in collection.value_fields() {
        values.push(match field.field_id {
            field::PARTICIPANT_ORDINAL => Value::Unsigned(ordinal as u64),
            other => {
                return Err(format!(
                    "land combats has no participant value mapping for field id {other}"
                ))
            }
        });
    }
    Ok(Record::new(values))
}

fn combat_key(combat_id: u32, generation: u64) -> Key {
    key(&[combat_id as u64, generation])
}

fn participant_key(combat_id: u32, generation: u64, side: u32, division_id: u32) -> Key {
    key(&[
        combat_id as u64,
        generation,
        side as u64,
        division_id as u64,
    ])
}

fn collection(schema: &Schema, collection_id: u32) -> core::result::Result<&Collection, String> {
    schema
        .collections
        .iter()
        .find(|collection| collection.collection_id == collection_id)
        .ok_or_else(|| format!("land combats schema lacks collection {collection_id}"))
}

fn require_canonical_schema(supplied: &Schema) -> core::result::Result<(), String> {
    if supplied != &schema() {
        return Err(NONCANONICAL_SCHEMA.into());
    }
    Ok(())
}

fn require_canonical_collection(
    supplied: &Collection,
    canonical_index: usize,
    error: &'static str,
) -> core::result::Result<(), String> {
    let canonical = schema();
    if supplied != &canonical.collections[canonical_index] {
        return Err(error.into());
    }
    Ok(())
}

fn check_row_len(row: &[u8], expected: usize, kind: &str) -> core::result::Result<(), String> {
    if row.len() != expected {
        return Err(format!(
            "land combat {kind} row length {} != {expected}",
            row.len()
        ));
    }
    Ok(())
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("validated fixed-width row"),
    )
}

fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("validated fixed-width row"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn combat_row(
        combat_id: u32,
        generation: u64,
        flags: u32,
        location: u32,
        day: u32,
        duration: u32,
        attackers: u32,
        defenders: u32,
    ) -> [u8; COMBAT_ROW_LEN] {
        let mut row = [0u8; COMBAT_ROW_LEN];
        row[0..4].copy_from_slice(&combat_id.to_le_bytes());
        row[4..12].copy_from_slice(&generation.to_le_bytes());
        row[12..16].copy_from_slice(&flags.to_le_bytes());
        row[16..20].copy_from_slice(&location.to_le_bytes());
        row[20..24].copy_from_slice(&day.to_le_bytes());
        row[24..28].copy_from_slice(&duration.to_le_bytes());
        row[28..32].copy_from_slice(&attackers.to_le_bytes());
        row[32..36].copy_from_slice(&defenders.to_le_bytes());
        row
    }

    fn participant_row(
        combat_id: u32,
        generation: u64,
        side: u32,
        division_id: u32,
    ) -> [u8; PARTICIPANT_ROW_LEN] {
        let mut row = [0u8; PARTICIPANT_ROW_LEN];
        row[0..4].copy_from_slice(&combat_id.to_le_bytes());
        row[4..12].copy_from_slice(&generation.to_le_bytes());
        row[12..16].copy_from_slice(&side.to_le_bytes());
        row[16..20].copy_from_slice(&division_id.to_le_bytes());
        row
    }

    fn payload(
        status_flags: u32,
        combats: &[[u8; COMBAT_ROW_LEN]],
        participants: &[[u8; PARTICIPANT_ROW_LEN]],
    ) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&17u64.to_le_bytes());
        payload.extend_from_slice(&(combats.len() as u32).to_le_bytes());
        payload.extend_from_slice(&(combats.len() as u32).to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&(participants.len() as u32).to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        for row in combats {
            payload.extend_from_slice(row);
        }
        for row in participants {
            payload.extend_from_slice(row);
        }
        payload
    }

    fn index_of(collection: &Collection, field_id: u32) -> usize {
        collection
            .value_fields()
            .position(|field| field.field_id == field_id)
            .expect("field exists")
    }

    #[test]
    fn schema_pins_lifecycle_identity_and_local_participant_order() {
        let schema = schema();
        schema.validate(0).expect("schema validates");
        assert_eq!(schema.collections.len(), COLLECTION_COUNT);
        assert_eq!(schema.collections[COMBATS_INDEX].shape, Shape::KeyedSet);
        assert_eq!(
            schema.collections[PARTICIPANTS_INDEX].shape,
            Shape::KeyedSet
        );
        assert_eq!(
            schema.collections[COMBATS_INDEX]
                .key_fields()
                .iter()
                .map(|field| field.field_id)
                .collect::<Vec<_>>(),
            vec![field::COMBAT_ID, field::GENERATION]
        );
    }

    #[test]
    fn success_payload_maps_fold_state_without_day_or_duration() {
        let combat = combat_row(
            7,
            44,
            KNOWN_COMBAT_FLAGS,
            123,
            9,
            3,
            2,
            1,
        );
        let participants = [
            participant_row(7, 44, SIDE_ATTACKER, 1001),
            participant_row(7, 44, SIDE_ATTACKER, 1002),
            participant_row(7, 44, SIDE_DEFENDER, 2001),
        ];
        let schema = schema();
        let decoded =
            decode_success_payload(&schema, &payload(0, &[combat], &participants), 0).unwrap();
        assert_eq!(decoded.combats.len(), 1);
        assert_eq!(decoded.participants.len(), 3);
        assert_eq!(decoded.combats[0].0, key(&[7, 44]));

        let combats = &schema.collections[COMBATS_INDEX];
        let values = &decoded.combats[0].1.values;
        assert_eq!(
            values[index_of(combats, field::LOCATION)],
            Value::ContentId(123)
        );
        assert_eq!(
            values[index_of(combats, field::ATTACKERS_KNOWN)],
            Value::Bool(true)
        );
        assert_eq!(
            values[index_of(combats, field::DEFENDERS_KNOWN)],
            Value::Bool(true)
        );
        assert_eq!(values.len(), 3, "day and duration are fold-only losses");
        assert_eq!(
            decoded
                .participants
                .iter()
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>(),
            vec![
                key(&[7, 44, 1, 1001]),
                key(&[7, 44, 1, 1002]),
                key(&[7, 44, 2, 2001]),
            ]
        );
        assert_eq!(
            decoded
                .participants
                .iter()
                .map(|(_, record)| record.values[0])
                .collect::<Vec<_>>(),
            vec![
                Value::Unsigned(0),
                Value::Unsigned(1),
                Value::Unsigned(0),
            ],
            "ordinals are local to each combat side"
        );
    }

    #[test]
    fn measured_empty_and_unknown_participants_remain_distinct() {
        let schema = schema();
        let measured_empty = combat_row(
            1,
            1,
            FLAG_HAS_LOCATION | FLAG_ATTACKERS_EXACT | FLAG_DEFENDERS_EXACT,
            8,
            0,
            0,
            0,
            0,
        );
        let unknown = combat_row(2, 2, 0, 0, 0, 0, 0, 0);
        let decoded =
            decode_success_payload(&schema, &payload(0, &[measured_empty, unknown], &[]), 0)
                .unwrap();
        let collection = &schema.collections[COMBATS_INDEX];
        assert_eq!(
            decoded.combats[0].1.values[index_of(collection, field::ATTACKERS_KNOWN)],
            Value::Bool(true)
        );
        assert_eq!(
            decoded.combats[1].1.values[index_of(collection, field::ATTACKERS_KNOWN)],
            Value::Bool(false)
        );
        assert_eq!(
            decoded.combats[1].1.values[index_of(collection, field::LOCATION)],
            Value::Unknown
        );
    }

    #[test]
    fn structural_boundary_runs_before_status_but_rows_do_not() {
        let malformed_row = combat_row(0, 0, u32::MAX, 99, 1, 1, 0, 0);
        let failure_payload = payload(1, &[malformed_row], &[]);
        assert!(
            split_payload(&failure_payload).is_ok(),
            "shipping fold excludes a well-framed failure before row checks"
        );
        assert!(decode_success_payload(&schema(), &failure_payload, 0).is_err());

        let mut bad_frame = failure_payload;
        bad_frame[28] = 1;
        assert!(
            split_payload(&bad_frame).is_err(),
            "header structure is checked even on a failure record"
        );
    }

    #[test]
    fn exact_participant_join_counts_and_duplicates_are_rejected() {
        let combat = combat_row(
            9,
            5,
            FLAG_HAS_LOCATION | FLAG_ATTACKERS_EXACT | FLAG_DEFENDERS_EXACT,
            3,
            0,
            0,
            1,
            0,
        );
        let missing = payload(0, &[combat], &[]);
        assert!(decode_success_payload(&schema(), &missing, 0)
            .unwrap_err()
            .contains("participant_count"));

        let duplicate = participant_row(9, 5, SIDE_ATTACKER, 77);
        let mut declared_two = combat;
        declared_two[28..32].copy_from_slice(&2u32.to_le_bytes());
        let duplicate_payload = payload(0, &[declared_two], &[duplicate, duplicate]);
        assert!(decode_success_payload(&schema(), &duplicate_payload, 0)
            .unwrap_err()
            .contains("duplicates participant"));

        let orphan = participant_row(10, 5, SIDE_ATTACKER, 77);
        let orphan_payload = payload(0, &[combat], &[orphan]);
        assert!(decode_success_payload(&schema(), &orphan_payload, 0)
            .unwrap_err()
            .contains("references unknown combat"));
    }

    #[test]
    fn packing_and_reserved_words_are_enforced() {
        let bad_location = combat_row(1, 1, 0, 4, 0, 0, 0, 0);
        assert!(decode_success_payload(&schema(), &payload(0, &[bad_location], &[]), 0)
            .unwrap_err()
            .contains("location without flag"));

        let bad_day = combat_row(1, 1, FLAG_HAS_LOCATION, 4, 1, 0, 0, 0);
        assert!(decode_success_payload(&schema(), &payload(0, &[bad_day], &[]), 0)
            .unwrap_err()
            .contains("day without flag"));

        let mut reserved = participant_row(1, 1, SIDE_ATTACKER, 3);
        reserved[20] = 1;
        assert!(record_from_participant_row(
            &schema().collections[PARTICIPANTS_INDEX],
            &reserved,
            0,
            0
        )
        .unwrap_err()
        .contains("reserved"));
    }

    #[test]
    fn payload_decode_rejects_every_noncanonical_schema_declaration() {
        let combat = combat_row(
            7,
            44,
            FLAG_HAS_LOCATION | FLAG_ATTACKERS_EXACT | FLAG_DEFENDERS_EXACT,
            123,
            0,
            0,
            1,
            0,
        );
        let participant = participant_row(7, 44, SIDE_ATTACKER, 1001);
        let bytes = payload(0, &[combat], &[participant]);
        let decoded =
            decode_success_payload(&schema(), &bytes, 0).expect("canonical schema maps every fact");
        assert_eq!(decoded.combats[0].1.values[0], Value::ContentId(123));
        assert_eq!(decoded.participants[0].1.values[0], Value::Unsigned(0));

        let mut cases = Vec::new();

        let mut omitted_field = schema();
        omitted_field.collections[COMBATS_INDEX].fields.remove(2);
        cases.push(("omitted captured location field", omitted_field));

        let mut retyped_field = schema();
        retyped_field.collections[COMBATS_INDEX].fields[3].kind = Kind::Unsigned;
        cases.push(("retyped attacker exactness witness", retyped_field));

        let mut reordered_fields = schema();
        reordered_fields.collections[COMBATS_INDEX].fields.swap(3, 4);
        cases.push(("reordered record fields", reordered_fields));

        let mut wrong_shape = schema();
        wrong_shape.collections[PARTICIPANTS_INDEX].shape = Shape::OrderedKeyedList;
        cases.push(("wrong participant collection shape", wrong_shape));

        let mut extra_field = schema();
        extra_field.collections[COMBATS_INDEX]
            .fields
            .push(Field::new(999, Kind::Unsigned));
        cases.push(("extra unmapped combat field", extra_field));

        let mut removed_key_flag = schema();
        removed_key_flag.collections[COMBATS_INDEX].fields[0].key_position = None;
        cases.push(("removed combat-id key flag", removed_key_flag));

        let mut changed_key_position = schema();
        changed_key_position.collections[COMBATS_INDEX].fields[0].key_position = Some(1);
        cases.push(("changed combat-id key position", changed_key_position));

        let mut changed_cadence = schema();
        changed_cadence.cadence_log2 = 1;
        cases.push(("changed cadence", changed_cadence));

        let mut event_part = schema();
        event_part.event_part = Some(crate::schema::EventPart {
            collection_id: 99,
            fields: vec![Field::new(1, Kind::Unsigned)],
            frontier_component_ids: vec![1],
            sequence_index: 0,
        });
        cases.push(("unexpected event part", event_part));

        let mut reordered_collections = schema();
        reordered_collections.collections.swap(COMBATS_INDEX, PARTICIPANTS_INDEX);
        cases.push(("reordered collections", reordered_collections));

        let mut omitted_collection = schema();
        omitted_collection.collections.pop();
        cases.push(("omitted participant collection", omitted_collection));

        for (case, mutated) in cases {
            assert_eq!(
                decode_success_payload(&mutated, &bytes, 0).unwrap_err(),
                NONCANONICAL_SCHEMA,
                "{case} must not silently lose or reinterpret a d21 fact"
            );
        }
    }

    #[test]
    fn direct_combat_mapper_requires_the_exact_canonical_collection() {
        let row = combat_row(
            7,
            44,
            FLAG_HAS_LOCATION | FLAG_ATTACKERS_EXACT | FLAG_DEFENDERS_EXACT,
            123,
            0,
            0,
            0,
            0,
        );
        let canonical = schema().collections[COMBATS_INDEX].clone();
        let mapped =
            record_from_combat_row(&canonical, &row, 0).expect("canonical collection maps");
        assert_eq!(mapped.values[0], Value::ContentId(123));
        assert_eq!(mapped.values[1], Value::Bool(true));
        assert_eq!(mapped.values[2], Value::Bool(true));

        let mut cases = Vec::new();
        let mut omitted = canonical.clone();
        omitted.fields.remove(2);
        cases.push(("omitted location", omitted));
        let mut retyped = canonical.clone();
        retyped.fields[2].kind = Kind::Unsigned;
        cases.push(("retyped location", retyped));
        let mut reordered = canonical.clone();
        reordered.fields.swap(3, 4);
        cases.push(("reordered exactness fields", reordered));
        let mut wrong_shape = canonical.clone();
        wrong_shape.shape = Shape::OrderedKeyedList;
        cases.push(("wrong shape", wrong_shape));
        let mut extra = canonical.clone();
        extra.fields.push(Field::new(999, Kind::Unsigned));
        cases.push(("extra unmapped field", extra));
        let mut removed_key_flag = canonical.clone();
        removed_key_flag.fields[0].key_position = None;
        cases.push(("removed combat-id key flag", removed_key_flag));
        let mut changed_key_position = canonical;
        changed_key_position.fields[0].key_position = Some(1);
        cases.push(("changed combat-id key position", changed_key_position));

        for (case, mutated) in cases {
            assert_eq!(
                record_from_combat_row(&mutated, &row, 0).unwrap_err(),
                NONCANONICAL_COMBATS_COLLECTION,
                "{case} must fail before mapping"
            );
        }
    }

    #[test]
    fn direct_participant_mapper_requires_the_exact_canonical_collection() {
        let row = participant_row(7, 44, SIDE_ATTACKER, 1001);
        let canonical = schema().collections[PARTICIPANTS_INDEX].clone();
        let mapped = record_from_participant_row(&canonical, &row, 3, 0)
            .expect("canonical collection maps the local ordinal");
        assert_eq!(mapped.values, vec![Value::Unsigned(3)]);

        let mut cases = Vec::new();
        let mut omitted = canonical.clone();
        omitted.fields.remove(4);
        cases.push(("omitted local ordinal", omitted));
        let mut retyped = canonical.clone();
        retyped.fields[4].kind = Kind::Signed;
        cases.push(("retyped local ordinal", retyped));
        let mut reordered = canonical.clone();
        reordered.fields.swap(2, 3);
        cases.push(("reordered side and division keys", reordered));
        let mut wrong_shape = canonical.clone();
        wrong_shape.shape = Shape::OrderedKeyedList;
        cases.push(("wrong shape", wrong_shape));
        let mut extra = canonical.clone();
        extra.fields.push(Field::new(999, Kind::Unsigned));
        cases.push(("extra unmapped field", extra));
        let mut removed_key_flag = canonical.clone();
        removed_key_flag.fields[0].key_position = None;
        cases.push(("removed combat-id key flag", removed_key_flag));
        let mut changed_key_position = canonical;
        changed_key_position.fields[2].key_position = Some(3);
        cases.push(("changed side key position", changed_key_position));

        for (case, mutated) in cases {
            assert_eq!(
                record_from_participant_row(&mutated, &row, 3, 0).unwrap_err(),
                NONCANONICAL_PARTICIPANTS_COLLECTION,
                "{case} must fail before mapping"
            );
        }
    }
}
